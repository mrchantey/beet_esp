//! A hardware-agnostic **scene server**: the firmware ships a tiny bootstrap
//! HTTP API ([`server`]) whose real routes arrive over the wire as a beet
//! scene. A scene (a reflection-serialized slice of the ECS) POSTed to `/load`
//! *becomes* the API; the behaviours it wires — [`ActionRoute`], the behaviour
//! tree leaves, and rhai [`Script`]s — live here, and the scene supplies only
//! which behaviour sits at which path.
//!
//! [`SceneServerPlugin`] registers every type a generic scene can carry. The
//! Alvik adds its own routes/actions and a reset handler behind the `alvik`
//! feature; the example (`examples/scenes.rs`) runs the same server on a bare
//! ESP32 (driving the on-board WS2812 via a [`Script`]) or on an Alvik.

pub mod routes;
#[cfg(feature = "rhai")]
pub mod script;
pub mod server;

use beet::prelude::*;

pub mod prelude {
    pub use super::SceneServerPlugin;
    pub use super::routes::*;
    #[cfg(feature = "rhai")]
    pub use super::script::*;
    pub use super::server::*;
}

/// Registers every route/action/scene type a generic scene can carry, so
/// reflection can (de)serialize it, and installs the generic [`ResetScene`]
/// handler. The router, `Sequence`/`Repeat` and `RunTimer` types are covered by
/// [`RouterPlugin`]/[`ActionPlugin`]. Under the `alvik` feature it also adds the
/// Alvik scene types via [`AlvikScenePlugin`](crate::alvik::scenes::AlvikScenePlugin).
pub struct SceneServerPlugin;

impl Plugin for SceneServerPlugin {
    fn build(&self, app: &mut App) {
        app.register_type::<self::routes::ActionRoute>()
            .register_type::<EndInDuration>();
        #[cfg(feature = "rhai")]
        app.register_type::<self::script::Script>();
        #[cfg(all(feature = "rhai", feature = "led"))]
        app.register_type::<self::script::LedScriptStep>();
        #[cfg(feature = "led")]
        app.add_observer(self::server::reset_leds);

        #[cfg(feature = "alvik")]
        app.add_plugins(crate::alvik::scenes::AlvikScenePlugin);
    }
}
