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

#[cfg(feature = "alvik")]
pub mod alvik;
pub mod esp32_plugin;
// ESP32 runtime plumbing: heap/PSRAM, health, the async bridge, the SNTP clock.
pub mod esp32_utils;
// Scene-carried control scripting (rhai + quickjs backends).
pub mod scripting;
// Cross-cutting utilities: typed quantities, the WS2812 LED, the RNG backend.
pub mod utils;
// The hardware-agnostic scene server: bootstrap routes that load their real
// routes over the wire. Needs beet's no_std router, so gated on `router`.
#[cfg(feature = "router")]
pub mod scene;
#[cfg(feature = "wifi")]
pub mod net;

pub mod prelude {
	#[cfg(feature = "alvik")]
	pub use crate::alvik::prelude::*;
	pub use crate::esp32_plugin::*;
	pub use crate::esp32_utils::prelude::*;
	pub use crate::esp_app_desc;
	pub use crate::main;
	#[cfg(feature = "router")]
	pub use crate::scene::prelude::*;
	// Empty unless a scripting backend is enabled; gate to avoid an unused glob.
	#[cfg(any(feature = "rhai", feature = "quickjs"))]
	pub use crate::scripting::prelude::*;
	pub use crate::utils::prelude::*;
	#[cfg(feature = "wifi")]
	pub use crate::net::*;
}

/// Emit the esp-idf bootloader application descriptor required to boot.
///
/// Invoke once at module scope (it defines a linker-section static, so it can't
/// live inside `main`). Hides the `esp_bootloader_esp_idf::esp_app_desc!()`
/// plumbing from examples.
#[macro_export]
macro_rules! esp_app_desc {
	() => {
		$crate::esp_bootloader_esp_idf::esp_app_desc!();
	};
}

/// Bytes of internal SRAM reclaimed from the second-stage bootloader, otherwise
/// unused. Donated as an [`Internal`](crate::esp32_utils::mem::Internal) heap region so the
/// Wi-Fi/BLE radio's DMA allocations have somewhere to go.
pub const RECLAIMED_INTERNAL_BYTES: usize = 73744;
