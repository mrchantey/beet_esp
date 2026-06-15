#![no_std]
// The lib doubles as the on-device test target (`[lib] harness = false`): under
// `cargo test` it is a `no_main` binary whose entry is `device_test::main`, not
// libtest's. A normal `cargo build` is unaffected (no `test` cfg).
#![cfg_attr(all(test, feature = "device"), no_main)]

extern crate alloc;

// Re-exported so the macros below (and `#[beet_esp::main]`) can reach these
// crates hermetically via absolute paths, without the calling example having to
// import ESP internals itself. Gated on `device`: without the hardware stack the
// crate compiles for the host (the scene-definition types only).
#[cfg(feature = "device")]
pub use esp_alloc;
#[cfg(feature = "device")]
pub use esp_bootloader_esp_idf;
#[cfg(feature = "device")]
pub use esp_hal;
#[cfg(feature = "device")]
pub use rtt_target;

/// Firmware panic handler: log the panic over RTT (in `BlockIfFull` so the
/// message is never lost), then halt. This is `panic-rtt-target`'s behaviour
/// inlined, so the crate owns its single `#[panic_handler]` rather than
/// force-linking an external one — which is what lets the on-device test build
/// swap in its own semihosting-exit handler (see `device_test`) without two
/// handlers colliding.
///
/// `not(test)` so the lib-as-test target (`cargo test`) uses its handler instead.
#[cfg(all(feature = "device", not(test)))]
#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
	use core::fmt::Write;
	critical_section::with(|_| {
		rtt_target::with_terminal_channel(|term| {
			term.set_mode(rtt_target::ChannelMode::BlockIfFull);
			let mut channel = term.write(0);
			let _ = writeln!(channel, "{info}");
		});
		loop {
			core::sync::atomic::compiler_fence(
				core::sync::atomic::Ordering::SeqCst,
			);
		}
	})
}

/// `#[beet_esp::main]` — wraps `fn main` with the ESP32 entry boilerplate.
#[cfg(feature = "device")]
pub use beet_esp_macros::main;

#[cfg(feature = "alvik")]
pub mod alvik;
// The on-device test entry: chip bring-up + beet test runner + semihosting exit
// + the test panic handler. Test-only, so a normal firmware build never links
// its `#[panic_handler]` (which would collide with the firmware's).
#[cfg(all(test, feature = "device"))]
mod device_test;
#[cfg(feature = "device")]
pub mod esp32_plugin;
// ESP32 runtime plumbing: heap/PSRAM, health, the async bridge, the SNTP clock.
#[cfg(feature = "device")]
pub mod esp32_utils;
// Scene-carried control scripting (rhai + quickjs backends).
pub mod scripting;
// Cross-cutting utilities: typed quantities, the WS2812 LED, the RNG backend.
pub mod utils;
// ESP-specific scene wiring on top of the upstream scene server (which loads the
// device's real routes over the wire). Needs beet's no_std router, so gated on
// `router`.
#[cfg(feature = "router")]
pub mod scene;
#[cfg(feature = "wifi")]
pub mod net;

pub mod prelude {
	#[cfg(feature = "alvik")]
	pub use crate::alvik::prelude::*;
	#[cfg(feature = "device")]
	pub use crate::esp32_plugin::*;
	#[cfg(feature = "device")]
	pub use crate::esp32_utils::prelude::*;
	#[cfg(feature = "device")]
	pub use crate::esp_app_desc;
	#[cfg(feature = "device")]
	pub use crate::main;
	#[cfg(feature = "router")]
	pub use crate::scene::prelude::*;
	// Empty unless the scripting layer is enabled; gate to avoid an unused glob.
	#[cfg(feature = "scripting")]
	pub use crate::scripting::prelude::*;
	pub use crate::utils::prelude::*;
	#[cfg(feature = "wifi")]
	pub use crate::net::*;
}

/// Emit the esp-idf bootloader application descriptor required to boot.
///
/// Invoke once at module scope (it defines a linker-section static, so it can't
/// live inside `main`). Hides the `esp_bootloader_esp_idf::esp_app_desc!()`
/// plumbing from examples. Device-only — it expands to a linker-section static.
#[cfg(feature = "device")]
#[macro_export]
macro_rules! esp_app_desc {
	() => {
		$crate::esp_bootloader_esp_idf::esp_app_desc!();
	};
}

/// Bytes of internal SRAM reclaimed from the second-stage bootloader, otherwise
/// unused. Donated as an [`Internal`](crate::esp32_utils::mem::Internal) heap region so the
/// Wi-Fi/BLE radio's DMA allocations have somewhere to go.
#[cfg(feature = "device")]
pub const RECLAIMED_INTERNAL_BYTES: usize = 73744;
