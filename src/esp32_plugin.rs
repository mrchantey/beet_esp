//! ESP32/Bevy bring-up: the embassy/`esp-rtos` starter and the baseline app
//! plugin. [`Esp32Plugin`] initialises the chip and embassy in a `PreStartup`
//! system, exposes the raw peripherals as non-send resources, and installs a
//! runner so a bare-metal app is driven by plain [`App::run`].

use crate::esp32_utils::async_bridge::spawn_driver;
use beet::prelude::*;
use defmt::info;
use embassy_time::Duration;
use embassy_time::Timer;
use esp_hal::peripherals::Peripherals;
use esp_hal::timer::timg::TimerGroup;
use esp_rtos::embassy::Executor;
use static_cell::StaticCell;

/// Hand-off slot for the chip peripherals.
///
/// `esp_hal::init()` may only run once, and it has to run in `main` (inside
/// [`mem::init_esp`](crate::esp32_utils::mem::init_esp), via
/// [`#[beet_esp::main]`](crate::main)) so PSRAM is mapped and registered
/// *before* `App::new()` builds the `World` and its type registry. The resulting
/// [`Peripherals`] are parked here for [`bring_up`] (a `PreStartup` system) to
/// collect, instead of `bring_up` calling `esp_hal::init()` a second time.
static PERIPHERALS: critical_section::Mutex<core::cell::Cell<Option<Peripherals>>> =
    critical_section::Mutex::new(core::cell::Cell::new(None));

/// Park the chip peripherals for [`bring_up`] to collect. Called once from
/// [`mem::init_esp`](crate::esp32_utils::mem::init_esp) after `esp_hal::init()`.
pub fn stash_peripherals(p: Peripherals) {
    critical_section::with(|cs| PERIPHERALS.borrow(cs).set(Some(p)));
}

/// Collect the peripherals parked by [`stash_peripherals`]. Panics if they were
/// never stashed (i.e. `#[beet_esp::main]` was not used).
fn take_peripherals() -> Peripherals {
    critical_section::with(|cs| PERIPHERALS.borrow(cs).take())
        .expect("peripherals not initialised — use #[beet_esp::main] before App::run")
}

/// Start the embassy executor on `esp-rtos` using timer group 0. Call once,
/// after peripheral init, before spawning tasks or running an [`App`].
pub fn start_embassy(
    timg0: esp_hal::peripherals::TIMG0<'static>,
    sw: esp_hal::peripherals::SW_INTERRUPT<'static>,
) {
    let timg0 = TimerGroup::new(timg0);
    let sw_interrupt = esp_hal::interrupt::software::SoftwareInterruptControl::new(sw);
    esp_rtos::start(timg0.timer0, sw_interrupt.software_interrupt0);
}

/// Baseline runtime for an esp32 Bevy app: brings up the chip and embassy,
/// logs a startup banner, and installs a runner so [`App::run`] drives the
/// schedule on the embassy executor.
///
/// It is LED-agnostic — it only ticks the schedule and exposes the raw
/// peripherals as non-send resources. Each domain plugin (e.g.
/// [`LedPlugin`](crate::utils::led::LedPlugin)) assembles its own drivers from those
/// peripherals and spawns its own async driver via the [`async_bridge`](crate::esp32_utils::async_bridge)
/// using the [`Spawner`](embassy_executor::Spawner) resource. See
/// [`#[beet_esp::main]`](crate::main).
pub struct Esp32Plugin;

impl Plugin for Esp32Plugin {
    fn build(&self, app: &mut App) {
        // Back bevy's `Instant` with the esp-hal `SYSTIMER` *before* adding
        // `TimePlugin` below: that plugin samples `Instant::now()` while building
        // `Time`, and on this no_std target the clock has no backend until we
        // install one. Add `Esp32Plugin` first so nothing samples time earlier.
        install_bevy_clock();
        app.add_systems(PreStartup, bring_up)
            .add_systems(Startup, || info!("esp32 bevy app started"))
            // Bevy's `Time`, ticked each frame off `Instant::now()` (backed by
            // `install_bevy_clock` above). Gives systems a `Res<Time>` instead of
            // a hand-rolled `Instant` compare.
            .add_plugins(beet::exports::bevy::time::TimePlugin)
            .set_runner(esp_runner);
        // beet's async bridge (the `action` feature) needs a task spawner; on
        // no_std there's no default. Install the bevy-task-pool-backed one —
        // `insert_resource` overrides AsyncPlugin's panicking default regardless
        // of plugin order. See `async_utils` for why this pool and not embassy.
        #[cfg(feature = "action")]
        crate::esp32_utils::async_utils::install_async_spawner(app);
    }
}

/// Initialise the chip, start the embassy runtime, and expose the peripherals
/// domain plugins need as non-send resources.
///
/// Exclusive so it can both call the hardware fns and `insert_non_send`. It runs
/// once in `PreStartup`, before any plugin's `Startup` split system claims a
/// peripheral. Calling `esp_hal::init`/[`start_embassy`] from a system (rather
/// than `main`) is the proven "run-then-start" order: the runner calls
/// `Executor::run` first, and this system runs inside the spawned tick task
/// before any embassy timer is awaited.
fn bring_up(world: &mut World) {
    // `esp_hal::init()` already ran in `mem::init_esp` (so PSRAM could be mapped
    // before the `World` was built); collect the peripherals it parked.
    let p = take_peripherals();
    start_embassy(p.TIMG0, p.SW_INTERRUPT);
    // Each esp-hal peripheral is a distinct type, so they key cleanly as
    // separate resources. A domain plugin removes whichever ones it owns; only
    // expose the ones a compiled-in domain plugin can actually claim.
    #[cfg(feature = "led")]
    {
        world.insert_non_send(p.RMT);
        world.insert_non_send(p.GPIO48);
    }
    #[cfg(feature = "wifi")]
    world.insert_non_send(p.WIFI);
    // UART1 + the STM32 control/check GPIOs for the Alvik driver, mirroring the
    // led/wifi blocks above (see `alvik::pinout`).
    #[cfg(feature = "alvik")]
    {
        world.insert_non_send(p.UART1);
        world.insert_non_send(p.GPIO43);
        world.insert_non_send(p.GPIO44);
        world.insert_non_send(p.GPIO6);
        world.insert_non_send(p.GPIO5);
        world.insert_non_send(p.GPIO7);
        world.insert_non_send(p.GPIO13);
    }

    // Without a domain plugin there are no peripherals to expose; `world` is then
    // only here to keep `bring_up` an exclusive system that runs in `PreStartup`.
    #[cfg(not(any(feature = "led", feature = "wifi", feature = "alvik")))]
    let _ = world;
}

/// Point bevy's monotonic [`Instant`] at the esp-hal `SYSTIMER`.
///
/// On this `no_std` target `bevy_platform`'s `Instant::now()` has no backend and
/// panics (it only auto-detects x86/aarch64 cycle counters). beet code paths
/// that time themselves — notably bevy's [`TimePlugin`](beet::exports::bevy::time::TimePlugin)
/// (it samples `Instant::now()` while initialising `Time` during `add_plugins`),
/// the router's `RequestLogger`, and the action layer — call it, so install an
/// elapsed-time getter backed by a monotonic clock.
///
/// Backed by [`esp_hal::time`] (the `SYSTIMER`), which is live the moment
/// `esp_hal::init` returns (in [`mem::init_esp`](crate::esp32_utils::mem::init_esp), before
/// `App::new()`) — *not* embassy's clock, which only starts at [`start_embassy`].
/// Called at the top of [`Esp32Plugin::build`], before it adds `TimePlugin`.
fn install_bevy_clock() {
    // `Instant` here is bevy_platform's, brought in via `use beet::prelude::*`
    // (beet_core re-exports `bevy::prelude::*`). Its `set_elapsed` hook lets us
    // back bevy's monotonic clock with the hardware timer on this no_std target.
    /// Monotonic microseconds since boot, from the esp-hal `SYSTIMER`.
    fn elapsed() -> core::time::Duration {
        core::time::Duration::from_micros(
            esp_hal::time::Instant::now().duration_since_epoch().as_micros(),
        )
    }
    // SAFETY: `elapsed` is a plain fn pointer with no shared mutable state; the
    // getter is installed once, before any `Instant::now()` can run.
    unsafe { Instant::set_elapsed(elapsed) };
}

/// How long the schedule sleeps between ticks.
const FRAME: Duration = Duration::from_millis(20);

/// Bevy runner: hands the schedule to the embassy executor and runs forever, so
/// callers just write `App::new()...run()`. The embassy
/// [`Spawner`](embassy_executor::Spawner) is exposed as a non-send resource so
/// domain plugins can spawn their own async drivers.
fn esp_runner(mut app: App) -> AppExit {
    app.finish();
    app.cleanup();

    static EXECUTOR: StaticCell<Executor> = StaticCell::new();
    let executor = EXECUTOR.init(Executor::new());
    executor.run(move |spawner| {
        app.insert_non_send(spawner);
        // Generic tick: advance the schedule, then yield a frame.
        spawn_driver(spawner, async move {
            loop {
                app.update();
                Timer::after(FRAME).await;
            }
        });
    })
}
