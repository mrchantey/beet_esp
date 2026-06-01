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
pub mod async_bridge;
#[cfg(feature = "clock")]
pub mod clock;
pub mod esp32_plugin;
pub mod health;
#[cfg(feature = "led")]
pub mod led;
pub mod mem;
#[cfg(feature = "random")]
pub mod random;
#[cfg(feature = "wifi")]
pub mod wifi;

pub mod prelude {
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
	pub use crate::init_esp;
	pub use crate::main;
	#[cfg(feature = "led")]
	pub use crate::led::*;
	pub use crate::mem::{External, Internal, PsramInfo};
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

/// Bytes of internal SRAM reclaimed from the second-stage bootloader, otherwise
/// unused. Donated as an [`Internal`](crate::mem::Internal) heap region so the
/// Wi-Fi/BLE radio's DMA allocations have somewhere to go.
pub const RECLAIMED_INTERNAL_BYTES: usize = 73744;

/// Set up the memory layout and binary-local statics a Bevy app on this board
/// needs, then bring the chip up far enough to register PSRAM **before**
/// `App::new()`.
///
/// On this board there are two pools of RAM with very different properties (see
/// [`mem`](crate::mem)): scarce internal SRAM (fast, DMA-capable, shared with the
/// radio and the stack) and plentiful external PSRAM (slow, no DMA, multi-MiB).
/// This macro wires them so that:
///
/// - **PSRAM is the default home** for every capability-agnostic allocation —
///   the Bevy `World`, the reflection / type registry, scene loading, per-request
///   buffers. No per-allocation ceremony; they just land there. PSRAM is mapped
///   and registered here, before `App::new()`, so even the registry Bevy builds
///   during `add_plugins` goes to PSRAM.
/// - **Internal SRAM is held in reserve** for the radio and any explicitly
///   [`Internal`](crate::mem::Internal) allocation. Its size is the knob below.
///
/// The attribute is `internal_reserve_kb`: how much fresh internal SRAM to carve
/// for the hot/DMA reserve (on top of the always-present ~72 KiB
/// bootloader-reclaimed region). Now that the bulk lives in PSRAM, it only sizes
/// the internal reserve.
///
/// Invoke at the top of `main`, before `App::new()`. It declares one per-binary
/// linker static — the internal heap [`Reserve`](crate::mem::Reserve) (a heap
/// region must be `'static`) — and forwards it to
/// [`mem::boot`](crate::mem::boot), which does the actual work. Embassy start
/// and peripheral distribution still happen later, in
/// [`Esp32Plugin`](crate::esp32_plugin::Esp32Plugin)'s `PreStartup` system,
/// which collects the peripherals `boot` parked.
///
/// This is the manual counterpart to [`#[beet_esp::main]`](crate::main), which
/// expands to it — so the setup can't drift between the two paths.
///
/// ```ignore
/// init_esp!();                        // default 64 KiB internal reserve
/// init_esp!(internal_reserve_kb: 96); // bigger hot/DMA reserve
/// init_esp!(internal_reserve: 48 * 1024); // or a byte size directly
/// App::new()
///     .add_plugins((Esp32Plugin, LedPlugin))
///     .run();
/// ```
#[macro_export]
macro_rules! init_esp {
	() => {
		// 64 KiB internal reserve: ample for the radio + DMA + hot allocations
		// now that the World/registry/request bulk lives in PSRAM.
		$crate::init_esp!(internal_reserve: 64 * 1024);
	};
	// Preferred surface: how much internal SRAM to reserve for the radio/hot use.
	(internal_reserve_kb: $kb:expr) => {
		$crate::init_esp!(internal_reserve: ($kb) * 1024);
	};
	// Back-compat alias for the old whole-heap knob. Same effect as
	// `internal_reserve_kb` now that the bulk lives in PSRAM.
	(heap_size_kb: $kb:expr) => {
		$crate::init_esp!(internal_reserve: ($kb) * 1024);
	};
	(internal_reserve: $size:expr) => {
		// The internal SRAM reserve: a binary-local linker static (a heap region
		// must be `'static`), sized by const generic so this declaration is the
		// macro's only job. Everything else — logging, PSRAM mapping, region
		// registration, the boot snapshot — is done by `mem::boot`.
		static mut __BEET_ESP_RESERVE: $crate::mem::Reserve<{ $size }> =
			$crate::mem::Reserve::uninit();
		unsafe { $crate::mem::boot(&mut *(&raw mut __BEET_ESP_RESERVE)) };
	};
}
