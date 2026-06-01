//! Pin and UART assignments for the Alvik's on-board Arduino Nano ESP32.
//!
//! These mirror `arduino-alvik`'s `pinout_definitions.py` / `uart.py` verbatim
//! (the firmware runs on the Nano ESP32 that sits on the robot). The concrete
//! `esp-hal` peripheral singletons (`GPIO43`, `UART1`, …) are claimed by
//! [`bring_up`](crate::esp32_plugin) and the [`driver`](super::driver); this
//! module is the single source of truth for *which* pins and the link speed.

/// UART1 baud rate to the STM32 carrier: 460800-8N1.
pub const BAUD_RATE: u32 = 460_800;

/// UART1 TX — ESP32 `GPIO43` → STM32 RX.
pub const UART_TX: u8 = 43;
/// UART1 RX — ESP32 `GPIO44` ← STM32 TX.
pub const UART_RX: u8 = 44;

/// `RESET_STM32` — ESP32 `GPIO6` (Nano D3) → STM32 NRST. Active low.
pub const RESET_STM32: u8 = 6;
/// `BOOT0_STM32` — ESP32 `GPIO5` (Nano D2) → STM32 Boot0.
pub const BOOT0_STM32: u8 = 5;
/// `NANO_CHK` — ESP32 `GPIO7` (Nano D4) → STM32 NANO_CHK.
pub const NANO_CHK: u8 = 7;
/// `CHECK_STM32` — ESP32 `GPIO13` (Nano A6), input pull-down, high when the
/// robot is powered on.
pub const CHECK_STM32: u8 = 13;
