//! Goals — intentions a player holds, rather than grievances he
//! re-notices.
//!
//! The organ that fixes the second structural problem in
//! `docs/player_mind.md`: today a want is a `HappinessEvent` behind a
//! cooldown, and `process_transfer_desire` rebuilds its whole reason set
//! from live ground truth every single week. So a player has no way to
//! *hold* anything — no "I'll give it until January", no wanting
//! something quietly for a season before saying it.
//!
//! | Piece | Holds |
//! |---|---|
//! | [`catalog`] | the 33 things a player can want, as data |
//! | [`goal`] | one intention: strength, urgency, progress, status |
//! | [`escalation`] | the ladder from private feeling to formal demand |
//! | [`stack`] | the twelve he holds, and the weekly think |
//! | [`evidence`] | why he wants it, and what stops him acting |
//! | [`bridge`] | the parallel-run mapping to the legacy desire enums |
//!
//! ## The ladder
//!
//! ```text
//! Latent   he feels it, nobody knows, it changes nothing
//!   ↓
//! Active   it shapes every decision he makes — silently
//!   ↓
//! Voiced   he says it; the manager and the press can hear
//!   ↓
//! Pressing a formal demand
//!   ↓
//! Satisfied · Frustrated · Abandoned
//! ```
//!
//! `Active` is the rung that does not exist today for anything but
//! `big_stage_inclination`, and it is where most of the realism lives: a
//! player well below the level that produces a transfer request will
//! still *listen* when a bigger club calls, and hold out a little longer
//! when one hasn't.
//!
//! Climbing takes the full bar and falling back takes a clear drop below
//! it, one rung per weekly review. A mind that flips state every week is
//! not a mind.

pub mod bridge;
pub mod catalog;
pub mod escalation;
pub mod evidence;
pub mod goal;
pub mod stack;

pub use bridge::{GoalBridge, ReasonMapping};
pub use catalog::{GoalDirection, GoalKind, GoalMask, GoalSpec};
pub use escalation::{Escalation, StatusChange};
pub use evidence::{GoalBlocker, GoalDomain, GoalEvidence, GoalOrigin};
pub use goal::{GoalStatus, MindGoal};
pub use stack::{GoalCensus, GoalReviewReport, GoalStack, GoalStore};
