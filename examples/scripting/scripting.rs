//! Smoke test: does the QuickJS engine build and run `no_std` on the ESP32-S3?
//!
//! ```sh
//! cargo run --release --no-default-features --features quickjs --example scripting
//! ```
//!
//! The engine's bare-metal build (rquickjs `rust-alloc`, routing the C engine's
//! allocations through esp-alloc) is re-exported through `beet::exports`, so this
//! evaluates a couple of trivial scripts to prove it links and runs on device
//! before it is wired into a real controller (see `src/alvik/scripting.rs`).

#![no_std]
#![no_main]

use beet::prelude::*;
use beet_esp::prelude::*;

#[beet_esp::main]
fn main() {
    smoke_test();
    App::new().add_plugins((Esp32Plugin, HealthPlugin)).run();
}

fn smoke_test() {
    use beet::exports::rquickjs::Context;
    use beet::exports::rquickjs::Runtime;

    extern crate alloc;
    use alloc::string::String;

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
            Err(err) => info!("quickjs arrow fn failed: {:?}", err),
        }

        // `console.log` / `error` / `dir` stream to the `log` facade over RTT.
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
}

