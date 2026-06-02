//! Smoke test: does the [rhai](https://rhai.rs) scripting engine build and run
//! `no_std` on the ESP32-S3?
//!
//! beet's own `scripting` feature pulls rhai with `std`, which can't target
//! `xtensa-esp32s3-none-elf`. rhai *does* ship a `no_std` build (libm +
//! hashbrown + core-error), so this evaluates two trivial scripts to prove the
//! engine links and runs on bare metal before wiring it into a real controller
//! (see [`alvik/scene.rs`](./alvik/scene.rs)'s `scripting` module).
//!
//! The script's input/output are marshalled by hand through rhai `Map`s rather
//! than serde: rhai's `serde` feature conflicts with serde's `no_std`/alloc
//! split (serde_core wants `std`), so the engine is built without it.
//!
//! Run with: `cargo run --release --no-default-features --features scripting --example rhai_spike`

#![no_std]
#![no_main]

use beet::prelude::*;
use beet_esp::prelude::*;
use defmt::info;

extern crate alloc;
use alloc::string::String;

#[beet_esp::main(internal_reserve_kb = 64)]
fn main() {
    let engine = rhai::Engine::new();
    let sum = engine.eval::<i64>("40 + 2").unwrap_or(-1);
    info!("rhai eval 40+2 = {}", sum);
    let text = engine.eval::<String>(r#""hello " + "rhai""#).unwrap_or_default();
    info!("rhai eval string = {}", text.as_str());
    App::new().add_plugins((Esp32Plugin, HealthPlugin)).run();
}
