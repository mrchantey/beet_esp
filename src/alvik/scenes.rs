//! Alvik scene support: the [`AlvikScenePlugin`] registration, the Alvik
//! [`ResetScene`] handler, and the canonical example scenes (rc, dance,
//! line-follower, roomba, script). The hardware-agnostic scene server and its
//! meta-routes ([`LoadScene`], [`ClearScene`], …) live in [`crate::scene`]; here
//! are only the Alvik-specific routes/actions and the scenes that wire them.

use crate::prelude::*;
use crate::scene::server::ResetScene;
use crate::scene::server::log_scene;
use beet::prelude::*;
use defmt::Debug2Format;
use defmt::info;
use defmt::warn;

extern crate alloc;
use alloc::string::String;

/// Registers every Alvik route/action/scene marker a loaded scene can carry, and
/// adds the Alvik [`ResetScene`] handler (stop motors + wheels). The generic
/// types (`ActionRoute`, `EndInDuration`, `Script`) are registered by
/// [`SceneServerPlugin`](crate::scene::SceneServerPlugin), which adds this
/// plugin under the `alvik` feature.
pub struct AlvikScenePlugin;

impl Plugin for AlvikScenePlugin {
    fn build(&self, app: &mut App) {
        app.register_type::<DriveRoute>()
            .register_type::<LedRoute>()
            .register_type::<ApplyDrive>()
            .register_type::<DriveCommand>()
            .register_type::<LineFollowStep>()
            .register_type::<RoombaStep>()
            .add_observer(reset_robot);
        #[cfg(feature = "rhai")]
        app.register_type::<super::scripting::AlvikScriptStep>();
    }
}

/// Alvik [`ResetScene`] handler: stop the motors and wheels — the safe resting
/// state. The UI LEDs are turned off by the generic
/// [`reset_leds`](crate::scene::server::reset_leds).
fn reset_robot(
    _ev: On<ResetScene>,
    mut drive: Single<&mut DifferentialDrive, With<AlvikRobot>>,
    mut wheels: Query<&mut WheelTarget>,
) {
    drive.linear = LinearVelocity::from_mm_per_sec(0.0);
    drive.angular = AngularVelocity::from_deg_per_sec(0.0);
    for mut target in &mut wheels {
        *target = WheelTarget::Speed(AngularVelocity::from_rpm(0.0));
    }
}

// ---------------------------------------------------------------------------
// Example scenes, built in code and dumped as JSON on boot.
// ---------------------------------------------------------------------------

/// `dance-routine` — forward 1s, left 1s, forward 1s, stop.
pub fn dance_scene() -> impl Bundle {
    (ActionRoute, PathPartial::new("dance-routine"), children![(
        Sequence::new(),
        children![
            (ApplyDrive, DriveCommand::drive(60.0, 0.0)),
            EndInDuration::pass(Duration::from_secs(1)),
            (ApplyDrive, DriveCommand::drive(0.0, 90.0)),
            EndInDuration::pass(Duration::from_secs(1)),
            (ApplyDrive, DriveCommand::drive(60.0, 0.0)),
            EndInDuration::pass(Duration::from_secs(1)),
            (ApplyDrive, DriveCommand::drive(0.0, 0.0)),
        ],
    )])
}

/// `line-follower` — repeat a bang-bang follow step every 50 ms.
pub fn line_follower_scene() -> impl Bundle {
    (ActionRoute, PathPartial::new("line-follower"), children![(
        Repeat::new(),
        children![(
            Sequence::new(),
            children![LineFollowStep, EndInDuration::pass(Duration::from_millis(50))],
        )],
    )])
}

/// `roomba` — repeat a wall-avoiding step every 50 ms.
pub fn roomba_scene() -> impl Bundle {
    (ActionRoute, PathPartial::new("roomba"), children![(
        Repeat::new(),
        children![(
            Sequence::new(),
            children![RoombaStep, EndInDuration::pass(Duration::from_millis(50))],
        )],
    )])
}

/// Dump every Alvik example scene as JSON on boot, so each can be saved and
/// POSTed to `/load`. The RC scene has two roots, so it is serialized as a pair.
pub fn log_alvik_scenes(world: &mut World) {
    // rc: two standalone route roots in one scene.
    let drive = world.spawn((DriveRoute, PathPartial::new("drive/:dir"))).id();
    let led = world.spawn((LedRoute, PathPartial::new("led/:side/:state"))).id();
    let bytes = WorldSerdeSaver::new(world)
        .with_entity_tree(drive)
        .with_entity_tree(led)
        .save(MediaType::Json);
    world.entity_mut(drive).despawn();
    world.entity_mut(led).despawn();
    match bytes.and_then(|bytes| bytes.as_utf8().map(String::from)) {
        Ok(json) => info!("scene[rc]:\n{}", json.as_str()),
        Err(err) => warn!("scene[rc] dump failed: {}", Debug2Format(&err)),
    }

    log_scene(world, "dance-routine", dance_scene());
    log_scene(world, "line-follower", line_follower_scene());
    log_scene(world, "roomba", roomba_scene());
    #[cfg(feature = "rhai")]
    log_scene(world, "script", super::scripting::script_scene());
}
