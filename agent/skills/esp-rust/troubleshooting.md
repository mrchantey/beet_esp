# Troubleshooting

Symptom → likely cause → fix. Grouped by build, flash/run, and runtime. For
deeper hardware/port issues see `setup-hardware.md`.

## Build errors

| Symptom                                                        | Cause / fix                                                                                                  |
| -------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------ |
| Linker error / `xtensa-esp32-elf-gcc not found` (Xtensa)       | The `espup` environment isn't on PATH. `. $HOME/export-esp.sh` (Unix), or re-run `espup install`. RISC-V doesn't need this. |
| `can't find crate for core` / target errors (RISC-V)           | Target not added: `rustup target add riscv32imc-unknown-none-elf` (C2/C3) or `riscv32imac-unknown-none-elf` (C6/H2). |
| Wrong compiler used on Xtensa / unknown `esp` channel          | Missing toolchain override. Ensure `rust-toolchain.toml` pins `channel = "esp"`, or build with `cargo +esp ...`. |
| `error: rust-src component is required`                        | `rustup component add rust-src` for the active toolchain.                                                    |
| Config change in `.cargo/config.toml` `[env]` not taking effect| `[env]`/`esp-config` options are build-time. Do a **clean build** (`cargo clean`) after editing them.        |
| `--features` ignored when installing a tool via `cargo binstall`| `binstall` fetches prebuilt binaries and can't enable features. Use `cargo install <tool> --features=... --locked` (e.g. esp-config TUI). |
| Wi-Fi/BLE/embassy options fail to compile                      | Dependency rules: `wifi`/`ble-trouble` need `alloc`+`unstable-hal`+`embassy`; `embassy` needs `unstable-hal`. Regenerate or add the deps. |

## Flash / run errors (`cargo run`, espflash, probe-rs)

| Symptom                                                        | Cause / fix                                                                                                  |
| -------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------ |
| `Permission denied` on `/dev/ttyUSB*` / `/dev/ttyACM*` (Linux) | Add user to the serial group (`dialout`, or `uucp` on Arch) and re-login. For probe-rs, install its udev rules. |
| No serial port found / flashes the wrong board                 | Multiple devices: pass `--port <PORT>`. Confirm the cable is a data cable and the board is powered.          |
| `espflash` can't detect chip / wrong chip                      | Run `espflash board-info`. Ensure the `--chip` in `.cargo/config.toml` matches the actual silicon.           |
| Chip stuck at `waiting for download` / new firmware won't run  | Still in Download mode. Reset the board, or flash with `--after watchdog-reset` (USB-Serial/JTAG). Enter download manually: hold **Boot**, tap **Reset**, release Boot. |
| `probe-rs list` shows nothing                                  | Wrong port or unsupported setup. probe-rs needs native USB-Serial-JTAG (C3 rev0.3+, C5/C6/C61/H2/S3) or an external ESP-Prog — **not** a USB-UART adapter. Check udev rules. See `setup-hardware.md`. |
| Flashing is very slow                                          | Default baud is 115200. Set `ESPFLASH_BAUD` higher.                                                          |
| probe-rs `--chip` / target mismatch                            | The `--chip` in `.cargo/config.toml` must match the connected chip.                                          |

## Runtime / behavior

| Symptom                                                        | Cause / fix                                                                                                  |
| -------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------ |
| No log output at all                                           | Missing logger init or panic handler. Ensure `esp_println::logger::init_logger_from_env()` / `rtt_init_defmt!()` runs first, and a panic handler crate is imported. |
| Logs appear but levels are filtered out                        | Adjust the runtime filter: `ESP_LOG=info` (`log`) or `DEFMT_LOG=info` (`defmt`); defaults live in `.cargo/config.toml`. |
| Heap allocation failures / OOM                                 | Increase `esp_alloc::heap_allocator!` size, or enable `alloc` if you're using `Vec`/`String`/`Box` without it. Consider PSRAM. |
| Async app does nothing / never wakes                           | Scheduler not started. Call `esp_rtos::start(timer, sw_interrupt)` before awaiting (see `writing-esp-hal-code.md`). |
| Wi-Fi/BLE init fails or radio dead                             | `esp-radio` needs the `esp-rtos` runtime running; init radio after `esp_rtos::start(...)`.                   |
| Random data races / corruption on Xtensa with PSRAM            | Atomics in PSRAM are broken on Xtensa — never allocate `Atomic*` into PSRAM. (RISC-V is fine.)               |
| Peripheral misbehaves / DMA never completes after a refactor   | A driver was dropped or `mem::forget`-ten unexpectedly. Don't forget drivers; keep them alive for the transfer's duration. |
| `Async` driver breaks when moved across cores                  | Async drivers aren't `Send`. Move the `Blocking` version, then `.into_async()` on the target core.           |

## Quick diagnostics

```shell
espflash board-info        # chip, revision, features, MAC over serial
probe-rs list              # is a debug probe/JTAG visible?
rustup show                # active toolchain + installed targets
cargo clean && cargo build # rule out stale build / config caching
```

When stuck, a generated project's `CLAUDE.md`/`AGENTS.md` lists the exact
per-crate "Additional configuration" doc links for that project.
