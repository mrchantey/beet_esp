//! ESP32/Bevy bring-up: the embassy/`esp-rtos` starter and the baseline app
//! plugin. The bulk of board bring-up is hidden behind the [`init_esp!`] macro;
//! [`Esp32Plugin`] installs a runner so a bare-metal app is driven by plain
//! [`App::run`].
//!
//! [`init_esp!`]: crate::init_esp

use crate::led::LedColor;
use crate::led::Ws2812;
use beet::prelude::*;
use defmt::info;
use embassy_time::Duration;
use embassy_time::Timer;
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

/// Baseline scaffolding for an esp32 Bevy app: logs a startup banner and
/// installs a runner so [`App::run`] drives the schedule on the embassy
/// executor, pushing the [`LedColor`] to the on-board LED each frame.
///
/// The LED hardware can't be driven from a (synchronous) Bevy system, so the
/// app hands it over as a non-send resource and the runner takes ownership
/// before the loop starts. See [`init_esp!`](crate::init_esp).
pub struct Esp32Plugin;

impl Plugin for Esp32Plugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, || info!("esp32 bevy app started"))
            .set_runner(esp_runner);
    }
}

/// Bevy runner: hands the schedule and the LED hardware to the embassy executor
/// and runs forever, so callers just write `App::new()...run()`.
fn esp_runner(mut app: App) -> AppExit {
    app.finish();
    app.cleanup();
    let led = app
        .world_mut()
        .remove_non_send::<Ws2812>()
        .expect("insert the Ws2812 with `insert_non_send` before `App::run()`");

    static EXECUTOR: StaticCell<Executor> = StaticCell::new();
    let executor = EXECUTOR.init(Executor::new());
    executor.run(move |spawner| {
        let task = render(app, led).expect("render task pool exhausted");
        spawner.spawn(task);
    })
}

/// Tick the schedule, then push the current [`LedColor`] to the LED. The
/// `await` here is why the loop lives in a task rather than a Bevy system —
/// it yields to the executor while the RMT transfer is in flight.
#[embassy_executor::task]
async fn render(mut app: App, mut led: Ws2812) -> ! {
    let mut led_color = app.world_mut().query::<&LedColor>();
    loop {
        app.update();
        if let Ok(color) = led_color.single(app.world()) {
            led.write(color.0).await;
        }
        Timer::after(Duration::from_millis(20)).await;
    }
}
