//! AlvikCarrier (STM32) firmware flasher: turns the Nano ESP32 into a
//! transparent USB ↔ UART bridge into the STM32's ROM bootloader, so a host-side
//! `stm32flash` can write a new carrier `.bin`.
//!
//! Our Rust port replaces the Arduino/MicroPython stack on the Nano ESP32, so we
//! have no Arduino tool to push a new STM32 carrier firmware. But the STM32 sits
//! behind the Nano (UART1 + the `BOOT0`/`RESET` control lines this firmware
//! already owns), and its on-chip ROM bootloader speaks the standard ST UART
//! protocol (AN3155). So flashing it is just: drop the STM32 into its bootloader,
//! then forward bytes between the host and UART1.
//!
//! # What it does
//!
//! On boot it drives `BOOT0` high and pulses `RESET`, leaving the STM32 in its
//! system bootloader, then bridges the native USB-Serial-JTAG CDC (`/dev/ttyACM*`
//! on the host) to UART1 at **9600 8E1** (even parity is mandatory for the STM32
//! ROM bootloader; the slow rate keeps the single-buffer bridge from dropping a
//! byte on overflow). The bridge runs forever; the STM32 is reset *into* the
//! bootloader once, up front, so no host control of `RESET`/`BOOT0` is needed.
//!
//! # Usage
//!
//! See `.agents/skills/flash-firmware.md` for the full procedure (and the
//! gotchas). **Run the bridge with `espflash`, not probe-rs** — probe-rs polls
//! the JTAG endpoint of the *same* USB device as the CDC and corrupts the bridged
//! stream; `espflash` flashes and resets the chip to run free of any debugger,
//! which is the only reliable way to drive the CDC:
//!
//! ```shell
//! # 1. build, then flash + run the bridge free of any debugger
//! cargo build --release --no-default-features --features device,alvik --bin flash-firmware
//! espflash flash --chip esp32s3 --port /dev/ttyACM0 \
//!     target/xtensa-esp32s3-none-elf/release/flash-firmware
//! # 2. (first time only) the carrier ships read-protected: remove RDP
//! #    (mass-erases), then re-run espflash to re-pulse the STM32 into the bootloader
//! stm32flash -b 9600 -k /dev/ttyACM0
//! espflash flash --chip esp32s3 --port /dev/ttyACM0 target/.../flash-firmware
//! # 3. write the firmware (a clean write ACKs every page, so a clean write == good
//! #    flash; a full -r read-back is the ideal check but is the most desync-prone step)
//! stm32flash -b 9600 -w firmware_1_1_1.bin /dev/ttyACM0
//! # 4. restore our app (resets the STM32 into the freshly-flashed carrier firmware)
//! cargo run --release --no-default-features --features device,alvik \
//!     --example alvik-sensors
//! ```

#![no_std]
#![no_main]

use beet::prelude::*;
use beet_esp::prelude::*;
use embassy_executor::Spawner;
use embassy_futures::join::join;
use beet_esp::esp32_utils::async_bridge::spawn_driver;
use embassy_time::Duration;
use embassy_time::Timer;
use embedded_io_async::Read;
use embedded_io_async::Write;
use esp_hal::gpio::Level;
use esp_hal::gpio::Output;
use esp_hal::gpio::OutputConfig;
use esp_hal::peripherals::GPIO5;
use esp_hal::peripherals::GPIO6;
use esp_hal::peripherals::GPIO43;
use esp_hal::peripherals::GPIO44;
use esp_hal::peripherals::UART1;
use esp_hal::peripherals::USB_DEVICE;
use esp_hal::uart::Config;
use esp_hal::uart::DataBits;
use esp_hal::uart::Parity;
use esp_hal::uart::StopBits;
use esp_hal::uart::Uart;
use esp_hal::usb_serial_jtag::UsbSerialJtag;

/// The STM32 ROM bootloader baud. 8E1; even parity is required by AN3155. Kept
/// deliberately slow (19200) so the UART RX FIFO never overflows mid-transfer
/// under a sustained STM32→host stream (e.g. a full read-back verify): the
/// single-buffer bridge drops a byte on overflow, which would desync
/// `stm32flash`'s strict request/response protocol. The STM32 ROM auto-bauds to
/// whatever UART1 sends, and the host-side `-b` is moot (the host link is
/// USB-CDC, not a real UART), so only this rate matters. 19200 proved borderline
/// (occasional single-byte drop mid-transfer, fatal to stm32flash), so 9600 for
/// a comfortable FIFO margin. ~186 KiB takes ~4 min; a one-time flash, so
/// reliability wins over speed.
const BRIDGE_BAUD: u32 = 9_600;

#[beet_esp::main]
fn main() {
    App::new()
        .add_plugins(Esp32Plugin)
        .add_systems(Startup, spawn_bridge)
        .run();
}

/// Claim UART1, the STM32 `BOOT0`/`RESET` GPIOs and the USB-Serial-JTAG CDC that
/// [`bring_up`](beet_esp::esp32_plugin) parked, then spawn the bridge task.
/// Exclusive so it can pull the non-send peripherals and the [`Spawner`].
fn spawn_bridge(world: &mut World) {
    let uart1 = world
        .remove_non_send::<UART1<'static>>()
        .expect("add Esp32Plugin (device,alvik) before flash-firmware");
    let tx = world.remove_non_send::<GPIO43<'static>>().expect("GPIO43");
    let rx = world.remove_non_send::<GPIO44<'static>>().expect("GPIO44");
    let reset = world.remove_non_send::<GPIO6<'static>>().expect("GPIO6");
    let boot0 = world.remove_non_send::<GPIO5<'static>>().expect("GPIO5");
    let usb = world.remove_non_send::<USB_DEVICE<'static>>().expect("USB_DEVICE");
    let spawner = *world.non_send::<Spawner>();

    let config = Config::default()
        .with_baudrate(BRIDGE_BAUD)
        .with_data_bits(DataBits::_8)
        .with_parity(Parity::Even)
        .with_stop_bits(StopBits::_1);
    let uart = Uart::new(uart1, config)
        .expect("failed to configure UART1")
        .with_tx(tx)
        .with_rx(rx)
        .into_async();
    let usb = UsbSerialJtag::new(usb).into_async();

    // RESET is active-low; BOOT0 high selects the ROM bootloader on the next
    // reset. Kept alive (moved into the task) so the levels persist while bridging.
    let bridge = Bridge {
        uart,
        usb,
        reset: Output::new(reset, Level::High, OutputConfig::default()),
        boot0: Output::new(boot0, Level::High, OutputConfig::default()),
    };
    spawn_driver(spawner, bridge.run());
}

/// Owns the two transports plus the STM32 control lines for the bridge's lifetime.
struct Bridge {
    uart: Uart<'static, esp_hal::Async>,
    usb: UsbSerialJtag<'static, esp_hal::Async>,
    reset: Output<'static>,
    boot0: Output<'static>,
}

impl Bridge {
    /// Reset the STM32 into its bootloader, then forward bytes both ways forever.
    async fn run(mut self) {
        // BOOT0 is already high; pulse RESET to latch the bootloader entry.
        self.boot0.set_high();
        self.reset.set_low();
        Timer::after(Duration::from_millis(50)).await;
        self.reset.set_high();
        // Let the ROM bootloader settle before the host starts its handshake.
        Timer::after(Duration::from_millis(200)).await;
        info!(
            "flash-firmware: STM32 in bootloader — bridging USB CDC <-> UART1 @ {} 8E1",
            BRIDGE_BAUD
        );
        info!("flash-firmware: run `stm32flash -b 115200 -w <fw>.bin -v /dev/ttyACM0`");

        let (mut uart_rx, mut uart_tx) = self.uart.split();
        let (mut usb_rx, mut usb_tx) = self.usb.split();

        // host -> STM32: drain the CDC and write it to UART1.
        let host_to_stm = async {
            let mut buf = [0u8; 256];
            loop {
                if let Ok(n) = Read::read(&mut usb_rx, &mut buf).await
                    && n > 0
                {
                    let _ = uart_tx.write_async(&buf[..n]).await;
                    let _ = uart_tx.flush_async().await;
                }
            }
        };
        // STM32 -> host: drain UART1 and write it to the CDC. A FIFO overflow
        // under a fast reply is harmless (the host's protocol retries), so ignore
        // read errors and keep pumping.
        let stm_to_host = async {
            let mut buf = [0u8; 256];
            loop {
                if let Ok(n) = uart_rx.read_async(&mut buf).await
                    && n > 0
                {
                    let _ = Write::write(&mut usb_tx, &buf[..n]).await;
                    let _ = Write::flush(&mut usb_tx).await;
                }
            }
        };
        join(host_to_stm, stm_to_host).await;
    }
}
