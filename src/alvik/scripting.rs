//! The Alvik's scene-carried rhai control step. The route's behaviour is a
//! program POSTed over the wire: each tick [`AlvikScriptStep`] gathers a sensor
//! snapshot, runs the entity's [`Script`] via the shared
//! [`run_script`](crate::scripting::rhai::run_script), and applies the drive/LED
//! output it returns. The script and its persistent `state` map are the generic
//! [`Script`] component; only the input gathered and the output applied are
//! Alvik-specific (vs the WS2812 [`LedScriptStep`](crate::scripting::rhai::LedScriptStep)).

use crate::prelude::*;
// Explicit import: disambiguates our scene [`Script`] from beet's own `Script`
// action, both of which the glob imports below pull into scope under `rhai`.
use crate::scripting::rhai::Script;
use crate::scripting::rhai::ScriptMap;
use crate::scripting::rhai::run_script;
use crate::scripting::rhai::unpack_color;
use beet::prelude::*;

/// Behaviour-tree leaf: gather the sensor snapshot, run this entity's
/// [`Script`], apply the drive + LED output. Loop it with [`Repeat`] for a live
/// controller. The script reads `input.*` (depth, line, yaw, touch, elapsed)
/// plus its own `state` map, and returns `#{ linear, angular, led_left,
/// led_right, state }`.
#[action(handler_only)]
#[derive(Default, Clone, Component, Reflect)]
#[reflect(Component)]
#[type_path = "alvik"]
#[require(Script)]
pub fn AlvikScriptStep(
    cx: In<ActionContext>,
    time: Res<Time>,
    mut scripts: Query<&mut Script>,
    sensors: Single<(&Tof, &LineSensors, &Orientation, &TouchValue), With<AlvikRobot>>,
    mut drive: Single<&mut DifferentialDrive, With<AlvikRobot>>,
    mut leds: Query<(&AlvikLed, &mut LedColor)>,
) -> Outcome {
    let Ok(mut script) = scripts.get_mut(cx.id()) else {
        return Outcome::PASS;
    };
    let (tof, line, orientation, touch) = *sensors;
    let mut input = ScriptMap::default();
    input.insert(
        "elapsed_ms".into(),
        Value::Int((time.elapsed_secs_f64() * 1000.0) as i64),
    );
    input.insert("depth_mm".into(), Value::Int(tof.center.as_millimeters() as i64));
    input.insert("line_left".into(), Value::Int(line.left as i64));
    input.insert("line_center".into(), Value::Int(line.center as i64));
    input.insert("line_right".into(), Value::Int(line.right as i64));
    input.insert(
        "yaw_deg".into(),
        Value::Float(
            orientation.0.to_euler(EulerRot::XYZ).2 as f64 * 180.0
                / core::f64::consts::PI,
        ),
    );
    input.insert("touch".into(), Value::Int(touch.0 as i64));

    match run_script(&script.source, &input, &script.state) {
        Ok((output, state)) => {
            let float = |key: &str| {
                output.get(key).and_then(|value| value.as_f64().ok()).unwrap_or(0.0) as f32
            };
            let int = |key: &str| {
                output.get(key).and_then(|value| value.as_i64().ok()).unwrap_or(0) as u32
            };
            let (linear, angular) = (float("linear"), float("angular"));
            let (led_left, led_right) = (int("led_left"), int("led_right"));
            drive.linear = LinearVelocity::from_mm_per_sec(linear);
            drive.angular = AngularVelocity::from_deg_per_sec(angular);
            for (led, mut color) in &mut leds {
                let packed = match led.side {
                    Side::Left => led_left,
                    Side::Right => led_right,
                };
                color.0 = unpack_color(packed);
            }
            info!(
                "scene: script -> ({} mm/s, {} deg/s) led {:#08x}/{:#08x}",
                linear, angular, led_left, led_right
            );
            script.state = state;
        }
        Err(err) => warn!("scene: script error: {}", err.as_str()),
    }
    Outcome::PASS
}

