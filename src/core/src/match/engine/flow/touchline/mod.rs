//! **The strip of ground outside the touchline**, and the ten or twelve
//! seconds of football that happen on it.
//!
//! # Why this exists at all
//!
//! A substitution used to be an assignment. `Substitutions::execute_substitution`
//! took the man off the pitch out of `field.players`, wrote the man off the
//! bench into his slot at his formation spot, and the match carried on inside
//! the same tick. Nothing moved, because there was nothing there to move: the
//! bench was parked at the off-pitch sentinel `(-500, -500)` and never
//! recorded, so as far as a replay was concerned a substitute did not exist
//! until the frame he appeared, fully formed, in the centre circle.
//!
//! Which is the second-most-watched thing a manager does in ninety minutes,
//! drawn as a body swap.
//!
//! # What happens instead
//!
//! * The bench is a real row of men standing in the run-off in front of their
//!   dugout ([`Bench`]), drawn by the recorder for the whole match.
//! * A substitution opens a [`SubstitutionBreak`]: play stops, the outgoing
//!   player runs for the nearest point on the bench touchline, the incoming
//!   one comes off the bench, in at the halfway line and out to the slot he
//!   is taking over. Everybody else stands still, because the ball is dead
//!   and this is what twenty men do while it is.
//! * The ROSTER, meanwhile, changes on the tick the decision is made, exactly
//!   as it always did — see the note on [`changeover`] for why that half
//!   cannot be deferred with the other. When the window closes the man who
//!   came off is still walking to his seat, and the recorder finishes it for
//!   him.
//!
//! # The two rules this module inherits from the celebration
//!
//! It is the same kind of thing as [`celebration`](super::celebration), and
//! the constraints are the same two, both load-bearing:
//!
//! 1. **Nothing in here may draw from [`MatchContext::rng`]**, emit an event,
//!    run a state machine or touch a statistic. The RNG stream is shared with
//!    every calibrated roll in the engine; one extra draw shifts every
//!    subsequent decision in the match. Variation comes from player ids.
//! 2. **A substitution only interrupts football that has already stopped.**
//!    The substitution pass is gated on a dead ball in
//!    `PlayLifecycle::play_inner`, so the break never freezes a ball in
//!    flight, never strands a possession, and never has to put one down.
//!
//! [`MatchContext::rng`]: super::context::MatchContext::rng

pub mod bench;
pub mod changeover;

pub use bench::{Bench, TouchlineStand};
pub use changeover::{
    Changeover, SubstitutionBreak, advance_substitution_break, finish_substitution_break,
};
