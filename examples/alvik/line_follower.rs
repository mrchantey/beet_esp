//! Line follower: a proportional controller that reads the three line sensors
//! and steers the wheels. Ported from `demo/line_follower.py`, expressed as a
//! plain ECS control system.
//!
//! Reads [`LineSensors`] off the robot root, writes each wheel's
//! [`WheelTarget`], and tints the RGB LEDs by steering direction.
//!
//! Run with: `cargo run --release --no-default-features --features alvik --example alvik-line-follower`

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
        .add_systems(Update, follow_line)
        .run();
}

/// Base forward speed (rpm) and proportional gain.
const BASE_RPM: f32 = 30.0;
const KP: f32 = 20.0;
/// Max wheel speed to clamp the control output to.
const MAX_RPM: f32 = 60.0;

/// Weighted-centroid line follower, run at ~10 Hz.
fn follow_line(
    time: Res<Time>,
    mut elapsed: Local<f32>,
    line: Single<&LineSensors, With<AlvikRobot>>,
    mut wheels: Query<(&Wheel, &mut WheelTarget)>,
    mut leds: Query<(&AlvikLed, &mut LedColor)>,
) {
    *elapsed += time.delta_secs();
    if *elapsed < 0.1 {
        return;
    }
    *elapsed = 0.0;

    // Centroid in [-1, 1]: negative = line to the left, positive = to the right.
    let (left, center, right) = (
        line.left as f32,
        line.center as f32,
        line.right as f32,
    );
    let weight = left + center + right;
    let error = if weight != 0.0 {
        2.0 - (left + 2.0 * center + 3.0 * right) / weight
    } else {
        0.0
    };
    let control = (error * KP).clamp(-MAX_RPM, MAX_RPM);

    for (wheel, mut target) in &mut wheels {
        let rpm = match wheel.side {
            Side::Left => BASE_RPM - control,
            Side::Right => BASE_RPM + control,
        };
        *target = WheelTarget::Speed(AngularVelocity::from_rpm(rpm.clamp(-MAX_RPM, MAX_RPM)));
    }

    // Green when centred, red when steering hard.
    let color = if control.abs() > 0.2 {
        Color::srgb(1.0, 0.0, 0.0)
    } else {
        Color::srgb(0.0, 1.0, 0.0)
    };
    for (_, mut led) in &mut leds {
        led.0 = color;
    }
}
