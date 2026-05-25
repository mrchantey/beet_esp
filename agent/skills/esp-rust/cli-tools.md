# CLI Tools Reference

Every command-line tool used across ESP Rust workflows. The **Used by** column
tells you whether a tool is relevant to your configuration — skip the rest.

| Tool             | Used by                                  |
| ---------------- | ---------------------------------------- |
| `rustup`         | all                                      |
| `espup`          | Xtensa (no_std) + all std/esp-idf        |
| `cargo-binstall` | all (optional, faster installs)          |
| `esp-generate`   | no_std project generation                |
| `cargo-generate` | std/esp-idf project generation           |
| `espflash`       | all (flashing/monitoring)                |
| `cargo-espflash` | all (cargo-subcommand variant)           |
| `probe-rs`       | flashing + on-chip debug + HIL testing   |
| `esp-config`     | optional config TUI (no_std esp-* crates)|
| `ldproxy`        | std/esp-idf builds                       |
| `idf.py`         | std/esp-idf + custom no_std bootloaders  |

Most installs are `cargo binstall -y <tool> --locked`; all of these also publish
release binaries installable via `cargo-binstall`.

---

## rustup — Rust toolchain manager (all)

Install Rust from <https://rustup.rs>. On Unix, **do not** install Rust from a
system package manager (`brew`/`apt`/`dnf`) — use rustup to avoid incompatibilities.
On Windows install an ABI (MSVC recommended; GNU for MinGW/MSYS2 interop).

RISC-V no_std needs `rust-src` + the chip target:

```shell
rustup toolchain install stable --component rust-src
rustup target add riscv32imc-unknown-none-elf    # ESP32-C2/C3
rustup target add riscv32imac-unknown-none-elf   # ESP32-C6/H2
```

Xtensa uses the forked `esp` toolchain from `espup` instead (see below).

---

## espup — Espressif toolchain installer (Xtensa + std)

Installs and maintains the forked Rust/LLVM/GCC toolchains needed for **Xtensa**
(and convenient for std/esp-idf across both arches). Not needed for RISC-V no_std.

```shell
cargo binstall -y espup --locked
espup install        # install all Espressif toolchains
espup update         # keep them current
espup uninstall      # remove them
```

Installs: Espressif Rust fork (the `esp` toolchain), a stable toolchain with
RISC-V support, the LLVM fork (Xtensa backend), and a GCC linker toolchain.

On **Unix**, source the env file it writes before building (path is printed;
commonly `~/export-esp.sh`):

```shell
. $HOME/export-esp.sh
```

Windows needs no extra env step.

---

## cargo-binstall — prebuilt binary installer (optional, all)

Installs released binaries instead of compiling from source:

```shell
cargo binstall espflash esp-generate espup ldproxy
```

---

## esp-generate — no_std project generator

Creates a working `no_std` esp-hal project with crates + Cargo features wired up
for the options you pick. Each `esp-generate` version targets a specific
ecosystem release — update it for the newest crates.

```shell
cargo binstall -y esp-generate --locked
esp-generate                       # interactive: prompts chip + name, then TUI
esp-generate --chip esp32c6 my-app # non-interactive-ish
```

In the TUI, toggle options (each has a description at the bottom); press **`s`**
at the root to generate. On save it checks for required/optional tools and tells
you what to install. See the README "Available Options" for the full list.

Key option groups:

- **Flashing tool:** default `espflash`, or enable
  *"Use probe-rs to flash and monitor instead of espflash"* for RTT + on-chip debug.
- With **espflash**, also consider *"Use the log crate to print messages"* +
  *"Use esp-backtrace as the panic handler."*
- With **probe-rs**, consider *"Use defmt to print messages"* +
  *"Use panic-rtt-target as the panic handler."*
- Can emit editor config for VS Code, Helix, Neovim, Zed.

Run the generated project: `cargo run --release` (compiles → flashes → monitors).

---

## cargo-generate + esp-idf-template — std project generator

For the **std / esp-idf** path (see `std-esp-idf.md`):

```shell
cargo binstall -y cargo-generate --locked
cargo generate esp-rs/esp-idf-template cargo   # binary crate
# (esp-idf-template also has a `cmake` flavor for ESP-IDF/CMake integration)
```

---

## espflash — serial flasher + monitor (all)

Native support for every `esp-hal`-compatible chip. Ships prebuilt ESP-IDF
bootloaders + default partition table, so a bare project flashes without extras.

```shell
cargo binstall -y espflash --locked
```

Common commands:

```shell
espflash flash <ELF>           # flash an ELF (auto-detect chip/port)
espflash monitor               # open serial monitor
espflash board-info            # chip, revision, features, MAC
espflash erase-flash           # wipe flash
espflash save-image ...        # build a flashable/OTA image file
espflash flash --partition-table partitions.csv --bootloader bootloader.bin <ELF>
espflash flash --after watchdog-reset <ELF>   # leave download mode (USB-Serial/JTAG)
```

- **Default baudrate is 115200** — increase it for faster flashing, easiest via
  the `ESPFLASH_BAUD` env var.
- It's the default `cargo run` runner in `esp-generate` espflash projects, so
  `cargo run --release` flashes + monitors.

---

## cargo-espflash — espflash as a cargo subcommand (all)

Same capabilities as `espflash`, integrated with cargo builds:

```shell
cargo binstall -y cargo-espflash --locked
cargo espflash flash --release --monitor
```

---

## probe-rs — flashing, on-chip debug, RTT, HIL tests

A toolset for debug-probe interaction with broad Espressif support. Beyond
flash/monitor it gives real debugging and RTT (used by `defmt`).

- **No external hardware needed** on chips with the `USB-JTAG-SERIAL` peripheral:
  **ESP32-C6, ESP32-H2, ESP32-S3, ESP32-C3 (rev 0.3+)**. Others need an external
  probe such as **ESP-Prog**.
- Install + probe setup: <https://probe.rs/docs/getting-started/installation/>.
- In `esp-generate`, enabling the probe-rs option sets it as the `cargo run`
  runner (RTT-based) and is required for `embedded-test` HIL testing.
- Used to flash/run `embedded-test` tests (`cargo test`).

---

## esp-config — config TUI (optional, no_std esp-* crates)

Edit `esp-config` build-time options via a TUI instead of hand-editing
`.cargo/config.toml`. Entirely optional.

```shell
# Must compile from source: the TUI is a Cargo feature, and `cargo binstall`
# fetches prebuilt binaries and cannot enable `--features`.
cargo install esp-config --features=tui --locked
esp-config        # run from the project directory
```

Can also take an explicit chip + config file if it can't infer them; not needed
for projects generated by `esp-generate`. See `no_std-app-dev.md` "Configuration"
for the underlying env-var / `[env]` mechanism.

---

## ldproxy — linker proxy (std/esp-idf)

A small linker wrapper the esp-idf build (`embuild`/`esp-idf-sys`) uses to pass
linker args. Required to build std projects:

```shell
cargo binstall -y ldproxy --locked
```

---

## idf.py — ESP-IDF build tool (std + custom bootloaders)

Part of a full **ESP-IDF** (C framework) install. Needed for std/esp-idf
development and for building a **custom no_std 2nd-stage bootloader**:

```shell
idf.py set-target <CHIP> build bootloader   # -> build/bootloader/bootloader.bin
idf.py menuconfig                           # edit sdkconfig options
```

Then feed `build/bootloader/bootloader.bin` to `espflash --bootloader`.
