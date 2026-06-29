//! Cross-cutting, hardware-light utilities: the thin peripheral wrappers that
//! don't belong to any one subsystem (the on-board WS2812 [`led`] and the
//! hardware-RNG [`random`] backend). The crate-wide typed quantities now live
//! upstream in `beet_core` (`beet::prelude::{Angle, AngularVelocity, Distance,
//! LinearVelocity}`); the prelude re-exports them so callers keep their old
//! `crate::prelude::LinearVelocity` paths.

#[cfg(feature = "led")]
pub mod led;
#[cfg(feature = "random")]
pub mod random;

pub mod prelude {
    #[cfg(feature = "led")]
    pub use super::led::*;
    // Typed quantities moved to beet_core; surface them here so the crate's old
    // `crate::utils::units::*` / `crate::prelude::*` import sites still resolve.
    pub use beet::prelude::{Angle, AngularVelocity, Distance, LinearVelocity};
}
