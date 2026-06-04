//! Memory health monitoring: heap + stack snapshots and a periodic report.
//!
//! These boards are tiny and we ask a lot of them (a Bevy `World`, serde, a
//! Wi-Fi/BLE stack), so this module measures the two things that actually run
//! out: the **heap** ([`esp_alloc`]) and the **main-task stack**.
//!
//! Three snapshots tell the story:
//! - **boot** — taken in [`on_boot`] (from [`mem::init_esp`](crate::esp32_utils::mem::init_esp), before
//!   `App::new()`), stored in a static. Heap is ~empty here, and this is also
//!   where the stack is *painted* (see below).
//! - **pre-startup** — taken by [`HealthPlugin`] in `PreStartup`, after the chip
//!   and embassy are up but before any app `Startup` system runs. The baseline
//!   we compare against for leak detection.
//! - **periodic** — every [`REPORT_SECS`] in `Update`, with a delta vs the last
//!   report and vs pre-startup so a slow climb (leak) is obvious.
//!
//! ## Stack measurement
//!
//! There is no counter for stack usage, so we use the standard "painting"
//! trick: at boot, fill the unused stack below the current frame with a
//! [`SENTINEL`] word; later, scan up from the bottom and count surviving
//! sentinels. The first overwritten word marks the deepest the stack ever
//! reached — an all-time high-water mark, not just the instantaneous depth.
//!
//! This measures the **CPU0 main-task stack**, which the embassy thread-mode
//! executor and every bridge-spawned driver share. Wi-Fi/BLE worker threads
//! spawned by `esp-radio` get their own heap-allocated stacks, so their cost
//! shows up under the heap, not here.

use beet::prelude::*;
use defmt::info;
use defmt::warn;
use embassy_time::Instant;

/// How often the periodic report fires.
const REPORT_SECS: u64 = 2;

/// Per-report heap growth below this (bytes) is treated as noise, not a climb.
/// Apps step up to a steady-state working set after startup; only *sustained*
/// growth across reports is a leak, so we track a streak rather than an offset.
const HEAP_GROWTH_NOISE: i64 = 256;

/// Warn once the heap has grown for this many consecutive reports — a flat or
/// one-step-then-flat heap (the healthy case) never reaches it.
const LEAK_STREAK: u32 = 3;

/// Warn if the stack ever came within this many bytes of the bottom of its
/// region (i.e. close to the esp-rtos stack guard / overflow).
const STACK_HEADROOM_WARN: usize = 2 * 1024;

// --- stack region, from the esp-hal linker script (`ld/sections/stack.x`) ----
//
// The `.stack` section grows *down* from `_stack_start_cpu0` (high address)
// toward `_stack_end_cpu0` (low address). esp-rtos adopts exactly this region
// as the main task's stack, and places its overflow guard a little above
// `_stack_end_cpu0`, so we keep [`GUARD_GAP`] clear of the bottom.
unsafe extern "C" {
    static _stack_start_cpu0: u32;
    static _stack_end_cpu0: u32;
}

/// Word written across the free stack at boot. Distinctive and unlikely to
/// occur naturally; a stray match only ever *over*-reports usage (safe side).
const SENTINEL: u32 = 0x5A5A_5A5A;

/// Bytes left unpainted/unscanned at the very bottom of the stack region, to
/// stay clear of the esp-rtos stack guard word and its data watchpoint
/// (default guard offset is 60 bytes; this is a generous margin).
const GUARD_GAP: usize = 256;

/// Bytes below the live stack pointer left unpainted at boot, so an interrupt
/// taken mid-paint (which pushes below the current SP) can never land in the
/// region being written.
const PAINT_MARGIN: usize = 1024;

#[inline(always)]
fn stack_lo() -> usize {
    (&raw const _stack_end_cpu0) as usize
}

#[inline(always)]
fn stack_hi() -> usize {
    (&raw const _stack_start_cpu0) as usize
}

/// Fill the currently-unused stack with [`SENTINEL`] so [`stack_usage`] can
/// later recover the high-water mark. Call once, as early as possible (deepest
/// free region) — [`on_boot`] does this from `main`.
///
/// Only the region *below* the live stack pointer is touched, so live frames,
/// return addresses and the esp-rtos guard are never disturbed.
#[inline(never)]
fn paint_stack() {
    let lo = (stack_lo() + GUARD_GAP + 3) & !3;

    // Address of a local == a point inside the current (live) frame; everything
    // above it is in use, so we only ever paint strictly below it.
    let mut probe: u32 = 0;
    let sp = (&raw mut probe) as usize;
    let hi = sp.saturating_sub(PAINT_MARGIN) & !3;

    if hi <= lo {
        return;
    }
    let mut p = lo as *mut u32;
    let end = hi as *mut u32;
    // Volatile so the writes can't be elided as "dead" — the reader lives in a
    // different function behind raw pointers and is invisible to the optimiser.
    unsafe {
        while p < end {
            p.write_volatile(SENTINEL);
            p = p.add(1);
        }
    }
}

/// Returns `(total, peak_used, headroom)` for the main-task stack, in bytes:
/// the region size, the all-time deepest usage, and how close the stack ever
/// came to the bottom (its minimum free margin). Requires [`paint_stack`] to
/// have run.
fn stack_usage() -> (usize, usize, usize) {
    let lo = stack_lo();
    let hi = stack_hi();
    let total = hi.saturating_sub(lo);

    let scan_lo = (lo + GUARD_GAP + 3) & !3;
    let mut p = scan_lo as *const u32;
    let end = hi as *const u32;
    let mut untouched = 0usize;
    // Reads freed stack only (below the current SP), so this is safe; it stops
    // at the first non-sentinel word, which is the all-time high-water line.
    unsafe {
        while p < end {
            if p.read_volatile() != SENTINEL {
                break;
            }
            untouched += 4;
            p = p.add(1);
        }
    }

    let peak_sp = scan_lo + untouched;
    let peak_used = hi.saturating_sub(peak_sp);
    let headroom = peak_sp.saturating_sub(lo);
    (total, peak_used, headroom)
}

/// A point-in-time view of memory. All scalar, so it is `Copy` and lives in a
/// static and in resources without ceremony.
#[derive(Clone, Copy, defmt::Format)]
pub struct MemSnapshot {
    /// Total heap size across all regions, bytes.
    pub heap_size: usize,
    /// Heap currently in use, bytes.
    pub heap_used: usize,
    /// Heap free, bytes.
    pub heap_free: usize,
    /// Largest the heap has ever been (peak `heap_used`), bytes.
    pub heap_max_used: usize,
    /// Lifetime bytes allocated (needs `internal-heap-stats`).
    pub heap_total_alloc: u64,
    /// Lifetime bytes freed (needs `internal-heap-stats`).
    pub heap_total_freed: u64,
    /// Total main-task stack region, bytes.
    pub stack_total: usize,
    /// All-time deepest stack usage, bytes.
    pub stack_peak_used: usize,
    /// Smallest free stack margin ever observed, bytes.
    pub stack_headroom: usize,
}

/// Capture the current heap stats and stack high-water mark.
pub fn snapshot() -> MemSnapshot {
    let s = esp_alloc::HEAP.stats();
    let (stack_total, stack_peak_used, stack_headroom) = stack_usage();
    MemSnapshot {
        heap_size: s.size,
        heap_used: s.current_usage,
        heap_free: s.size.saturating_sub(s.current_usage),
        heap_max_used: s.max_usage,
        heap_total_alloc: s.total_allocated,
        heap_total_freed: s.total_freed,
        stack_total,
        stack_peak_used,
        stack_headroom,
    }
}

/// Boot snapshot, captured in [`on_boot`] before the app builds. Read once by
/// [`HealthPlugin`] in `PreStartup`. Written once, single-threaded, very early;
/// the critical section guards the later cross-task read.
static BOOT: critical_section::Mutex<core::cell::Cell<Option<MemSnapshot>>> =
    critical_section::Mutex::new(core::cell::Cell::new(None));

/// PSRAM bring-up result, recorded by [`mem::init_esp`](crate::esp32_utils::mem::init_esp) via
/// [`set_psram_info`] so the report can show whether the bulk pool came up and
/// how large it is.
static PSRAM: critical_section::Mutex<core::cell::Cell<Option<crate::esp32_utils::mem::PsramInfo>>> =
    critical_section::Mutex::new(core::cell::Cell::new(None));

/// Record the PSRAM bring-up result. Called from
/// [`mem::init_esp`](crate::esp32_utils::mem::init_esp) after it maps PSRAM.
pub fn set_psram_info(info: crate::esp32_utils::mem::PsramInfo) {
    critical_section::with(|cs| PSRAM.borrow(cs).set(Some(info)));
}

/// Read the recorded PSRAM info, if any.
fn psram_info() -> Option<crate::esp32_utils::mem::PsramInfo> {
    critical_section::with(|cs| PSRAM.borrow(cs).get())
}

/// Paint the stack and record the boot snapshot. Called from
/// [`mem::init_esp`](crate::esp32_utils::mem::init_esp) at the top of `main`, before `App::new()`.
pub fn on_boot() {
    paint_stack();
    let snap = snapshot();
    critical_section::with(|cs| BOOT.borrow(cs).set(Some(snap)));
}

/// Read the boot snapshot recorded by [`on_boot`], if any.
fn boot_snapshot() -> Option<MemSnapshot> {
    critical_section::with(|cs| BOOT.borrow(cs).get())
}

/// Snapshots accumulated over the run, for delta reporting.
#[derive(Resource, Default)]
struct HealthState {
    prestartup: Option<MemSnapshot>,
    last: Option<MemSnapshot>,
    reports: u32,
    /// Consecutive reports where the heap grew past [`HEAP_GROWTH_NOISE`].
    growth_streak: u32,
}

/// Logs heap + stack usage at startup and every [`REPORT_SECS`], flagging heap
/// growth (leaks) and low stack headroom (overflow risk).
///
/// Add it after [`Esp32Plugin`](crate::esp32_plugin::Esp32Plugin). The boot
/// snapshot it reports is captured automatically by
/// [`#[beet_esp::main]`](crate::main).
pub struct HealthPlugin;

impl Plugin for HealthPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<HealthState>()
            .add_systems(PreStartup, capture_baseline)
            .add_systems(Update, report);
    }
}

/// Log the boot vs pre-startup deltas and stash the baseline.
fn capture_baseline(mut state: ResMut<HealthState>) {
    let pre = snapshot();
    info!("── health: baseline ──");
    match psram_info() {
        Some(p) if p.present() => info!(
            "psram:       {} B mapped at {=usize:#x} (bulk pool: World, registry, request buffers)",
            p.size, p.start
        ),
        Some(_) => warn!("psram:       not detected — bulk allocations fall back to internal SRAM"),
        None => info!("psram:       (not initialised via #[beet_esp::main])"),
    }
    if let Some(boot) = boot_snapshot() {
        info!(
            "boot:        heap {}/{} B used, stack region {} B",
            boot.heap_used, boot.heap_size, boot.stack_total
        );
    }
    info!(
        "pre-startup: heap {}/{} B used ({} B free), stack peak {} B",
        pre.heap_used, pre.heap_size, pre.heap_free, pre.stack_peak_used
    );
    // Per-region breakdown so internal vs PSRAM occupancy is visible at baseline.
    info!("{}", esp_alloc::HEAP.stats());
    state.prestartup = Some(pre);
    state.last = Some(pre);
}

/// Periodic report, throttled to [`REPORT_SECS`].
fn report(mut state: ResMut<HealthState>, mut last_tick: Local<Option<Instant>>) {
    let now = Instant::now();
    let due = last_tick.is_none_or(|t| (now - t).as_secs() >= REPORT_SECS);
    if !due {
        return;
    }
    *last_tick = Some(now);

    let snap = snapshot();
    state.reports += 1;

    info!(
        "── health #{} (up {} s) ──",
        state.reports,
        now.as_secs()
    );
    info!("{}", esp_alloc::HEAP.stats());
    info!(
        "stack: peak {} / {} B used, min headroom {} B",
        snap.stack_peak_used, snap.stack_total, snap.stack_headroom
    );

    if let Some(last) = state.last {
        let d_last = snap.heap_used as i64 - last.heap_used as i64;
        let d_pre = state
            .prestartup
            .map_or(0, |pre| snap.heap_used as i64 - pre.heap_used as i64);
        info!("Δ heap: {} B since last, {} B since pre-startup", d_last, d_pre);

        // A leak shows as the heap climbing report after report; a one-time step
        // up to steady state (then flat) resets the streak and stays quiet.
        if d_last > HEAP_GROWTH_NOISE {
            state.growth_streak += 1;
        } else {
            state.growth_streak = 0;
        }
        if state.growth_streak >= LEAK_STREAK {
            warn!(
                "heap climbing {} reports straight (now {} B used) — likely leak",
                state.growth_streak, snap.heap_used
            );
        }
    }
    if snap.stack_headroom < STACK_HEADROOM_WARN {
        warn!(
            "stack headroom down to {} B — close to the guard, raise the stack",
            snap.stack_headroom
        );
    }

    state.last = Some(snap);
}
