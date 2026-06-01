//! The ucPack wire codec: frame encoder, streaming decoder and CRC8.
//!
//! ucPack (`github:arduino/ucPack`) frames a payload as:
//!
//! ```text
//! [START=0x41 'A'] [LEN] [CODE] [PAYLOAD...] [END=0x23 '#'] [CRC8]
//! ```
//!
//! where `LEN` counts `CODE` through the last payload byte (`1 + payload`) and
//! `CRC8` is CRC8-MAXIM (poly `0x8C`) over those `LEN` bytes. This module knows
//! only frames, codes, scalars and CRC, so it is reusable for any ucPack peer —
//! the Alvik-specific [`Command`](super::protocol::Command) /
//! [`Status`](super::protocol::Status) mapping lives in `protocol.rs`.

const START: u8 = 0x41; // 'A'
const END: u8 = 0x23; // '#'

/// Largest payload we send or receive (C6F = 6×f32 = 24 bytes).
pub const PAYLOAD_MAX: usize = 24;
/// Largest full frame: `START LEN CODE …payload… END CRC` = `PAYLOAD_MAX + 5`.
pub const FRAME_MAX: usize = PAYLOAD_MAX + 5;

/// CRC8-MAXIM (poly `0x8C`, reflected, init `0x00`), computed over `CODE` plus
/// payload — the `LEN` bytes the frame's trailing CRC covers.
pub fn crc8(data: &[u8]) -> u8 {
    let mut crc = 0u8;
    for &byte in data {
        let mut bits = byte;
        for _ in 0..8 {
            let mix = (crc ^ bits) & 1;
            crc >>= 1;
            if mix != 0 {
                crc ^= 0x8C;
            }
            bits >>= 1;
        }
    }
    crc
}

/// A decoded, CRC-validated frame: its `code` and an owned copy of the payload.
///
/// Owned (not borrowed) so the [`Decoder`] can keep filling its buffer after a
/// frame is popped. Scalar accessors read the wire's little-endian layout.
#[derive(Clone, Copy)]
pub struct Frame {
    /// The single-byte message code (`'j'`, `0x7E`, …).
    pub code: u8,
    payload: [u8; PAYLOAD_MAX],
    len: usize,
}

impl Frame {
    /// The raw payload bytes after `code`.
    pub fn payload(&self) -> &[u8] {
        &self.payload[..self.len]
    }
    /// The `u8` at byte `offset`.
    pub fn u8(&self, offset: usize) -> u8 {
        self.payload[offset]
    }
    /// The little-endian `i16` at byte `offset`.
    pub fn i16(&self, offset: usize) -> i16 {
        i16::from_le_bytes([self.payload[offset], self.payload[offset + 1]])
    }
    /// The little-endian `f32` at byte `offset`.
    pub fn f32(&self, offset: usize) -> f32 {
        f32::from_le_bytes([
            self.payload[offset],
            self.payload[offset + 1],
            self.payload[offset + 2],
            self.payload[offset + 3],
        ])
    }
}

/// Builds ucPack frames into a reused scratch buffer.
///
/// Each `c*` method mirrors a ucPack packer (`packetC1B`, `packetC2F`, …),
/// writing the framed bytes and returning the slice ready to hand to the UART.
/// No allocation: the scratch is a fixed [`FRAME_MAX`] array.
pub struct Encoder {
    buf: [u8; FRAME_MAX],
}

impl Default for Encoder {
    fn default() -> Self {
        Self::new()
    }
}

impl Encoder {
    pub const fn new() -> Self {
        Self {
            buf: [0; FRAME_MAX],
        }
    }

    /// Frame `code` + `payload` into the scratch and return the full frame.
    fn frame(&mut self, code: u8, payload: &[u8]) -> &[u8] {
        let len = 1 + payload.len(); // CODE + payload
        self.buf[0] = START;
        self.buf[1] = len as u8;
        self.buf[2] = code;
        self.buf[3..3 + payload.len()].copy_from_slice(payload);
        self.buf[2 + len] = END;
        self.buf[3 + len] = crc8(&self.buf[2..2 + len]);
        &self.buf[..len + 4]
    }

    /// 1 × `u8`.
    pub fn c1b(&mut self, code: u8, a: u8) -> &[u8] {
        self.frame(code, &[a])
    }
    /// 2 × `u8`.
    pub fn c2b(&mut self, code: u8, a: u8, b: u8) -> &[u8] {
        self.frame(code, &[a, b])
    }
    /// 3 × `u8`.
    pub fn c3b(&mut self, code: u8, a: u8, b: u8, c: u8) -> &[u8] {
        self.frame(code, &[a, b, c])
    }
    /// 1 × `f32`.
    pub fn c1f(&mut self, code: u8, a: f32) -> &[u8] {
        self.frame(code, &a.to_le_bytes())
    }
    /// 2 × `f32`.
    pub fn c2f(&mut self, code: u8, a: f32, b: f32) -> &[u8] {
        let mut payload = [0u8; 8];
        payload[0..4].copy_from_slice(&a.to_le_bytes());
        payload[4..8].copy_from_slice(&b.to_le_bytes());
        self.frame(code, &payload)
    }
    /// 3 × `f32`.
    pub fn c3f(&mut self, code: u8, a: f32, b: f32, c: f32) -> &[u8] {
        let mut payload = [0u8; 12];
        payload[0..4].copy_from_slice(&a.to_le_bytes());
        payload[4..8].copy_from_slice(&b.to_le_bytes());
        payload[8..12].copy_from_slice(&c.to_le_bytes());
        self.frame(code, &payload)
    }
    /// `u8` + 3 × `f32` (wheel PID gains).
    pub fn c1b3f(&mut self, code: u8, a: u8, x: f32, y: f32, z: f32) -> &[u8] {
        let mut payload = [0u8; 13];
        payload[0] = a;
        payload[1..5].copy_from_slice(&x.to_le_bytes());
        payload[5..9].copy_from_slice(&y.to_le_bytes());
        payload[9..13].copy_from_slice(&z.to_le_bytes());
        self.frame(code, &payload)
    }
    /// 2 × `u8` + `f32` (per-wheel label/sub-command + value).
    pub fn c2b1f(&mut self, code: u8, a: u8, b: u8, x: f32) -> &[u8] {
        let mut payload = [0u8; 6];
        payload[0] = a;
        payload[1] = b;
        payload[2..6].copy_from_slice(&x.to_le_bytes());
        self.frame(code, &payload)
    }
}

/// Streaming frame parser: bytes are [`push`](Self::push)ed in as the UART
/// delivers them, and complete frames [`pop`](Self::pop)ped out once `START`,
/// `LEN`, `END` and `CRC` all line up. A bad CRC or misframe drops the leading
/// byte and resyncs on the next `START`.
pub struct Decoder {
    buf: [u8; Self::CAP],
    len: usize,
}

impl Default for Decoder {
    fn default() -> Self {
        Self::new()
    }
}

impl Decoder {
    /// Holds several max frames so a burst read never overflows mid-frame.
    const CAP: usize = 256;

    pub const fn new() -> Self {
        Self {
            buf: [0; Self::CAP],
            len: 0,
        }
    }

    /// Append one received byte, dropping the oldest if the buffer is full
    /// (only happens under sustained garbage, where old bytes are worthless).
    pub fn push(&mut self, byte: u8) {
        if self.len == Self::CAP {
            self.drop_front(1);
        }
        self.buf[self.len] = byte;
        self.len += 1;
    }

    /// Append a slice of received bytes.
    pub fn extend(&mut self, bytes: &[u8]) {
        for &byte in bytes {
            self.push(byte);
        }
    }

    /// Pop the next complete, CRC-valid frame, or `None` if more bytes are
    /// needed. Call in a loop to drain everything a read delivered.
    pub fn pop(&mut self) -> Option<Frame> {
        loop {
            // Resync: discard anything before a START byte.
            if self.len == 0 {
                return None;
            }
            if self.buf[0] != START {
                self.drop_front(1);
                continue;
            }
            if self.len < 2 {
                return None; // need LEN
            }
            let payload_len = (self.buf[1] as usize).wrapping_sub(1); // LEN counts CODE too
            if self.buf[1] == 0 || payload_len > PAYLOAD_MAX {
                self.drop_front(1); // implausible length — resync
                continue;
            }
            let total = self.buf[1] as usize + 4; // START LEN …LEN bytes… END CRC
            if self.len < total {
                return None; // frame not fully arrived yet
            }
            let len_field = self.buf[1] as usize;
            let end_ok = self.buf[len_field + 2] == END;
            let crc_ok = self.buf[len_field + 3] == crc8(&self.buf[2..2 + len_field]);
            if !end_ok || !crc_ok {
                self.drop_front(1); // misframe — resync past this START
                continue;
            }
            let mut payload = [0u8; PAYLOAD_MAX];
            payload[..payload_len].copy_from_slice(&self.buf[3..3 + payload_len]);
            let frame = Frame {
                code: self.buf[2],
                payload,
                len: payload_len,
            };
            self.drop_front(total);
            return Some(frame);
        }
    }

    fn drop_front(&mut self, n: usize) {
        self.buf.copy_within(n..self.len, 0);
        self.len -= n;
    }
}
