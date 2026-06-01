//! Read and log the Alvik's sensors: line, color (+label), ToF, IMU,
//! orientation, battery and touch. Consolidates the upstream `sensors/*.py`
//! readers into one example, logged twice a second over defmt/RTT.
//!
//! Run with: `cargo run --release --no-default-features --features alvik --example alvik-sensors`

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
        .add_systems(Update, log_sensors)
        .run();
}

/// The robot-root sensor components this example reads.
#[derive(QueryData)]
struct Sensors {
    line: &'static LineSensors,
    color: &'static ColorSensor,
    tof: &'static Tof,
    imu: &'static Imu,
    orientation: &'static Orientation,
    battery: &'static BatterState,
    touch: &'static TouchValue,
    connected: &'static Connected,
}

/// Log the full sensor set every half second.
fn log_sensors(time: Res<Time>, mut elapsed: Local<f32>, robot: Single<Sensors, With<AlvikRobot>>) {
    *elapsed += time.delta_secs();
    if *elapsed < 0.5 {
        return;
    }
    *elapsed = 0.0;

    if !robot.connected.0 {
        info!("alvik: not connected yet...");
        return;
    }

    info!(
        "LINE  l={} c={} r={}",
        robot.line.left, robot.line.center, robot.line.right
    );
    info!(
        "COLOR raw=({}, {}, {}) label={}",
        robot.color.raw().0,
        robot.color.raw().1,
        robot.color.raw().2,
        Debug2Format(&color_to_label(&robot.color.as_color()))
    );
    info!(
        "TOF   l={}mm c={}mm r={}mm",
        robot.tof.left.as_millimeters(),
        robot.tof.center.as_millimeters(),
        robot.tof.right.as_millimeters()
    );
    info!(
        "IMU   accel=({}, {}, {}) gyro=({}, {}, {})",
        robot.imu.accel.x,
        robot.imu.accel.y,
        robot.imu.accel.z,
        robot.imu.gyro.x,
        robot.imu.gyro.y,
        robot.imu.gyro.z
    );
    let euler = robot.orientation.0.to_euler(EulerRot::XYZ);
    info!(
        "ORIENT roll={} pitch={} yaw={} (rad)",
        euler.0, euler.1, euler.2
    );
    info!(
        "BATT  {}% charging={}",
        robot.battery.percent, robot.battery.charging
    );
    info!("TOUCH bits={:08b}", robot.touch.0);
}
