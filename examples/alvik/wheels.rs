//! Drive the Alvik's wheels by speed and log the measured speed back. Combines
//! upstream `actuators/move_wheels.py` + `wheels_speed.py`.
//!
//! `WheelTarget` is the command component on each wheel child; mutating it
//! queues a per-wheel speed command. `WheelState` carries the measured speed
//! and position back from the `j` / `w` status.
//!
//! Run with: `cargo run --release --no-default-features --features alvik --example alvik-wheels`

#![no_std]
#![no_main]

use beet::prelude::*;
use beet_esp::prelude::*;
use defmt::Debug2Format;
use defmt::info;

#[beet_esp::main]
fn main() {
    App::new()
        .add_plugins((Esp32Plugin, HealthPlugin, AlvikPlugin))
        .add_systems(Update, (set_target_speed, log_measured_speed))
        .run();
}

/// Every three seconds, step both wheels to the next target speed (rpm).
fn set_target_speed(
    time: Res<Time>,
    mut elapsed: Local<f32>,
    mut step: Local<usize>,
    mut wheels: Query<&mut WheelTarget, With<Wheel>>,
) {
    *elapsed += time.delta_secs();
    if *elapsed < 3.0 {
        return;
    }
    *elapsed = 0.0;

    let speeds = [10.0, 30.0, -20.0, 0.0];
    let rpm = speeds[*step % speeds.len()];
    *step += 1;
    info!("wheels: target {} rpm", rpm);

    for mut target in &mut wheels {
        *target = WheelTarget::Speed(AngularVelocity::from_rpm(rpm));
    }
}

/// Log each wheel's measured speed twice a second.
fn log_measured_speed(
    time: Res<Time>,
    mut elapsed: Local<f32>,
    wheels: Query<(&Wheel, &WheelState)>,
) {
    *elapsed += time.delta_secs();
    if *elapsed < 0.5 {
        return;
    }
    *elapsed = 0.0;

    for (wheel, state) in &wheels {
        info!(
            "wheel {} measured {} rpm",
            Debug2Format(&wheel.side),
            state.speed.as_rpm()
        );
    }
}
