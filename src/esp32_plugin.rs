//! ESP32/Bevy bring-up: the embassy/`esp-rtos` starter and the baseline app
//! plugin. [`Esp32Plugin`] initialises the chip and embassy in a `PreStartup`
//! system, exposes the raw peripherals as non-send resources, and installs a
//! runner so a bare-metal app is driven by plain [`App::run`].

use crate::bridge::spawn_driver;
use beet::prelude::*;
use defmt::info;
use embassy_time::Duration;
use embassy_time::Timer;
use esp_hal::clock::CpuClock;
use esp_hal::timer::timg::TimerGroup;
use esp_rtos::embassy::Executor;
use static_cell::StaticCell;

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
/// [`LedPlugin`](crate::led::LedPlugin)) assembles its own drivers from those
/// peripherals and spawns its own async driver via the [`bridge`](crate::bridge)
/// using the [`Spawner`](embassy_executor::Spawner) resource. See
/// [`init_esp!`](crate::init_esp).
pub struct Esp32Plugin;

impl Plugin for Esp32Plugin {
    fn build(&self, app: &mut App) {
        app.add_systems(PreStartup, bring_up)
            .add_systems(Startup, || info!("esp32 bevy app started"))
            .set_runner(esp_runner);
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
    let p = esp_hal::init(esp_hal::Config::default().with_cpu_clock(CpuClock::max()));
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

    // Without a domain plugin there are no peripherals to expose; `world` is then
    // only here to keep `bring_up` an exclusive system that runs in `PreStartup`.
    #[cfg(not(any(feature = "led", feature = "wifi")))]
    let _ = world;
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
