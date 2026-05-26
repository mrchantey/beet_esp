//! Addressable RGB LED hue-fade ("blinky"), as a Bevy app.
//!
//! A minimal Bevy `App` on bare metal: [`Esp32Plugin`] logs a startup banner,
//! [`HueFadePlugin`] advances the hue each `Update` and writes the colour to the
//! [`LedColor`] resource. The render loop reads that colour and pushes it to the
//! on-board WS2812 over RMT via [`Ws2812`].
//!
//! The on-board LED is `GPIO48` on the official Espressif DevKitC-1 / DevKitM-1;
//! some clone boards use `GPIO38` — swap the pin below if nothing lights up.
//!
//! Run with: `cargo run --release --example blinky`

#![no_std]
#![no_main]

use beet::prelude::App;
use beet_esp::prelude::*;
use embassy_executor::Spawner;
use embassy_time::{Duration, Timer};
use esp_hal::clock::CpuClock;
use panic_rtt_target as _;

esp_bootloader_esp_idf::esp_app_desc!();

#[esp_rtos::main]
async fn main(_spawner: Spawner) -> ! {
    rtt_target::rtt_init_defmt!();

    // Bevy's World and schedules allocate, so bring up a heap.
    esp_alloc::heap_allocator!(size: 96 * 1024);

    let p = esp_hal::init(esp_hal::Config::default().with_cpu_clock(CpuClock::max()));
    start_embassy(p.TIMG0, p.SW_INTERRUPT);

    let mut led = Ws2812::new(p.RMT, p.GPIO48);

    let mut app = App::new();
    app.add_plugins((Esp32Plugin, HueFadePlugin::default()));

    loop {
        app.update();
        led.write(app.world().resource::<LedColor>().0).await;
        Timer::after(Duration::from_millis(20)).await;
    }
}
