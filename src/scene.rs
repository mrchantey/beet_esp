//! ESP-specific scene wiring. The hardware-agnostic scene server — the bootstrap
//! HTTP meta-routes ([`LoadScene`], [`ClearScene`], …), [`ActionRoute`], the
//! [`BeetSceneRoot`] marker and the [`ResetScene`] event — now lives upstream in
//! [`beet::router`]'s `scene_management`. Here we add the firmware's own scene
//! types (rhai [`Script`](crate::scripting::rhai::Script)s) and the
//! [`ResetScene`] handlers for its hardware (LEDs, and under `alvik` the robot),
//! plus an on-device [`log_scene`] helper that dumps a scene over defmt.

use beet::prelude::*;
use defmt::Debug2Format;
use defmt::info;
use defmt::warn;

extern crate alloc;
use alloc::string::String;

pub mod prelude {
    pub use super::EspScenePlugin;
    pub use super::log_scene;
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

/// Serialize a one-root scene `bundle` to JSON and log it over defmt, so it can
/// be saved and POSTed to `/load`. The canonical example scenes are dumped this
/// way on boot.
pub fn log_scene(world: &mut World, label: &str, bundle: impl Bundle) {
    let root = world.spawn(bundle).id();
    let bytes = WorldSerdeSaver::new(world)
        .with_entity_tree(root)
        .save(MediaType::Json);
    world.entity_mut(root).despawn();
    match bytes.and_then(|bytes| bytes.as_utf8().map(String::from)) {
        Ok(json) => info!("scene[{}]:\n{}", label, json.as_str()),
        Err(err) => warn!("scene[{}] dump failed: {}", label, Debug2Format(&err)),
    }
}
