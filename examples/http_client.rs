//! Wi-Fi HTTP client, as a Bevy app, using beet's networking types.
//!
//! Mirrors beet's `examples/net/http_client.rs` on the ESP32, the same way
//! [`blinky`](../blinky.rs) mirrors an LED: [`WifiPlugin`] joins the AP named by
//! the `SSID`/`PASSWORD` env vars (from `.env`), installs the ESP32 transport,
//! shares the network stack, and (under `action`) spawns the background
//! `client_driver` that services the request bridge.
//!
//! [`ping`] then issues a one-shot `Request::get(url).send().await` straight from
//! the Bevy async layer via [`AsyncCommands::run_local`]. No per-request driver
//! and no manual bridge handling: the request crosses to the background
//! `client_driver` (an embassy task) and the awaited [`Response`] comes back —
//! the exact same API as beet's std client, with the `action` feature enabling
//! the async layer.
//!
//! ## Why `run_local` and why one-shot
//!
//! `run_local` is the `!Send` variant: the esp transport future is `!Send` on
//! this target, so it must stay on the local (single-core) bevy pool.
//!
//! Caveat — the bevy pool only ever advances futures that await the `World`. A
//! future that awaits *silicon* (an embassy `Timer::after`, a raw `TcpSocket`
//! read) does **not** progress there. `.send().await` works because it parks on
//! the request bridge, which the embassy-side `client_driver` services and wakes
//! across the executor boundary (the same cross-executor wakeup the server drain
//! relies on). But a `Timer::after` inside a `run_local` task would never fire,
//! so you cannot build a periodic loop that *sleeps* on the bevy pool.
//!
//! ## Periodic polling: throttle the *system*, not the *task*
//!
//! Because a `run_local` task cannot itself sleep, the idiomatic periodic poll
//! keeps the timing on the Bevy side: a normal `Update` system, run every frame,
//! checks elapsed time and fires a fresh one-shot `run_local` request only when
//! due — exactly how [`HealthPlugin`](beet_esp::health) throttles its report.
//! The system never blocks; each request is still a one-shot that bridges to the
//! background `client_driver`. See [`poll_example_com`] below. (You *could*
//! instead put the whole loop on an embassy driver calling `.send().await` with
//! an embassy `Timer::after` between requests, but for ECS-driven polling the
//! Bevy-system throttle is simpler and keeps the request on the world side.)
//!
//! Run with: `cargo run --release --no-default-features --features wifi,action --example http_client`
//!
//! The `action` feature is required: `run_local`/[`AsyncCommands`] are the Bevy
//! async layer, which on this no_std target is only compiled in (and its task
//! spawner only installed, see [`Esp32Plugin`](beet_esp::esp32_plugin)) under
//! `action`.

#![no_std]
#![no_main]

use beet::prelude::*;
use beet_esp::prelude::*;
use defmt::info;
use defmt::warn;


/// How often [`poll_example_com`] issues its periodic request.
const POLL_SECS: u64 = 10;

// The World/registry/request bulk lives in PSRAM now (see `beet_esp::mem`), so
// internal SRAM only has to hold the radio + DMA + stack. The default 64 KiB
// internal reserve (+ the ~72 KiB reclaimed region) is plenty.
#[beet_esp::main(internal_reserve_kb = 64)]
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
