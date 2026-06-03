//! Bare-metal glue for the [QuickJS](https://bellard.org/quickjs/) engine
//! (the `quickjs` feature), reached through [`beet::exports::rquickjs`].
//!
//! Two things the engine needs that a no_std esp-hal build does not provide out
//! of the box: a [`console`](install_console) for scripts to log through, and a
//! clock. The clock is wired at the C level: `src/quickjs_shim.c` calls the
//! [`beet_esp_monotonic_ns`] / [`beet_esp_wall_us`] hooks defined here.

use beet::exports::rquickjs;
use beet::prelude::*;
use defmt::error;
use defmt::info;
use esp_hal::time::Instant;
use rquickjs::Coerced;
use rquickjs::Ctx;
use rquickjs::Function;
use rquickjs::Object;
use rquickjs::Value;
use rquickjs::function::Rest;

extern crate alloc;
use alloc::string::String;

/// Install a `console` global on `ctx` whose `log`, `error` and `dir` methods
/// stream to `defmt` over RTT. `log`/`error` join their arguments with spaces
/// (JS string coercion); `dir` JSON-renders each argument so objects show their
/// structure rather than `[object Object]`.
pub fn install_console(ctx: &Ctx<'_>) -> rquickjs::Result<()> {
    let console = Object::new(ctx.clone())?;

    console.set(
        "log",
        Function::new(ctx.clone(), |args: Rest<Coerced<String>>| {
            info!("console.log: {}", join(args).as_str());
        })?,
    )?;
    console.set(
        "error",
        Function::new(ctx.clone(), |args: Rest<Coerced<String>>| {
            error!("console.error: {}", join(args).as_str());
        })?,
    )?;
    console.set(
        "dir",
        Function::new(ctx.clone(), |args: Rest<Value<'_>>| {
            info!("console.dir: {}", render(args).as_str());
        })?,
    )?;

    ctx.globals().set("console", console)?;
    Ok(())
}

/// Join already-coerced string arguments with spaces, mirroring `console.log`.
fn join(args: Rest<Coerced<String>>) -> String {
    args.0
        .iter()
        .map(|arg| arg.as_str())
        .collect::<alloc::vec::Vec<_>>()
        .join(" ")
}

/// Render arguments for `console.dir`: JSON where possible (so objects expand),
/// falling back to plain string coercion for values JSON can't represent. Each
/// value carries its own context, so no separate [`Ctx`] argument is needed.
fn render(args: Rest<Value<'_>>) -> String {
    args.0
        .into_iter()
        .map(|value| {
            value
                .ctx()
                .json_stringify(value.clone())
                .ok()
                .flatten()
                .and_then(|json| json.to_string().ok())
                .or_else(|| {
                    value.get::<Coerced<String>>().ok().map(|str| str.0)
                })
                .unwrap_or_default()
        })
        .collect::<alloc::vec::Vec<_>>()
        .join(" ")
}

/// Monotonic clock in nanoseconds, backing the engine's internal timing
/// (`clock_gettime(CLOCK_MONOTONIC)`). Sourced from the esp-hal system timer, so
/// it is valid as soon as `mem::init_esp` has run — no embassy driver required.
#[unsafe(no_mangle)]
extern "C" fn beet_esp_monotonic_ns() -> u64 {
    Instant::now().duration_since_epoch().as_micros().saturating_mul(1000)
}

/// Wall-clock time in microseconds since the Unix epoch, backing `Date`
/// (`gettimeofday`). Served from beet's [`time_ext`] hook, which the
/// `ClockPlugin` disciplines over SNTP. Returns 0 until a sync lands rather than
/// substituting monotonic time, which would read as a wildly wrong wall clock.
#[unsafe(no_mangle)]
extern "C" fn beet_esp_wall_us() -> i64 {
    time_ext::try_now()
        .map(|since_epoch| since_epoch.as_micros() as i64)
        .unwrap_or(0)
}

/// The QuickJS C engine calls `abort()` on unrecoverable internal errors. Route
/// it into a Rust panic so it reaches the `panic-rtt-target` handler, rather
/// than pulling newlib's `abort` (which drags in `raise`/`_kill` syscall stubs
/// absent on bare metal).
#[unsafe(no_mangle)]
extern "C" fn abort() -> ! {
    panic!("quickjs C runtime called abort()");
}
