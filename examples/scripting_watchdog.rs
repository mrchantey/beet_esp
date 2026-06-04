//! Bounding runaway *synchronous* QuickJS scripts with `Runtime::set_deadline`,
//! which installs a QuickJS interrupt handler that aborts a script once it has run
//! past a time budget.
//!
//! 1. `while (true) {}` with a 2 s deadline returns an `Err` (aborted) instead of
//!    wedging forever.
//! 2. A normal short script still completes with a deadline set — the guard only
//!    fires on overruns.
//!
//! Note there is no hardware watchdog to feed here: esp-hal's `init` disables them
//! all and esp-rtos arms none, so a long sync script hogs the executor but does
//! not reset the chip. The deadline is what bounds runaways; it is also the hook
//! to feed a watchdog from, if one is ever armed.
//!
//! Run with: `cargo run --release --no-default-features --features quickjs --example scripting_watchdog`

#![no_std]
#![no_main]

use beet::exports::rquickjs::Context;
use beet::exports::rquickjs::Runtime;
use beet::prelude::*;
use beet_esp::prelude::*;
use core::time::Duration;
use defmt::info;
use esp_hal::time::Instant;

fn now_ms() -> u64 {
    Instant::now().duration_since_epoch().as_micros() / 1000
}

#[beet_esp::main]
fn main() {
    let runtime = Runtime::new_esp().unwrap();
    let context = Context::full(&runtime).unwrap();

    // 1) The deadline guard against an infinite loop: the interrupt handler
    //    aborts it after 2 s, so eval returns Err instead of hanging forever.
    runtime.set_deadline(Duration::from_secs(2));
    context.with(|ctx| {
        let start = now_ms();
        let aborted = ctx.eval::<i64, _>("while (true) {}").is_err();
        info!("infinite loop: aborted={} elapsed={}ms", aborted, now_ms() - start);
    });
    runtime.clear_deadline();

    // 2) A normal short script still completes with a deadline set — the guard
    //    only fires on overruns, so it does not interfere with real scripts.
    runtime.set_deadline(Duration::from_secs(5));
    context.with(|ctx| {
        let start = now_ms();
        let value = ctx
            .eval::<i64, _>("let s = 0; for (let i = 0; i < 10000; i++) { s += i; } s")
            .unwrap_or(-1);
        info!("bounded script: value={} elapsed={}ms", value, now_ms() - start);
    });
    runtime.clear_deadline();

    App::new().add_plugins((Esp32Plugin, HealthPlugin)).run();
}
