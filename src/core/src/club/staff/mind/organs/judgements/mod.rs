//! The judgements organ — staff-only.
//!
//! Memory and goals are shared with the player ([`club::mind::organs`]).
//! This third organ is not: it is a persistent, revisable read of
//! **other people's ability**, and a player has no use for one.
//!
//! It is also where the duplication in `docs/staff_mind.md` §2.6 is
//! answered. Today a coach's per-player state lives in two places —
//! `CoachMemoryStore` on `Staff`, which travels with him, and
//! `CoachDecisionState.impressions` on `TeamCollection`, which is
//! rebuilt from nothing whenever the head-coach id changes. Two
//! accumulators that exist precisely to have history, one of them
//! attached to the club rather than the man. Here there is one store,
//! and it belongs to the coach.
//!
//! [`club::mind::organs`]: crate::club::mind::organs

pub mod judgement;
pub mod store;

pub use judgement::{JudgementOutcome, PlayerJudgement};
pub use store::{JudgementCensus, JudgementStore, Judgements};
