//! Mind organs — the state every sub-mind reads and writes.
//!
//! The organs are deliberately *not* owned by any one sub-mind. A goal
//! formed by the career mind is felt by the social mind; a memory laid
//! down by the professional mind colours a financial decision. Putting
//! the shared state in one place is what stops the sub-minds having to
//! know about each other.
//!
//! Two organs are live. [`memory`] holds what happened and what it
//! meant; [`goals`] holds what he wants about it. They are already
//! coupled in the direction that matters: what he currently wants
//! decides how deeply an event brands itself on him
//! ([`MindOrgans::relevance_for`]).
//!
//! [`journal`] is the third, and it is not an organ in the same sense —
//! nothing reads it back. It is the dated trail of what the other two
//! did, kept so a mind can be shown rather than only inspected.
//!
//! `beliefs` and `mood` follow in their own phases — see
//! `docs/player_mind.md`.

pub mod goals;
pub mod journal;
pub mod memory;

pub use goals::{
    Escalation, FormedWant, GoalBlocker, GoalBridge, GoalCensus, GoalDirection, GoalDomain,
    GoalEvidence, GoalKind, GoalMask, GoalOrigin, GoalReviewReport, GoalSpec, GoalStack,
    GoalStatus, GoalStore, MindGoal, ReasonMapping, StatusChange,
};
pub use journal::{MindJournal, MindNote, MindNoteKind, MindNoteStore};
pub use memory::{
    ActorAccount, ActorKind, ActorRef, AttributionLedger, ConsolidationReport, Consolidator,
    EncodingInputs, EpisodeDomain, EpisodeFlags, EpisodeKind, EpisodeStore, EpochDay, FactClaim,
    ForgettingCurve, FormedFact, Ledger, LedgerEntry, MemoryCensus, MemoryContext, MindClock,
    MindEpisode, MindHolder, MindMemory, Recall, RecallContext, RecallCue, RecallResult,
    RecalledEpisode, Semantic, SemanticFact, SemanticStore,
};

/// The shared state of one mind.
///
/// Held by [`PlayerMind`] and passed to every sub-mind's `observe` /
/// `reflect` / `appraise` / `weigh`.
///
/// [`PlayerMind`]: super::PlayerMind
#[derive(Debug, Clone, Copy, Default)]
pub struct MindOrgans {
    /// What he remembers, what he concluded, and where he stands with
    /// everyone who has mattered.
    pub memory: MindMemory,
    /// What he wants, and how close each want is to being said out loud.
    pub goals: GoalStack,
    /// The dated turning points the other two produced. Written by the
    /// weekly think, read by nothing inside the simulation.
    pub journal: MindJournal,
}

impl MindOrgans {
    pub fn new() -> Self {
        Self::default()
    }

    /// How much an event of this character bears on what he currently
    /// wants, 0..1 — the `relevance` term in [`EncodingInputs`].
    ///
    /// This is the coupling between the two organs, and it is the reason
    /// two players remember the same season differently. Being left out
    /// of a squad brands itself on a man whose whole ambition is
    /// first-team football; for a settled veteran it is a Tuesday.
    ///
    /// Neutral (0.5) when he wants nothing in that part of his life,
    /// which is exactly what a context-free encode should get.
    pub fn relevance_for(&self, domain: EpisodeDomain) -> f32 {
        match Self::goal_domain_for(domain) {
            Some(goal_domain) => self.goals.relevance_of(goal_domain),
            // Nothing in the goal model speaks to injuries or
            // bereavements; they land at their own intrinsic weight.
            None => 0.5,
        }
    }

    /// Write the week's turning points into the diary.
    ///
    /// One call, shared by both minds, at the end of the periodic think —
    /// the only place where both reports are in hand and the date is
    /// certain. Everything it writes was already decided; nothing here
    /// re-derives a turn from a snapshot.
    ///
    /// Want formations carry their own date, because they happen on
    /// whichever day the emit site noticed. Rung walks and convictions
    /// are dated `today`, because the review and the consolidation pass
    /// are literally when the mind reached them.
    pub fn journal_tick(
        &mut self,
        goals: Option<&GoalReviewReport>,
        consolidation: Option<&ConsolidationReport>,
        today: EpochDay,
    ) {
        if let Some(report) = goals {
            for want in report.formed() {
                self.journal.record(MindNote::want(
                    MindNoteKind::WantFormed,
                    want.kind,
                    want.day,
                ));
            }
            for change in report.changes() {
                if let Some(kind) = Self::note_for(change) {
                    self.journal
                        .record(MindNote::want(kind, change.kind, today));
                }
            }
        }

        if let Some(report) = consolidation {
            for fact in report.formed() {
                self.journal
                    .record(MindNote::conviction(fact.claim, fact.subject, today));
            }
        }
    }

    /// Which turn, if any, a rung walk is worth writing down.
    ///
    /// Climbing to [`Voiced`] is the first thing anyone outside his head
    /// can observe, so it is worth a line; *falling back* to it from a
    /// formal demand is him going quiet again, which the diary does not
    /// pretend is the same event. The two silent rungs never appear:
    /// [`Latent`] is a feeling he has not acknowledged and [`Active`] is
    /// one he acts on without saying, and a diary that announced either
    /// would be reporting something nobody — including him — could point
    /// to on the day.
    ///
    /// [`Voiced`]: GoalStatus::Voiced
    /// [`Latent`]: GoalStatus::Latent
    /// [`Active`]: GoalStatus::Active
    fn note_for(change: StatusChange) -> Option<MindNoteKind> {
        match change.to {
            GoalStatus::Voiced if change.from.rung() < GoalStatus::Voiced.rung() => {
                Some(MindNoteKind::WantVoiced)
            }
            GoalStatus::Pressing => Some(MindNoteKind::WantPressed),
            GoalStatus::Satisfied => Some(MindNoteKind::WantSatisfied),
            GoalStatus::Frustrated => Some(MindNoteKind::WantFrustrated),
            GoalStatus::Abandoned => Some(MindNoteKind::WantAbandoned),
            GoalStatus::Voiced | GoalStatus::Latent | GoalStatus::Active => None,
        }
    }

    /// Which part of what he wants an episode speaks to. `None` for the
    /// domains no goal covers.
    fn goal_domain_for(domain: EpisodeDomain) -> Option<GoalDomain> {
        match domain {
            EpisodeDomain::Career => Some(GoalDomain::Career),
            EpisodeDomain::Professional => Some(GoalDomain::Professional),
            EpisodeDomain::Competitive => Some(GoalDomain::Competitive),
            EpisodeDomain::Social => Some(GoalDomain::Social),
            EpisodeDomain::Financial => Some(GoalDomain::Financial),
            EpisodeDomain::Body | EpisodeDomain::Life => None,

            // Staff-side
            EpisodeDomain::Management => Some(GoalDomain::Management),
            EpisodeDomain::Boardroom => Some(GoalDomain::Boardroom),
            EpisodeDomain::Squad => Some(GoalDomain::Squad),
            EpisodeDomain::Philosophy => Some(GoalDomain::Philosophy),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TODAY: EpochDay = 10_000;

    #[test]
    fn an_empty_mind_finds_nothing_especially_relevant() {
        let organs = MindOrgans::new();
        for domain in [
            EpisodeDomain::Career,
            EpisodeDomain::Professional,
            EpisodeDomain::Competitive,
            EpisodeDomain::Social,
            EpisodeDomain::Financial,
            EpisodeDomain::Body,
            EpisodeDomain::Life,
        ] {
            assert_eq!(organs.relevance_for(domain), 0.5, "{domain:?}");
        }
    }

    #[test]
    fn what_he_wants_decides_what_brands_itself_on_him() {
        let mut organs = MindOrgans::new();
        let mut day = TODAY;
        for _ in 0..14 {
            day += 7;
            organs.goals.pursue(
                GoalKind::PlayFirstTeamFootball,
                GoalOrigin::Survival,
                GoalEvidence::EMPTY,
                1.0,
                day,
            );
            organs.goals.review(day);
        }

        assert!(
            organs.relevance_for(EpisodeDomain::Competitive) > 0.8,
            "minutes are all he thinks about"
        );
        assert_eq!(
            organs.relevance_for(EpisodeDomain::Financial),
            0.5,
            "and the wage is not"
        );
        assert_eq!(
            organs.relevance_for(EpisodeDomain::Body),
            0.5,
            "an injury lands at its own weight either way"
        );
    }

    #[test]
    fn the_organs_stay_copy() {
        fn assert_copy<T: Copy>() {}
        assert_copy::<MindOrgans>();
    }
}
