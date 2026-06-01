//! Alvik-specific message types and their mapping to ucPack frames.
//!
//! [`Command`]s flow ESP32 → STM32 ([`encode`](Command::encode) via the ucPack
//! [`Encoder`]); [`Status`] messages flow STM32 → ESP32
//! ([`decode`](Status::decode) from a ucPack [`Frame`]). Typed quantities are
//! converted to the wire's fixed units (deg, rpm, mm, mm/s, deg/s) here, at the
//! encode/decode boundary, and nowhere else.

use crate::alvik::types::Side;
use crate::alvik::ucpack::Encoder;
use crate::alvik::ucpack::Frame;
use crate::units::Angle;
use crate::units::AngularVelocity;
use crate::units::Distance;
use crate::units::LinearVelocity;

/// A command sent to the STM32 carrier.
#[derive(Debug, Clone, Copy, PartialEq, defmt::Format)]
pub enum Command {
    /// `B` — set behavior code.
    SetBehavior(u8),
    /// `R` — rotate in place by an angle.
    Rotate(Angle),
    /// `G` — move forward/back by a distance.
    Move(Distance),
    /// `J` — set both wheel speeds.
    SetWheelSpeeds {
        left: AngularVelocity,
        right: AngularVelocity,
    },
    /// `A` — set both wheel target angles.
    SetWheelAngles { left: Angle, right: Angle },
    /// `V` — drive by linear + angular velocity.
    Drive {
        linear: LinearVelocity,
        angular: AngularVelocity,
    },
    /// `S` — set servo A/B positions (0..=180°, sent as raw degrees).
    SetServo { a: u8, b: u8 },
    /// `L` — set the aggregated LED state byte.
    SetLeds(u8),
    /// `Z` — reset the robot pose.
    ResetPose {
        x: Distance,
        y: Distance,
        theta: Angle,
    },
    /// `W`+`Z` — reset a wheel's reference position.
    WheelReset { wheel: Side, position: Angle },
    /// `W`+`V` — set a single wheel's speed.
    WheelSpeed {
        wheel: Side,
        speed: AngularVelocity,
    },
    /// `W`+`P` — set a single wheel's target position.
    WheelPosition { wheel: Side, position: Angle },
    /// `P` — set a wheel's PID gains.
    SetPidGains {
        wheel: Side,
        kp: f32,
        ki: f32,
        kd: f32,
    },
    /// `X` — handshake / acknowledgement (payload is usually `'K'`).
    Ack(u8),
}

impl Command {
    /// Frame this command into `enc`'s scratch buffer, ready for the UART.
    pub fn encode<'a>(&self, enc: &'a mut Encoder) -> &'a [u8] {
        match *self {
            Command::SetBehavior(b) => enc.c1b(b'B', b),
            Command::Rotate(angle) => enc.c1f(b'R', angle.as_degrees()),
            Command::Move(distance) => enc.c1f(b'G', distance.as_millimeters()),
            Command::SetWheelSpeeds { left, right } => {
                enc.c2f(b'J', left.as_rpm(), right.as_rpm())
            }
            Command::SetWheelAngles { left, right } => {
                enc.c2f(b'A', left.as_degrees(), right.as_degrees())
            }
            Command::Drive { linear, angular } => {
                enc.c2f(b'V', linear.as_mm_per_sec(), angular.as_deg_per_sec())
            }
            Command::SetServo { a, b } => enc.c2b(b'S', a, b),
            Command::SetLeds(state) => enc.c1b(b'L', state),
            Command::ResetPose { x, y, theta } => enc.c3f(
                b'Z',
                x.as_millimeters(),
                y.as_millimeters(),
                theta.as_degrees(),
            ),
            Command::WheelReset { wheel, position } => {
                enc.c2b1f(b'W', wheel.label(), b'Z', position.as_degrees())
            }
            Command::WheelSpeed { wheel, speed } => {
                enc.c2b1f(b'W', wheel.label(), b'V', speed.as_rpm())
            }
            Command::WheelPosition { wheel, position } => {
                enc.c2b1f(b'W', wheel.label(), b'P', position.as_degrees())
            }
            Command::SetPidGains { wheel, kp, ki, kd } => {
                enc.c1b3f(b'P', wheel.label(), kp, ki, kd)
            }
            Command::Ack(b) => enc.c1b(b'X', b),
        }
    }
}

/// A status message received from the STM32 carrier.
#[derive(Debug, Clone, Copy, PartialEq, defmt::Format)]
pub enum Status {
    /// `j` — measured wheel speeds.
    WheelSpeeds {
        left: AngularVelocity,
        right: AngularVelocity,
    },
    /// `w` — measured wheel positions.
    WheelPositions { left: Angle, right: Angle },
    /// `l` — line sensors L, C, R (raw).
    Line { left: i16, center: i16, right: i16 },
    /// `c` — color sensor raw R, G, B.
    Color { red: i16, green: i16, blue: i16 },
    /// `i` — IMU `ax, ay, az, gx, gy, gz`.
    Imu([f32; 6]),
    /// `q` — orientation roll, pitch, yaw.
    Orientation {
        roll: Angle,
        pitch: Angle,
        yaw: Angle,
    },
    /// `p` — battery state of charge; `charging` is the sign of the wire value.
    Battery { percent: f32, charging: bool },
    /// `d` — ToF distances L, C, R.
    Tof {
        left: Distance,
        center: Distance,
        right: Distance,
    },
    /// `f` — ToF matrix L, CL, C, CR, R, top, bottom.
    TofMatrix {
        left: Distance,
        center_left: Distance,
        center: Distance,
        center_right: Distance,
        right: Distance,
        top: Distance,
        bottom: Distance,
    },
    /// `t` — touch button bitmask.
    Touch(u8),
    /// `m` — move/tilt bitmask.
    Motion(u8),
    /// `b` — current behavior code.
    Behavior(u8),
    /// `v` — robot linear + angular velocity.
    Velocity {
        linear: LinearVelocity,
        angular: AngularVelocity,
    },
    /// `z` — robot pose x, y, theta.
    Pose {
        x: Distance,
        y: Distance,
        theta: Angle,
    },
    /// `x` — acknowledgement byte.
    Ack(u8),
    /// `0x7E` — firmware version major, minor, patch.
    FwVersion { major: u8, minor: u8, patch: u8 },
}

impl Status {
    /// Decode a validated ucPack [`Frame`] into a [`Status`], or `None` for an
    /// unrecognised code.
    pub fn decode(frame: &Frame) -> Option<Status> {
        let status = match frame.code {
            b'j' => Status::WheelSpeeds {
                left: AngularVelocity::from_rpm(frame.f32(0)),
                right: AngularVelocity::from_rpm(frame.f32(4)),
            },
            b'w' => Status::WheelPositions {
                left: Angle::from_degrees(frame.f32(0)),
                right: Angle::from_degrees(frame.f32(4)),
            },
            b'l' => Status::Line {
                left: frame.i16(0),
                center: frame.i16(2),
                right: frame.i16(4),
            },
            b'c' => Status::Color {
                red: frame.i16(0),
                green: frame.i16(2),
                blue: frame.i16(4),
            },
            b'i' => Status::Imu([
                frame.f32(0),
                frame.f32(4),
                frame.f32(8),
                frame.f32(12),
                frame.f32(16),
                frame.f32(20),
            ]),
            b'q' => Status::Orientation {
                roll: Angle::from_degrees(frame.f32(0)),
                pitch: Angle::from_degrees(frame.f32(4)),
                yaw: Angle::from_degrees(frame.f32(8)),
            },
            b'p' => {
                let raw = frame.f32(0);
                Status::Battery {
                    percent: if raw < 0.0 { -raw } else { raw },
                    charging: raw > 0.0,
                }
            }
            b'd' => Status::Tof {
                left: Distance::from_millimeters(frame.i16(0) as f32),
                center: Distance::from_millimeters(frame.i16(2) as f32),
                right: Distance::from_millimeters(frame.i16(4) as f32),
            },
            b'f' => Status::TofMatrix {
                left: Distance::from_millimeters(frame.i16(0) as f32),
                center_left: Distance::from_millimeters(frame.i16(2) as f32),
                center: Distance::from_millimeters(frame.i16(4) as f32),
                center_right: Distance::from_millimeters(frame.i16(6) as f32),
                right: Distance::from_millimeters(frame.i16(8) as f32),
                top: Distance::from_millimeters(frame.i16(10) as f32),
                bottom: Distance::from_millimeters(frame.i16(12) as f32),
            },
            b't' => Status::Touch(frame.u8(0)),
            b'm' => Status::Motion(frame.u8(0)),
            b'b' => Status::Behavior(frame.u8(0)),
            b'v' => Status::Velocity {
                linear: LinearVelocity::from_mm_per_sec(frame.f32(0)),
                angular: AngularVelocity::from_deg_per_sec(frame.f32(4)),
            },
            b'z' => Status::Pose {
                x: Distance::from_millimeters(frame.f32(0)),
                y: Distance::from_millimeters(frame.f32(4)),
                theta: Angle::from_degrees(frame.f32(8)),
            },
            b'x' => Status::Ack(frame.u8(0)),
            0x7E => Status::FwVersion {
                major: frame.u8(0),
                minor: frame.u8(1),
                patch: frame.u8(2),
            },
            _ => return None,
        };
        Some(status)
    }
}
