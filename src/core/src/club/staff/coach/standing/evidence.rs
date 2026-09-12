//! What moves a standing, and by how much.
//!
//! One place that turns things that happen — a match, a card, a talk, a run
//! of weeks in training, a transfer request — into a signed impulse. Kept
//! apart from [`StandingLadder`] so the ladder is about *deciding* and this
//! is about *evidence*, and so a tuning pass on one does not have to read
//! the other.
//!
//! Every impulse leaves here already scaled by the coach: how hard he feels
//! things at all, whether he feels the bad more than the good, whether this
//! is his own signing he is reluctant to admit a mistake about, and whether
//! the evidence agrees with the first impression he formed.
//!
//! [`StandingLadder`]: super::ladder::StandingLadder

use super::ladder::{CoachStanding, GrievanceFlags, LadderContext, StandingLadder};
use super::tuning::StandingTuning;
use crate::club::staff::coach::memory::CoachMatchObservation;
use crate::club::staff::perception::CoachProfile;

/// How the coach relates to this particular player, beyond the ladder's own
/// context: things that bend how hard the evidence lands rather than what
/// the evidence is.
#[derive(Debug, Clone, Copy)]
pub struct EvidenceLens {
    /// He signed him, and admitting the mistake means admitting he made it.
    pub is_my_signing: bool,
    /// Days since the signing. The reluctance is worst in the first year.
    pub days_since_signing: i64,
    /// The first impression he formed, −1..=1, or zero when he has none.
    /// Evidence that agrees with it counts for more.
    pub first_impression: f32,
    /// The coach's own `loyalty` attribute, 0–20.
    pub loyalty: f32,
}

impl Default for EvidenceLens {
    fn default() -> Self {
        EvidenceLens {
            is_my_signing: false,
            days_since_signing: i64::MAX,
            first_impression: 0.0,
            loyalty: 10.0,
        }
    }
}

/// Turns what happened into what the coach feels about it.
pub struct StandingEvidence;

impl StandingEvidence {
    /// Apply everything one match says about a player.
    ///
    /// `expected` is what the coach had this player down for — his own
    /// long-form baseline, not the league's — so the same 6.2 is a
    /// disappointment from one man and a step up from another. That is the
    /// difference between a standing and a rating.
    pub fn from_match(
        standing: &mut CoachStanding,
        observation: &CoachMatchObservation,
        expected: f32,
        profile: &CoachProfile,
        lens: &EvidenceLens,
        context: &LadderContext,
    ) {
        let rating = observation.effective_rating.clamp(0.0, 10.0);
        let occasion = observation.match_importance >= StandingTuning::OCCASION_IMPORTANCE;

        // ── What he did ──
        let gap = rating - expected;
        let mut raw =
            (gap * StandingTuning::RATING_PER_POINT).clamp(-StandingTuning::RATING_CLAMP, StandingTuning::RATING_CLAMP);

        if observation.is_starter && observation.is_big_match() {
            raw += if rating >= StandingTuning::BIG_MATCH_GOOD_RATING {
                StandingTuning::BIG_MATCH_GOOD
            } else if rating < StandingTuning::BIG_MATCH_FAILURE_RATING {
                StandingTuning::BIG_MATCH_BAD
            } else {
                0.0
            };
        }

        let occasion_scale = if occasion {
            StandingTuning::OCCASION_MULTIPLIER
        } else {
            1.0
        };
        if observation.errors_leading_to_goal > 0 {
            raw += StandingTuning::COSTLY_ERROR
                * (observation.errors_leading_to_goal as f32).min(2.0)
                * occasion_scale;
        }
        if observation.red_cards > 0 {
            raw += StandingTuning::RED_CARD * occasion_scale;
        }
        if observation.was_substituted_early && observation.is_starter {
            raw += StandingTuning::EARLY_HOOK;
        }
        if observation.is_starter
            && observation.minutes_played >= 80
            && observation.errors_leading_to_goal == 0
            && observation.red_cards == 0
        {
            raw += StandingTuning::CLEAN_FULL_MATCH;
        }

        // ── What it cost us ──
        let cost_the_occasion = occasion
            && observation.is_starter
            && !observation.team_won
            && (observation.errors_leading_to_goal > 0 || observation.red_cards > 0);
        if cost_the_occasion {
            standing
                .grievance
                .insert(GrievanceFlags::COST_THE_OCCASION);
        }

        StandingLadder::nudge(standing, Self::scaled(raw, profile, lens));

        if observation.is_starter && observation.is_big_match() {
            StandingLadder::note_big_match(standing, rating, context);
        }
    }

    /// A card, a fine, a problem in the dressing room.
    pub fn indiscipline(standing: &mut CoachStanding, profile: &CoachProfile, lens: &EvidenceLens) {
        // A disciplinarian feels it harder — the same attribute that makes
        // him issue the fine makes the offence matter to him.
        let discipline = profile.negativity_bias.clamp(0.0, 1.0);
        let raw = StandingTuning::DISCIPLINE_EVENT * (1.0 + discipline * 0.5);
        standing.grievance.insert(GrievanceFlags::INDISCIPLINE);
        StandingLadder::nudge(standing, Self::scaled(raw, profile, lens));
    }

    /// He would not play. Not scaled by anything: this is not a reading, it
    /// is a fact, and no personality makes it acceptable.
    pub fn refused_to_play(standing: &mut CoachStanding) {
        standing.grievance.insert(GrievanceFlags::REFUSED);
        StandingLadder::nudge(standing, StandingTuning::REFUSED);
    }

    /// He asked to leave. A loyal manager takes it harder, and it stings
    /// most when the coach was picking him.
    pub fn asked_to_leave(standing: &mut CoachStanding, lens: &EvidenceLens) {
        standing.grievance.insert(GrievanceFlags::WANTS_OUT);
        let raw = if lens.loyalty >= StandingTuning::LOYAL_ATTRIBUTE {
            StandingTuning::WANTS_OUT_LOYAL
        } else {
            StandingTuning::WANTS_OUT
        };
        // Scaled by how much the coach wanted to keep him: a squad player
        // taking his chance is not a betrayal.
        let weight = if standing.rung.is_favoured() { 1.0 } else { 0.5 };
        StandingLadder::nudge(standing, raw * weight);
    }

    /// He took it to the press.
    pub fn went_public(standing: &mut CoachStanding) {
        standing.grievance.insert(GrievanceFlags::WENT_PUBLIC);
        StandingLadder::nudge(standing, StandingTuning::WENT_PUBLIC);
    }

    /// They talked, and it went one way or the other.
    pub fn talked(
        standing: &mut CoachStanding,
        went_well: bool,
        profile: &CoachProfile,
        lens: &EvidenceLens,
    ) {
        let raw = if went_well {
            StandingTuning::TALK_POSITIVE
        } else {
            StandingTuning::TALK_NEGATIVE
        };
        StandingLadder::nudge(standing, Self::scaled(raw, profile, lens));
    }

    /// A week of training at one extreme or the other. Only a *run* of them
    /// counts — training is the slow signal, and one good week in the gym
    /// has never got anybody into a team.
    pub fn training_week(
        standing: &mut CoachStanding,
        impression: f32,
        profile: &CoachProfile,
        lens: &EvidenceLens,
    ) {
        let direction = if impression >= StandingTuning::TRAINING_HIGH {
            1
        } else if impression < StandingTuning::TRAINING_LOW {
            -1
        } else {
            standing.training_run = 0;
            return;
        };

        // A run only counts while it keeps going the same way.
        if standing.training_run.signum() != direction {
            standing.training_run = 0;
        }
        standing.training_run = standing
            .training_run
            .saturating_add(direction)
            .clamp(-100, 100);

        if standing.training_run.unsigned_abs() < StandingTuning::TRAINING_RUN_WEEKS {
            return;
        }
        if standing.training_month.abs() >= StandingTuning::TRAINING_MONTHLY_CAP {
            return;
        }
        let raw = StandingTuning::TRAINING_WEEK * direction as f32;
        standing.training_month += raw;
        StandingLadder::nudge(standing, Self::scaled(raw, profile, lens));
    }

    /// A month has turned; the training allowance resets.
    pub fn new_month(standing: &mut CoachStanding) {
        standing.training_month = 0.0;
    }

    /// A promise of minutes, kept.
    pub fn promise_kept(standing: &mut CoachStanding) {
        StandingLadder::nudge(standing, StandingTuning::PROMISE_KEPT);
        standing.debt = (standing.debt - StandingTuning::PROMISE_KEPT).max(0.0);
    }

    /// A promise of minutes, broken. The player's standing does not move —
    /// this was the coach's failure, not his — but the coach now owes him
    /// something, and a man-manager pays.
    pub fn promise_broken(standing: &mut CoachStanding) {
        standing.debt = (standing.debt + StandingTuning::DEBT_PROMISE_BROKEN).clamp(0.0, 1.0);
    }

    /// Picked against the evidence, and he answered.
    pub fn repaid_faith(standing: &mut CoachStanding, protected_until: chrono::NaiveDate) {
        StandingLadder::nudge(standing, StandingTuning::REPAID_FAITH);
        standing.debt = 0.0;
        standing.protected_until = Some(protected_until);
    }

    /// Scale one raw impulse by everything about this coach and this
    /// relationship that bends how hard it lands.
    fn scaled(raw: f32, profile: &CoachProfile, lens: &EvidenceLens) -> f32 {
        if raw == 0.0 {
            return 0.0;
        }
        let mut scaled = raw * StandingTuning::gain(profile.emotional_volatility);
        scaled *= StandingTuning::negativity_scale(raw, profile.negativity_bias);

        // A coach defends his own signing.
        if lens.is_my_signing && raw < 0.0 {
            let span = if lens.days_since_signing <= StandingTuning::SUNK_COST_YEAR_DAYS {
                StandingTuning::SUNK_COST_YEAR_ONE_SPAN
            } else {
                StandingTuning::SUNK_COST_AFTER_SPAN
            };
            scaled *= 1.0 - profile.stubbornness.clamp(0.0, 1.0) * span;
        }

        // And he hears what he expected to hear.
        if lens.first_impression != 0.0 {
            let confirms = lens.first_impression.signum() == raw.signum();
            let shift = profile.confirmation_bias.clamp(0.0, 1.0) * StandingTuning::CONFIRMATION_SPAN;
            scaled *= if confirms { 1.0 + shift } else { 1.0 - shift };
        }

        scaled
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::club::staff::CoachingStyle;
    use crate::club::staff::StaffStub;
    use crate::{Staff, StaffMental};
    use chrono::NaiveDate;

    /// Fixture builders, grouped so the tests read as sentences.
    struct Fx;

    impl Fx {
        fn date(day: i64) -> NaiveDate {
            NaiveDate::from_ymd_opt(2030, 1, 1).unwrap() + chrono::Duration::days(day)
        }

        fn staff(mental: StaffMental, style: CoachingStyle) -> Staff {
            let mut staff = StaffStub::default();
            staff.id = 1;
            staff.staff_attributes.mental = mental;
            staff.staff_attributes.knowledge.judging_player_ability = 12;
            staff.coaching_style = style;
            staff
        }

        fn mental(man_management: u8, discipline: u8, determination: u8) -> StaffMental {
            StaffMental {
                adaptability: 12,
                determination,
                discipline,
                man_management,
                motivating: 12,
            }
        }

        fn balanced() -> CoachProfile {
            CoachProfile::from_staff(&Self::staff(
                Self::mental(12, 12, 12),
                CoachingStyle::Democratic,
            ))
        }

        fn stern() -> CoachProfile {
            CoachProfile::from_staff(&Self::staff(
                Self::mental(5, 18, 14),
                CoachingStyle::Authoritarian,
            ))
        }

        fn league_match(rating: f32) -> CoachMatchObservation {
            CoachMatchObservation {
                player_id: 7,
                effective_rating: rating,
                minutes_played: 90,
                is_starter: true,
                match_importance: 0.7,
                is_cup: false,
                is_derby: false,
                is_continental: false,
                goals: 0,
                assists: 0,
                errors_leading_to_goal: 0,
                yellow_cards: 0,
                red_cards: 0,
                team_won: true,
                was_substituted_early: false,
                role_fit: 1.0,
                professionalism_signal: 0.7,
                date: Self::date(0),
            }
        }

        fn the_final(rating: f32) -> CoachMatchObservation {
            let mut observation = Self::league_match(rating);
            observation.is_cup = true;
            observation.match_importance = 0.95;
            observation.team_won = false;
            observation
        }
    }

    #[test]
    fn a_standing_moves_against_what_the_coach_expected_of_this_player() {
        let profile = Fx::balanced();
        let lens = EvidenceLens::default();
        let context = LadderContext::default();

        let mut modest = CoachStanding::default();
        let mut star = CoachStanding::default();
        // The same 6.2, from a man he had down as a 6.0 and one he had down
        // as a 7.4.
        StandingEvidence::from_match(
            &mut modest,
            &Fx::league_match(6.2),
            6.0,
            &profile,
            &lens,
            &context,
        );
        StandingEvidence::from_match(
            &mut star,
            &Fx::league_match(6.2),
            7.4,
            &profile,
            &lens,
            &context,
        );

        assert!(modest.score > 0.0, "he did a little better than expected");
        assert!(star.score < 0.0, "and the other one rather worse");
    }

    #[test]
    fn a_red_card_in_a_final_lands_twice_as_hard_as_one_in_a_league_game() {
        let profile = Fx::balanced();
        let lens = EvidenceLens::default();
        let context = LadderContext::default();

        let mut tuesday = CoachStanding::default();
        let mut cup_final = CoachStanding::default();
        let mut ordinary = Fx::league_match(5.0);
        ordinary.red_cards = 1;
        let mut occasion = Fx::the_final(5.0);
        occasion.red_cards = 1;

        StandingEvidence::from_match(&mut tuesday, &ordinary, 6.8, &profile, &lens, &context);
        StandingEvidence::from_match(&mut cup_final, &occasion, 6.8, &profile, &lens, &context);

        assert!(cup_final.score < tuesday.score);
        assert!(
            cup_final
                .grievance
                .contains(GrievanceFlags::COST_THE_OCCASION),
            "and he knows exactly what it cost"
        );
        assert!(
            !tuesday
                .grievance
                .contains(GrievanceFlags::COST_THE_OCCASION)
        );
    }

    #[test]
    fn a_stern_coach_feels_a_bad_night_harder_than_a_warm_one() {
        let lens = EvidenceLens::default();
        let context = LadderContext::default();
        let mut warm = CoachStanding::default();
        let mut stern = CoachStanding::default();

        StandingEvidence::from_match(
            &mut warm,
            &Fx::league_match(5.0),
            7.0,
            &Fx::balanced(),
            &lens,
            &context,
        );
        StandingEvidence::from_match(
            &mut stern,
            &Fx::league_match(5.0),
            7.0,
            &Fx::stern(),
            &lens,
            &context,
        );

        assert!(
            stern.score < warm.score,
            "stern={} warm={}",
            stern.score,
            warm.score
        );
    }

    #[test]
    fn a_coach_gives_his_own_signing_a_longer_rope_in_the_first_year() {
        let profile = Fx::stern();
        let context = LadderContext::default();
        let stranger = EvidenceLens::default();
        let mine = EvidenceLens {
            is_my_signing: true,
            days_since_signing: 60,
            ..EvidenceLens::default()
        };
        let mine_long_ago = EvidenceLens {
            is_my_signing: true,
            days_since_signing: 900,
            ..EvidenceLens::default()
        };

        let mut a = CoachStanding::default();
        let mut b = CoachStanding::default();
        let mut c = CoachStanding::default();
        for (standing, lens) in [
            (&mut a, &stranger),
            (&mut b, &mine),
            (&mut c, &mine_long_ago),
        ] {
            StandingEvidence::from_match(
                standing,
                &Fx::league_match(5.0),
                7.0,
                &profile,
                lens,
                &context,
            );
        }

        assert!(b.score > a.score, "his own man gets the benefit");
        assert!(
            c.score < b.score && c.score > a.score,
            "and less of it three years on"
        );
    }

    #[test]
    fn one_good_week_in_the_gym_has_never_got_anybody_into_a_team() {
        let profile = Fx::balanced();
        let lens = EvidenceLens::default();
        let mut standing = CoachStanding::default();

        for _ in 0..3 {
            StandingEvidence::training_week(&mut standing, 0.8, &profile, &lens);
        }
        assert_eq!(standing.score, 0.0, "three weeks is not a run");

        StandingEvidence::training_week(&mut standing, 0.8, &profile, &lens);
        assert!(standing.score > 0.0, "the fourth is");
    }

    #[test]
    fn a_run_of_training_cannot_ratchet_a_man_into_the_side() {
        let profile = Fx::balanced();
        let lens = EvidenceLens::default();
        let mut standing = CoachStanding::default();
        for _ in 0..40 {
            StandingEvidence::training_week(&mut standing, 0.9, &profile, &lens);
        }
        assert!(
            standing.score <= StandingTuning::TRAINING_MONTHLY_CAP * 2.0,
            "a month's allowance is a month's allowance: {}",
            standing.score
        );
    }

    #[test]
    fn a_broken_promise_costs_the_coach_and_not_the_player() {
        let mut standing = CoachStanding::default();
        StandingEvidence::promise_broken(&mut standing);
        assert_eq!(standing.score, 0.0, "the player did nothing wrong");
        assert!(standing.debt > 0.0, "but he is owed something");
        assert!(
            !standing.owes_a_start(0.7, 0.4),
            "one fudged assurance is not yet a debt he has to settle"
        );

        StandingEvidence::promise_broken(&mut standing);
        assert!(standing.owes_a_start(0.7, 0.4), "twice is");
        assert!(
            !standing.owes_a_start(0.7, 0.9),
            "and not in a game that matters"
        );
        assert!(
            !standing.owes_a_start(0.2, 0.4),
            "nor from a manager who does not pay his debts"
        );
    }

    #[test]
    fn a_man_he_rated_asking_to_leave_stings_more_than_a_squad_player_doing_it() {
        let lens = EvidenceLens::default();
        let mut favoured = CoachStanding::seeded(
            0.5,
            GrievanceFlags::default(),
            Fx::date(0),
        );
        let mut fringe = CoachStanding::default();

        let before = favoured.score;
        StandingEvidence::asked_to_leave(&mut favoured, &lens);
        StandingEvidence::asked_to_leave(&mut fringe, &lens);

        assert!(before - favoured.score > fringe.score.abs());
    }

    #[test]
    fn a_loyal_manager_takes_a_transfer_request_harder() {
        let ordinary = EvidenceLens::default();
        let loyal = EvidenceLens {
            loyalty: 17.0,
            ..EvidenceLens::default()
        };
        let mut a = CoachStanding::seeded(0.5, GrievanceFlags::default(), Fx::date(0));
        let mut b = CoachStanding::seeded(0.5, GrievanceFlags::default(), Fx::date(0));
        StandingEvidence::asked_to_leave(&mut a, &ordinary);
        StandingEvidence::asked_to_leave(&mut b, &loyal);
        assert!(b.score < a.score);
    }

    #[test]
    fn he_hears_what_he_expected_to_hear() {
        let profile = Fx::staff(Fx::mental(8, 14, 16), CoachingStyle::Authoritarian);
        let profile = CoachProfile::from_staff(&profile);
        let context = LadderContext::default();
        let doubted = EvidenceLens {
            first_impression: -0.6,
            ..EvidenceLens::default()
        };
        let backed = EvidenceLens {
            first_impression: 0.6,
            ..EvidenceLens::default()
        };

        let mut a = CoachStanding::default();
        let mut b = CoachStanding::default();
        StandingEvidence::from_match(
            &mut a,
            &Fx::league_match(5.0),
            7.0,
            &profile,
            &doubted,
            &context,
        );
        StandingEvidence::from_match(
            &mut b,
            &Fx::league_match(5.0),
            7.0,
            &profile,
            &backed,
            &context,
        );
        assert!(
            a.score < b.score,
            "the bad night confirms one read and contradicts the other"
        );
    }
}
