//! Scene-carried control scripting. A scene ships a beet [`Script`] whose
//! program a backend engine evaluates each tick — rhai
//! ([`beet::exports::rhai`], pure-Rust `no_std`) or quickjs (the bundled C
//! engine). The typed `Script<Input, Output>` and its [`Value`]-marshalled
//! runtimes now live upstream in beet_action; here we add only the per-domain
//! *step* actions that gather a domain input, run the script and apply its
//! output, plus the [`ScriptState`] a stateful script threads tick to tick.
//!
//! Every input/output type is plain serde: [`Value`] is the marshalling
//! currency in and out of the engine, so a step's only job is to build its typed
//! input and apply its typed output — no engine-specific glue.

#[cfg(feature = "quickjs")]
pub mod quickjs;

#[cfg(feature = "led")]
pub use color::*;
#[cfg(all(feature = "led", any(feature = "rhai", feature = "quickjs")))]
pub use led_step::*;
#[cfg(feature = "scripting")]
pub use script_state::*;

pub mod prelude {
    #[cfg(feature = "quickjs")]
    pub use super::quickjs::{RuntimeEspExt, install_console};
    #[cfg(feature = "led")]
    pub use super::color::*;
    #[cfg(all(feature = "led", any(feature = "rhai", feature = "quickjs")))]
    pub use super::led_step::*;
    #[cfg(feature = "scripting")]
    pub use super::script_state::*;
}

// ---------------------------------------------------------------------------
// Persistent script state (engine- and domain-agnostic).
// ---------------------------------------------------------------------------

#[cfg(feature = "scripting")]
mod script_state {
    use beet::prelude::*;

    /// Persistent scratch memory a stateful [`Script`] step threads tick to
    /// tick: a string-keyed map of reflectable [`Value`]s, so a scene can ship
    /// any scratch shape (a counter, a mode, a list) rather than a fixed array.
    /// A step passes it in as the script's `input.state` and stores back
    /// whatever the script returns as `state`. Defaults to an empty map, so a
    /// script can probe `"key" in input.state` on its first tick.
    #[derive(Debug, Default, Clone, Component, Reflect)]
    #[reflect(Component, Default)]
    #[type_path = "scene"]
    pub struct ScriptState(pub HashMap<String, Value>);
}

// ---------------------------------------------------------------------------
// WS2812 colour packing, shared by the LED step and the Alvik UI LEDs.
// ---------------------------------------------------------------------------

#[cfg(feature = "led")]
mod color {
    use beet::prelude::*;

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

// ---------------------------------------------------------------------------
// The on-board WS2812 LED step: the generic (non-Alvik) script demo.
// ---------------------------------------------------------------------------

#[cfg(all(feature = "led", any(feature = "rhai", feature = "quickjs")))]
mod led_step {
    use super::ScriptState;
    use super::color::*;
    use crate::utils::led::LedColor;
    use crate::utils::led::Ws2812Led;
    use beet::prelude::*;

    /// The snapshot handed to the LED [`Script`] each tick, bound to `input`.
    /// `Reflect` only so `Script<LedInput, LedOutput>` has a type path to
    /// register; the value itself is marshalled through serde, never reflected.
    #[derive(Serialize, Reflect)]
    pub struct LedInput {
        /// Milliseconds since boot.
        pub elapsed_ms: i64,
        /// The LED's current colour, packed `0xRRGGBB`.
        pub led: u32,
        /// The script's persistent state (see [`ScriptState`]).
        pub state: HashMap<String, Value>,
    }

    /// The map the LED [`Script`] returns each tick.
    #[derive(Deserialize, Reflect)]
    pub struct LedOutput {
        /// New LED colour packed `0xRRGGBB`; the colour is left unchanged when
        /// the script omits it.
        #[serde(default)]
        pub led: Option<u32>,
        /// The next persistent state to thread to the following tick.
        #[serde(default)]
        pub state: HashMap<String, Value>,
    }

    /// Behaviour-tree leaf: feed the elapsed time and the LED's current colour
    /// to this entity's [`Script`], then apply the colour it returns. Loop it
    /// with [`Repeat`] for a live LED program. The script reads
    /// `input.elapsed_ms`, `input.led` (packed `0xRRGGBB`) and `input.state`,
    /// returning `#{ led, state }`.
    #[action(handler_only)]
    #[derive(Default, Clone, Component, Reflect)]
    #[reflect(Component)]
    #[type_path = "scene"]
    #[require(Script<LedInput, LedOutput>, ScriptState)]
    pub fn LedScriptStep(
        cx: In<ActionContext>,
        time: Res<Time>,
        mut scripts: Query<(&Script<LedInput, LedOutput>, &mut ScriptState)>,
        mut leds: Query<&mut LedColor, With<Ws2812Led>>,
    ) -> Outcome {
        let Ok((script, mut state)) = scripts.get_mut(cx.id()) else {
            return Outcome::PASS;
        };
        let Ok(mut led) = leds.single_mut() else {
            return Outcome::PASS;
        };

        let input = LedInput {
            elapsed_ms: (time.elapsed_secs_f64() * 1000.0) as i64,
            led: pack_color(led.0),
            state: state.0.clone(),
        };
        match script.run(input) {
            Ok(output) => {
                if let Some(packed) = output.led {
                    led.0 = unpack_color(packed);
                    info!("scene: led script -> {:#08x}", packed);
                }
                state.0 = output.state;
            }
            Err(err) => warn!("scene: script error: {}", err),
        }
        Outcome::PASS
    }
}
