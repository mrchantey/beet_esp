# beet_esp

Embedded firmware for the [beet](https://github.com/mrchantey/beet) project,
targeting the **ESP32-S3** (Xtensa).

`no_std` Rust on `esp-hal`, with async via embassy and connectivity via
`esp-radio` (Wi-Fi + BLE).

## Quickstart

```sh
. $HOME/export-esp.sh && cargo run --example blinky
```


## Stack

- **HAL:** `esp-hal` 1.1 (`no_std`, bare metal)
- **Async runtime:** embassy (`esp-rtos`)
- **Connectivity:** `esp-radio` — Wi-Fi (`embassy-net`/`smoltcp`) and BLE (`trouble-host`), with COEX
- **Heap:** `esp-alloc`
- **Logging:** `defmt` over RTT
- **Panic handler:** `panic-rtt-target`
- **Flash/debug:** `probe-rs` (ESP32-S3 native USB JTAG — no external probe needed)

## Prerequisites

- Espressif Rust (Xtensa) toolchain via [`espup`](https://github.com/esp-rs/espup):
  ```shell
  cargo binstall espup && espup install
  ```
  Then source the env in each shell (sets `LIBCLANG_PATH` etc.):
  ```shell
  . $HOME/export-esp.sh
  ```
- [`probe-rs`](https://probe.rs) for flashing, monitoring and on-chip debug.

The pinned toolchain is recorded in `rust-toolchain.toml`.

## Build, flash, test

```shell
cargo build --release          # compile
cargo run   --release          # flash + monitor (probe-rs runner)
cargo test                     # on-hardware tests (embedded-test)
```

The target (`xtensa-esp32s3-none-elf`), runner and `build-std` are configured in
`.cargo/config.toml`.



### ESP32-s3

# ESP32-S3-WROOM-1 Variants

| Variant | Flash | PSRAM | Temp Range | Notes |
|---|---|---|---|---|
| N4 | 4 MB (Quad SPI) | — | –40 ~ 85 °C | Entry-level, no PSRAM |
| N8 | 8 MB (Quad SPI) | — | –40 ~ 85 °C | |
| N16 | 16 MB (Quad SPI) | — | –40 ~ 85 °C | |
| H4 | 4 MB (Quad SPI) | — | –40 ~ **105 °C** | High-temp industrial variant |
| N4R2 | 4 MB (Quad SPI) | 2 MB (Quad SPI) | –40 ~ 85 °C | |
| N8R2 | 8 MB (Quad SPI) | 2 MB (Quad SPI) | –40 ~ 85 °C | |
| N16R2 | 16 MB (Quad SPI) | 2 MB (Quad SPI) | –40 ~ 85 °C | |
| N4R8 | 4 MB (Quad SPI) | 8 MB (Octal SPI) | –40 ~ 65 °C | Octal PSRAM; note narrower temp range |
| N8R8 | 8 MB (Quad SPI) | 8 MB (Octal SPI) | –40 ~ 65 °C | |
| N16R8 | 16 MB (Quad SPI) | 8 MB (Octal SPI) | –40 ~ 65 °C | Used by the **Arduino Nano ESP32** (via u-blox NORA-W106) and therefore the **Arduino Alvik** |