//! Smoke test: does the [rhai](https://rhai.rs) scripting engine build and run
//! `no_std` on the ESP32-S3?
//!
//! beet's own `scripting` feature pulls rhai with `std`, which can't target
//! `xtensa-esp32s3-none-elf`. rhai *does* ship a `no_std` build, re-exported as
//! [`beet::exports::rhai`], so this evaluates two trivial scripts to prove the
//! engine links and runs on bare metal before wiring it into a real controller
//! (see `src/alvik/scripting.rs`).
//!
//! Run with: `cargo run --release --no-default-features --features scripting --example scripting`

#![no_std]
#![no_main]

use beet::exports::rhai;
use beet::prelude::*;
use beet_esp::prelude::*;
use defmt::info;

extern crate alloc;
use alloc::string::String;

#[beet_esp::main]
fn main() {
    let engine = rhai::Engine::new();
    let sum = engine.eval::<i64>("40 + 2").unwrap_or(-1);
    info!("rhai eval 40+2 = {}", sum);
    let text = engine.eval::<String>(r#""hello " + "rhai""#).unwrap_or_default();
    info!("rhai eval string = {}", text.as_str());
    App::new().add_plugins((Esp32Plugin, HealthPlugin)).run();
}
