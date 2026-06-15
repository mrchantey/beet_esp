//! Entity-based LED backends. [`LedColor`] is the one universal component every
//! backend reads; a per-entity marker selects the *write path*:
//!
//! - [`Ws2812Led`] — an addressable LED over RMT (this module).
//! - [`AlvikLed`](crate::alvik::AlvikLed) — an Alvik UI LED over UART, written
//!   by the `alvik` module (it aggregates several LEDs into one wire byte).
//!
//! The pure components ([`LedColor`], [`HueFade`], the [`Ws2812Led`] marker)
//! compile on the host so scenes can carry them; the RMT driver and its systems
//! live in the [`device`] submodule, re-exported only under the `device` feature.
//!
//! [`LedPlugin`] is registration-only: it installs the [`Ws2812Drivers`]
//! resource, the driver-claim and flush systems, but spawns no LED entity. Apps
//! spawn `(LedColor::default(), Ws2812Led)` (see `examples/blinky.rs`).

use beet::prelude::*;

/// Backend marker: this LED entity is an addressable WS2812 driven over RMT.
/// Pair it with a [`LedColor`]; [`flush_ws2812`](device::flush_ws2812) writes the
/// colour to hardware.
#[derive(Component, Clone, Copy, Default)]
pub struct Ws2812Led;

/// The colour an LED entity should currently show. App systems write it; the
/// backend flush system reads it and pushes it to the hardware.
#[derive(Component, Clone, Copy)]
pub struct LedColor(pub Color);

impl Default for LedColor {
    fn default() -> Self {
        Self(Color::BLACK)
    }
}

/// Cycles an entity's [`LedColor`] through the hue wheel. Attach it to an LED
/// entity and [`cycle_hue`](device::cycle_hue) advances it each update.
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

#[cfg(feature = "device")]
pub use device::*;

/// The WS2812 RMT driver and its systems — everything that needs the esp-hal
/// hardware stack, gated behind `device` so the pure components above still
/// compile for the host.
#[cfg(feature = "device")]
mod device {
    use super::HueFade;
    use super::LedColor;
    use super::Ws2812Led;
    use beet::prelude::*;
    use esp_hal::Blocking;
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

    /// The WS2812 drivers in play, keyed by their LED entity. A non-send resource
    /// because RMT channels are `!Send`. Populated at [`PostStartup`] by
    /// [`claim_ws2812_drivers`] from the peripherals
    /// [`bring_up`](crate::esp32_plugin) exposed; an entry's [`Ws2812`] drop
    /// resets the peripheral.
    #[derive(Default)]
    pub struct Ws2812Drivers(pub HashMap<Entity, Ws2812>);

    /// Registers the WS2812 backend: the [`Ws2812Drivers`] resource, the
    /// [`PostStartup`] claim of the on-board LED's peripherals, and the per-frame
    /// [`flush_ws2812`] write. Spawns no entity — the app does that.
    pub struct LedPlugin;

    impl Plugin for LedPlugin {
        fn build(&self, app: &mut App) {
            app.insert_non_send(Ws2812Drivers::default())
                .add_systems(PostStartup, claim_ws2812_drivers)
                .add_systems(Update, (cycle_hue, flush_ws2812).chain())
                .add_observer(release_ws2812_driver);
        }
    }

    /// Build a [`Ws2812`] for each [`Ws2812Led`] entity from the on-board LED's
    /// peripherals (`RMT` + `GPIO48`) and store it keyed by the entity.
    ///
    /// Exclusive so it can pull the non-send peripherals. Runs in [`PostStartup`],
    /// after the app's [`Startup`] spawns its LED entity. Only the single on-board
    /// LED is wired up here; supporting more would draw pins from a pool.
    fn claim_ws2812_drivers(world: &mut World) {
        let entities = world.with_state::<Query<Entity, With<Ws2812Led>>, _>(|query| {
            query.iter().collect::<alloc::vec::Vec<_>>()
        });
        let Some(&entity) = entities.first() else {
            return;
        };
        let rmt = world
            .remove_non_send::<RMT<'static>>()
            .expect("add Esp32Plugin before LedPlugin — bring_up provides the RMT peripheral");
        let pin = world
            .remove_non_send::<GPIO48<'static>>()
            .expect("add Esp32Plugin before LedPlugin — bring_up provides GPIO48");
        let led = Ws2812::new(rmt, pin);
        world
            .get_non_send_mut::<Ws2812Drivers>()
            .expect("LedPlugin inserts Ws2812Drivers")
            .0
            .insert(entity, led);
    }

    /// Drop a [`Ws2812Led`] entity's driver when the marker is removed, freeing
    /// the channel (its [`Ws2812`] drop resets the peripheral).
    fn release_ws2812_driver(
        remove: On<Remove, Ws2812Led>,
        mut drivers: NonSendMut<Ws2812Drivers>,
    ) {
        drivers.0.remove(&remove.entity);
    }

    /// Advances every [`HueFade`] and writes the resulting colour to its [`LedColor`].
    fn cycle_hue(mut query: Query<(&mut HueFade, &mut LedColor)>) {
        for (mut fade, mut color) in &mut query {
            fade.hue = (fade.hue + fade.step) % 360.0;
            color.0 = Color::hsl(fade.hue, 1.0, 0.5);
        }
    }

    /// Writes each changed [`Ws2812Led`] entity's [`LedColor`] to its driver.
    fn flush_ws2812(
        mut drivers: NonSendMut<Ws2812Drivers>,
        query: Populated<(Entity, &LedColor), (With<Ws2812Led>, Changed<LedColor>)>,
    ) {
        for (entity, color) in &query {
            if let Some(led) = drivers.0.get_mut(&entity) {
                led.write(color.0);
            }
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

    #[cfg(test)]
    mod test {
        use super::*;

        #[beet::test]
        fn grb_packs_in_wire_order() {
            // WS2812 wire order is green, red, blue: `g << 16 | r << 8 | b`.
            Grb::from_color(Color::srgb(1.0, 0.0, 0.0), 255).0.xpect_eq(0x00_FF_00);
            Grb::from_color(Color::srgb(0.0, 1.0, 0.0), 255).0.xpect_eq(0xFF_00_00);
            Grb::from_color(Color::srgb(0.0, 0.0, 1.0), 255).0.xpect_eq(0x00_00_FF);
        }

        #[beet::test]
        fn grb_scales_by_brightness() {
            // Half brightness halves every channel (255 * 128 / 255 == 128).
            Grb::from_color(Color::srgb(1.0, 1.0, 1.0), 128).0.xpect_eq(0x80_80_80);
        }
    }

    /// An on-board addressable LED (WS2812 / SK68xx) driven over RMT.
    ///
    /// Hides the RMT channel configuration and WS2812 bit-timing so callers just
    /// hand it a [`Color`]. The transmit is blocking — one pixel is ~30 us, far
    /// shorter than a schedule frame — so the write happens inline in a system, no
    /// async bridge needed. The channel is parked in an `Option` so each
    /// transmit can consume and return it.
    pub struct Ws2812 {
        channel: Option<Channel<'static, Blocking, Tx>>,
        buf: [PulseCode; PIXEL_CODES],
        brightness: u8,
    }

    impl Ws2812 {
        /// Configure RMT channel 0 to drive a WS2812 on `pin` (e.g. `GPIO48` on
        /// the official DevKitC-1). Brightness defaults to a non-blinding 24/255.
        pub fn new(rmt: RMT<'static>, pin: impl PeripheralOutput<'static>) -> Self {
            let rmt = Rmt::new(rmt, Rate::from_mhz(80)).expect("failed to initialise RMT");
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
                channel: Some(channel),
                buf: [PulseCode::end_marker(); PIXEL_CODES],
                brightness: 24,
            }
        }

        /// Overall brightness cap, 0..=255.
        pub fn with_brightness(mut self, brightness: u8) -> Self {
            self.brightness = brightness;
            self
        }

        /// Encode and transmit a single pixel of the given colour, blocking until
        /// the transmission completes (and parking the channel back for next time).
        pub fn write(&mut self, color: Color) {
            Grb::from_color(color, self.brightness).encode(&mut self.buf);
            let channel = self.channel.take().expect("channel parked between writes");
            self.channel = Some(match channel.transmit(&self.buf) {
                Ok(transaction) => match transaction.wait() {
                    Ok(channel) => channel,
                    Err((e, channel)) => {
                        error!("RMT transmit failed: {e:?}");
                        channel
                    }
                },
                Err((e, channel)) => {
                    error!("RMT transmit start failed: {e:?}");
                    channel
                }
            });
        }
    }
}
