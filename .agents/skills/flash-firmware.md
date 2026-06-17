# Flashing AlvikCarrier (STM32) firmware

How to update the Alvik's **carrier firmware** (the STM32F411 on the robot's
carrier board) from outside the Arduino ecosystem, using our own `flash-firmware`
bin as a USB-to-UART bridge into the STM32 ROM bootloader.

## When to use this

The Rust port replaces the Arduino/MicroPython stack on the Nano ESP32, so we
have no Arduino tool to push carrier firmware. But the STM32 sits behind the Nano
on UART1, with its `BOOT0`/`RESET` lines wired to ESP `GPIO5`/`GPIO6` (which our
firmware already drives), and the STM32 ROM bootloader speaks the standard ST
UART protocol (AN3155). So `src/bin/flash-firmware.rs` drops the STM32 into its
bootloader and bridges the Nano's native USB-Serial-JTAG CDC (`/dev/ttyACM*`) to
UART1, and host-side `stm32flash` writes the new `.bin`.

The two version lines are independent:

- **AlvikCarrier firmware** (STM32, this skill): the wire peer of our UART driver.
  Our port targets `1.1.1` (`REQUIRED_FW` in `src/alvik/driver.rs`).
- **Arduino Alvik MicroPython library**: the layer we *replaced*; irrelevant here.

## Prerequisites

The device must be the Alvik's Nano ESP32 (`probe-rs list` shows `303a:1001`) and
the Alvik must be **powered on**. You need three things:

1. **The firmware `.bin`.** Arduino publishes a flashable binary per release:
   ```shell
   gh release list --repo arduino-libraries/Arduino_AlvikCarrier        # find versions
   gh release download --repo arduino-libraries/Arduino_AlvikCarrier 1.1.1 \
       --pattern 'firmware_*.bin' --dir .agents/tmp/alvik-fw
   ```
   The assets are named `firmware_<maj>_<min>_<patch>.bin`. Confirm the target
   version is compatible with the `REQUIRED_FW` our driver expects.

2. **`stm32flash`** (the canonical C tool; handles the RDP + reset dance far
   better than the python `stm32loader`). Not in the Arch repos; build from source
   (no deps, no sudo):
   ```shell
   cd .agents/tmp && git clone --depth 1 https://gitlab.com/stm32flash/stm32flash.git
   cd stm32flash && make          # produces ./stm32flash
   ```

3. **The bridge bin built**: `cargo build --release --no-default-features
   --features device,alvik --bin flash-firmware`.

## Procedure

The bridge runs UART1 at **9600 8E1** (even parity is mandatory for the STM32 ROM
bootloader; the slow rate is for reliability, see Gotchas). The host-side `-b` is
cosmetic (the host link is USB-CDC, not a real UART), but pass `-b 9600` for
clarity. **Run the bridge with `espflash`, never probe-rs** (see Gotchas).

```shell
TTY=/dev/ttyACM0     # the Nano's USB-Serial-JTAG CDC; confirm with `ls /dev/ttyACM*`
SF=.agents/tmp/stm32flash/stm32flash
FW=.agents/tmp/alvik-fw/firmware_1_1_1.bin
ELF=target/xtensa-esp32s3-none-elf/release/flash-firmware   # or $CARGO_TARGET_DIR/...

. $HOME/export-esp.sh

# 1. build, then flash + run the bridge FREE of any debugger (espflash resets the
#    chip to run the app and exits; probe-rs would corrupt the CDC, see Gotchas)
cargo build --release --no-default-features --features device,alvik --bin flash-firmware
espflash flash --chip esp32s3 --port $TTY $ELF
sleep 4

# 2. sanity check the link (retry a couple times; the first 0x7F may miss)
for n in 1 2 3; do $SF -b 9600 $TTY 2>&1 | grep -q "Device ID" && break; sleep 1; done
$SF -b 9600 $TTY                        # expect: Device ID 0x0431 (STM32F411xx)

# 3. FIRST TIME ONLY: the carrier ships read-protected. Remove RDP (MASS-ERASES),
#    then re-run espflash to re-pulse the STM32 into the bootloader.
$SF -b 9600 -k $TTY
espflash flash --chip esp32s3 --port $TTY $ELF && sleep 4

# 4. write the firmware. A clean write ACKs every page against its checksum, so a
#    write that finishes without error == a good flash. Give it ~7 min at 9600.
$SF -b 9600 -w "$FW" $TTY

# 5. (optional, ideal) byte-compare read-back. This is the most desync-prone step
#    at 9600; if it fails partway, the flash is usually still fine (the write ACKed).
$SF -b 9600 -S 0x08000000:$(stat -c%s "$FW") -r /tmp/rb.bin $TTY && cmp "$FW" /tmp/rb.bin && echo MATCH

# 6. restore our app — `cargo run` (probe-rs) is fine here, it only resets the
#    STM32 (BOOT0 low) into the freshly-flashed carrier firmware and shows RTT.
cargo run --release --no-default-features --features device,alvik --example alvik-sensors
# confirm: `alvik: firmware (M, m, p) ...` and streaming LINE/COLOR/TOF/IMU lines.
```

## Gotchas

- **Run the bridge with espflash, NOT probe-rs.** This is the single biggest
  lesson. probe-rs polls the JTAG endpoint of the *same* `303a:1001` USB device as
  the CDC, which corrupts the bridged stream and makes `stm32flash` fail at init or
  desync mid-transfer ("Got byte 0xNN instead of ACK", "Failed to init device").
  `espflash flash <elf>` writes the app and resets the chip to run **free of any
  debugger**, leaving the CDC clean. probe-rs (cargo run) is only fine for the
  final step, where it just resets the STM32 into the new firmware and shows RTT.

- **Re-pulse by re-running espflash.** The bridge pulses the STM32 into its
  bootloader once, at its own boot. After anything that resets the STM32 (a failed
  transfer that locks the bootloader, or the `-k` option-byte change), re-run
  `espflash flash <elf>` to reboot the bridge and re-pulse a fresh bootloader entry.

- **`pkill -f "probe-rs..."` is a footgun**: the pattern matches your own shell
  command line and kills the parent (exit 144). Use `pkill -x probe-rs` / kill by PID.

- **Readout protection (first flash of a factory carrier).** The factory firmware
  enables RDP level 1; write fails until `stm32flash -k` (readout-unprotect), which
  mass-erases. The carrier stores no calibration in flash, so the erase is benign
  (verified: re-flashing 1.0.0 afterward restored full sensor function). After `-k`,
  re-run espflash before writing. Once unprotected, later re-flashes skip `-k`.

- **9600 baud for reliability.** The single-buffer bridge drops a byte on UART RX
  FIFO overflow, and one lost byte is fatal to `stm32flash`'s protocol. 19200 is
  borderline (works sometimes); 9600 is solid for the write. The continuous
  read-back (`-r`) is the most demanding direction and can still desync — treat a
  clean *write* as the real success signal. To change, edit `BRIDGE_BAUD` in
  `src/bin/flash-firmware.rs`. (A proper fix would decouple UART-drain from USB-write
  with a buffered task, which would allow a much higher rate.)

- **Recovery / no-brick.** The STM32 ROM bootloader lives in silicon and is always
  reachable (`BOOT0` high + reset, which the bridge does on boot), so a failed or
  interrupted write never bricks the carrier: re-run espflash and retry the write.

## Known issue: carrier 1.1.1 hangs on this unit

Flashing `firmware_1_1_1.bin` verified byte-exact, but the carrier then stayed
**silent** (zero UART, the Nano logs "not connected", no error-LED blink). 1.0.0
re-flashed to the same chip works perfectly (all sensors stream), so the hardware
is healthy and the flash path is sound — the hang is inside 1.1.x's `begin()`. The
new-in-1.1.x init that hangs *silently* (no `errorLed`) is `beginBMS()` (the
MAX17332 battery gauge) and the ToF-matrix init. On 1.0.0 the battery read **0%**,
so the prime suspect is a depleted/low battery stalling the BMS init. **Charge the
Alvik before retrying 1.1.x.** Error-LED blink codes (carrier `LED_BUILTIN`/PC8) if
it *does* blink: 1 = APDS color, 2 = BMS battery, 4 = IMU.
