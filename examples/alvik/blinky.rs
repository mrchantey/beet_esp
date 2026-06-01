//! Alvik "hello world": cycle the two RGB UI LEDs and toggle the illuminator
//! and built-in LED. Mirrors upstream `actuators/leds_setting.py`.
//!
//! The LEDs are plain [`LedColor`] components on the `AlvikLed` child entities
//! that [`spawn_robot`] creates; `AlvikPlugin`'s flush aggregates them into the
//! single wire byte. The illuminator / built-in LED are bool components on the
//! robot root.
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
        .add_systems(Startup, |mut commands: Commands| {
            spawn_robot(&mut commands);
        })
        .add_systems(Update, animate_leds)
        .run();
}

/// One step every second: walk both RGB LEDs through a colour palette and blink
/// the illuminator / built-in LED in step.
fn animate_leds(
    time: Res<Time>,
    mut elapsed: Local<f32>,
    mut step: Local<usize>,
    mut leds: Query<(&AlvikLed, &mut LedColor)>,
    robot: Single<(&mut Illuminator, &mut BuiltinLed), With<AlvikRobot>>,
) {
    *elapsed += time.delta_secs();
    if *elapsed < 1.0 {
        return;
    }
    *elapsed = 0.0;

    let palette = [
        Color::srgb(1.0, 0.0, 0.0),
        Color::srgb(0.0, 1.0, 0.0),
        Color::srgb(0.0, 0.0, 1.0),
        Color::WHITE,
    ];
    let index = *step % palette.len();
    *step += 1;

    for (led, mut color) in &mut leds {
        // Offset the right LED so the two are never the same colour.
        color.0 = match led.side {
            Side::Left => palette[index],
            Side::Right => palette[(index + 2) % palette.len()],
        };
    }

    let (mut illuminator, mut builtin) = robot.into_inner();
    illuminator.0 = index % 2 == 0;
    builtin.0 = index % 2 == 1;
}
