# beet_esp

ESP32-S3 embedded firmware for the [beet](https://github.com/mrchantey/beet)
project. `no_std` Rust on `esp-hal`.

Always begin a conversation with 'gday pete'.

**Never create git commits.** Only Pete commits. The agent may stage or edit
files, but committing is Pete's job alone.

## Configuration

Set at generation time — don't change without a reason.

- **Chip:** ESP32-S3 (Xtensa). Target: `xtensa-esp32s3-none-elf`.
- **Environment:** `no_std`, `esp-hal` 1.1.
- **Async:** embassy via `esp-rtos`.
- **Connectivity:** `esp-radio` — Wi-Fi + BLE (`trouble-host`), COEX enabled.
- **Heap:** `esp-alloc` (two heaps; the second adds RAM for Wi-Fi/BLE COEX).
- **Logging:** `defmt` over RTT. **Panic handler:** `panic-rtt-target`.
- **Flash/debug:** `probe-rs` (S3 native USB JTAG — no external probe needed).
- **Tests:** `embedded-test` (runs on hardware).

The exact generator options are recorded in a `generator parameters:` comment at
the top of `src/bin/main.rs`.

## Environment setup (required before building)

The Xtensa toolchain comes from `espup`, not stock rustup. Every shell needs the
env vars sourced:

```shell
. $HOME/export-esp.sh   # sets LIBCLANG_PATH etc.
```

The toolchain is pinned in `rust-toolchain.toml` (the `esp` channel). Build
failures with linker or libclang errors are almost always a missing
`export-esp.sh`.

## Commands

```shell
cargo build --release    # compile
cargo run   --release    # flash + monitor (probe-rs runner)
cargo test               # on-hardware tests (embedded-test)
```

Target, runner and `build-std` are configured in `.cargo/config.toml`.

**`cargo run` does not exit.** The probe-rs runner flashes and then attaches an
RTT/`defmt` monitor that streams output indefinitely — it never returns on its
own. When running non-interactively (e.g. from an agent), always wrap it in a
timeout so it detaches after capturing output, e.g.:

```shell
timeout -s INT 30s cargo run --release   # flash, stream ~30s, then detach
```

## Gotchas

- **Don't `mem::forget` esp-hal drivers** — their `Drop` resets the peripheral
  and cancels in-flight DMA; forgetting leaves hardware in a bad state.
- `esp-hal`'s core is 1.0-stable, but **most ancillary crates and many peripheral
  drivers are unstable and not covered by SemVer**. Pin dependencies; `cargo
  update` can break unstable features. Read migration guides between releases.
- Per-chip API docs: <https://docs.espressif.com/projects/rust/> — pick ESP32-S3.

## Hardware

Dual-port ESP32-S3 DevKit, brought up and verified 2026-05. Day-to-day:

- **Use the `USB` port** (native USB-Serial-JTAG `303a:1001`, what `probe-rs`
  drives), not `COM` (a CH340 UART bridge — serial only, no JTAG).
- **Keep `COM` unplugged while using `probe-rs`** — its auto-reset lines can tug
  `GPIO0`/`EN` into download mode.
- udev rules are installed and `probe-rs list` shows `ESP JTAG -- 303a:1001`.

If a flash succeeds but no `defmt` ever appears (probe-rs scans for RTT
forever), the chip is stuck in download mode — see the "sticky download mode"
entry in `agent/skills/esp-rust/troubleshooting.md`.

## Deeper reference

A full `esp-rust` skill (project generation, hardware setup, esp-hal coding
patterns, troubleshooting) lives in-repo at `agent/skills/esp-rust/`.
