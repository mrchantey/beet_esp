//! Cross-cutting, hardware-light utilities: the thin peripheral wrappers that
//! don't belong to any one subsystem (the on-board WS2812 [`led`] and the
//! hardware-RNG [`random`] backend). The crate-wide typed quantities now live
//! upstream in `beet_core`; import them directly from `beet::prelude`
//! (`Angle`, `AngularVelocity`, `Distance`, `LinearVelocity`) at each use site.

#[cfg(feature = "led")]
pub mod led;
#[cfg(feature = "random")]
pub mod random;

pub mod prelude {
    #[cfg(feature = "led")]
    pub use super::led::*;
}
