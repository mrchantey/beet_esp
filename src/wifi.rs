//! Wi-Fi as an ECS-friendly mechanism, mirroring beet's networking workflow.
//!
//! Like [`led`](crate::led) turns the RMT peripheral into Bevy components driven
//! by an async task, this turns the ESP32 Wi-Fi station + `embassy-net` stack
//! into:
//!
//! - a **client** with beet's `Request::get(..).send().await` shape — a [`Request`]
//!   is encoded, handed to the async [`client_driver`] over the [`bridge`](crate::bridge)
//!   [`Queue`], and the awaited [`Response`] comes back through a one-shot signal;
//! - a **server** as a component — spawn [`HttpServer`] and each request fires a
//!   [`ServerRequest`] observer trigger (a simple, canned `200 OK` is returned for
//!   now; richer handler routing is what we're still blocked on upstream).
//!
//! [`WifiPlugin`] brings the station up, runs DHCP, and shares the
//! [`Stack`] so both drivers can open sockets.

use crate::bridge::Queue;
use crate::bridge::spawn_driver;
use alloc::sync::Arc;
use beet::prelude::*;
use defmt::info;
use defmt::warn;
use embassy_executor::Spawner;
use embassy_net::IpEndpoint;
use embassy_net::Runner;
use embassy_net::Stack;
use embassy_net::StackResources;
use embassy_net::tcp::TcpSocket;
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::signal::Signal;
use embassy_time::Duration;
use embassy_time::Timer;
use embedded_io_async::Write as _;
use esp_hal::peripherals::WIFI;
use esp_hal::rng::Rng;
use esp_radio::wifi::Config as RadioConfig;
use esp_radio::wifi::ControllerConfig;
use esp_radio::wifi::Interface;
use esp_radio::wifi::WifiController;
use esp_radio::wifi::sta::StationConfig;
use static_cell::StaticCell;

/// Brings up the Wi-Fi station and `embassy-net` stack, then shares the
/// [`Stack`] so the client and any [`HttpServer`] can open sockets.
///
/// Add it after [`Esp32Plugin`](crate::esp32_plugin::Esp32Plugin) (which exposes
/// the `WIFI` peripheral). The station joins the given AP via DHCP; once up, the
/// client driver services [`Request::send`] calls and each [`HttpServer`] entity
/// gets its own accept loop.
pub struct WifiPlugin {
    ssid: &'static str,
    password: &'static str,
}

impl WifiPlugin {
    /// Join the AP named by `ssid` using `password` (e.g. from `env!("SSID")`).
    pub fn new(ssid: &'static str, password: &'static str) -> Self {
        Self { ssid, password }
    }
}

impl Plugin for WifiPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(WifiCredentials {
            ssid: self.ssid,
            password: self.password,
        })
        .add_systems(Startup, start_wifi)
        .add_systems(PostStartup, spawn_servers)
        .add_systems(Update, drain_server_requests);
    }
}

/// Station credentials, stashed by [`WifiPlugin`] for [`start_wifi`] to read.
#[derive(Resource, Clone)]
struct WifiCredentials {
    ssid: &'static str,
    password: &'static str,
}

/// Claim the `WIFI` peripheral, join the AP, start DHCP and the network task,
/// and spawn the client driver. Exclusive so it can pull the non-send peripheral
/// and [`Spawner`], and publish the resulting [`Stack`].
///
/// Runs in `Startup`, after `bring_up`'s `PreStartup` (so the chip, embassy and
/// the radio scheduler are already running).
fn start_wifi(world: &mut World) {
    let creds = world.resource::<WifiCredentials>().clone();
    let wifi = world
        .remove_non_send::<WIFI<'static>>()
        .expect("add Esp32Plugin before WifiPlugin — bring_up provides the WIFI peripheral");
    let spawner = *world.non_send::<Spawner>();

    let station_config = RadioConfig::Station(
        StationConfig::default()
            .with_ssid(creds.ssid)
            .with_password(creds.password.into()),
    );
    let (controller, interfaces) = esp_radio::wifi::new(
        wifi,
        ControllerConfig::default().with_initial_config(station_config),
    )
    .expect("failed to initialize Wi-Fi controller");

    let net_config = embassy_net::Config::dhcpv4(Default::default());
    let rng = Rng::new();
    let seed = ((rng.random() as u64) << 32) | rng.random() as u64;

    static RESOURCES: StaticCell<StackResources<4>> = StaticCell::new();
    let resources = RESOURCES.init(StackResources::new());
    let (stack, runner) = embassy_net::new(interfaces.station, net_config, resources, seed);

    info!("Wi-Fi station joining `{}`", creds.ssid);
    spawn_driver(spawner, connection_loop(controller));
    spawn_driver(spawner, net_loop(runner));
    spawn_driver(spawner, client_driver(stack));

    // Stack is Copy and !Send; keep it as a non-send resource so the server
    // spawner (a later system) can hand a copy to each accept loop.
    world.insert_non_send(stack);
}

/// Keep the station associated, reconnecting on drop-out — same shape as the
/// stock `wifi-client`/`wifi-server` examples.
async fn connection_loop(mut controller: WifiController<'static>) {
    loop {
        match controller.connect_async().await {
            Ok(info) => {
                info!("Wi-Fi connected: {:?}", info);
                let reason = controller.wait_for_disconnect_async().await;
                warn!("Wi-Fi disconnected: {:?}", reason);
            }
            Err(e) => {
                warn!("connect_async failed: {:?}; retrying in 3s", e);
                Timer::after(Duration::from_secs(3)).await;
            }
        }
    }
}

/// Drive the `embassy-net` stack (polls the device, runs DHCP).
async fn net_loop(mut runner: Runner<'static, Interface<'static>>) {
    runner.run().await
}

// ---------------------------------------------------------------------------
// Client: `Request::get(..).send().await`
// ---------------------------------------------------------------------------

/// Pending client requests handed from [`Request::send`] to [`client_driver`].
static CLIENT_JOBS: Queue<ClientJob, 4> = Queue::new();

/// One-shot reply channel for an in-flight [`Request`].
type Reply = Arc<Signal<CriticalSectionRawMutex, Result<Response, WifiError>>>;

/// A request encoded and queued for the driver, with the channel to reply on.
struct ClientJob {
    remote: IpEndpoint,
    bytes: alloc::vec::Vec<u8>,
    reply: Reply,
}

/// Why a [`Request`] could not complete.
#[derive(Debug, Clone, Copy, defmt::Format)]
pub enum WifiError {
    /// The client queue was full (too many in-flight requests).
    QueueFull,
    /// The TCP connection to the remote could not be established.
    Connect,
    /// Writing the request to the socket failed.
    Write,
}

/// An outbound HTTP request, mirroring beet's `Request` builder.
///
/// ```ignore
/// let remote = IpEndpoint::new(IpAddress::Ipv4(Ipv4Address::new(1, 1, 1, 1)), 80);
/// let response = Request::get(remote).send().await?;
/// info!("status {}", response.status());
/// ```
pub struct Request {
    remote: IpEndpoint,
    method: &'static str,
    path: String,
    host: String,
    body: Option<String>,
}

impl Request {
    /// A `GET` to `/` on the given endpoint. The `Host` header defaults to the
    /// endpoint's IP; override it with [`with_host`](Self::with_host).
    pub fn get(remote: IpEndpoint) -> Self {
        Self {
            remote,
            method: "GET",
            path: String::from("/"),
            host: alloc::format!("{}", remote.addr),
            body: None,
        }
    }

    /// Set the request path (e.g. `/api/status`).
    pub fn with_path(mut self, path: impl Into<String>) -> Self {
        self.path = path.into();
        self
    }

    /// Set the `Host` header (a server may route on it).
    pub fn with_host(mut self, host: impl Into<String>) -> Self {
        self.host = host.into();
        self
    }

    /// Send a body and switch the method to `POST`.
    pub fn with_body(mut self, body: impl Into<String>) -> Self {
        self.method = "POST";
        self.body = Some(body.into());
        self
    }

    /// Encode, queue, and await the [`Response`].
    ///
    /// Call from an async driver (e.g. one spawned with
    /// [`spawn_driver`](crate::bridge::spawn_driver)); the actual socket work
    /// happens on [`client_driver`].
    pub async fn send(self) -> Result<Response, WifiError> {
        let reply: Reply = Arc::new(Signal::new());
        let job = ClientJob {
            remote: self.remote,
            bytes: self.encode(),
            reply: reply.clone(),
        };
        CLIENT_JOBS.send(job).map_err(|_| WifiError::QueueFull)?;
        reply.wait().await
    }

    /// Serialise to raw HTTP/1.1 request bytes.
    fn encode(&self) -> alloc::vec::Vec<u8> {
        let body = self.body.as_deref().unwrap_or("");
        let head = alloc::format!(
            "{} {} HTTP/1.1\r\nHost: {}\r\nConnection: close\r\nContent-Length: {}\r\n\r\n",
            self.method,
            self.path,
            self.host,
            body.len(),
        );
        let mut bytes = head.into_bytes();
        bytes.extend_from_slice(body.as_bytes());
        bytes
    }
}

/// A received HTTP response: parsed status plus the raw bytes.
pub struct Response {
    status: u16,
    body: alloc::vec::Vec<u8>,
}

impl Response {
    /// The HTTP status code (0 if the status line could not be parsed).
    pub fn status(&self) -> u16 {
        self.status
    }

    /// The raw response bytes (headers + body).
    pub fn bytes(&self) -> &[u8] {
        &self.body
    }

    /// The response as UTF-8, lossy-empty if it is not valid UTF-8.
    pub fn text(&self) -> &str {
        core::str::from_utf8(&self.body).unwrap_or("")
    }
}

/// Services [`CLIENT_JOBS`]: one TCP request at a time, replying on each job's
/// signal. Owns a copy of the [`Stack`]; waits for DHCP before the first job.
async fn client_driver(stack: Stack<'static>) {
    stack.wait_config_up().await;
    if let Some(cfg) = stack.config_v4() {
        info!("Wi-Fi up: {}", cfg);
    }
    loop {
        let job = CLIENT_JOBS.recv().await;
        let result = exchange(stack, &job).await;
        job.reply.signal(result);
    }
}

/// Open a socket, send the encoded request, and read the whole response.
async fn exchange(stack: Stack<'static>, job: &ClientJob) -> Result<Response, WifiError> {
    let mut rx = [0u8; 1536];
    let mut tx = [0u8; 1536];
    let mut socket = TcpSocket::new(stack, &mut rx, &mut tx);
    socket.set_timeout(Some(Duration::from_secs(10)));

    socket.connect(job.remote).await.map_err(|e| {
        warn!("connect failed: {:?}", e);
        WifiError::Connect
    })?;
    socket.write_all(&job.bytes).await.map_err(|e| {
        warn!("write failed: {:?}", e);
        WifiError::Write
    })?;

    let mut body = alloc::vec::Vec::new();
    let mut buf = [0u8; 512];
    loop {
        match socket.read(&mut buf).await {
            Ok(0) => break,
            Ok(n) => body.extend_from_slice(&buf[..n]),
            Err(e) => {
                warn!("read failed: {:?}", e);
                break;
            }
        }
    }
    let status = parse_status(&body).unwrap_or(0);
    Ok(Response { status, body })
}

/// Pull the status code out of an HTTP status line (`HTTP/1.1 200 OK`).
fn parse_status(bytes: &[u8]) -> Option<u16> {
    let line = bytes.split(|&b| b == b'\n').next()?;
    let line = core::str::from_utf8(line).ok()?;
    line.split_whitespace().nth(1)?.parse().ok()
}

// ---------------------------------------------------------------------------
// Server: an `HttpServer` component triggering a `ServerRequest` observer
// ---------------------------------------------------------------------------

/// A TCP/HTTP server bound to a port, spawned as an ECS component.
///
/// Each accepted request returns a canned `200 OK` and fires a [`ServerRequest`]
/// observer trigger so app systems can react (count visitors, blink an LED, …).
#[derive(Component, Clone, Copy)]
pub struct HttpServer {
    /// The TCP port to listen on.
    pub port: u16,
}

impl Default for HttpServer {
    fn default() -> Self {
        Self { port: 8080 }
    }
}

impl HttpServer {
    /// Listen on the given port.
    pub fn new(port: u16) -> Self {
        Self { port }
    }
}

/// Fired as an observer trigger for every request an [`HttpServer`] handles.
#[derive(Event, Clone)]
pub struct ServerRequest {
    /// The port the request arrived on.
    pub port: u16,
    /// 1-based request count for this server since boot.
    pub count: u32,
    /// The HTTP request line (e.g. `GET / HTTP/1.1`).
    pub line: String,
}

/// Requests handed from a [`server_loop`] to the ECS via [`drain_server_requests`].
static SERVER_REQUESTS: Queue<ServerRequest, 8> = Queue::new();

/// Spawn an accept loop for each [`HttpServer`] entity. Exclusive so it can read
/// the non-send [`Stack`]/[`Spawner`]; runs in `PostStartup`, after
/// [`start_wifi`].
fn spawn_servers(world: &mut World) {
    let stack = *world.non_send::<Stack<'static>>();
    let spawner = *world.non_send::<Spawner>();

    let mut query = world.query::<&HttpServer>();
    let ports: alloc::vec::Vec<u16> = query.iter(world).map(|server| server.port).collect();
    for port in ports {
        spawn_driver(spawner, server_loop(stack, port));
    }
}

/// Accept connections forever; reply with a canned page and queue a
/// [`ServerRequest`] for the ECS to observe.
async fn server_loop(stack: Stack<'static>, port: u16) {
    stack.wait_config_up().await;
    if let Some(cfg) = stack.config_v4() {
        info!("HTTP server: http://{}:{}", cfg.address.address(), port);
    }

    let mut count: u32 = 0;
    loop {
        let mut rx = [0u8; 1536];
        let mut tx = [0u8; 1536];
        let mut socket = TcpSocket::new(stack, &mut rx, &mut tx);
        socket.set_timeout(Some(Duration::from_secs(10)));

        if let Err(e) = socket.accept(port).await {
            warn!("accept failed: {:?}", e);
            continue;
        }
        count += 1;

        let mut req = [0u8; 512];
        let n = socket.read(&mut req).await.unwrap_or(0);
        let line = first_line(&req[..n]);

        let response = b"HTTP/1.0 200 OK\r\nContent-Type: text/plain\r\nConnection: close\r\n\r\n\
            hello from the beet_esp ECS server\r\n";
        if let Err(e) = socket.write_all(response).await {
            warn!("write failed: {:?}", e);
        }
        if let Err(e) = socket.flush().await {
            warn!("flush failed: {:?}", e);
        }
        socket.close();

        // Drop-oldest semantics would be nicer, but a full queue just means the
        // ECS is behind; skip the notification rather than block the accept loop.
        let _ = SERVER_REQUESTS.send(ServerRequest { port, count, line });
    }
}

/// Drain queued [`ServerRequest`]s and fire their observer triggers.
fn drain_server_requests(mut commands: Commands) {
    while let Some(request) = SERVER_REQUESTS.try_recv() {
        commands.trigger(request);
    }
}

/// First line of a request buffer, up to the first CR/LF.
fn first_line(bytes: &[u8]) -> String {
    let line = bytes.split(|&b| b == b'\r' || b == b'\n').next().unwrap_or(&[]);
    String::from_utf8_lossy(line).into_owned()
}
