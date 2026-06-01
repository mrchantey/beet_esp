//! Sweep the Alvik's two servos. Ported from `actuators/set_servo.py`.
//!
//! Each [`Servo`] child carries an [`Angle`] position (clamped 0..=180° on
//! flush); `AlvikPlugin` packs both into the `S` command when either changes.
//!
//! Run with: `cargo run --release --no-default-features --features alvik --example alvik-servo`

#![no_std]
#![no_main]

use beet::prelude::*;
use beet_esp::prelude::*;
use defmt::info;

#[beet_esp::main]
fn main() {
    App::new()
        .add_plugins((Esp32Plugin, HealthPlugin, AlvikPlugin))
        .add_systems(Update, sweep_servos)
        .run();
}

/// Step the servos through a sweep once a second; B mirrors A.
fn sweep_servos(
    time: Res<Time>,
    mut elapsed: Local<f32>,
    mut step: Local<usize>,
    mut servos: Query<&mut Servo>,
) {
    *elapsed += time.delta_secs();
    if *elapsed < 1.0 {
        return;
    }
    *elapsed = 0.0;

    let positions = [0.0, 45.0, 90.0, 135.0, 180.0, 135.0, 90.0, 45.0];
    let index = *step % positions.len();
    *step += 1;
    let a = positions[index];
    let b = 180.0 - a;
    info!("servo: A={}° B={}°", a, b);

    for mut servo in &mut servos {
        servo.position = match servo.id {
            ServoId::A => Angle::from_degrees(a),
            ServoId::B => Angle::from_degrees(b),
        };
    }
}
