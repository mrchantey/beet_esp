//! Scene-carried rhai control scripts. The route's behaviour is a program
//! POSTed over the wire: each tick [`AlvikScriptStep`] gathers a sensor
//! snapshot, runs the script, and applies the drive/LED output it returns,
//! including a `state` array the script owns as persistent memory.
//!
//! beet's own `Script` action is std-only (rhai pulls std there), so this wires
//! rhai directly through [`beet::exports::rhai`]: it builds `no_std`, and the
//! input/output are marshalled by hand through rhai `Map`s rather than serde
//! (serde's `no_std` split fights rhai's).

use crate::prelude::*;
use beet::exports::rhai;
use beet::prelude::*;
use defmt::info;
use defmt::warn;
use rhai::Dynamic;
use rhai::Engine;
use rhai::Map;
use rhai::Scope;

extern crate alloc;
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

/// The sensor snapshot exposed to a script as the `input` map, plus the
/// script-owned `state` array (its scratch memory between ticks).
struct AlvikInput {
    elapsed_ms: i64,
    depth_mm: i64,
    line_left: i64,
    line_center: i64,
    line_right: i64,
    yaw_deg: f64,
    touch: i64,
}

/// What a script returns: drive velocity, both UI LED colours (packed
/// `0xRRGGBB`), and the next `state` array.
struct AlvikOutput {
    linear_mm_s: f32,
    angular_deg_s: f32,
    led_left: u32,
    led_right: u32,
    state: Vec<i64>,
}

/// A rhai control script carried in a scene. `source` is the program;
/// `state` is the persistent memory it reads and rewrites each tick.
#[derive(Debug, Default, Clone, Component, Reflect)]
#[reflect(Component)]
#[type_path = "alvik"]
pub struct AlvikScript {
    /// rhai source. Reads `input.*` and `state`, returns a map.
    source: String,
    /// Script-owned scratch memory, threaded tick to tick.
    state: Vec<i64>,
}

/// Behaviour-tree leaf: gather input, run this entity's [`AlvikScript`],
/// apply the output. Loop it with [`Repeat`] for a live controller.
#[action(handler_only)]
#[derive(Default, Clone, Component, Reflect)]
#[reflect(Component)]
#[type_path = "alvik"]
#[require(AlvikScript)]
pub fn AlvikScriptStep(
    cx: In<ActionContext>,
    time: Res<Time>,
    mut scripts: Query<&mut AlvikScript>,
    sensors: Single<(&Tof, &LineSensors, &Orientation, &TouchValue), With<AlvikRobot>>,
    mut drive: Single<&mut DifferentialDrive, With<AlvikRobot>>,
    mut leds: Query<(&AlvikLed, &mut LedColor)>,
) -> Outcome {
    let Ok(mut script) = scripts.get_mut(cx.id()) else {
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
    };

    match run_alvik_script(&script.source, &input, &script.state) {
        Ok(output) => {
            drive.linear = LinearVelocity::from_mm_per_sec(output.linear_mm_s);
            drive.angular = AngularVelocity::from_deg_per_sec(output.angular_deg_s);
            for (led, mut color) in &mut leds {
                let packed = match led.side {
                    Side::Left => output.led_left,
                    Side::Right => output.led_right,
                };
                color.0 = unpack_color(packed);
            }
            info!(
                "scene: script -> ({} mm/s, {} deg/s) led {:#08x}/{:#08x}",
                output.linear_mm_s, output.angular_deg_s, output.led_left, output.led_right
            );
            script.state = output.state;
        }
        Err(err) => warn!("scene: script error: {}", err.as_str()),
    }
    Outcome::PASS
}

/// Run `source` against the gathered `input` + `state`, returning the
/// parsed [`AlvikOutput`]. Marshals through rhai [`Map`]s (no serde).
fn run_alvik_script(
    source: &str,
    input: &AlvikInput,
    state: &[i64],
) -> core::result::Result<AlvikOutput, String> {
    let engine = Engine::new();
    let mut scope = Scope::new();

    let mut map = Map::new();
    map.insert("elapsed_ms".into(), Dynamic::from_int(input.elapsed_ms));
    map.insert("depth_mm".into(), Dynamic::from_int(input.depth_mm));
    map.insert("line_left".into(), Dynamic::from_int(input.line_left));
    map.insert("line_center".into(), Dynamic::from_int(input.line_center));
    map.insert("line_right".into(), Dynamic::from_int(input.line_right));
    map.insert("yaw_deg".into(), Dynamic::from_float(input.yaw_deg));
    map.insert("touch".into(), Dynamic::from_int(input.touch));
    scope.push("input", map);
    scope.push(
        "state",
        state.iter().copied().map(Dynamic::from_int).collect::<rhai::Array>(),
    );

    let result = engine
        .eval_with_scope::<Dynamic>(&mut scope, source)
        .map_err(|err| format!("{err}"))?;
    let out = result
        .try_cast::<Map>()
        .ok_or_else(|| String::from("script must return a map"))?;

    let float = |key: &str| {
        out.get(key)
            .and_then(|value| value.as_float().ok().or_else(|| value.as_int().ok().map(|int| int as f64)))
            .unwrap_or(0.0)
    };
    let int = |key: &str| out.get(key).and_then(|value| value.as_int().ok()).unwrap_or(0);
    let next_state = out
        .get("state")
        .and_then(|value| value.clone().try_cast::<rhai::Array>())
        .map(|array| array.iter().filter_map(|value| value.as_int().ok()).collect())
        .unwrap_or_default();

    Ok(AlvikOutput {
        linear_mm_s: float("linear") as f32,
        angular_deg_s: float("angular") as f32,
        led_left: int("led_left") as u32,
        led_right: int("led_right") as u32,
        state: next_state,
    })
}

/// Unpack a `0xRRGGBB` integer into a [`Color`].
fn unpack_color(packed: u32) -> Color {
    Color::srgb_u8((packed >> 16) as u8, (packed >> 8) as u8, packed as u8)
}

/// A demo script: back off when something is within 20 cm, otherwise cruise,
/// pulsing the LEDs from a counter held in `state`.
const DEMO_SCRIPT: &str = r#"
let t = if state.len() > 0 { state[0] } else { 0 };
let near = input.depth_mm > 0 && input.depth_mm < 200;
let bright = if t % 6 < 3 { 255 } else { 20 };
#{
    linear: if near { -40.0 } else { 50.0 },
    angular: if near { 90.0 } else { 0.0 },
    led_left: bright * 256,
    led_right: bright,
    state: [t + 1],
}
"#;

/// `script` — repeat the demo [`AlvikScript`] every 100 ms.
pub fn script_scene() -> impl Bundle {
    (ActionRoute, PathPartial::new("script"), children![(
        Repeat::new(),
        children![(
            Sequence::new(),
            children![
                (
                    AlvikScriptStep,
                    AlvikScript {
                        source: String::from(DEMO_SCRIPT),
                        state: Vec::new(),
                    },
                ),
                EndInDuration::pass(Duration::from_millis(100)),
            ],
        )],
    )])
}
