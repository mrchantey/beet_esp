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
    let runtime = Runtime::new().unwrap();
    // QuickJS's stack-overflow guard defaults to a 1 MB budget; on the small esp
    // stack that sits past the real stack bottom, so deep scripts would smash the
    // stack instead of raising a clean RangeError. Cap it to a real budget.
    runtime.set_max_stack_size(64 * 1024);
    let context = Context::full(&runtime).unwrap();
    context.with(|ctx| {
        let sum = ctx.eval::<i64, _>("40 + 2").unwrap_or(-1);
        info!("quickjs eval 40+2 = {}", sum);
        let text = ctx
            .eval::<String, _>(r#""hello " + "quickjs""#)
            .unwrap_or_default();
        info!("quickjs eval string = {}", text.as_str());

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
