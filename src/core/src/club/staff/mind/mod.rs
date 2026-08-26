//! `StaffMind` — the global mind for the people in the dugout.
//!
//! The staff side started from the opposite position to the player. The
//! player had a rich mood layer and no memory of anyone; the staff has a
//! real, working **memory of players** (`CoachMemory`) and a **decision
//! engine** (`CoachDecisionEngine`) — the pattern the player plan cited
//! as already proven in this repo — and almost nothing else. A manager
//! could tell you what he thought of every player he had coached, and
//! nothing at all about the club that sacked him.
//!
//! ## Shape
//!
//! ```text
//! StaffMind                          the global mind
//! ├── organs: StaffOrgans            shared state every faculty uses
//! │   ├── memory                     episodes · convictions · standing accounts
//! │   ├── goals                      what he wants, and how loudly
//! │   └── judgements                 his read of every player — staff-only
//! └── faculties                      each: observe · reflect · appraise · weigh
//!     ├── ambition                   where his career is going, and whether this job survives
//!     ├── authority                  his standing with the board, the room and the stands
//!     ├── judgement                  his read of players — and whether it was right
//!     ├── philosophy                 how he believes football should be played
//!     └── welfare                    the workload, and whether he still wants it
//! ```
//!
//! Memory and goals are literally the player's organs, not a copy of
//! them — they live at [`club::mind::organs`] and both minds run the
//! same machinery over the same catalogs. `EpisodeKind`, `FactClaim` and
//! `GoalKind` carry manager rows alongside the player rows, and
//! `ActorRef` points both ways: the player's memory of a coach and the
//! coach's memory of that player use the same key type.
//!
//! ## What is genuinely different
//!
//! **He looks up as well as down.** A player reads one manager; a
//! manager reads a board above him, supporters around him and thirty
//! players below. [`AuthorityMind`] models three standings.
//!
//! **He has an identity.** [`PhilosophyMind`] has no player equivalent,
//! and it is what finally gives the board's `style_alignment` a
//! counterparty.
//!
//! **He can be wrong about people.** [`JudgementMind`] is the only
//! faculty in either mind where a verdict is scored against what
//! actually happened.
//!
//! ## Status
//!
//! Most of this runs **alongside** the existing staff layers rather
//! than replacing them, exactly as the player mind runs alongside
//! `PlayerHappiness`. `job_satisfaction` still owns staff morale and
//! `CoachDecisionEngine` is not replaced by any of it — see
//! `docs/staff_mind.md` for what each phase does and does not switch
//! over.
//!
//! One thing is not parallel-run. Both of a coach's per-player stores
//! now live in [`organs::judgements`], on the man: `CoachMemory` (which
//! selection reads) and `CoachDecisionState` (which squad composition
//! and the recruitment budget read). That move is guarded by a pinned
//! before/after census rather than by a parallel run, because there is
//! no sensible way to run two homes for one store at once.
//!
//! [`organs::judgements`]: organs::judgements
//!
//! [`club::mind::organs`]: crate::club::mind::organs

pub mod ambition;
pub mod authority;
#[cfg(test)]
mod integration;
pub mod judgement;
pub mod organs;
pub mod philosophy;
pub mod situation;
pub mod submind;
pub mod welfare;

pub use ambition::AmbitionMind;
pub use authority::AuthorityMind;
pub use judgement::JudgementMind;
pub use organs::{
    JudgementCensus, JudgementOutcome, JudgementStore, Judgements, PlayerJudgement, StaffOrgans,
};
pub use philosophy::PhilosophyMind;
pub use situation::StaffSituation;
pub use submind::{
    MindOption, MoodContribution, ReasonSet, StaffSubMind, StaffView, WeightedReason,
};
pub use welfare::WelfareMind;

use crate::club::mind::organs::goals::{
    GoalCensus, GoalDomain, GoalEvidence, GoalKind, GoalOrigin, GoalReviewReport, GoalStack,
    MindGoal,
};
use crate::club::mind::organs::memory::{
    ActorRef, ConsolidationReport, EncodingInputs, EpisodeKind, EpochDay, FactClaim, MemoryCensus,
    MemoryContext, MindClock, MindEpisode, MindHolder, MindMemory, RecallContext, RecallCue,
    RecallResult,
};
use crate::club::person::PersonAttributes;
use chrono::NaiveDate;
use std::cmp::Ordering;

/// Everything a mind tick needs from the world, gathered once by the
/// caller so the mind never walks the simulator graph.
#[derive(Debug, Clone, Copy)]
pub struct StaffTickContext {
    pub today: NaiveDate,
    /// The club he works for. 0 when out of work — and a manager
    /// between jobs is a real state, not a missing one.
    pub club_id: u32,
    /// Personality, 0–20.
    pub professionalism: f32,
    pub consistency: f32,
    pub temperament: f32,
    pub loyalty: f32,
    /// Current `job_satisfaction`, 0–100. Drives mood-congruent recall
    /// the same way morale does for a player.
    pub satisfaction: f32,
}

impl StaffTickContext {
    pub fn new(
        today: NaiveDate,
        club_id: u32,
        attributes: &PersonAttributes,
        satisfaction: f32,
    ) -> Self {
        StaffTickContext {
            today,
            club_id,
            professionalism: attributes.professionalism,
            consistency: attributes.consistency,
            temperament: attributes.temperament,
            loyalty: attributes.loyalty,
            satisfaction,
        }
    }

    /// The write-side view, for [`MindMemory::record`]. Tagged
    /// [`MindHolder::Staff`], which is what decides that a title won
    /// here consolidates to "I built something there" rather than to
    /// "I won everything here".
    pub fn memory(&self) -> MemoryContext {
        MemoryContext {
            today: MindClock::day(self.today),
            holder: MindHolder::Staff,
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
            morale: self.satisfaction,
        }
    }

    #[inline]
    pub fn day(&self) -> EpochDay {
        MindClock::day(self.today)
    }
}

/// The global mind: shared organs, one organ of its own, and the five
/// faculties that reason over them.
#[derive(Debug, Clone, Copy, Default)]
pub struct StaffMind {
    pub organs: StaffOrgans,

    /// Where his career is going, and whether this job survives.
    pub ambition: AmbitionMind,
    /// His standing with the board, the room and the stands.
    pub authority: AuthorityMind,
    /// His read of the people he picks.
    pub judgement: JudgementMind,
    /// His football.
    pub philosophy: PhilosophyMind,
    /// What the job is costing him.
    pub welfare: WelfareMind,
}

impl StaffMind {
    pub fn new() -> Self {
        Self::default()
    }

    /// Let every faculty interpret what happened.
    ///
    /// This is the one structural place the staff mind departs from the
    /// player mind, which routes each episode to exactly one faculty by
    /// domain. A manager's events are institutional, and the same event
    /// genuinely lands on more than one of him: a public vote of
    /// confidence is a fact about the board *and* about whether the job
    /// survives; being sacked ends a career chapter *and* takes the load
    /// off; a title confirms his football, adds to his honours, and
    /// makes him want another season.
    ///
    /// Every `observe` is written as an opt-in match over the variants
    /// that faculty cares about, so the fan-out costs five cheap
    /// dispatches and nothing is counted twice.
    fn dispatch(&mut self, episode: &MindEpisode) {
        self.ambition.observe(episode, &mut self.organs);
        self.authority.observe(episode, &mut self.organs);
        self.judgement.observe(episode, &mut self.organs);
        self.philosophy.observe(episode, &mut self.organs);
        self.welfare.observe(episode, &mut self.organs);
    }

    /// Let every faculty think, in turn.
    ///
    /// Order matters in one place: ambition runs first, so the reading
    /// of whether the job survives is current before the faculties that
    /// take their cue from a crisis — philosophy bending under pressure,
    /// welfare accruing strain — form anything against it.
    fn reflect_all(&mut self, view: &StaffView<'_>) {
        self.ambition.reflect(view, &mut self.organs);
        self.authority.reflect(view, &mut self.organs);
        self.judgement.reflect(view, &mut self.organs);
        self.philosophy.reflect(view, &mut self.organs);
        self.welfare.reflect(view, &mut self.organs);
    }

    /// How he feels about the job, as the five faculties see it.
    ///
    /// **Runs alongside `job_satisfaction`, not instead of it.** Phase
    /// S4 of `docs/staff_mind.md` does what phase 4 did on the player
    /// side: build the replacement, run it in parallel, and prove
    /// agreement before anything changes hands.
    pub fn appraise(&self) -> StaffMoodProfile {
        StaffMoodProfile {
            ambition: self.ambition.appraise(&self.organs),
            authority: self.authority.appraise(&self.organs),
            judgement: self.judgement.appraise(&self.organs),
            philosophy: self.philosophy.appraise(&self.organs),
            welfare: self.welfare.appraise(&self.organs),
        }
    }

    /// Every faculty's opinion on a decision, merged.
    ///
    /// The five are asked in fan-out order and their reasons folded
    /// together; a faculty with no view on this option contributes
    /// nothing rather than a neutral zero.
    pub fn deliberate(&self, option: MindOption) -> ReasonSet {
        let mut reasons = self.ambition.weigh(option, &self.organs);
        reasons.absorb(&self.authority.weigh(option, &self.organs));
        reasons.absorb(&self.judgement.weigh(option, &self.organs));
        reasons.absorb(&self.philosophy.weigh(option, &self.organs));
        reasons.absorb(&self.welfare.weigh(option, &self.organs));
        reasons
    }

    // ── Memory ──────────────────────────────────────────────────

    #[inline]
    pub fn memory(&self) -> &MindMemory {
        self.organs.memory()
    }

    #[inline]
    pub fn memory_mut(&mut self) -> &mut MindMemory {
        self.organs.memory_mut()
    }

    /// Record something that happened. The entry point emit sites use.
    pub fn remember(&mut self, kind: EpisodeKind, who: ActorRef, ctx: &StaffTickContext) {
        let spec = kind.spec();
        let encoding = EncodingInputs {
            intensity: spec.intensity,
            relevance: self.organs.relevance_for(spec.domain),
            surprise: 0.5,
        };
        let memory_ctx = ctx.memory();
        self.organs
            .shared
            .memory
            .record(kind, who, &memory_ctx, encoding, None);

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
    /// much this mattered to him.
    pub fn remember_with(
        &mut self,
        kind: EpisodeKind,
        who: ActorRef,
        ctx: &StaffTickContext,
        encoding: EncodingInputs,
        valence_override: Option<f32>,
    ) {
        self.organs
            .shared
            .memory
            .record(kind, who, &ctx.memory(), encoding, valence_override);
    }

    /// Bring back what a cue reaches. Rehearses what it returns.
    pub fn recall(&mut self, cue: RecallCue, ctx: &StaffTickContext) -> RecallResult {
        self.organs.shared.memory.recall(cue, &ctx.recall())
    }

    /// How he feels about a club, read-only. The question asked of every
    /// job on the table — and the one `candidate_accepts_terms` cannot
    /// ask today.
    pub fn club_sentiment(&self, club_id: u32, ctx: &StaffTickContext) -> f32 {
        self.organs
            .shared
            .memory
            .club_sentiment(club_id, &ctx.recall())
    }

    /// Where he stands with one person or institution.
    pub fn standing_with(&self, actor: ActorRef, ctx: &StaffTickContext) -> f32 {
        self.organs.shared.memory.standing_with(actor, ctx.day())
    }

    /// Is `claim` held about `subject`, and how firmly?
    #[inline]
    pub fn believes(&self, claim: FactClaim, subject: ActorRef) -> f32 {
        self.organs.shared.memory.believes(claim, subject)
    }

    // ── Goals ───────────────────────────────────────────────────

    #[inline]
    pub fn goals(&self) -> &GoalStack {
        self.organs.goals()
    }

    #[inline]
    pub fn goals_mut(&mut self) -> &mut GoalStack {
        self.organs.goals_mut()
    }

    /// Want something, or want it more.
    pub fn pursue(
        &mut self,
        kind: GoalKind,
        origin: GoalOrigin,
        evidence: GoalEvidence,
        amount: f32,
        ctx: &StaffTickContext,
    ) -> bool {
        self.organs
            .shared
            .goals
            .pursue(kind, origin, evidence, amount, ctx.day())
    }

    #[inline]
    pub fn pressure_of(&self, kind: GoalKind) -> f32 {
        self.organs.shared.goals.pressure_of(kind)
    }

    /// Net pull out of this job, 0..1 — including the wants he has never
    /// said out loud.
    #[inline]
    pub fn wants_out(&self) -> f32 {
        self.organs.shared.goals.wants_to_leave()
    }

    /// Has he formally said any of it?
    #[inline]
    pub fn is_pressing(&self) -> bool {
        self.organs.shared.goals.is_pressing()
    }

    #[inline]
    pub fn strongest_goal(&self) -> Option<&MindGoal> {
        self.organs.shared.goals.strongest()
    }

    // ── Judgements ──────────────────────────────────────────────

    /// What he makes of a player he is starting to work with. Idempotent
    /// — a second look is not a second player.
    pub fn form_judgement(
        &mut self,
        player: ActorRef,
        level: f32,
        ceiling: f32,
        ctx: &StaffTickContext,
    ) {
        Judgements::form(
            &mut self.organs.judgements,
            player,
            level,
            ceiling,
            ctx.day(),
        );
    }

    /// He watched him play.
    pub fn watched(
        &mut self,
        player: ActorRef,
        rating: f32,
        big_match: bool,
        ctx: &StaffTickContext,
    ) {
        Judgements::watched(
            &mut self.organs.judgements,
            player,
            rating,
            big_match,
            ctx.day(),
        );
    }

    #[inline]
    pub fn judgement_of(&self, player: ActorRef) -> Option<&PlayerJudgement> {
        Judgements::of(&self.organs.judgements, player)
    }

    /// A player's career has answered a question he had a view on.
    ///
    /// This is the loop-closer: a coach who wrote off a player who then
    /// became very good learns something from it, which is what makes
    /// his eye improve over a career rather than staying at whatever
    /// `CoachProfile` seeded.
    pub fn settle_judgement(
        &mut self,
        player: ActorRef,
        true_level: f32,
        ctx: &StaffTickContext,
    ) -> Option<JudgementOutcome> {
        self.judgement
            .settle(&mut self.organs, player, true_level, ctx.day())
    }

    // ── The tick ────────────────────────────────────────────────

    /// The periodic think, without a situation: the goal stack is
    /// reviewed and consolidation runs, but **the faculties do not
    /// reflect.**
    ///
    /// That omission is deliberate and is inherited from the player
    /// side, where it was a real defect. A neutral situation is not a
    /// neutral input — to the ambition mind it reads as a manager
    /// sitting mid-table with a board that half-trusts him, which would
    /// quietly resolve the very wants a caller with no situation to
    /// offer knows nothing about. No situation, no thinking.
    pub fn tick(&mut self, ctx: &StaffTickContext) -> StaffMindReport {
        self.run_tick(ctx, None)
    }

    /// The full think: the five faculties reflect on where he actually
    /// is, then the goal stack is reviewed and consolidation runs.
    pub fn tick_with(
        &mut self,
        ctx: &StaffTickContext,
        situation: &StaffSituation,
    ) -> StaffMindReport {
        self.run_tick(ctx, Some(situation))
    }

    fn run_tick(
        &mut self,
        ctx: &StaffTickContext,
        situation: Option<&StaffSituation>,
    ) -> StaffMindReport {
        if let Some(situation) = situation {
            let view = StaffView {
                tick: ctx,
                situation,
            };
            self.reflect_all(&view);
        }

        let goals = self.organs.shared.goals.review(ctx.day());
        let consolidation = self.organs.shared.memory.maybe_consolidate(&ctx.memory());
        // A manager keeps the same diary, on the same terms. Nothing
        // renders it yet — the staff page has no mind block at all — but
        // the notes are authored where the turns are decided rather than
        // waiting for a reader, because a diary that starts the day
        // someone builds a page for it has no past to show.
        self.organs
            .shared
            .journal_tick(goals.as_ref(), consolidation.as_ref(), ctx.day());
        StaffMindReport {
            goals,
            consolidation,
        }
    }

    /// Called when he leaves a club.
    ///
    /// The three organs answer this differently, and the difference is
    /// the design. **Memory keeps everything** — a career is what a
    /// manager carries between jobs. **Judgements keep everything too**,
    /// which is the whole point of moving them onto the man: a manager
    /// who rated a player at one club still rates him at the next.
    /// **Goals resolve**: what he wanted out of is answered by leaving,
    /// what he wanted at the old club is moot, and what he wants for
    /// himself travels with him.
    pub fn on_club_change(&mut self, leaving_club_id: u32) {
        self.organs.shared.memory.on_club_change(leaving_club_id);
        self.organs.shared.goals.on_club_change();

        // Standing is about a specific board, a specific room and a
        // specific crowd, and none of it travels. What he believes about
        // those people stays in memory as a conviction.
        self.authority.on_club_change(ActorRef::NONE);
        // His football gets to be his own again.
        self.philosophy.on_club_change();
    }

    // ── Census ──────────────────────────────────────────────────

    pub fn census(&self) -> MemoryCensus {
        self.organs.shared.memory.census()
    }

    pub fn goal_census(&self) -> GoalCensus {
        self.organs.shared.goals.census()
    }

    pub fn judgement_census(&self, ctx: &StaffTickContext) -> JudgementCensus {
        Judgements::census(&self.organs.judgements, ctx.day())
    }
}

/// How the five faculties read a manager's job satisfaction.
///
/// **This runs alongside `Staff::job_satisfaction`, not instead of it.**
/// The existing field is one `f32` moved by four nudges, so the parallel
/// run here is cheap to reconcile — but it is still a parallel run, and
/// the switch-over is gated on the manager-market census in
/// `docs/staff_mind.md` §10.
#[derive(Debug, Clone, Copy)]
pub struct StaffMoodProfile {
    pub ambition: MoodContribution,
    pub authority: MoodContribution,
    pub judgement: MoodContribution,
    pub philosophy: MoodContribution,
    pub welfare: MoodContribution,
}

impl StaffMoodProfile {
    /// Every contribution, in fan-out order.
    pub fn contributions(&self) -> [MoodContribution; 5] {
        [
            self.ambition,
            self.authority,
            self.judgement,
            self.philosophy,
            self.welfare,
        ]
    }

    /// Net read of how he feels about the job, confidence-weighted so a
    /// faculty with nothing to go on does not dilute the ones that have.
    pub fn net(&self) -> f32 {
        self.contributions().iter().map(|c| c.weighted()).sum()
    }

    /// How much of him is actually being read.
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

    /// The same reading on the 0..100 scale `job_satisfaction` uses, so
    /// the parallel run can be compared without a conversion at every
    /// call site. 50 is neutral.
    ///
    /// One-to-one. [`Self::net`] sums five contributions each bounded at
    /// ±10, so its range is ±50 and `50 + net` covers exactly 0..100. An
    /// earlier ×2.5 here treated `net` as a mean; the `.dev/mind` census
    /// read the result as +46.8 points high with only 2% of managers
    /// inside ±10, which is what a saturated scale looks like.
    pub fn as_satisfaction(&self) -> f32 {
        (50.0 + self.net()).clamp(0.0, 100.0)
    }
}

/// What one mind tick did.
#[derive(Debug, Clone, Copy, Default)]
pub struct StaffMindReport {
    /// `Some` on the ticks the weekly goal review actually ran.
    pub goals: Option<GoalReviewReport>,
    /// `Some` on the ticks consolidation actually ran.
    pub consolidation: Option<ConsolidationReport>,
}

/// Which faculty a domain speaks for. Exposed for the census harness and
/// the staff profile page, which group by faculty rather than by goal.
impl StaffMind {
    pub fn faculty_domains() -> [GoalDomain; 5] {
        [
            GoalDomain::Management,
            GoalDomain::Boardroom,
            GoalDomain::Squad,
            GoalDomain::Philosophy,
            GoalDomain::Welfare,
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Fixture builders. Grouped on a type rather than left loose, so
    /// the file reads as `Fixture::context()` at every call site.
    struct Fixture;

    impl Fixture {
        fn attributes() -> PersonAttributes {
            PersonAttributes {
                adaptability: 12.0,
                ambition: 13.0,
                controversy: 5.0,
                loyalty: 12.0,
                pressure: 12.0,
                professionalism: 13.0,
                sportsmanship: 12.0,
                temperament: 10.0,
                consistency: 12.0,
                important_matches: 12.0,
                dirtiness: 4.0,
            }
        }

        fn date(year: i32, month: u32, day: u32) -> NaiveDate {
            NaiveDate::from_ymd_opt(year, month, day).expect("valid fixture date")
        }

        fn context(date: NaiveDate, club: u32) -> StaffTickContext {
            StaffTickContext::new(date, club, &Self::attributes(), 50.0)
        }
    }

    #[test]
    fn a_fresh_mind_is_empty_and_ticks_quietly() {
        let mut mind = StaffMind::new();
        let report = mind.tick(&Fixture::context(Fixture::date(2030, 8, 1), 7));
        assert_eq!(mind.census(), MemoryCensus::default());
        assert!(report.consolidation.is_some());
        assert_eq!(
            report.consolidation.unwrap(),
            ConsolidationReport::default()
        );
    }

    #[test]
    fn every_faculty_speaks_for_a_distinct_axis() {
        let mut domains = StaffMind::faculty_domains().to_vec();
        let before = domains.len();
        domains.dedup();
        assert_eq!(before, domains.len());

        let mind = StaffMind::new();
        assert_eq!(
            mind.appraise()
                .contributions()
                .map(|contribution| contribution.domain)
                .to_vec(),
            StaffMind::faculty_domains().to_vec(),
            "the mood profile is in fan-out order"
        );
    }

    #[test]
    fn a_tick_with_no_situation_forms_nothing() {
        let mut mind = StaffMind::new();
        let start = Fixture::date(2030, 8, 1);
        for week in 0..20 {
            mind.tick(&Fixture::context(
                start + chrono::Duration::days(week * 7),
                7,
            ));
        }
        assert!(
            mind.goals().is_empty(),
            "no situation, no thinking — a neutral read is not a read"
        );
    }

    #[test]
    fn remembering_reaches_the_memory_organ_and_the_faculty_at_once() {
        let mut mind = StaffMind::new();
        let c = Fixture::context(Fixture::date(2030, 8, 1), 7);
        mind.remember(EpisodeKind::AppointedManager, ActorRef::club(7), &c);

        assert_eq!(mind.census().episodes, 1);
        assert_eq!(mind.census().flashbulbs, 1);
        assert_eq!(mind.ambition.club, ActorRef::club(7));
    }

    #[test]
    fn a_move_never_clears_what_he_remembers_or_what_he_thinks_of_people() {
        let mut mind = StaffMind::new();
        let c = Fixture::context(Fixture::date(2030, 8, 1), 7);
        mind.remember(EpisodeKind::WonLeagueTitle, ActorRef::club(7), &c);
        mind.form_judgement(ActorRef::player(4), 0.8, 0.9, &c);
        let episodes = mind.census();

        mind.on_club_change(7);

        assert_eq!(mind.census(), episodes);
        assert!(mind.judgement_of(ActorRef::player(4)).is_some());
    }

    #[test]
    fn silence_is_distinguishable_from_contentment() {
        let mind = StaffMind::new();
        let profile = mind.appraise();
        assert!(
            profile.coverage() < 0.3,
            "a manager with no history is barely readable: {}",
            profile.coverage()
        );
        assert!(
            profile.ambition.is_silent() && profile.authority.is_silent(),
            "and the faculties say so rather than reporting a confident zero"
        );
    }

    #[test]
    fn the_mind_stays_copy_so_cloning_a_staff_member_stays_cheap() {
        fn assert_copy<T: Copy>() {}
        assert_copy::<StaffMind>();
    }

    #[test]
    fn the_footprint_is_declared() {
        // Three organs plus five faculties, entirely inline. Staff are
        // an order of magnitude fewer than players, so the budget is
        // generous — but it is still a budget.
        //
        // Raised by 200 bytes when the journal landed: twelve dated notes
        // at sixteen bytes each, and they are the only record of the
        // turns the goal stack prunes. Paid once, deliberately.
        assert!(
            size_of::<StaffMind>() <= 3656,
            "StaffMind grew to {} bytes",
            size_of::<StaffMind>()
        );
    }
}
