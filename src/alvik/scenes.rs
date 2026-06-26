//! Alvik scene support: the [`AlvikScenePlugin`] registration of the Alvik
//! route/action/scene types a loaded scene can carry, the Alvik [`ResetScene`]
//! handler, and the [`Drive`] / [`AlvikScript`] authoring widgets. The
//! hardware-agnostic scene server and its meta-routes ([`LoadScene`],
//! [`ClearScene`], …) live upstream in [`beet::router`].

use crate::prelude::*;
use alloc::string::String;
use beet::prelude::*;

/// Registers every Alvik route/action/scene type a loaded scene can carry, the
/// route-path and script authoring templates, and the Alvik [`ResetScene`]
/// handler (stop motors + wheels). The generic types (`SpawnAction`,
/// `EndInDuration`, `Script`) are registered by
/// [`EspScenePlugin`](crate::scene::EspScenePlugin), which adds this plugin under
/// the `alvik` feature.
pub struct AlvikScenePlugin;

impl Plugin for AlvikScenePlugin {
    fn build(&self, app: &mut App) {
        app.register_type::<DriveHandler>()
            .register_type::<LedHandler>()
            .register_type::<ApplyDrive>()
            .register_type::<DriveCommand>()
            .register_type::<LineFollowStep>()
            .register_type::<RoombaStep>()
            // The `<Drive>` authoring façade over `(ApplyDrive, DriveCommand)`.
            .register_type::<Drive>()
            .add_observer(reset_robot);
        // The script step plus its data: the typed `Script` and the `ScriptState`
        // it threads (registered once in `EspScenePlugin`), and the `<AlvikScript>`
        // authoring template over them.
        #[cfg(any(feature = "rhai", feature = "quickjs"))]
        app.register_type::<super::scripting::AlvikScriptStep>()
            .register_type::<Script<
                super::scripting::AlvikInput,
                super::scripting::AlvikOutput,
            >>()
            .register_template::<AlvikScript>();
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
// Alvik authoring widgets
// ---------------------------------------------------------------------------

/// `<Drive linear={60.0} angular={0.0}/>` — a behaviour-tree leaf applying a fixed
/// drive velocity (mm/s, deg/s), the façade over `(ApplyDrive, DriveCommand)`.
/// Pair with an `<EndInDuration>` in a `<Sequence>` to "drive like this for N ms".
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

/// `<AlvikScript script="..." language="rhai">` — a behaviour-tree leaf running a
/// script robot controller each tick (`input.depth_mm`/`input.line_*`/`input.state`
/// -> `#{ linear, angular, led_left, led_right, state }`). The authoring template
/// over `(AlvikScriptStep, Script<AlvikInput, AlvikOutput>)`.
///
/// `language` selects the backend ([`ScriptLanguage::from_str`]), falling back to
/// the build default when absent, so the same scene runs under rhai or quickjs.
#[cfg(any(feature = "rhai", feature = "quickjs"))]
#[template]
pub fn AlvikScript(
    #[prop(into)] script: String,
    language: Option<String>,
) -> impl Bundle {
    let language = language
        .and_then(|name| name.parse::<ScriptLanguage>().ok())
        .unwrap_or_default();
    (
        super::scripting::AlvikScriptStep,
        Script::<super::scripting::AlvikInput, super::scripting::AlvikOutput>::new(
            language, script,
        ),
    )
}
