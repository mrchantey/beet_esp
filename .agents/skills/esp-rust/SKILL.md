---
name: esp-rust
description: >-
  Embedded Rust development on Espressif ESP32 SoCs. Use when scaffolding a new
  project, connecting a board, or writing, building, flashing, or debugging Rust
  firmware for ESP32, ESP32-S (Xtensa) or ESP32-C/ESP32-H (RISC-V) chips. Covers
  toolchain setup, project generation (esp-generate), hardware/port setup,
  flashing/monitoring (espflash, probe-rs), esp-hal drivers, async/embassy,
  alloc, logging, OTA, testing, and the esp-idf/std path.
---

# Rust on Espressif (ESP32) SoCs

This skill is split by configuration so you only load what the current project
needs. **Do not read every file.** Detect the configuration first, then read
the matching files.

## Step 1 — Detect the configuration

Two axes decide everything: **architecture** and **environment**.

### Architecture (from the target chip)

| Chip(s)                                  | Architecture |
| ---------------------------------------- | ------------ |
| ESP32, ESP32-S2, ESP32-S3                | **Xtensa**   |
| ESP32-C2, ESP32-C3, ESP32-C6, ESP32-H2   | **RISC-V**   |

Notes: ESP8266 is **not** supported by `esp-hal`. ESP32-C2 = ESP8684,
ESP32-C3 = ESP8685 (pin-compatible drop-in for ESP8266).

Find the chip from: `.cargo/config.toml` (`target`/`build.target`),
`Cargo.toml` features (e.g. `esp-hal = { features = ["esp32c6"] }`), or
`rust-toolchain.toml`.

### Environment (`no_std` vs `std`)

| Signal                                                              | Environment      |
| ------------------------------------------------------------------- | ---------------- |
| `#![no_std]` in `main.rs`; depends on `esp-hal`                     | **no_std**       |
| Depends on `esp-idf-hal` / `esp-idf-svc` / `esp-idf-sys`            | **std (esp-idf)**|
| Target ends in `-none-elf` (e.g. `riscv32imc-unknown-none-elf`)     | **no_std**       |
| Target ends in `-espidf` (e.g. `riscv32imc-esp-espidf`)             | **std (esp-idf)**|

This book and the bulk of this skill cover the **`no_std` / `esp-hal`** path.

## Step 2 — Read the matching files

| Configuration            | Read these files                                                                       |
| ------------------------ | -------------------------------------------------------------------------------------- |
| **no_std + RISC-V**      | `no_std-riscv.md` + `no_std-app-dev.md` + `writing-esp-hal-code.md` + `cli-tools.md`   |
| **no_std + Xtensa**      | `no_std-xtensa.md` + `no_std-app-dev.md` + `writing-esp-hal-code.md` + `cli-tools.md`  |
| **std + esp-idf**        | `std-esp-idf.md` + `cli-tools.md`                                                       |

`writing-esp-hal-code.md` is the firmware-writing reference (program skeleton,
init, drivers); read it when actually editing/authoring `no_std` code.
`cli-tools.md` is the reference for every command-line tool; pull the section
for the tool you actually need.

## Workflows

For end-to-end tasks, read the workflow file first — it tells you what to ask the
user and what to run, and points back to the config/tool files above.

| Task                                            | Read                |
| ----------------------------------------------- | ------------------- |
| **Create a new project** (interview + generate) | `setup-project.md`  |
| **Connect the board** (port choice, flashing)   | `setup-hardware.md` |
| **Build/flash/runtime failing**                 | `troubleshooting.md`|

Typical first-time order: `setup-project.md` → `setup-hardware.md` → `cargo run`.
Both workflows reference the architecture/tool files, so detect the config
(Steps 1–2 above) before starting.

## Universal essentials (true for all configs)

- **Core crate:** `esp-hal` is the bare-metal HAL that ties everything together
  (chip init + peripheral drivers). `esp-idf-hal` is its `std` counterpart.
- **Stability / SemVer:** `esp-hal` 1.0 has a stable core, but most ancillary
  crates and many peripheral drivers are **unstable** and not covered by SemVer.
  Expect `cargo update` to break unstable features. Pin dependencies and read
  migration guides between releases. Per-peripheral stability is documented in
  the `esp-hal` README "Peripheral support" section.
- **Project generation:** `esp-generate` (no_std) or `esp-idf-template` (std)
  produce a working skeleton with the right crates + features wired up. Prefer
  generating over hand-assembling `Cargo.toml`.
- **Each generator version targets a specific ecosystem release.** Update the
  generator before generating if you want the latest crate versions.
- **Authoritative docs:** <https://docs.espressif.com/projects/rust/> (per-chip
  API docs). Pick the docs for the exact chip — peripherals and config differ.
- **Don't `mem::forget` drivers.** Driver `Drop` impls reset the peripheral and
  cancel in-flight DMA; forgetting them leaves peripherals/DMA in a bad state.
