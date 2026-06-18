//! Alvik scene support: the [`AlvikScenePlugin`] registration of the Alvik
//! route/action/scene markers a loaded scene can carry, plus the Alvik
//! [`ResetScene`] handler. The hardware-agnostic scene server and its
//! meta-routes ([`LoadScene`], [`ClearScene`], …) live upstream in
//! [`beet::router`]; the example scenes that wire these markers are generated on
//! the host by the `scenes` crate.

use alloc::string::String;
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
            // The `<Drive>` BSX authoring façade over `(ApplyDrive, DriveCommand)`.
            .register_type::<Drive>()
            .add_observer(reset_robot);
        // The script step plus its data: the typed `Script` and the `ScriptState`
        // it threads (`ScriptState` is registered once in `EspScenePlugin`), and the
        // `<AlvikScript>` BSX authoring façade over them.
        #[cfg(any(feature = "rhai", feature = "quickjs"))]
        app.register_type::<super::scripting::AlvikScriptStep>()
            .register_type::<Script<
                super::scripting::AlvikInput,
                super::scripting::AlvikOutput,
            >>()
            .register_type::<AlvikScript>();
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

// ---------------------------------------------------------------------------
// Alvik BSX scene-authoring widgets
// ---------------------------------------------------------------------------
// The Alvik-specific façades, paired with the generic ones in
// [`crate::scene`] (`<RouteAction>`/`<Loop>`/`<Steps>`/`<Wait>`/`<At>`). Each is
// a non-generic component whose `on_add` inserts the concrete behaviour, so a
// pushed `.bsx` scene reads as a behaviour tree (see [`crate::scene`] for why the
// generic primitives cannot be bare tags).

/// `<Drive linear={60.0} angular={0.0}/>` — a behaviour-tree leaf applying a fixed
/// drive velocity (mm/s, deg/s), the façade over `(ApplyDrive, DriveCommand)`.
/// Pair with `<Wait>` in `<Steps>` to "drive like this for N ms".
#[derive(Debug, Default, Component, Reflect)]
#[reflect(Component, Default)]
#[component(on_add = drive_on_add)]
pub struct Drive {
    /// Forward speed, mm/s (negative = reverse).
    pub linear: f32,
    /// Turn rate, deg/s (positive = left).
    pub angular: f32,
}

/// Insert `(ApplyDrive, DriveCommand)` from the declared velocities.
fn drive_on_add(mut world: DeferredWorld, cx: HookContext) {
    let (linear, angular) = world
        .entity(cx.entity)
        .get::<Drive>()
        .map(|drive| (drive.linear, drive.angular))
        .unwrap_or_default();
    world
        .commands()
        .entity(cx.entity)
        .insert((ApplyDrive, DriveCommand::drive(linear, angular)));
}

/// `<AlvikScript rhai="...">` — a behaviour-tree leaf running a rhai robot
/// controller each tick (`input.depth_mm`/`input.line_*`/`input.state` -> `#{
/// linear, angular, led_left, led_right, state }`), the façade over
/// `(AlvikScriptStep, Script<AlvikInput, AlvikOutput>)`.
#[cfg(any(feature = "rhai", feature = "quickjs"))]
#[derive(Debug, Default, Component, Reflect)]
#[reflect(Component, Default)]
#[component(on_add = alvik_script_on_add)]
pub struct AlvikScript {
    /// The rhai source run each tick.
    pub rhai: String,
}

/// Insert `(AlvikScriptStep, Script::rhai(..))` from the declared source.
#[cfg(any(feature = "rhai", feature = "quickjs"))]
fn alvik_script_on_add(mut world: DeferredWorld, cx: HookContext) {
    let source = world
        .entity(cx.entity)
        .get::<AlvikScript>()
        .map(|step| step.rhai.clone())
        .unwrap_or_default();
    world.commands().entity(cx.entity).insert((
        super::scripting::AlvikScriptStep,
        Script::<super::scripting::AlvikInput, super::scripting::AlvikOutput>::rhai(
            source,
        ),
    ));
}
