//! Arduino Alvik support: drive the robot's STM32 carrier over UART from a Bevy
//! app, with all data as components and all behaviour as systems / observers.
//!
//! Add [`AlvikPlugin`](plugin::AlvikPlugin) (after
//! [`Esp32Plugin`](crate::esp32_plugin)); it claims UART1 and the STM32 control
//! GPIOs, runs the bring-up handshake on an embassy task, installs the per-frame
//! transport systems, and spawns the robot entity tree. Apps then read sensor
//! components and write command components (`WheelTarget`, `DifferentialDrive`,
//! `Servo`, `LedColor`); the systems carry them to and from the wire. To run
//! logic when the robot appears, add an observer on `On<Add, AlvikRobot>`.
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
pub mod plugin;
pub mod protocol;
pub mod systems;
pub mod types;
pub mod ucpack;

pub mod prelude {
    pub use super::components::*;
    pub use super::driver::ALVIK_EVENTS;
    pub use super::driver::ALVIK_OUT;
    pub use super::driver::ALVIK_STATE;
    pub use super::events::*;
    pub use super::plugin::AlvikPlugin;
    pub use super::plugin::spawn_robot;
    pub use super::protocol::Command;
    pub use super::protocol::Status;
    pub use super::types::Side;
    pub use super::types::TiltAxis;
    pub use super::types::TouchButton;
}
