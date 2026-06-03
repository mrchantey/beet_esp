//! Wi-Fi HTTP client as a Bevy app, using beet's networking types.
//!
//! [`WifiPlugin`] joins the AP named by the `WIFI_SSID`/`WIFI_PASSWORD` env vars,
//! installs the ESP32 transport and (under `action`) the background request
//! driver. A `Startup` ping plus a throttled `Update` poll each issue a one-shot
//! `Request::get(url).send().await` from the Bevy async layer.
//!
//! Run with: `cargo run --release --no-default-features --features wifi,action --example http_client`

#![no_std]
#![no_main]

use beet::prelude::*;
use beet_esp::prelude::*;
use defmt::info;
use defmt::warn;


/// How often [`poll_example_com`] issues its periodic request.
const POLL_SECS: u64 = 10;

#[beet_esp::main]
fn main() {
    App::new()
        .add_plugins((Esp32Plugin, HealthPlugin, WifiPlugin::from_env()))
        .add_systems(Startup, ping_example_com)
        .add_systems(Update, poll_example_com)
        .run();
}

fn ping_example_com(commands: AsyncCommands) {
    get_example_com(commands);
}

fn poll_example_com(
    commands: AsyncCommands,
    time: Res<Time>,
    mut timer: Local<Option<Timer>>,
) {
    let timer = timer
        .get_or_insert_with(|| Timer::new(Duration::from_secs(POLL_SECS), TimerMode::Repeating));
    if timer.tick(time.delta()).just_finished() {
        get_example_com(commands);
    }
}

fn get_example_com(commands: AsyncCommands) {
    commands.run_local(async move |_world: AsyncWorld| {
        match Request::get("http://example.com").send().await {
            Ok(response) => {
                info!(
                    "client GET example.com -> status {}",
                    response.status().as_u16()
                );
            }
            Err(e) => warn!("client request failed: {}", defmt::Debug2Format(&e)),
        }
    });
}
