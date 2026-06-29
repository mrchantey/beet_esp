//! The [`AlvikPlugin`] and the [`spawn_robot`] startup system it installs.

use crate::alvik::components::*;
use crate::alvik::driver;
use crate::alvik::events;
use crate::alvik::systems;
use crate::alvik::types::Side;
use crate::utils::led::LedColor;
use beet::prelude::*;

/// Installs the Alvik driver and transport, and spawns the robot entity tree.
/// Add after [`Esp32Plugin`](crate::esp32_plugin) (which exposes UART1 + the
/// Alvik GPIOs) and [`LedPlugin`](crate::utils::led::LedPlugin) (the `AlvikLed` backend
/// reuses [`LedColor`]).
///
/// The plugin spawns the robot itself, so apps just add the plugin. To run logic
/// when the robot appears, add an observer on `On<Add, AlvikRobot>`.
pub struct AlvikPlugin;

impl Plugin for AlvikPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, (driver::spawn_alvik_driver, spawn_robot))
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

/// Spawn the Alvik robot entity tree: the [`AlvikRobot`] root carrying every
/// sensor/state component, with two wheel, two servo and two RGB-LED children.
/// Apps read and write these components; unused ones cost only a default.
pub fn spawn_robot(mut commands: Commands) {
    commands.spawn((
        AlvikRobot,
        // Grouped into sub-bundles: a flat tuple would exceed the 15-element
        // `Bundle` impl. State, then sensors.
        (
            Connected::default(),
            FirmwareVersion::default(),
            BehaviorCode::default(),
            BatterState::default(),
            DifferentialDrive::default(),
            Illuminator(true),
            BuiltinLed(false),
        ),
        (
            LineSensors::default(),
            ColorSensor::default(),
            Tof::default(),
            Imu::default(),
            Orientation::default(),
            RobotPose::default(),
            // Commanded velocity: the upstream `Drive` leaf writes these on the
            // robot (its agent), and `flush_drive` sends them to the wire.
            LinearVelocity::default(),
            AngularVelocity::default(),
            TouchValue::default(),
            MotionValue::default(),
        ),
        children![
            (
                Wheel { side: Side::Left },
                WheelState::default(),
                WheelTarget::Speed(AngularVelocity::default()),
            ),
            (
                Wheel { side: Side::Right },
                WheelState::default(),
                WheelTarget::Speed(AngularVelocity::default()),
            ),
            (Servo {
                id: ServoId::A,
                position: Angle::from_degrees(90.0),
            }),
            (Servo {
                id: ServoId::B,
                position: Angle::from_degrees(90.0),
            }),
            (AlvikLed { side: Side::Left }, LedColor::default()),
            (AlvikLed { side: Side::Right }, LedColor::default()),
        ],
    ));
}
