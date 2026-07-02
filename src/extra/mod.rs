//! Higher-level, composable capabilities a loaded `.bsx` scene wires together,
//! built on the firmware's hardware + transport layers.
//!
//! Unlike [`alvik`](crate::alvik) (the robot driver) or [`net`](crate::net) (the
//! socket/HTTP transport), these are *registered* reflectable pieces a pushed scene
//! composes — nothing here runs by default. Gated like the `alvik` feature since
//! the perceive-act body needs both `alvik` (the robot) and `sockets` (the exchange
//! transport back to the agent).

// The perceive-act body: the reflectable types (the `<AgentSocket>` transport + the
// `whoami`/`apply-heading` capability routes) a scene wires to turn the Alvik into
// the v3 socket body client.
pub mod perceive_act_body;

pub mod prelude {
    pub use super::perceive_act_body::*;
}
