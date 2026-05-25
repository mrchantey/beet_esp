# beet_esp

Embedded firmware for the [beet](https://github.com/mrchantey/beet) project,
targeting the **ESP32-S3** (Xtensa).

`no_std` Rust on `esp-hal`, with async via embassy and connectivity via
`esp-radio` (Wi-Fi + BLE).

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

## Notes

- Scaffolded with [`esp-generate`](https://github.com/esp-rs/esp-generate); the
  exact options are recorded as a comment at the top of `src/bin/main.rs`.
- CI runs basic checks via GitHub Actions (`.github/workflows/rust_ci.yml`).
