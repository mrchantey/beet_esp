//! Scene-carried control scripting. A backend engine evaluates the program a
//! scene ships: [`rhai`] (pure-Rust, `no_std`) and [`quickjs`] (the bundled C
//! engine). The engine-agnostic [`Script`](rhai::Script) component plus the
//! per-domain step systems live with the [`rhai`] backend today; [`quickjs`] is
//! the smoke-tested alternative engine.

#[cfg(feature = "quickjs")]
pub mod quickjs;
#[cfg(feature = "rhai")]
pub mod rhai;

pub mod prelude {
    #[cfg(feature = "quickjs")]
    pub use super::quickjs::{RuntimeEspExt, install_console};
    #[cfg(feature = "rhai")]
    pub use super::rhai::*;
}
