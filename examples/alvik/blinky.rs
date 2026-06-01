//! Alvik "hello world": fade the two RGB UI LEDs smoothly around the hue wheel,
//! the Alvik counterpart to the on-board WS2812 [`blinky`](../blinky.rs) example.
//!
//! The LEDs are plain [`LedColor`] components on the `AlvikLed` child entities
//! that `AlvikPlugin` spawns; its flush aggregates them into the single wire
//! byte, thresholding each channel, so the fade lands on the eight colours the
//! UI LEDs can show.
//!
//! Run with: `cargo run --release --no-default-features --features alvik --example alvik-blinky`

#![no_std]
#![no_main]

use beet::prelude::*;
use beet_esp::prelude::*;

#[beet_esp::main]
fn main() {
    App::new()
        .add_plugins((Esp32Plugin, HealthPlugin, AlvikPlugin))
        .add_systems(Update, fade_leds)
        .run();
}

/// Advance a shared hue each frame and paint both RGB LEDs from it, the right
/// LED offset half a turn so the two stay complementary.
fn fade_leds(
    time: Res<Time>,
    mut hue: Local<f32>,
    mut leds: Query<(&AlvikLed, &mut LedColor)>,
) {
    *hue = (*hue + time.delta_secs() * 60.0) % 360.0;
    for (led, mut color) in &mut leds {
        let hue = match led.side {
            Side::Left => *hue,
            Side::Right => (*hue + 180.0) % 360.0,
        };
        color.0 = Color::hsl(hue, 1.0, 0.5);
    }
}
