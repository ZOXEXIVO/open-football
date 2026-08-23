//! **The challenge** — going to the man with the ball, and what happens
//! when you get there.
//!
//! * [`distances`] — the ordered thresholds. [`TackleEngagement`] and
//!   [`MarkEngagement`] each say where a player commits and where he
//!   gives up, and the ordering between the two is the whole point: a
//!   state whose give-up condition overlaps its own entry condition is a
//!   two-cycle, and the engine has burned millions of state entries on
//!   exactly that defect.
//! * [`duel`] — what he does once engaged. [`TackleDecision`] (does he
//!   commit this second, or keep containing), [`RecoveryChallenge`]
//!   (the poke, the stretch and the slide across — the challenge a man
//!   who has been gone PAST can still make, and the one the engine had
//!   no model for at all), [`ContactFoul`] (the shirt-pull that is not a
//!   challenge for the ball at all), and
//!   [`PenaltyRisk`], which both consult so restraint in the box asks
//!   the referee's question in the referee's terms — the BALL's
//!   position, not the fouler's.

pub mod distances;
pub mod duel;

pub use distances::*;
pub use duel::*;
