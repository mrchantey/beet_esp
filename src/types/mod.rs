//! Conversions from Bevy colours into microcontroller wire values.

use beet::prelude::Color;
use beet::prelude::ColorExt;
use esp_hal::gpio::Level;
use esp_hal::rmt::PulseCode;

// WS2812 bit timing in RMT ticks. The RMT clock is 80 MHz with divider 1, so
// one tick is 12.5 ns. Each bit is a high pulse then a low pulse; the ratio
// distinguishes 0 from 1. The period is ~1.25 us (800 kHz).
const T0H: u16 = 32; // 0.40 us high for a '0'
const T0L: u16 = 68; // 0.85 us low
const T1H: u16 = 68; // 0.85 us high for a '1'
const T1L: u16 = 32; // 0.40 us low

/// Number of RMT pulse codes for one WS2812 pixel: 24 data bits + end marker.
pub const PIXEL_CODES: usize = 24 + 1;

/// A single WS2812 pixel packed in wire order (green, red, blue), 8 bits per
/// channel — the "microcontroller value" a [`Color`] becomes on the wire.
#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub struct Grb(pub u32);

impl Grb {
	/// Convert a Bevy [`Color`] to a WS2812 pixel, scaling each channel by
	/// `brightness` (0..=255) since the bare LED is blinding at full scale.
	pub fn from_color(color: Color, brightness: u8) -> Self {
		let srgb = color.to_srgba_u8();
		let scale = |v: u8| (v as u16 * brightness as u16 / 255) as u32;
		Self((scale(srgb.green) << 16) | (scale(srgb.red) << 8) | scale(srgb.blue))
	}

	/// Encode this pixel into RMT pulse codes (MSB first, WS2812 expects it).
	pub fn encode(self, buf: &mut [PulseCode; PIXEL_CODES]) {
		let zero = PulseCode::new(Level::High, T0H, Level::Low, T0L);
		let one = PulseCode::new(Level::High, T1H, Level::Low, T1L);
		for (i, slot) in buf.iter_mut().take(24).enumerate() {
			*slot = if (self.0 >> (23 - i)) & 1 == 1 { one } else { zero };
		}
		buf[24] = PulseCode::end_marker();
	}
}

impl From<Color> for Grb {
	fn from(color: Color) -> Self {
		Self::from_color(color, 255)
	}
}
