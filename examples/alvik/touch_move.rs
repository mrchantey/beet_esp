//! Respond to touch and motion as ECS observer events. Reframes upstream
//! `events/*.py` + `demo/touch_move.py`: instead of registering callbacks you
//! add observers for [`TouchPressed`], [`Shaken`], [`Lifted`], [`Dropped`] and
//! [`Tilted`], which `AlvikPlugin` triggers on each edge.
//!
//! Run with: `cargo run --release --no-default-features --features alvik --example alvik-touch-move`

#![no_std]
#![no_main]

use beet::prelude::*;
use beet_esp::prelude::*;

#[beet_esp::main]
fn main() {
    App::new()
        .add_plugins((Esp32Plugin, HealthPlugin, AlvikPlugin))
        .add_observer(on_touch)
        .add_observer(on_shake)
        .add_observer(on_lift)
        .add_observer(on_drop)
        .add_observer(on_tilt)
        .run();
}

/// Light both LEDs blue on any touch and log which button it was.
fn on_touch(ev: On<TouchPressed>, mut leds: Query<&mut LedColor, With<AlvikLed>>) {
    info!("touch: {:?}", ev.event().0);
    for mut led in &mut leds {
        led.0 = Color::srgb(0.0, 0.0, 1.0);
    }
}

fn on_shake(_: On<Shaken>) {
    info!("motion: shaken!");
}

fn on_lift(_: On<Lifted>) {
    info!("motion: lifted");
}

fn on_drop(_: On<Dropped>) {
    info!("motion: dropped");
}

fn on_tilt(ev: On<Tilted>) {
    info!("motion: tilted {:?}", ev.event().0);
}
