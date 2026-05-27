//! Wi-Fi client + server, as a Bevy app.
//!
//! Mirrors the beet networking workflow on the ESP32, the same way
//! [`blinky`](../blinky.rs) mirrors an LED: [`WifiPlugin`] joins the AP named by
//! the `SSID`/`PASSWORD` env vars (from `.env`) and shares the network stack.
//!
//! - **Client:** [`ping`] spawns a driver that loops over
//!   `Request::get(..).send().await`, hitting Cloudflare's `1.1.1.1` and logging
//!   the status — beet's `Request` shape, driven by the ECS↔async bridge.
//! - **Server:** an [`HttpServer`] is spawned as a *component*; each request
//!   fires the [`ServerRequest`] observer ([`on_request`]), which here just logs.
//!   Hit it from the same LAN once it logs its IP: `curl http://<device-ip>:8080`
//!
//! Run with: `cargo run --release --example ecs-wifi`

#![no_std]
#![no_main]

use beet::prelude::*;
use beet_esp::prelude::*;
use defmt::info;
use defmt::warn;
use embassy_executor::Spawner;
use embassy_net::IpAddress;
use embassy_net::IpEndpoint;
use embassy_net::Ipv4Address;
use embassy_time::Duration;
use embassy_time::Timer;

const SSID: &str = env!("SSID");
const PASSWORD: &str = env!("PASSWORD");

#[beet_esp::main]
fn main() {
    App::new()
        .add_plugins((Esp32Plugin, WifiPlugin::new(SSID, PASSWORD)))
        .add_observer(on_request)
        .add_systems(Startup, (spawn_server, ping))
        .run();
}

/// Spawn the server as a component — [`WifiPlugin`] gives it an accept loop.
fn spawn_server(mut commands: Commands) {
    commands.spawn(HttpServer::new(8080));
}

/// Kick off the client: a driver that periodically GETs `1.1.1.1`.
///
/// Exclusive so it can pull the [`Spawner`]; this is the bridge's "spawn a
/// long-lived async driver" pattern, standing in for beet's `AsyncCommands`.
fn ping(world: &mut World) {
    let spawner = *world.non_send::<Spawner>();
    spawn_driver(spawner, async move {
        // Cloudflare DNS over plain HTTP on its anycast address — no DNS needed.
        let remote = IpEndpoint::new(IpAddress::Ipv4(Ipv4Address::new(1, 1, 1, 1)), 80);
        loop {
            match Request::get(remote).send().await {
                Ok(response) => info!("client GET 1.1.1.1 -> status {}", response.status()),
                Err(e) => warn!("client request failed: {:?}", e),
            }
            Timer::after(Duration::from_secs(15)).await;
        }
    });
}

/// Observer fired for every request the [`HttpServer`] handles.
fn on_request(request: On<ServerRequest>) {
    let request = request.event();
    info!(
        "server request #{} on :{} -> `{}`",
        request.count,
        request.port,
        request.line.as_str()
    );
}
