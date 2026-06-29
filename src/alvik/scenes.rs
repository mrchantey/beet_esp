//! Alvik scene support: the [`AlvikScenePlugin`] registration of the Alvik
//! route/action/scene types a loaded scene can carry, the Alvik [`ResetScene`]
//! handler, the [`bind_routes_to_robot`] binder that makes the robot the agent of
//! every loaded route (so the upstream `<Drive>` leaf writes the robot's velocity),
//! and the [`AlvikScript`] authoring widget. The hardware-agnostic scene server
//! and its meta-routes ([`LoadScene`], [`ClearScene`], …) live upstream in
//! [`beet::router`].

use crate::prelude::*;
// Only the `<AlvikScript>` template (gated on a scripting backend) names `String`.
#[cfg(any(feature = "rhai", feature = "quickjs"))]
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
            .register_type::<LineFollowStep>()
            .register_type::<RoombaStep>()
            // Bind every loaded route to the robot so the upstream `<Drive>` leaf
            // (registered by `ActionPlugin`) resolves its agent to the robot and
            // writes the robot's commanded velocity.
            .add_systems(Update, bind_routes_to_robot)
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

/// Bind every loaded [`RouteAction`] to the [`AlvikRobot`] as its agent, so a
/// `<Drive>` leaf in the route's tree resolves its agent (up the ancestors to the
/// `RouteAction`, then through [`ActionOf`]) to the robot and writes the robot's
/// commanded [`LinearVelocity`]/[`AngularVelocity`]. Idempotent: the
/// `Without<ActionOf>` filter skips routes already bound.
fn bind_routes_to_robot(
    mut commands: Commands,
    robot: Single<Entity, With<AlvikRobot>>,
    routes: Query<Entity, (With<RouteAction>, Without<ActionOf>)>,
) {
    for entity in &routes {
        commands.entity(entity).insert(ActionOf(*robot));
    }
}

// ---------------------------------------------------------------------------
// Alvik authoring widgets
// ---------------------------------------------------------------------------

// `<Drive linear={60.0} angular={0.0}/>` resolves to the upstream, environment-
// agnostic `beet::prelude::Drive` leaf (registered by `ActionPlugin`): it writes
// the agent's commanded `LinearVelocity`/`AngularVelocity`, which on the robot is
// the `AlvikRobot` root (bound by `bind_routes_to_robot`). No firmware façade.

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
