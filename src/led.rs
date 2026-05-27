//! On-board addressable LED: the WS2812 RMT driver, its Bevy components and
//! hue-fade animation, and the GRB wire encoding a [`Color`] becomes.

use crate::bridge::Latest;
use crate::bridge::spawn_driver;
use beet::prelude::*;
use embassy_executor::Spawner;
use esp_hal::Async;
use esp_hal::gpio::Level;
use esp_hal::gpio::interconnect::PeripheralOutput;
use esp_hal::peripherals::GPIO48;
use esp_hal::peripherals::RMT;
use esp_hal::rmt::Channel;
use esp_hal::rmt::PulseCode;
use esp_hal::rmt::Rmt;
use esp_hal::rmt::Tx;
use esp_hal::rmt::TxChannelConfig;
use esp_hal::rmt::TxChannelCreator;
use esp_hal::time::Rate;

/// Latest-wins pipe carrying the desired LED colour from Bevy systems to the
/// async RMT driver. Systems push via [`flush_led`]; the driver pulls the newest
/// value and writes it, dropping any colour superseded while a write is in
/// flight — exactly the "hold off until the current write completes" behaviour.
pub static LED_IN: Latest<Color> = Latest::new();

/// Owns everything LED: claims the on-board WS2812's peripherals and spawns its
/// async RMT driver at startup, spawns an LED entity, and advances any
/// [`HueFade`] each update — pushing the colour to the driver via [`LED_IN`].
#[derive(Default)]
pub struct LedPlugin {
    /// Hue-fade parameters for the LED entity spawned at startup.
    pub hue_fade: HueFade,
}

impl Plugin for LedPlugin {
    fn build(&self, app: &mut App) {
        let hue_fade = self.hue_fade;
        app.add_systems(
            Startup,
            (spawn_led_driver, move |mut commands: Commands| {
                commands.spawn((LedColor::default(), hue_fade));
            }),
        )
        .add_systems(Update, (cycle_hue, flush_led).chain());
    }
}

/// Claim the LED's peripherals (`RMT` + `GPIO48`) that
/// [`bring_up`](crate::esp32_plugin) exposed, build the [`Ws2812`], and spawn
/// the async driver that writes the latest [`LED_IN`] colour over RMT — holding
/// off the next pull until the in-flight write completes.
///
/// Exclusive so it can pull the non-send peripherals and the [`Spawner`] from
/// the world. Runs in `Startup`, after `bring_up`'s `PreStartup`.
fn spawn_led_driver(world: &mut World) {
    let rmt = world
        .remove_non_send::<RMT<'static>>()
        .expect("add Esp32Plugin before LedPlugin — bring_up provides the RMT peripheral");
    let pin = world
        .remove_non_send::<GPIO48<'static>>()
        .expect("add Esp32Plugin before LedPlugin — bring_up provides GPIO48");
    let spawner = *world.non_send::<Spawner>();

    let mut led = Ws2812::new(rmt, pin);
    spawn_driver(spawner, async move {
        loop {
            let color = LED_IN.recv().await;
            led.write(color).await;
        }
    });
}

/// Advances every [`HueFade`] and writes the resulting colour to its [`LedColor`].
fn cycle_hue(mut query: Query<(&mut HueFade, &mut LedColor)>) {
    for (mut fade, mut color) in &mut query {
        fade.hue = (fade.hue + fade.step) % 360.0;
        color.0 = Color::hsl(fade.hue, 1.0, 0.5);
    }
}

/// Pushes the LED entity's current [`LedColor`] into [`LED_IN`] for the async
/// driver to pick up.
fn flush_led(query: Query<&LedColor>) {
    if let Ok(color) = query.single() {
        LED_IN.send(color.0);
    }
}

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
#[derive(Clone, Copy, Default, PartialEq, Eq, Deref, DerefMut)]
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
            *slot = if (self.0 >> (23 - i)) & 1 == 1 {
                one
            } else {
                zero
            };
        }
        buf[24] = PulseCode::end_marker();
    }
}

impl From<Color> for Grb {
    fn from(color: Color) -> Self {
        Self::from_color(color, 255)
    }
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
/// entity and [`LedPlugin`] advances it each update.
#[derive(Component, Clone, Copy)]
pub struct HueFade {
    /// Current hue in degrees, 0..360.
    pub hue: f32,
    /// Degrees of hue advanced per update.
    pub step: f32,
}

impl Default for HueFade {
    fn default() -> Self {
        Self {
            hue: 0.0,
            step: 1.5,
        }
    }
}
