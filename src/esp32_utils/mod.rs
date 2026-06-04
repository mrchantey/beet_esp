//! ESP32 runtime plumbing shared across subsystems: heap/PSRAM bring-up
//! ([`mem`]), boot/health telemetry ([`health`]), the sync↔async [`async_bridge`]
//! (and its [`async_utils`] spawner), and the SNTP wall [`clock`]. The
//! hardware-agnostic [`esp32_plugin`](crate::esp32_plugin) wires these together.

pub mod async_bridge;
pub mod health;
pub mod mem;

#[cfg(feature = "action")]
pub mod async_utils;
#[cfg(feature = "clock")]
pub mod clock;

pub mod prelude {
    // Re-export the module, not its contents, so callers reach the primitives via
    // the `async_bridge::` prefix (e.g. `async_bridge::spawn_worker`).
    pub use super::async_bridge;
    pub use super::health::*;
    pub use super::mem::{External, Internal, PsramInfo};

    #[cfg(feature = "action")]
    pub use super::async_utils::*;
    #[cfg(feature = "clock")]
    pub use super::clock::*;
}
