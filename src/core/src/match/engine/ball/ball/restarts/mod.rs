//! **Getting the ball back into play**, and noticing when it never was.
//!
//! * [`awaited`] — [`AwaitedRestart`], the whole restart state machine
//!   the ball carries: where it is taken from, who takes it, and the
//!   patience clock; plus [`PassOriginRestart`], which restart a live
//!   pass came from.
//! * [`offside`] — the geometry snapshot taken at the kick
//!   ([`OffsideSnapshot`]) and the line itself ([`OffsideLine`]), shared
//!   by the referee and by the passer deciding whether to play it.
//! * [`restart`](self) — the takers and their walks: [`DeadBall`],
//!   [`ThrowIn`], [`CornerWalk`], [`FoulWalk`] (private; the rest is
//!   `impl Ball`).
//! * [`stall`] — the position-anchor stall detector and its snapshot
//!   diagnostics: the net that catches a ball nobody ever restarted.

pub mod awaited;
pub mod offside;
mod restart;
pub mod stall;

pub use awaited::{AwaitedRestart, PassOriginRestart};
pub use offside::{OffsideLine, OffsideSnapshot};
pub use restart::{CornerWalk, DeadBall, FoulWalk, ThrowIn};
