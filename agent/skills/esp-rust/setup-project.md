# Workflow: Set Up a New Project

Goal: produce a working ESP Rust project skeleton with the correct crates and
config. The generators are interactive by default, but **both support a
non-interactive mode** — so the right flow is: **gather the config from the user,
then run the generator headless** with the exact options.

Do **not** guess the chip or options. Ask. A wrong chip/option set produces a
project that won't build or flash.

## Step 0 — Prerequisites

- Toolchain for the target architecture is installed (`no_std-riscv.md`,
  `no_std-xtensa.md`, or `std-esp-idf.md`).
- The generator is installed (`cli-tools.md`):
  - no_std: `cargo binstall -y esp-generate --locked`
  - std:    `cargo binstall -y cargo-generate --locked`

## Step 1 — Interview the user

Ask these, in order. Stop and resolve architecture/environment first since they
gate everything else.

1. **Target chip / dev board?** Determines architecture (see SKILL.md table).
   If they only know the board name, identify the chip from it. esp-generate
   supports: `esp32`, `esp32c2`, `esp32c3`, `esp32c5`, `esp32c6`, `esp32c61`,
   `esp32h2`, `esp32s2`, `esp32s3` (C-series and C5/C61 are RISC-V; S-series and
   plain ESP32 are Xtensa). Run `esp-generate list-options` for the live list.
2. **Environment: `no_std` (esp-hal) or `std` (esp-idf)?** Default and book-path
   is `no_std`. Only choose `std` if they need ESP-IDF/std (`std-esp-idf.md`).
3. **Flashing/debugging tool: `espflash` (default) or `probe-rs`?**
   - `probe-rs` enables RTT/`defmt` and on-chip debugging + `embedded-test`, but
     needs a JTAG connection. On `esp32c5/c6/c61/h2/s3` it works over the chip's
     native USB; on `esp32/s2/c2/c3` it needs an **external probe (ESP-Prog)** —
     a plain USB-UART adapter will **not** work. Confirm their hardware can do it
     (see `setup-hardware.md`). If unsure, default to `espflash`.
4. **Logging frontend?** Tie to the flashing tool:
   - `espflash` → `log` (via esp-println). `probe-rs` → `defmt`.
   (Only one log frontend may be selected.)
5. **Panic handler?** `espflash` → `esp-backtrace`. `probe-rs` → `panic-rtt-target`.
   (Only one panic handler.)
6. **Heap?** `alloc` (esp-alloc) — needed for `Vec`/`String`/`Box`/boxed futures.
7. **Async?** `embassy` (requires `unstable-hal`).
8. **Connectivity?** `wifi` and/or `ble-trouble` (BLE). **Each requires `alloc`
   + `unstable-hal` + `embassy`** — enable those automatically if chosen.
   Wi-Fi/BLE chip compatibility varies; the generator rejects invalid combos.
9. **On-hardware tests?** `embedded-test` (requires `probe-rs`).
10. **Extras?** `ci` (GitHub Actions), `wokwi` (simulator), and a coding-agent
    guidance file — **recommend `claude`**, which drops a project `CLAUDE.md`
    with chip-specific notes and doc links.
11. **Project name** (kebab-case).

Use `esp-generate explain <option>` if the user asks what an option does.

## Step 2 — Resolve option dependencies

Build a valid option set before running. Rules (esp-generate enforces these,
but pre-resolve to avoid surprises):

| Option            | Requires                              | Mutually exclusive with    |
| ----------------- | ------------------------------------- | -------------------------- |
| `embassy`         | `unstable-hal`                        | —                          |
| `wifi`            | `alloc`, `unstable-hal`, `embassy`    | —                          |
| `ble-trouble`     | `alloc`, `unstable-hal`, `embassy`    | —                          |
| `log`             | espflash path (no `probe-rs`)         | `defmt` (one log frontend) |
| `defmt`           | —                                     | `log`                      |
| `esp-backtrace`   | espflash path (no `probe-rs`)         | `panic-rtt-target`         |
| `panic-rtt-target`| `probe-rs`                            | `esp-backtrace`            |
| `embedded-test`   | `probe-rs`                            | —                          |
| `stack-smashing-protection` | nightly Rust                | —                          |

One `chip`, one log frontend, one panic handler, one agent-guidance file.

## Step 3 — Confirm, then generate

Show the user the exact command and confirm before running.

### no_std (esp-generate, headless)

```shell
esp-generate --headless \
  -o esp32c6 \
  -o unstable-hal -o embassy -o alloc -o wifi \
  -o log -o esp-backtrace \
  -o claude \
  my-project
```

(The chip is just another `-o` value. Add/remove `-o` flags per the interview.)

### std (cargo-generate, non-interactive)

```shell
cargo generate esp-rs/esp-idf-template cargo \
  --name my-project \
  -d mcu=esp32c6
# add -d <key>=<value> for each template prompt; --silent fails if any are unset
```

If you can't determine every template variable up front, run it interactively
and let the user answer prompts. See `std-esp-idf.md`.

## Step 4 — Verify

1. On save, `esp-generate` checks for required/optional tools and reports what's
   missing — install anything it flags (`cli-tools.md`).
2. Build it: `cd my-project && cargo build` (use `cargo +esp build` on Xtensa if
   no `rust-toolchain.toml` override was written — esp-generate writes one).
3. Flashing/monitoring happens via `cargo run --release` once hardware is
   connected — but **do `setup-hardware.md` first** so the right port is used.
4. If a `CLAUDE.md`/`AGENTS.md` was generated, read it — it documents the chip,
   the enabled options, and the per-crate "Additional configuration" doc links
   for this specific project.

If the build or first flash fails, see `troubleshooting.md`. To start writing
firmware, see `writing-esp-hal-code.md`.

## Notes

- `espup install` (Xtensa/std toolchain) is itself non-interactive — just run it.
- Each `esp-generate` version pins a specific ecosystem release. Update it first
  (`cargo binstall -y esp-generate --locked`) if the user wants the newest crates.
- Generation options are baked into source as `generator parameters:` comments
  in `src/bin/main.rs`; check them before editing option-dependent code.
