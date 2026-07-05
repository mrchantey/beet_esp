//! Alvik scene support: the [`AlvikScenePlugin`] registration of the Alvik
//! route/action/scene types a loaded scene can carry, the [`Alvik`] root element
//! that assembles the robot bundle, the Alvik [`ResetScene`] handler, and the
//! [`AlvikScript`] authoring widget. The hardware-agnostic scene server and its
//! meta-routes ([`LoadScene`], [`ClearScene`], …) live upstream in
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
            // The `<Alvik>` root element: assembles the robot's hardware bundle and
            // slots its body (the `<Router>`). The firmware boots `<Alvik><Router>…`
            // so the robot is the scene root and a loaded `<SetDrive>` leaf resolves its
            // `DifferentialDrive` to the robot via root-ancestor (no marker needed).
            .register_template::<Alvik>()
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
    info!("reset: stopping motors and wheels");
    drive.linear = LinearVelocity::default();
    drive.angular = AngularVelocity::default();
    for mut target in &mut wheels {
        *target = WheelTarget::Speed(AngularVelocity::from_rpm(0.0));
    }
}

/// The `<Alvik>` root element: the robot's full hardware bundle (the [`AlvikRobot`]
/// marker, every state + sensor component including the commanded
/// [`DifferentialDrive`], and the wheel / servo / RGB-LED children), with a
/// `<Slot/>` hosting its body — the firmware slots the `<Router>` there, so the
/// boot tree is `<Alvik><Router>…</Router></Alvik>`.
///
/// Because the robot is the *root* of that tree, a loaded behaviour's
/// [`AgentQuery`] resolves its agent to this entity by root-ancestor fallback (a
/// loaded `<RouteAction>` is reparented under the server, whose root ancestor is
/// the robot). So a `<SetDrive>` leaf writes the robot's `DifferentialDrive` with no
/// marker, no `ActionOf` and no resource; a route whose agent lacks a
/// `DifferentialDrive` errors loudly, the desired feedback.
///
/// Mirrors beet's [`Foxie`](beet::prelude::Foxie) template: a Rust `#[template]`
/// has no neutral host (`<Fragment>` is `.bsx`-only), so an inert `<span>` carries
/// the bundle and the slot. The `Element` is harmless on a hardware entity (it is
/// never rendered), and the robot's transport systems query `With<AlvikRobot>`
/// regardless.
#[template]
pub fn Alvik() -> impl Bundle {
    rsx! {
        <span {(
            AlvikRobot,
            // Grouped into sub-bundles: a flat tuple would exceed the 15-element
            // `Bundle` impl. State, then sensors.
            (
                Connected::default(),
                FirmwareVersion::default(),
                BehaviorCode::default(),
                BatterState::default(),
                Illuminator(true),
                BuiltinLed(false),
            ),
            (
                LineSensors::default(),
                ColorSensor::default(),
                Tof::default(),
                Imu::default(),
                Orientation::default(),
                RobotPose::default(),
                // Commanded velocity: the upstream `SetDrive` leaf writes this on the
                // robot (its agent, resolved as the root ancestor), and `flush_drive`
                // sends it to the wire.
                DifferentialDrive::default(),
                TouchValue::default(),
                MotionValue::default(),
            ),
            children![
                (
                    Wheel { side: Side::Left },
                    WheelState::default(),
                    WheelTarget::Speed(AngularVelocity::default()),
                ),
                (
                    Wheel { side: Side::Right },
                    WheelState::default(),
                    WheelTarget::Speed(AngularVelocity::default()),
                ),
                (Servo {
                    id: ServoId::A,
                    position: Angle::from_degrees(90.0),
                }),
                (Servo {
                    id: ServoId::B,
                    position: Angle::from_degrees(90.0),
                }),
                (AlvikLed { side: Side::Left }, LedColor::default()),
                (AlvikLed { side: Side::Right }, LedColor::default()),
            ],
        )}>
            <Slot/>
        </span>
    }
}

// ---------------------------------------------------------------------------
// Alvik authoring widgets
// ---------------------------------------------------------------------------

// `<SetDrive linear={60.0} angular={0.0}/>` resolves to the upstream, environment-
// agnostic `beet::prelude::SetDrive` leaf (registered by `ActionPlugin`): it writes
// the agent's commanded `DifferentialDrive`, which on the robot is the `AlvikRobot`
// root resolved by `AgentQuery`'s root-ancestor fallback (the loaded route is a
// descendant of the robot, which is the scene root). No firmware façade, no marker.

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
