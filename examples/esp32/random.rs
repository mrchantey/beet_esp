//! beet `RandomSource` on the ESP32 — proving randomness works in `no_std`.
//!
//! beet's [`RandomSource`] is a ChaCha generator. `from_seed` is pure and works
//! anywhere, but `default()` needs *entropy*, which on a bare target means a
//! `getrandom` backend. getrandom 0.3 ships none for `*-none-elf`, so this crate
//! selects getrandom's `custom` backend (`getrandom_backend="custom"` in
//! `.cargo/config.toml`) and supplies the entropy symbol in
//! [`beet_esp::utils::random`] — backed by the esp-hal hardware RNG.
//!
//! This example exercises both halves:
//! - [`seeded_is_reproducible`]: the same seed yields the same sequence (the
//!   ChaCha core), and reproduces beet_core's documented `22` test vector.
//! - [`entropy_draws`]: `RandomSource::default()` is entropy-seeded, so its draws
//!   prove the custom backend is linked and the hardware RNG is feeding it.
//!
//! No Wi-Fi here, so the RNG is pseudo-random (the RF subsystem is off — see the
//! `esp_hal::rng` entropy pre-conditions); the point is that the getrandom
//! plumbing links and runs.
//!
//! Run with: `cargo run --release --no-default-features --features random --example random`

#![no_std]
#![no_main]

use beet::prelude::*;
use beet_esp::prelude::*;

#[beet_esp::main]
fn main() {
    let mut app = App::new();
    app.add_plugins((Esp32Plugin, HealthPlugin));
    // Runs in `Startup`, after `Esp32Plugin`'s `PreStartup` bring-up has
    // initialised the chip — the hardware RNG (and so getrandom) is only ready
    // then, which is why we don't build a `RandomSource::default()` in `main`.
    app.add_systems(Startup, random_demo);
    app.run();
}

/// Exercises both halves of [`RandomSource`]: the deterministic seeded core and
/// the entropy-seeded `default()` (which routes through the getrandom custom
/// backend in [`beet_esp::utils::random`] to the esp-hal hardware RNG).
fn random_demo() {
    // -- seeded: same seed, same sequence (no entropy needed) --
    let mut a = RandomSource::from_seed(7);
    let mut b = RandomSource::from_seed(7);
    for _ in 0..4 {
        let x = a.random::<u32>();
        let y = b.random::<u32>();
        assert_eq!(x, y, "from_seed must be deterministic");
    }
    info!("from_seed(7) is reproducible");
    let v = RandomSource::from_seed(7).random_range(10u32..100);
    info!("from_seed(7).random_range(10..100) = {} (expect 22)", v);

    // -- entropy: default() seeds via getrandom -> custom backend -> HW RNG --
    let mut source = RandomSource::default();
    info!("entropy draws (getrandom custom backend -> esp-hal RNG):");
    for _ in 0..5 {
        info!("  d6 roll: {}", source.random_range(1u32..=6));
    }
    let bytes: [u8; 8] = core::array::from_fn(|_| source.random::<u8>());
    info!("  8 random bytes: {:?}", bytes);
}
