//! **Rules that move a player against his own state's wishes.**
//!
//! Every state machine decides where its player wants to be. These four
//! reach past that decision, because the situation is one no single
//! state can see — and each exists because leaving it to the states
//! produced a visible defect:
//!
//! * [`corner_hold`] — [`CornerHold`]: stand where the corner routine
//!   put you until the ball comes near. Without it the box empties
//!   inside the corner's own lifetime.
//! * [`restart_carry`] — [`RestartCarry`]: the taker walking the ball
//!   back to the spot. Every chase behaviour reads a ball at your feet
//!   as reached, so he stopped dead the moment he picked it up.
//! * [`keeper_space`] — [`KeeperReleaseSpace`]: back out of the area
//!   when the keeper has it in his hands. Nothing else moved a player
//!   for that, so forwards stood over a keeper holding the ball.
//! * [`loose_ball`] — [`LooseBallChase`]: keep a race a race. The
//!   `TakeBall` states' plain separation pointed away from the rival,
//!   which is to say away from the ball.
//!
//! The first three are applied at the single point every state's
//! movement converges on (`StateProcessor::process_inner`), and the
//! order there is load-bearing: they run **ahead of** `ShapeDiscipline`,
//! whose slots would otherwise pull the player straight back into the
//! position he is being moved out of.

pub mod corner_hold;
pub mod keeper_space;
pub mod loose_ball;
pub mod restart_carry;

pub use corner_hold::*;
pub use keeper_space::*;
pub use loose_ball::*;
pub use restart_carry::*;
