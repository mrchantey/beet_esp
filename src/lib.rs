#![no_std]

extern crate alloc;

// Re-exported so the macros below (and `#[beet_esp::main]`) can reach these
// crates hermetically via absolute paths, without the calling example having to
// import ESP internals itself.
pub use esp_alloc;
pub use esp_bootloader_esp_idf;
pub use esp_hal;
pub use panic_rtt_target;
pub use rtt_target;

/// `#[beet_esp::main]` — wraps `fn main` with the ESP32 entry boilerplate.
pub use beet_esp_macros::main;

#[cfg(feature = "action")]
pub mod async_utils;
pub mod bridge;
#[cfg(feature = "clock")]
pub mod clock;
pub mod esp32_plugin;
pub mod health;
#[cfg(feature = "led")]
pub mod led;
#[cfg(feature = "random")]
pub mod random;
#[cfg(feature = "wifi")]
pub mod wifi;

pub mod prelude {
	#[cfg(feature = "action")]
	pub use crate::async_utils::*;
	pub use crate::bridge::*;
	#[cfg(feature = "clock")]
	pub use crate::clock::*;
	pub use crate::esp32_plugin::*;
	pub use crate::esp_app_desc;
	pub use crate::health::*;
	pub use crate::init_esp;
	pub use crate::main;
	#[cfg(feature = "led")]
	pub use crate::led::*;
	#[cfg(feature = "wifi")]
	pub use crate::wifi::*;
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

/// Set up the binary-local statics a Bevy app on this board needs: RTT/`defmt`
/// logging and the heap (a Bevy `World` allocates, so the heap must exist before
/// `App::new()`).
///
/// Invoke at the top of `main`, before `App::new()`. These define per-binary
/// linker statics (`_SEGGER_RTT`, the heap region) so they must expand in the
/// binary, not inside a library fn. Chip/embassy bring-up and peripheral
/// distribution happen later, inside
/// [`Esp32Plugin`](crate::esp32_plugin::Esp32Plugin)'s `PreStartup` system.
///
/// This is the manual counterpart to [`#[beet_esp::main]`](crate::main), which
/// expands to it — so the heap/RTT setup can't drift between the two paths.
///
/// ```ignore
/// init_esp!();                     // default 96 KiB heap
/// init_esp!(heap_size: 64 * 1024); // or pick a size
/// App::new()
///     .add_plugins((Esp32Plugin, LedPlugin))
///     .run();
/// ```
#[macro_export]
macro_rules! init_esp {
	() => {
		$crate::init_esp!(heap_size: 96 * 1024);
	};
	(heap_size: $size:expr) => {
		$crate::rtt_target::rtt_init_defmt!();
		// The main heap, in the RAM the stack also uses.
		$crate::esp_alloc::heap_allocator!(size: $size);
		// A second heap in the RAM reclaimed from the bootloader, otherwise
		// unused. esp-radio (Wi-Fi/BLE COEX) needs this extra region; it is
		// harmless for apps that don't use the radio.
		$crate::esp_alloc::heap_allocator!(#[$crate::esp_hal::ram(reclaimed)] size: 73744);
		// Paint the stack and record the boot memory snapshot, before anything
		// allocates. Picked up by `HealthPlugin` in `PreStartup`.
		$crate::health::on_boot();
	};
}
