//! The judgements organ — staff-only.
//!
//! Memory and goals are shared with the player ([`club::mind::organs`]).
//! This third organ is not: it is everything a coach thinks about the
//! people he picks, and a player has no use for one.
//!
//! Three tenants, all of them living on the man:
//!
//! | Module | Holds |
//! |---|---|
//! | [`judgement`] | [`PlayerJudgement`] — a persistent, revisable, **scorable** read of a player's ability |
//! | [`coach_memory`] | [`CoachMemory`] — the coach's interpretation of a body of work; what selection reads |
//! | [`impressions`] | [`CoachDecisionState`] — his running impressions of the squad, and the accumulators that go with them |
//!
//! This is where `docs/staff_mind.md` §2.6 is answered. A coach's
//! per-player state used to live in two places: `CoachMemoryStore` on
//! `Staff`, which travelled with him, and `CoachDecisionState` on
//! `TeamCollection`, which was rebuilt from nothing whenever the
//! head-coach id changed. Two accumulators that exist precisely to have
//! history, one of them attached to the club rather than the man, and
//! discarded when he left. Now there is one organ, and it belongs to
//! the coach.
//!
//! What stays club-side is the one genuinely club-side fact:
//! `TeamCollection::previous_head_coach_id`, so a club knows when its
//! manager has *changed* and the squad reacts.
//!
//! [`club::mind::organs`]: crate::club::mind::organs

#[cfg(test)]
mod census;
pub mod coach_memory;
mod impression_lens;
pub mod impressions;
pub mod judgement;
pub mod store;

pub use coach_memory::{
    CoachMatchObservation, CoachMemory, CoachMemoryFlags, CoachMemoryStore, MemoryEngine,
};
pub use impressions::CoachDecisionState;
pub use judgement::{JudgementOutcome, PlayerJudgement};
pub use store::{JudgementCensus, JudgementStore, Judgements};
