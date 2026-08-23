//! The sub-mind contract.
//!
//! Five faculties, one interface, four verbs. A sub-mind owns a slice of
//! how a player reads his own situation; the shared state it reasons
//! over — what he remembers, what he wants — lives in [`MindOrgans`] so
//! no sub-mind has to know about any other.
//!
//! ```text
//! observe   something happened; interpret it
//! reflect   the periodic think: form wants, revise the reading
//! appraise  contribute to how he feels
//! weigh     give an opinion on a decision he faces
//! ```
//!
//! Declared as a trait for discipline and called through concrete fields
//! for speed — the world tick is CPU-bound and there is no reason to pay
//! for dynamic dispatch across five known types. Adding a faculty is one
//! folder, one field on [`PlayerMind`], four methods, and its name in the
//! fan-out.
//!
//! The three things a faculty *returns* — a mood contribution, a set of
//! reasons, the option it was asked about — are not player-specific and
//! live in [`crate::club::mind::verdict`], shared with the staff mind.
//! They are re-exported here so every existing `mind::submind::…` path
//! keeps working.
//!
//! [`MindOrgans`]: super::organs::MindOrgans
//! [`PlayerMind`]: super::PlayerMind

use super::organs::MindOrgans;
use super::organs::goals::GoalDomain;
use super::organs::memory::{EpochDay, MindClock, MindEpisode};
use super::situation::MindSituation;
use crate::club::player::mind::MindTickContext;

pub use crate::club::mind::verdict::{MindOption, MoodContribution, ReasonSet, WeightedReason};

/// What a sub-mind reads when it thinks. The tick context plus the
/// read-only picture of where the player actually is.
#[derive(Debug, Clone, Copy)]
pub struct MindView<'a> {
    pub tick: &'a MindTickContext,
    pub situation: &'a MindSituation,
}

impl MindView<'_> {
    /// Today, on the mind's own compact clock.
    #[inline]
    pub fn today(&self) -> EpochDay {
        MindClock::day(self.tick.today)
    }
}

/// One faculty of a player's mind.
pub trait SubMind {
    /// Which part of his life this faculty speaks for.
    fn domain(&self) -> GoalDomain;

    /// Interpret something that happened. Called per episode, at the
    /// moment it is recorded — so a faculty sees events as they land
    /// rather than discovering them on its next think.
    fn observe(&mut self, episode: &MindEpisode, organs: &mut MindOrgans);

    /// The periodic think. Reads its own state and the organs, revises
    /// its reading of the world, and forms or advances the wants it is
    /// responsible for.
    fn reflect(&mut self, view: &MindView<'_>, organs: &mut MindOrgans);

    /// This faculty's contribution to how he feels.
    fn appraise(&self, organs: &MindOrgans) -> MoodContribution;

    /// This faculty's opinion on a decision. Empty until the
    /// deliberation layer lands in phase 5.
    fn weigh(&self, _option: MindOption, _organs: &MindOrgans) -> ReasonSet {
        ReasonSet::new()
    }
}
