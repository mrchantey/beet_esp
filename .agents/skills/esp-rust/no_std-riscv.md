# no_std on RISC-V (ESP32-C / ESP32-H)

For bare-metal `esp-hal` firmware on RISC-V chips. **Read `no_std-app-dev.md`**
for application topics (logging, alloc, async, OTA, testing, config) — they are
shared with Xtensa and not repeated here.

## Chips and targets

RISC-V chips use **upstream Rust** — no compiler fork required. Targets are
**Tier 2**.

| Chip(s)              | Rust target                       |
| -------------------- | --------------------------------- |
| ESP32-C2, ESP32-C3   | `riscv32imc-unknown-none-elf`     |
| ESP32-C6, ESP32-H2   | `riscv32imac-unknown-none-elf`    |

The `imc` vs `imac` difference reflects the RISC-V extensions each chip supports
(the `a` = atomics extension). Use the right one or builds/atomics will fail.

## Toolchain setup

You do **not** need `espup` and you do **not** need to know anything about the
Xtensa fork or ESP-IDF.

```shell
# stable or nightly both work; rust-src is required
rustup toolchain install stable --component rust-src

# add the target for your chip
rustup target add riscv32imc-unknown-none-elf   # C2, C3
rustup target add riscv32imac-unknown-none-elf  # C6, H2
```

`rustfmt`, `clippy`, and `rust-analyzer` are optional but recommended.

That's the whole toolchain story for RISC-V. Move on to `cli-tools.md` for the
generation/flashing tools (`esp-generate`, `espflash`, `probe-rs`).

## RISC-V specifics worth knowing

- **PSRAM atomics work correctly** on RISC-V (unlike Xtensa). The allocator may
  hold `Atomic*` types in PSRAM without the data-race caveat that affects Xtensa.
- **`probe-rs` / on-chip debugging without external hardware** is available on
  chips with the `USB-JTAG-SERIAL` peripheral: **ESP32-C6, ESP32-H2, and
  ESP32-C3 (revision 0.3+)**. Otherwise use an external probe (ESP-Prog).
- **RTIC** (Real-Time Interrupt-driven Concurrency) currently supports only
  **ESP32-C3 and ESP32-C6** among Espressif chips.

## Typical workflow

```shell
esp-generate --chip esp32c6 my-app   # or run interactively: esp-generate
cd my-app
cargo run --release                  # compiles, flashes, monitors
```

See `cli-tools.md` for `esp-generate` options and `espflash`/`probe-rs` details.
