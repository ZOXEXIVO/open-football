//! One coach's read of one player.
//!
//! The organ the player mind has no equivalent of: a persistent,
//! revisable opinion about **someone else's ability**, which can be
//! wrong.
//!
//! Two properties neither `CoachMemory` nor `CoachDecisionState` has
//! today, and both are the point:
//!
//! * **It survives the job.** A manager who rated a player at one club
//!   still rates him at the next, which is how real managers sign the
//!   same players over and over. The store lives on the man.
//! * **It can be scored.** [`JudgementOutcome`] closes the loop — a
//!   coach who wrote off a player who then became very good learns
//!   something from having been wrong.
//!
//! 28 bytes, `Copy`, packed the same way an episode is: `u8` percentages
//! where 1% resolution is plenty, and the mind's own [`EpochDay`] clock
//! instead of a `NaiveDate`.

use crate::club::mind::organs::memory::{ActorRef, EpochDay, MindClock};

/// Whether a judgement was borne out.
///
/// `Open` until the player's career settles the question — which for a
/// young player is years after the coach first formed a view, and that
/// delay is exactly what makes the verdict worth anything.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum JudgementOutcome {
    /// Not yet settled.
    #[default]
    Open,
    /// He was right about him.
    Vindicated,
    /// He was wrong. Feeds `IWasWrongAboutHim` and nudges the lens
    /// toward patience.
    Wrong,
}

impl JudgementOutcome {
    #[inline]
    pub fn is_settled(self) -> bool {
        !matches!(self, JudgementOutcome::Open)
    }

    pub fn as_i18n_key(self) -> &'static str {
        match self {
            JudgementOutcome::Open => "mind_judgement_open",
            JudgementOutcome::Vindicated => "mind_judgement_vindicated",
            JudgementOutcome::Wrong => "mind_judgement_wrong",
        }
    }
}

/// What a coach thinks of a player.
#[derive(Debug, Clone, Copy, Default)]
pub struct PlayerJudgement {
    /// Who this is about. Always [`ActorRef::player`].
    pub player: ActorRef,

    /// What he thinks the player is worth now, 0..=100 of the
    /// observable band.
    level_pct: u8,
    /// What he thinks the player will become. Never below `level_pct`
    /// — a ceiling under a floor is not an opinion, it is a bug.
    ceiling_pct: u8,
    /// How sure he is, 0..=100. Rises with matches watched, falls with
    /// time apart. The difference between "I don't rate him" and "I
    /// don't know him".
    confidence_pct: u8,

    /// The four trust axes `CoachMemory` already models, kept as they
    /// are because they are already right — only their lifetime changes.
    tactical_trust_pct: u8,
    big_match_trust_pct: u8,
    training_trust_pct: u8,
    professionalism_pct: u8,

    /// The accumulator that currently dies with `CoachDecisionState`.
    /// How hot this player is running in the coach's head right now —
    /// it is what makes a manager act on a bad night rather than sleep
    /// on it.
    heat_pct: u8,

    /// Form as *this coach* reads it: the last few performances against
    /// what he expected of this player, not against the league.
    /// −100..=100.
    recent_form_pct: i8,
    /// The long-run baseline he holds the player to, in tenths of a
    /// match rating (0..=100 → 0.0..10.0).
    long_form_tenths: u8,

    /// Matches watched. Saturates — the difference between eighty and
    /// eight hundred is not a difference of opinion.
    pub observed: u16,
    /// When the view was first formed.
    pub formed: EpochDay,
    /// Last time he saw the player. Confidence fades from here.
    pub last_seen: EpochDay,

    /// Was he right? See [`JudgementOutcome`].
    pub outcome: JudgementOutcome,
}

impl PlayerJudgement {
    /// Confidence gained per match watched, as a fraction of the gap to
    /// certainty. Diminishing: the fourth viewing tells him far more
    /// than the fortieth.
    pub const VIEWING_GAIN: f32 = 0.16;

    /// Confidence lost per year apart, as a fraction of what is left.
    /// A manager does not forget what he thought of a player; he stops
    /// being sure it still holds.
    pub const ABSENCE_LOSS_PER_YEAR: f32 = 0.35;

    /// How much a settled judgement holds its confidence against
    /// absence. A view that was proved right does not fade the way a
    /// guess does.
    pub const SETTLED_RETENTION: f32 = 0.55;

    /// A view formed on first contact — what he makes of a player he
    /// has only just started working with.
    pub fn forming(player: ActorRef, level: f32, ceiling: f32, today: EpochDay) -> Self {
        let level_pct = Self::to_pct(level);
        PlayerJudgement {
            player,
            level_pct,
            ceiling_pct: Self::to_pct(ceiling).max(level_pct),
            // Deliberately low. He has an impression, not a view.
            confidence_pct: 15,
            tactical_trust_pct: 50,
            big_match_trust_pct: 50,
            training_trust_pct: 50,
            professionalism_pct: 50,
            heat_pct: 0,
            recent_form_pct: 0,
            long_form_tenths: 65,
            observed: 0,
            formed: today,
            last_seen: today,
            outcome: JudgementOutcome::Open,
        }
    }

    #[inline]
    fn to_pct(value: f32) -> u8 {
        (value.clamp(0.0, 1.0) * 100.0).round() as u8
    }

    #[inline]
    pub fn level(&self) -> f32 {
        self.level_pct as f32 / 100.0
    }

    #[inline]
    pub fn ceiling(&self) -> f32 {
        self.ceiling_pct as f32 / 100.0
    }

    /// How sure he is *today* — the stored value, faded by time apart.
    /// The read every consumer should use; the raw field is only what
    /// he was sure of when he last saw him.
    pub fn confidence(&self, today: EpochDay) -> f32 {
        let stored = self.confidence_pct as f32 / 100.0;
        let years = MindClock::elapsed_f32(self.last_seen, today) / 365.0;
        if years <= 0.0 {
            return stored;
        }
        let held = if self.outcome.is_settled() {
            Self::SETTLED_RETENTION
        } else {
            0.0
        };
        let fading = 1.0 - held;
        let retained = held + fading * (1.0 - Self::ABSENCE_LOSS_PER_YEAR).powf(years);
        (stored * retained).clamp(0.0, 1.0)
    }

    /// How sure he was when he last looked, before any fading. What
    /// [`Self::settle`] is judged against.
    #[inline]
    pub fn commitment(&self) -> f32 {
        self.confidence_pct as f32 / 100.0
    }

    #[inline]
    pub fn tactical_trust(&self) -> f32 {
        self.tactical_trust_pct as f32 / 100.0
    }

    #[inline]
    pub fn big_match_trust(&self) -> f32 {
        self.big_match_trust_pct as f32 / 100.0
    }

    #[inline]
    pub fn training_trust(&self) -> f32 {
        self.training_trust_pct as f32 / 100.0
    }

    #[inline]
    pub fn professionalism(&self) -> f32 {
        self.professionalism_pct as f32 / 100.0
    }

    #[inline]
    pub fn heat(&self) -> f32 {
        self.heat_pct as f32 / 100.0
    }

    #[inline]
    pub fn recent_form(&self) -> f32 {
        self.recent_form_pct as f32 / 100.0
    }

    #[inline]
    pub fn long_form(&self) -> f32 {
        self.long_form_tenths as f32 / 10.0
    }

    /// He watched him play. `rating` is the match rating on the usual
    /// 0..10 scale; `big_match` says whether the occasion was one that
    /// tells you anything about temperament.
    pub fn watched(&mut self, rating: f32, big_match: bool, today: EpochDay) {
        let rating = rating.clamp(0.0, 10.0);
        let expected = self.long_form();
        let delta = rating - expected;

        // The long-run baseline moves slowly; the recent read moves fast.
        let long = expected + delta * 0.08;
        self.long_form_tenths = (long.clamp(0.0, 10.0) * 10.0).round() as u8;
        let form = self.recent_form() * 0.6 + (delta / 3.0).clamp(-1.0, 1.0) * 0.4;
        self.recent_form_pct = (form.clamp(-1.0, 1.0) * 100.0).round() as i8;

        // Tactical trust is the slow one — it is a read of reliability,
        // and reliability is by definition not settled in one match.
        Self::drift(&mut self.tactical_trust_pct, delta / 6.0, 0.15);
        if big_match {
            Self::drift(&mut self.big_match_trust_pct, delta / 4.0, 0.28);
        }

        // Heat rises on a divergence in either direction and cools on
        // an ordinary night. A manager stops thinking about a player who
        // keeps doing exactly what he expected.
        let divergence = (delta.abs() / 3.0).clamp(0.0, 1.0);
        let heat = self.heat() * 0.82 + divergence * 0.30;
        self.heat_pct = (heat.clamp(0.0, 1.0) * 100.0).round() as u8;

        self.observed = self.observed.saturating_add(1);
        self.last_seen = today;

        let confidence = self.confidence(today);
        self.confidence_pct =
            ((confidence + (1.0 - confidence) * Self::VIEWING_GAIN) * 100.0).round() as u8;
    }

    /// Nudge a packed 0..=100 axis by `delta` (in 0..1 units), damped by
    /// `rate`. Wrapped rather than repeated at four call sites.
    fn drift(axis: &mut u8, delta: f32, rate: f32) {
        let current = *axis as f32 / 100.0;
        let moved = (current + delta * rate).clamp(0.0, 1.0);
        *axis = (moved * 100.0).round() as u8;
    }

    /// Revise what he thinks the player is worth. Confidence damps the
    /// revision: a coach who is sure of a player does not re-rate him on
    /// one training session.
    pub fn revise(&mut self, level: f32, ceiling: f32, today: EpochDay) {
        let certainty = self.confidence(today);
        let inertia = 0.35 + certainty * 0.5;
        let blended_level = self.level() * inertia + level.clamp(0.0, 1.0) * (1.0 - inertia);
        let blended_ceiling = self.ceiling() * inertia + ceiling.clamp(0.0, 1.0) * (1.0 - inertia);

        self.level_pct = Self::to_pct(blended_level);
        self.ceiling_pct = Self::to_pct(blended_ceiling).max(self.level_pct);
    }

    /// The player did something the coach reads as character rather
    /// than form: repaid his faith, or refused to play for him.
    /// `weight` is signed, −1..+1.
    pub fn character_signal(&mut self, weight: f32, today: EpochDay) {
        let weight = weight.clamp(-1.0, 1.0);
        Self::drift(&mut self.professionalism_pct, weight, 0.30);
        Self::drift(&mut self.training_trust_pct, weight, 0.22);
        Self::drift(&mut self.tactical_trust_pct, weight, 0.12);
        let heat = (self.heat() + weight.abs() * 0.45).clamp(0.0, 1.0);
        self.heat_pct = (heat * 100.0).round() as u8;
        self.last_seen = today;
    }

    /// How much a full view of this player would be lost by dropping
    /// him from the store. Confidence, plus a floor for a judgement that
    /// has been settled — being proved wrong about someone is worth
    /// keeping precisely because it was expensive to learn.
    pub fn retention(&self, today: EpochDay) -> f32 {
        let base = self.confidence(today);
        match self.outcome {
            JudgementOutcome::Open => base,
            _ => base.max(0.5),
        }
    }

    /// Settle the question. `true_level` is what the player actually
    /// turned out to be worth, on the same 0..1 observable band.
    ///
    /// Returns the verdict, or `None` when the coach never held a firm
    /// enough view to be right or wrong about — an opinion he was never
    /// sure of teaches him nothing.
    ///
    /// Takes no date: the question is about the view he held, and the
    /// years since cannot un-commit him to it.
    pub fn settle(&mut self, true_level: f32) -> Option<JudgementOutcome> {
        if self.outcome.is_settled() {
            return None;
        }
        // Gated on how sure he *was*, not how sure he is now. Whether a
        // man was committed enough to be right or wrong about someone is
        // a fact about the view he held at the time; the years since
        // have blurred it but they cannot un-commit him.
        if self.commitment() < Self::VERDICT_CONFIDENCE {
            return None;
        }

        let error = (self.ceiling() - true_level.clamp(0.0, 1.0)).abs();
        let outcome = if error > Self::VERDICT_ERROR {
            JudgementOutcome::Wrong
        } else {
            JudgementOutcome::Vindicated
        };
        self.outcome = outcome;
        Some(outcome)
    }

    /// Confidence below which a coach was never committed enough for the
    /// outcome to be a lesson.
    pub const VERDICT_CONFIDENCE: f32 = 0.35;

    /// How far his ceiling has to miss before he was *wrong* rather than
    /// merely imprecise. Twenty points of the observable band is roughly
    /// a full star.
    pub const VERDICT_ERROR: f32 = 0.20;

    /// Does he rate him enough to build around?
    #[inline]
    pub fn is_worth_building_around(&self, today: EpochDay) -> bool {
        self.level() >= 0.70 && self.confidence(today) >= 0.5
    }

    /// Has he stopped counting on him?
    #[inline]
    pub fn has_written_him_off(&self, today: EpochDay) -> bool {
        self.level() <= 0.35 && self.confidence(today) >= 0.5
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const YEAR: EpochDay = 365;

    fn judging(level: f32, ceiling: f32) -> PlayerJudgement {
        PlayerJudgement::forming(ActorRef::player(9), level, ceiling, 1_000)
    }

    #[test]
    fn a_ceiling_is_never_below_the_floor() {
        let view = judging(0.8, 0.4);
        assert!(view.ceiling() >= view.level());
    }

    #[test]
    fn watching_him_builds_confidence_with_diminishing_returns() {
        let mut view = judging(0.5, 0.7);
        let start = view.confidence(1_000);

        view.watched(6.5, false, 1_007);
        let after_one = view.confidence(1_007);
        for week in 2..10 {
            view.watched(6.5, false, 1_000 + week * 7);
        }
        let after_nine = view.confidence(1_063);

        assert!(after_one > start);
        assert!(after_nine > after_one);
        assert!(
            after_nine - after_one < (after_one - start) * 9.0,
            "the fortieth viewing must tell him less than the fourth"
        );
    }

    #[test]
    fn a_view_fades_when_he_stops_seeing_the_player() {
        let mut view = judging(0.6, 0.8);
        for week in 0..20 {
            view.watched(7.0, false, 1_000 + week * 7);
        }
        let fresh = view.confidence(1_140);
        let decade_later = view.confidence(1_140 + YEAR * 10);

        assert!(fresh > 0.5, "twenty viewings is a real view: {fresh}");
        assert!(decade_later < fresh * 0.2, "but it does not stay sharp");
    }

    #[test]
    fn being_proved_right_holds_a_view_against_the_years() {
        let mut sure = judging(0.6, 0.8);
        let mut settled = judging(0.6, 0.8);
        for week in 0..20 {
            sure.watched(7.0, false, 1_000 + week * 7);
            settled.watched(7.0, false, 1_000 + week * 7);
        }
        settled.outcome = JudgementOutcome::Vindicated;

        let later = 1_140 + YEAR * 8;
        assert!(
            settled.confidence(later) > sure.confidence(later) * 2.0,
            "a view that was borne out does not fade like a guess"
        );
    }

    #[test]
    fn a_view_he_was_never_sure_of_teaches_him_nothing() {
        let mut barely = judging(0.4, 0.45);
        assert_eq!(
            barely.settle(0.95),
            None,
            "he never committed, so he was not wrong"
        );
        assert_eq!(barely.outcome, JudgementOutcome::Open);
    }

    #[test]
    fn writing_off_a_player_who_becomes_very_good_is_a_lesson() {
        let mut wrote_off = judging(0.30, 0.35);
        for week in 0..16 {
            wrote_off.watched(5.5, false, 1_000 + week * 7);
        }
        assert_eq!(wrote_off.settle(0.90), Some(JudgementOutcome::Wrong));
        assert_eq!(
            wrote_off.settle(0.90),
            None,
            "a question is only settled once"
        );
    }

    #[test]
    fn a_view_that_lands_is_vindicated() {
        let mut backed = judging(0.55, 0.85);
        for week in 0..16 {
            backed.watched(7.5, false, 1_000 + week * 7);
        }
        assert_eq!(backed.settle(0.88), Some(JudgementOutcome::Vindicated));
    }

    #[test]
    fn a_big_night_moves_temperament_and_an_ordinary_one_does_not() {
        let mut league = judging(0.6, 0.7);
        let mut final_night = judging(0.6, 0.7);
        league.watched(3.5, false, 1_010);
        final_night.watched(3.5, true, 1_010);

        assert_eq!(
            league.big_match_trust(),
            0.5,
            "a wet Tuesday says nothing about a final"
        );
        assert!(final_night.big_match_trust() < 0.5);
    }

    #[test]
    fn heat_cools_when_a_player_keeps_doing_what_was_expected() {
        let mut view = judging(0.6, 0.7);
        view.watched(2.0, false, 1_010);
        let hot = view.heat();
        for week in 2..12 {
            view.watched(view.long_form(), false, 1_000 + week * 7);
        }
        assert!(hot > 0.15, "a shocker gets his attention: {hot}");
        assert!(
            view.heat() < hot * 0.5,
            "and then he stops thinking about it"
        );
    }

    #[test]
    fn certainty_makes_him_slower_to_re_rate() {
        let mut fresh = judging(0.5, 0.6);
        let mut settled_in = judging(0.5, 0.6);
        for week in 0..24 {
            settled_in.watched(6.5, false, 1_000 + week * 7);
        }

        fresh.revise(0.9, 0.95, 1_170);
        settled_in.revise(0.9, 0.95, 1_170);

        assert!(
            fresh.level() > settled_in.level(),
            "a firm view resists a single new data point"
        );
    }

    #[test]
    fn a_judgement_stays_inside_its_budget() {
        // 32 bytes × 48 slots = 1.5 KB of judgement per member of staff.
        assert!(
            size_of::<PlayerJudgement>() <= 32,
            "PlayerJudgement grew to {} bytes",
            size_of::<PlayerJudgement>()
        );
    }
}
