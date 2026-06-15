//! On-device test entry for `cargo test -p beet_esp --lib`.
//!
//! Runs beet_esp's `#[beet::test]` unit tests on the ESP32-S3 through beet's own
//! harness (`beet::testing`), not a foreign one. The flow:
//!
//! 1. bring the chip up by hand (NOT `#[beet_esp::main]`, which force-links the
//!    firmware's `panic-rtt-target` handler — we install our own that exits);
//! 2. assemble a bevy `App` from the esp bring-up plugin plus
//!    [`TestPlugin`](beet::testing::TestPlugin), seeded with every test
//!    registered into the `linkme` slice via [`embedded_tests`];
//! 3. tick the schedule until the suite writes an `AppExit`, then report the
//!    result by exiting through semihosting — the probe-rs runner turns that exit
//!    code into the `cargo test` pass/fail.
//!
//! A failing assertion panics; the [`panic`] handler logs the message + location
//! over RTT (blocking until the host drains it) then exits non-zero, so the host
//! sees the failure. Only the first failure is reported (abort-on-first-failure):
//! there is no unwinding under `panic = abort`, so the runner cannot catch.

use crate::esp32_plugin::Esp32Plugin;
use crate::esp32_utils::async_bridge::spawn_driver;
use crate::esp32_utils::mem::Reserve;
use crate::esp32_utils::mem::init_esp;
use beet::prelude::*;
use beet::testing::TestPlugin;
use beet::testing::TestRunnerConfig;
use beet::testing::embedded_tests;
use beet::testing::tests_bundle;
use core::fmt::Write;
use embassy_time::Duration;
use embassy_time::Timer;
use esp_rtos::embassy::Executor;
use static_cell::StaticCell;

// The esp-idf bootloader application descriptor, required to boot. Item-scope
// linker static, so it sits beside `main` rather than inside it.
crate::esp_app_desc!();

/// Internal SRAM reserve donated as a DMA-capable heap region. 64 KiB matches
/// the `#[beet_esp::main]` default; tests do not run the radio, so it is ample.
static mut RESERVE: Reserve<{ 64 * 1024 }> = Reserve::uninit();

/// Test-binary entry: bring up the chip, then run the beet test app.
#[esp_hal::main]
fn main() -> ! {
	// `esp_hal::init` + PSRAM/heap + RTT `log` + park peripherals, exactly as
	// `#[beet_esp::main]` does, but without its `panic-rtt-target` import.
	unsafe { init_esp(&raw mut RESERVE) };

	let mut app = App::new();
	// `Esp32Plugin` brings up embassy/clock/peripherals and sets the firmware
	// runner; override it with the test runner (last `set_runner` wins).
	app.add_plugins((Esp32Plugin, TestPlugin))
		.set_runner(esp_test_runner);
	app.world_mut().spawn((
		TestRunnerConfig::default(),
		tests_bundle(embedded_tests()),
	));
	app.run();
	unreachable!("the test runner reports via semihosting and never returns")
}

/// How long the schedule sleeps between ticks while async tests run.
const FRAME: Duration = Duration::from_millis(5);

/// Bevy runner for the test app: ticks the schedule on the embassy executor
/// until the suite writes an `AppExit`, then exits through semihosting so the
/// probe-rs runner reports the result. Mirrors `esp32_plugin::esp_runner`, but
/// watches for the suite outcome instead of looping forever.
fn esp_test_runner(mut app: App) -> AppExit {
	app.finish();
	app.cleanup();

	static EXECUTOR: StaticCell<Executor> = StaticCell::new();
	let executor = EXECUTOR.init(Executor::new());
	executor.run(move |spawner| {
		app.insert_non_send(spawner);
		spawn_driver(spawner, async move {
			loop {
				app.update();
				if let Some(exit) = app.should_exit() {
					// 0 = suite passed, non-zero = failed (a returned `Err`); a
					// panicking assertion never reaches here, it hits `panic`.
					semihosting::process::exit(exit.exit_code());
				}
				Timer::after(FRAME).await;
			}
		});
	})
}

/// Panic handler for the test binary: `panic-rtt-target`'s body, but ending in a
/// semihosting exit instead of `loop {}`.
///
/// Logs the `PanicInfo` (the matcher's Expected/Received message plus the
/// `#[track_caller]` location) over RTT in `BlockIfFull` mode, which blocks until
/// the host drains the buffer so the message is never lost, then exits non-zero
/// so the probe-rs runner reports failure. The firmware's `panic-rtt-target`
/// loops forever here, which would hang probe-rs.
#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
	critical_section::with(|_| {
		rtt_target::with_terminal_channel(|term| {
			term.set_mode(rtt_target::ChannelMode::BlockIfFull);
			let mut channel = term.write(0);
			let _ = writeln!(channel, "{info}");
		});
	});
	semihosting::process::exit(101)
}
