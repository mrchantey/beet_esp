//! ESP32/Bevy bring-up: the embassy/`esp-rtos` starter and the baseline app
//! plugin. The bulk of board bring-up is hidden behind the [`init_esp!`] macro;
//! these are the pieces it (and lower-level examples) call into.
//!
//! [`init_esp!`]: crate::init_esp

use beet::prelude::*;
use defmt::info;
use esp_hal::timer::timg::TimerGroup;

/// Start the embassy executor on `esp-rtos` using timer group 0. Call once,
/// after peripheral init, before spawning tasks or running an [`App`].
pub fn start_embassy(
	timg0: esp_hal::peripherals::TIMG0<'static>,
	sw: esp_hal::peripherals::SW_INTERRUPT<'static>,
) {
	let timg0 = TimerGroup::new(timg0);
	let sw_interrupt = esp_hal::interrupt::software::SoftwareInterruptControl::new(sw);
	esp_rtos::start(timg0.timer0, sw_interrupt.software_interrupt0);
}

/// Baseline scaffolding for an esp32 Bevy app: logs a startup banner.
pub struct Esp32Plugin;

impl Plugin for Esp32Plugin {
	fn build(&self, app: &mut App) {
		app.add_systems(Startup, || info!("esp32 bevy app started"));
	}
}
