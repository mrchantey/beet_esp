//! Addressable RGB LED hue-fade ("blinky").
//!
//! Drives the on-board WS2812 / SK68xx addressable LED via the RMT peripheral,
//! cycling smoothly around the colour wheel. The WS2812 bit timing is encoded
//! directly into RMT pulse codes, so there's no dependency on an external
//! smart-led crate.
//!
//! The on-board LED pin differs between ESP32-S3 boards: official Espressif
//! DevKitC-1 / DevKitM-1 use GPIO48, while some clone boards use GPIO38. If
//! nothing lights up, swap the pin at `LED_PIN` below (see the comment there).
//!
//! Run with: `cargo run --release --example blinky`

#![no_std]
#![no_main]

use defmt::info;
use embassy_executor::Spawner;
use embassy_time::{Duration, Timer};
use esp_hal::clock::CpuClock;
use esp_hal::gpio::Level;
use esp_hal::rmt::{PulseCode, Rmt, TxChannelConfig, TxChannelCreator};
use esp_hal::time::Rate;
use esp_hal::timer::timg::TimerGroup;
use panic_rtt_target as _;

esp_bootloader_esp_idf::esp_app_desc!();

// WS2812 bit timing, in RMT ticks. RMT clock is 80 MHz with divider 1, so one
// tick is 12.5 ns. A bit is a high pulse followed by a low pulse; the ratio
// distinguishes 0 from 1. Period is ~1.25 us (800 kHz).
const T0H: u16 = 32; // 0.40 us high for a '0'
const T0L: u16 = 68; // 0.85 us low
const T1H: u16 = 68; // 0.85 us high for a '1'
const T1L: u16 = 32; // 0.40 us low

// One WS2812 pixel = 24 bits, plus an RMT end marker.
const PIXEL_CODES: usize = 24 + 1;

// Overall brightness cap (0..=255). The bare LED is blinding at full scale.
const BRIGHTNESS: u16 = 24;

/// Encode a packed GRB colour into RMT pulse codes for a single WS2812 pixel.
fn encode(grb: u32, buf: &mut [PulseCode; PIXEL_CODES]) {
    let zero = PulseCode::new(Level::High, T0H, Level::Low, T0L);
    let one = PulseCode::new(Level::High, T1H, Level::Low, T1L);
    for (i, slot) in buf.iter_mut().take(24).enumerate() {
        // WS2812 expects MSB first.
        *slot = if (grb >> (23 - i)) & 1 == 1 { one } else { zero };
    }
    buf[24] = PulseCode::end_marker();
}

/// Map a hue position (0..=255) around the colour wheel at full saturation and
/// value. Returns `(r, g, b)`.
fn wheel(pos: u8) -> (u8, u8, u8) {
    let p = pos as u16;
    if pos < 85 {
        ((255 - p * 3) as u8, 0, (p * 3) as u8)
    } else if pos < 170 {
        let p = p - 85;
        (0, (p * 3) as u8, (255 - p * 3) as u8)
    } else {
        let p = p - 170;
        ((p * 3) as u8, (255 - p * 3) as u8, 0)
    }
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

    info!("blinky: hue fade on addressable LED");

    let rmt = Rmt::new(peripherals.RMT, Rate::from_mhz(80))
        .expect("failed to initialise RMT")
        .into_async();

    // On-board addressable LED data pin (verified on this board).
    let led_pin = peripherals.GPIO48;

    let mut channel = rmt
        .channel0
        .configure_tx(
            &TxChannelConfig::default()
                .with_clk_divider(1)
                .with_idle_output(true)
                .with_idle_output_level(Level::Low)
                .with_carrier_modulation(false),
        )
        .expect("failed to configure RMT TX channel")
        .with_pin(led_pin);

    let mut buf = [PulseCode::end_marker(); PIXEL_CODES];
    let mut hue: u8 = 0;

    loop {
        let (r, g, b) = wheel(hue);
        // Scale by brightness, then pack as GRB (WS2812 wire order).
        let r = (r as u16 * BRIGHTNESS / 255) as u32;
        let g = (g as u16 * BRIGHTNESS / 255) as u32;
        let b = (b as u16 * BRIGHTNESS / 255) as u32;
        let grb = (g << 16) | (r << 8) | b;

        encode(grb, &mut buf);
        if let Err(e) = channel.transmit(&buf).await {
            defmt::error!("RMT transmit failed: {}", e);
        }

        hue = hue.wrapping_add(1);
        Timer::after(Duration::from_millis(20)).await;
    }
}
