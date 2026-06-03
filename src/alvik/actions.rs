//! Behaviour-tree leaves a scene can play over time. Unlike the one-shot
//! [`routes`](super::routes), these are [`Outcome`]-returning actions meant to
//! sit under a [`Sequence`] or [`Repeat`] and tick repeatedly.

use crate::prelude::*;
use beet::prelude::*;
use defmt::info;

/// A drive velocity a behaviour-tree leaf can apply. Carried alongside
/// [`ApplyDrive`] so a scene can configure each step's motion.
#[derive(Debug, Default, Clone, Component, Reflect)]
#[reflect(Component, Default)]
#[type_path = "alvik"]
pub struct DriveCommand {
    /// Forward speed, mm/s (negative = reverse).
    linear_mm_s: f32,
    /// Turn rate, deg/s (positive = left).
    angular_deg_s: f32,
}

impl DriveCommand {
    /// A command with the given linear (mm/s) and angular (deg/s) rates.
    pub const fn drive(linear_mm_s: f32, angular_deg_s: f32) -> Self {
        Self { linear_mm_s, angular_deg_s }
    }
}

/// Behaviour-tree leaf: apply this entity's [`DriveCommand`] to the robot, then
/// [`Outcome::PASS`]. Pair with [`EndInDuration`] in a [`Sequence`] to "drive
/// like this for N seconds".
#[action(handler_only)]
#[derive(Default, Clone, Component, Reflect)]
#[reflect(Component)]
#[type_path = "alvik"]
#[require(DriveCommand)]
pub fn ApplyDrive(
    cx: In<ActionContext>,
    commands: Query<&DriveCommand>,
    mut drive: Single<&mut DifferentialDrive, With<AlvikRobot>>,
) -> Outcome {
    if let Ok(command) = commands.get(cx.id()) {
        drive.linear = LinearVelocity::from_mm_per_sec(command.linear_mm_s);
        drive.angular = AngularVelocity::from_deg_per_sec(command.angular_deg_s);
        info!(
            "scene: apply drive ({} mm/s, {} deg/s)",
            command.linear_mm_s, command.angular_deg_s
        );
    }
    Outcome::PASS
}

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
