//! Wi-Fi client + server, as a Bevy app, using beet's networking types.
//!
//! Mirrors the beet networking workflow on the ESP32, the same way
//! [`blinky`](../blinky.rs) mirrors an LED: [`WifiPlugin`] joins the AP named by
//! the `SSID`/`PASSWORD` env vars (from `.env`) and shares the network stack.
//!
//! - **Client:** [`ping`] spawns a driver that loops over beet's
//!   `Request::get(url).send().await`, hitting `http://example.com` (resolved via
//!   DHCP-provided DNS) and logging the status — the exact same API as beet's
//!   `examples/net/http_client.rs`, with the ESP32 transport installed by
//!   [`WifiPlugin`].
//! - **Server:** an [`HttpServer`] is registered with a handler system via
//!   [`add_http_server`](WifiAppExt::add_http_server), mirroring beet's
//!   `HttpServer` + `Action::<Request, Response>` pattern. Each request runs
//!   [`handle`] on the ECS and returns a beet [`Response`].
//!   Hit it from the same LAN once it logs its IP: `curl http://<device-ip>:8080`
//!
//! Run with: `cargo run --release --no-default-features --features wifi --example ecs_wifi`

#![no_std]
#![no_main]

use beet::prelude::*;
use beet_esp::prelude::*;
use defmt::info;
use defmt::warn;
use embassy_executor::Spawner;
use embassy_time::Duration;
use embassy_time::Timer;

const SSID: &str = env!("SSID");
const PASSWORD: &str = env!("PASSWORD");

#[beet_esp::main]
fn main() {
    App::new()
        .add_plugins((Esp32Plugin, HealthPlugin, WifiPlugin::new(SSID, PASSWORD)))
        .init_resource::<Visits>()
        // Register the server + its handler, beet-style.
        .add_http_server(8080, handle)
        .add_systems(Startup, ping)
        .run();
}

/// Visitor counter, bumped by the server [`handle`]r — the ECS state a beet
/// `Action` handler would query.
#[derive(Resource, Default)]
struct Visits(u32);

/// Server handler: a normal Bevy system taking the incoming [`Request`] and
/// returning a [`Response`], just like beet's `Action::<Request, Response>`.
fn handle(request: In<Request>, mut visits: ResMut<Visits>) -> Response {
    visits.0 += 1;
    info!(
        "server request #{} -> {}",
        visits.0,
        request.0.path_string().as_str()
    );
    Response::ok_body(
        format!("hello from the beet_esp ECS server\nyou are visitor #{}\n", visits.0),
        MediaType::Text,
    )
}

/// Kick off the client: a driver that periodically GETs `example.com`.
///
/// Exclusive so it can pull the [`Spawner`]; this is the bridge's "spawn a
/// long-lived async driver" pattern, standing in for beet's `AsyncCommands`.
fn ping(world: &mut World) {
    let spawner = *world.non_send::<Spawner>();
    spawn_driver(spawner, async move {
        loop {
            match Request::get("http://example.com").send().await {
                Ok(response) => {
                    info!("client GET example.com -> status {}", response.status().as_u16());
                }
                Err(e) => warn!("client request failed: {}", defmt::Debug2Format(&e)),
            }
            Timer::after(Duration::from_secs(15)).await;
        }
    });
}
