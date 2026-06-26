//! ESP-specific scene wiring. The hardware-agnostic scene server — the bootstrap
//! HTTP meta-routes ([`LoadScene`], [`ClearScene`], …), [`SpawnAction`], the
//! [`BeetSceneRoot`] marker and the [`ResetScene`] event — lives upstream in
//! [`beet::router`]'s `scene_management`. Here we add only the firmware's own
//! scene types: the [`RouteAction`] path binder, the [`ResetScene`] handlers for
//! its hardware (LEDs, and under `alvik` the robot), and the [`LedScript`]
//! authoring template.
//!
//! A pushed `.bsx` scene reads as a behaviour tree authored straight from the
//! upstream primitives — `<Repeat>`/`<Sequence>` (the generic `Repeat<()>` /
//! `Sequence<(), ()>` resolve from a bare tag), `<EndInDuration duration="50ms"/>`
//! (a delay leaf, the duration coerced from a number/string by beet's BSX engine),
//! and `<LedScript>`/`<AlvikScript>` (script leaves). No non-generic alias
//! components stand in for them.

use alloc::string::String;
use beet::prelude::*;

pub mod prelude {
    pub use super::EspScenePlugin;
    pub use super::RouteAction;
    #[cfg(all(feature = "led", any(feature = "rhai", feature = "quickjs")))]
    pub use super::LedScript;
    #[cfg(feature = "led")]
    pub use super::reset_leds;
}

/// Registers the firmware's scene capabilities on top of the upstream
/// [`SceneServerPlugin`]: the [`RouteAction`] path binder, the [`ScriptState`] a
/// stateful script threads, and the [`ResetScene`] handler for the on-board LED
/// (and, under `alvik`, the robot). The router, `Sequence`/`Repeat` and
/// `EndInDuration` types are covered by [`RouterPlugin`]/[`ActionPlugin`]; the
/// typed `Script` and its runtime come from beet_action.
pub struct EspScenePlugin;

impl Plugin for EspScenePlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(SceneServerPlugin)
            .register_type::<RouteAction>()
            // The behaviour-tree delay leaf. Registered already by `ActionPlugin`,
            // but augment its registration with `ReflectDefault` so a partial
            // `<EndInDuration duration="50ms"/>` patch (only the duration) merges
            // over `EndInDuration::default()` (value = `Outcome::PASS`) on load.
            .register_type::<EndInDuration<(), Outcome>>()
            .register_type_data::<EndInDuration<(), Outcome>, ReflectDefault>();
        // The persistent state every stateful script step threads.
        #[cfg(feature = "scripting")]
        app.register_type::<crate::scripting::ScriptState>();
        // The LED script step plus its typed `Script` data, and the `<LedScript>`
        // authoring template over them.
        #[cfg(all(feature = "led", any(feature = "rhai", feature = "quickjs")))]
        app.register_type::<crate::scripting::LedScriptStep>()
            .register_type::<Script<
                crate::scripting::LedInput,
                crate::scripting::LedOutput,
            >>()
            .register_template::<LedScript>();
        #[cfg(feature = "led")]
        app.add_observer(reset_leds);
        #[cfg(feature = "alvik")]
        app.add_plugins(crate::alvik::scenes::AlvikScenePlugin);
    }
}

/// Generic [`ResetScene`] handler: turn every LED off. Domain plugins add their
/// own observers to stop their actuators (the Alvik its motors).
#[cfg(feature = "led")]
pub fn reset_leds(_ev: On<ResetScene>, mut leds: Query<&mut crate::utils::led::LedColor>) {
    for mut color in &mut leds {
        color.0 = Color::BLACK;
    }
}

/// `<RouteAction path="roomba">` — install a behaviour-tree route at `path`. Its
/// first child is the tree, spawned and run when the path is called (eg
/// `beet run roomba`).
///
/// The one path-binding component the firmware keeps: a route's path is a string,
/// but [`PathPartial`]'s field is a parsed segment list, so a string-to-path
/// bridge is unavoidable. Being a component (not a `#[template]`) its markup
/// children build as real ECS children — the parent/child shape the behaviour
/// tree runs on, and the single child [`SpawnAction`] runs on call.
#[derive(Debug, Default, Component, Reflect)]
#[reflect(Component, Default)]
#[component(on_add = route_action_on_add)]
pub struct RouteAction {
    /// The route path, eg `roomba` or `drive/:dir`.
    pub path: String,
}

/// Insert `(PathPartial, SpawnAction)` from the declared path, turning the entity
/// into a scene route whose child tree runs on call.
fn route_action_on_add(mut world: DeferredWorld, cx: HookContext) {
    let path = world
        .entity(cx.entity)
        .get::<RouteAction>()
        .map(|route| route.path.clone())
        .unwrap_or_default();
    world
        .commands()
        .entity(cx.entity)
        .insert((PathPartial::new(&path), SpawnAction));
}

/// `<LedScript script="..." language="rhai">` — a behaviour-tree leaf running a
/// script LED program each tick (`input.elapsed_ms`/`input.led`/`input.state` ->
/// `#{ led, state }`). The non-generic authoring template over `(LedScriptStep,
/// Script<LedInput, LedOutput>)`, so the on-board WS2812 is programmable from a
/// pushed `.bsx`.
///
/// `language` selects the backend ([`ScriptLanguage::from_str`]), falling back to
/// the build default when absent — so the same scene runs under rhai or quickjs.
/// Mirrors upstream's [`ScriptRoute`] template (see `beet::router`).
#[cfg(all(feature = "led", any(feature = "rhai", feature = "quickjs")))]
#[template]
pub fn LedScript(
    #[prop(into)] script: String,
    language: Option<String>,
) -> impl Bundle {
    let language = language
        .and_then(|name| name.parse::<ScriptLanguage>().ok())
        .unwrap_or_default();
    (
        crate::scripting::LedScriptStep,
        Script::<crate::scripting::LedInput, crate::scripting::LedOutput>::new(
            language, script,
        ),
    )
}
