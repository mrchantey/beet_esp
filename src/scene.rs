//! ESP-specific scene wiring. The hardware-agnostic scene server — the bootstrap
//! HTTP meta-routes ([`LoadScene`], [`ClearScene`], …), [`SpawnAction`], the
//! [`BeetSceneRoot`] marker and the [`ResetScene`] event — now lives upstream in
//! [`beet::router`]'s `scene_management`. Here we add the firmware's own scene
//! types (rhai [`Script`](crate::scripting::rhai::Script)s) and the
//! [`ResetScene`] handlers for its hardware (LEDs, and under `alvik` the robot).

use beet::prelude::*;

pub mod prelude {
    pub use super::EspScenePlugin;
    #[cfg(feature = "led")]
    pub use super::reset_leds;
}

/// Registers the firmware's scene capabilities on top of the upstream
/// [`SceneServerPlugin`]: the rhai [`Script`](crate::scripting::rhai::Script)
/// types and the [`ResetScene`] handlers for the on-board LED (and, under
/// `alvik`, the robot). The router, `Sequence`/`Repeat` and `RunTimer` types are
/// covered by [`RouterPlugin`]/[`ActionPlugin`].
pub struct EspScenePlugin;

impl Plugin for EspScenePlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(SceneServerPlugin)
            .register_type::<EndInDuration>();
        #[cfg(feature = "rhai")]
        app.register_type::<crate::scripting::rhai::Script>();
        #[cfg(all(feature = "rhai", feature = "led"))]
        app.register_type::<crate::scripting::rhai::LedScriptStep>();
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
