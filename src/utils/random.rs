//! getrandom `custom` backend for this bare ESP32-S3 target.
//!
//! getrandom 0.3 ships no entropy source for `*-none-elf` targets, so
//! `.cargo/config.toml` selects its `custom` backend
//! (`--cfg getrandom_backend="custom"`) and this module supplies the
//! `__getrandom_v03_custom` symbol getrandom links against, backed by the
//! esp-hal hardware RNG.
//!
//! That is the one piece an app needs to use beet's
//! [`RandomSource`](beet::prelude::RandomSource) on-device: its
//! `default()` seeds a ChaCha generator from `getrandom`, which routes here.
//!
//! With the RF subsystem up (Wi-Fi/BLE enabled) the hardware RNG is a true
//! TRNG; without it the output is only pseudo-random — see the [`esp_hal::rng`]
//! docs for the entropy pre-conditions.

use esp_hal::rng::Rng;

/// getrandom 0.3 `custom` backend hook: fill `len` bytes at `dest` from the
/// esp-hal hardware RNG. Linked in automatically by getrandom; not called
/// directly.
///
/// # Safety
///
/// `dest` must be non-null and valid for writes of `len` bytes. getrandom
/// upholds this contract for every call.
#[unsafe(no_mangle)]
unsafe extern "Rust" fn __getrandom_v03_custom(
    dest: *mut u8,
    len: usize,
) -> Result<(), getrandom::Error> {
    // SAFETY: getrandom guarantees `dest` is valid for `len` writes.
    let buffer = unsafe { core::slice::from_raw_parts_mut(dest, len) };
    Rng::new().read(buffer);
    Ok(())
}
