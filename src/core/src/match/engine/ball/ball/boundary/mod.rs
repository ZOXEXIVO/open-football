//! **The edges of the playing area** — everything that happens because
//! the ball reached a line, a post, or the hoardings.
//!
//! * [`goal`](self) — did it go in, over, or wide (private; it only adds
//!   `impl Ball` methods).
//! * [`frame`] — the woodwork itself: real posts and crossbar with
//!   swept-volume contact, and the rebounds off them.
//! * [`net`] — what the ball does once it has crossed the line.
//! * [`runoff`] — the ground outside the lines and the boards at the end
//!   of it, which is where a restart taker goes to fetch the ball.

pub mod frame;
mod goal;
pub mod net;
pub mod runoff;

pub use runoff::{Perimeter, RunOff};
