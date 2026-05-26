//! ESP32 glue for running Bevy apps on the board: an async WS2812 driver, the
//! embassy/`esp-rtos` bring-up helper, and Bevy plugins that drive the logic.

use beet::prelude::*;
use crate::types::Grb;
use crate::types::PIXEL_CODES;
use defmt::info;
use esp_hal::Async;
use esp_hal::gpio::interconnect::PeripheralOutput;
use esp_hal::peripherals::RMT;
use esp_hal::rmt::Channel;
use esp_hal::rmt::PulseCode;
use esp_hal::rmt::Rmt;
use esp_hal::rmt::Tx;
use esp_hal::rmt::TxChannelConfig;
use esp_hal::rmt::TxChannelCreator;
use esp_hal::time::Rate;
use esp_hal::timer::timg::TimerGroup;
use esp_hal::gpio::Level;

/// Start the embassy executor on `esp-rtos` using timer group 0. Call once,
/// after [`esp_hal::init`], before spawning tasks or running an [`App`].
pub fn start_embassy(timg0: esp_hal::peripherals::TIMG0<'static>, sw: esp_hal::peripherals::SW_INTERRUPT<'static>) {
	let timg0 = TimerGroup::new(timg0);
	let sw_interrupt = esp_hal::interrupt::software::SoftwareInterruptControl::new(sw);
	esp_rtos::start(timg0.timer0, sw_interrupt.software_interrupt0);
}

/// An on-board addressable LED (WS2812 / SK68xx) driven over RMT in async mode.
///
/// Hides the RMT channel configuration and WS2812 bit-timing so callers just
/// hand it a [`Color`].
pub struct Ws2812 {
	channel: Channel<'static, Async, Tx>,
	buf: [PulseCode; PIXEL_CODES],
	brightness: u8,
}

impl Ws2812 {
	/// Configure RMT channel 0 to drive a WS2812 on `pin` (e.g. `GPIO48` on the
	/// official DevKitC-1). Brightness defaults to a non-blinding 24/255.
	pub fn new(rmt: RMT<'static>, pin: impl PeripheralOutput<'static>) -> Self {
		let rmt = Rmt::new(rmt, Rate::from_mhz(80))
			.expect("failed to initialise RMT")
			.into_async();
		let channel = rmt
			.channel0
			.configure_tx(
				&TxChannelConfig::default()
					.with_clk_divider(1)
					.with_idle_output(true)
					.with_idle_output_level(Level::Low)
					.with_carrier_modulation(false),
			)
			.expect("failed to configure RMT TX channel")
			.with_pin(pin);
		Self {
			channel,
			buf: [PulseCode::end_marker(); PIXEL_CODES],
			brightness: 24,
		}
	}

	/// Overall brightness cap, 0..=255.
	pub fn with_brightness(mut self, brightness: u8) -> Self {
		self.brightness = brightness;
		self
	}

	/// Encode and transmit a single pixel of the given colour.
	pub async fn write(&mut self, color: Color) {
		Grb::from_color(color, self.brightness).encode(&mut self.buf);
		if let Err(e) = self.channel.transmit(&self.buf).await {
			defmt::error!("RMT transmit failed: {}", e);
		}
	}
}

/// The colour the LED should currently show. Updated by app systems and read by
/// the render loop.
#[derive(Resource, Clone, Copy)]
pub struct LedColor(pub Color);

impl Default for LedColor {
	fn default() -> Self {
		Self(Color::BLACK)
	}
}

/// Current hue in degrees (0..360).
#[derive(Resource, Clone, Copy, Default)]
pub struct Hue(pub f32);

/// Baseline scaffolding for an esp32 Bevy app: a startup banner.
pub struct Esp32Plugin;

impl Plugin for Esp32Plugin {
	fn build(&self, app: &mut App) {
		app.add_systems(Startup, || info!("esp32 bevy app started"));
	}
}

/// Rotates the hue every update and writes the resulting colour to [`LedColor`].
pub struct HueFadePlugin {
	/// Degrees of hue advanced per update.
	pub step: f32,
}

impl Default for HueFadePlugin {
	fn default() -> Self {
		Self { step: 1.5 }
	}
}

impl Plugin for HueFadePlugin {
	fn build(&self, app: &mut App) {
		let step = self.step;
		app.init_resource::<Hue>()
			.init_resource::<LedColor>()
			.add_systems(
				Update,
				move |mut hue: ResMut<Hue>, mut color: ResMut<LedColor>| {
					hue.0 = (hue.0 + step) % 360.0;
					color.0 = Color::hsl(hue.0, 1.0, 0.5);
				},
			);
	}
}
