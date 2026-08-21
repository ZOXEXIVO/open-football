//! Cross-cutting state helpers — the rules every role's state machine
//! shares, grouped by concern:
//!
//! * [`exertion`] — how hard a player is working, what it costs his
//!   condition, and the speed ceiling that tier imposes.
//! * [`engagement`] — going to the man with the ball: where he commits,
//!   where he gives up, whether he tackles, and whether it is a foul.
//! * [`overrides`] — rules that move a player against his own state's
//!   wishes (corner hold, restart carry, keeper release space, the
//!   loose-ball race).
//! * [`injured`] — the shared down-injured handler.
//!
//! Each group re-exports its own modules and this module re-exports the
//! groups, so the flat `states::Item` paths the state machines import
//! keep resolving exactly as they did before the regrouping.

pub mod engagement;
pub mod exertion;
pub mod injured;
pub mod overrides;

pub use engagement::*;
pub use exertion::*;
pub use injured::*;
pub use overrides::*;
