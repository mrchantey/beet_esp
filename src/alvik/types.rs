//! Small free-standing value types shared across the Alvik modules: the wheel
//! [`Side`], the decoded [`TouchButton`] / [`TiltAxis`] enums, and the
//! [`EdgeState`] the event detector keeps between frames.

/// Which wheel a per-wheel command addresses. The discriminant is the wire
/// label byte the STM32 expects.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Side {
    Left,
    Right,
}

impl Side {
    /// The ucPack label byte (`'L'` / `'R'`).
    pub fn label(self) -> u8 {
        match self {
            Side::Left => b'L',
            Side::Right => b'R',
        }
    }
}

/// A touch button, as decoded from the `t` bitmask.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TouchButton {
    Ok,
    Cancel,
    Center,
    Up,
    Left,
    Down,
    Right,
}

/// A tilt axis, as decoded from the `m` bitmask.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TiltAxis {
    X,
    NegX,
    Y,
    NegY,
    Z,
    NegZ,
}

/// Previous-frame bitmasks for edge detection. `motion` starts at the upright
/// `-Z` tilt bit so a fresh, level robot does not fire a tilt on the first frame.
pub struct EdgeState {
    pub(crate) touch: u8,
    pub(crate) motion: u8,
}

impl Default for EdgeState {
    fn default() -> Self {
        Self {
            touch: 0,
            motion: super::events::MOTION_TILT_NEG_Z,
        }
    }
}
