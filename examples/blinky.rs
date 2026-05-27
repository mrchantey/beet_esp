//! Addressable RGB LED hue-fade ("blinky"), as a Bevy app.
//!
//! [`init_esp!`] hides the board bring-up and hands back the on-board WS2812;
//! the app takes it as a non-send resource. [`LedPlugin`] spawns an LED entity
//! and advances its [`HueFade`] each `Update`, writing the colour to the
//! entity's [`LedColor`]. [`Esp32Plugin`]'s runner drives the schedule on the
//! embassy executor and pushes that colour to the LED over RMT — so this is
//! just a Bevy app with `App::new()...run()`.
//!
//! The on-board LED is `GPIO48` on the official Espressif DevKitC-1 / DevKitM-1;
//! some clone boards use `GPIO38` — see `init_esp!` if nothing lights up.
//!
//! Run with: `cargo run --release --example blinky`

#![no_std]
#![no_main]

use beet::prelude::*;
use beet_esp::prelude::*;
use panic_rtt_target as _;

beet_esp::esp_app_desc!();

#[esp_hal::main]
fn main() -> ! {
    init_esp!(led);

    App::new()
        .insert_non_send(led)
        .add_plugins((Esp32Plugin, LedPlugin::default()))
        .run();

    unreachable!("the esp runner never returns")
}
