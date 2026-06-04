//! Bare-metal glue for the [QuickJS](https://bellard.org/quickjs/) engine
//! (the `quickjs` feature), reached through [`beet::exports::rquickjs`].
//!
//! Two things the engine needs that a no_std esp-hal build does not provide out
//! of the box: a [`console`](install_console) for scripts to log through, and a
//! clock. The clock is wired at the C level: `src/scripting/quickjs_shim.c` calls the
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
use rquickjs::Runtime;
use rquickjs::Value;
use rquickjs::function::Rest;

extern crate alloc;
use alloc::string::String;

/// The C stack budget handed to QuickJS on this target. The engine's default is
/// 1 MB, which overshoots the esp main-task stack: its overflow guard then sits
/// past the real stack bottom, so deep scripts smash the stack (hard fault)
/// instead of raising a `RangeError`. This keeps the guard inside the real stack.
pub const ESP_STACK_LIMIT: usize = 64 * 1024;

/// Extension adding an esp-tuned [`Runtime`] constructor.
#[extend::ext(name = RuntimeEspExt)]
pub impl Runtime {
    /// Like [`Runtime::new`], but caps the stack-overflow guard to a budget that
    /// fits the esp main-task stack (see [`ESP_STACK_LIMIT`]). Prefer this over
    /// [`Runtime::new`] on device so deep scripts fail cleanly.
    ///
    /// Additionally sets a default deadline of two seconds.
    fn new_esp() -> rquickjs::Result<Runtime> {
        let runtime = Runtime::new()?;
        runtime.set_max_stack_size(ESP_STACK_LIMIT);
        runtime.set_deadline(Duration::from_secs(2));
        Ok(runtime)
    }

    /// Install an interrupt handler that aborts a synchronous script once it has
    /// run for longer than `budget`. QuickJS calls the handler periodically while
    /// executing; returning `true` raises an `InterruptError`, so a runaway loop
    /// (`while (true) {}`) returns an `Err` instead of wedging the executor.
    ///
    /// The budget starts now, so call this just before the [`eval`] it guards
    /// (or reinstall per eval). This is the hook to feed a watchdog from too, if
    /// one is ever armed — note esp-hal's `init` disables all hardware
    /// watchdogs, so today nothing resets the chip for a long sync script; this
    /// guard is what bounds them. Pass `None` via [`clear_deadline`] to remove it.
    fn set_deadline(&self, budget: core::time::Duration) {
        let start_us = Instant::now().duration_since_epoch().as_micros();
        let budget_us = budget.as_micros() as u64;
        self.set_interrupt_handler(Some(alloc::boxed::Box::new(move || {
            Instant::now()
                .duration_since_epoch()
                .as_micros()
                .saturating_sub(start_us)
                >= budget_us
        })));
    }

    /// Remove any deadline installed by [`set_deadline`].
    fn clear_deadline(&self) {
        self.set_interrupt_handler(None);
    }
}

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
                .or_else(|| value.get::<Coerced<String>>().ok().map(|str| str.0))
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
    Instant::now()
        .duration_since_epoch()
        .as_micros()
        .saturating_mul(1000)
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
