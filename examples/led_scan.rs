//! Addressable-LED GPIO scanner.
//!
//! A diagnostic for when you don't know which pin the on-board WS2812 is wired
//! to. It walks every safe-to-drive GPIO, blinking each (500 ms white on,
//! 500 ms off = 1 s per pin) and logging the pin number over defmt. Watch the
//! board: when the LED blinks, read the log to see which `GPIOxx` is currently
//! being driven. The sweep loops forever (~31 s per full pass).
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

use defmt::info;
use embassy_executor::Spawner;
use embassy_time::{Duration, Timer};
use esp_hal::clock::CpuClock;
use esp_hal::gpio::{AnyPin, Level, Pin};
use esp_hal::rmt::{PulseCode, Rmt, TxChannelConfig, TxChannelCreator};
use esp_hal::time::Rate;
use esp_hal::timer::timg::TimerGroup;
use panic_rtt_target as _;

esp_bootloader_esp_idf::esp_app_desc!();

// WS2812 bit timing in RMT ticks (80 MHz clock, divider 1 -> 12.5 ns/tick).
const T0H: u16 = 32;
const T0L: u16 = 68;
const T1H: u16 = 68;
const T1L: u16 = 32;

const PIXEL_CODES: usize = 24 + 1;

// Bright but not blinding, so the blink is unmistakable during the scan.
const BRIGHTNESS: u16 = 40;

const PIN_COUNT: usize = 31;

fn encode(grb: u32, buf: &mut [PulseCode; PIXEL_CODES]) {
    let zero = PulseCode::new(Level::High, T0H, Level::Low, T0L);
    let one = PulseCode::new(Level::High, T1H, Level::Low, T1L);
    for (i, slot) in buf.iter_mut().take(24).enumerate() {
        *slot = if (grb >> (23 - i)) & 1 == 1 { one } else { zero };
    }
    buf[24] = PulseCode::end_marker();
}

#[esp_rtos::main]
async fn main(_spawner: Spawner) -> ! {
    rtt_target::rtt_init_defmt!();

    let config = esp_hal::Config::default().with_cpu_clock(CpuClock::max());
    let peripherals = esp_hal::init(config);

    let timg0 = TimerGroup::new(peripherals.TIMG0);
    let sw_interrupt =
        esp_hal::interrupt::software::SoftwareInterruptControl::new(peripherals.SW_INTERRUPT);
    esp_rtos::start(timg0.timer0, sw_interrupt.software_interrupt0);

    // Every safe-to-drive GPIO. GPIO48/38 first (usual on-board WS2812 pins),
    // then the rest ascending.
    let nums: [u8; PIN_COUNT] = [
        48, 38, 0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 21, 39, 40, 41,
        42, 43, 44, 45, 46, 47,
    ];
    let mut pins: [AnyPin; PIN_COUNT] = [
        peripherals.GPIO48.degrade(),
        peripherals.GPIO38.degrade(),
        peripherals.GPIO0.degrade(),
        peripherals.GPIO1.degrade(),
        peripherals.GPIO2.degrade(),
        peripherals.GPIO3.degrade(),
        peripherals.GPIO4.degrade(),
        peripherals.GPIO5.degrade(),
        peripherals.GPIO6.degrade(),
        peripherals.GPIO7.degrade(),
        peripherals.GPIO8.degrade(),
        peripherals.GPIO9.degrade(),
        peripherals.GPIO10.degrade(),
        peripherals.GPIO11.degrade(),
        peripherals.GPIO12.degrade(),
        peripherals.GPIO13.degrade(),
        peripherals.GPIO14.degrade(),
        peripherals.GPIO15.degrade(),
        peripherals.GPIO16.degrade(),
        peripherals.GPIO17.degrade(),
        peripherals.GPIO18.degrade(),
        peripherals.GPIO21.degrade(),
        peripherals.GPIO39.degrade(),
        peripherals.GPIO40.degrade(),
        peripherals.GPIO41.degrade(),
        peripherals.GPIO42.degrade(),
        peripherals.GPIO43.degrade(),
        peripherals.GPIO44.degrade(),
        peripherals.GPIO45.degrade(),
        peripherals.GPIO46.degrade(),
        peripherals.GPIO47.degrade(),
    ];

    let rmt = Rmt::new(peripherals.RMT, Rate::from_mhz(80))
        .expect("failed to initialise RMT")
        .into_async();
    let mut creator = rmt.channel0;

    let cfg = TxChannelConfig::default()
        .with_clk_divider(1)
        .with_idle_output(true)
        .with_idle_output_level(Level::Low)
        .with_carrier_modulation(false);

    let level = BRIGHTNESS as u32;
    let white = (level << 16) | (level << 8) | level; // GRB, all channels equal
    let off = 0u32;

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

            encode(white, &mut buf);
            channel.transmit(&buf).await.ok();
            Timer::after(Duration::from_millis(500)).await;
            encode(off, &mut buf);
            channel.transmit(&buf).await.ok();
            Timer::after(Duration::from_millis(500)).await;
            // `channel` drops here, releasing the pin for the next candidate.
        }
    }
}
