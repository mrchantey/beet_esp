//! Addressable RGB LED hue-fade ("blinky"), as a Bevy app.
//!
//! [`#[beet_esp::main]`](beet_esp::main) hides the ESP32 entry boilerplate.
//! [`Esp32Plugin`] brings up the chip and embassy and drives the schedule on the
//! executor; [`LedPlugin`] claims the on-board WS2812 and spawns its async RMT
//! driver. The example owns the rest: `setup` spawns an LED entity and
//! [`cycle_hue`]/[`flush_led`] advance its [`HueFade`] each `Update` — so this is
//! just a Bevy app with `App::new()...run()`.
//!
//! The on-board LED is `GPIO48` on the official Espressif DevKitC-1 / DevKitM-1;
//! some clone boards use `GPIO38` — see [`LedPlugin`] if nothing lights up.
//!
//! Run with: `cargo run --release --example blinky`

#![no_std]
#![no_main]

use beet::prelude::*;
use beet_esp::prelude::*;

#[beet_esp::main]
fn main() {
    App::new()
        .add_plugins((Esp32Plugin, LedPlugin))
        .add_systems(Startup, setup)
        .run();
}

/// Spawn the on-board LED entity with a hue-fade animation. [`LedPlugin`] spawns
/// the async driver; this creates the entity its [`Update`] systems drive.
fn setup(mut commands: Commands) {
    commands.spawn((LedColor::default(), HueFade::default()));
}
