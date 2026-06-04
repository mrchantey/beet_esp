//! Cross-cutting, hardware-light utilities: pure `no_std` math ([`units`]) plus
//! the thin peripheral wrappers that don't belong to any one subsystem (the
//! on-board WS2812 [`led`] and the hardware-RNG [`random`] backend).

// Crate-wide typed quantities (pure no_std math, no hardware deps).
pub mod units;

#[cfg(feature = "led")]
pub mod led;
#[cfg(feature = "random")]
pub mod random;

pub mod prelude {
    #[cfg(feature = "led")]
    pub use super::led::*;
    pub use super::units::*;
}
