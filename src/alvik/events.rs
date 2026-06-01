//! Touch and motion edge events, the ECS-native form of the upstream
//! callback registry. [`detect_events`] compares the previous and current
//! [`Touch`] / [`Motion`] bitmasks each frame and `trigger`s an observer event
//! on each rising (or, for a drop, falling) edge. Users add observers instead
//! of registering callbacks.

use crate::alvik::components::AlvikRobot;
use crate::alvik::components::Motion;
use crate::alvik::components::Touch;
use beet::prelude::*;

/// A touch button, as decoded from the `t` bitmask.
#[derive(Debug, Clone, Copy, PartialEq, Eq, defmt::Format)]
pub enum TouchButton {
    Ok,
    Cancel,
    Center,
    Up,
    Left,
    Down,
    Right,
}

/// A tilt axis, as decoded from the `m` bitmask.
#[derive(Debug, Clone, Copy, PartialEq, Eq, defmt::Format)]
pub enum TiltAxis {
    X,
    NegX,
    Y,
    NegY,
    Z,
    NegZ,
}

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

/// Previous-frame bitmasks for edge detection. `motion` starts at `0x80`
/// (`-Z` tilted, i.e. upright) so a fresh, level robot does not fire a tilt.
pub struct EdgeState {
    touch: u8,
    motion: u8,
}

impl Default for EdgeState {
    fn default() -> Self {
        Self {
            touch: 0,
            motion: 0x80,
        }
    }
}

/// True if `mask` is newly set going from `prev` to `next`.
fn rising(prev: u8, next: u8, mask: u8) -> bool {
    prev & mask == 0 && next & mask != 0
}

/// True if `mask` was set in `prev` and cleared in `next`.
fn falling(prev: u8, next: u8, mask: u8) -> bool {
    prev & mask != 0 && next & mask == 0
}

/// Compare this frame's [`Touch`] / [`Motion`] against last frame's and trigger
/// the matching observer events. Added to the schedule by
/// [`AlvikPlugin`](super::AlvikPlugin).
pub fn detect_events(
    mut commands: Commands,
    mut prev: Local<EdgeState>,
    robot: Single<(&Touch, &Motion), With<AlvikRobot>>,
) {
    let (touch, motion) = (robot.0.0, robot.1.0);

    for (mask, button) in [
        (0b0000_0010, TouchButton::Ok),
        (0b0000_0100, TouchButton::Cancel),
        (0b0000_1000, TouchButton::Center),
        (0b0001_0000, TouchButton::Up),
        (0b0010_0000, TouchButton::Left),
        (0b0100_0000, TouchButton::Down),
        (0b1000_0000, TouchButton::Right),
    ] {
        if rising(prev.touch, touch, mask) {
            commands.trigger(TouchPressed(button));
        }
    }

    if rising(prev.motion, motion, 0b0000_0001) {
        commands.trigger(Shaken);
    }
    if rising(prev.motion, motion, 0b0000_0010) {
        commands.trigger(Lifted);
    }
    if falling(prev.motion, motion, 0b0000_0010) {
        commands.trigger(Dropped);
    }
    for (mask, axis) in [
        (0b0000_0100, TiltAxis::X),
        (0b0000_1000, TiltAxis::NegX),
        (0b0001_0000, TiltAxis::Y),
        (0b0010_0000, TiltAxis::NegY),
        (0b0100_0000, TiltAxis::Z),
        (0b1000_0000, TiltAxis::NegZ),
    ] {
        if rising(prev.motion, motion, mask) {
            commands.trigger(Tilted(axis));
        }
    }

    prev.touch = touch;
    prev.motion = motion;
}
