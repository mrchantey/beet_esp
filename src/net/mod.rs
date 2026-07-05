//! Wi-Fi as an ECS-friendly mechanism, using beet's networking types.
//!
//! Like [`led`](crate::utils::led) turns the RMT peripheral into Bevy components driven
//! by an async task, this turns the ESP32 Wi-Fi station + `embassy-net` stack
//! into beet's transport-agnostic [`Request`]/[`Response`] workflow:
//!
//! - a **client** speaking beet's `Request::get(url).send().await` shape (see
//!   [`http_client`]). The ESP32 transport is registered with beet via
//!   [`set_http_client`], so calling [`Request::send`] anywhere routes through
//!   the Wi-Fi stack.
//! - a **server** as beet's standard [`HttpServer`] component (see
//!   [`http_server`], gated on the `action` feature). Spawn `(HttpServer,
//!   exchange_handler(..))` or `(HttpServer, default_router(children![..]))` and
//!   each request is dispatched through beet's async action layer.
//!
//! [`WifiPlugin`] brings the station up, runs DHCP, and shares the [`Stack`] so
//! both the client driver and any [`HttpServer`] accept loop can open sockets.

use crate::esp32_utils::async_bridge::spawn_driver;
use beet::prelude::*;
use embassy_executor::Spawner;
use embassy_net::Ipv4Address;
use embassy_net::Ipv4Cidr;
use embassy_net::Runner;
use embassy_net::StackResources;
use embassy_net::StaticConfigV4;
use embassy_time::Duration;
use embassy_time::Timer;
use esp_hal::peripherals::WIFI;
use esp_hal::rng::Rng;
use esp_radio::wifi::Config as RadioConfig;
use esp_radio::wifi::ControllerConfig;
use esp_radio::wifi::Interface;
use esp_radio::wifi::WifiController;
use esp_radio::wifi::sta::StationConfig;
use static_cell::StaticCell;

pub mod http_client;
#[cfg(feature = "action")]
pub mod http_server;
#[cfg(feature = "sockets")]
pub mod socket_client;
#[cfg(feature = "secure")]
pub mod tls_client;
#[cfg(feature = "mdns")]
pub mod mdns;
#[cfg(feature = "mdns")]
pub mod mdns_browser;

#[cfg(feature = "mdns")]
pub use mdns::MDns;
#[cfg(feature = "mdns")]
pub use mdns_browser::{EmbassyUdpEndpoint, EmbassyUdpSocket};

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
    /// An optional static IPv4 for the station; `None` uses DHCP. The device's
    /// address is a link concern, shared by every socket (server, client, mDNS),
    /// so it lives here on the connection, not on a single `HttpServer`.
    static_ip: Option<[u8; 4]>,
}

impl WifiPlugin {
    /// Join the AP named by `ssid` using `password` (eg from `env!("BEET_WIFI_SSID")`).
    pub fn new(ssid: &'static str, password: &'static str) -> Self {
        Self {
            ssid,
            password,
            static_ip: None,
        }
    }

    /// Join the AP named by the `BEET_WIFI_SSID`/`BEET_WIFI_PASSWORD` env vars,
    /// which `build.rs` exposes from the local `.env` (so credentials stay
    /// uncommitted).
    pub fn from_env() -> Self {
        Self::new(env!("BEET_WIFI_SSID"), env!("BEET_WIFI_PASSWORD"))
    }

    /// Assign an explicit static IPv4 for the station. A `/24` is assumed with
    /// the gateway at `.1`. Absent any static IP, the station uses DHCP.
    pub fn with_static_ip(mut self, ip: [u8; 4]) -> Self {
        self.static_ip = Some(ip);
        self
    }

    /// Assign a static IPv4 from the `BEET_WIFI_STATIC_IP` env var (dotted, eg
    /// `192.168.0.50`), which `build.rs` exposes from the local `.env`. Absent,
    /// the station falls back to DHCP. A `/24` is assumed with the gateway at
    /// `.1`.
    pub fn with_env_static_ip(mut self) -> Self {
        self.static_ip = option_env!("BEET_WIFI_STATIC_IP").map(parse_ipv4);
        self
    }
}

/// Parse a dotted IPv4 string (eg `192.168.0.50`) into octets. Panics on a
/// malformed address — the value is a compile-time `.env` literal, so a bad one
/// is a build-config error.
fn parse_ipv4(addr: &str) -> [u8; 4] {
    let mut octets = [0u8; 4];
    let mut parts = addr.split('.');
    for octet in octets.iter_mut() {
        *octet = parts
            .next()
            .and_then(|part| part.trim().parse().ok())
            .expect("WIFI_STATIC_IP must be a dotted IPv4 like 192.168.0.50");
    }
    octets
}

impl Plugin for WifiPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(WifiConfig {
            ssid: self.ssid,
            password: self.password,
            static_ip: self.static_ip,
        })
        .add_systems(Startup, start_wifi);

        // The server backend (beet's `HttpServer` component) needs the async
        // action layer to dispatch through `entity.exchange`, so it is gated on
        // `action`. Installs the backend + the request-drain system; spawning an
        // `HttpServer` then drives everything via beet's `on_add` hook.
        //
        // Both the server drain and the client `run_local` path go through
        // beet's async bridge, which needs `AsyncPlugin` (it inserts the
        // `AsyncWorld` resource) and `ActionPlugin`. `RouterPlugin` also adds
        // these, but a plain client/server app has no router, so add them here
        // too. `init_plugin` is idempotent, so it is safe when both are present.
        #[cfg(feature = "action")]
        {
            app.init_plugin::<ActionPlugin>()
                .init_plugin::<AsyncPlugin>()
                .add_plugins(http_server::http_server_plugin);
        }

        // The mDNS service browser: beet_net's agnostic browser engine
        // (`MdnsBrowserPlugin` + `handle_udp_packet` observer + `DiscoveredServices`)
        // plus this crate's embassy browser task and the `UdpPacket` queue drain.
        // Spawning an `MdnsBrowser { service_type }` entity then starts browsing
        // once the Stack is up. `mdns` implies `action`, so the engine's commands
        // and the drain's observers are available.
        #[cfg(feature = "mdns")]
        app.add_plugins(mdns_browser::mdns_browser_plugin);
    }
}

/// Station config, stashed by [`WifiPlugin`] for [`start_wifi`] to read: the AP
/// credentials plus an optional static IPv4 (`None` = DHCP).
#[derive(Resource, Clone)]
struct WifiConfig {
    ssid: &'static str,
    password: &'static str,
    static_ip: Option<[u8; 4]>,
}

/// Claim the `WIFI` peripheral, join the AP, start DHCP and the network task,
/// and spawn the client driver. Exclusive so it can pull the non-send peripheral
/// and [`Spawner`], and publish the resulting [`Stack`].
///
/// Runs in `Startup`, after `bring_up`'s `PreStartup` (so the chip, embassy and
/// the radio scheduler are already running).
fn start_wifi(world: &mut World) {
    let config = world.resource::<WifiConfig>().clone();
    let wifi = world
        .remove_non_send::<WIFI<'static>>()
        .expect("add Esp32Plugin before WifiPlugin — bring_up provides the WIFI peripheral");
    let spawner = *world.non_send::<Spawner>();

    let station_config = RadioConfig::Station(
        StationConfig::default()
            .with_ssid(config.ssid)
            .with_password(config.password.into()),
    );
    let (controller, interfaces) = esp_radio::wifi::new(
        wifi,
        ControllerConfig::default().with_initial_config(station_config),
    )
    .expect("failed to initialize Wi-Fi controller");

    // A static IP (a `/24` with the gateway at `.1`) brings the stack up at a
    // known address; otherwise DHCP assigns one. Configured here at station
    // bring-up so every socket — server, client, mDNS — sees the same address
    // from the start.
    let net_config = match config.static_ip {
        Some([a, b, c, d]) => embassy_net::Config::ipv4_static(StaticConfigV4 {
            address: Ipv4Cidr::new(Ipv4Address::new(a, b, c, d), 24),
            gateway: Some(Ipv4Address::new(a, b, c, 1)),
            dns_servers: Default::default(),
        }),
        None => embassy_net::Config::dhcpv4(Default::default()),
    };
    let rng = Rng::new();
    let seed = ((rng.random() as u64) << 32) | rng.random() as u64;

    // smoltcp socket budget. DHCP holds one for the stack's life; an HTTP server
    // accept loop and an HTTP client GET each take one; the `mdns` responder/
    // resolver task adds a persistent UDP socket. With mDNS active, a client GET
    // issued while the server is listening pushed past the old budget of 4 and
    // smoltcp panicked ("adding a socket to a full SocketSet"), so allow 6 — plus
    // one more for a live WebSocket client socket under `sockets`.
    #[cfg(feature = "sockets")]
    const STACK_SOCKETS: usize = 7;
    #[cfg(not(feature = "sockets"))]
    const STACK_SOCKETS: usize = 6;
    static RESOURCES: StaticCell<StackResources<STACK_SOCKETS>> = StaticCell::new();
    let resources = RESOURCES.init(StackResources::new());
    let (stack, runner) = embassy_net::new(interfaces.station, net_config, resources, seed);

    info!("Wi-Fi station joining `{}`", config.ssid);
    spawn_driver(spawner, connection_loop(controller));
    spawn_driver(spawner, net_loop(runner));
    spawn_driver(spawner, http_client::client_driver(stack));

    // Register the ESP32 transport so beet's `Request::send` routes here. It is
    // a one-time install; a second `WifiPlugin` (or a transport feature compiled
    // into beet_net) would already own it, so just warn rather than panic.
    if set_http_client(http_client::esp_send).is_err() {
        warn!("an HTTP transport was already installed; Request::send will not use Wi-Fi");
    }

    // The WebSocket client transport (mirrors the HTTP client): a driver owning
    // the Stack plus the `Socket::connect` hook. Gated on `sockets`.
    #[cfg(feature = "sockets")]
    {
        spawn_driver(spawner, socket_client::socket_driver(stack));
        if sockets::set_socket_client(socket_client::esp_connect).is_err() {
            warn!(
                "a WebSocket transport was already installed; Socket::connect will not use Wi-Fi"
            );
        }
    }

    // Stack is Copy and !Send; keep it as a non-send resource so each server
    // accept loop (spawned by the `on_add` hook) can hand a copy to its loop.
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
