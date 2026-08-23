//! `PlayerMind` — the global mind, and the sub-minds inside it.
//!
//! The player's psychology already exists in this codebase; it is just
//! spread across four layers that cannot see each other — mood
//! (`player/happiness/`), desire (`player/transfer/processing.rs`),
//! adaptation (`player/personality/adaptation.rs`) and relationship
//! (`player/events/`, `team/behaviour/`). Each re-derives the player's
//! priorities from raw fields, and none of them can hold an intention,
//! remember a person, or explain a decision.
//!
//! `PlayerMind` is the single owner. It mirrors the pattern already
//! proven on the staff side ([`CoachMemory`] + [`CoachDecisionEngine`]):
//! persistent per-subject memory feeding a stateless engine that returns
//! scored, *explained* verdicts.
//!
//! ## Shape
//!
//! ```text
//! PlayerMind                        the global mind
//! ├── organs: MindOrgans            shared state every faculty uses
//! │   ├── memory                    episodes · convictions · standing accounts
//! │   └── goals                     what he wants, and how loudly
//! └── faculties                     each: observe · reflect · appraise · weigh
//!     ├── career                    where he is going, and the time left
//!     ├── competitive               belief in himself, and his place in the side
//!     ├── professional              his read of the man picking the team
//!     ├── social                    whether he belongs here
//!     └── financial                 what he thinks he is worth
//! ```
//!
//! Sub-minds are concrete named fields, not `dyn` behind a registry:
//! the world tick is CPU-bound (see `simulator_parallelization_audit`)
//! and an explicit fan-out costs nothing. Each implements the same four
//! verbs — `observe`, `reflect`, `appraise`, `weigh` — so adding a
//! faculty is one file, one field and four methods.
//!
//! The organs are coupled in the direction that matters: what he
//! currently wants decides how deeply an event brands itself on him
//! ([`MindOrgans::relevance_for`]), so two players remember the same
//! season differently.
//!
//! ## Status
//!
//! Phases 1, 3 and 4 of `docs/player_mind.md` are live: memory, goals,
//! and the five faculties. Beliefs (phase 2's surprise term) and
//! deliberation (phase 5, [`SubMind::weigh`]) are still to come.
//!
//! Everything here runs **alongside** the four legacy layers rather than
//! replacing them. `PlayerHappiness` still owns morale and
//! `process_transfer_desire` still owns `Req`; the mind accumulates in
//! parallel so each swap-over can be proven on a real corpus before it
//! is made, rather than invalidating a thousand calibrated tests at once.
//!
//! [`CoachMemory`]: crate::club::staff::coach::CoachMemory
//! [`CoachDecisionEngine`]: crate::club::staff::coach::CoachDecisionEngine

pub mod career;
pub mod competitive;
pub mod financial;
#[cfg(test)]
mod integration;
// The organs are shared with `StaffMind` and live at `club::mind::organs`.
// Re-exported here so every `club::player::mind::organs::…` path — and
// every `super::organs::…` inside a faculty — keeps working unchanged.
pub use crate::club::mind::organs;
pub mod professional;
pub mod situation;
pub mod social;
pub mod submind;

pub use career::{CareerMind, CareerStage};
pub use competitive::CompetitiveMind;
pub use financial::FinancialMind;
pub use professional::ProfessionalMind;
pub use situation::MindSituation;
pub use social::SocialMind;
pub use submind::{MindOption, MindView, MoodContribution, ReasonSet, SubMind, WeightedReason};

pub use organs::MindOrgans;
pub use organs::goals::{
    Escalation, GoalBlocker, GoalBridge, GoalCensus, GoalDirection, GoalEvidence, GoalKind,
    GoalMask, GoalOrigin, GoalReviewReport, GoalSpec, GoalStack, GoalStatus, MindGoal,
    ReasonMapping, StatusChange,
};
// `GoalDomain` is re-exported from the goals organ, but the name reads
// better unqualified alongside `EpisodeDomain`.
pub use organs::goals::GoalDomain;
pub use organs::memory::{
    ActorAccount, ActorKind, ActorRef, AttributionLedger, ConsolidationReport, Consolidator,
    EncodingInputs, EpisodeDomain, EpisodeFlags, EpisodeKind, EpisodeStore, EpochDay, FactClaim,
    ForgettingCurve, Ledger, LedgerEntry, MemoryCensus, MemoryContext, MindClock, MindEpisode,
    MindHolder, MindMemory, Recall, RecallContext, RecallCue, RecallResult, RecalledEpisode,
    Semantic, SemanticFact, SemanticStore,
};

use crate::club::person::PersonAttributes;
use chrono::NaiveDate;
use std::cmp::Ordering;

/// Everything a mind tick needs from the world, gathered once by the
/// caller so the mind never walks the simulator graph. Mirrors the
/// `TransferDesireContext` convention already used by the desire path.
#[derive(Debug, Clone, Copy)]
pub struct MindTickContext {
    pub today: NaiveDate,
    /// The club he is at. 0 when clubless.
    pub club_id: u32,
    /// Personality, 0–20.
    pub professionalism: f32,
    pub consistency: f32,
    pub temperament: f32,
    pub loyalty: f32,
    /// Current morale, 0–100. Drives mood-congruent recall.
    pub morale: f32,
}

impl MindTickContext {
    pub fn new(today: NaiveDate, club_id: u32, attributes: &PersonAttributes, morale: f32) -> Self {
        MindTickContext {
            today,
            club_id,
            professionalism: attributes.professionalism,
            consistency: attributes.consistency,
            temperament: attributes.temperament,
            loyalty: attributes.loyalty,
            morale,
        }
    }

    /// The write-side view, for [`MindMemory::record`].
    pub fn memory(&self) -> MemoryContext {
        MemoryContext {
            today: MindClock::day(self.today),
            holder: MindHolder::Player,
            club_id: self.club_id,
            professionalism: self.professionalism,
            consistency: self.consistency,
            temperament: self.temperament,
        }
    }

    /// The read-side view, for [`MindMemory::recall`].
    pub fn recall(&self) -> RecallContext {
        RecallContext {
            today: MindClock::day(self.today),
            professionalism: self.professionalism,
            consistency: self.consistency,
            temperament: self.temperament,
            loyalty: self.loyalty,
            morale: self.morale,
        }
    }
}

/// The global mind: shared organs, and the five faculties that reason
/// over them.
///
/// Sub-minds are concrete named fields rather than `dyn SubMind` behind
/// a registry. The world tick is CPU-bound (see
/// `simulator_parallelization_audit`), the set is known at compile time,
/// and an explicit fan-out costs nothing — while dynamic dispatch would
/// cost an indirect call per faculty per player per week for no gain.
#[derive(Debug, Clone, Copy, Default)]
pub struct PlayerMind {
    pub organs: MindOrgans,

    /// Where he is going, and how much time is left to get there.
    pub career: CareerMind,
    /// His belief in himself as a player, and his standing in the side.
    pub competitive: CompetitiveMind,
    /// His read of the man picking the team.
    pub professional: ProfessionalMind,
    /// Whether he belongs here.
    pub social: SocialMind,
    /// What he thinks he is worth.
    pub financial: FinancialMind,
}

impl PlayerMind {
    pub fn new() -> Self {
        Self::default()
    }

    /// Route an episode to the faculty that cares about it.
    ///
    /// Every episode reaches exactly one sub-mind, chosen by its
    /// domain. Two domains have no faculty of their own and are routed
    /// where they actually land: the body speaks to a player's belief in
    /// himself, and life outside the game speaks to whether he feels at
    /// home.
    fn dispatch(&mut self, episode: &MindEpisode) {
        match episode.kind.spec().domain {
            EpisodeDomain::Career => self.career.observe(episode, &mut self.organs),
            EpisodeDomain::Professional => self.professional.observe(episode, &mut self.organs),
            EpisodeDomain::Competitive | EpisodeDomain::Body => {
                self.competitive.observe(episode, &mut self.organs)
            }
            EpisodeDomain::Social | EpisodeDomain::Life => {
                self.social.observe(episode, &mut self.organs)
            }
            EpisodeDomain::Financial => self.financial.observe(episode, &mut self.organs),

            // The staff-side domains never reach a player's mind — no
            // emit site records a manager episode against a player —
            // but the catalog is shared, so the arm has to exist.
            // Career is where one would land if it ever did.
            EpisodeDomain::Management
            | EpisodeDomain::Boardroom
            | EpisodeDomain::Squad
            | EpisodeDomain::Philosophy => self.career.observe(episode, &mut self.organs),
        }
    }

    /// Let every faculty think, in turn.
    ///
    /// Order matters in one place and one place only: the social mind
    /// runs last, so the wants that keep a player somewhere are formed
    /// against a stack that already contains the ones pulling him away —
    /// and the competition rules resolve them in the same review rather
    /// than a week later.
    fn reflect_all(&mut self, view: &MindView<'_>) {
        self.career.reflect(view, &mut self.organs);
        self.competitive.reflect(view, &mut self.organs);
        self.professional.reflect(view, &mut self.organs);
        self.financial.reflect(view, &mut self.organs);
        self.social.reflect(view, &mut self.organs);
    }

    /// How he feels, as the five faculties see it.
    ///
    /// Runs alongside `PlayerHappiness` rather than replacing it — see
    /// [`MoodProfile`] for what that parallel run is for.
    pub fn appraise(&self) -> MoodProfile {
        MoodProfile {
            career: self.career.appraise(&self.organs),
            competitive: self.competitive.appraise(&self.organs),
            professional: self.professional.appraise(&self.organs),
            social: self.social.appraise(&self.organs),
            financial: self.financial.appraise(&self.organs),
        }
    }

    /// Borrow what he remembers.
    #[inline]
    pub fn memory(&self) -> &MindMemory {
        &self.organs.memory
    }

    #[inline]
    pub fn memory_mut(&mut self) -> &mut MindMemory {
        &mut self.organs.memory
    }

    /// Borrow what he wants.
    #[inline]
    pub fn goals(&self) -> &GoalStack {
        &self.organs.goals
    }

    #[inline]
    pub fn goals_mut(&mut self) -> &mut GoalStack {
        &mut self.organs.goals
    }

    /// Record something that happened.
    ///
    /// The entry point emit sites use. Encoding takes its intensity from
    /// the catalog and its **relevance from what he currently wants** —
    /// so the same event brands itself on one player and passes another
    /// by. Surprise stays neutral until beliefs land (phase 2), which is
    /// the honest reading: without an expectation there is nothing to be
    /// surprised against.
    pub fn remember(&mut self, kind: EpisodeKind, who: ActorRef, ctx: &MindTickContext) {
        let spec = kind.spec();
        let encoding = EncodingInputs {
            intensity: spec.intensity,
            relevance: self.organs.relevance_for(spec.domain),
            surprise: 0.5,
        };
        let memory_ctx = ctx.memory();
        self.organs
            .memory
            .record(kind, who, &memory_ctx, encoding, None);

        // The faculty that cares hears about it now, not on its next
        // think. A player who was dropped this morning does not carry on
        // as though he was not.
        let episode = MindEpisode::new(
            kind,
            who,
            memory_ctx.club_id,
            memory_ctx.today,
            spec.valence,
            encoding.strength(),
        );
        self.dispatch(&episode);
    }

    /// Record with explicit encoding — for sites that already know how
    /// much this mattered to him, or how far it diverged from what he
    /// expected.
    pub fn remember_with(
        &mut self,
        kind: EpisodeKind,
        who: ActorRef,
        ctx: &MindTickContext,
        encoding: EncodingInputs,
        valence_override: Option<f32>,
    ) {
        self.organs
            .memory
            .record(kind, who, &ctx.memory(), encoding, valence_override);
    }

    /// Bring back what a cue reaches. Rehearses what it returns.
    pub fn recall(&mut self, cue: RecallCue, ctx: &MindTickContext) -> RecallResult {
        self.organs.memory.recall(cue, &ctx.recall())
    }

    /// How he feels about a club, read-only — the question asked of
    /// every option on a transfer shortlist. Only an actual
    /// [`Self::recall`] counts as remembering.
    pub fn club_sentiment(&self, club_id: u32, ctx: &MindTickContext) -> f32 {
        self.organs.memory.club_sentiment(club_id, &ctx.recall())
    }

    /// Where he stands with one person, with any supporting conviction
    /// holding it against the years.
    pub fn standing_with(&self, actor: ActorRef, ctx: &MindTickContext) -> f32 {
        self.organs
            .memory
            .standing_with(actor, MindClock::day(ctx.today))
    }

    /// Want something, or want it more. The single write the whole
    /// simulation uses to feed the goal stack.
    ///
    /// Returns `true` if the want is new.
    pub fn pursue(
        &mut self,
        kind: GoalKind,
        origin: GoalOrigin,
        evidence: GoalEvidence,
        amount: f32,
        ctx: &MindTickContext,
    ) -> bool {
        self.organs
            .goals
            .pursue(kind, origin, evidence, amount, MindClock::day(ctx.today))
    }

    /// Move a want toward being satisfied.
    pub fn advance(&mut self, kind: GoalKind, amount: f32) {
        self.organs.goals.advance(kind, amount);
    }

    /// How hard a want presses, 0..1. `0.0` when he does not hold it.
    #[inline]
    pub fn pressure_of(&self, kind: GoalKind) -> f32 {
        self.organs.goals.pressure_of(kind)
    }

    /// Net pull away from the club, 0..1 — including the wants he has
    /// never said out loud. This is what
    /// [`Player::big_stage_inclination`] does for one want, generalised
    /// to all of them.
    ///
    /// [`Player::big_stage_inclination`]: crate::Player::big_stage_inclination
    #[inline]
    pub fn wants_to_leave(&self) -> f32 {
        self.organs.goals.wants_to_leave()
    }

    /// Has he formally demanded anything?
    #[inline]
    pub fn is_pressing(&self) -> bool {
        self.organs.goals.is_pressing()
    }

    /// The want that dominates him — what the narrative layers lead with.
    #[inline]
    pub fn strongest_goal(&self) -> Option<&MindGoal> {
        self.organs.goals.strongest()
    }

    /// The periodic think. Called from `Player::simulate`; cheap enough
    /// to call every tick, and a no-op on the days it has nothing to do.
    ///
    /// Two passes on their own cadences: the weekly goal review (decay,
    /// competition, the escalation ladder) and the monthly consolidation
    /// that banks what recent episodes meant before they fade. Sub-mind
    /// `reflect` joins them from phase 4.
    ///
    /// Goals are reviewed *before* consolidation, so an episode recorded
    /// later in the same tick is encoded against what he wants today
    /// rather than what he wanted last week.
    /// The periodic think, without a situation: the goal stack is
    /// reviewed and consolidation runs, but **the faculties do not
    /// reflect.**
    ///
    /// That omission is deliberate. A neutral situation is not a neutral
    /// input — to the competitive mind it reads as a player getting the
    /// minutes his role implies, which would quietly *satisfy* the very
    /// wants a caller with no situation to offer knows nothing about.
    /// No situation, no thinking.
    pub fn tick(&mut self, ctx: &MindTickContext) -> MindTickReport {
        self.run_tick(ctx, None)
    }

    /// The full think: the five faculties reflect on where he actually
    /// is, then the goal stack is reviewed and consolidation runs.
    pub fn tick_with(
        &mut self,
        ctx: &MindTickContext,
        situation: &MindSituation,
    ) -> MindTickReport {
        self.run_tick(ctx, Some(situation))
    }

    fn run_tick(
        &mut self,
        ctx: &MindTickContext,
        situation: Option<&MindSituation>,
    ) -> MindTickReport {
        let today = MindClock::day(ctx.today);

        // The faculties think first, so a want formed this morning is on
        // the stack before the review that decides whether he says it —
        // and before consolidation encodes anything against it.
        if let Some(situation) = situation {
            let view = MindView {
                tick: ctx,
                situation,
            };
            self.reflect_all(&view);
        }

        let goals = self.organs.goals.review(today);
        let consolidation = self.organs.memory.maybe_consolidate(&ctx.memory());
        MindTickReport {
            goals,
            consolidation,
        }
    }

    /// Called when the player changes club.
    ///
    /// The two organs answer this differently, and the difference is the
    /// design. **Memory keeps everything** — a career is the one thing a
    /// player carries between clubs, and every other per-club field on
    /// `Player` resets (`reset_on_club_change`). **Goals resolve**: what
    /// he wanted *out of* is answered by the move, what he wanted *at*
    /// the old club is moot, and what he wants for himself travels with
    /// him.
    pub fn on_club_change(&mut self, leaving_club_id: u32) {
        self.organs.memory.on_club_change(leaving_club_id);
        self.organs.goals.on_club_change();

        // Belonging is about a place and does not travel; the read of a
        // manager is about a person and is reset when it becomes someone
        // else. The career and financial faculties carry over untouched
        // — a career is continuous, and being underpaid is not settled
        // by changing employer.
        self.social.on_club_change();
        self.professional.on_manager_change(ActorRef::NONE);
    }

    /// Census for the `.dev/mind` harness and the player profile UI.
    pub fn census(&self) -> MemoryCensus {
        self.organs.memory.census()
    }

    /// What he currently wants, by rung.
    pub fn goal_census(&self) -> GoalCensus {
        self.organs.goals.census()
    }
}

/// How the five faculties read a player's mood.
///
/// **This runs alongside `PlayerHappiness`, not instead of it.** Morale
/// is the single most heavily calibrated number in the simulation — 13
/// factors, 204 event types, and roughly a thousand tests that depend on
/// where it lands. Swapping its inputs out in one step would invalidate
/// a large slice of that at once, so phase 4 does what phase 3 did with
/// goals: build the replacement, run it in parallel, and prove
/// agreement before anything changes hands.
///
/// What can honestly be claimed today is **directional** agreement — a
/// faculty that reads a player as unhappy corresponds to a negative
/// factor on the same axis. Full numeric parity with
/// `happiness/processing.rs` is phase 4b, and it needs a population-level
/// census rather than unit tests.
#[derive(Debug, Clone, Copy)]
pub struct MoodProfile {
    pub career: MoodContribution,
    pub competitive: MoodContribution,
    pub professional: MoodContribution,
    pub social: MoodContribution,
    pub financial: MoodContribution,
}

impl MoodProfile {
    /// Every contribution, in fan-out order.
    pub fn contributions(&self) -> [MoodContribution; 5] {
        [
            self.career,
            self.competitive,
            self.professional,
            self.social,
            self.financial,
        ]
    }

    /// Net read of how he feels, on the same rough scale as the existing
    /// morale factors. Confidence-weighted, so a faculty with nothing to
    /// go on does not dilute the ones that have.
    pub fn net(&self) -> f32 {
        self.contributions().iter().map(|c| c.weighted()).sum()
    }

    /// How much of him is actually being read. Low on a young player at
    /// a new club with no history — which is correct, and is the thing a
    /// flat "morale = 50" cannot express.
    pub fn coverage(&self) -> f32 {
        let contributions = self.contributions();
        let total: f32 = contributions.iter().map(|c| c.confidence).sum();
        total / contributions.len() as f32
    }

    /// The faculty weighing on him most, if any is.
    pub fn heaviest_concern(&self) -> Option<MoodContribution> {
        self.contributions()
            .into_iter()
            .filter(|c| !c.is_silent() && c.weighted() < 0.0)
            .min_by(|a, b| {
                a.weighted()
                    .partial_cmp(&b.weighted())
                    .unwrap_or(Ordering::Equal)
            })
    }

    /// The same reading on the 0..100 scale `PlayerHappiness::morale`
    /// uses, so the parallel run can be compared without a conversion at
    /// every call site. 50 is neutral.
    ///
    /// One-to-one, and that is not arbitrary: [`Self::net`] is a **sum**
    /// of five contributions each bounded at ±10, so its own range is
    /// ±50 and `50 + net` covers exactly 0..100. An earlier ×2.5 here
    /// read `net` as a mean and saturated the whole staff-side parity
    /// check against the ceiling — see `docs/staff_mind.md` §12.
    ///
    /// **Not a replacement for morale.** It is the left-hand side of the
    /// phase-4b parity check and nothing reads it in the live sim; see
    /// [`MoodProfile`] for why the swap is gated on a population census.
    pub fn as_morale(&self) -> f32 {
        (50.0 + self.net()).clamp(0.0, 100.0)
    }
}

/// What one mind tick did.
#[derive(Debug, Clone, Copy, Default)]
pub struct MindTickReport {
    /// `Some` on the ticks the weekly goal review actually ran.
    pub goals: Option<GoalReviewReport>,
    /// `Some` on the ticks consolidation actually ran.
    pub consolidation: Option<ConsolidationReport>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn attrs() -> PersonAttributes {
        PersonAttributes {
            adaptability: 12.0,
            ambition: 12.0,
            controversy: 5.0,
            loyalty: 12.0,
            pressure: 12.0,
            professionalism: 12.0,
            sportsmanship: 12.0,
            temperament: 10.0,
            consistency: 12.0,
            important_matches: 12.0,
            dirtiness: 5.0,
        }
    }

    fn d(y: i32, m: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, day).unwrap()
    }

    fn ctx(date: NaiveDate, club: u32) -> MindTickContext {
        MindTickContext::new(date, club, &attrs(), 50.0)
    }

    #[test]
    fn a_fresh_mind_is_empty_and_ticks_quietly() {
        let mut mind = PlayerMind::new();
        let report = mind.tick(&ctx(d(2026, 8, 23), 7));
        assert_eq!(mind.census(), MemoryCensus::default());
        // First tick consolidates (last_consolidated starts at day 0, so
        // any in-sim date is a month past it) and finds nothing to do.
        assert!(report.consolidation.is_some());
        assert_eq!(
            report.consolidation.unwrap(),
            ConsolidationReport::default()
        );
    }

    #[test]
    fn remembering_reaches_the_memory_organ() {
        let mut mind = PlayerMind::new();
        let c = ctx(d(2026, 8, 23), 7);
        mind.remember(EpisodeKind::SeniorDebut, ActorRef::NONE, &c);
        assert_eq!(mind.census().episodes, 1);
        assert_eq!(mind.census().flashbulbs, 1);
    }

    #[test]
    fn a_move_never_clears_what_he_remembers() {
        let mut mind = PlayerMind::new();
        let c = ctx(d(2026, 8, 23), 7);
        mind.remember(EpisodeKind::WonLeagueTitle, ActorRef::NONE, &c);
        let before = mind.census();

        mind.on_club_change(7);

        assert_eq!(
            mind.census(),
            before,
            "memory is the one thing a player takes with him"
        );
    }

    #[test]
    fn club_sentiment_is_available_without_a_recall() {
        let mut mind = PlayerMind::new();
        let c = ctx(d(2026, 8, 23), 7);
        mind.remember(EpisodeKind::SeniorDebut, ActorRef::NONE, &c);
        assert!(mind.club_sentiment(7, &c) > 0.0);
        assert_eq!(mind.club_sentiment(99, &c), 0.0);
    }

    #[test]
    fn the_mind_stays_copy_so_cloning_a_player_stays_cheap() {
        fn assert_copy<T: Copy>() {}
        assert_copy::<PlayerMind>();
    }
}
