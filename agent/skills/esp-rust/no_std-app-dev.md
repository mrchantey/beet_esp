# no_std Application Development (esp-hal)

Shared topics for `no_std` firmware on **both** RISC-V and Xtensa. Read this
alongside your architecture file (`no_std-riscv.md` or `no_std-xtensa.md`). For
the program skeleton and how to actually write the code, see
`writing-esp-hal-code.md`; when something won't build/flash/run, see
`troubleshooting.md`.

## The esp-hal ecosystem (ancillary crates)

`esp-hal` is the core: chip init + peripheral drivers. Common companions:

| Crate                    | Purpose                                                              | Stability |
| ------------------------ | ------------------------------------------------------------------- | --------- |
| `esp-hal`                | Bare-metal (`no_std`) HAL for all ESP32 devices                     | Stable\*  |
| `esp-alloc`              | `no_std` heap allocator                                             | Unstable  |
| `esp-backtrace`          | Backtraces / panic handler                                          | Unstable  |
| `esp-println`            | Print + `log` logging output                                        | Unstable  |
| `esp-bootloader-esp-idf` | ESP-IDF 2nd-stage bootloader support, incl. OTA                     | Unstable  |
| `esp-config`             | Build-time configuration system (env / `esp_config.yml`)           | Unstable  |
| `esp-radio`              | Wi-Fi, BLE, IEEE 802.15.4, ESP-NOW                                  | Unstable  |
| `esp-rtos`               | Scheduler/RTOS backend; embassy integration; `esp-radio` runtime    | Unstable  |
| `esp-storage`            | Flash storage utilities                                             | Unstable  |
| `esp-hal-smartled`       | Driver for the addressable WS2812B RGB LED on many DevKits          | Unstable  |
| `xtensa-lx` / `-lx-rt`   | Xtensa low-level access + runtime (Xtensa only)                     | Unstable  |
| `esp-riscv-rt`           | RISC-V startup/runtime (RISC-V only)                                | Unstable  |

\* Core is stable; many drivers within `esp-hal` are individually unstable.

**Radio note:** `esp-radio` needs a continuously running background runtime
(timers, interrupts, state machines). It depends on an `esp-radio-rtos-driver`
implementation; **`esp-rtos` is the default/supported backend**. `esp-generate`
adds `esp-rtos` automatically when you enable the relevant template options.

**embedded ecosystem:** `esp-hal` implements `embedded-hal` traits (HAL-agnostic
drivers), `embedded-io` (`no_std` analogue of `std::io`), and `rand_core` for the
RNG peripheral.

## Startup & bootloader

Boot sequence on Espressif chips:

1. **First-stage (ROM) bootloader** — burned into ROM, immutable. Sets up
   arch registers, reads boot mode + reset reason, loads the 2nd-stage loader.
2. **Second-stage bootloader** — sets up RAM/PSRAM/flash, loads your app.

Only the **ESP-IDF bootloader** is supported as 2nd-stage today (MCUboot is
planned). It uses the ESP image format + a **partition table** to know where to
place binaries. Entries have label, type (`app`/`data`/…), subtype, and offset.

- `espflash`/`cargo-espflash` ship **prebuilt** ESP-IDF bootloaders and a default
  partition table — you usually don't supply your own.
- For a **custom bootloader**: install ESP-IDF, configure via `idf.py menuconfig`
  / `sdkconfig`, build with `idf.py set-target <CHIP> build bootloader`, then pass
  `--bootloader build/bootloader/bootloader.bin` to `espflash`/`cargo-espflash`
  (or set it in the espflash config file). A custom partition table is similar.

## Configuration (esp-config)

`esp-config` exposes build-time options for `esp-*` crates that don't fit Cargo
features. Find options in each crate's docs (e.g. `esp-hal`'s "Additional
configuration" section, per chip).

Set them two ways:

- **Env var** named exactly like the option, e.g.
  `ESP_HAL_CONFIG_PLACE_ANON_IN_RAM=true`.
- **`.cargo/config.toml` `[env]`** (also sets the env var):

  ```toml
  # .cargo/config.toml
  [env]
  ESP_HAL_CONFIG_PLACE_ANON_IN_RAM = "true"
  ```

  After editing `[env]`, do a **clean build**. CLI env vars override `[env]`.

**Multi-target projects:** keep a baseline `.cargo/config.toml` (always read),
add a per-config file under `.cargo/`, and add a Cargo alias to select it, e.g.
`run-config-a = "run --config=./.cargo/config_a.toml --release"`.

Define your own options declaratively in an `esp_config.yml`. `esp-config` also
ships an optional TUI (`esp-config` command) — see `cli-tools.md`.

## Logging

Two frameworks; `esp-generate` wires whichever you pick:

- **`defmt`** — compact binary-encoded logs, low overhead. **Best paired with
  `probe-rs`.** Recommended panic handler: `panic-rtt-target`.
- **`log`** — the standard string-logging facade (`info!`, `warn!`, …). Use the
  logger in **`esp-println`** (or implement your own). **Pairs with `espflash`.**
  Recommended panic handler: `esp-backtrace`.

Rule of thumb: `probe-rs` → `defmt`; `espflash` → `log` + `esp-println`.

## Alloc / heap

`no_std` has no heap by default. Add one with **`esp-alloc`** to enable `Vec`,
`Box`, etc. Understand the trade-offs first:

- **Fragmentation** and **runtime overhead** are real costs; prefer static
  allocation where practical.
- You may have **exactly one global allocator**, but it can span multiple regions
  (internal RAM, PSRAM, multiple blocks). For multiple allocators use the nightly
  `allocator_api` plus `allocator-api2` (which `esp-alloc` implements).

```rust
// Reclaim ~64 kB the 2nd-stage bootloader used (unusable as stack, fine as heap)
heap_allocator!(#[ram(reclaimed)] size: 64000);
```

**PSRAM** extends RAM via external memory (some chips). ⚠️ On **Xtensa**, atomics
in PSRAM are broken — never allocate `Atomic*` there. RISC-V is fine. (Details in
your architecture file.)

## Async

`esp-hal` drivers are **`Blocking` by default**; convert with `.into_async()`.

⚠️ `Async` drivers are **not `Send`** — they register interrupts on the current
core. To use one on another core, move the **`Blocking`** version, then call
`.into_async()` on the target core.

Frameworks:

- **Embassy** — the common embedded async executor (statically-allocated tasks;
  keep a `Spawner` to spawn more later). Integrated via **`esp-rtos`**, which
  provides the interrupt-mode executor, a multicore-aware thread-mode executor,
  the embassy time driver, and the timer waiter queue.
- **ArielOS** — a Rust IoT OS built on `esp-hal`; adds multicore scheduler,
  networking, drivers; integrates with embassy.
- **RTIC** — real-time concurrency framework. ESP support is **ESP32-C3 and
  ESP32-C6 only**.

## Testing

- **Host testing first.** Test as much as possible on the host (faster, CI-able,
  saves flash write cycles). Only put hardware-dependent logic in HIL tests.
- **Hardware-in-Loop (HIL)** uses the **`embedded-test`** framework (tests are
  `#[test]` fns, mimics the std harness; supports `#[should_panic]`, timeouts,
  IDE integration). Flash/run via **`probe-rs`** over the **`USB-JTAG-SERIAL`**
  port (chips without it need ESP-Prog or similar).
- Generate it: pick `embedded-test` under the `probe-rs` options in
  `esp-generate`, then run `cargo test` with the device connected.

## OTA (over-the-air updates)

Update firmware without physical flashing tools. OTA relies on the bootloader to
switch / replace / roll back images. For the supported ESP-IDF bootloader, use
the **`esp-bootloader-esp-idf`** crate. See the OTA example in the `esp-hal` repo
(`examples/ota`) — it also shows how to build an OTA binary with `espflash`.

## FAQ / gotchas

- **Don't `mem::forget` drivers** — their `Drop` resets the peripheral and
  cancels in-flight DMA. Forgetting leaves peripherals/DMA misconfigured.
- **Binary size:** build with `--release`; tune `[profile]` settings; prune
  dependencies; filter out unread log levels. See `min-sized-rust` and the
  Embedded Rust Book's speed-vs-size chapter.
- **Git dependencies:** use Cargo's git deps and `[patch]` to track crate `main`
  branches (the ecosystem moves fast).
- **Download mode:** the chip enters Download (firmware-receive) vs SPI Boot (run
  app) mode based on strapping-pin levels sampled at reset. Serial shows
  "waiting for download" in Download mode. Return to app: reset the board, or use
  `espflash --after watchdog-reset` (USB-Serial/JTAG mode).
- **Editors:** `esp-generate` can emit recommended settings/extensions for VS
  Code, Helix, Neovim, and Zed during generation.
