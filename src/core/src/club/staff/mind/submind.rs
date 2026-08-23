//! The sub-mind contract, staff side.
//!
//! The same four verbs the player's faculties implement, over
//! [`StaffOrgans`] instead of `MindOrgans` — because a manager has a
//! third organ and the trait has to be able to reach it.
//!
//! ```text
//! observe   something happened; interpret it
//! reflect   the periodic think: form wants, revise the reading
//! appraise  contribute to how he feels about the job
//! weigh     give an opinion on a decision he faces
//! ```
//!
//! What a faculty *returns* is shared with the player mind and lives in
//! [`club::mind::verdict`]: a [`MoodContribution`] on one axis, or a
//! [`ReasonSet`] of named, weighted arguments about a [`MindOption`].
//!
//! [`club::mind::verdict`]: crate::club::mind::verdict

use super::organs::StaffOrgans;
use super::situation::StaffSituation;
use crate::club::mind::organs::goals::GoalDomain;
use crate::club::mind::organs::memory::{EpochDay, MindClock, MindEpisode};
use crate::club::staff::mind::StaffTickContext;

pub use crate::club::mind::verdict::{MindOption, MoodContribution, ReasonSet, WeightedReason};

/// What a faculty reads when it thinks.
#[derive(Debug, Clone, Copy)]
pub struct StaffView<'a> {
    pub tick: &'a StaffTickContext,
    pub situation: &'a StaffSituation,
}

impl StaffView<'_> {
    /// Today, on the mind's own compact clock.
    #[inline]
    pub fn today(&self) -> EpochDay {
        MindClock::day(self.tick.today)
    }
}

/// One faculty of a manager's mind.
pub trait StaffSubMind {
    /// Which part of the job this faculty speaks for.
    fn domain(&self) -> GoalDomain;

    /// Interpret something that happened, at the moment it is recorded.
    fn observe(&mut self, episode: &MindEpisode, organs: &mut StaffOrgans);

    /// The periodic think: revise the reading, form or advance wants.
    fn reflect(&mut self, view: &StaffView<'_>, organs: &mut StaffOrgans);

    /// This faculty's contribution to how he feels about the job.
    fn appraise(&self, organs: &StaffOrgans) -> MoodContribution;

    /// This faculty's opinion on a decision. Empty by default so a
    /// faculty opts into the decisions it actually has a view on rather
    /// than being obliged to answer every question.
    fn weigh(&self, _option: MindOption, _organs: &StaffOrgans) -> ReasonSet {
        ReasonSet::new()
    }
}
