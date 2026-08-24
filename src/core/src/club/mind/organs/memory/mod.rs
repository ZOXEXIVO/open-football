//! Autobiographical memory.
//!
//! Four stores and one rule: **bank the meaning before you lose the
//! evidence.**
//!
//! | Store | Cap | Decays | Holds |
//! |---|---|---|---|
//! | [`EpisodeStore`] | 32 | power law | what happened, to whom, where |
//! | [`SemanticStore`] | 24 | never | what it all meant |
//! | [`AttributionLedger`] | 32 | slow drift, floored | the standing balance with each person and club |
//! | milestones | — | never | (folded into flashbulb episodes) |
//!
//! ≈1.4 KB per player, entirely inline — no allocation at construction,
//! none on insert, and a `Player::clone` copies it rather than chasing
//! pointers.
//!
//! The design question this answers: *a player returns to a club after
//! ten years and remembers the place and the people.* He does not do it
//! by keeping ten years of events. Episodes fade on a heavy-tailed curve
//! ([`forgetting`]), [`consolidation`] distils the repeated ones into
//! convictions that never decay, the [`ledger`] carries a running
//! balance that outlives both, and [`recall`] brings the lot back when a
//! cue reaches it — rehearsing what it returns, which is why walking
//! back in makes it vivid again.
//!
//! ## Writing to memory
//!
//! Emit sites call [`MindMemory::record`]. Everything else — encoding
//! strength, flags, the ledger posting, eviction — happens here.

pub mod actor;
pub mod consolidation;
pub mod episode;
pub mod epoch;
pub mod forgetting;
pub mod ledger;
pub mod recall;
pub mod semantic;
pub mod store;

pub use actor::{ActorKind, ActorRef};
pub use consolidation::{
    ConsolidationReport, ConsolidationRule, Consolidator, EpisodeStore, FactSubject, MindHolder,
};
pub use episode::{
    EncodingInputs, EpisodeDomain, EpisodeFlags, EpisodeKind, EpisodeSpec, MindEpisode,
};
pub use epoch::{EpochDay, MindClock};
pub use forgetting::ForgettingCurve;
pub use ledger::{ActorAccount, AttributionLedger, Ledger, LedgerEntry};
pub use recall::{Recall, RecallContext, RecallCue, RecallResult, RecalledEpisode};
pub use semantic::{FactClaim, Semantic, SemanticFact, SemanticStore};
pub use store::FixedStore;

/// What a player carries with him.
#[derive(Debug, Clone, Copy, Default)]
pub struct MindMemory {
    pub episodes: EpisodeStore,
    pub semantic: SemanticStore,
    pub ledger: AttributionLedger,
    /// Last consolidation pass. Consolidation is monthly; this gates it
    /// without a separate scheduler.
    pub last_consolidated: EpochDay,
}

/// Everything a write needs to know about the man doing the
/// remembering. Built once per tick by the caller and passed to each
/// [`MindMemory::record`].
#[derive(Debug, Clone, Copy)]
pub struct MemoryContext {
    pub today: EpochDay,
    /// Whose mind this is. Decides which reading of an episode
    /// consolidation banks — see [`MindHolder`].
    pub holder: MindHolder,
    /// The club he is at right now. 0 when clubless — episodes recorded
    /// then are not club-cued.
    pub club_id: u32,
    /// Personality, 0–20.
    pub professionalism: f32,
    pub consistency: f32,
    pub temperament: f32,
}

impl MemoryContext {
    /// A neutral context for tests and for sites that have no
    /// personality to hand.
    pub fn neutral(today: EpochDay, club_id: u32) -> Self {
        MemoryContext {
            today,
            holder: MindHolder::Player,
            club_id,
            professionalism: 10.0,
            consistency: 10.0,
            temperament: 10.0,
        }
    }

    /// A neutral context for a member of staff.
    pub fn neutral_staff(today: EpochDay, club_id: u32) -> Self {
        MemoryContext {
            holder: MindHolder::Staff,
            ..Self::neutral(today, club_id)
        }
    }
}

impl MindMemory {
    /// Days between consolidation passes.
    pub const CONSOLIDATION_PERIOD_DAYS: u16 = 30;

    pub fn new() -> Self {
        Self::default()
    }

    /// Record something that happened.
    ///
    /// `encoding` carries the three factors that decide how deeply it
    /// lands — intensity from the catalog, relevance to what he
    /// currently wants, and surprise against what he expected. Sites
    /// with no goal or belief context to offer pass
    /// [`EncodingInputs::neutral`] and get the catalog anchor.
    ///
    /// `valence_override` lets a site set the sign for the episodes
    /// whose meaning genuinely depends on context (a manager leaving is
    /// a release for one player and a loss for another). `None` takes
    /// the catalog's.
    ///
    /// Also posts to the attribution ledger, which is why every emit
    /// site should name an actor when it has one — an episode with no
    /// counterparty teaches him nothing about anybody.
    pub fn record(
        &mut self,
        kind: EpisodeKind,
        who: ActorRef,
        ctx: &MemoryContext,
        encoding: EncodingInputs,
        valence_override: Option<f32>,
    ) {
        if kind == EpisodeKind::None {
            return;
        }

        let spec = kind.spec();
        let valence = valence_override.unwrap_or(spec.valence).clamp(-1.0, 1.0);
        let strength = encoding.strength();

        let episode = MindEpisode::new(kind, who, ctx.club_id, ctx.today, valence, strength);

        self.episodes.push_evicting(
            episode,
            |existing| {
                Consolidator::eviction_rank(
                    existing,
                    ctx.today,
                    ctx.professionalism,
                    ctx.consistency,
                    ctx.temperament,
                )
            },
            // Flashbulb landmarks are not displaced by ordinary traffic.
            |existing| !existing.is_flashbulb(),
        );

        Ledger::post(
            &mut self.ledger,
            who,
            LedgerEntry::from_episode(valence, strength, spec.betrayal),
            ctx.today,
        );
    }

    /// Convenience for the many sites that have no goal / belief context
    /// yet: records at the catalog's own intensity.
    pub fn record_plain(&mut self, kind: EpisodeKind, who: ActorRef, ctx: &MemoryContext) {
        let intensity = kind.spec().intensity;
        self.record(kind, who, ctx, EncodingInputs::neutral(intensity), None);
    }

    /// Run consolidation if a month has passed. Cheap no-op otherwise —
    /// safe to call every tick.
    pub fn maybe_consolidate(&mut self, ctx: &MemoryContext) -> Option<ConsolidationReport> {
        if MindClock::elapsed(self.last_consolidated, ctx.today) < Self::CONSOLIDATION_PERIOD_DAYS {
            return None;
        }
        self.last_consolidated = ctx.today;
        Some(Consolidator::run(
            &mut self.episodes,
            &mut self.semantic,
            &mut self.ledger,
            ctx.today,
            ctx.holder,
            ctx.professionalism,
            ctx.consistency,
            ctx.temperament,
        ))
    }

    /// Remember, in response to a cue. Rehearses what it returns — see
    /// [`recall`] for why that is the point.
    pub fn recall(&mut self, cue: RecallCue, ctx: &RecallContext) -> RecallResult {
        Recall::cue(&mut self.episodes, &self.semantic, &self.ledger, cue, ctx)
    }

    /// Look at what a cue *would* bring back, without bringing it back.
    ///
    /// For readers rather than for the man himself: a profile page, a
    /// scout report, the census. Reading a memory must never strengthen
    /// it, or a player who happens to be looked at often would forget
    /// nothing.
    pub fn inspect(&self, cue: RecallCue, ctx: &RecallContext) -> RecallResult {
        Recall::inspect(&self.episodes, &self.semantic, &self.ledger, cue, ctx)
    }

    /// How he feels about a club, read-only. The question the transfer
    /// path asks about every option on the table; only an actual return
    /// ([`Self::recall`]) counts as remembering.
    pub fn club_sentiment(&self, club_id: u32, ctx: &RecallContext) -> f32 {
        Recall::club_sentiment(&self.episodes, &self.semantic, &self.ledger, club_id, ctx)
    }

    /// Standing with one person, with any supporting conviction holding
    /// it against drift. This is what makes a seven-year-old grudge
    /// still bite.
    pub fn standing_with(&self, actor: ActorRef, today: EpochDay) -> f32 {
        let floor = Ledger::floor_from_sentiment(Semantic::sentiment(&self.semantic, actor));
        Ledger::standing(&self.ledger, actor, today, floor)
    }

    /// Is `claim` held about `subject`, and how firmly?
    pub fn believes(&self, claim: FactClaim, subject: ActorRef) -> f32 {
        Semantic::strength_of(&self.semantic, claim, subject)
    }

    /// Mark every episode from the club he is leaving, so "the place I
    /// used to be" is cheap to find. Called on club change.
    pub fn on_club_change(&mut self, leaving_club_id: u32) {
        if leaving_club_id == 0 {
            return;
        }
        for episode in self.episodes.iter_mut() {
            if episode.where_club == leaving_club_id {
                episode.flags.insert(EpisodeFlags::FORMER_CLUB);
            }
        }
    }

    /// Census counters for the `.dev/mind` harness and the UI.
    pub fn census(&self) -> MemoryCensus {
        MemoryCensus {
            episodes: self.episodes.len() as u16,
            flashbulbs: self.episodes.iter().filter(|e| e.is_flashbulb()).count() as u16,
            consolidated: self.episodes.iter().filter(|e| e.is_consolidated()).count() as u16,
            facts: self.semantic.len() as u16,
            accounts: self.ledger.len() as u16,
        }
    }
}

/// What a player's memory currently holds.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct MemoryCensus {
    pub episodes: u16,
    pub flashbulbs: u16,
    pub consolidated: u16,
    pub facts: u16,
    pub accounts: u16,
}

#[cfg(test)]
mod tests {
    use super::*;

    const YEAR: EpochDay = 365;

    fn ctx(today: EpochDay, club: u32) -> MemoryContext {
        MemoryContext::neutral(today, club)
    }

    fn recall_ctx(today: EpochDay) -> RecallContext {
        RecallContext {
            today,
            professionalism: 10.0,
            consistency: 10.0,
            temperament: 10.0,
            loyalty: 12.0,
            morale: 50.0,
        }
    }

    #[test]
    fn recording_writes_the_episode_and_the_account_together() {
        let mut memory = MindMemory::new();
        let coach = ActorRef::staff(412);
        memory.record_plain(EpisodeKind::ManagerPromiseBroken, coach, &ctx(100, 7));

        assert_eq!(memory.census().episodes, 1);
        assert!(
            memory.standing_with(coach, 100) < 0.0,
            "the episode and the account move together"
        );
    }

    #[test]
    fn a_betrayal_costs_trust_where_bad_news_costs_warmth() {
        let mut betrayed = MindMemory::new();
        let mut disappointed = MindMemory::new();
        let coach = ActorRef::staff(1);

        betrayed.record_plain(EpisodeKind::ManagerPromiseBroken, coach, &ctx(0, 7));
        disappointed.record_plain(EpisodeKind::ManagerPublicCriticism, coach, &ctx(0, 7));

        let b = Ledger::account(&betrayed.ledger, coach, 0, 0.0).unwrap();
        let d = Ledger::account(&disappointed.ledger, coach, 0, 0.0).unwrap();
        assert!(b.trust() < d.trust() * 2.0, "broken word is a trust event");
    }

    #[test]
    fn consolidation_only_runs_monthly() {
        let mut memory = MindMemory::new();
        memory.record_plain(EpisodeKind::SeniorDebut, ActorRef::NONE, &ctx(10, 7));

        assert!(memory.maybe_consolidate(&ctx(20, 7)).is_none(), "too soon");
        assert!(memory.maybe_consolidate(&ctx(40, 7)).is_some());
        assert!(
            memory.maybe_consolidate(&ctx(45, 7)).is_none(),
            "and not again straight away"
        );
    }

    #[test]
    fn a_full_store_never_loses_a_landmark_to_routine_traffic() {
        let mut memory = MindMemory::new();
        memory.record_plain(EpisodeKind::SeniorDebut, ActorRef::NONE, &ctx(0, 7));

        // Flood it with ordinary events.
        for day in 1..200u16 {
            memory.record_plain(EpisodeKind::MediaPraise, ActorRef::NONE, &ctx(day, 7));
        }

        assert!(
            memory
                .episodes
                .find(|e| e.kind == EpisodeKind::SeniorDebut)
                .is_some(),
            "two hundred nothing-events must not push out his debut"
        );
        assert!(memory.episodes.len() <= memory.episodes.capacity());
    }

    #[test]
    fn a_whole_career_stays_inside_the_footprint() {
        let mut memory = MindMemory::new();
        let kinds = [
            EpisodeKind::DerbyWin,
            EpisodeKind::FansHostility,
            EpisodeKind::TeammateConflict,
            EpisodeKind::ManagerPublicCriticism,
            EpisodeKind::DecisiveGoal,
            EpisodeKind::CostlyError,
        ];

        // Fifteen years of weekly incident.
        for week in 0..(52u16 * 15) {
            let day = week * 7;
            let kind = kinds[(week as usize) % kinds.len()];
            let who = match kind {
                EpisodeKind::FansHostility => ActorRef::fans(7),
                EpisodeKind::TeammateConflict => ActorRef::player(100 + (week % 11) as u32),
                EpisodeKind::ManagerPublicCriticism => ActorRef::staff(412),
                _ => ActorRef::NONE,
            };
            memory.record_plain(kind, who, &ctx(day, 7));
            memory.maybe_consolidate(&ctx(day, 7));
        }

        let census = memory.census();
        assert!(census.episodes <= 32);
        assert!(census.facts <= 24);
        assert!(census.accounts <= 32);
        assert!(
            census.facts > 0,
            "fifteen years must have taught him something"
        );
    }

    #[test]
    fn memory_is_copy_and_inline() {
        fn assert_copy<T: Copy>() {}
        assert_copy::<MindMemory>();
        assert!(
            size_of::<MindMemory>() <= 1792,
            "MindMemory grew to {} bytes — revisit the budget in docs/player_mind.md",
            size_of::<MindMemory>()
        );
    }

    #[test]
    fn the_ten_year_return() {
        // The whole system, end to end, as one story.
        let mut memory = MindMemory::new();
        let coach = ActorRef::staff(412);

        // 2026–2029 at club 7.
        memory.record_plain(EpisodeKind::SeniorDebut, ActorRef::NONE, &ctx(0, 7));
        memory.record_plain(EpisodeKind::FirstGoalForClub, ActorRef::NONE, &ctx(60, 7));
        memory.record_plain(EpisodeKind::WonLeagueTitle, ActorRef::NONE, &ctx(400, 7));
        for i in 0..5u16 {
            memory.record_plain(
                EpisodeKind::FansAdoration,
                ActorRef::fans(7),
                &ctx(420 + i * 20, 7),
            );
        }
        for i in 0..3u16 {
            memory.record_plain(
                EpisodeKind::ManagerPromiseBroken,
                coach,
                &ctx(800 + i * 40, 7),
            );
        }
        memory.record_plain(
            EpisodeKind::SoldAgainstWill,
            ActorRef::club(7),
            &ctx(1000, 7),
        );
        memory.maybe_consolidate(&ctx(1030, 7));
        memory.on_club_change(7);

        // Ten years elsewhere, with a career's worth of noise on top.
        let mut day = 1030u16;
        for week in 0..(52u16 * 10) {
            day = 1030 + week * 7;
            memory.record_plain(EpisodeKind::DerbyWin, ActorRef::NONE, &ctx(day, 9));
            memory.maybe_consolidate(&ctx(day, 9));
        }

        // He is offered a return.
        let result = memory.recall(RecallCue::Club(7), &recall_ctx(day));

        assert!(
            !result.is_empty(),
            "ten years and a career of noise later, the place must still be there"
        );
        assert!(
            memory.believes(FactClaim::BrokeThroughHere, ActorRef::club(7)) > 0.0,
            "that is where he broke through"
        );
        assert!(
            memory.believes(FactClaim::WonEverythingHere, ActorRef::club(7)) > 0.0,
            "and where he won things"
        );
        assert!(
            memory.believes(FactClaim::WasSoldAgainstMyWill, ActorRef::club(7)) > 0.0,
            "and where they sold him"
        );
        assert!(
            memory.believes(FactClaim::FansTurnedOnMe, ActorRef::club(7)) == 0.0,
            "the fans were good to him and he has not invented otherwise"
        );

        // And the man is remembered separately from the badge.
        assert!(
            memory.standing_with(coach, day) < -0.15,
            "the manager who broke his word three times is still that man, got {}",
            memory.standing_with(coach, day)
        );
        assert!(
            memory.believes(FactClaim::HisWordIsWorthless, coach) > 0.0,
            "and he knows why"
        );
    }
}
