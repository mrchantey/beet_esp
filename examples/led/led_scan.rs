//! Addressable-LED GPIO scanner.
//!
//! A diagnostic for when you don't know which pin the on-board WS2812 is wired
//! to. It walks every safe-to-drive GPIO, blinking each (500 ms white on,
//! 500 ms off = 1 s per pin) and logging the pin number over defmt. Watch the
//! board: when the LED blinks, read the log to see which `GPIOxx` is currently
//! being driven. The sweep loops forever (~31 s per full pass).
//!
//! Unlike `blinky`, this drives a different pin each step, so it reconfigures
//! the RMT channel per pin rather than using the [`Ws2812`] driver; it still
//! shares the WS2812 colour/encoding via [`Grb`].
//!
//! Attach a monitor without reflashing (which would restart the sweep) using:
//!
//!   probe-rs attach --chip esp32s3 --probe <VID:PID:SERIAL> \
//!     /home/pete/.cargo_target/xtensa-esp32s3-none-elf/release/examples/led_scan
//!
//! Run with: `cargo run --release --example led_scan`
//!
//! Deliberately omitted (unsafe to toggle on this board): GPIO19/20 (native
//! USB-Serial-JTAG that probe-rs uses), GPIO26-32 (SPI flash), GPIO33-37
//! (octal PSRAM on R8 modules). GPIO22-25 don't exist on the ESP32-S3.

#![no_std]
#![no_main]

use beet::prelude::Color;
use beet_esp::prelude::Grb;
use beet_esp::prelude::PIXEL_CODES;
use beet_esp::prelude::start_embassy;
use defmt::info;
use embassy_executor::Spawner;
use embassy_time::{Duration, Timer};
use esp_hal::clock::CpuClock;
use esp_hal::gpio::{AnyPin, Level, Pin};
use esp_hal::rmt::{PulseCode, Rmt, TxChannelConfig, TxChannelCreator};
use esp_hal::time::Rate;
use panic_rtt_target as _;

esp_bootloader_esp_idf::esp_app_desc!();

// Bright but not blinding, so the blink is unmistakable during the scan.
const BRIGHTNESS: u8 = 40;

const PIN_COUNT: usize = 31;

#[esp_rtos::main]
async fn main(_spawner: Spawner) -> ! {
    rtt_target::rtt_init_defmt!();

    let p = esp_hal::init(esp_hal::Config::default().with_cpu_clock(CpuClock::max()));
    start_embassy(p.TIMG0, p.SW_INTERRUPT);

    // Every safe-to-drive GPIO. GPIO48/38 first (usual on-board WS2812 pins),
    // then the rest ascending.
    let nums: [u8; PIN_COUNT] = [
        48, 38, 0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 21, 39, 40, 41,
        42, 43, 44, 45, 46, 47,
    ];
    let mut pins: [AnyPin; PIN_COUNT] = [
        p.GPIO48.degrade(),
        p.GPIO38.degrade(),
        p.GPIO0.degrade(),
        p.GPIO1.degrade(),
        p.GPIO2.degrade(),
        p.GPIO3.degrade(),
        p.GPIO4.degrade(),
        p.GPIO5.degrade(),
        p.GPIO6.degrade(),
        p.GPIO7.degrade(),
        p.GPIO8.degrade(),
        p.GPIO9.degrade(),
        p.GPIO10.degrade(),
        p.GPIO11.degrade(),
        p.GPIO12.degrade(),
        p.GPIO13.degrade(),
        p.GPIO14.degrade(),
        p.GPIO15.degrade(),
        p.GPIO16.degrade(),
        p.GPIO17.degrade(),
        p.GPIO18.degrade(),
        p.GPIO21.degrade(),
        p.GPIO39.degrade(),
        p.GPIO40.degrade(),
        p.GPIO41.degrade(),
        p.GPIO42.degrade(),
        p.GPIO43.degrade(),
        p.GPIO44.degrade(),
        p.GPIO45.degrade(),
        p.GPIO46.degrade(),
        p.GPIO47.degrade(),
    ];

    let rmt = Rmt::new(p.RMT, Rate::from_mhz(80))
        .expect("failed to initialise RMT")
        .into_async();
    let mut creator = rmt.channel0;

    let cfg = TxChannelConfig::default()
        .with_clk_divider(1)
        .with_idle_output(true)
        .with_idle_output_level(Level::Low)
        .with_carrier_modulation(false);

    let white = Grb::from_color(Color::WHITE, BRIGHTNESS);
    let off = Grb::default();

    let mut buf = [PulseCode::end_marker(); PIXEL_CODES];

    info!("led_scan: sweeping {} pins, 1s each", nums.len());
    loop {
        for (i, pin) in pins.iter_mut().enumerate() {
            info!("scan -> GPIO{}", nums[i]);

            let mut channel = creator
                .reborrow()
                .configure_tx(&cfg)
                .expect("failed to configure RMT TX channel")
                .with_pin(pin.reborrow());

            white.encode(&mut buf);
            channel.transmit(&buf).await.ok();
            Timer::after(Duration::from_millis(500)).await;
            off.encode(&mut buf);
            channel.transmit(&buf).await.ok();
            Timer::after(Duration::from_millis(500)).await;
            // `channel` drops here, releasing the pin for the next candidate.
        }
    }
}
