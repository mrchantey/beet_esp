# no_std on Xtensa (ESP32 / ESP32-S2 / ESP32-S3)

For bare-metal `esp-hal` firmware on Xtensa chips. **Read `no_std-app-dev.md`**
for application topics (logging, alloc, async, OTA, testing, config) — they are
shared with RISC-V and not repeated here.

## Chips

Xtensa chips: **ESP32, ESP32-S2, ESP32-S3**.

## Why Xtensa is different

Rust does not officially support Xtensa, because Rust compiles via LLVM and
**upstream LLVM has no Xtensa backend yet**. Espressif maintains **forks of both
LLVM and the Rust compiler** that add Xtensa support, and is upstreaming them.
So Xtensa needs a forked toolchain, installed via `espup`.

## Toolchain setup (espup)

```shell
cargo binstall -y espup --locked      # or download a release binary / cargo-binstall
espup install                     # installs all toolchains for Espressif targets
```

On **Unix**, source the env file `espup` writes (so the `esp` toolchain and GCC
are on PATH); the path is printed by `espup` (commonly `~/export-esp.sh`):

```shell
. $HOME/export-esp.sh
```

Windows users don't need this step. Keep it updated with `espup update`.

### What `espup install` provides

- The **Espressif Rust fork** (the `esp` toolchain) with Xtensa target support.
- A `stable` toolchain with RISC-V support (so one install covers both arches).
- The **LLVM fork** with Xtensa support.
- A **GCC toolchain** that links the final binary.

The forked compiler coexists with standard Rust and is selected via the usual
rustup [overrides] — typically a `rust-toolchain.toml` pinning `channel = "esp"`,
which `esp-generate` writes for you. Invoke explicitly with `cargo +esp ...` if
no override is set.

[overrides]: https://rust-lang.github.io/rustup/overrides.html

## Xtensa specifics worth knowing

- **PSRAM atomics are broken on Xtensa.** `Atomic*` operations in external PSRAM
  can cause data races and silently misbehave. The global allocator **must not**
  allocate `Atomic*` types (directly or indirectly) into PSRAM. This does not
  affect RISC-V chips. See ESP-IDF external-RAM restrictions for details.
- **`probe-rs` without external hardware** needs the `USB-JTAG-SERIAL`
  peripheral, which among Xtensa chips exists **only on ESP32-S3**. For **ESP32
  and ESP32-S2** you need an external probe (ESP-Prog) for probe-rs / HIL testing.
- Because Xtensa needs the GCC linker from `espup`, a missing/old `export-esp`
  environment is the usual cause of "linker not found" / link errors.

## Typical workflow

```shell
esp-generate --chip esp32s3 my-app   # or run interactively: esp-generate
cd my-app
cargo run --release                  # compiles, flashes, monitors
```

See `cli-tools.md` for `esp-generate` options and `espflash`/`probe-rs` details.
