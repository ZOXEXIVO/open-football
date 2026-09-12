//! Where a player stands with his manager, and how hard it is to move.
//!
//! Before this, a coach's displeasure was a stateless side-effect of an
//! exponential moving average: a bad month lowered `form_pressure`, selection
//! dropped by a fraction of a slot point, and the week the average recovered
//! the player was back. Nothing anywhere *decided* anything.
//!
//! Real managers decide, remember having decided, and need more evidence to
//! reverse a decision than they needed to make it. That is hysteresis, and
//! it is the whole of this file: a continuous [`CoachStanding::score`] that
//! evidence moves, and a discrete [`StandingRung`] that only changes when
//! the score clears a threshold set well past the one that would put it
//! back. Between the two thresholds the coach holds his position — which is
//! what makes being dropped a thing that happens to a career rather than a
//! thing that happens on a Tuesday.

use super::tuning::StandingTuning;
use chrono::NaiveDate;

/// Where a player stands. Ordered from most to least favoured, so a
/// consumer can compare two players without a lookup table.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default)]
pub enum StandingRung {
    /// He does not leave the side.
    Undroppable,
    /// He plays unless there is a reason.
    Trusted,
    /// The coach likes what he sees.
    InFavour,
    /// Neither one thing nor the other. Where everybody starts.
    #[default]
    Neutral,
    /// The coach is thinking about him, and not warmly.
    UnderReview,
    /// He has stopped being picked.
    OutOfFavour,
    /// He has stopped being considered.
    FrozenOut,
}

impl StandingRung {
    /// Roles that get a man picked ahead of an equal.
    #[inline]
    pub fn is_favoured(self) -> bool {
        matches!(self, Self::Undroppable | Self::Trusted | Self::InFavour)
    }

    /// Roles that keep a man out of the side.
    #[inline]
    pub fn is_out(self) -> bool {
        matches!(self, Self::OutOfFavour | Self::FrozenOut)
    }

    /// Signed shift on the coach's preference to start him.
    pub fn selection_shift(self) -> f32 {
        match self {
            Self::Undroppable => StandingTuning::SHIFT_UNDROPPABLE,
            Self::Trusted => StandingTuning::SHIFT_TRUSTED,
            Self::InFavour => StandingTuning::SHIFT_IN_FAVOUR,
            Self::Neutral => 0.0,
            Self::UnderReview => StandingTuning::SHIFT_UNDER_REVIEW,
            Self::OutOfFavour => StandingTuning::SHIFT_OUT_OF_FAVOUR,
            Self::FrozenOut => StandingTuning::SHIFT_FROZEN_OUT,
        }
    }

    /// Signed shift on the coach's preference to name him in the eighteen.
    pub fn bench_shift(self) -> f32 {
        match self {
            Self::Undroppable => StandingTuning::BENCH_UNDROPPABLE,
            Self::Trusted => StandingTuning::BENCH_TRUSTED,
            Self::InFavour => StandingTuning::BENCH_IN_FAVOUR,
            Self::Neutral => 0.0,
            Self::UnderReview => StandingTuning::BENCH_UNDER_REVIEW,
            Self::OutOfFavour => StandingTuning::BENCH_OUT_OF_FAVOUR,
            Self::FrozenOut => StandingTuning::BENCH_FROZEN_OUT,
        }
    }

    /// How much of the ordinary form pressure still reaches a man on this
    /// rung. A coach backs the players he trusts through a slump.
    pub fn form_dampener(self) -> f32 {
        match self {
            Self::Undroppable => StandingTuning::UNDROPPABLE_FORM_DAMPENER,
            Self::Trusted => StandingTuning::TRUSTED_FORM_DAMPENER,
            _ => 1.0,
        }
    }

    pub fn as_i18n_key(self) -> &'static str {
        match self {
            Self::Undroppable => "standing_undroppable",
            Self::Trusted => "standing_trusted",
            Self::InFavour => "standing_in_favour",
            Self::Neutral => "standing_neutral",
            Self::UnderReview => "standing_under_review",
            Self::OutOfFavour => "standing_out_of_favour",
            Self::FrozenOut => "standing_frozen_out",
        }
    }
}

/// The things a coach is holding against a player right now. Distinct from
/// the dossier's scars, which are what is left years later; these are live,
/// and some of them override the score outright.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct GrievanceFlags(u8);

impl GrievanceFlags {
    /// A big-match failure is on the record and the window is filling.
    pub const BIG_MATCH_PENDING: u8 = 1 << 0;
    /// He cost us on the night that counted.
    pub const COST_THE_OCCASION: u8 = 1 << 1;
    /// Cards, fines, a problem in the dressing room.
    pub const INDISCIPLINE: u8 = 1 << 2;
    /// He would not play.
    pub const REFUSED: u8 = 1 << 3;
    /// He asked out.
    pub const WANTS_OUT: u8 = 1 << 4;
    /// He took it to the press.
    pub const WENT_PUBLIC: u8 = 1 << 5;
    /// He has run out of chances in the big matches.
    pub const BIG_MATCH_UNTRUSTED: u8 = 1 << 6;

    /// Grievances that freeze a man out whatever the score says.
    pub const UNFORGIVABLE: u8 = Self::REFUSED | Self::WENT_PUBLIC;

    #[inline]
    pub fn contains(self, flag: u8) -> bool {
        self.0 & flag != 0
    }

    #[inline]
    pub fn insert(&mut self, flag: u8) {
        self.0 |= flag;
    }

    #[inline]
    pub fn remove(&mut self, flag: u8) {
        self.0 &= !flag;
    }

    #[inline]
    pub fn bits(self) -> u8 {
        self.0
    }

    #[inline]
    pub fn is_empty(self) -> bool {
        self.0 == 0
    }
}

/// A player's standing with one coach, during one spell.
///
/// Lives inside [`CoachMemory`] and dies with it: this is a live reading of
/// a working relationship, and when the relationship ends it is consolidated
/// into the dossier rather than carried around.
///
/// [`CoachMemory`]: crate::club::staff::coach::CoachMemory
#[derive(Debug, Clone, Copy)]
pub struct CoachStanding {
    /// −1..=1. What the evidence says.
    pub score: f32,
    /// What the coach has actually decided, which lags the evidence in both
    /// directions.
    pub rung: StandingRung,
    /// When he decided it.
    pub since: Option<NaiveDate>,
    /// 0..1. How much of the coach's attention this player is taking up.
    /// It is what makes a manager act on a bad night rather than sleep on
    /// it, and it cools when a player does what was expected of him.
    pub heat: f32,
    /// 0..1. "I owe him a start." Broken promises and unfair drops put it
    /// there; a man-manager pays it.
    pub debt: f32,
    pub grievance: GrievanceFlags,
    /// Sliding window of big-match starts: bit set means he failed.
    pub big_match_window: u8,
    /// Consecutive big-match starts he has answered with since.
    pub big_match_answers: u8,
    /// Matches on the current rung.
    pub matches_this_rung: u8,
    /// Consecutive weeks of training at one extreme, signed by direction.
    pub training_run: i8,
    /// Standing accrued from training this month, so a run cannot become an
    /// unbounded ratchet.
    pub training_month: f32,
    /// Until when the coach is backing him through anything, after he
    /// repaid the faith.
    pub protected_until: Option<NaiveDate>,
    /// Matches of real form pressure a favourite has been carried through.
    pub carried_matches: u8,
    /// He was out of the side under this coach and played his way back in.
    /// A latch: the fact survives whatever happens afterwards, because
    /// what it records is that he did it once.
    pub bounced_back: bool,
}

impl Default for CoachStanding {
    fn default() -> Self {
        CoachStanding {
            score: 0.0,
            rung: StandingRung::Neutral,
            since: None,
            heat: 0.0,
            debt: 0.0,
            grievance: GrievanceFlags::default(),
            big_match_window: 0,
            big_match_answers: 0,
            matches_this_rung: 0,
            training_run: 0,
            training_month: 0.0,
            protected_until: None,
            carried_matches: 0,
            bounced_back: false,
        }
    }
}

impl CoachStanding {
    /// A standing seeded from what a coach already knew about a man he has
    /// worked with before. No hysteresis: a fresh spell starts wherever the
    /// evidence says, because there is no decision yet to hold to.
    pub fn seeded(score: f32, grievance: GrievanceFlags, today: NaiveDate) -> Self {
        let score = score.clamp(-1.0, 1.0);
        let mut standing = CoachStanding {
            score,
            grievance,
            since: Some(today),
            ..CoachStanding::default()
        };
        standing.rung = StandingLadder::rung_for_score(score);
        // A live grievance he has not forgotten puts a man under a cloud
        // from day one — but not out of the side before he has kicked a
        // ball. Second chances exist; they are simply watched.
        if grievance.contains(GrievanceFlags::UNFORGIVABLE)
            && standing.rung < StandingRung::OutOfFavour
        {
            standing.rung = StandingRung::OutOfFavour;
        }
        standing
    }

    /// Is the coach carrying him through a rough patch on credit?
    pub fn is_protected(&self, today: NaiveDate) -> bool {
        self.protected_until.is_some_and(|until| today <= until)
    }

    /// How much of the ordinary form pressure reaches him, all things
    /// considered.
    pub fn form_dampener(&self, today: NaiveDate) -> f32 {
        let rung = self.rung.form_dampener();
        if self.is_protected(today) {
            rung.min(StandingTuning::PROTECTED_FORM_DAMPENER)
        } else {
            rung
        }
    }

    /// Does the coach owe him a start he is in a position to pay?
    pub fn owes_a_start(&self, man_management: f32, match_importance: f32) -> bool {
        self.debt >= StandingTuning::SECOND_CHANCE_DEBT
            && man_management >= StandingTuning::SECOND_CHANCE_MAN_MANAGEMENT
            && match_importance < StandingTuning::SECOND_CHANCE_IMPORTANCE
    }

    /// Has he run out of big-match chances?
    #[inline]
    pub fn is_big_match_untrusted(&self) -> bool {
        self.grievance
            .contains(GrievanceFlags::BIG_MATCH_UNTRUSTED)
    }
}

/// The rules that move a standing: how an impulse lands, when a rung
/// changes, and what a rung refuses to change into.
pub struct StandingLadder;

impl StandingLadder {
    /// Where a score sits when there is no standing decision to hold to —
    /// a fresh spell, or a reunion.
    pub fn rung_for_score(score: f32) -> StandingRung {
        if score >= StandingTuning::ENTER_UNDROPPABLE {
            // Never on day one: the matches gate in `settle` holds it to
            // Trusted until he has watched enough. Seeding deliberately
            // stops here too.
            StandingRung::Trusted
        } else if score >= StandingTuning::ENTER_TRUSTED {
            StandingRung::Trusted
        } else if score >= StandingTuning::ENTER_IN_FAVOUR {
            StandingRung::InFavour
        } else if score <= StandingTuning::ENTER_FROZEN_OUT {
            StandingRung::FrozenOut
        } else if score <= StandingTuning::ENTER_OUT_OF_FAVOUR {
            StandingRung::OutOfFavour
        } else if score <= StandingTuning::ENTER_UNDER_REVIEW {
            StandingRung::UnderReview
        } else {
            StandingRung::Neutral
        }
    }

    /// Apply one signed impulse, already personality-scaled by the caller.
    /// Heat rises with the size of the shock in either direction — a coach
    /// notices a man who surprises him, good or bad.
    pub fn nudge(standing: &mut CoachStanding, impulse: f32) {
        standing.score = (standing.score + impulse).clamp(-1.0, 1.0);
        standing.heat =
            (standing.heat + impulse.abs() * StandingTuning::HEAT_PER_IMPULSE).clamp(0.0, 1.0);
    }

    /// A match has been played and the impulses for it applied. Cool the
    /// heat, count the match on the rung, and let the rung catch up.
    pub fn after_match(standing: &mut CoachStanding, context: &LadderContext, today: NaiveDate) {
        standing.heat *= StandingTuning::HEAT_DECAY_PER_MATCH;
        standing.matches_this_rung = standing.matches_this_rung.saturating_add(1);
        Self::settle(standing, context, today);
    }

    /// Time has passed without the coach seeing him. The score drifts back
    /// toward nothing and the heat goes out of it — an old grudge against a
    /// man you never see is not a grudge, it is a note.
    pub fn decay(standing: &mut CoachStanding, months: f32, context: &LadderContext, today: NaiveDate) {
        if months <= 0.0 {
            return;
        }
        let drift = StandingTuning::IDLE_DRIFT_PER_MONTH * months;
        if standing.score > 0.0 {
            standing.score = (standing.score - drift).max(0.0);
        } else if standing.score < 0.0 {
            standing.score = (standing.score + drift).min(0.0);
        }
        standing.heat *= StandingTuning::HEAT_DECAY_PER_IDLE_MONTH.powf(months);
        standing.training_month = 0.0;
        Self::settle(standing, context, today);
    }

    /// Let the decision catch up with the evidence, if it is entitled to.
    ///
    /// Everything that makes this hysteresis rather than a lookup lives
    /// here: separate enter and leave thresholds, minimum stays, the
    /// grievances that override the score, and the floors that stop a coach
    /// freezing out a man he cannot afford to lose.
    pub fn settle(standing: &mut CoachStanding, context: &LadderContext, today: NaiveDate) {
        let target = Self::target_rung(standing, context, today);
        if target == standing.rung {
            return;
        }
        if !Self::may_leave(standing, context, today) {
            return;
        }
        // Climbing out of the cold back into the side is a thing a player
        // did, and it stays on the record.
        if standing.rung.is_out() && target.is_favoured() {
            standing.bounced_back = true;
        }
        standing.rung = target;
        standing.since = Some(today);
        standing.matches_this_rung = 0;
        standing.carried_matches = 0;
    }

    /// Where the evidence says he belongs, before the minimum stays.
    fn target_rung(
        standing: &CoachStanding,
        context: &LadderContext,
        today: NaiveDate,
    ) -> StandingRung {
        // ── The overrides ──
        // A refusal or a public row is not a score, it is a fact, and it
        // ends the argument.
        if standing
            .grievance
            .contains(GrievanceFlags::UNFORGIVABLE)
        {
            return Self::floor(StandingRung::FrozenOut, context);
        }

        let score = standing.score;
        let current = standing.rung;

        // ── Climbing ──
        if score >= StandingTuning::ENTER_UNDROPPABLE
            && context.matches_observed >= StandingTuning::UNDROPPABLE_MATCHES
        {
            return StandingRung::Undroppable;
        }
        if score >= StandingTuning::ENTER_TRUSTED {
            // Leaving Undroppable needs the lower threshold to be cleared.
            if current == StandingRung::Undroppable && score >= StandingTuning::LEAVE_UNDROPPABLE {
                return StandingRung::Undroppable;
            }
            return StandingRung::Trusted;
        }
        if score >= StandingTuning::ENTER_IN_FAVOUR {
            if current == StandingRung::Trusted && score >= StandingTuning::LEAVE_TRUSTED {
                return StandingRung::Trusted;
            }
            if current == StandingRung::Undroppable && score >= StandingTuning::LEAVE_UNDROPPABLE {
                return StandingRung::Undroppable;
            }
            return StandingRung::InFavour;
        }

        // ── Falling ──
        if score <= StandingTuning::ENTER_FROZEN_OUT {
            return Self::floor(StandingRung::FrozenOut, context);
        }
        // A long, settled spell out of favour hardens under a stubborn man.
        if current == StandingRung::OutOfFavour
            && score <= StandingTuning::OUT_OF_FAVOUR_TO_FROZEN_SCORE
            && context.stubbornness >= StandingTuning::OUT_OF_FAVOUR_TO_FROZEN_STUBBORNNESS
            && Self::days_on_rung(standing, today) >= StandingTuning::OUT_OF_FAVOUR_TO_FROZEN_DAYS
        {
            return Self::floor(StandingRung::FrozenOut, context);
        }
        if score <= StandingTuning::ENTER_OUT_OF_FAVOUR {
            return Self::floor(StandingRung::OutOfFavour, context);
        }
        // A review that keeps going one way ends the same place.
        if current == StandingRung::UnderReview
            && standing.matches_this_rung >= StandingTuning::REVIEW_TO_OUT_OF_FAVOUR_MATCHES
            && score <= StandingTuning::REVIEW_TO_OUT_OF_FAVOUR_SCORE
        {
            return Self::floor(StandingRung::OutOfFavour, context);
        }
        if score <= StandingTuning::ENTER_UNDER_REVIEW
            || (standing.heat >= StandingTuning::HEAT_UNDER_REVIEW
                && score <= StandingTuning::HEAT_UNDER_REVIEW_SCORE)
        {
            return Self::floor(StandingRung::UnderReview, context);
        }

        // ── Holding ──
        // Between the thresholds, a decision already taken stands.
        match current {
            StandingRung::FrozenOut if score <= StandingTuning::LEAVE_FROZEN_OUT => current,
            StandingRung::OutOfFavour if score <= StandingTuning::LEAVE_OUT_OF_FAVOUR => current,
            StandingRung::UnderReview if score <= StandingTuning::LEAVE_UNDER_REVIEW => current,
            StandingRung::InFavour if score >= StandingTuning::LEAVE_IN_FAVOUR => current,
            StandingRung::Trusted if score >= StandingTuning::LEAVE_TRUSTED => current,
            StandingRung::Undroppable if score >= StandingTuning::LEAVE_UNDROPPABLE => current,
            _ => StandingRung::Neutral,
        }
    }

    /// The rungs a coach is not free to use, whatever he thinks.
    fn floor(target: StandingRung, context: &LadderContext) -> StandingRung {
        // A squad too thin to leave anyone out does not leave anyone out.
        if target == StandingRung::FrozenOut && context.fit_seniors < StandingTuning::FROZEN_SQUAD_FLOOR
        {
            return StandingRung::OutOfFavour;
        }
        // A manager who wants to freeze out his own captain takes the
        // armband off him first.
        if context.is_captain && target.is_out() {
            return StandingRung::UnderReview;
        }
        // A patient coach gives a young player a run before judging him.
        if context.in_youth_grace && target.is_out() {
            return StandingRung::UnderReview;
        }
        target
    }

    /// Has he served the minimum on the rung he is on?
    fn may_leave(standing: &CoachStanding, context: &LadderContext, today: NaiveDate) -> bool {
        let days = Self::days_on_rung(standing, today);
        match standing.rung {
            StandingRung::UnderReview => {
                standing.matches_this_rung >= StandingTuning::MIN_STAY_UNDER_REVIEW_MATCHES
            }
            StandingRung::OutOfFavour => {
                let matches_needed = if context.stubbornness >= StandingTuning::STUBBORN {
                    StandingTuning::MIN_STAY_OUT_OF_FAVOUR_MATCHES_STUBBORN
                } else {
                    StandingTuning::MIN_STAY_OUT_OF_FAVOUR_MATCHES
                };
                // Falling further needs no patience; climbing back does.
                if standing.score <= StandingTuning::ENTER_FROZEN_OUT {
                    return true;
                }
                days >= StandingTuning::MIN_STAY_OUT_OF_FAVOUR_DAYS
                    && standing.matches_this_rung >= matches_needed
                    && context.has_recovery_signal
            }
            StandingRung::FrozenOut => {
                if days < StandingTuning::MIN_STAY_FROZEN_DAYS {
                    return false;
                }
                if standing
                    .grievance
                    .contains(GrievanceFlags::UNFORGIVABLE)
                {
                    return false;
                }
                context.cleared_the_air
                    || (context.man_management >= StandingTuning::THAW_MAN_MANAGEMENT
                        && days >= StandingTuning::MIN_STAY_FROZEN_DAYS + StandingTuning::THAW_DAYS)
            }
            _ => true,
        }
    }

    fn days_on_rung(standing: &CoachStanding, today: NaiveDate) -> i64 {
        standing
            .since
            .map(|since| (today - since).num_days())
            .unwrap_or(i64::MAX)
    }

    /// Record a big-match start and decide whether the coach has run out of
    /// chances to give him.
    pub fn note_big_match(
        standing: &mut CoachStanding,
        rating: f32,
        context: &LadderContext,
    ) {
        let failed = rating < StandingTuning::BIG_MATCH_FAILURE_RATING;
        standing.big_match_window = ((standing.big_match_window << 1) | u8::from(failed))
            & StandingTuning::BIG_MATCH_WINDOW_MASK;

        if failed {
            standing.big_match_answers = 0;
            standing
                .grievance
                .insert(GrievanceFlags::BIG_MATCH_PENDING);
        } else if rating >= StandingTuning::BIG_MATCH_REDEMPTION_RATING {
            standing.big_match_answers = standing.big_match_answers.saturating_add(1);
        }

        let failures = standing.big_match_window.count_ones();
        let needed = Self::big_match_tolerance(context);

        if failures >= needed && !standing.is_big_match_untrusted() {
            // A warm coach gives a man he trusts one more chance at it.
            let grace = context.man_management >= StandingTuning::BIG_MATCH_GRACE_MAN_MANAGEMENT
                && standing.rung.is_favoured()
                && failures == needed;
            if !grace {
                standing
                    .grievance
                    .insert(GrievanceFlags::BIG_MATCH_UNTRUSTED);
            }
        }

        if standing.is_big_match_untrusted()
            && standing.big_match_answers >= StandingTuning::BIG_MATCH_REDEMPTION_RUN
        {
            standing
                .grievance
                .remove(GrievanceFlags::BIG_MATCH_UNTRUSTED);
            standing.grievance.remove(GrievanceFlags::BIG_MATCH_PENDING);
            standing.big_match_window = 0;
        }
    }

    /// How many big-match failures this coach tolerates.
    fn big_match_tolerance(context: &LadderContext) -> u32 {
        if context.volatility >= StandingTuning::BIG_MATCH_VOLATILITY {
            StandingTuning::BIG_MATCH_FAILURES_VOLATILE
        } else if context.judging_accuracy >= StandingTuning::BIG_MATCH_JUDGING {
            StandingTuning::BIG_MATCH_FAILURES_PATIENT
        } else {
            StandingTuning::BIG_MATCH_FAILURES
        }
    }

    /// A favourite who keeps under-performing is eventually dropped, and the
    /// point at which that happens is a fact about the manager.
    pub fn carry_or_drop(
        standing: &mut CoachStanding,
        under_pressure: bool,
        context: &LadderContext,
    ) {
        if !standing.rung.is_favoured() || !under_pressure {
            if !under_pressure {
                standing.carried_matches = 0;
            }
            return;
        }
        standing.carried_matches = standing.carried_matches.saturating_add(1);
        let cap = if context.judging_accuracy >= StandingTuning::FAVOURITE_CAP_JUDGING {
            StandingTuning::FAVOURITE_CAP_MATCHES_SHARP
        } else {
            StandingTuning::FAVOURITE_CAP_MATCHES
        };
        if standing.carried_matches >= cap {
            Self::nudge(standing, StandingTuning::FAVOURITE_CAP_PENALTY);
            standing.carried_matches = 0;
        }
    }
}

/// What the ladder needs to know about the coach, the player and the squad
/// that it cannot read off the standing itself. Gathered by the caller, so
/// the ladder never walks the simulator graph.
#[derive(Debug, Clone, Copy)]
pub struct LadderContext {
    pub stubbornness: f32,
    pub man_management: f32,
    pub volatility: f32,
    pub judging_accuracy: f32,
    /// Matches of this player the coach has watched.
    pub matches_observed: u16,
    /// Fit senior outfielders available to him.
    pub fit_seniors: usize,
    pub is_captain: bool,
    /// A young player inside the run a patient coach gives him.
    pub in_youth_grace: bool,
    /// Cameos, training, or a talk that says he is working his way back.
    pub has_recovery_signal: bool,
    /// A conversation happened and it went well.
    pub cleared_the_air: bool,
}

impl Default for LadderContext {
    fn default() -> Self {
        LadderContext {
            stubbornness: 0.5,
            man_management: 0.5,
            volatility: 0.5,
            judging_accuracy: 0.5,
            matches_observed: 0,
            // Assume a full squad: the floor exists to stop a coach
            // freezing out a man he cannot replace, and a caller with no
            // squad to declare is not in that situation.
            fit_seniors: StandingTuning::FROZEN_SQUAD_FLOOR,
            is_captain: false,
            in_youth_grace: false,
            has_recovery_signal: false,
            cleared_the_air: false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Fixture builders, grouped so the tests read as sentences.
    struct Fx;

    impl Fx {
        fn date(day: u32) -> NaiveDate {
            NaiveDate::from_ymd_opt(2030, 1, 1).unwrap() + chrono::Duration::days(day as i64)
        }

        fn context() -> LadderContext {
            LadderContext {
                matches_observed: 30,
                ..LadderContext::default()
            }
        }

        fn stubborn() -> LadderContext {
            LadderContext {
                stubbornness: 0.8,
                ..Self::context()
            }
        }

        /// Push the score to `target` over `matches` matches, settling each.
        fn drive(standing: &mut CoachStanding, target: f32, context: &LadderContext, from: u32) {
            let step = (target - standing.score) / 10.0;
            for match_number in 0..10 {
                StandingLadder::nudge(standing, step);
                StandingLadder::after_match(standing, context, Fx::date(from + match_number * 7));
            }
        }
    }

    #[test]
    fn everybody_starts_neutral_and_a_neutral_standing_changes_nothing() {
        let standing = CoachStanding::default();
        assert_eq!(standing.rung, StandingRung::Neutral);
        assert_eq!(standing.rung.selection_shift(), 0.0);
        assert_eq!(standing.rung.bench_shift(), 0.0);
        assert_eq!(standing.rung.form_dampener(), 1.0);
    }

    #[test]
    fn a_run_of_bad_matches_costs_a_man_his_place_and_it_takes_longer_to_get_back() {
        let context = Fx::context();
        let mut standing = CoachStanding::default();

        Fx::drive(&mut standing, -0.5, &context, 0);
        assert_eq!(
            standing.rung,
            StandingRung::OutOfFavour,
            "score {}",
            standing.score
        );

        // The evidence turns, but without a recovery signal he stays out.
        Fx::drive(&mut standing, 0.1, &context, 200);
        assert_eq!(
            standing.rung,
            StandingRung::OutOfFavour,
            "a decision is not reversed by the score alone"
        );

        // With one, he comes back.
        let recovering = LadderContext {
            has_recovery_signal: true,
            ..context
        };
        StandingLadder::settle(&mut standing, &recovering, Fx::date(400));
        assert!(
            !standing.rung.is_out(),
            "two good cameos win his place back: {:?}",
            standing.rung
        );
    }

    #[test]
    fn a_stubborn_coach_needs_longer_to_change_his_mind() {
        let patient = LadderContext {
            has_recovery_signal: true,
            ..Fx::context()
        };
        let stubborn = LadderContext {
            has_recovery_signal: true,
            ..Fx::stubborn()
        };

        let mut quick = CoachStanding::default();
        let mut slow = CoachStanding::default();
        for standing in [&mut quick, &mut slow] {
            StandingLadder::nudge(standing, -0.5);
        }
        StandingLadder::settle(&mut quick, &patient, Fx::date(0));
        StandingLadder::settle(&mut slow, &stubborn, Fx::date(0));
        assert!(quick.rung.is_out() && slow.rung.is_out());

        // Same evidence, same time, three matches each.
        for (standing, context) in [(&mut quick, &patient), (&mut slow, &stubborn)] {
            StandingLadder::nudge(standing, 0.3);
            for day in 0..3 {
                StandingLadder::after_match(standing, context, Fx::date(30 + day * 7));
            }
        }
        StandingLadder::settle(&mut quick, &patient, Fx::date(60));
        StandingLadder::settle(&mut slow, &stubborn, Fx::date(60));

        assert!(!quick.rung.is_out(), "three matches was enough for him");
        assert!(
            slow.rung.is_out(),
            "and not for the stubborn one: {:?}",
            slow.rung
        );
    }

    #[test]
    fn a_refusal_to_play_ends_the_argument_whatever_the_score_says() {
        let context = Fx::context();
        let mut standing = CoachStanding::default();
        Fx::drive(&mut standing, 0.8, &context, 0);
        assert!(standing.rung.is_favoured());

        standing.grievance.insert(GrievanceFlags::REFUSED);
        StandingLadder::settle(&mut standing, &context, Fx::date(100));
        assert_eq!(standing.rung, StandingRung::FrozenOut);

        // And nothing brings him back while it stands.
        StandingLadder::nudge(&mut standing, 1.0);
        StandingLadder::settle(&mut standing, &context, Fx::date(400));
        assert_eq!(standing.rung, StandingRung::FrozenOut);
    }

    #[test]
    fn a_squad_too_thin_to_leave_anyone_out_does_not_leave_anyone_out() {
        let thin = LadderContext {
            fit_seniors: 11,
            ..Fx::context()
        };
        let mut standing = CoachStanding::default();
        StandingLadder::nudge(&mut standing, -0.9);
        StandingLadder::settle(&mut standing, &thin, Fx::date(0));
        assert_eq!(
            standing.rung,
            StandingRung::OutOfFavour,
            "needs must — he is still in the eighteen"
        );
    }

    #[test]
    fn a_manager_takes_the_armband_off_a_man_before_freezing_him_out() {
        let captain = LadderContext {
            is_captain: true,
            ..Fx::context()
        };
        let mut standing = CoachStanding::default();
        StandingLadder::nudge(&mut standing, -0.9);
        StandingLadder::settle(&mut standing, &captain, Fx::date(0));
        assert_eq!(standing.rung, StandingRung::UnderReview);
    }

    #[test]
    fn a_second_big_match_failure_costs_him_the_next_one() {
        let context = Fx::context();
        let mut standing = CoachStanding::default();

        StandingLadder::note_big_match(&mut standing, 5.0, &context);
        assert!(
            !standing.is_big_match_untrusted(),
            "one bad night is one bad night"
        );

        StandingLadder::note_big_match(&mut standing, 5.2, &context);
        assert!(standing.is_big_match_untrusted());
    }

    #[test]
    fn a_volatile_coach_needs_only_one() {
        let volatile = LadderContext {
            volatility: 0.85,
            ..Fx::context()
        };
        let mut standing = CoachStanding::default();
        StandingLadder::note_big_match(&mut standing, 5.0, &volatile);
        assert!(standing.is_big_match_untrusted());
    }

    #[test]
    fn a_good_judge_waits_for_a_third() {
        let sharp = LadderContext {
            judging_accuracy: 0.9,
            ..Fx::context()
        };
        let mut standing = CoachStanding::default();
        StandingLadder::note_big_match(&mut standing, 5.0, &sharp);
        StandingLadder::note_big_match(&mut standing, 5.0, &sharp);
        assert!(!standing.is_big_match_untrusted());
        StandingLadder::note_big_match(&mut standing, 5.0, &sharp);
        assert!(standing.is_big_match_untrusted());
    }

    #[test]
    fn two_answers_on_the_big_nights_win_the_trust_back() {
        let context = Fx::context();
        let mut standing = CoachStanding::default();
        StandingLadder::note_big_match(&mut standing, 5.0, &context);
        StandingLadder::note_big_match(&mut standing, 5.0, &context);
        assert!(standing.is_big_match_untrusted());

        StandingLadder::note_big_match(&mut standing, 7.0, &context);
        assert!(standing.is_big_match_untrusted(), "one is not enough");
        StandingLadder::note_big_match(&mut standing, 7.1, &context);
        assert!(!standing.is_big_match_untrusted());
    }

    #[test]
    fn a_favourite_survives_five_bad_games_and_not_eight() {
        let context = Fx::context();
        let mut standing = CoachStanding::default();
        Fx::drive(&mut standing, 0.55, &context, 0);
        assert_eq!(standing.rung, StandingRung::Trusted);
        let before = standing.score;

        for _ in 0..5 {
            StandingLadder::carry_or_drop(&mut standing, true, &context);
        }
        assert_eq!(standing.score, before, "he is still backing him");

        for _ in 0..3 {
            StandingLadder::carry_or_drop(&mut standing, true, &context);
        }
        assert!(
            standing.score < before,
            "eight is where even a favourite runs out: {} → {}",
            before,
            standing.score
        );
    }

    #[test]
    fn a_sharp_judge_sees_it_three_matches_sooner() {
        let sharp = LadderContext {
            judging_accuracy: 0.8,
            ..Fx::context()
        };
        let mut standing = CoachStanding::default();
        Fx::drive(&mut standing, 0.55, &sharp, 0);
        let before = standing.score;
        for _ in 0..5 {
            StandingLadder::carry_or_drop(&mut standing, true, &sharp);
        }
        assert!(standing.score < before);
    }

    #[test]
    fn a_standing_nobody_adds_to_drifts_back_toward_nothing() {
        let context = Fx::context();
        let mut standing = CoachStanding::default();
        Fx::drive(&mut standing, -0.5, &context, 0);
        let out_of_favour = standing.score;

        StandingLadder::decay(&mut standing, 12.0, &context, Fx::date(400));
        assert!(
            standing.score > out_of_favour,
            "a year away softens it: {} → {}",
            out_of_favour,
            standing.score
        );
        assert!(standing.heat < 0.05);
    }

    #[test]
    fn a_trusted_man_feels_less_of_the_form_pressure() {
        let context = Fx::context();
        let mut standing = CoachStanding::default();
        Fx::drive(&mut standing, 0.5, &context, 0);
        assert!(standing.form_dampener(Fx::date(0)) < 1.0);
    }

    #[test]
    fn repaying_the_coachs_faith_buys_protection_through_the_next_dip() {
        let mut standing = CoachStanding::default();
        assert_eq!(standing.form_dampener(Fx::date(0)), 1.0);
        standing.protected_until = Some(Fx::date(90));
        assert!(standing.form_dampener(Fx::date(30)) < 1.0);
        assert_eq!(standing.form_dampener(Fx::date(120)), 1.0);
    }

    #[test]
    fn a_seeded_standing_starts_where_the_evidence_says_with_no_decision_to_hold_to() {
        let warm = CoachStanding::seeded(0.4, GrievanceFlags::default(), Fx::date(0));
        assert_eq!(warm.rung, StandingRung::Trusted);
        assert_eq!(warm.matches_this_rung, 0);

        let mut grievance = GrievanceFlags::default();
        grievance.insert(GrievanceFlags::REFUSED);
        let sour = CoachStanding::seeded(0.0, grievance, Fx::date(0));
        assert_eq!(
            sour.rung,
            StandingRung::OutOfFavour,
            "a second chance, and a watched one"
        );
    }
}
