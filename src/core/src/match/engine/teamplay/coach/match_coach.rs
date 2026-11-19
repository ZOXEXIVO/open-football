//! **The live coach.** [`MatchCoach`] — one per side — holding the card,
//! the plan, the stance and the read, the possession / shot counters the
//! shooting gate reads, and the record of what he did.

use crate::MatchTacticType;
use crate::club::staff::CoachingStyle;
use crate::club::team::talks::TeamTalkTone;
use crate::r#match::engine::flow::arena::formation_variant::FormationLine;
use crate::r#match::engine::teamplay::coach::card::CoachCard;
use crate::r#match::engine::teamplay::coach::metrics::{
    CoachRead, ReadCounters, RollingTeamMetrics,
};
use crate::r#match::engine::teamplay::coach::plan::{GamePlan, SquadFit};
use crate::r#match::engine::teamplay::coach::record::{ChangeMoment, CoachRecord, PlanChange};
use crate::r#match::engine::teamplay::coach::stance::{Outlook, Situation, Stance};

/// A formation the side could switch to: how much it commits and how used
/// the side is to it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ShapeChoice {
    pub shape: MatchTacticType,
    pub commitment: f32,
    pub familiarity: f32,
}

/// Per-team coach state during a match
#[derive(Debug, Clone)]
pub struct MatchCoach {
    /// The man in the dugout, for the whole match.
    pub card: CoachCard,
    /// The plan the side is playing.
    pub plan: GamePlan,
    /// His picture of the fixture, his reading of the match as he last saw
    /// it, and the commitment that reading asks for.
    outlook: Outlook,
    situation: Situation,
    stance: Stance,
    /// The stance at kickoff, whose demand the plan carries: the levers
    /// answer what the stance has moved since.
    kickoff_stance: f32,
    /// The commitment the side wanted when the shape in force was set.
    shape_commitment: f32,
    /// What the eleven can carry out, read at kickoff.
    fit: SquadFit,
    /// The minute the plan in force was set, the side's last change of
    /// formation, the moment waiting for a decision and the next review.
    plan_since: f32,
    last_shape_change: Option<f32>,
    pending: Option<ChangeMoment>,
    next_review: f32,
    /// A plan pinned by the harness stands for the whole match.
    pinned_plan: bool,
    /// Tick when instruction was last updated
    pub last_update_tick: u64,
    /// Team's last shot tick (for team-wide shot cooldown)
    pub last_shot_tick: u64,
    /// Tick when this team most recently gained possession. Used as a
    /// build-up gate: teams can't shoot within a short window of winning
    /// the ball, which forces an outlet pass / progression instead of
    /// hack-and-counter. Updated by the match loop on possession-change.
    pub last_possession_gain_tick: u64,
    /// Shots fired in the current possession. Reset when we lose the
    /// ball (possession change TO us, FROM us). Real football: one
    /// quality chance per possession. Rebound / tap-in scrambles
    /// (ball leaves owner briefly but team keeps control) don't
    /// count as a new possession — the cap holds until the opposition
    /// touches the ball.
    pub shots_this_possession: u32,
    /// The coach's moving read of the run of play, and what it says now.
    read: CoachRead,
    pub metrics: RollingTeamMetrics,
    /// Running possession, field-tilt and played ticks, and line breaks
    /// conceded, kept on every tactical refresh so a coach evaluation does
    /// not walk the players for them.
    pub cum_possession_ticks: u32,
    pub cum_field_tilt_ticks: u32,
    pub cum_ticks: u32,
    pub cum_line_breaks_against: u32,
    /// The last line is broken as of the last refresh.
    line_broken: bool,
    /// What this coach has done so far, for the result.
    pub record: CoachRecord,
}

impl Default for MatchCoach {
    fn default() -> Self {
        MatchCoach {
            card: CoachCard::neutral(),
            plan: GamePlan::neutral(),
            outlook: Outlook::default(),
            situation: Situation::default(),
            stance: Stance::default(),
            kickoff_stance: 0.0,
            shape_commitment: 0.5,
            fit: SquadFit::default(),
            plan_since: 0.0,
            last_shape_change: None,
            pending: None,
            next_review: Self::REVIEW_EVERY,
            pinned_plan: false,
            last_update_tick: 0,
            last_shot_tick: 0,
            last_possession_gain_tick: 0,
            shots_this_possession: 0,
            read: CoachRead::default(),
            metrics: RollingTeamMetrics::default(),
            cum_possession_ticks: 0,
            cum_field_tilt_ticks: 0,
            cum_ticks: 0,
            cum_line_breaks_against: 0,
            line_broken: false,
            record: CoachRecord::default(),
        }
    }
}

impl MatchCoach {
    /// Minutes of play between the coach's reviews of the read.
    pub const REVIEW_EVERY: f32 = 5.0;
    /// How long a plan stands before a read can call it failing.
    pub const PLAN_SETTLES: f32 = 10.0;
    /// The fewest minutes between two changes of a side's formation.
    pub const SHAPE_GAP: f32 = 12.0;
    /// How far the plan he wants must have moved from the one in force
    /// before he changes it: a floor, and what his resistance to change
    /// adds. At the interval he can explain a change in the dressing room,
    /// so half as much is enough.
    const CHANGE_FROM: f32 = 0.03;
    const CHANGE_RESISTANCE: f32 = 0.07;
    const INTERVAL_EASE: f32 = 0.5;
    /// What playing a formation the side is not used to costs, in
    /// commitment: a man between the lines when it has never played it.
    const FAMILIARITY_COST: f32 = 0.12;
    /// How far, in xG over the read, the balance may fall short of what the
    /// plan expected before the coach calls it failing: wide for a poor
    /// reader of a match, narrow for a sharp one.
    const FAILING_MARGIN: f32 = 0.9;
    const FAILING_READ: f32 = 0.6;
    /// A half this many goals above what the kickoff rates expected is
    /// worth praising, and one this many below needs lifting; between them
    /// it met what he expected of it.
    const PRAISE_FROM: f32 = 1.0;
    const LIFT_FROM: f32 = 0.5;
    const HALF_MINUTES: f32 = 45.0;

    pub fn new() -> Self {
        Self::default()
    }

    /// The side kicks off under `card` in the fixture `outlook` describes,
    /// with an eleven that fits the plan as `fit` reads: the coach sets the
    /// plan, unless one is pinned.
    pub fn on_kickoff(
        &mut self,
        card: CoachCard,
        outlook: Outlook,
        fit: &SquadFit,
        pinned: Option<GamePlan>,
    ) {
        self.card = card;
        self.outlook = outlook;
        self.fit = *fit;
        self.situation = Situation::default();
        self.kickoff_stance = self.reading();
        self.stance.kick_off(self.kickoff_stance);
        self.plan =
            pinned.unwrap_or_else(|| GamePlan::for_kickoff(&card, self.stance.current(), fit));
        self.shape_commitment = self.plan.commitment;
        self.record.kickoff_plan = Some(self.plan);
        self.pinned_plan = pinned.is_some();
    }

    /// The match as the coach sees it at `minute`, on his cadence, with the
    /// side's running counters for his read of the run of play.
    pub fn observe(&mut self, situation: Situation, minute: f32, counters: ReadCounters) {
        self.read.note(minute as u32, counters);
        self.metrics = self.read.read();
        self.situation = situation;
        let reading = self.reading();
        self.stance.follow(reading, minute, &self.card.temperament);
        self.review(minute);
    }

    /// A goal the coach can see: for his side when `scored`.
    pub fn on_goal(&mut self, scored: bool, minute: f32) {
        let before = self.reading();
        self.situation.difference += if scored { 1 } else { -1 };
        let after = self.reading();
        self.stance.on_goal(before, after, &self.card.temperament);
        self.stance.follow(after, minute, &self.card.temperament);
        self.pending = Some(if scored {
            ChangeMoment::GoalFor
        } else {
            ChangeMoment::GoalAgainst
        });
    }

    /// The interval: the coach gives his talk, recorded with the result,
    /// and reconsiders the plan before the second half.
    pub fn on_half_time(&mut self) -> TeamTalkTone {
        self.pending = Some(ChangeMoment::HalfTime);
        let tone = self.half_time_tone();
        self.record.half_time_talk = Some(tone);
        tone
    }

    /// The talk: praise for a half well above what he expected of it, calm
    /// for one that met it, and below it his own way of lifting them —
    /// criticism from a volatile or authoritarian man, passion from a
    /// motivator, encouragement from a man-manager.
    fn half_time_tone(&self) -> TeamTalkTone {
        let rates = self.outlook.rates(
            self.situation.own_men,
            self.situation.opp_men,
            self.kickoff_stance,
        );
        let surplus =
            self.situation.difference as f32 - (rates.own - rates.opp) * Self::HALF_MINUTES;
        if surplus >= Self::PRAISE_FROM {
            return TeamTalkTone::Praise;
        }
        if surplus > -Self::LIFT_FROM {
            return TeamTalkTone::TacticalSilent;
        }
        let t = &self.card.temperament;
        let severity = if self.card.style == CoachingStyle::Authoritarian {
            1.0
        } else {
            t.volatility
        };
        [
            (TeamTalkTone::Passionate, t.motivating),
            (TeamTalkTone::Criticise, severity),
        ]
        .into_iter()
        .fold((TeamTalkTone::Encourage, t.man_management), |best, way| {
            if way.1 > best.1 { way } else { best }
        })
        .0
    }

    /// A substitution forced on the side by an injury.
    pub fn on_forced_change(&mut self) {
        self.pending = Some(ChangeMoment::ForcedChange);
    }

    /// Every five minutes of play the coach looks at his read; it is a
    /// moment only when the read says the plan is failing.
    fn review(&mut self, minute: f32) {
        if minute < self.next_review {
            return;
        }
        while self.next_review <= minute {
            self.next_review += Self::REVIEW_EVERY;
        }
        if self.pending.is_none() && self.read_failing(minute) {
            self.pending = Some(ChangeMoment::FailingRead);
        }
    }

    /// Whether the read says the plan is failing: the shortfall is past the
    /// coach's margin, once the plan has had time to settle.
    pub fn read_failing(&self, minute: f32) -> bool {
        minute - self.plan_since >= Self::PLAN_SETTLES && self.shortfall(minute) > self.margin()
    }

    /// How far the chances made and conceded over the read fall short of
    /// what the kickoff rates expect of the side at its stance.
    fn shortfall(&self, minute: f32) -> f32 {
        let window = minute.min(CoachRead::WINDOW as f32);
        let rates = self.outlook.rates(
            self.situation.own_men,
            self.situation.opp_men,
            self.stance.current(),
        );
        let expected = (rates.own - rates.opp) * window;
        expected - (self.metrics.xg_for_last_15 - self.metrics.xg_against_last_15)
    }

    /// The shortfall the coach calls failing: wide for a poor reader of a
    /// match, narrow for a sharp one.
    fn margin(&self) -> f32 {
        Self::FAILING_MARGIN - Self::FAILING_READ * self.card.temperament.read
    }

    /// Who is on top as the coach reads it, −1 … +1: his shortfall over his
    /// margin, weighted by how well he reads a match.
    fn verdict(&self, minute: f32) -> f32 {
        (-self.card.temperament.read * self.shortfall(minute) / self.margin()).clamp(-1.0, 1.0)
    }

    /// Whether a moment is waiting for his decision.
    pub fn has_moment(&self) -> bool {
        self.pending.is_some()
    }

    /// At a moment, the plan the coach wants for how his side is playing: an
    /// outplayed side planned for as an underdog would be, a side on top as
    /// a favourite. He changes the settings when they move further than he
    /// needs to act, and the formation among `choices` when another carries
    /// the commitment he wants — at his verdict and at his stance through the
    /// regime's `gain` — better by as much. Nothing between moments, and
    /// nothing for a pinned plan.
    pub fn decide(
        &mut self,
        minute: f32,
        shape: MatchTacticType,
        choices: &[ShapeChoice],
        gain: f32,
    ) -> Option<PlanChange> {
        let moment = self.pending.take()?;
        if self.pinned_plan {
            return None;
        }
        let ease = if moment == ChangeMoment::HalfTime {
            Self::INTERVAL_EASE
        } else {
            1.0
        };
        let threshold = ease
            * (Self::CHANGE_FROM
                + Self::CHANGE_RESISTANCE * self.card.temperament.resistance_to_change());
        let verdict = self.verdict(minute);
        let plan_at =
            |stance: f32| GamePlan::for_kickoff(&self.card, stance.clamp(-1.0, 1.0), &self.fit);
        let wanted = plan_at(self.kickoff_stance + verdict);
        let plan = if wanted.distance(&self.plan) > threshold {
            wanted
        } else {
            self.plan
        };
        let commitment = plan_at(
            self.kickoff_stance + verdict + gain * (self.stance.current() - self.kickoff_stance),
        )
        .commitment;
        let can_reshape = self
            .last_shape_change
            .is_none_or(|at| minute - at >= Self::SHAPE_GAP);
        let new_shape = if can_reshape {
            let held = choices
                .iter()
                .find(|c| c.shape == shape)
                .map_or(self.shape_commitment, |c| c.commitment);
            Self::choose_shape(
                held + commitment - self.shape_commitment,
                shape,
                choices,
                threshold,
            )
        } else {
            shape
        };
        if plan == self.plan && new_shape == shape {
            return None;
        }
        let change = PlanChange {
            minute: minute as u8,
            moment,
            shape_before: shape,
            shape_after: new_shape,
            plan_before: self.plan,
            plan_after: plan,
        };
        self.plan = plan;
        self.plan_since = minute;
        if change.changes_shape() {
            self.last_shape_change = Some(minute);
            self.shape_commitment = commitment;
        }
        self.record.changes.push(change);
        Some(change)
    }

    /// The formation closest to the commitment wanted, a shape the side is
    /// not used to costing it, and the one it is in kept unless another is
    /// closer by `stay`.
    fn choose_shape(
        commitment: f32,
        current: MatchTacticType,
        choices: &[ShapeChoice],
        stay: f32,
    ) -> MatchTacticType {
        choices
            .iter()
            .map(|c| {
                let kept = if c.shape == current { stay } else { 0.0 };
                let cost = (c.commitment - commitment).abs()
                    + Self::FAMILIARITY_COST * (1.0 - c.familiarity)
                    - kept;
                (c.shape, cost)
            })
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .map_or(current, |(shape, _)| shape)
    }

    /// Whether the opposition has the ball past the side's last line now;
    /// each time it gets there counts once.
    pub fn note_line(&mut self, broken: bool) {
        if broken && !self.line_broken {
            self.cum_line_breaks_against += 1;
            #[cfg(feature = "match-logs")]
            {
                self.record.line_breaks_against += 1;
            }
        }
        self.line_broken = broken;
    }

    /// A man sent off: one of his own when `own`.
    pub fn on_sending_off(&mut self, own: bool, minute: f32) {
        let men = if own {
            &mut self.situation.own_men
        } else {
            &mut self.situation.opp_men
        };
        *men = men.saturating_sub(1);
        let reading = self.reading();
        self.stance.follow(reading, minute, &self.card.temperament);
        self.pending = Some(ChangeMoment::SendingOff);
    }

    /// How much the side should commit, −1 … +1.
    pub fn stance(&self) -> f32 {
        self.stance.current()
    }

    /// How far the stance has moved since kickoff: what the levers answer
    /// on top of the plan, whatever plan is in force.
    pub fn stance_shift(&self) -> f32 {
        self.stance.current() - self.kickoff_stance
    }

    fn reading(&self) -> f32 {
        Stance::reading(&self.outlook, &self.card.temperament, &self.situation)
    }

    /// The commitment reading for `minute`, kept for the census.
    #[cfg(feature = "match-logs")]
    pub fn note_commitment(&mut self, minute: usize) {
        self.record.note_commitment(minute, self.stance.current());
    }

    /// Whether the team should allow a shot right now. **Always yes.**
    ///
    /// 2026-08-16: the three team-level shot gates are REMOVED — a
    /// spacing cooldown (one shot per 7.5 s), a build-up delay (no shot
    /// within 1 s of winning the ball), and a per-possession quota (at
    /// most two attempts).
    ///
    /// None of them has a real-life analogue. Football has no per-team
    /// shot timer and no attempt quota: a side that works three openings
    /// in one spell of pressure takes three shots. What these gates did
    /// was veto a strike on WHO SHOT LAST AND WHEN, without being able to
    /// see the chance at all — so a speculative 25-yarder locked the team
    /// out of a tap-in four seconds later. The note kept below recorded
    /// the symptom without drawing the conclusion: across the shot-bar
    /// titration ON-TARGET shots FELL 3.0 → 2.4 per team while total
    /// shots rose 12.6 → 30.1.
    ///
    /// Shot volume is not theirs to hold down. It belongs to defending —
    /// pressure, blocks, and the quality of the look — which is where the
    /// pressing work will put it. See `SHOT_BAR_BASE` for the rest of the
    /// teardown.
    ///
    /// Kept as a method, and the counters it read (`last_shot_tick`,
    /// `shots_this_possession`, `last_possession_gain_tick`) are still
    /// maintained, so a real constraint can be reintroduced here without
    /// re-plumbing every caller.
    pub fn can_shoot(&self, _current_tick: u64, _rebound_live: bool) -> bool {
        true
    }

    /// The gates this replaced, preserved for the pressing work.
    ///
    /// Their reasoning is sound about SPAM and wrong about football: they
    /// filter by timing rather than by chance quality. If something like
    /// them comes back, it should read the look, not the clock.
    #[allow(dead_code)]
    fn legacy_shot_gates(&self, current_tick: u64, rebound_live: bool) -> bool {
        // Per-team shot cadence — see type docs for the full rationale.
        // 500 → 750 ticks (~7.5s spacing). Briefly tried 1000 but it
        // disproportionately hurt weak teams: they shoot rarely already,
        // and a 10s gate killed too many of their quick-counter chances
        // (rebounds, second-balls in the box) while barely throttling
        // strong sides who already pace their shots. 750 hits the
        // strong side enough to matter without crushing the upset tail.
        //
        // `rebound_live` (a dangerous parry / loose block deflection in
        // the last ~3 s — see `TeamOps::can_shoot`) suspends the
        // spacing and build-up gates: the rebound arrives 0.5-1.5 s
        // after the original shot, so without the exemption the spacing
        // gate blocked EVERY second-chance strike — contradicting the
        // possession-cap design note below, and deleting one of
        // football's core goal patterns (~4-6% of real goals).
        //
        // Suspected 2026-08-13 of costing more than it saves: it filters
        // by TIMING rather than by chance quality, so a speculative
        // 25-yarder locks the team out of a tap-in four seconds later,
        // and across the shot-bar titration ON-TARGET shots FELL 3.0 →
        // 2.4 per team while total shots went 12.6 → 30.1. Tried at 250
        // ticks: goals 1.90 → 2.00 and 0-0 15% → 13%, both inside the
        // n=60 noise floor, and `ball STUCK` went the wrong way (29.9 →
        // 47.8 s/match). Left at 750 for want of evidence — re-test it
        // with n≥300 alongside the chance-supply work.
        let shot_spaced = rebound_live || current_tick.saturating_sub(self.last_shot_tick) >= 750;
        // Build-up gate: a team that just won possession can't fire
        // within ~1 second. Real football: even elite counter-attacks
        // need at least one progressive pass before a shot arrives.
        let settled =
            rebound_live || current_tick.saturating_sub(self.last_possession_gain_tick) >= 100;
        // Possession-phase shot cap: at most TWO shots per possession.
        // Real football: a possession typically produces ONE chance,
        // but rebounds (saved/blocked → ball comes back to attackers)
        // are a real and common path to goals — Klopp-era Liverpool
        // and most pressing teams convert plenty from rebounds.
        // Cap of 1 forbade ALL rebound shots, including legitimate ones
        // where the GK parries to a striker's feet. Cap of 2 paired
        // with the 5s team-shot cooldown still rules out box-scramble
        // spam (4 shots in 2s) but unlocks the realistic "shoot →
        // parry → tap-in" pattern.
        let phase_allows = self.shots_this_possession < 2;
        shot_spaced && settled && phase_allows
    }

    /// Record that this team just won possession. Starts the build-up
    /// gate AND resets the per-possession shot counter.
    pub fn record_possession_gain(&mut self, current_tick: u64) {
        self.last_possession_gain_tick = current_tick;
        self.shots_this_possession = 0;
    }

    pub fn record_shot(&mut self, current_tick: u64) {
        self.last_shot_tick = current_tick;
        self.shots_this_possession += 1;
    }

    /// The line this coach gives up a man from when his side goes short:
    /// the midfield while he is chasing the game, the attack otherwise.
    pub fn spare_line(&self) -> FormationLine {
        if self.stance() > 0.0 {
            FormationLine::Midfield
        } else {
            FormationLine::Attack
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::r#match::engine::teamplay::coach::card::Temperament;

    fn coach(temperament: Temperament) -> MatchCoach {
        let mut card = CoachCard::neutral();
        card.temperament = temperament;
        let mut coach = MatchCoach::new();
        coach.on_kickoff(card, Outlook::default(), &SquadFit::default(), None);
        coach
    }

    fn reader(read: f32) -> Temperament {
        Temperament {
            read,
            ..Temperament::neutral()
        }
    }

    /// Plays level to `minutes`, each minute creating `xg_for` and conceding
    /// `xg_against`.
    fn played(coach: &mut MatchCoach, minutes: u32, xg_for: f32, xg_against: f32) {
        for minute in 0..=minutes {
            let m = minute as f32;
            coach.observe(
                Situation {
                    minutes_left: 90.0 - m,
                    ..Situation::default()
                },
                m,
                ReadCounters {
                    xg_for: xg_for * m,
                    xg_against: xg_against * m,
                    ticks: minute * 6_000,
                    ..ReadCounters::default()
                },
            );
        }
    }

    #[test]
    fn a_read_conceding_the_chances_fails_and_an_even_one_holds() {
        let mut failing = coach(reader(0.5));
        played(&mut failing, 20, 0.0, 0.1);
        assert!(failing.read_failing(20.0));

        let mut healthy = coach(reader(0.5));
        played(&mut healthy, 20, 0.02, 0.02);
        assert!(!healthy.read_failing(20.0));
    }

    #[test]
    fn a_plan_is_given_ten_minutes_before_the_read_can_fail_it() {
        let mut early = coach(reader(0.5));
        played(&mut early, 8, 0.0, 0.3);
        assert!(!early.read_failing(8.0));
    }

    #[test]
    fn a_sharp_reader_sees_a_smaller_shortfall() {
        // Half a goal's worth of chances behind over the read.
        let (mut sharp, mut poor) = (coach(reader(1.0)), coach(reader(0.0)));
        played(&mut sharp, 20, 0.0, 0.5 / 15.0);
        played(&mut poor, 20, 0.0, 0.5 / 15.0);
        assert!(sharp.read_failing(20.0));
        assert!(!poor.read_failing(20.0));
    }

    fn only_442() -> [ShapeChoice; 1] {
        [ShapeChoice {
            shape: MatchTacticType::T442,
            commitment: 0.5,
            familiarity: 1.0,
        }]
    }

    /// The first minute at which a moment finds the plan worth changing, as
    /// a side conceding the chances falls further behind its read.
    fn first_change(temperament: Temperament) -> Option<u32> {
        let mut c = coach(temperament);
        (1..=30).find(|&minute| {
            let m = minute as f32;
            c.observe(
                Situation {
                    minutes_left: 90.0 - m,
                    ..Situation::default()
                },
                m,
                ReadCounters {
                    xg_against: 0.07 * m,
                    ticks: minute * 6_000,
                    ..ReadCounters::default()
                },
            );
            c.on_forced_change();
            c.decide(m, MatchTacticType::T442, &only_442(), 0.25)
                .is_some()
        })
    }

    #[test]
    fn a_stubborn_coach_needs_the_match_to_move_further_than_an_adaptable_one() {
        let adaptable = first_change(Temperament {
            stubbornness: 0.0,
            adaptability: 1.0,
            ..Temperament::neutral()
        })
        .expect("the adaptable coach changes");
        let stubborn = first_change(Temperament {
            stubbornness: 1.0,
            adaptability: 0.0,
            ..Temperament::neutral()
        })
        .unwrap_or(u32::MAX);
        assert!(
            adaptable < stubborn,
            "adaptable {adaptable}', stubborn {stubborn}'"
        );
    }

    #[test]
    fn nothing_changes_between_moments() {
        let mut c = coach(Temperament::neutral());
        let kickoff = c.plan;
        let choices = [ShapeChoice {
            shape: MatchTacticType::T442,
            commitment: 0.5,
            familiarity: 1.0,
        }];
        for minute in 1..=85 {
            let m = minute as f32;
            c.observe(
                Situation {
                    difference: if minute >= 70 { -1 } else { 0 },
                    minutes_left: 90.0 - m,
                    ..Situation::default()
                },
                m,
                ReadCounters::default(),
            );
            assert_eq!(c.decide(m, MatchTacticType::T442, &choices, 1.0), None);
        }
        assert!(c.stance() > 0.5, "the stance has moved: {}", c.stance());
        assert_eq!(c.plan, kickoff);
    }

    /// The coach's decision at half-time after `played` a first half.
    fn at_half_time(
        temperament: Temperament,
        xg_for: f32,
        xg_against: f32,
    ) -> (MatchCoach, Option<PlanChange>) {
        let mut c = coach(temperament);
        played(&mut c, 45, xg_for, xg_against);
        c.on_half_time();
        let change = c.decide(45.0, MatchTacticType::T442, &only_442(), 0.25);
        (c, change)
    }

    #[test]
    fn a_change_keeps_the_push() {
        let (mut changed, change) = at_half_time(Temperament::neutral(), 0.0, 0.08);
        let (mut kept, none) = at_half_time(Temperament::neutral(), 0.02, 0.02);
        assert!(change.is_some() && none.is_none());
        for c in [&mut changed, &mut kept] {
            c.observe(
                Situation {
                    difference: -1,
                    minutes_left: 20.0,
                    ..Situation::default()
                },
                70.0,
                ReadCounters::default(),
            );
        }
        assert!(changed.stance_shift() > 0.0);
        assert_eq!(changed.stance_shift(), kept.stance_shift());
        assert_eq!(
            changed.stance_shift(),
            changed.stance() - changed.kickoff_stance
        );
    }

    #[test]
    fn an_outplayed_side_is_asked_for_less() {
        let (_, change) = at_half_time(Temperament::neutral(), 0.0, 0.08);
        let change = change.expect("a change");
        let (before, after) = (change.plan_before, change.plan_after);
        assert!(after.block_height < before.block_height);
        assert!(after.press < before.press);
        assert!(after.commitment < before.commitment);
    }

    #[test]
    fn a_side_on_top_is_asked_for_more() {
        let (_, change) = at_half_time(Temperament::neutral(), 0.08, 0.0);
        let change = change.expect("a change");
        let (before, after) = (change.plan_before, change.plan_after);
        assert!(after.block_height > before.block_height);
        assert!(after.press > before.press);
        assert!(after.commitment > before.commitment);
    }

    #[test]
    fn a_sharp_reader_answers_further() {
        let moved = |read: f32| {
            at_half_time(reader(read), 0.0, 0.03)
                .1
                .map_or(0.0, |c| c.plan_after.distance(&c.plan_before))
        };
        assert!(
            moved(0.9) > moved(0.2),
            "sharp {}, poor {}",
            moved(0.9),
            moved(0.2)
        );
    }

    #[test]
    fn a_goal_with_a_healthy_read_changes_no_settings() {
        let mut c = coach(Temperament::neutral());
        let kickoff = c.plan;
        played(&mut c, 30, 0.02, 0.02);
        c.on_goal(false, 30.0);
        assert_eq!(
            c.decide(30.0, MatchTacticType::T442, &only_442(), 0.25),
            None
        );
        assert_eq!(c.plan, kickoff);
    }

    #[test]
    fn a_shape_the_side_has_never_played_loses_to_a_familiar_one_as_close() {
        let choice = |shape, commitment, familiarity| ShapeChoice {
            shape,
            commitment,
            familiarity,
        };
        let choices = [
            choice(MatchTacticType::T442, 0.5, 1.0),
            choice(MatchTacticType::T433, 0.62, 0.0),
            choice(MatchTacticType::T352, 0.62, 1.0),
        ];
        let mut c = coach(Temperament::neutral());
        for minute in [58.0, 60.0] {
            c.observe(
                Situation {
                    difference: -1,
                    minutes_left: 90.0 - minute,
                    ..Situation::default()
                },
                minute,
                ReadCounters::default(),
            );
        }
        c.on_forced_change();
        let change = c
            .decide(60.0, MatchTacticType::T442, &choices, 1.0)
            .expect("a change");
        assert_eq!(change.shape_after, MatchTacticType::T352);
    }

    #[test]
    fn each_side_changes_shape_on_its_own_clock() {
        let choices = [
            ShapeChoice {
                shape: MatchTacticType::T442,
                commitment: 0.5,
                familiarity: 1.0,
            },
            ShapeChoice {
                shape: MatchTacticType::T433,
                commitment: 0.62,
                familiarity: 1.0,
            },
        ];
        let chasing_at = |minute: f32| {
            let mut c = coach(Temperament::neutral());
            c.observe(
                Situation {
                    difference: -1,
                    minutes_left: 90.0 - minute,
                    ..Situation::default()
                },
                minute,
                ReadCounters::default(),
            );
            c.observe(
                Situation {
                    difference: -1,
                    minutes_left: 88.0 - minute,
                    ..Situation::default()
                },
                minute + 2.0,
                ReadCounters::default(),
            );
            c.on_forced_change();
            c.decide(minute + 2.0, MatchTacticType::T442, &choices, 1.0)
        };
        let home = chasing_at(58.0).expect("home changes");
        let away = chasing_at(60.0).expect("away changes");
        assert_eq!((home.minute, away.minute), (60, 62));
        assert_eq!(home.shape_after, MatchTacticType::T433);
        assert_eq!(away.shape_after, MatchTacticType::T433);
    }

    /// The tone a coach of `temperament` and `style` takes at the interval,
    /// `difference` goals up with a side `gap` levels stronger.
    fn talk(
        temperament: Temperament,
        style: CoachingStyle,
        gap: f32,
        difference: i32,
    ) -> (TeamTalkTone, MatchCoach) {
        let mut card = CoachCard::neutral();
        card.temperament = temperament;
        card.style = style;
        let mut c = MatchCoach::new();
        let outlook = Outlook {
            strength_gap: gap,
            ..Outlook::default()
        };
        c.on_kickoff(card, outlook, &SquadFit::default(), None);
        c.observe(
            Situation {
                difference,
                minutes_left: 45.0,
                ..Situation::default()
            },
            45.0,
            ReadCounters::default(),
        );
        (c.on_half_time(), c)
    }

    #[test]
    fn the_tone_follows_the_half_and_is_recorded() {
        let neutral = Temperament::neutral();
        let (trailing, coach) = talk(neutral, CoachingStyle::Democratic, 4.0, -1);
        let (leading, _) = talk(neutral, CoachingStyle::Democratic, 4.0, 2);
        assert_eq!(leading, TeamTalkTone::Praise);
        assert_ne!(trailing, leading);
        assert_eq!(coach.record.half_time_talk, Some(trailing));
    }

    #[test]
    fn a_half_that_meets_what_he_expected_is_met_with_calm() {
        let (tone, _) = talk(Temperament::neutral(), CoachingStyle::Democratic, 0.0, 0);
        assert_eq!(tone, TeamTalkTone::TacticalSilent);
    }

    #[test]
    fn below_what_he_expected_each_coach_lifts_them_his_own_way() {
        let with = |volatility, motivating, man_management| Temperament {
            volatility,
            motivating,
            man_management,
            ..Temperament::neutral()
        };
        let below = |t, style| talk(t, style, 0.0, -2).0;
        assert_eq!(
            below(with(0.9, 0.4, 0.4), CoachingStyle::Democratic),
            TeamTalkTone::Criticise
        );
        assert_eq!(
            below(with(0.3, 0.4, 0.4), CoachingStyle::Authoritarian),
            TeamTalkTone::Criticise
        );
        assert_eq!(
            below(with(0.3, 0.9, 0.4), CoachingStyle::Democratic),
            TeamTalkTone::Passionate
        );
        assert_eq!(
            below(with(0.3, 0.4, 0.9), CoachingStyle::Democratic),
            TeamTalkTone::Encourage
        );
    }
}
