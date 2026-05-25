# Workflow: Connect the Hardware

Goal: get the board physically connected on the **correct port** for the chosen
flashing/debug tool, and verify the host can see it. Picking the wrong port is
the most common reason `cargo run` / flashing fails.

You can't see the user's desk — **ask them to confirm** what they plugged in and
relay the verification command output.

## Step 1 — Understand the ports

Most Espressif DevKits expose **two USB ports** (using the ESP32-C6-DevKitC-1 as
the example; labels vary by board):

| Port (typical label) | Goes through            | Use for                                                        |
| -------------------- | ----------------------- | -------------------------------------------------------------- |
| **UART** / "COM"     | Onboard USB-to-UART chip| Power, `espflash` flashing + serial monitor (the classic path) |
| **USB** / "USB"      | Chip's native USB-Serial-JTAG | Power, flashing, USB comms, **JTAG debugging → `probe-rs`** |

Some boards (especially cheap ones) have **only a USB-UART adapter** and no
native USB peripheral exposed.

## Step 2 — Pick the port for your tool

### Using `espflash` (default flashing path)

- Use the **UART bridge** port on classic boards. On chips with native
  USB-Serial-JTAG, that port works for flashing too — either is fine.
- This is what `cargo run --release` uses in espflash-configured projects.

### Using `probe-rs` (RTT/`defmt`, debugging, `embedded-test`)

`probe-rs` needs a **JTAG** connection, not a plain serial port:

- **Chips with native USB-Serial-JTAG** — `ESP32-C3 (rev 0.3+)`, `ESP32-C5`,
  `ESP32-C6`, `ESP32-C61`, `ESP32-H2`, `ESP32-S3` — plug into the **native USB**
  port; no external hardware needed.
- **Chips without it** — `ESP32`, `ESP32-S2`, `ESP32-C2` (and any board exposing
  only a USB-UART adapter) — you need an **external debug probe (ESP-Prog)** wired
  to the JTAG pins. A USB-UART adapter will **not** work with `probe-rs`.

## Step 3 — Verify the host sees the device

Ask the user to run the relevant check and share the output.

```shell
espflash board-info      # confirms chip, revision, features, MAC over serial
probe-rs list            # confirms a debug probe/JTAG is visible (probe-rs path)
```

Identify the serial device:

- **Linux:** USB-UART bridge → `/dev/ttyUSB*`; native USB-Serial-JTAG →
  `/dev/ttyACM*`.
- **macOS:** `/dev/cu.usbserial-*` or `/dev/cu.usbmodem-*`.
- **Windows:** a `COM` port in Device Manager.

If **multiple boards** are connected, pass `--port <PORT>` to `espflash` (or set
it in the project config) so it targets the right one.

## Step 4 — Permissions (Linux)

"Permission denied" on the port is common:

- Add the user to the serial group (`dialout` on most distros, `uucp` on Arch),
  then log out/in: `sudo usermod -aG dialout $USER`.
- For `probe-rs`, install its **udev rules** (see probe.rs setup docs) so the
  JTAG device is accessible without root.

## Step 5 — Boot / Download mode

To flash, the chip must be in **Download mode**; to run firmware it must be in
**SPI Boot mode**. The ROM bootloader picks the mode from strapping-pin levels
sampled at reset.

- `espflash`/`probe-rs` normally toggle DTR/RTS to enter Download mode
  automatically. If that fails (some boards/USB-Serial-JTAG setups), do it
  manually: **hold `Boot`, press and release `Reset`, release `Boot`.**
- Serial output `waiting for download` means the chip is in Download mode.
- After flashing, the chip returns to run firmware on reset. If it stays in
  Download mode over USB-Serial/JTAG, flash with
  `espflash ... --after watchdog-reset`.

## Step 6 — Hand off

Once `board-info` (or `probe-rs list`) succeeds, the project is ready to flash:

```shell
cargo run --release      # builds, flashes, and opens the monitor
```

If it was generated for `probe-rs`, ensure you're on the native USB / probe
connection from Step 2 before running.

If flashing or `probe-rs list` keeps failing, see `troubleshooting.md`.
