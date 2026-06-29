//! The [`AlvikPlugin`]: the Alvik driver and per-frame transport systems.

use crate::alvik::driver;
use crate::alvik::events;
use crate::alvik::systems;
use beet::prelude::*;

/// Installs the Alvik driver and transport. Add after
/// [`Esp32Plugin`](crate::esp32_plugin) (which exposes UART1 + the Alvik GPIOs)
/// and [`LedPlugin`](crate::utils::led::LedPlugin) (the `AlvikLed` backend reuses
/// [`LedColor`](crate::utils::led::LedColor)).
///
/// The robot entity tree itself is no longer spawned here: the firmware's `setup`
/// (in `main.rs`) spawns the [`AlvikRobot`] as the scene *root* — the bundle the
/// [`Alvik`](super::scenes::Alvik) element assembles — with the scene server as a
/// child, so the robot is the root ancestor a loaded behaviour's agent resolves
/// to. The transport systems below query `With<AlvikRobot>` and so do not care
/// where the robot sits in the tree. To run logic when the robot appears, add an
/// observer on `On<Add, AlvikRobot>`.
pub struct AlvikPlugin;

impl Plugin for AlvikPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, driver::spawn_alvik_driver)
            // Read sensors, then detect touch/move edges off the fresh state.
            .add_systems(
                PreUpdate,
                (systems::apply_status, events::detect_events).chain(),
            )
            // Write any changed command components after app logic.
            .add_systems(
                PostUpdate,
                (
                    systems::flush_wheels,
                    systems::flush_drive,
                    systems::flush_servos,
                    systems::flush_alvik_leds,
                ),
            );
    }
}
