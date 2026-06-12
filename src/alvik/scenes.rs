//! Alvik scene support: the [`AlvikScenePlugin`] registration of the Alvik
//! route/action/scene markers a loaded scene can carry, plus the Alvik
//! [`ResetScene`] handler. The hardware-agnostic scene server and its
//! meta-routes ([`LoadScene`], [`ClearScene`], …) live upstream in
//! [`beet::router`]; the example scenes that wire these markers are generated on
//! the host by the `scenes` crate.

use crate::prelude::*;
use beet::prelude::*;

/// Registers every Alvik route/action/scene marker a loaded scene can carry, and
/// adds the Alvik [`ResetScene`] handler (stop motors + wheels). The generic
/// types (`SpawnAction`, `EndInDuration`, `Script`) are registered by
/// [`EspScenePlugin`](crate::scene::EspScenePlugin), which adds this plugin
/// under the `alvik` feature.
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
        // The script step plus its data: the typed `Script` and the `ScriptState`
        // it threads. `ScriptState` is registered once in `EspScenePlugin`.
        #[cfg(any(feature = "rhai", feature = "quickjs"))]
        app.register_type::<super::scripting::AlvikScriptStep>()
            .register_type::<Script<
                super::scripting::AlvikInput,
                super::scripting::AlvikOutput,
            >>();
    }
}

/// Alvik [`ResetScene`] handler: stop the motors and wheels — the safe resting
/// state. The UI LEDs are turned off by the generic
/// [`reset_leds`](crate::scene::reset_leds).
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
