//! Wi-Fi HTTP client, as a Bevy app, using beet's networking types.
//!
//! Mirrors beet's `examples/net/http_client.rs` on the ESP32, the same way
//! [`blinky`](../blinky.rs) mirrors an LED: [`WifiPlugin`] joins the AP named by
//! the `SSID`/`PASSWORD` env vars (from `.env`), installs the ESP32 transport,
//! and shares the network stack. [`ping`] then loops over
//! `Request::get(url).send().await`, hitting `http://example.com` (resolved via
//! DHCP-provided DNS) and logging the status — the exact same API as beet's std
//! client, no `action` feature needed.
//!
//! Run with: `cargo run --release --no-default-features --features wifi --example http_client`

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
        .add_systems(Startup, ping)
        .run();
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
