//! Keeps a scripting engine resident and evaluates a small script in a loop,
//! reporting heap cost and per-eval time so rhai and QuickJS can be compared on
//! device. The engine is chosen by the enabled feature, matching
//! `ScriptLanguage::default()`'s precedence (QuickJS preferred, then rhai), so
//! the same example builds either way:
//!
//! ```sh
//! cargo run --release --no-default-features --features quickjs --example scripting_bench
//! cargo run --release --no-default-features --features rhai    --example scripting_bench
//! ```

#![no_std]
#![no_main]

use beet::prelude::*;
use beet_esp::esp32_utils::health;
use beet_esp::prelude::*;
use defmt::info;
use esp_hal::time::Instant;

/// How many times to evaluate the script. Kept modest so the synchronous loop
/// finishes well inside the task watchdog window (each eval re-parses + runs).
const ITERATIONS: u32 = 100;

#[beet_esp::main]
fn main() {
    // Snapshot only at the boundaries: heap stats come from `esp_alloc`, which
    // tracks the lifetime peak itself (`heap_max_used`), so there is no need to
    // poll inside the hot loop (and `snapshot` walks the stack, which is not for
    // a tight loop).
    let baseline = health::snapshot();
    let (after_engine, after_run, elapsed_us, checksum) = run_benchmark();

    let engine_resident = after_engine.heap_used.saturating_sub(baseline.heap_used);
    let peak = after_run.heap_max_used.saturating_sub(baseline.heap_used);

    info!("── scripting benchmark: {} ──", ENGINE);
    info!("iterations:      {}", ITERATIONS);
    info!("checksum:        {} (guards against dead-code elimination)", checksum);
    info!("engine resident: {} B heap (held while idle)", engine_resident);
    info!("peak heap:       {} B above baseline (lifetime max)", peak);
    info!("total time:      {} us", elapsed_us);
    info!("per eval:        {} us", elapsed_us / ITERATIONS as u64);

    App::new().add_plugins((Esp32Plugin, HealthPlugin)).run();
}

/// Microseconds since boot, from the esp-hal system timer.
fn now_us() -> u64 {
    Instant::now().duration_since_epoch().as_micros()
}

// ---------------------------------------------------------------------------
// QuickJS backend (preferred, matching `ScriptLanguage::default()`).
// ---------------------------------------------------------------------------
#[cfg(feature = "quickjs")]
const ENGINE: &str = "quickjs";

/// Returns `(after_engine, after_run, elapsed_us, checksum)`.
#[cfg(feature = "quickjs")]
fn run_benchmark() -> (health::MemSnapshot, health::MemSnapshot, u64, i64) {
    use beet::exports::rquickjs::Context;
    use beet::exports::rquickjs::Runtime;

    // An IIFE so the script's completion value is the returned number.
    const SCRIPT: &str =
        "(() => { let s = 0; for (let i = 0; i < 50; i++) { s += i * 2; } return s; })()";

    // `new_esp` caps the stack-overflow guard to fit the esp stack.
    let runtime = Runtime::new_esp().unwrap();
    let context = Context::full(&runtime).unwrap();
    let after_engine = health::snapshot();

    let mut checksum = 0i64;
    let start = now_us();
    context.with(|ctx| {
        for _ in 0..ITERATIONS {
            checksum = checksum.wrapping_add(ctx.eval::<i64, _>(SCRIPT).unwrap());
        }
    });
    (after_engine, health::snapshot(), now_us() - start, checksum)
}

// ---------------------------------------------------------------------------
// rhai backend (when QuickJS is not enabled).
// ---------------------------------------------------------------------------
#[cfg(all(feature = "rhai", not(feature = "quickjs")))]
const ENGINE: &str = "rhai";

/// Returns `(after_engine, after_run, elapsed_us, checksum)`.
#[cfg(all(feature = "rhai", not(feature = "quickjs")))]
fn run_benchmark() -> (health::MemSnapshot, health::MemSnapshot, u64, i64) {
    use beet::exports::rhai::Engine;

    const SCRIPT: &str = "let s = 0; for i in 0..50 { s += i * 2; } s";

    let engine = Engine::new();
    let after_engine = health::snapshot();

    let mut checksum = 0i64;
    let start = now_us();
    for _ in 0..ITERATIONS {
        checksum = checksum.wrapping_add(engine.eval::<i64>(SCRIPT).unwrap());
    }
    (after_engine, health::snapshot(), now_us() - start, checksum)
}

#[cfg(not(any(feature = "quickjs", feature = "rhai")))]
compile_error!("scripting_bench needs a backend: enable `quickjs` or `rhai`");
