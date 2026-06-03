//! Smoke test: does the [QuickJS](https://bellard.org/quickjs/) engine build and
//! run `no_std` on the ESP32-S3?
//!
//! QuickJS is a C engine; rquickjs bundles it and compiles it with the `cc`
//! crate. The `quickjs` feature selects rquickjs's `rust-alloc` mode (no `std`),
//! routing the engine's allocations through esp-alloc's global allocator, and
//! re-exports it as [`beet::exports::rquickjs`]. This evaluates two trivial
//! scripts to prove the engine links and runs on bare metal before wiring it
//! into a real controller.
//!
//! Run with: `cargo run --release --no-default-features --features quickjs --example scripting_quickjs`

#![no_std]
#![no_main]

use beet::exports::rquickjs;
use beet::prelude::*;
use beet_esp::prelude::*;
use defmt::info;

extern crate alloc;
use alloc::string::String;
use rquickjs::Context;
use rquickjs::Runtime;

#[beet_esp::main]
fn main() {
    // `new_esp` caps QuickJS's stack-overflow guard to fit the esp stack (the
    // 1 MB default would let deep scripts hard-fault instead of erroring).
    let runtime = Runtime::new_esp().unwrap();
    let context = Context::full(&runtime).unwrap();
    context.with(|ctx| {
        let sum = ctx.eval::<i64, _>("40 + 2").unwrap_or(-1);
        info!("quickjs eval 40+2 = {}", sum);
        let text = ctx
            .eval::<String, _>(r#""hello " + "quickjs""#)
            .unwrap_or_default();
        info!("quickjs eval string = {}", text.as_str());

        // Arrow function: confirms whether the capped stack lets the parser's
        // arrow-disambiguation pass run instead of overflowing.
        match ctx.eval::<i64, _>("(() => 6 * 7)()") {
            Ok(value) => info!("quickjs arrow fn = {}", value),
            Err(err) => info!("quickjs arrow fn failed: {}", defmt::Debug2Format(&err)),
        }

        // `console.log` / `error` / `dir` stream to defmt over RTT.
        install_console(&ctx).unwrap();
        ctx.eval::<(), _>(
            r#"
            console.log("hello from", "js", 1 + 2);
            console.error("this is an error line");
            console.dir({ name: "alvik", speed: 42, tags: ["a", "b"] });
            "#,
        )
        .unwrap();
    });
    App::new().add_plugins((Esp32Plugin, HealthPlugin)).run();
}
