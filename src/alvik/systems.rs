//! Per-frame transport systems: flush command components to [`ALVIK_OUT`] and
//! apply incoming status from [`ALVIK_STATE`] / [`ALVIK_EVENTS`] to the sensor
//! and state components. Continuous state rides the snapshot; discrete messages
//! (touch, move, behavior, ack, firmware) ride the event queue.

use crate::alvik::components::*;
use crate::alvik::driver::ALVIK_EVENTS;
use crate::alvik::driver::ALVIK_OUT;
use crate::alvik::driver::ALVIK_STATE;
use crate::alvik::protocol::Command;
use crate::alvik::protocol::Status;
use crate::alvik::types::Side;
use crate::led::LedColor;
use crate::units::AngularVelocity;
use crate::units::LinearVelocity;
use beet::prelude::*;

/// Queue a per-wheel speed/position command whenever a [`WheelTarget`] changes.
pub fn flush_wheels(query: Populated<(&Wheel, &WheelTarget), Changed<WheelTarget>>) {
    for (wheel, target) in &query {
        let command = match *target {
            WheelTarget::Speed(speed) => Command::WheelSpeed {
                wheel: wheel.side,
                speed,
            },
            WheelTarget::Position(position) => Command::WheelPosition {
                wheel: wheel.side,
                position,
            },
        };
        let _ = ALVIK_OUT.send(command);
    }
}

/// Queue a drive command whenever the [`DifferentialDrive`] changes.
pub fn flush_drive(query: Populated<&DifferentialDrive, Changed<DifferentialDrive>>) {
    for drive in &query {
        let _ = ALVIK_OUT.send(Command::Drive {
            linear: drive.linear,
            angular: drive.angular,
        });
    }
}

/// Compose both servo positions into one `S` command when either changes.
/// Aggregated (not per-entity) because the wire packet carries A and B together.
pub fn flush_servos(mut last: Local<Option<(u8, u8)>>, servos: Populated<&Servo>) {
    let mut positions = (None, None);
    for servo in &servos {
        match servo.id {
            ServoId::A => positions.0 = Some(servo.degrees()),
            ServoId::B => positions.1 = Some(servo.degrees()),
        }
    }
    if let (Some(a), Some(b)) = positions {
        if *last != Some((a, b)) {
            let _ = ALVIK_OUT.send(Command::SetServo { a, b });
            *last = Some((a, b));
        }
    }
}

/// Aggregate the builtin LED, illuminator and both RGB UI LEDs into the single
/// `L` state byte and queue it when it changes. Each RGB channel thresholds its
/// [`LedColor`] to on/off. Skips entirely when the app drives no Alvik LED, so
/// the illuminator set during bring-up is left untouched.
pub fn flush_alvik_leds(
    mut last: Local<Option<u8>>,
    robot: Single<(Option<&Illuminator>, Option<&BuiltinLed>), With<AlvikRobot>>,
    leds: Query<(&AlvikLed, &LedColor)>,
) {
    let (illuminator, builtin) = *robot;
    if leds.is_empty() && illuminator.is_none() && builtin.is_none() {
        return;
    }
    let mut byte = 0u8;
    if builtin.map(|b| b.0).unwrap_or(false) {
        byte |= 0b0000_0001;
    }
    if illuminator.map(|i| i.0).unwrap_or(false) {
        byte |= 0b0000_0010;
    }
    for (led, color) in &leds {
        let srgb = color.0.to_srgba();
        let (red, green, blue) = match led.side {
            Side::Left => (0b0000_0100, 0b0000_1000, 0b0001_0000),
            Side::Right => (0b0010_0000, 0b0100_0000, 0b1000_0000),
        };
        if srgb.red > 0.5 {
            byte |= red;
        }
        if srgb.green > 0.5 {
            byte |= green;
        }
        if srgb.blue > 0.5 {
            byte |= blue;
        }
    }
    if *last != Some(byte) {
        let _ = ALVIK_OUT.send(Command::SetLeds(byte));
        *last = Some(byte);
    }
}

/// Apply the latest continuous sensor snapshot and drain discrete status events
/// onto the robot's components. All components are optional, so an app spawns
/// only the ones it reads.
pub fn apply_status(
    robot: Single<AlvikComponents, With<AlvikRobot>>,
    mut wheels: Query<(&Wheel, &mut WheelState)>,
) {
    let mut robot = robot;
    if let Some(snapshot) = ALVIK_STATE.try_recv() {
        if let Some(pose) = robot.pose.as_mut() {
            pose.0 = Pose::from_xy_theta(
                snapshot.pose.0.as_millimeters(),
                snapshot.pose.1.as_millimeters(),
                snapshot.pose.2.as_radians(),
            );
        }
        if let Some(imu) = robot.imu.as_mut() {
            imu.accel = Vec3::new(snapshot.imu[0], snapshot.imu[1], snapshot.imu[2]);
            imu.gyro = Vec3::new(snapshot.imu[3], snapshot.imu[4], snapshot.imu[5]);
        }
        if let Some(orientation) = robot.orientation.as_mut() {
            let (roll, pitch, yaw) = snapshot.orientation;
            orientation.0 = Quat::from_euler(
                EulerRot::XYZ,
                roll.as_radians(),
                pitch.as_radians(),
                yaw.as_radians(),
            );
        }
        if let Some(line) = robot.line.as_mut() {
            line.left = snapshot.line.0;
            line.center = snapshot.line.1;
            line.right = snapshot.line.2;
        }
        if let Some(color) = robot.color.as_mut() {
            color.raw = snapshot.color;
            color.color = color.normalize_color();
        }
        if let Some(tof) = robot.tof.as_mut() {
            tof.left = snapshot.tof_left;
            tof.center_left = snapshot.tof_center_left;
            tof.center = snapshot.tof_center;
            tof.center_right = snapshot.tof_center_right;
            tof.right = snapshot.tof_right;
            tof.top = snapshot.tof_top;
            tof.bottom = snapshot.tof_bottom;
        }
        if let Some(linear) = robot.linear.as_mut() {
            **linear = snapshot.velocity.0;
        }
        if let Some(angular) = robot.angular.as_mut() {
            **angular = snapshot.velocity.1;
        }
        if let Some(battery) = robot.battery.as_mut() {
            battery.percent = snapshot.battery.0;
            battery.charging = snapshot.battery.1;
        }
        for (wheel, mut state) in &mut wheels {
            match wheel.side {
                Side::Left => {
                    state.speed = snapshot.wheel_left_speed;
                    state.position = snapshot.wheel_left_position;
                }
                Side::Right => {
                    state.speed = snapshot.wheel_right_speed;
                    state.position = snapshot.wheel_right_position;
                }
            }
        }
    }

    while let Some(status) = ALVIK_EVENTS.try_recv() {
        match status {
            Status::Touch(bits) => {
                if let Some(touch) = robot.touch.as_mut() {
                    touch.0 = bits;
                }
            }
            Status::Motion(bits) => {
                if let Some(motion) = robot.motion.as_mut() {
                    motion.0 = bits;
                }
            }
            Status::Behavior(code) => {
                if let Some(behavior) = robot.behavior.as_mut() {
                    behavior.0 = code;
                }
            }
            Status::FwVersion {
                major,
                minor,
                patch,
            } => {
                if let Some(version) = robot.firmware.as_mut() {
                    version.major = major;
                    version.minor = minor;
                    version.patch = patch;
                }
                if let Some(connected) = robot.connected.as_mut() {
                    connected.0 = true;
                }
            }
            _ => {}
        }
    }
}

/// The robot-root components [`apply_status`] writes, each optional so an app
/// spawns only what it uses. A [`QueryData`] struct keeps the otherwise huge
/// tuple readable.
#[derive(QueryData)]
#[query_data(mutable)]
pub struct AlvikComponents {
    pose: Option<&'static mut RobotPose>,
    imu: Option<&'static mut Imu>,
    orientation: Option<&'static mut Orientation>,
    line: Option<&'static mut LineSensors>,
    color: Option<&'static mut ColorSensor>,
    tof: Option<&'static mut Tof>,
    linear: Option<&'static mut LinearVelocity>,
    angular: Option<&'static mut AngularVelocity>,
    battery: Option<&'static mut BatterState>,
    touch: Option<&'static mut TouchValue>,
    motion: Option<&'static mut MotionValue>,
    behavior: Option<&'static mut BehaviorCode>,
    connected: Option<&'static mut Connected>,
    firmware: Option<&'static mut FirmwareVersion>,
}
