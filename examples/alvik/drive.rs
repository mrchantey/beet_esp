//! Drive the Alvik by linear + angular velocity, and issue one-shot move /
//! rotate commands. Ported from `actuators/pose_example.py`.
//!
//! [`DifferentialDrive`] on the robot root is the continuous velocity command;
//! [`Command::Move`] / [`Command::Rotate`] are one-shot commands pushed straight
//! onto [`ALVIK_OUT`]. The robot's measured [`RobotPose`] is logged back.
//!
//! Run with: `cargo run --release --no-default-features --features alvik --example alvik-drive`

#![no_std]
#![no_main]

use beet::prelude::*;
use beet_esp::prelude::*;
// Explicit to disambiguate from bevy's `Command` trait (both are in scope via
// the glob preludes above).
use beet_esp::alvik::protocol::Command;

#[beet_esp::main]
fn main() {
    App::new()
        .add_plugins((Esp32Plugin, HealthPlugin, AlvikPlugin))
        .add_systems(Update, (step_drive, log_pose))
        .run();
}

/// Walk through a little manoeuvre every four seconds: forward, arc, spin, stop.
fn step_drive(
    time: Res<Time>,
    mut elapsed: Local<f32>,
    mut step: Local<usize>,
    mut drive: Single<&mut DifferentialDrive, With<AlvikRobot>>,
) {
    *elapsed += time.delta_secs();
    if *elapsed < 4.0 {
        return;
    }
    *elapsed = 0.0;

    match *step % 4 {
        0 => {
            info!("drive: forward 50 mm/s");
            drive.linear = LinearVelocity::from_mm_per_sec(50.0);
            drive.angular = AngularVelocity::from_deg_per_sec(0.0);
        }
        1 => {
            info!("drive: arc");
            drive.linear = LinearVelocity::from_mm_per_sec(40.0);
            drive.angular = AngularVelocity::from_deg_per_sec(45.0);
        }
        2 => {
            info!("drive: spin in place");
            drive.linear = LinearVelocity::from_mm_per_sec(0.0);
            drive.angular = AngularVelocity::from_deg_per_sec(90.0);
        }
        _ => {
            info!("drive: stop, then rotate 90 deg");
            drive.linear = LinearVelocity::from_mm_per_sec(0.0);
            drive.angular = AngularVelocity::from_deg_per_sec(0.0);
            // One-shot rotate command straight onto the out queue.
            let _ = ALVIK_OUT.send(Command::Rotate(Angle::from_degrees(90.0)));
        }
    }
    *step += 1;
}

/// Log the measured pose once a second.
fn log_pose(
    time: Res<Time>,
    mut elapsed: Local<f32>,
    pose: Single<&RobotPose, With<AlvikRobot>>,
) {
    *elapsed += time.delta_secs();
    if *elapsed < 1.0 {
        return;
    }
    *elapsed = 0.0;
    info!(
        "pose x={}mm y={}mm theta={}rad",
        pose.0.position.x,
        pose.0.position.y,
        pose.0.yaw()
    );
}
