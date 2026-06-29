//! Behaviour-tree leaves a scene can play over time. Unlike the one-shot
//! [`routes`](super::routes), these are [`Outcome`]-returning actions meant to
//! sit under a [`Sequence`] or [`Repeat`] and tick repeatedly.
//!
//! The fixed-velocity `<Drive linear=.. angular=..>` leaf is the upstream,
//! environment-agnostic [`Drive`](beet::prelude::Drive): it writes the agent's
//! commanded [`LinearVelocity`]/[`AngularVelocity`], which on the robot is the
//! [`AlvikRobot`] root (bound via [`bind_routes_to_robot`](super::scenes::bind_routes_to_robot)).
//! The sensor-driven steps below set those same two components directly.

use crate::prelude::*;
use beet::prelude::*;

/// Line-sensor reading at or above this counts as "black" (over the line).
/// A rough guess for the Alvik's reflectance sensors — tune on the bench.
const LINE_BLACK_THRESHOLD: i16 = 500;

/// Behaviour-tree leaf: one bang-bang line-following step. Forward while the
/// left sensor sees white, steer right while it sees black. Loop it with
/// [`Repeat`] for a continuous follower.
#[action(handler_only)]
#[derive(Default, Clone, Component, Reflect)]
#[reflect(Component)]
#[type_path = "alvik"]
pub fn LineFollowStep(
    _cx: In<ActionContext>,
    line: Single<&LineSensors, With<AlvikRobot>>,
    mut drive: Single<&mut DifferentialDrive, With<AlvikRobot>>,
) -> Outcome {
    if line.left >= LINE_BLACK_THRESHOLD {
        // left over the line — steer back to the right
        drive.linear = LinearVelocity::from_mm_per_sec(20.0);
        drive.angular = AngularVelocity::from_deg_per_sec(-60.0);
    } else {
        drive.linear = LinearVelocity::from_mm_per_sec(40.0);
        drive.angular = AngularVelocity::from_deg_per_sec(0.0);
    }
    Outcome::PASS
}

/// Turn away from a wall closer than this (mm).
const ROOMBA_NEAR_MM: f32 = 200.0;

/// Behaviour-tree leaf: one roomba step. Drive forward until the centre ToF
/// reads a wall closer than [`ROOMBA_NEAR_MM`], then spin right to clear it.
/// Loop it with [`Repeat`].
#[action(handler_only)]
#[derive(Default, Clone, Component, Reflect)]
#[reflect(Component)]
#[type_path = "alvik"]
pub fn RoombaStep(
    _cx: In<ActionContext>,
    tof: Single<&Tof, With<AlvikRobot>>,
    mut drive: Single<&mut DifferentialDrive, With<AlvikRobot>>,
) -> Outcome {
    if tof.center.as_millimeters() <= ROOMBA_NEAR_MM {
        drive.linear = LinearVelocity::from_mm_per_sec(0.0);
        drive.angular = AngularVelocity::from_deg_per_sec(-90.0);
    } else {
        drive.linear = LinearVelocity::from_mm_per_sec(50.0);
        drive.angular = AngularVelocity::from_deg_per_sec(0.0);
    }
    Outcome::PASS
}
