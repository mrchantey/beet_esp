//! Arduino Alvik support: drive the robot's STM32 carrier over UART from a Bevy
//! app, with all data as components and all behaviour as systems / observers.
//!
//! Add [`AlvikPlugin`] (after [`Esp32Plugin`](crate::esp32_plugin)); it claims
//! UART1 and the STM32 control GPIOs, runs the bring-up handshake on an embassy
//! task, and installs the per-frame transport systems. Spawn the robot entity
//! tree with [`spawn_robot`], then read sensor components and write command
//! components (`WheelTarget`, `DifferentialDrive`, `Servo`, `LedColor`); the
//! systems carry them to and from the wire.
//!
//! The wire protocol (ucPack framing, command/status codes) lives in
//! [`ucpack`] and [`protocol`]; see `agent/plans/alvik-plan.md` for the decode.
//!
//! # Hardware
//!
//! The firmware runs on the Alvik's on-board **Arduino Nano ESP32** (a real
//! Alvik plugs in for testing). Pins follow `pinout_definitions.py` verbatim,
//! centralised in [`pinout`]. Bring-up waits for `CHECK_STM32` to go high (robot
//! powered on), so on a bare breakout board with no Alvik attached the driver
//! just logs "waiting for robot power" and idles, which is a handy smoke test.
//!
//! Verified end to end on a real Alvik: bring-up handshake, every sensor (line,
//! colour, ToF, IMU, orientation, battery, touch), the motor command + `j`/`w`
//! feedback loop, and the RGB UI LEDs.
//!
//! # Flashing
//!
//! Day to day, plain `cargo run` works (the `.cargo/config.toml` runner is
//! `probe-rs`, unchanged from any other ESP32-S3 target):
//!
//! ```shell
//! cargo run --release --no-default-features --features alvik --example alvik-sensors
//! ```
//!
//! ## First flash of a fresh Nano ESP32 (one time only)
//!
//! A factory Nano ESP32 ships with the **Arduino** bootloader, which cannot load
//! an esp-hal app, and while running it enumerates as Arduino CDC (`2341:056b`)
//! so `probe-rs` never sees its JTAG. `probe-rs` / `cargo run` only writes the
//! *app* and relies on a compatible bootloader already being present (that is
//! why a previously-flashed breakout "just works" but a fresh Nano does not), so
//! the very first flash installs the esp-idf bootloader with `espflash`:
//!
//! 1. Enter ROM download mode: bridge the **B1** pad (GPIO0) to the adjacent
//!    **GND** pin with a jumper, and while bridged press **RST** once. The RGB
//!    LED turns purple (yellow on older boards) and the board re-enumerates as
//!    `303a:1001` on `/dev/ttyACM*`.
//! 2. Flash bootloader + partition table + app in one shot (this overwrites the
//!    Arduino bootloader):
//!
//!    ```shell
//!    espflash flash --chip esp32s3 --port /dev/ttyACM0 \
//!        target/xtensa-esp32s3-none-elf/release/examples/alvik-sensors
//!    ```
//!
//! 3. Done. The board now has a valid bootloader and our firmware keeps the
//!    USB-Serial-JTAG exposed, so from here on plain `cargo run` flashes and
//!    monitors it like any other dev board: no jumper, no `espflash`.
//!
//! `espflash` can also reflash later without `cargo run` (`espflash flash ...`,
//! then `probe-rs attach --chip esp32s3 <elf>` for the RTT logs). Note the Nano
//! is also powered from the Alvik battery, so unplugging USB alone does not power
//! cycle it; press RST (or switch off the battery) for a clean reset.

pub mod components;
pub mod driver;
pub mod events;
pub mod pinout;
pub mod protocol;
pub mod systems;
pub mod ucpack;

use crate::led::LedColor;
use crate::units::AngularVelocity;
use beet::prelude::*;

pub mod prelude {
    pub use super::AlvikPlugin;
    pub use super::components::*;
    pub use super::driver::ALVIK_EVENTS;
    pub use super::driver::ALVIK_OUT;
    pub use super::driver::ALVIK_STATE;
    pub use super::events::*;
    pub use super::protocol::Command;
    pub use super::protocol::Side;
    pub use super::protocol::Status;
    pub use super::spawn_robot;
}

use components::*;
use protocol::Side;

/// Installs the Alvik driver and transport. Add after [`Esp32Plugin`] (which
/// exposes UART1 + the Alvik GPIOs) and [`LedPlugin`](crate::led::LedPlugin)
/// (the `AlvikLed` backend reuses `LedColor`).
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

/// Spawn the Alvik robot entity tree: the [`AlvikRobot`] root carrying every
/// sensor/state component, with two wheel, two servo and two RGB-LED children.
/// Apps read and write these components; unused ones cost only a default.
pub fn spawn_robot(commands: &mut Commands) -> Entity {
    commands
        .spawn((
            AlvikRobot,
            // Grouped into sub-bundles: a flat tuple would exceed the 15-element
            // `Bundle` impl. State, then sensors.
            (
                Connected::default(),
                FwVersion::default(),
                Behaviour::default(),
                BatterySoc::default(),
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
                RobotVelocity::default(),
                Touch::default(),
                Motion::default(),
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
                    position: crate::units::Angle::from_degrees(90.0),
                }),
                (Servo {
                    id: ServoId::B,
                    position: crate::units::Angle::from_degrees(90.0),
                }),
                (AlvikLed { side: Side::Left }, LedColor::default()),
                (AlvikLed { side: Side::Right }, LedColor::default()),
            ],
        ))
        .id()
}
