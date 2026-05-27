#![no_std]

extern crate alloc;

// Re-exported so the macros below can reach these crates hermetically via
// `$crate`, without the calling example having to import ESP internals itself.
pub use esp_alloc;
pub use esp_bootloader_esp_idf;
pub use esp_hal;
pub use rtt_target;

pub mod esp32_plugin;
pub mod led;

pub mod prelude {
	pub use crate::esp32_plugin::*;
	pub use crate::esp_app_desc;
	pub use crate::init_esp;
	pub use crate::led::*;
}

/// Emit the esp-idf bootloader application descriptor required to boot.
///
/// Invoke once at module scope (it defines a linker-section static, so it can't
/// live inside `main` or [`init_esp!`]). Hides the
/// `esp_bootloader_esp_idf::esp_app_desc!()` plumbing from examples.
#[macro_export]
macro_rules! esp_app_desc {
	() => {
		$crate::esp_bootloader_esp_idf::esp_app_desc!();
	};
}

/// Bring up the board for a Bevy app and bind `$led` to the on-board WS2812.
///
/// Expands at the top of `main` into the ESP32 wiring an app needs but
/// shouldn't have to spell out: RTT/`defmt` logging, the heap (a Bevy `World`
/// allocates), CPU clock and peripheral init, and the embassy executor on
/// `esp-rtos`. `$led` is bound to a [`Ws2812`](crate::led::Ws2812) on `GPIO48`
/// (the on-board LED of the official DevKitC-1/M-1); hand it to the app with
/// `insert_non_send` so [`Esp32Plugin`](crate::esp32_plugin::Esp32Plugin)'s
/// runner can drive it.
///
/// ```ignore
/// init_esp!(led);
/// App::new()
///     .insert_non_send(led)
///     .add_plugins((Esp32Plugin, LedPlugin::default()))
///     .run();
/// ```
#[macro_export]
macro_rules! init_esp {
	($led:ident) => {
		$crate::rtt_target::rtt_init_defmt!();
		// Bevy's World and schedules allocate, so bring up a heap first.
		$crate::esp_alloc::heap_allocator!(size: 96 * 1024);
		let peripherals = $crate::esp_hal::init(
			$crate::esp_hal::Config::default()
				.with_cpu_clock($crate::esp_hal::clock::CpuClock::max()),
		);
		$crate::esp32_plugin::start_embassy(peripherals.TIMG0, peripherals.SW_INTERRUPT);
		let mut $led = $crate::led::Ws2812::new(peripherals.RMT, peripherals.GPIO48);
	};
}
