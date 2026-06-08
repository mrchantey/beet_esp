//! Memory placement: where allocations land, and the seam for steering them.
//!
//! ## The model
//!
//! This board has two very different pools of RAM, and they are *not*
//! interchangeable:
//!
//! - **Internal SRAM** — 512 KiB, fast, and the only memory the Wi-Fi/BLE radio
//!   can DMA into. Scarce: the radio, task stacks, `.data`/`.bss` and the stack
//!   all live here. This is the **hot / DMA** pool.
//! - **PSRAM** — external SPI RAM (multiple MiB on this module), reached through
//!   the cache. Slower, *cannot* be a DMA target, but there is megabytes of it.
//!   This is the **cold / bulk** pool.
//!
//! The whole point of this module is to put the big *cold* structures (the Bevy
//! `World`, the reflection / type registry, scene loading, per-request buffers)
//! in PSRAM and keep the scarce internal SRAM for the radio and DMA.
//!
//! ## How the split works (capability tiering)
//!
//! Rust has a single global allocator and Bevy's collections use it, so we can't
//! redirect "just Bevy". Instead we lean on [`esp_alloc`]'s region capabilities:
//!
//! - Every heap region is tagged [`Internal`](esp_alloc::MemoryCapability::Internal)
//!   or [`External`](esp_alloc::MemoryCapability::External).
//! - A request carrying *no* capability (the plain global allocator — Bevy, beet,
//!   `alloc::*`) is served from the **first region that fits**, in the order the
//!   regions were registered.
//! - A request carrying the `Internal` capability is served **only** from
//!   internal regions.
//!
//! [`init_mem`] registers PSRAM **first**, so every capability-agnostic
//! allocation lands in PSRAM by default — no per-allocation ceremony. esp-radio
//! asks for its DMA buffers through `malloc_internal` / `wifi_malloc`, which
//! carry the `Internal` capability, so they are pinned to internal SRAM and the
//! radio keeps working. (esp-radio's *non*-DMA bookkeeping uses the plain,
//! capability-free `malloc`, which is free to land in PSRAM — that's fine and
//! actually relieves internal pressure.)
//!
//! ## Putting something in fast internal RAM on purpose
//!
//! PSRAM is the no-ceremony default. When you specifically need fast / DMA /
//! latency-sensitive memory, name it: allocate with [`Internal`], the
//! intent-revealing allocator handle re-exported here. It is an
//! `allocator_api2::Allocator`, so it works with the `*_in` constructors:
//!
//! ```ignore
//! use beet_esp::prelude::*;
//! use allocator_api2::vec::Vec;
//!
//! // Cold + big: just allocate. Lands in PSRAM.
//! let registry = Vec::<TypeInfo>::new();
//!
//! // Hot / DMA: say so. Pinned to internal SRAM.
//! let dma_buf = Vec::<u8, _>::with_capacity_in(1536, Internal);
//! ```
//!
//! For the symmetric case (force something into PSRAM even if you'd otherwise
//! reach for a typed pool) use [`External`].

pub use esp_alloc::ExternalMemory as External;
pub use esp_alloc::InternalMemory as Internal;
pub use esp_alloc::MemoryCapability;

use core::mem::MaybeUninit;
use esp_alloc::HeapRegion;
use esp_hal::peripherals::Peripherals;
use esp_hal::psram::Psram;
use esp_hal::psram::PsramConfig;

/// A binary-local internal-SRAM reserve buffer of `N` bytes, donated to the
/// internal heap by [`init_esp`].
///
/// A heap region must be a `'static` linker static, so this can't be allocated
/// by a library fn — the entry macro declares one as a `static mut` in the
/// binary. The size is a const generic so the macro only forwards a number; all
/// the `MaybeUninit` ceremony lives here instead of expanded in every binary.
pub struct Reserve<const N: usize>(MaybeUninit<[u8; N]>);

impl<const N: usize> Reserve<N> {
    /// An uninitialised reserve. `const` so it can initialise a `static`.
    pub const fn uninit() -> Self {
        Self(MaybeUninit::uninit())
    }

    /// `(ptr, len)` over the whole buffer, for registering as a heap region.
    /// The pointer's `'static` validity comes from the caller's static storage.
    fn region(&mut self) -> (*mut u8, usize) {
        (self.0.as_mut_ptr() as *mut u8, N)
    }
}

/// The bootloader-reclaimed internal SRAM region (linker section `dram2_seg`),
/// otherwise unused. Fixed-size and config-independent, so it lives here rather
/// than in the macro; [`init_esp`] registers it as an [`Internal`] heap region so
/// radio DMA can use it.
#[esp_hal::ram(reclaimed)]
static mut RECLAIMED: MaybeUninit<[u8; crate::RECLAIMED_INTERNAL_BYTES]> = MaybeUninit::uninit();

/// Outcome of bringing PSRAM up, reported once at boot so the placement model is
/// observable rather than assumed.
#[derive(Debug, Clone, Copy)]
pub struct PsramInfo {
    /// Base virtual address of the mapped PSRAM window.
    pub start: usize,
    /// Mapped, usable PSRAM size in bytes (0 if no PSRAM was detected).
    pub size: usize,
}

impl PsramInfo {
    /// Whether usable PSRAM was detected and mapped.
    pub fn present(&self) -> bool {
        self.size > 0
    }
}

/// Bring PSRAM up, register it as the **default** (first) heap region, then add
/// the internal SRAM regions the caller donated.
///
/// Called once by [`init_esp`], before `App::new()` so the Bevy `World` and the
/// reflection registry it builds during `add_plugins` land in PSRAM.
///
/// `reclaimed` is the bootloader-reclaimed internal region ([`RECLAIMED`]) and
/// `reserve` the freshly-carved internal reserve ([`Reserve`]), each as
/// `(ptr, len)`. Both are tagged [`Internal`] so radio DMA can use them.
///
/// Region order is deliberate: **PSRAM first** (so capability-free allocations
/// default there), then the two internal regions (so `Internal`-capability radio
/// allocations have somewhere to go). esp-alloc allows at most three regions;
/// PSRAM + reserve + reclaimed is exactly three.
///
/// # Safety
///
/// `reserve` and `reclaimed` must each be a `'static`, exclusively-owned,
/// non-overlapping byte region with non-zero length. The caller ([`init_esp`]) must
/// not register those regions itself.
unsafe fn init_mem(
    reserve: (*mut u8, usize),
    reclaimed: (*mut u8, usize),
) -> (Peripherals, PsramInfo) {
    let p = esp_hal::init(esp_hal::Config::default().with_cpu_clock(esp_hal::clock::CpuClock::max()));

    // Auto-detects octal vs quad and the size; warns (does not panic) if no chip
    // is present, leaving `raw_parts` empty so we simply skip the region.
    //
    // `Psram::new` wants `PSRAM<'static>`, which we can't move out of the
    // `#[non_exhaustive]` `Peripherals` while keeping the rest. `Psram` only
    // touches the peripheral to map the window once and then holds it inert, so
    // steal a second `'static` handle for it; the original in `p` is never used
    // again (PSRAM is not exposed as a peripheral resource).
    let psram_peri = unsafe { esp_hal::peripherals::PSRAM::steal() };
    let psram = Psram::new(psram_peri, PsramConfig::default());
    let (psram_start, psram_size) = psram.raw_parts();
    let info = PsramInfo {
        start: psram_start as usize,
        size: psram_size,
    };

    unsafe {
        // PSRAM FIRST → it becomes the home for every capability-agnostic
        // allocation (the bulk: World, type registry, request buffers).
        if info.present() {
            esp_alloc::HEAP.add_region(HeapRegion::new(
                psram_start,
                psram_size,
                MemoryCapability::External.into(),
            ));
        }
        // Internal regions AFTER → they only ever serve `Internal`-capability
        // requests (radio DMA) plus any overflow once PSRAM is full.
        esp_alloc::HEAP.add_region(HeapRegion::new(
            reserve.0,
            reserve.1,
            MemoryCapability::Internal.into(),
        ));
        esp_alloc::HEAP.add_region(HeapRegion::new(
            reclaimed.0,
            reclaimed.1,
            MemoryCapability::Internal.into(),
        ));
    }

    (p, info)
}

/// Full boot-time bring-up, and the only thing
/// [`#[beet_esp::main]`](crate::main) has to call: start RTT `log` output,
/// map + register the heaps, park the peripherals, record the PSRAM result, and
/// snapshot/paint the stack.
///
/// The macro's sole remaining job is to declare the [`Reserve`] linker static
/// (a heap region must be `'static`, so it must be binary-local — a generic fn
/// can't size a `static` from its own const param) and hand it here; all the
/// *logic* lives in this module rather than expanded in every binary. Order
/// matters: logging first, then [`init_mem`] (PSRAM-first, also the
/// [`RECLAIMED`] internal region), then stash the peripherals for
/// [`Esp32Plugin`](crate::esp32_plugin)'s `PreStartup` system, record the PSRAM
/// info for [`HealthPlugin`](crate::esp32_utils::health), and finally paint the stack +
/// capture the boot snapshot before anything allocates.
///
/// Call once, from the entry macro, at the top of `main` before `App::new()`.
///
/// # Safety
///
/// `reserve` must point to a `'static`, exclusively-owned reserve registered
/// nowhere else — exactly the `static mut Reserve<N>` the entry macro declares
/// and passes by `&raw mut`. Must be called once.
pub unsafe fn init_esp<const N: usize>(reserve: *mut Reserve<N>) {
    // Start the RTT `log` backend before anything logs. Declares the
    // `_SEGGER_RTT` control block and installs the global logger; guarded against
    // a double call, so it's safe here. `beet`'s `tracing` macros reach it via
    // tracing's `log` fallback (no subscriber on bare metal).
    crate::rtt_target::rtt_init_log!(log::LevelFilter::Info);

    // The static is `'static` and only touched here, so the exclusive borrow is
    // sound for the duration of registration.
    let reserve = unsafe { &mut *reserve };
    let reclaimed = (
        (&raw mut RECLAIMED) as *mut u8,
        crate::RECLAIMED_INTERNAL_BYTES,
    );
    let (peripherals, psram) = unsafe { init_mem(reserve.region(), reclaimed) };
    crate::esp32_plugin::stash_peripherals(peripherals);
    crate::esp32_utils::health::set_psram_info(psram);
    // Paint the stack and record the boot memory snapshot, before anything
    // allocates. Picked up by `HealthPlugin` in `PreStartup`.
    crate::esp32_utils::health::on_boot();
}
