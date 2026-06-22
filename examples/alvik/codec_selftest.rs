//! On-device self-test for the Alvik ucPack codec and typed quantities.
//!
//! The repo's `embedded-test` harness can't link the `beet_esp` lib (its
//! `panic_rtt_target` panic handler clashes with `embedded_test`'s), so this
//! flashable binary stands in: it runs the pure byte/float assertions the unit
//! tests would and logs `PASS`/`FAIL` per check over RTT. Needs no Alvik
//! attached, so it doubles as a board-agnostic sanity check.
//!
//! Run with: `cargo run --release --no-default-features --features alvik --example alvik-codec-selftest`

#![no_std]
#![no_main]

use beet::prelude::*;
use beet_esp::alvik::protocol::Command;
use beet_esp::alvik::types::Side;
use beet_esp::alvik::protocol::Status;
use beet_esp::alvik::ucpack::Decoder;
use beet_esp::alvik::ucpack::Encoder;
use beet_esp::alvik::ucpack::crc8;
use beet_esp::prelude::*;

#[beet_esp::main]
fn main() {
    App::new()
        .add_plugins((Esp32Plugin, HealthPlugin))
        .add_systems(Update, run_selftest)
        .run();
}

/// Run every check once and log a final tally.
///
/// Runs a couple seconds after boot (gated on the elapsed clock) rather than at
/// `Startup`: the firmware's RTT logger is non-blocking, so a one-shot burst of
/// ~25 lines emitted during the boot storm (PSRAM init + first health dump) is
/// dropped before the host attaches. Waiting until the buffer has drained lets the
/// whole tally stream out reliably.
fn run_selftest(time: Res<Time>, mut done: Local<bool>) {
    if *done || time.elapsed_secs() < 2.0 {
        return;
    }
    *done = true;

    let mut checks = Checks::default();

    // Frame layout: `X` 'K' ack → START LEN CODE PAYLOAD END CRC.
    let mut enc = Encoder::new();
    let frame = enc.c1b(b'X', b'K');
    checks.eq("frame.start", frame[0] as u32, 0x41);
    checks.eq("frame.len", frame[1] as u32, 2);
    checks.eq("frame.code", frame[2] as u32, b'X' as u32);
    checks.eq("frame.end", frame[4] as u32, 0x23);
    checks.eq("frame.crc", frame[5] as u32, crc8(&[b'X', b'K']) as u32);

    // Encode → streaming decode round-trip of a 2×f32 payload.
    let bytes = enc.c2f(b'V', 1.5, -2.5);
    let mut dec = Decoder::new();
    dec.extend(bytes);
    match dec.pop() {
        Some(frame) => {
            checks.eq("roundtrip.code", frame.code as u32, b'V' as u32);
            checks.close("roundtrip.f0", frame.f32(0), 1.5);
            checks.close("roundtrip.f4", frame.f32(4), -2.5);
        }
        None => checks.fail("roundtrip.pop"),
    }
    checks.bool("roundtrip.drained", dec.pop().is_none());

    // Resync past leading garbage (including a false START byte).
    let mut dec = Decoder::new();
    let good = enc.c1b(b'b', 7);
    let mut owned = [0u8; 8];
    owned[..good.len()].copy_from_slice(good);
    dec.extend(&[0x00, 0x41, 0xFF, 0x12]);
    dec.extend(&owned[..good.len()]);
    match dec.pop() {
        Some(frame) => {
            checks.eq("resync.code", frame.code as u32, b'b' as u32);
            checks.eq("resync.payload", frame.u8(0) as u32, 7);
        }
        None => checks.fail("resync.pop"),
    }

    // Status decode shares the wire codes with the encoder.
    let bytes = enc.c2f(b'j', 12.0, -12.0);
    let mut dec = Decoder::new();
    dec.extend(bytes);
    match dec.pop().and_then(|frame| Status::decode(&frame)) {
        Some(Status::WheelSpeeds { left, right }) => {
            checks.close("status.left", left.as_rpm(), 12.0);
            checks.close("status.right", right.as_rpm(), -12.0);
        }
        _ => checks.fail("status.decode"),
    }

    // Per-wheel command framing: `W` <label> 'V' <rpm>.
    let frame = Command::WheelSpeed {
        wheel: Side::Right,
        speed: AngularVelocity::from_rpm(20.0),
    }
    .encode(&mut enc);
    checks.eq("cmd.code", frame[2] as u32, b'W' as u32);
    checks.eq("cmd.label", frame[3] as u32, b'R' as u32);
    checks.eq("cmd.sub", frame[4] as u32, b'V' as u32);

    // Cross-unit ratios from `conversions.py`.
    checks.close("angle.deg", Angle::from_degrees(180.0).as_radians(), core::f32::consts::PI);
    checks.close("angle.rev", Angle::from_revolutions(1.0).as_degrees(), 360.0);
    checks.close("angle.pct", Angle::from_percent(25.0).as_degrees(), 90.0);
    checks.close("av.rpm", AngularVelocity::from_rpm(60.0).as_rev_per_sec(), 1.0);
    checks.close("av.degs", AngularVelocity::from_deg_per_sec(180.0).as_rpm(), 30.0);
    checks.close("dist.cm", Distance::from_centimeters(5.0).as_millimeters(), 50.0);
    checks.close("dist.in", Distance::from_inches(1.0).as_millimeters(), 25.4);
    checks.close("vel.ms", LinearVelocity::from_m_per_sec(1.0).as_mm_per_sec(), 1000.0);

    if checks.failed == 0 {
        info!("SELFTEST PASSED: {} checks", checks.passed);
    } else {
        error!(
            "SELFTEST FAILED: {} of {} checks failed",
            checks.failed,
            checks.passed + checks.failed
        );
    }
}

/// Tally of passing / failing checks, logging each result over RTT.
#[derive(Default)]
struct Checks {
    passed: u32,
    failed: u32,
}

impl Checks {
    fn record(&mut self, name: &str, ok: bool) {
        if ok {
            self.passed += 1;
            info!("PASS {}", name);
        } else {
            self.failed += 1;
            error!("FAIL {}", name);
        }
    }
    fn bool(&mut self, name: &str, ok: bool) {
        self.record(name, ok);
    }
    fn fail(&mut self, name: &str) {
        self.record(name, false);
    }
    fn eq(&mut self, name: &str, got: u32, want: u32) {
        self.record(name, got == want);
    }
    fn close(&mut self, name: &str, got: f32, want: f32) {
        self.record(name, (got - want) < 0.01 && (want - got) < 0.01);
    }
}
