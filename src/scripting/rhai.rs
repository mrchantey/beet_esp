//! Scene-carried rhai control scripts, engine-agnostic in shape: a [`Script`]
//! component holds the program plus its persistent state, and [`run_script`]
//! runs it for one tick. The per-domain step systems ([`LedScriptStep`] here,
//! [`AlvikScriptStep`](crate::alvik::scripting) under the `alvik` feature)
//! gather a domain input map, call [`run_script`], then apply the output map —
//! so the input/output shapes diverge while the run path is shared.
//!
//! beet's own `Script` action is std-only (rhai pulls std there), so this wires
//! rhai directly through [`beet::exports::rhai`]: it builds `no_std`, and the
//! maps are marshalled to/from rhai by hand (serde's `no_std` split fights
//! rhai's). The persistent state is a reflectable [`Value`] map, so a scene can
//! ship any scratch shape rather than a flat array.

use beet::exports::rhai;
use beet::prelude::*;
use rhai::Dynamic;
use rhai::Engine;
use rhai::Scope;

extern crate alloc;
use alloc::format;
use alloc::string::String;

/// A json-like map of string keys to [`Value`]s, the currency [`run_script`]
/// trades in: the input it is given, the output it returns, and the persistent
/// state a [`Script`] threads tick to tick.
pub type ScriptMap = HashMap<String, Value>;

/// A rhai control script carried in a scene. `source` is the program; `state`
/// is the persistent memory it reads and rewrites each tick — a reflectable
/// [`Value`] map, so a scene can ship any scratch shape (a counter, a mode, a
/// list) rather than a fixed array.
#[derive(Debug, Default, Clone, Component, Reflect)]
#[reflect(Component)]
#[type_path = "scene"]
pub struct Script {
    /// rhai source. Reads `input` and `state`, returns a map.
    pub source: String,
    /// Script-owned scratch memory, threaded tick to tick.
    pub state: ScriptMap,
}

/// Run `source` for one tick with `input` and the script's `state` in scope.
/// The script returns a map; its `state` entry (a map) becomes the next
/// persistent state and the remaining entries are the output the caller
/// applies. Marshals to/from rhai by hand (no serde).
pub fn run_script(
    source: &str,
    input: &ScriptMap,
    state: &ScriptMap,
) -> core::result::Result<(ScriptMap, ScriptMap), String> {
    let engine = Engine::new();
    let mut scope = Scope::new();
    scope.push("input", map_to_dynamic(input));
    scope.push("state", map_to_dynamic(state));

    let result = engine
        .eval_with_scope::<Dynamic>(&mut scope, source)
        .map_err(|err| format!("{err}"))?;
    let mut output = match dynamic_to_value(result) {
        Value::Map(map) => map_into_script_map(map),
        _ => return Err(String::from("script must return a map")),
    };
    // the `state` entry is special: it is split out as the next persistent
    // state, leaving the rest of the map as the output the caller applies.
    let next_state = match output.remove("state") {
        Some(Value::Map(map)) => map_into_script_map(map),
        _ => ScriptMap::default(),
    };
    Ok((output, next_state))
}

/// Flatten a beet [`Map`] into a [`ScriptMap`] (string keys).
fn map_into_script_map(map: Map) -> ScriptMap {
    map.0
        .into_iter()
        .map(|(key, value)| (key.to_string(), value))
        .collect()
}

/// Marshal a [`ScriptMap`] into a rhai map [`Dynamic`].
fn map_to_dynamic(map: &ScriptMap) -> Dynamic {
    Dynamic::from_map(
        map.iter()
            .map(|(key, value)| (key.as_str().into(), value_to_dynamic(value)))
            .collect::<rhai::Map>(),
    )
}

/// Marshal a beet [`Value`] into a rhai [`Dynamic`].
fn value_to_dynamic(value: &Value) -> Dynamic {
    match value {
        Value::Null => Dynamic::UNIT,
        Value::Bool(val) => (*val).into(),
        Value::Int(val) => (*val).into(),
        Value::Uint(val) => (*val as i64).into(),
        Value::Float(val) => (*val).into(),
        Value::Str(val) => val.as_str().into(),
        Value::Bytes(val) => Dynamic::from_array(
            val.iter().map(|byte| (*byte as i64).into()).collect::<rhai::Array>(),
        ),
        Value::Map(map) => Dynamic::from_map(
            map.iter()
                .map(|(key, value)| (key.as_str().into(), value_to_dynamic(value)))
                .collect::<rhai::Map>(),
        ),
        Value::List(list) => Dynamic::from_array(
            list.iter().map(value_to_dynamic).collect::<rhai::Array>(),
        ),
    }
}

/// Marshal a rhai [`Dynamic`] back into a beet [`Value`].
fn dynamic_to_value(value: Dynamic) -> Value {
    if value.is_unit() {
        Value::Null
    } else if value.is_bool() {
        Value::Bool(value.as_bool().unwrap_or_default())
    } else if value.is_int() {
        Value::Int(value.as_int().unwrap_or_default())
    } else if value.is_float() {
        Value::Float(value.as_float().unwrap_or_default())
    } else if value.is_string() {
        Value::Str(value.into_string().unwrap_or_default().into())
    } else if value.is_array() {
        Value::List(
            value
                .into_array()
                .unwrap_or_default()
                .into_iter()
                .map(dynamic_to_value)
                .collect(),
        )
    } else if value.is_map() {
        Value::Map(Map(value
            .cast::<rhai::Map>()
            .into_iter()
            .map(|(key, value)| (key.as_str().into(), dynamic_to_value(value)))
            .collect()))
    } else {
        Value::Null
    }
}

// ---------------------------------------------------------------------------
// The on-board WS2812 LED step: the generic (non-Alvik) script demo.
// ---------------------------------------------------------------------------

#[cfg(feature = "led")]
pub use led_step::*;

#[cfg(feature = "led")]
mod led_step {
    use super::*;
    use crate::utils::led::LedColor;
    use crate::utils::led::Ws2812Led;
    use defmt::info;
    use defmt::warn;

    /// Behaviour-tree leaf: feed the elapsed time and the LED's current colour
    /// to this entity's [`Script`], then apply the colour it returns. Loop it
    /// with [`Repeat`] for a live LED program. The script reads
    /// `input.elapsed_ms` + `input.led` (packed `0xRRGGBB`) and returns
    /// `#{ led, state }`.
    #[action(handler_only)]
    #[derive(Default, Clone, Component, Reflect)]
    #[reflect(Component)]
    #[type_path = "scene"]
    #[require(Script)]
    pub fn LedScriptStep(
        cx: In<ActionContext>,
        time: Res<Time>,
        mut scripts: Query<&mut Script>,
        mut leds: Query<&mut LedColor, With<Ws2812Led>>,
    ) -> Outcome {
        let Ok(mut script) = scripts.get_mut(cx.id()) else {
            return Outcome::PASS;
        };
        let Ok(mut led) = leds.single_mut() else {
            return Outcome::PASS;
        };

        let mut input = ScriptMap::default();
        input.insert(
            "elapsed_ms".into(),
            Value::Int((time.elapsed_secs_f64() * 1000.0) as i64),
        );
        input.insert("led".into(), Value::Int(pack_color(led.0) as i64));

        match run_script(&script.source, &input, &script.state) {
            Ok((output, state)) => {
                if let Some(packed) =
                    output.get("led").and_then(|value| value.as_i64().ok())
                {
                    led.0 = unpack_color(packed as u32);
                    info!("scene: led script -> {:#08x}", packed as u32);
                }
                script.state = state;
            }
            Err(err) => warn!("scene: script error: {}", err.as_str()),
        }
        Outcome::PASS
    }

    /// Pack a [`Color`] into a `0xRRGGBB` integer.
    pub fn pack_color(color: Color) -> u32 {
        let srgb = color.to_srgba_u8();
        ((srgb.red as u32) << 16) | ((srgb.green as u32) << 8) | srgb.blue as u32
    }

    /// Unpack a `0xRRGGBB` integer into a [`Color`].
    pub fn unpack_color(packed: u32) -> Color {
        Color::srgb_u8((packed >> 16) as u8, (packed >> 8) as u8, packed as u8)
    }
}
