//! One intention, held over time.
//!
//! The thing that does not exist in the simulation today. A desire
//! currently fires a `HappinessEvent`, sits in a decaying log, and is
//! gone inside a year; `process_transfer_desire` rebuilds its whole
//! reason set from live ground truth every single week. So a player
//! cannot *hold* anything. He cannot decide "I'll give it until
//! January" — he can only re-notice the same grievance fifty-two times.
//!
//! A [`MindGoal`] persists. It has a strength that grows with evidence
//! and fades without it, an urgency that rises as a window closes, a
//! progress that can satisfy it, and a status ladder it climbs and
//! descends. Behaviour is coherent across months because the state is
//! carried rather than recomputed.

use super::catalog::GoalKind;
use super::evidence::{GoalBlocker, GoalEvidence, GoalOrigin};
use crate::club::mind::organs::memory::{EpochDay, MindClock};

/// How far along the ladder from private feeling to formal demand.
///
/// The ladder *is* the escalation, and it is stateful — which is the
/// whole difference from the current model. A goal can sit at
/// [`Active`] for a season, silently shaping every decision he makes,
/// then be voiced, then pressed. Today that middle rung exists for
/// exactly one want (`big_stage_inclination`); here it exists for all
/// of them.
///
/// [`Active`]: GoalStatus::Active
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum GoalStatus {
    /// He feels it. It does not yet shape what he does, and nobody
    /// knows.
    #[default]
    Latent,
    /// It shapes every decision he makes, silently. He will listen to
    /// an approach he would otherwise have dismissed, hold out a little
    /// longer for the right one — and say nothing about why.
    Active,
    /// Said out loud. A mood event fires, the manager can talk about it,
    /// the press can pick it up.
    Voiced,
    /// A formal demand — a transfer request, an ultimatum.
    Pressing,
    /// Achieved. Banks a positive episode and stops.
    Satisfied,
    /// The deadline passed unmet. Banks a negative episode, hardens what
    /// he believes, and — for the goals that have somewhere further to
    /// go — feeds whatever comes next.
    Frustrated,
    /// He let it go: age, resignation, or a better goal displaced it.
    Abandoned,
}

impl GoalStatus {
    /// Still in play — the mind should keep reviewing it.
    #[inline]
    pub fn is_live(self) -> bool {
        matches!(
            self,
            GoalStatus::Latent | GoalStatus::Active | GoalStatus::Voiced | GoalStatus::Pressing
        )
    }

    /// Finished, one way or another.
    #[inline]
    pub fn is_resolved(self) -> bool {
        !self.is_live()
    }

    /// Anyone outside his own head can tell.
    #[inline]
    pub fn is_public(self) -> bool {
        matches!(self, GoalStatus::Voiced | GoalStatus::Pressing)
    }

    /// Shapes decisions, whether or not it has been spoken.
    #[inline]
    pub fn shapes_decisions(self) -> bool {
        matches!(
            self,
            GoalStatus::Active | GoalStatus::Voiced | GoalStatus::Pressing
        )
    }

    /// Rung on the ladder, for comparisons and for the one-way rule
    /// (a goal never silently un-presses inside a single review).
    #[inline]
    pub fn rung(self) -> u8 {
        match self {
            GoalStatus::Latent => 0,
            GoalStatus::Active => 1,
            GoalStatus::Voiced => 2,
            GoalStatus::Pressing => 3,
            GoalStatus::Satisfied | GoalStatus::Frustrated | GoalStatus::Abandoned => 4,
        }
    }

    pub fn as_i18n_key(self) -> &'static str {
        match self {
            GoalStatus::Latent => "mind_goal_status_latent",
            GoalStatus::Active => "mind_goal_status_active",
            GoalStatus::Voiced => "mind_goal_status_voiced",
            GoalStatus::Pressing => "mind_goal_status_pressing",
            GoalStatus::Satisfied => "mind_goal_status_satisfied",
            GoalStatus::Frustrated => "mind_goal_status_frustrated",
            GoalStatus::Abandoned => "mind_goal_status_abandoned",
        }
    }

    pub const ALL: &'static [GoalStatus] = &[
        GoalStatus::Latent,
        GoalStatus::Active,
        GoalStatus::Voiced,
        GoalStatus::Pressing,
        GoalStatus::Satisfied,
        GoalStatus::Frustrated,
        GoalStatus::Abandoned,
    ];
}

/// Something a player wants, carried until it resolves. 24 bytes.
#[derive(Debug, Clone, Copy, Default)]
pub struct MindGoal {
    pub kind: GoalKind,
    pub origin: GoalOrigin,
    pub status: GoalStatus,
    pub blocked_by: GoalBlocker,
    pub evidence: GoalEvidence,
    /// When it first appeared.
    pub formed_on: EpochDay,
    /// Last time anything reinforced it. Decay runs from here, so a goal
    /// being fed every week never fades.
    pub last_fed: EpochDay,
    /// The date he has privately given it — "by January". 0 for none.
    /// Passing it unmet is what turns the goal [`Frustrated`], and it is
    /// what lets a player behave coherently for four months instead of
    /// re-rolling the same grievance weekly.
    ///
    /// [`Frustrated`]: GoalStatus::Frustrated
    pub deadline: EpochDay,
    /// 0..=10000 basis points — how badly he wants it.
    ///
    /// Basis points rather than whole percent, and the difference is
    /// load-bearing. Decay is multiplicative and small — a want fading
    /// at 4% a month sheds under 1% per weekly review — so at percent
    /// resolution the new value rounds back to the same integer once
    /// strength drops below about 53, and the fade stops dead. A want
    /// nothing had fed for four years would sit at half strength
    /// forever.
    strength_bp: u16,
    /// 0..=100 — how much the timing presses.
    urgency_pct: u8,
    /// 0..=100 — how far toward satisfied.
    progress_pct: u8,
    /// Times it has been reinforced. Saturates. A want with forty
    /// separate confirmations behind it is not the same as one with two.
    pub reinforcements: u8,
}

impl MindGoal {
    /// Strength a goal is born with. Deliberately low: a want appears as
    /// a feeling, not a demand, and has to be fed before it is anything.
    pub const SEED_STRENGTH: f32 = 0.22;

    /// Strength below which a live goal is let go.
    pub const SPENT: f32 = 0.06;

    /// Fraction of the gap to certainty one reinforcement closes.
    /// Diminishing, so the first evidence moves a want far more than
    /// the tenth — the same shape the semantic store uses, for the same
    /// reason.
    pub const REINFORCEMENT_GAIN: f32 = 0.30;

    pub fn new(
        kind: GoalKind,
        origin: GoalOrigin,
        evidence: GoalEvidence,
        today: EpochDay,
    ) -> Self {
        MindGoal {
            kind,
            origin,
            status: GoalStatus::Latent,
            blocked_by: GoalBlocker::None,
            evidence,
            formed_on: today,
            last_fed: today,
            deadline: 0,
            strength_bp: (Self::SEED_STRENGTH * 10_000.0).round() as u16,
            urgency_pct: 0,
            progress_pct: 0,
            reinforcements: 1,
        }
    }

    #[inline]
    pub fn strength(&self) -> f32 {
        self.strength_bp as f32 / 10_000.0
    }

    #[inline]
    pub fn set_strength(&mut self, value: f32) {
        self.strength_bp = (value.clamp(0.0, 1.0) * 10_000.0).round() as u16;
    }

    #[inline]
    pub fn urgency(&self) -> f32 {
        self.urgency_pct as f32 / 100.0
    }

    #[inline]
    pub fn set_urgency(&mut self, value: f32) {
        self.urgency_pct = (value.clamp(0.0, 1.0) * 100.0).round() as u8;
    }

    #[inline]
    pub fn progress(&self) -> f32 {
        self.progress_pct as f32 / 100.0
    }

    #[inline]
    pub fn set_progress(&mut self, value: f32) {
        self.progress_pct = (value.clamp(0.0, 1.0) * 100.0).round() as u8;
    }

    #[inline]
    pub fn has_deadline(&self) -> bool {
        self.deadline != 0
    }

    /// Give himself until a date. "I'll see how the first half of the
    /// season goes."
    pub fn commit_until(&mut self, deadline: EpochDay) {
        self.deadline = deadline;
    }

    /// The number the escalation ladder actually reads: how much he
    /// wants it, weighted by how much the timing presses, discounted by
    /// how far it is already satisfied, and biased by the character of
    /// the want.
    ///
    /// Continuous throughout — the only bars anywhere are the catalog's
    /// `voice_at` / `press_at`, which are properties of the want rather
    /// than thresholds bolted onto the mechanism.
    pub fn pressure(&self) -> f32 {
        let timing = 0.65 + 0.35 * self.urgency();
        let unmet = 1.0 - self.progress();
        (self.strength() * timing * unmet * self.origin.escalation_bias()).clamp(0.0, 1.0)
    }

    /// Another reason to want it. Strengthens with diminishing returns,
    /// resets the decay clock, and folds in whatever new signal came
    /// with it.
    pub fn reinforce(&mut self, amount: f32, evidence: GoalEvidence, today: EpochDay) {
        let gain = Self::REINFORCEMENT_GAIN * amount.clamp(0.0, 1.0);
        let strength = self.strength();
        self.set_strength(strength + (1.0 - strength) * gain);
        self.evidence.merge(evidence);
        self.last_fed = today;
        self.reinforcements = self.reinforcements.saturating_add(1);
    }

    /// Time passing with nothing to feed it.
    ///
    /// Takes the span since the *last decay*, not a date. Decaying "to a
    /// date" from `last_fed` looks equivalent and is not: called once a
    /// week on a want nothing is feeding, it re-applies an ever-growing
    /// span to an already-decayed strength, and nineteen weekly reviews
    /// compound to roughly nineteen times the intended fade. A want that
    /// should still have been there was silently abandoned instead.
    pub fn decay(&mut self, per_month: f32, days_elapsed: u16) {
        if days_elapsed == 0 {
            return;
        }
        let months = days_elapsed as f32 / 30.0;
        let retained = (1.0 - per_month.clamp(0.0, 0.95)).powf(months);
        self.set_strength(self.strength() * retained);
    }

    /// Something happened that moves it toward being satisfied.
    pub fn advance(&mut self, amount: f32) {
        self.set_progress(self.progress() + amount.clamp(0.0, 1.0));
    }

    /// Time constant for urgency accrued by waiting. At ~400 days a want
    /// held with nothing happening is around 60% urgent; at two years,
    /// 84%.
    pub const URGENCY_TAU_DAYS: f32 = 400.0;

    /// Urgency the goal has earned simply by going unmet.
    ///
    /// Nothing else in the simulation raises urgency yet, and without
    /// this the timing term would sit at its floor forever — capping
    /// pressure at 65% and making the `press_at` bars unreachable for
    /// most wants. It is also the more truthful model: a man who has
    /// wanted the same thing for two seasons with nothing changing is
    /// not in the same state as one who decided last week, at identical
    /// strength.
    ///
    /// A deadline overrides age when it is nearer — the last month
    /// before the date he gave himself presses harder than the two years
    /// before it.
    ///
    /// Only ever raises: an unmet want does not get less pressing by
    /// being waited out, and callers that push urgency higher (an
    /// expiring contract, a closing window) are never undone by this.
    pub fn accrue_urgency(&mut self, today: EpochDay) {
        let waited = MindClock::elapsed_f32(self.formed_on, today);
        let by_age = 1.0 - (-waited / Self::URGENCY_TAU_DAYS).exp();

        let by_deadline = if self.has_deadline() && today < self.deadline {
            let window = MindClock::elapsed_f32(self.formed_on, self.deadline).max(1.0);
            let elapsed = MindClock::elapsed_f32(self.formed_on, today);
            (elapsed / window).clamp(0.0, 1.0)
        } else {
            0.0
        };

        // A want cannot be more pressing than it is wanted. Without this
        // the two curves fight: strength fades on a want nothing is
        // feeding while age drives urgency up forever, so a grievance
        // everyone has stopped giving him reasons for would go on
        // pressing harder every year. Capping by strength makes the
        // whole thing subside together, which is what letting go of
        // something actually looks like.
        let accrued = by_age.max(by_deadline).min(self.strength());
        self.set_urgency(self.urgency().max(accrued));
    }

    /// A competing goal took hold. Weakens without resetting the clock —
    /// he has not stopped wanting it, something else simply matters more.
    pub fn yield_to_competition(&mut self, amount: f32) {
        self.set_strength(self.strength() * (1.0 - amount.clamp(0.0, 1.0)));
    }

    /// Days he has held it.
    #[inline]
    pub fn age_days(&self, today: EpochDay) -> u16 {
        MindClock::elapsed(self.formed_on, today)
    }

    /// Has the date he gave himself passed?
    #[inline]
    pub fn deadline_passed(&self, today: EpochDay) -> bool {
        self.has_deadline() && today >= self.deadline
    }

    #[inline]
    pub fn is_live(&self) -> bool {
        self.status.is_live()
    }

    #[inline]
    pub fn is_spent(&self) -> bool {
        self.strength() < Self::SPENT
    }

    /// How much this goal is worth keeping when the stack is full.
    /// Resolved goals rank below anything still live, and a goal he has
    /// said out loud outranks one he has not.
    pub fn keep_rank(&self) -> f32 {
        if !self.is_live() {
            return 0.0;
        }
        self.pressure() + self.status.rung() as f32 * 0.1
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TODAY: EpochDay = 10_000;

    fn goal(kind: GoalKind) -> MindGoal {
        MindGoal::new(kind, GoalOrigin::SelfDrive, GoalEvidence::EMPTY, TODAY)
    }

    #[test]
    fn a_new_goal_starts_quiet() {
        let g = goal(GoalKind::StepUpToABiggerClub);
        assert_eq!(g.status, GoalStatus::Latent);
        assert!(g.strength() < 0.3, "a want appears as a feeling");
        assert!(!g.status.is_public());
        assert!(!g.status.shapes_decisions());
    }

    #[test]
    fn reinforcement_strengthens_with_diminishing_returns() {
        let mut g = goal(GoalKind::StepUpToABiggerClub);
        let mut prior = g.strength();
        let mut first_gain = 0.0;

        for round in 0..8 {
            g.reinforce(1.0, GoalEvidence::EMPTY, TODAY);
            let gain = g.strength() - prior;
            if round == 0 {
                first_gain = gain;
            } else {
                assert!(gain <= first_gain + 1e-6, "gains must not grow");
            }
            prior = g.strength();
        }
        assert!(prior < 1.0001);
        assert!(prior > 0.85, "sustained evidence gets there: {prior}");
    }

    #[test]
    fn a_goal_nobody_feeds_fades() {
        let mut g = goal(GoalKind::GetAReleaseClause);
        for _ in 0..6 {
            g.reinforce(1.0, GoalEvidence::EMPTY, TODAY);
        }
        let fed = g.strength();

        g.decay(GoalKind::GetAReleaseClause.spec().decay_per_month, 365);
        assert!(
            g.strength() < fed * 0.2,
            "a year of silence: {fed} → {}",
            g.strength()
        );
    }

    #[test]
    fn a_grievance_outlasts_a_passing_want_over_the_same_silence() {
        let mut grievance = goal(GoalKind::LeaveThisClub);
        let mut passing = goal(GoalKind::GetAReleaseClause);
        for _ in 0..6 {
            grievance.reinforce(1.0, GoalEvidence::EMPTY, TODAY);
            passing.reinforce(1.0, GoalEvidence::EMPTY, TODAY);
        }

        grievance.decay(GoalKind::LeaveThisClub.spec().decay_per_month, 365);
        passing.decay(GoalKind::GetAReleaseClause.spec().decay_per_month, 365);

        assert!(
            grievance.strength() > passing.strength() * 3.0,
            "a decision to get out does not evaporate the way a contract wish does"
        );
    }

    #[test]
    fn a_want_kept_alive_stays_alive() {
        let mut fed = goal(GoalKind::StepUpToABiggerClub);
        let mut ignored = goal(GoalKind::StepUpToABiggerClub);
        for _ in 0..5 {
            fed.reinforce(1.0, GoalEvidence::EMPTY, TODAY);
            ignored.reinforce(1.0, GoalEvidence::EMPTY, TODAY);
        }

        // Both sit through the same span of time; only one of them is
        // fed part-way through.
        let decay = GoalKind::StepUpToABiggerClub.spec().decay_per_month;
        fed.decay(decay, 150);
        ignored.decay(decay, 150);
        fed.reinforce(0.4, GoalEvidence::EMPTY, TODAY + 150);
        fed.decay(decay, 150);
        ignored.decay(decay, 150);

        assert!(
            fed.strength() > ignored.strength(),
            "a want kept alive stays alive: {} vs {}",
            fed.strength(),
            ignored.strength()
        );
    }

    #[test]
    fn pressure_rises_with_urgency_and_falls_with_progress() {
        let mut g = goal(GoalKind::PlayFirstTeamFootball);
        for _ in 0..6 {
            g.reinforce(1.0, GoalEvidence::EMPTY, TODAY);
        }
        let base = g.pressure();

        g.set_urgency(1.0);
        assert!(g.pressure() > base, "a closing window presses harder");

        g.set_progress(0.9);
        assert!(
            g.pressure() < base,
            "and a want mostly met stops pressing at all"
        );
    }

    #[test]
    fn a_fully_satisfied_goal_exerts_no_pressure() {
        let mut g = goal(GoalKind::WinBackMyPlace);
        for _ in 0..8 {
            g.reinforce(1.0, GoalEvidence::EMPTY, TODAY);
        }
        g.advance(1.0);
        assert_eq!(g.pressure(), 0.0);
    }

    #[test]
    fn the_character_of_a_want_changes_how_hard_it_presses() {
        let build = |origin: GoalOrigin| {
            let mut g = MindGoal::new(GoalKind::LeaveThisClub, origin, GoalEvidence::EMPTY, TODAY);
            for _ in 0..5 {
                g.reinforce(1.0, GoalEvidence::EMPTY, TODAY);
            }
            g.pressure()
        };

        assert!(
            build(GoalOrigin::Grievance) > build(GoalOrigin::SelfDrive),
            "being wronged presses harder than wanting more"
        );
        assert!(
            build(GoalOrigin::Attachment) < build(GoalOrigin::SelfDrive),
            "and fondness presses least"
        );
    }

    #[test]
    fn a_deadline_is_a_date_he_gave_himself() {
        let mut g = goal(GoalKind::PlayFirstTeamFootball);
        assert!(!g.has_deadline());
        assert!(!g.deadline_passed(TODAY + 10_000));

        g.commit_until(TODAY + 120);
        assert!(!g.deadline_passed(TODAY + 60), "January has not come yet");
        assert!(g.deadline_passed(TODAY + 121));
    }

    #[test]
    fn yielding_to_competition_does_not_reset_the_clock() {
        let mut g = goal(GoalKind::LeaveThisClub);
        for _ in 0..5 {
            g.reinforce(1.0, GoalEvidence::EMPTY, TODAY);
        }
        let fed_on = g.last_fed;
        g.yield_to_competition(0.4);

        assert!(g.strength() < 1.0);
        assert_eq!(
            g.last_fed, fed_on,
            "he has not stopped wanting it — something else simply matters more"
        );
    }

    #[test]
    fn evidence_accumulates_across_reinforcements() {
        let mut g = MindGoal::new(
            GoalKind::GoHome,
            GoalOrigin::Attachment,
            GoalEvidence::of(&[GoalEvidence::HOMESICK]),
            TODAY,
        );
        g.reinforce(
            1.0,
            GoalEvidence::of(&[GoalEvidence::LANGUAGE_BARRIER]),
            TODAY,
        );
        assert_eq!(g.evidence.count(), 2);
        assert!(g.evidence.contains(GoalEvidence::HOMESICK));
        assert!(g.evidence.contains(GoalEvidence::LANGUAGE_BARRIER));
    }

    #[test]
    fn the_status_ladder_is_ordered_and_keys_are_unique() {
        assert!(GoalStatus::Latent.rung() < GoalStatus::Active.rung());
        assert!(GoalStatus::Active.rung() < GoalStatus::Voiced.rung());
        assert!(GoalStatus::Voiced.rung() < GoalStatus::Pressing.rung());

        let mut keys: Vec<&str> = GoalStatus::ALL.iter().map(|s| s.as_i18n_key()).collect();
        keys.sort_unstable();
        let before = keys.len();
        keys.dedup();
        assert_eq!(before, keys.len());
    }

    #[test]
    fn only_the_middle_rungs_shape_decisions_silently() {
        assert!(!GoalStatus::Latent.shapes_decisions());
        assert!(GoalStatus::Active.shapes_decisions());
        assert!(!GoalStatus::Active.is_public(), "active is silent");
        assert!(GoalStatus::Voiced.is_public());
        assert!(GoalStatus::Pressing.is_public());
        assert!(!GoalStatus::Satisfied.is_live());
    }

    #[test]
    fn a_goal_stays_within_its_budget() {
        assert!(
            size_of::<MindGoal>() <= 24,
            "MindGoal grew to {}",
            size_of::<MindGoal>()
        );
    }
}
