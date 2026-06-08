//! Smoke test: does a scripting engine build and run `no_std` on the ESP32-S3?
//!
//! The engine is chosen by the enabled feature, matching
//! `ScriptLanguage::default()`'s precedence (QuickJS preferred, then rhai), so
//! the same example builds either way:
//!
//! ```sh
//! cargo run --release --no-default-features --features rhai    --example scripting
//! cargo run --release --no-default-features --features quickjs --example scripting
//! ```
//!
//! beet's own `scripting` feature pulls each engine with `std`, which can't
//! target `xtensa-esp32s3-none-elf`. Both ship a bare-metal build re-exported
//! through `beet::exports` (rhai `no_std`; rquickjs `rust-alloc`), so this
//! evaluates a couple of trivial scripts to prove the engine links and runs on
//! device before it is wired into a real controller (see `src/alvik/scripting.rs`).

#![no_std]
#![no_main]

use beet::prelude::*;
use beet_esp::prelude::*;

#[beet_esp::main]
fn main() {
    smoke_test();
    App::new().add_plugins((Esp32Plugin, HealthPlugin)).run();
}

// ---------------------------------------------------------------------------
// QuickJS backend (preferred, matching `ScriptLanguage::default()`).
// ---------------------------------------------------------------------------
#[cfg(feature = "quickjs")]
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

// ---------------------------------------------------------------------------
// rhai backend (when QuickJS is not enabled).
// ---------------------------------------------------------------------------
#[cfg(all(feature = "rhai", not(feature = "quickjs")))]
fn smoke_test() {
    use beet::exports::rhai;

    extern crate alloc;
    use alloc::string::String;

    let engine = rhai::Engine::new();
    let sum = engine.eval::<i64>("40 + 2").unwrap_or(-1);
    info!("rhai eval 40+2 = {}", sum);
    let text = engine.eval::<String>(r#""hello " + "rhai""#).unwrap_or_default();
    info!("rhai eval string = {}", text.as_str());
}

#[cfg(not(any(feature = "quickjs", feature = "rhai")))]
compile_error!("scripting needs a backend: enable `quickjs` or `rhai`");
