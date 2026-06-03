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
#[cfg(feature = "action")]
pub mod async_utils;
pub mod async_bridge;
#[cfg(feature = "clock")]
pub mod clock;
pub mod esp32_plugin;
pub mod health;
#[cfg(feature = "led")]
pub mod led;
pub mod mem;
// QuickJS console + clock glue, reached through `beet::exports::rquickjs`.
#[cfg(feature = "quickjs")]
pub mod quickjs;
#[cfg(feature = "random")]
pub mod random;
// The hardware-agnostic scene server: bootstrap routes that load their real
// routes over the wire. Needs beet's no_std router, so gated on `router`.
#[cfg(feature = "router")]
pub mod scene;
// Crate-wide typed quantities (pure no_std math, no hardware deps).
pub mod units;
#[cfg(feature = "wifi")]
pub mod wifi;

pub mod prelude {
	#[cfg(feature = "alvik")]
	pub use crate::alvik::prelude::*;
	#[cfg(feature = "action")]
	pub use crate::async_utils::*;
	// Re-export the module, not its contents, so callers reach the primitives via
	// the `async_bridge::` prefix (e.g. `async_bridge::spawn_worker`).
	pub use crate::async_bridge;
	#[cfg(feature = "clock")]
	pub use crate::clock::*;
	pub use crate::esp32_plugin::*;
	pub use crate::esp_app_desc;
	pub use crate::health::*;
	pub use crate::main;
	#[cfg(feature = "led")]
	pub use crate::led::*;
	pub use crate::mem::{External, Internal, PsramInfo};
	#[cfg(feature = "quickjs")]
	pub use crate::quickjs::{RuntimeEspExt, install_console};
	#[cfg(feature = "router")]
	pub use crate::scene::prelude::*;
	pub use crate::units::*;
	#[cfg(feature = "wifi")]
	pub use crate::wifi::*;
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
/// unused. Donated as an [`Internal`](crate::mem::Internal) heap region so the
/// Wi-Fi/BLE radio's DMA allocations have somewhere to go.
pub const RECLAIMED_INTERNAL_BYTES: usize = 73744;
