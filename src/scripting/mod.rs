//! Scene-carried control scripting. A scene ships a beet [`Script`] whose
//! JavaScript program the embedded QuickJS engine (the bundled C engine,
//! `no_std` over esp-alloc) evaluates each tick. The typed
//! `Script<Input, Output>` and its [`Value`]-marshalled runtime live upstream in
//! beet_action; here we add only the per-domain *step* actions that gather a
//! domain input, run the script and apply its output, plus the [`ScriptState`]
//! a stateful script threads tick to tick.
//!
//! Every input/output type is plain serde: [`Value`] is the marshalling
//! currency in and out of the engine, so a step's only job is to build its typed
//! input and apply its typed output, with no engine-specific glue.

#[cfg(feature = "quickjs")]
pub mod quickjs;

#[cfg(feature = "led")]
pub use color::*;
#[cfg(all(feature = "led", feature = "quickjs"))]
pub use led_step::*;
#[cfg(feature = "scripting")]
pub use script_state::*;

pub mod prelude {
    #[cfg(feature = "quickjs")]
    pub use super::quickjs::{RuntimeEspExt, install_console};
    #[cfg(feature = "led")]
    pub use super::color::*;
    #[cfg(all(feature = "led", feature = "quickjs"))]
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
    use serde::Serialize;
    use serde::de::DeserializeOwned;

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

    impl ScriptState {
        /// Run one tick of a domain script step on the caller: `gather` builds
        /// the input from the world and the current state, the caller's
        /// [`Script`] evaluates it under its [`ScriptConfig`] (default when
        /// absent), and `apply` writes the output back, returning the state to
        /// thread to the next tick.
        ///
        /// A missing script or input (eg the hardware is not up yet) skips the
        /// tick, and a script error is logged rather than failing the leaf, so a
        /// live [`Repeat`] loop survives one bad tick.
        pub async fn step<Input, Output>(
            cx: ActionContext,
            gather: impl 'static
            + Send
            + FnOnce(&mut World, HashMap<String, Value>) -> Option<Input>,
            apply: impl 'static + Send + FnOnce(&mut World, Output) -> HashMap<String, Value>,
        ) -> Result<Outcome>
        where
            Input: 'static + Send + Sync + Serialize,
            Output: 'static + Send + Sync + DeserializeOwned,
        {
            let entity = cx.id();
            let world = cx.world().clone();
            // the script, its grant and the input are cloned out in one access:
            // the eval is awaited, and the world moves on in the meantime.
            let Some((script, config, input)) = world
                .with(move |world: &mut World| {
                    let script = world.get::<Script<Input, Output>>(entity)?.clone();
                    let config = world.get::<ScriptConfig>(entity).cloned().unwrap_or_default();
                    let state = world.get::<ScriptState>(entity)?.0.clone();
                    gather(world, state).map(|input| (script, config, input))
                })
                .await
            else {
                return Outcome::PASS.xok();
            };
            match script.run(input, world.clone(), &config).await {
                Ok(output) => {
                    world
                        .with(move |world: &mut World| {
                            let state = apply(world, output);
                            if let Some(mut current) = world.get_mut::<ScriptState>(entity) {
                                current.0 = state;
                            }
                        })
                        .await
                }
                Err(err) => warn!("scene: script error: {err}"),
            }
            Outcome::PASS.xok()
        }
    }
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

#[cfg(all(feature = "led", feature = "quickjs"))]
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
    /// returning `({ led, state })`.
    #[action(local)]
    #[derive(Default, Clone, Component, Reflect)]
    #[reflect(Component)]
    #[type_path = "scene"]
    #[require(Script<LedInput, LedOutput>, ScriptState)]
    pub async fn LedScriptStep(cx: ActionContext) -> Result<Outcome> {
        ScriptState::step(
            cx,
            |world, state| {
                world.with_state::<(Res<Time>, Query<&LedColor, With<Ws2812Led>>), _>(
                    |(time, leds)| {
                        LedInput {
                            elapsed_ms: (time.elapsed_secs_f64() * 1000.0) as i64,
                            led: pack_color(leds.single().ok()?.0),
                            state,
                        }
                        .xsome()
                    },
                )
            },
            |world, output: LedOutput| {
                if let Some(packed) = output.led {
                    world.with_state::<Query<&mut LedColor, With<Ws2812Led>>, _>(
                        |mut leds| {
                            for mut led in &mut leds {
                                led.0 = unpack_color(packed);
                            }
                        },
                    );
                    info!("scene: led script -> {:#08x}", packed);
                }
                output.state
            },
        )
        .await
    }
}
