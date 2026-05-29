//! Wi-Fi as an ECS-friendly mechanism, using beet's networking types.
//!
//! Like [`led`](crate::led) turns the RMT peripheral into Bevy components driven
//! by an async task, this turns the ESP32 Wi-Fi station + `embassy-net` stack
//! into beet's transport-agnostic [`Request`]/[`Response`] workflow:
//!
//! - a **client** speaking beet's `Request::get(url).send().await` shape. The
//!   ESP32 transport is registered with beet via [`set_http_client`], so calling
//!   [`Request::send`] anywhere routes through the [`client_driver`]: the request
//!   is encoded to HTTP/1.1, handed over the [`bridge`](crate::bridge) [`Queue`],
//!   and the awaited [`Response`] comes back through a one-shot signal;
//! - a **server** as a component. Spawn [`HttpServer`] with a handler system
//!   (`In<Request>` → [`Response`]) via [`WifiAppExt::add_http_server`], mirroring
//!   beet's `HttpServer` + `Action::<Request, Response>` pattern. Each request is
//!   parsed into a beet [`Request`], run through the handler on the ECS, and the
//!   returned [`Response`] is serialised back to the socket.
//!
//! [`WifiPlugin`] brings the station up, runs DHCP, and shares the
//! [`Stack`] so both drivers can open sockets.

use crate::bridge::Queue;
use crate::bridge::spawn_driver;
use alloc::sync::Arc;
use beet::exports::bevy::ecs::system::SystemId;
use beet::prelude::*;
use defmt::info;
use defmt::warn;
use embassy_executor::Spawner;
use embassy_net::IpAddress;
use embassy_net::IpEndpoint;
use embassy_net::Runner;
use embassy_net::Stack;
use embassy_net::StackResources;
use embassy_net::dns::DnsQueryType;
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

    // Register the ESP32 transport so beet's `Request::send` routes here. It is
    // a one-time install; a second `WifiPlugin` (or a transport feature compiled
    // into beet_net) would already own it, so just warn rather than panic.
    if set_http_client(esp_send).is_err() {
        warn!("an HTTP transport was already installed; Request::send will not use Wi-Fi");
    }

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
// Client: beet's `Request::get(url).send().await`, via `set_http_client`
// ---------------------------------------------------------------------------

/// Pending client requests handed from [`esp_send`] to [`client_driver`].
static CLIENT_JOBS: Queue<ClientJob, 4> = Queue::new();

/// A request encoded and queued for the driver, with the channel to reply on.
struct ClientJob {
    /// Authority host to resolve (an IP literal or a DNS name).
    host: String,
    /// TCP port to connect to.
    port: u16,
    /// The full HTTP/1.1 request, ready to write to the socket.
    bytes: Vec<u8>,
    /// One-shot reply channel for the awaited [`Response`].
    reply: Arc<Signal<CriticalSectionRawMutex, Result<Response, BevyError>>>,
}

/// beet's transport hook (see [`set_http_client`]): encode the [`Request`], queue
/// it for [`client_driver`], and await the [`Response`].
///
/// Installed once by [`start_wifi`]; thereafter any `Request::send` on an
/// `http://` URL flows through here.
fn esp_send(request: Request) -> MaybeSendBoxedFuture<'static, Result<Response>> {
    Box::pin(async move {
        if request.scheme().is_secure() {
            bevybail!("the esp32 Wi-Fi transport only supports plain http, not https/tls");
        }
        let authority = request.authority().to_string();
        if authority.is_empty() {
            bevybail!("request URL has no host: {}", request.uri());
        }
        let (host, port) = split_authority(&authority, 80);

        let (parts, body) = request.into_parts();
        let body = body.into_bytes().await?;
        let bytes = encode_request(&parts, &authority, &body);

        let reply = Arc::new(Signal::new());
        let job = ClientJob {
            host,
            port,
            bytes,
            reply: reply.clone(),
        };
        CLIENT_JOBS
            .send(job)
            .map_err(|_| bevyhow!("Wi-Fi client queue full"))?;
        reply.wait().await
    })
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

/// Resolve the host, open a socket, send the encoded request, and read the whole
/// response into a beet [`Response`].
async fn exchange(stack: Stack<'static>, job: &ClientJob) -> Result<Response> {
    // `dns_query` short-circuits IP literals, so this covers both `1.1.1.1` and
    // real hostnames (DNS servers come from the DHCP lease).
    let addrs = stack
        .dns_query(&job.host, DnsQueryType::A)
        .await
        .map_err(|e| bevyhow!("DNS lookup for `{}` failed: {:?}", job.host.as_str(), e))?;
    let addr: IpAddress = addrs
        .into_iter()
        .next()
        .ok_or_else(|| bevyhow!("no DNS results for `{}`", job.host.as_str()))?;
    let remote = IpEndpoint::new(addr, job.port);

    let mut rx = [0u8; 1536];
    let mut tx = [0u8; 1536];
    let mut socket = TcpSocket::new(stack, &mut rx, &mut tx);
    socket.set_timeout(Some(Duration::from_secs(10)));

    socket
        .connect(remote)
        .await
        .map_err(|e| bevyhow!("connect to {} failed: {:?}", job.host.as_str(), e))?;
    socket
        .write_all(&job.bytes)
        .await
        .map_err(|e| bevyhow!("write failed: {:?}", e))?;

    let mut raw = Vec::new();
    let mut buf = [0u8; 512];
    loop {
        match socket.read(&mut buf).await {
            Ok(0) => break,
            Ok(n) => raw.extend_from_slice(&buf[..n]),
            Err(e) => {
                warn!("read failed: {:?}", e);
                break;
            }
        }
    }
    Ok(parse_response(&raw))
}

// ---------------------------------------------------------------------------
// Server: an `HttpServer` component + a handler system (In<Request> -> Response)
// ---------------------------------------------------------------------------

/// A TCP/HTTP server bound to a port, spawned as an ECS component.
///
/// Mirrors beet's `HttpServer`: spawn it together with a handler system via
/// [`WifiAppExt::add_http_server`]. An `HttpServer` spawned without a handler
/// answers every request with `404 Not Found`.
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

/// The handler for an [`HttpServer`]: a one-shot system taking the incoming
/// [`Request`] and returning a [`Response`], registered via
/// [`WifiAppExt::add_http_server`]. This is the no_std stand-in for beet's
/// `Action::<Request, Response>`.
#[derive(Component)]
struct ServerHandler(SystemId<In<Request>, Response>);

/// Extension for registering an [`HttpServer`] together with its handler system.
pub trait WifiAppExt {
    /// Spawn an [`HttpServer`] on `port` whose requests are handled by `handler`,
    /// a system of the form `fn(In<Request>, ...) -> Response`.
    ///
    /// ```ignore
    /// app.add_http_server(8080, |request: In<Request>| {
    ///     Response::ok_body("hello from beet_esp", MediaType::Text)
    /// });
    /// ```
    fn add_http_server<M>(
        &mut self,
        port: u16,
        handler: impl IntoSystem<In<Request>, Response, M> + 'static,
    ) -> &mut Self;
}

impl WifiAppExt for App {
    fn add_http_server<M>(
        &mut self,
        port: u16,
        handler: impl IntoSystem<In<Request>, Response, M> + 'static,
    ) -> &mut Self {
        let id = self.world_mut().register_system(handler);
        self.world_mut().spawn((HttpServer::new(port), ServerHandler(id)));
        self
    }
}

/// An accepted request awaiting an ECS-produced [`Response`].
struct ServerExchange {
    /// The port the request arrived on (selects the handler).
    port: u16,
    /// The parsed incoming request.
    request: Request,
    /// One-shot channel the drain system replies on.
    reply: Arc<Signal<CriticalSectionRawMutex, Response>>,
}

/// Requests handed from a [`server_loop`] to the ECS via [`drain_server_requests`].
static SERVER_EXCHANGES: Queue<ServerExchange, 4> = Queue::new();

/// Spawn an accept loop for each [`HttpServer`] entity. Exclusive so it can read
/// the non-send [`Stack`]/[`Spawner`]; runs in `PostStartup`, after
/// [`start_wifi`].
fn spawn_servers(world: &mut World) {
    let stack = *world.non_send::<Stack<'static>>();
    let spawner = *world.non_send::<Spawner>();

    let mut query = world.query::<&HttpServer>();
    let ports: Vec<u16> = query.iter(world).map(|server| server.port).collect();
    for port in ports {
        spawn_driver(spawner, server_loop(stack, port));
    }
}

/// Accept connections forever; parse each into a beet [`Request`], hand it to the
/// ECS for a [`Response`], and serialise the reply back to the socket.
async fn server_loop(stack: Stack<'static>, port: u16) {
    stack.wait_config_up().await;
    if let Some(cfg) = stack.config_v4() {
        info!("HTTP server: http://{}:{}", cfg.address.address(), port);
    }

    loop {
        let mut rx = [0u8; 1536];
        let mut tx = [0u8; 1536];
        let mut socket = TcpSocket::new(stack, &mut rx, &mut tx);
        socket.set_timeout(Some(Duration::from_secs(10)));

        if let Err(e) = socket.accept(port).await {
            warn!("accept failed: {:?}", e);
            continue;
        }

        let raw = read_http(&mut socket).await;
        let request = match parse_http_request(&raw) {
            Ok(request) => request,
            Err(e) => {
                warn!("malformed request: {}", defmt::Debug2Format(&e));
                let _ = socket
                    .write_all(b"HTTP/1.1 400 Bad Request\r\ncontent-length: 0\r\nconnection: close\r\n\r\n")
                    .await;
                let _ = socket.flush().await;
                socket.close();
                continue;
            }
        };

        let reply = Arc::new(Signal::new());
        let exchange = ServerExchange {
            port,
            request,
            reply: reply.clone(),
        };
        if SERVER_EXCHANGES.send(exchange).is_err() {
            warn!("server queue full; dropping request on :{}", port);
            let _ = socket
                .write_all(b"HTTP/1.1 503 Service Unavailable\r\ncontent-length: 0\r\nconnection: close\r\n\r\n")
                .await;
            let _ = socket.flush().await;
            socket.close();
            continue;
        }

        let response = reply.wait().await;
        let bytes = serialize_response(response).await;
        if let Err(e) = socket.write_all(&bytes).await {
            warn!("write failed: {:?}", e);
        }
        if let Err(e) = socket.flush().await {
            warn!("flush failed: {:?}", e);
        }
        socket.close();
    }
}

/// Drain queued [`ServerExchange`]s, run each through its [`HttpServer`]'s handler
/// system, and reply with the produced [`Response`].
fn drain_server_requests(world: &mut World) {
    // Snapshot (port, handler) pairs; the query borrow must end before we call
    // `run_system_with` (which needs `&mut World`).
    let mut handlers: Vec<(u16, SystemId<In<Request>, Response>)> = Vec::new();
    {
        let mut query = world.query::<(&HttpServer, &ServerHandler)>();
        for (server, handler) in query.iter(world) {
            handlers.push((server.port, handler.0));
        }
    }

    while let Some(exchange) = SERVER_EXCHANGES.try_recv() {
        let handler = handlers
            .iter()
            .find(|(port, _)| *port == exchange.port)
            .map(|(_, id)| *id);
        let response = match handler {
            Some(id) => world
                .run_system_with(id, exchange.request)
                .unwrap_or_else(|_| {
                    warn!("server handler errored on :{}", exchange.port);
                    Response::internal_error()
                }),
            None => Response::not_found(),
        };
        exchange.reply.signal(response);
    }
}

// ---------------------------------------------------------------------------
// HTTP/1.1 wire helpers (no_std, beet types)
// ---------------------------------------------------------------------------

/// Split an authority into `(host, port)`, falling back to `default_port`.
fn split_authority(authority: &str, default_port: u16) -> (String, u16) {
    match authority.rsplit_once(':') {
        Some((host, port)) => (host.to_string(), port.parse().unwrap_or(default_port)),
        None => (authority.to_string(), default_port),
    }
}

/// The uppercase HTTP token for a method (the [`HttpMethod`] `Display` is
/// title-case, e.g. `Get`, so it can't be used on the wire).
fn method_token(method: &HttpMethod) -> &'static str {
    match method {
        HttpMethod::Get => "GET",
        HttpMethod::Post => "POST",
        HttpMethod::Put => "PUT",
        HttpMethod::Patch => "PATCH",
        HttpMethod::Delete => "DELETE",
        HttpMethod::Options => "OPTIONS",
        HttpMethod::Head => "HEAD",
        HttpMethod::Trace => "TRACE",
        HttpMethod::Connect => "CONNECT",
    }
}

/// Headers the encoders set themselves; user-supplied copies are skipped to
/// avoid duplicates.
fn is_managed_header(key: &str) -> bool {
    key.eq_ignore_ascii_case("host")
        || key.eq_ignore_ascii_case("content-length")
        || key.eq_ignore_ascii_case("connection")
}

/// Serialise a [`Request`] to raw HTTP/1.1 bytes (origin-form target).
fn encode_request(parts: &RequestParts, authority: &str, body: &[u8]) -> Vec<u8> {
    let path = parts.path_string();
    let query = parts.query_string();
    let target = if query.is_empty() {
        path
    } else {
        format!("{path}?{query}")
    };

    let mut head = format!("{} {} HTTP/1.1\r\n", method_token(parts.method()), target);
    head.push_str(&format!("Host: {authority}\r\n"));
    for (key, values) in parts.headers().iter_all() {
        if is_managed_header(key) {
            continue;
        }
        for value in values {
            head.push_str(&format!("{key}: {value}\r\n"));
        }
    }
    head.push_str(&format!("Content-Length: {}\r\n", body.len()));
    head.push_str("Connection: close\r\n\r\n");

    let mut bytes = head.into_bytes();
    bytes.extend_from_slice(body);
    bytes
}

/// Parse raw HTTP/1.1 response bytes into a beet [`Response`].
fn parse_response(raw: &[u8]) -> Response {
    let (header_section, body) = split_head_body(raw);
    let header_str = String::from_utf8_lossy(header_section);
    let mut lines = header_str.lines();

    let status = lines
        .next()
        .and_then(parse_status_line)
        .unwrap_or(0);
    let mut parts = ResponseParts::new(StatusCode::new(status));
    for line in lines {
        if line.is_empty() {
            break;
        }
        if let Some((key, value)) = line.split_once(':') {
            parts.headers.set_raw(key.trim(), value.trim());
        }
    }
    Response::new(parts, body.to_vec().into())
}

/// Pull the status code out of an HTTP status line (`HTTP/1.1 200 OK`).
fn parse_status_line(line: &str) -> Option<u16> {
    line.split_whitespace().nth(1)?.parse().ok()
}

/// Parse raw HTTP/1.1 request bytes into a beet [`Request`].
fn parse_http_request(raw: &[u8]) -> Result<Request> {
    let (header_section, body) = split_head_body(raw);
    let header_str = String::from_utf8_lossy(header_section);
    let mut lines = header_str.lines();

    let request_line = lines.next().ok_or_else(|| bevyhow!("empty request"))?;
    let mut tokens = request_line.split_whitespace();
    let method_str = tokens.next().ok_or_else(|| bevyhow!("missing HTTP method"))?;
    let path = tokens.next().ok_or_else(|| bevyhow!("missing HTTP path"))?;

    let method = match method_str.to_ascii_uppercase().as_str() {
        "GET" => HttpMethod::Get,
        "POST" => HttpMethod::Post,
        "PUT" => HttpMethod::Put,
        "DELETE" => HttpMethod::Delete,
        "PATCH" => HttpMethod::Patch,
        "HEAD" => HttpMethod::Head,
        "OPTIONS" => HttpMethod::Options,
        other => return Err(bevyhow!("unsupported HTTP method: {other}")),
    };

    let mut request = Request::new(method, path);
    for line in lines {
        if line.is_empty() {
            break;
        }
        if let Some((key, value)) = line.split_once(':') {
            request.headers.set_raw(key.trim(), value.trim());
        }
    }
    if !body.is_empty() {
        request.set_body(body);
    }
    Ok(request)
}

/// Serialise a beet [`Response`] into raw HTTP/1.1 bytes.
async fn serialize_response(response: Response) -> Vec<u8> {
    let (parts, body) = response.into_parts();
    let body = body.into_bytes().await.unwrap_or_default();
    let status = parts.status();

    let mut head = format!("HTTP/1.1 {} {}\r\n", status.as_u16(), status.message());
    for (key, values) in parts.headers().iter_all() {
        if is_managed_header(key) {
            continue;
        }
        for value in values {
            head.push_str(&format!("{key}: {value}\r\n"));
        }
    }
    head.push_str(&format!("content-length: {}\r\n", body.len()));
    head.push_str("connection: close\r\n\r\n");

    let mut bytes = head.into_bytes();
    bytes.extend_from_slice(&body);
    bytes
}

/// Read a full HTTP message off the socket: headers, plus the body if a
/// `Content-Length` says there is one. Stops at EOF or once the body is in.
async fn read_http(socket: &mut TcpSocket<'_>) -> Vec<u8> {
    const CAP: usize = 8192;
    let mut buf = Vec::new();
    let mut chunk = [0u8; 512];
    loop {
        match socket.read(&mut chunk).await {
            Ok(0) => break,
            Ok(n) => {
                buf.extend_from_slice(&chunk[..n]);
                if let Some(header_end) = find_header_end(&buf) {
                    let content_length = parse_content_length(&buf[..header_end]);
                    if buf.len() >= header_end + content_length {
                        break;
                    }
                }
                if buf.len() >= CAP {
                    break;
                }
            }
            Err(e) => {
                warn!("read failed: {:?}", e);
                break;
            }
        }
    }
    buf
}

/// Split a raw HTTP message into `(headers, body)` on the blank-line separator.
fn split_head_body(raw: &[u8]) -> (&[u8], &[u8]) {
    if let Some(pos) = find_subslice(raw, b"\r\n\r\n") {
        (&raw[..pos], &raw[pos + 4..])
    } else if let Some(pos) = find_subslice(raw, b"\n\n") {
        (&raw[..pos], &raw[pos + 2..])
    } else {
        (raw, &[])
    }
}

/// Byte offset just past the blank line ending the headers, if present.
fn find_header_end(buf: &[u8]) -> Option<usize> {
    if let Some(pos) = find_subslice(buf, b"\r\n\r\n") {
        Some(pos + 4)
    } else {
        find_subslice(buf, b"\n\n").map(|pos| pos + 2)
    }
}

/// Extract the `Content-Length` value from raw header bytes (0 if absent).
fn parse_content_length(header_bytes: &[u8]) -> usize {
    let header_str = String::from_utf8_lossy(header_bytes);
    for line in header_str.lines() {
        if let Some((key, value)) = line.split_once(':') {
            if key.trim().eq_ignore_ascii_case("content-length") {
                return value.trim().parse::<usize>().unwrap_or(0);
            }
        }
    }
    0
}

/// First index of `needle` within `haystack`.
fn find_subslice(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}
