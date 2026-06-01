//! Touch and motion edge events, the ECS-native form of the upstream
//! callback registry. [`detect_events`] compares the previous and current
//! [`TouchValue`] / [`MotionValue`] bitmasks each frame and `trigger`s an
//! observer event on each rising (or, for a drop, falling) edge. Users add
//! observers instead of registering callbacks.

use crate::alvik::components::AlvikRobot;
use crate::alvik::components::MotionValue;
use crate::alvik::components::TouchValue;
use crate::alvik::types::EdgeState;
use crate::alvik::types::TiltAxis;
use crate::alvik::types::TouchButton;
use beet::prelude::*;

// Touch button bits in the `t` status bitmask.
const TOUCH_OK: u8 = 0b0000_0010;
const TOUCH_CANCEL: u8 = 0b0000_0100;
const TOUCH_CENTER: u8 = 0b0000_1000;
const TOUCH_UP: u8 = 0b0001_0000;
const TOUCH_LEFT: u8 = 0b0010_0000;
const TOUCH_DOWN: u8 = 0b0100_0000;
const TOUCH_RIGHT: u8 = 0b1000_0000;

// Move/tilt bits in the `m` status bitmask.
const MOTION_SHAKE: u8 = 0b0000_0001;
const MOTION_LIFT: u8 = 0b0000_0010;
const MOTION_TILT_X: u8 = 0b0000_0100;
const MOTION_TILT_NEG_X: u8 = 0b0000_1000;
const MOTION_TILT_Y: u8 = 0b0001_0000;
const MOTION_TILT_NEG_Y: u8 = 0b0010_0000;
const MOTION_TILT_Z: u8 = 0b0100_0000;
/// Upright (face-up) tilt bit; also [`EdgeState`]'s starting motion mask.
pub(crate) const MOTION_TILT_NEG_Z: u8 = 0b1000_0000;

/// A touch button was just pressed.
#[derive(Event, Debug, Clone, Copy)]
pub struct TouchPressed(pub TouchButton);

/// The robot was just shaken.
#[derive(Event, Debug, Clone, Copy)]
pub struct Shaken;

/// The robot was just lifted off the ground.
#[derive(Event, Debug, Clone, Copy)]
pub struct Lifted;

/// The robot was just set back down.
#[derive(Event, Debug, Clone, Copy)]
pub struct Dropped;

/// The robot was just tilted onto an axis.
#[derive(Event, Debug, Clone, Copy)]
pub struct Tilted(pub TiltAxis);

/// True if `mask` is newly set going from `prev` to `next`.
fn rising(prev: u8, next: u8, mask: u8) -> bool {
    prev & mask == 0 && next & mask != 0
}

/// True if `mask` was set in `prev` and cleared in `next`.
fn falling(prev: u8, next: u8, mask: u8) -> bool {
    prev & mask != 0 && next & mask == 0
}

/// Compare this frame's [`TouchValue`] / [`MotionValue`] against last frame's
/// and trigger the matching observer events. Added to the schedule by
/// [`AlvikPlugin`](super::plugin::AlvikPlugin).
pub fn detect_events(
    mut commands: Commands,
    mut prev: Local<EdgeState>,
    robot: Single<(&TouchValue, &MotionValue), With<AlvikRobot>>,
) {
    let (touch, motion) = (robot.0.0, robot.1.0);

    for (mask, button) in [
        (TOUCH_OK, TouchButton::Ok),
        (TOUCH_CANCEL, TouchButton::Cancel),
        (TOUCH_CENTER, TouchButton::Center),
        (TOUCH_UP, TouchButton::Up),
        (TOUCH_LEFT, TouchButton::Left),
        (TOUCH_DOWN, TouchButton::Down),
        (TOUCH_RIGHT, TouchButton::Right),
    ] {
        if rising(prev.touch, touch, mask) {
            commands.trigger(TouchPressed(button));
        }
    }

    if rising(prev.motion, motion, MOTION_SHAKE) {
        commands.trigger(Shaken);
    }
    if rising(prev.motion, motion, MOTION_LIFT) {
        commands.trigger(Lifted);
    }
    if falling(prev.motion, motion, MOTION_LIFT) {
        commands.trigger(Dropped);
    }
    for (mask, axis) in [
        (MOTION_TILT_X, TiltAxis::X),
        (MOTION_TILT_NEG_X, TiltAxis::NegX),
        (MOTION_TILT_Y, TiltAxis::Y),
        (MOTION_TILT_NEG_Y, TiltAxis::NegY),
        (MOTION_TILT_Z, TiltAxis::Z),
        (MOTION_TILT_NEG_Z, TiltAxis::NegZ),
    ] {
        if rising(prev.motion, motion, mask) {
            commands.trigger(Tilted(axis));
        }
    }

    prev.touch = touch;
    prev.motion = motion;
}
