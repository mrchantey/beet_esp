# Writing esp-hal Firmware (no_std)

The program skeleton and idioms for `no_std` esp-hal apps, for **both** RISC-V
and Xtensa. This is the shape `esp-generate` emits (esp-hal 1.x).

> Exact peripheral driver signatures (GPIO/I2C/SPI/UART/…) change across esp-hal
> releases. Use this for the program *structure*; confirm peripheral APIs against
> the per-chip docs (<https://docs.espressif.com/projects/rust/>) and the
> version-matched examples at
> `https://github.com/esp-rs/esp-hal/tree/esp-hal-v<VERSION>/examples`.

## Anatomy of every esp-hal program

1. `#![no_std]` and `#![no_main]` at the crate root.
2. A **panic handler** — usually pulled in by importing a crate for its side
   effect: `use esp_backtrace as _;` (espflash path) or
   `use panic_rtt_target as _;` (probe-rs path). With `defmt`/`log` you may
   instead define one that logs `panic_info` then loops.
3. **App descriptor** (required by the ESP-IDF bootloader — don't omit it):
   `esp_bootloader_esp_idf::esp_app_desc!();`
4. An **entry macro** on a `-> !` (never-returning) `main`.
5. **Chip init**: build a `Config`, call `esp_hal::init(config)` → `Peripherals`.
6. (If logging) initialize the logger. (If `alloc`) initialize the heap.
7. Construct drivers from peripheral singletons, then your loop.

The generated crate also denies `clippy::mem_forget` — a lint backing the rule
that you must not `mem::forget` esp-hal drivers (their `Drop` resets the
peripheral and cancels DMA).

## Blocking skeleton

```rust
#![no_std]
#![no_main]

use esp_hal::{
    clock::CpuClock,
    main,
    time::{Duration, Instant},
};
use esp_backtrace as _;          // panic handler (espflash path)
use log::info;                   // with the `log` frontend

esp_bootloader_esp_idf::esp_app_desc!();

#[main]
fn main() -> ! {
    esp_println::logger::init_logger_from_env();   // `log` frontend init

    let config = esp_hal::Config::default().with_cpu_clock(CpuClock::max());
    let peripherals = esp_hal::init(config);

    // construct drivers from `peripherals.*` here

    loop {
        info!("Hello world!");
        let start = Instant::now();
        while start.elapsed() < Duration::from_millis(500) {}
    }
}
```

## Async skeleton (embassy via esp-rtos)

Embassy support comes through **`esp-rtos`** — the entry macro is
**`#[esp_rtos::main]`** (not `esp_hal_embassy::main`), and `main` takes a
`Spawner`. You must **start the scheduler** with a timer + software interrupt
before awaiting.

```rust
#![no_std]
#![no_main]

use esp_hal::clock::CpuClock;
use esp_hal::timer::timg::TimerGroup;
use embassy_executor::Spawner;
use embassy_time::{Duration, Timer};
use esp_backtrace as _;

esp_bootloader_esp_idf::esp_app_desc!();

#[esp_rtos::main]
async fn main(spawner: Spawner) -> ! {
    let config = esp_hal::Config::default().with_cpu_clock(CpuClock::max());
    let peripherals = esp_hal::init(config);

    let timg0 = TimerGroup::new(peripherals.TIMG0);
    let sw_int = esp_hal::interrupt::software::SoftwareInterruptControl::new(
        peripherals.SW_INTERRUPT,
    );
    esp_rtos::start(timg0.timer0, sw_int.software_interrupt0);   // start scheduler

    // spawner.spawn(my_task()).ok();
    let _ = spawner;

    loop {
        Timer::after(Duration::from_secs(1)).await;
    }
}
```

Tasks are `#[embassy_executor::task]` async fns spawned via the `Spawner`. Keep
a copy of the `Spawner` (e.g. pass it into tasks) to spawn more later.

## Acquiring peripherals & drivers

- `esp_hal::init` returns a `Peripherals` struct; each peripheral is a **move-only
  singleton** field (`peripherals.GPIO8`, `peripherals.I2C0`, `peripherals.TIMG0`,
  `peripherals.WIFI`, …). A driver consumes the singleton, so you can't construct
  two drivers for the same peripheral.
- Representative GPIO (verify against your version's docs):

  ```rust
  use esp_hal::gpio::{Level, Output, OutputConfig};
  let mut led = Output::new(peripherals.GPIO8, Level::Low, OutputConfig::default());
  led.set_high();
  ```

- Drivers are **`Blocking` by default**; get an async one with `.into_async()`.
  Async drivers are **not `Send`** (they register interrupts on the current
  core) — see `no_std-app-dev.md` "Async" for the cross-core rule.

## Logger / heap init lines (match your generation options)

| Option              | Init line in `main`                          |
| ------------------- | -------------------------------------------- |
| `log` (espflash)    | `esp_println::logger::init_logger_from_env();` |
| `defmt` + probe-rs  | `rtt_target::rtt_init_defmt!();`             |
| probe-rs, no defmt  | `rtt_target::rtt_init_print!();`             |
| `alloc`             | `esp_alloc::heap_allocator!(#[esp_hal::ram(reclaimed)] size: 65536);` |

Log level is controlled at runtime by env vars (`ESP_LOG` for `log`,
`DEFMT_LOG` for `defmt`), defaulted in `.cargo/config.toml`.

## Wi-Fi / BLE entry points (need `alloc`+`embassy`+esp-rtos)

After `esp_rtos::start(...)`:

```rust
// Wi-Fi
let (mut _controller, _interfaces) =
    esp_radio::wifi::new(peripherals.WIFI, Default::default())
        .expect("Wi-Fi init failed");
```

BLE uses `trouble-host` over an `esp_radio::ble` connector. These APIs are
unstable — start from the matching `esp-hal`/`trouble` examples.

## Where to look next

- Per-peripheral API + per-chip config: <https://docs.espressif.com/projects/rust/>
- Working examples (pin to your esp-hal version tag): `esp-rs/esp-hal/examples`
- App-level topics (alloc, async rules, logging choice, OTA, testing): `no_std-app-dev.md`
- A generated project's `CLAUDE.md`/`AGENTS.md` lists the exact doc links for its
  enabled crates.
