//! The Alvik's scene-carried control step. The route's behaviour is a program
//! POSTed over the wire: each tick [`AlvikScriptStep`] gathers a sensor
//! snapshot into [`AlvikInput`], runs the entity's [`Script`] via the shared
//! [`Value`]-marshalled backend, and applies the [`AlvikOutput`] it returns to
//! the drive + LEDs. The script and its persistent [`ScriptState`] are generic;
//! only the input gathered and the output applied are Alvik-specific (vs the
//! WS2812 [`LedScriptStep`](crate::scripting::LedScriptStep)).

use crate::prelude::*;
use crate::scripting::ScriptState;
use crate::scripting::unpack_color;
use beet::prelude::*;

/// The sensor snapshot handed to the Alvik [`Script`] each tick, bound to
/// `input`. Plain serde: it is marshalled to the engine through [`Value`], so a
/// script reads `input.depth_mm`, `input.yaw_deg`, `input.state.*` and so on.
/// `Reflect` only so `Script<AlvikInput, AlvikOutput>` has a type path to
/// register; the value itself is never reflected.
#[derive(Serialize, Reflect)]
pub struct AlvikInput {
    /// Milliseconds since boot.
    pub elapsed_ms: i64,
    /// Forward time-of-flight distance, mm (`0` if unreadable).
    pub depth_mm: i64,
    /// Left line-follower reflectance.
    pub line_left: i64,
    /// Centre line-follower reflectance.
    pub line_center: i64,
    /// Right line-follower reflectance.
    pub line_right: i64,
    /// Heading about the vertical axis, degrees.
    pub yaw_deg: f64,
    /// Touch-button bitmask.
    pub touch: i64,
    /// The script's persistent state (see [`ScriptState`]).
    pub state: HashMap<String, Value>,
}

/// The command map the Alvik [`Script`] returns each tick. Every field is
/// optional, defaulting to "no change", so a script need only set what it drives.
#[derive(Deserialize, Reflect)]
pub struct AlvikOutput {
    /// Forward velocity, mm/s.
    #[serde(default)]
    pub linear: f32,
    /// Turn rate, deg/s.
    #[serde(default)]
    pub angular: f32,
    /// Left UI LED packed `0xRRGGBB`.
    #[serde(default)]
    pub led_left: u32,
    /// Right UI LED packed `0xRRGGBB`.
    #[serde(default)]
    pub led_right: u32,
    /// The next persistent state to thread to the following tick.
    #[serde(default)]
    pub state: HashMap<String, Value>,
}

/// Behaviour-tree leaf: gather the sensor snapshot, run this entity's
/// [`Script`], apply the drive + LED output. Loop it with [`Repeat`] for a live
/// controller. The script reads `input.*` (depth, line, yaw, touch, elapsed)
/// plus `input.state`, and returns `#{ linear, angular, led_left, led_right,
/// state }`.
#[action(handler_only)]
#[derive(Default, Clone, Component, Reflect)]
#[reflect(Component)]
#[type_path = "alvik"]
#[require(Script<AlvikInput, AlvikOutput>, ScriptState)]
pub fn AlvikScriptStep(
    cx: In<ActionContext>,
    time: Res<Time>,
    mut scripts: Query<(&Script<AlvikInput, AlvikOutput>, &mut ScriptState)>,
    sensors: Single<(&Tof, &LineSensors, &Orientation, &TouchValue), With<AlvikRobot>>,
    mut drive: Single<&mut DifferentialDrive, With<AlvikRobot>>,
    mut leds: Query<(&AlvikLed, &mut LedColor)>,
) -> Outcome {
    let Ok((script, mut state)) = scripts.get_mut(cx.id()) else {
        return Outcome::PASS;
    };
    let (tof, line, orientation, touch) = *sensors;
    let input = AlvikInput {
        elapsed_ms: (time.elapsed_secs_f64() * 1000.0) as i64,
        depth_mm: tof.center.as_millimeters() as i64,
        line_left: line.left as i64,
        line_center: line.center as i64,
        line_right: line.right as i64,
        yaw_deg: orientation.0.to_euler(EulerRot::XYZ).2 as f64 * 180.0
            / core::f64::consts::PI,
        touch: touch.0 as i64,
        state: state.0.clone(),
    };

    match script.run(input) {
        Ok(output) => {
            drive.linear = LinearVelocity::from_mm_per_sec(output.linear);
            drive.angular = AngularVelocity::from_deg_per_sec(output.angular);
            for (led, mut color) in &mut leds {
                let packed = match led.side {
                    Side::Left => output.led_left,
                    Side::Right => output.led_right,
                };
                color.0 = unpack_color(packed);
            }
            info!(
                "scene: script -> ({} mm/s, {} deg/s) led {:#08x}/{:#08x}",
                output.linear, output.angular, output.led_left, output.led_right
            );
            state.0 = output.state;
        }
        Err(err) => warn!("scene: script error: {}", err),
    }
    Outcome::PASS
}
