//! Bevy components for the Alvik: the robot root, its wheels / servos / LEDs,
//! and the sensor readouts. All Alvik data is a component — systems in
//! [`systems`](super::systems) flush command components to the wire and apply
//! incoming status to the sensor components.

use crate::alvik::types::Side;
use crate::units::Angle;
use crate::units::AngularVelocity;
use crate::units::Distance;
use crate::units::LinearVelocity;
use beet::prelude::*;

/// Marker for the Alvik robot root entity. Its wheels, servos and LEDs are
/// child entities; its own continuous state (battery, behavior, pose, sensors)
/// rides on sibling components.
#[derive(Component, Default, Clone, Copy)]
pub struct AlvikRobot;

/// Whether the bring-up handshake has completed and the link is live.
#[derive(Component, Default, Clone, Copy, defmt::Format)]
pub struct Connected(pub bool);

/// The carrier firmware version reported at bring-up.
#[derive(Component, Default, Clone, Copy, defmt::Format)]
pub struct FirmwareVersion {
    pub major: u8,
    pub minor: u8,
    pub patch: u8,
}

/// The robot's current behavior code (`b` status).
#[derive(Component, Default, Clone, Copy, defmt::Format)]
pub struct BehaviorCode(pub u8);

/// Battery state of charge; `charging` is the sign of the wire value.
#[derive(Component, Default, Clone, Copy, defmt::Format)]
pub struct BatterState {
    pub percent: f32,
    pub charging: bool,
}

// --- wheels -----------------------------------------------------------------

/// A wheel child entity, tagged with its side.
#[derive(Component, Clone, Copy, defmt::Format)]
pub struct Wheel {
    pub side: Side,
}

/// Commanded wheel intent. Setting it (a change) queues the matching wire
/// command in [`flush_wheels`](super::systems::flush_wheels).
#[derive(Component, Clone, Copy, defmt::Format)]
pub enum WheelTarget {
    Speed(AngularVelocity),
    Position(Angle),
}

/// Measured wheel state from the `j` (speed) and `w` (position) status.
#[derive(Component, Default, Clone, Copy, defmt::Format)]
pub struct WheelState {
    pub speed: AngularVelocity,
    pub position: Angle,
}

// --- drive ------------------------------------------------------------------

/// Differential-drive command: linear + angular velocity (`V`). Changing it
/// queues a drive command.
#[derive(Component, Default, Clone, Copy, defmt::Format)]
pub struct DifferentialDrive {
    pub linear: LinearVelocity,
    pub angular: AngularVelocity,
}

// --- servos -----------------------------------------------------------------

/// Which servo header a [`Servo`] drives.
#[derive(Debug, Clone, Copy, PartialEq, Eq, defmt::Format)]
pub enum ServoId {
    A,
    B,
}

/// A servo child entity. `position` is clamped to 0..=180° on flush.
#[derive(Component, Clone, Copy, defmt::Format)]
pub struct Servo {
    pub id: ServoId,
    pub position: Angle,
}

impl Servo {
    /// Position in whole degrees, clamped to the servo's 0..=180° range.
    pub fn degrees(&self) -> u8 {
        self.position.as_degrees().clamp(0.0, 180.0) as u8
    }
}

// --- LEDs -------------------------------------------------------------------

/// Backend marker: this LED entity is one of the Alvik's two RGB UI LEDs,
/// written over UART. Pair it with a [`LedColor`](crate::led::LedColor); the
/// colour is thresholded per channel into the aggregated LED byte by
/// [`flush_alvik_leds`](super::systems::flush_alvik_leds).
#[derive(Component, Clone, Copy)]
pub struct AlvikLed {
    pub side: Side,
}

/// The illuminator LED on the robot root.
#[derive(Component, Default, Clone, Copy, defmt::Format)]
pub struct Illuminator(pub bool);

/// The built-in LED on the robot root.
#[derive(Component, Default, Clone, Copy, defmt::Format)]
pub struct BuiltinLed(pub bool);

// --- sensors ----------------------------------------------------------------

/// Line-following reflectance sensors (`l` status), raw counts.
#[derive(Component, Default, Clone, Copy, defmt::Format)]
pub struct LineSensors {
    pub left: i16,
    pub center: i16,
    pub right: i16,
}

/// Color sensor (`c` status): the raw per-channel counts plus the calibrated
/// reading cached as a [`Color`]. [`apply_status`](super::systems::apply_status)
/// refreshes `color` from `raw` via [`ColorSensor::normalize_color`].
#[derive(Component, Default, Clone, Copy, Get)]
pub struct ColorSensor {
    raw: (i16, i16, i16),
    color: Color,
}

impl ColorSensor {
    pub fn update(&mut self, red: i16, green: i16, blue: i16) {
        self.raw = (red, green, blue);
        self.color = Self::normalize_color(red, green, blue);
    }

    /// Normalize the raw counts against the black/white calibration into a
    /// [`Color`], mirroring upstream `_normalize_color`.
    fn normalize_color(red: i16, green: i16, blue: i16) -> Color {
        /// Per-channel white reference (`WHITE_CAL` in the upstream constants).
        const WHITE_CAL: (f32, f32, f32) = (450.0, 500.0, 510.0);
        /// Per-channel black reference (`BLACK_CAL`).
        const BLACK_CAL: (f32, f32, f32) = (160.0, 200.0, 190.0);
        let channel = |value: i16, black: f32, white: f32| {
            let value = (value as f32).clamp(black, white);
            (value - black) / (white - black)
        };
        Color::srgb(
            channel(red, BLACK_CAL.0, WHITE_CAL.0),
            channel(green, BLACK_CAL.1, WHITE_CAL.1),
            channel(blue, BLACK_CAL.2, WHITE_CAL.2),
        )
    }
}

/// Time-of-flight distance sensors (`d` short read; `f` adds top/bottom).
#[derive(Component, Default, Clone, Copy, defmt::Format)]
pub struct Tof {
    pub left: Distance,
    pub center_left: Distance,
    pub center: Distance,
    pub center_right: Distance,
    pub right: Distance,
    pub top: Distance,
    pub bottom: Distance,
}

/// IMU readout (`i` status): linear acceleration and angular rate.
#[derive(Component, Default, Clone, Copy)]
pub struct Imu {
    pub accel: Vec3,
    pub gyro: Vec3,
}

/// Orientation from the `q` status (roll, pitch, yaw) as a rotation.
#[derive(Component, Default, Clone, Copy, Deref)]
pub struct Orientation(pub Quat);

/// Robot pose from the `z` status (x, y in mm, theta about Z).
#[derive(Component, Default, Deref)]
pub struct RobotPose(pub Pose);

/// Touch button bitmask (`t` status). See
/// [`TouchButton`](super::types::TouchButton).
#[derive(Component, Default, Clone, Copy, defmt::Format)]
pub struct TouchValue(pub u8);

impl TouchValue {
    /// The raw bitmask normalized to `0.0..=1.0`.
    pub fn normalized(&self) -> f32 {
        self.0 as f32 / u8::MAX as f32
    }
}

/// Move/tilt bitmask (`m` status). Drives the shake/lift/tilt observer events.
#[derive(Component, Default, Clone, Copy, defmt::Format)]
pub struct MotionValue(pub u8);

impl MotionValue {
    /// The raw bitmask normalized to `0.0..=1.0`.
    pub fn normalized(&self) -> f32 {
        self.0 as f32 / u8::MAX as f32
    }
}

// --- color classification ---------------------------------------------------

/// The discrete colour the sensor sees, from the `hsv2label` classifier.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, defmt::Format)]
pub enum ColorLabel {
    #[default]
    Undefined,
    Black,
    Grey,
    LightGrey,
    White,
    Yellow,
    LightGreen,
    Green,
    LightBlue,
    Blue,
    Violet,
    Brown,
    Orange,
    Red,
}

/// Classify a [`Color`] into a [`ColorLabel`], a port of upstream `hsv2label`
/// over the colour's HSV channels.
pub fn color_to_label(color: &Color) -> ColorLabel {
    let hsv = Hsva::from(*color);
    let (h, s, v) = (hsv.hue, hsv.saturation, hsv.value);
    if s < 0.1 {
        if v < 0.05 {
            ColorLabel::Black
        } else if v < 0.15 {
            ColorLabel::Grey
        } else if v < 0.8 {
            ColorLabel::LightGrey
        } else {
            ColorLabel::White
        }
    } else if v > 0.1 {
        if (20.0..90.0).contains(&h) {
            ColorLabel::Yellow
        } else if (90.0..140.0).contains(&h) {
            ColorLabel::LightGreen
        } else if (140.0..170.0).contains(&h) {
            ColorLabel::Green
        } else if (170.0..210.0).contains(&h) {
            ColorLabel::LightBlue
        } else if (210.0..250.0).contains(&h) {
            ColorLabel::Blue
        } else if (250.0..280.0).contains(&h) {
            ColorLabel::Violet
        } else if v < 0.5 && s < 0.45 {
            ColorLabel::Brown
        } else if v > 0.77 {
            ColorLabel::Orange
        } else {
            ColorLabel::Red
        }
    } else {
        ColorLabel::Black
    }
}
