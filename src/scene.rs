//! ESP-specific scene wiring. The hardware-agnostic scene server — the bootstrap
//! HTTP meta-routes ([`LoadScene`], [`ClearScene`], …), [`SpawnAction`], the
//! [`BeetSceneRoot`] marker and the [`ResetScene`] event — now lives upstream in
//! [`beet::router`]'s `scene_management`. Here we add the firmware's own scene
//! types (the script steps plus their [`ScriptState`](crate::scripting::ScriptState))
//! and the [`ResetScene`] handlers for its hardware (LEDs, and under `alvik` the
//! robot).

use alloc::string::String;
use beet::prelude::*;

pub mod prelude {
    pub use super::At;
    pub use super::EspScenePlugin;
    pub use super::Loop;
    pub use super::RouteAction;
    pub use super::Steps;
    pub use super::Wait;
    #[cfg(all(feature = "led", any(feature = "rhai", feature = "quickjs")))]
    pub use super::LedScript;
    #[cfg(feature = "led")]
    pub use super::reset_leds;
}

/// Registers the firmware's scene capabilities on top of the upstream
/// [`SceneServerPlugin`]: the script step types plus their
/// [`ScriptState`](crate::scripting::ScriptState), and the [`ResetScene`]
/// handlers for the on-board LED (and, under `alvik`, the robot). The router,
/// `Sequence`/`Repeat` and `RunTimer` types are covered by
/// [`RouterPlugin`]/[`ActionPlugin`]. The typed `Script` and its runtime come
/// from beet_action.
pub struct EspScenePlugin;

impl Plugin for EspScenePlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(SceneServerPlugin)
            .register_type::<EndInDuration>()
            // The BSX scene-authoring widgets (see below), so a pushed `.bsx`
            // resolves `<RouteAction>`/`<Loop>`/`<Steps>`/`<Wait>` as components.
            .register_type::<RouteAction>()
            .register_type::<At>()
            .register_type::<Loop>()
            .register_type::<Steps>()
            .register_type::<Wait>();
        // The persistent state every stateful script step threads.
        #[cfg(feature = "scripting")]
        app.register_type::<crate::scripting::ScriptState>();
        // The LED script step plus its typed `Script` data, and the `<LedScript>`
        // BSX authoring façade over them.
        #[cfg(all(feature = "led", any(feature = "rhai", feature = "quickjs")))]
        app.register_type::<crate::scripting::LedScriptStep>()
            .register_type::<Script<
                crate::scripting::LedInput,
                crate::scripting::LedOutput,
            >>()
            .register_type::<LedScript>();
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

// ---------------------------------------------------------------------------
// BSX scene-authoring widgets
// ---------------------------------------------------------------------------
// Non-generic component tags so a pushed `.bsx` scene reads as a behaviour tree.
// The tree primitives (`Repeat<_>`, `Sequence<_>`, `EndInDuration<_>`) are
// generic, and a BSX tag resolves a registered *short type path* (generics and
// all), so a bare `<Repeat>` cannot match `Repeat<()>`. These wrappers are the
// non-generic façade: each requires or inserts the concrete behaviour component.
// Being components (not `#[template]`s) their markup children build as real ECS
// children — the parent/child shape the behaviour tree runs on.

/// `<RouteAction path="roomba">` — install a behaviour-tree route at `path`. Its
/// first child is the tree, spawned and run when the path is called (eg
/// `beet run roomba`).
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

/// `<DriveRoute {At{path:"drive/:dir"}}/>` — bind a route path to a direct
/// `#[action(route)]` handler (vs the behaviour-tree [`RouteAction`]). Spread onto
/// the handler tag, it inserts the `PathPartial` the router dispatches it on.
#[derive(Debug, Default, Component, Reflect)]
#[reflect(Component, Default)]
#[component(on_add = at_on_add)]
pub struct At {
    /// The route path, eg `drive/:dir`.
    pub path: String,
}

/// Insert the `PathPartial` parsed from the declared path.
fn at_on_add(mut world: DeferredWorld, cx: HookContext) {
    let path = world
        .entity(cx.entity)
        .get::<At>()
        .map(|at| at.path.clone())
        .unwrap_or_default();
    world
        .commands()
        .entity(cx.entity)
        .insert(PathPartial::new(&path));
}

/// `<Loop>` — repeat its single child forever (the non-generic `Repeat` façade).
#[derive(Debug, Default, Component, Reflect)]
#[reflect(Component, Default)]
#[require(Repeat<()>)]
pub struct Loop;

/// `<Steps>` — run its children in order each tick (the non-generic `Sequence`
/// façade).
#[derive(Debug, Default, Component, Reflect)]
#[reflect(Component, Default)]
#[require(Sequence<(), ()>)]
pub struct Steps;

/// `<Wait ms="50"/>` — a behaviour-tree leaf that passes after `ms` milliseconds,
/// the timer that paces a [`Loop`].
#[derive(Debug, Default, Component, Reflect)]
#[reflect(Component, Default)]
#[component(on_add = wait_on_add)]
pub struct Wait {
    /// Milliseconds before the leaf passes.
    pub ms: u64,
}

/// Insert the concrete `EndInDuration` timer from the declared milliseconds.
fn wait_on_add(mut world: DeferredWorld, cx: HookContext) {
    let ms = world
        .entity(cx.entity)
        .get::<Wait>()
        .map(|wait| wait.ms)
        .unwrap_or_default();
    world
        .commands()
        .entity(cx.entity)
        .insert(EndInDuration::pass(Duration::from_millis(ms)));
}

/// `<LedScript rhai="...">` — a behaviour-tree leaf running a rhai LED program
/// each tick (`input.elapsed_ms`/`input.led`/`input.state` -> `#{ led, state }`).
/// The non-generic façade over `(LedScriptStep, Script<LedInput, LedOutput>)`, so
/// the on-board WS2812 is programmable from a pushed `.bsx`.
#[cfg(all(feature = "led", any(feature = "rhai", feature = "quickjs")))]
#[derive(Debug, Default, Component, Reflect)]
#[reflect(Component, Default)]
#[component(on_add = led_script_on_add)]
pub struct LedScript {
    /// The rhai source run each tick.
    pub rhai: String,
}

/// Insert `(LedScriptStep, Script::rhai(..))` from the declared source.
#[cfg(all(feature = "led", any(feature = "rhai", feature = "quickjs")))]
fn led_script_on_add(mut world: DeferredWorld, cx: HookContext) {
    let source = world
        .entity(cx.entity)
        .get::<LedScript>()
        .map(|step| step.rhai.clone())
        .unwrap_or_default();
    world.commands().entity(cx.entity).insert((
        crate::scripting::LedScriptStep,
        Script::<crate::scripting::LedInput, crate::scripting::LedOutput>::rhai(
            source,
        ),
    ));
}
