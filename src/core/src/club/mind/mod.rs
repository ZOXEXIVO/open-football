//! The mind, shared.
//!
//! Two people at a football club hold the same kind of interior life:
//! they remember specific things that happened, they distil those into
//! convictions that outlive the memories, they keep a running account of
//! everyone who has mattered, and they want things — quietly at first,
//! then out loud.
//!
//! None of that machinery is player-specific, so it does not live under
//! the player any more. [`organs`] is the shared state; the two global
//! minds that own it are:
//!
//! | Mind | Where | Faculties |
//! |---|---|---|
//! | [`PlayerMind`] | `club::player::mind` | career · competitive · professional · social · financial |
//! | [`StaffMind`] | `club::staff::mind` | ambition · authority · judgement · philosophy · welfare |
//!
//! The catalogs are **extended, not duplicated**. `EpisodeKind`,
//! `FactClaim` and `GoalKind` carry manager rows alongside the player
//! rows, because much of the vocabulary is genuinely shared: a manager
//! and a player who were both at a club the year it went down remember
//! the same event, and [`ActorRef`] already spans both directions — the
//! player's memory of a coach and the coach's memory of that player use
//! the same key type, pointing opposite ways.
//!
//! See `docs/player_mind.md` and `docs/staff_mind.md`.
//!
//! [`PlayerMind`]: crate::club::player::mind::PlayerMind
//! [`StaffMind`]: crate::club::staff::mind::StaffMind
//! [`ActorRef`]: organs::memory::ActorRef

#[cfg(test)]
mod catalog_tests;
pub mod organs;
pub mod verdict;

pub use organs::MindOrgans;
pub use verdict::{MindOption, MoodContribution, ReasonSet, WeightedReason};
