//! beet wall-clock time over SNTP on the ESP32.
//!
//! [`WifiPlugin`] joins the AP, then [`ClockPlugin`] runs an SNTP client and,
//! once it gets a reply, installs beet's [`time_ext::now`] source. The
//! [`report_time`] system polls [`time_ext::try_now`] every couple of seconds:
//! before the sync it reports "still loading", and afterwards it logs real Unix
//! time — proving the `set_now`/`try_now` hook works end-to-end on hardware.
//!
//! Run with: `cargo run --release --no-default-features --features clock --example clock`

#![no_std]
#![no_main]

use beet::prelude::*;
use beet_esp::prelude::*;
use embassy_time::Duration;
use embassy_time::Instant;


#[beet_esp::main]
fn main() {
    App::new()
        .add_plugins((
            Esp32Plugin,
            HealthPlugin,
            WifiPlugin::from_env(),
            ClockPlugin::default(),
        ))
        .add_systems(Update, report_time)
        .run();
}

/// Log the wall clock every ~2s (throttled off the monotonic clock so we don't
/// spam every tick). `try_now` errors until the first SNTP sync lands.
fn report_time(mut last: Local<Option<Instant>>) {
    let mono = Instant::now();
    let due = last.is_none_or(|t| mono.duration_since(t) >= Duration::from_secs(2));
    if !due {
        return;
    }
    *last = Some(mono);

    match time_ext::try_now() {
        Ok(d) => info!(
            "wall clock: {} unix ms ({} s since epoch)",
            d.as_millis() as u64,
            d.as_secs()
        ),
        Err(_) => info!("clock still loading (SNTP not synced yet)"),
    }
}
