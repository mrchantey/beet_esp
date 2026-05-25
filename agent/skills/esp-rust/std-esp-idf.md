# std on esp-idf

> **Scope note:** This path is **not covered by The Rust on ESP Book** (which is
> `no_std`/`esp-hal` only). This file is a concise orientation; treat the linked
> repos as authoritative and verify versions/targets before relying on details.
> If the project uses `esp-hal` and `#![no_std]`, you are on the wrong file — see
> `no_std-riscv.md` / `no_std-xtensa.md` instead.

## What this is

The `std` approach runs Rust **on top of the ESP-IDF C framework**. You get a
real Rust `std`: `std::thread`, `std::sync`, `std::net` (TCP/IP via lwIP),
`std::fs` (via ESP-IDF VFS), timers, etc. ESP-IDF is compiled and linked into
your binary at build time.

Use it when you want ESP-IDF's batteries (FreeRTOS, full networking stack, NVS,
mature Wi-Fi/BLE) and `std` ergonomics. Use `no_std`/`esp-hal` when you want a
pure-Rust, leaner, bare-metal stack.

## Core crates

| Crate          | Role                                                                   |
| -------------- | ---------------------------------------------------------------------- |
| `esp-idf-sys`  | Unsafe bindings to ESP-IDF C APIs; its build script builds ESP-IDF.    |
| `esp-idf-hal`  | `embedded-hal` implementations over ESP-IDF (GPIO, I2C, SPI, UART, …). |
| `esp-idf-svc`  | Higher-level services: Wi-Fi, Ethernet, HTTP, MQTT, NVS, SNTP, …       |
| `embedded-svc` | Service trait abstractions implemented by `esp-idf-svc`.               |

## Targets

ESP-IDF uses dedicated `*-espidf` targets (built with `-Z build-std`, so a
nightly-capable or the `esp` toolchain is required):

| Chip(s)              | Target                       |
| -------------------- | ---------------------------- |
| ESP32                | `xtensa-esp32-espidf`        |
| ESP32-S2             | `xtensa-esp32s2-espidf`      |
| ESP32-S3             | `xtensa-esp32s3-espidf`      |
| ESP32-C2, ESP32-C3   | `riscv32imc-esp-espidf`      |
| ESP32-C6, ESP32-H2   | `riscv32imac-esp-espidf`     |

Xtensa chips need the forked `esp` toolchain from **`espup`** (see `cli-tools.md`).
RISC-V chips also work via the `esp` toolchain (or nightly with `build-std`).

## Toolchain & prerequisites

- `espup install` (provides the `esp` toolchain that ships the `*-espidf`
  targets and Xtensa support). See `cli-tools.md`.
- **`ldproxy`**: `cargo binstall -y ldproxy --locked` (the build links through it).
- Host tooling for building ESP-IDF: Git, Python, CMake, Ninja. `esp-idf-sys`
  can download and install a matching ESP-IDF + tools automatically on first
  build (via `embuild`); this first build is slow and network-heavy.
- Relevant env vars (usually set in `.cargo/config.toml` `[env]`): `MCU`
  (e.g. `esp32c6`), `ESP_IDF_VERSION` (e.g. a release tag/branch), and optionally
  `ESP_IDF_TOOLS_INSTALL_DIR`.

## Project setup & build

Generate from the official template (see `cli-tools.md`):

```shell
cargo generate esp-rs/esp-idf-template cargo
```

The template wires up `.cargo/config.toml` (target, `runner = "espflash flash
--monitor"`, `build-std`, and the `[env]` block) plus `sdkconfig.defaults`.

```shell
cargo build
cargo run            # builds, flashes via espflash, opens the monitor
```

Flashing/monitoring uses the same `espflash` / `probe-rs` tools as `no_std`
(see `cli-tools.md`). Bootloader/partition concepts also match (ESP-IDF
bootloader); ESP-IDF config is edited through `sdkconfig` / `idf.py menuconfig`.

## Authoritative sources

- esp-idf-template: <https://github.com/esp-rs/esp-idf-template>
- esp-idf-hal: <https://github.com/esp-rs/esp-idf-hal>
- esp-idf-svc: <https://github.com/esp-rs/esp-idf-svc>
- esp-idf-sys: <https://github.com/esp-rs/esp-idf-sys>
- "std training" for Espressif: <https://docs.esp-rs.org/std-training/>
