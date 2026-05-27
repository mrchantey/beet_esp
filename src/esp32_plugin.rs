//! ESP32 glue for running Bevy apps on the board: an async WS2812 driver, the
//! embassy/`esp-rtos` bring-up helper, and Bevy plugins that drive the logic.

use beet::prelude::*;
use crate::types::Grb;
use crate::types::PIXEL_CODES;
use defmt::info;
use esp_hal::Async;
use esp_hal::clock::CpuClock;
use esp_hal::gpio::interconnect::PeripheralOutput;
use esp_hal::peripherals::Peripherals;
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

/// Board bring-up that must run before constructing a Bevy [`App`]: installs
/// the heap (the `World` allocates) and sets the CPU clock. Returns the
/// peripherals so the caller can start embassy and wire up hardware that the
/// async render loop drives (e.g. the [`Ws2812`]).
///
/// Initialise RTT/`defmt` logging (`rtt_target::rtt_init_defmt!()`) in `main`
/// before calling this — the RTT control block is a per-binary singleton, so it
/// can't live behind a shared helper.
pub fn init_board() -> Peripherals {
	esp_alloc::heap_allocator!(size: 96 * 1024);
	esp_hal::init(esp_hal::Config::default().with_cpu_clock(CpuClock::max()))
}

/// Start the embassy executor on `esp-rtos` using timer group 0. Call once,
/// after [`init_board`], before spawning tasks or running an [`App`].
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

/// The colour an LED entity should currently show. App systems write it; the
/// async render loop reads it and pushes it to the hardware.
#[derive(Component, Clone, Copy)]
pub struct LedColor(pub Color);

impl Default for LedColor {
	fn default() -> Self {
		Self(Color::BLACK)
	}
}

/// Cycles an entity's [`LedColor`] through the hue wheel. Attach it to an LED
/// entity and [`Esp32Plugin`] advances it each update.
#[derive(Component, Clone, Copy)]
pub struct HueFade {
	/// Current hue in degrees, 0..360.
	pub hue: f32,
	/// Degrees of hue advanced per update.
	pub step: f32,
}

impl Default for HueFade {
	fn default() -> Self {
		Self { hue: 0.0, step: 1.5 }
	}
}

/// Baseline scaffolding for an esp32 Bevy app: logs a startup banner, spawns an
/// LED entity, and advances any [`HueFade`] each update.
pub struct Esp32Plugin {
	/// Hue-fade parameters for the LED entity spawned at startup.
	pub hue_fade: HueFade,
}

impl Default for Esp32Plugin {
	fn default() -> Self {
		Self { hue_fade: HueFade::default() }
	}
}

impl Plugin for Esp32Plugin {
	fn build(&self, app: &mut App) {
		let hue_fade = self.hue_fade;
		app.add_systems(Startup, move |mut commands: Commands| {
			info!("esp32 bevy app started");
			commands.spawn((LedColor::default(), hue_fade));
		})
		.add_systems(Update, cycle_hue);
	}
}

/// Advances every [`HueFade`] and writes the resulting colour to its [`LedColor`].
fn cycle_hue(mut query: Query<(&mut HueFade, &mut LedColor)>) {
	for (mut fade, mut color) in &mut query {
		fade.hue = (fade.hue + fade.step) % 360.0;
		color.0 = Color::hsl(fade.hue, 1.0, 0.5);
	}
}
