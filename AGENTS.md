# beet_esp


ESP32-S3 embedded firmware for the [beet](https://github.com/mrchantey/beet)
project. `no_std` Rust on `esp-hal`.

Always begin a conversation with 'gday pete'.

## Context

This is a downstream library from our primary project called beet. We're working off a work tree so that we can make changes freely. You have permission to make changes to the worktree as required, but do not commit changes so the user can review them.

beet worktree: `/home/pete/me/worktrees/beet/apps/beet`

Always pull in this file to context first:
`/home/pete/me/worktrees/beet/apps/beet/AGENTS.md`

## Configuration

Set at generation time — don't change without a reason.

- **Chip:** ESP32-S3 (Xtensa). Target: `xtensa-esp32s3-none-elf`.
- **Environment:** `no_std`, `esp-hal` 1.1.
- **Async:** embassy via `esp-rtos`.
- **Connectivity:** `esp-radio` — Wi-Fi + BLE (`trouble-host`), COEX enabled.
- **Heap:** `esp-alloc` (two heaps; the second adds RAM for Wi-Fi/BLE COEX).
- **Logging:** `log`/`tracing` over RTT. **Panic handler:** the crate's own RTT
  handler (`beet_esp` lib `#[panic_handler]`, which the on-device test build
  swaps for a semihosting-exit handler so a failed test reports rather than hangs).
- **Flash/debug:** `probe-rs` (S3 native USB JTAG, no external probe needed).
- **Tests:** beet's own on-hardware harness (`beet_core::testing` via the
  `testing_embedded` feature, registered with `linkme`), run with
  `cargo test -p beet_esp --lib`. See `src/device_test.rs`.

The exact generator options are recorded in a `generator parameters:` comment at
the top of `src/main.rs`.

## First-time machine bootstrap

A fresh machine has none of the toolchain. Verified from a clean state
2026-07-02. Every step is user-level (`~/.cargo`, `~/.rustup`) except the udev
rule, which is the one and only `sudo` step.

```shell
# 1. Xtensa Rust toolchain + ~/export-esp.sh (large download, ~1-2 GB)
cargo binstall -y espup && espup install

# 2. Flash/debug tooling (probe-rs, cargo-flash, cargo-embed)
cargo binstall -y probe-rs-tools

# 3. sccache. The global ~/.cargo/config.toml sets `rustc-wrapper = "sccache"`,
#    so builds die with "could not execute process `sccache`" without it.
cargo binstall -y sccache

# 4. udev rule so probe-rs can open the USB-JTAG as a normal user (THE sudo step).
#    Without it probe-rs errors "failed to open device (errno 13)". Note `probe-rs
#    list` still works read-only, so a missing rule looks fine but is not:
#    flashing needs write access.
#
#    The upstream probe-rs rule file matches with ATTRS{} (device-or-parent) and
#    did NOT fire for the ESP32-S3 on this machine's udev (verified with
#    `udevadm test`: the file is read but no rule matches, MODE stays 0664, no
#    uaccess tag). Match the device's OWN attrs (ATTR) instead. GROUP="wheel"
#    grants access directly (pete is in wheel) so it does not depend on
#    logind/seat; uaccess is a bonus for the active desktop session.
echo 'SUBSYSTEM=="usb", ATTR{idVendor}=="303a", ATTR{idProduct}=="1001", MODE="0660", GROUP="wheel", TAG+="uaccess"' \
  | sudo tee /etc/udev/rules.d/70-esp32s3-jtag.rules
sudo udevadm control --reload-rules
sudo udevadm trigger --action=add --attr-match=idVendor=303a
# GROUP="wheel" is applied by udev directly, so no physical replug is needed for
# access. (uaccess only re-applies on a real device add/replug.)
```

Confirm access with `probe-rs info --chip esp32s3`: the node becomes
`group=wheel crw-rw----` and it attaches (`Xtensa Chip IDCODE ...`).

```shell
# 5. Wi-Fi credentials. The firmware reads BEET_WIFI_SSID / BEET_WIFI_PASSWORD
#    via env! at compile time (build.rs exposes them from .env), so the build
#    fails without them. Copy the template and fill in your network.
cp .env.example .env   # then edit BEET_WIFI_SSID / BEET_WIFI_PASSWORD
```

Note the build honours a global `CARGO_TARGET_DIR` if set (this machine points it
at `~/.cargo_target`), so the firmware ELF lands there, not in the project-local
`target/`.

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
RTT/`defmt` monitor that streams output indefinitely, it never returns on its
own. When running non-interactively (e.g. from an agent), always wrap it in a
timeout so it detaches after capturing output.

**Size the timeout to clear the flash first.** The timeout also kills the
erase/program/verify that runs *before* any RTT appears, so too short a window
aborts mid-program and leaves the chip in an unknown state (a silent app that
looks like dead hardware). The full `alvik` release build (~2.5 MiB) takes
~100-110s to flash before it streams, so 30s is only safe for re-attaching to an
already-flashed chip. Give a fresh flash a generous window:

```shell
timeout -s INT 30s  cargo run --release                     # re-attach only (no reflash)
timeout -s INT 240s cargo run --release --features alvik     # fresh flash: ~2 min to program, then streams
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
- Once the udev rule from "First-time machine bootstrap" is installed, `probe-rs
  list` shows `ESP JTAG -- 303a:1001` and can flash. Note enumeration works
  read-only even without the rule, so `probe-rs list` succeeding does not by
  itself prove flashing will work.
- **On-board addressable LED (WS2812) is on `GPIO48`**, driven over RMT. See
  `examples/blinky.rs` (RGB hue fade) and `examples/led_scan.rs` (the GPIO
  scanner that found it).
- **Two+ boards at once:** each appears in `probe-rs list` with its own serial;
  the `.cargo/config.toml` runner sets no `--probe`, so target one explicitly,
  e.g. `probe-rs run --probe 303a:1001:<SERIAL> --chip esp32s3 … <elf>`.
- **Board in `lsusb` as `303a:4001` but missing from `probe-rs list`:** its
  firmware grabbed the USB-OTG port as a CDC serial (`/dev/ttyACM*`), hiding the
  native JTAG. Enter download mode (hold `BOOT`, tap `RST`) to restore
  `303a:1001`, then flash and cold-boot.
- **alvik**: If there's an Arduino Alvik plugged in, it's safe to assume that it is in an upright postion and wheels are not touching the ground so you should freely test motors etc as needed.

If a flash succeeds but no `defmt` ever appears (probe-rs scans for RTT
forever), the chip is stuck in download mode: **the app isn't running, so don't
mistake it for a dead peripheral** (a silent app looks exactly like a broken
LED/sensor). Cold-boot (unplug ~10s, leave `COM` out, replug `USB`) before you
start debugging hardware. See the "sticky download mode" entry in trouibleshooting.

## Deeper reference

A full `esp-rust` skill (project generation, hardware setup, esp-hal coding
patterns, troubleshooting) lives in-repo at `.agents/skills/esp-rust/`.

All troubleshooting is located at `.agents/skills/esp-rust/troubleshooting.md`



## Verification


After making code changes, we need to verify on device if it's plugged in. Ensure everything is compiling, re-upload to the device which is plugged in, and verify all good
