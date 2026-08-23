//! The organs of a manager's mind.
//!
//! Two of the three are shared with the player and live at
//! [`club::mind::organs`] — episodes, convictions, the standing account
//! with everyone who has mattered, and the goal stack with its
//! escalation ladder. None of that machinery knows or cares whose mind
//! it is in.
//!
//! The third, [`judgements`], is his alone.
//!
//! [`club::mind::organs`]: crate::club::mind::organs

pub mod judgements;

pub use judgements::{
    JudgementCensus, JudgementOutcome, JudgementStore, Judgements, PlayerJudgement,
};

use crate::club::mind::MindOrgans;
use crate::club::mind::organs::goals::{GoalDomain, GoalStack};
use crate::club::mind::organs::memory::{EpisodeDomain, MindMemory};

/// The shared state of one manager's mind.
///
/// Composition rather than a second copy of [`MindOrgans`]: the shared
/// pair is held whole, and the accessors below mean no caller has to
/// know it is nested.
#[derive(Debug, Clone, Copy, Default)]
pub struct StaffOrgans {
    /// What he remembers and what he wants — the same two organs a
    /// player has, running the same machinery.
    pub shared: MindOrgans,
    /// What he thinks of everyone he has coached.
    pub judgements: JudgementStore,
}

impl StaffOrgans {
    pub fn new() -> Self {
        Self::default()
    }

    #[inline]
    pub fn memory(&self) -> &MindMemory {
        &self.shared.memory
    }

    #[inline]
    pub fn memory_mut(&mut self) -> &mut MindMemory {
        &mut self.shared.memory
    }

    #[inline]
    pub fn goals(&self) -> &GoalStack {
        &self.shared.goals
    }

    #[inline]
    pub fn goals_mut(&mut self) -> &mut GoalStack {
        &mut self.shared.goals
    }

    /// How much an event of this character bears on what he currently
    /// wants — the coupling that decides what brands itself on him.
    /// See [`MindOrgans::relevance_for`].
    #[inline]
    pub fn relevance_for(&self, domain: EpisodeDomain) -> f32 {
        self.shared.relevance_for(domain)
    }

    /// How hard the wants in one part of his job press on him.
    #[inline]
    pub fn pressure_in(&self, domain: GoalDomain) -> f32 {
        self.shared
            .goals
            .strongest_in(domain)
            .map(|goal| goal.pressure())
            .unwrap_or(0.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_organs_stay_copy_so_cloning_a_staff_member_stays_cheap() {
        fn assert_copy<T: Copy>() {}
        assert_copy::<StaffOrgans>();
    }

    #[test]
    fn an_empty_manager_finds_nothing_especially_relevant() {
        let organs = StaffOrgans::new();
        for domain in [
            EpisodeDomain::Management,
            EpisodeDomain::Boardroom,
            EpisodeDomain::Squad,
            EpisodeDomain::Philosophy,
        ] {
            assert_eq!(organs.relevance_for(domain), 0.5, "{domain:?}");
        }
    }

    #[test]
    fn the_shared_organs_are_reached_without_naming_the_nesting() {
        let mut organs = StaffOrgans::new();
        assert!(organs.goals().is_empty());
        assert_eq!(organs.memory_mut().census().episodes, 0);
        assert_eq!(organs.pressure_in(GoalDomain::Management), 0.0);
    }
}
