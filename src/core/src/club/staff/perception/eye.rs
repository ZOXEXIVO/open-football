use chrono::NaiveDate;

use crate::utils::DateUtils;
use crate::{Player, Staff};

use super::ability::AbilityEstimator;
use super::potential::{EstimationContext, PotentialEstimator};
use super::profile::CoachProfile;
use super::utils::{date_to_week, perception_noise_raw};

/// The judge's eye — the number a coach report prints next to "Potential".
///
/// This is the one place outside growth simulation, generation, the
/// player's own retirement self-assessment and the edit form that reads
/// `player_attributes.potential_ability`, and it is licensed for DISPLAY
/// only (user decision, 2026-09-07). A coach who watches a boy every day
/// perceives talent the observable estimator has no channel for: how fast
/// he picks things up, his instinct, his touch against his age. The
/// simulation does not model any of that, so the eye reads the ceiling
/// itself and then blurs it by everything the judge cannot see — his own
/// `judging_player_potential`, the player's age, how often he watches him,
/// and his biases. The blur is a settled opinion of the man with a slow
/// monthly drift, so a report is wrong the way a person is wrong:
/// consistently, and by more when the judge is worse or the boy younger.
///
/// Club DECISIONS — scouting, transfers, loans, development, selection —
/// must keep reading [`PotentialEstimator`]. Feeding them this read would
/// make every AI club as sharp as its best judge's stars, which is
/// omniscience with a coat of noise on it.
pub struct CoachEye;

impl CoachEye {
    /// Error band the best judge in the world still carries: talent is not
    /// a number anyone can see.
    const WIDTH_FLOOR: f32 = 8.0;

    /// Extra band a hopeless judge (`judging_player_potential` 1) adds on
    /// top of the floor. Together with [`Self::WIDTH_FLOOR`] a middling
    /// judge reads a 17-year-old to within about a star.
    const WIDTH_JUDGING: f32 = 52.0;

    /// Share of the band removed by a saturated observation history —
    /// familiarity narrows a read, it never makes one perfect.
    const OBSERVATION_RELIEF: f32 = 0.30;

    /// Observation count at which familiarity saturates.
    const OBSERVATIONS_SATURATED: u8 = 20;

    /// Band added when the player is not in the judge's daily training
    /// group and the judge has no particular eye for youth.
    const VISIBILITY_INFLATION: f32 = 0.5;

    /// Share of the opinion that is the judge's settled view of the man,
    /// versus this month's mood. The settled part dominates so a report
    /// does not flip a star between two Mondays.
    const SETTLED_SHARE: f32 = 0.75;

    const SETTLED_SALT: u32 = 0x5EEA_0011;
    const MONTHLY_SALT: u32 = 0xC1B5_3E11;

    /// Seed of the judge-free consensus read — one shared market opinion
    /// per player, for views with no club behind them.
    const CONSENSUS_SEED: u32 = 0x0C05_E575;

    /// The consensus reads like a middling judge on a cold read.
    const CONSENSUS_ACCURACY: f32 = 0.5;

    /// A pessimistic judge lands low, an optimistic one high, by up to
    /// this many points either side.
    const PESSIMISM_SWING: f32 = 6.0;

    /// Up to this much a physically-biased judge adds for a tall, quick
    /// youngster — the one bias that flatters the wrong boy.
    const PHYSICAL_BIAS_SWING: f32 = 8.0;

    /// One judge's read of one player, on the 1..200 scale.
    pub fn read(player: &Player, staff: &Staff, ctx: &EstimationContext, date: NaiveDate) -> u8 {
        let profile = CoachProfile::from_staff(staff);
        Self::read_with_profile(player, &profile, ctx, date)
    }

    /// [`Self::read`] for callers that already hold the judge's profile.
    pub fn read_with_profile(
        player: &Player,
        profile: &CoachProfile,
        ctx: &EstimationContext,
        date: NaiveDate,
    ) -> u8 {
        let age = DateUtils::age(player.birth_date, date);
        let visibility = Self::visibility(profile, ctx.is_main_team);
        let width = Self::width(
            profile.potential_accuracy,
            age,
            visibility,
            ctx.observation_count,
        );
        let opinion = Self::opinion(profile.coach_seed, player.id, date);
        let pessimism = -(profile.negativity_bias - 0.5) * Self::PESSIMISM_SWING;
        let physical = Self::physical_bias(player, profile.physical_bias_youth, age);
        Self::settle(
            player,
            Self::ceiling(player) + opinion * width + pessimism + physical,
        )
    }

    /// The market's read of a player nobody employs — free agents, the
    /// retired — as a middling judge's cold read with no personal biases.
    /// Seeded by the player alone, so it is one opinion the world shares.
    pub fn consensus(player: &Player, date: NaiveDate) -> u8 {
        let age = DateUtils::age(player.birth_date, date);
        let width = Self::width(Self::CONSENSUS_ACCURACY, age, 1.0, 0);
        let opinion = Self::opinion(Self::CONSENSUS_SEED, player.id, date);
        Self::settle(player, Self::ceiling(player) + opinion * width)
    }

    /// Half-width of the error band, on the 1..200 scale. `accuracy` is
    /// the judge's `judging_player_potential` normalised to 0..1,
    /// `visibility` how much of the player's week he sees (1.0 in his own
    /// training group).
    pub fn width(accuracy: f32, age: u8, visibility: f32, observations: u8) -> f32 {
        let base = Self::WIDTH_FLOOR + Self::WIDTH_JUDGING * (1.0 - accuracy.clamp(0.0, 1.0));
        let unseen = (1.0 - visibility.clamp(0.0, 1.0)) * Self::VISIBILITY_INFLATION;
        let familiarity = observations.min(Self::OBSERVATIONS_SATURATED) as f32
            / Self::OBSERVATIONS_SATURATED as f32;
        let relief = 1.0 - Self::OBSERVATION_RELIEF * familiarity;
        (base * Self::age_factor(age) * (1.0 + unseen) * relief).clamp(3.0, 70.0)
    }

    /// How much of a ceiling is still guesswork at this age. A boy of
    /// sixteen has a decade of growing in front of him; at twenty-five the
    /// man is mostly what he will be.
    fn age_factor(age: u8) -> f32 {
        match age {
            0..=16 => 1.30,
            17..=18 => 1.15,
            19..=21 => 1.00,
            22..=24 => 0.70,
            _ => 0.40,
        }
    }

    /// Share of the player's week the judge actually sees. Full for his
    /// own training group; a reserve or academy boy is glimpsed unless
    /// the judge has an eye for youth.
    fn visibility(profile: &CoachProfile, is_main_team: bool) -> f32 {
        if is_main_team {
            1.0
        } else {
            (0.55 + profile.youth_preference * 0.4).min(1.0)
        }
    }

    /// The judge's opinion in [-1, 1]: a settled view of this player that
    /// never changes, blended with a mood that re-rolls every four weeks.
    fn opinion(seed: u32, player_id: u32, date: NaiveDate) -> f32 {
        let settled = perception_noise_raw(seed, player_id, Self::SETTLED_SALT);
        let period = date_to_week(date) / 4;
        let monthly_salt = Self::MONTHLY_SALT.wrapping_add(period.wrapping_mul(11));
        let monthly = perception_noise_raw(seed, player_id, monthly_salt);
        Self::SETTLED_SHARE * settled + (1.0 - Self::SETTLED_SHARE) * monthly
    }

    /// Height and raw pace read as a ceiling to a physically-minded judge
    /// — the same signal the observable estimator uses, kept here so the
    /// eye is wrong about the same boys a real coach is wrong about.
    fn physical_bias(player: &Player, physical_bias_youth: f32, age: u8) -> f32 {
        if age > 21 {
            return 0.0;
        }
        let height = player.player_attributes.height as f32;
        let height_signal = ((height - 178.0) / 12.0).clamp(-0.4, 1.0);
        let pace_signal = ((player.skills.physical.pace - 12.0) / 8.0).clamp(0.0, 1.0);
        (height_signal * 0.4 + pace_signal * 0.6) * physical_bias_youth * Self::PHYSICAL_BIAS_SWING
    }

    fn ceiling(player: &Player) -> f32 {
        player.player_attributes.potential_ability as f32
    }

    /// No judge tells you a ceiling is below where the man already plays.
    fn floor(player: &Player) -> f32 {
        AbilityEstimator::observable_level(player).max(PotentialEstimator::visible_ability(player))
            as f32
    }

    fn settle(player: &Player, read: f32) -> u8 {
        read.clamp(Self::floor(player), 200.0).round() as u8
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::club::StaffStub;
    use crate::club::player::generators::PlayerGenerator;
    use crate::{PeopleNameGeneratorData, PlayerPositionType, PlayerSkills};
    use chrono::{Datelike, Days};

    /// Fixtures: a judge of a given eye, and a boy of a given age, level
    /// and hidden ceiling.
    struct Eye;

    impl Eye {
        fn today() -> NaiveDate {
            NaiveDate::from_ymd_opt(2026, 5, 8).unwrap()
        }

        fn judge(id: u32, judging_potential: u8) -> Staff {
            let mut s = StaffStub::default();
            s.id = id;
            s.staff_attributes.knowledge.judging_player_potential = judging_potential;
            s.staff_attributes.knowledge.judging_player_ability = judging_potential;
            s.staff_attributes.coaching.working_with_youngsters = 12;
            s.staff_attributes.mental.adaptability = 12;
            s.staff_attributes.mental.determination = 12;
            s.staff_attributes.mental.discipline = 12;
            s.staff_attributes.mental.man_management = 12;
            s.staff_attributes.coaching.attacking = 12;
            s.staff_attributes.coaching.defending = 12;
            s.staff_attributes.coaching.fitness = 12;
            s.staff_attributes.coaching.mental = 12;
            s.staff_attributes.coaching.tactical = 12;
            s.staff_attributes.coaching.technical = 12;
            s
        }

        fn boy(id: u32, age: u8, visible: u8, ceiling: u8) -> Player {
            let names = PeopleNameGeneratorData {
                first_names: vec!["Test".to_string()],
                last_names: vec!["Player".to_string()],
                nicknames: Vec::new(),
            };
            let today = Self::today();
            let mut p = PlayerGenerator::generate(
                id,
                today,
                PlayerPositionType::MidfielderCenter,
                visible,
                &names,
            );
            p.id = id;
            p.birth_date = NaiveDate::from_ymd_opt(today.year() - age as i32, 1, 1).unwrap();
            p.skills = PlayerSkills::flat_for_ability(visible);
            p.player_attributes.height = 180;
            p.player_attributes.current_ability = visible;
            p.player_attributes.potential_ability = ceiling;
            p
        }

        fn mean_error(judge: &Staff, ceiling: u8) -> f32 {
            let ctx = EstimationContext::default();
            let total: f32 = (1..=40u32)
                .map(|id| {
                    let boy = Self::boy(id, 17, 70, ceiling);
                    (CoachEye::read(&boy, judge, &ctx, Self::today()) as f32 - ceiling as f32).abs()
                })
                .sum();
            total / 40.0
        }
    }

    /// The point of the eye: a good judge reads the hidden ceiling close,
    /// a poor one reads it wide — and both are anchored on the truth, not
    /// on where the boy plays today.
    #[test]
    fn a_better_judge_reads_closer_to_the_hidden_ceiling() {
        let weak = Eye::judge(7, 2);
        let elite = Eye::judge(7, 19);
        let weak_error = Eye::mean_error(&weak, 150);
        let elite_error = Eye::mean_error(&elite, 150);
        assert!(
            elite_error < weak_error,
            "elite judge error {elite_error} should be below weak {weak_error}"
        );
        assert!(elite_error < 15.0, "elite judge mean error {elite_error}");
        assert!(weak_error > 12.0, "weak judge mean error {weak_error}");
    }

    /// Two boys with the same visible skills and different ceilings read
    /// differently to the same judge — the inversion of the observable
    /// estimator's contract, and the whole reason the eye exists.
    #[test]
    fn identical_visible_skills_read_apart_when_the_ceilings_differ() {
        let judge = Eye::judge(7, 12);
        let ctx = EstimationContext::default();
        let high = Eye::boy(300, 19, 100, 180);
        let low = Eye::boy(300, 19, 100, 105);
        let high_read = CoachEye::read(&high, &judge, &ctx, Eye::today());
        let low_read = CoachEye::read(&low, &judge, &ctx, Eye::today());
        // Same id, same judge, same month ⇒ same opinion; only the ceiling
        // moved, so the reads differ by the ceiling gap (less any floor).
        assert!(
            high_read as i16 - low_read as i16 >= 60,
            "high {high_read} vs low {low_read}"
        );
    }

    #[test]
    fn width_narrows_with_judging_age_familiarity_and_visibility() {
        assert!(CoachEye::width(0.9, 17, 1.0, 20) < CoachEye::width(0.1, 17, 1.0, 20));
        assert!(CoachEye::width(0.5, 30, 1.0, 0) < CoachEye::width(0.5, 16, 1.0, 0));
        assert!(CoachEye::width(0.5, 17, 1.0, 20) < CoachEye::width(0.5, 17, 1.0, 0));
        assert!(CoachEye::width(0.5, 17, 0.6, 0) > CoachEye::width(0.5, 17, 1.0, 0));
        // A middling judge reads a 17-year-old to about a star (40 points).
        let middling = CoachEye::width(0.5, 17, 1.0, 20);
        assert!((25.0..45.0).contains(&middling), "middling width {middling}");
    }

    /// The read never drops below where the man already plays, whatever
    /// the judge or the ceiling.
    #[test]
    fn read_never_falls_below_visible_ability() {
        let ctx = EstimationContext::default();
        for age in [16u8, 19, 22, 26, 30, 34] {
            for judging in [1u8, 5, 10, 15, 20] {
                let judge = Eye::judge(9, judging);
                let boy = Eye::boy(11, age, 130, 131);
                let read = CoachEye::read(&boy, &judge, &ctx, Eye::today());
                let visible = PotentialEstimator::visible_ability(&boy);
                assert!(read >= visible, "read {read} below visible {visible}");
                assert!((1..=200).contains(&read));
            }
        }
    }

    #[test]
    fn read_is_deterministic_for_fixed_inputs() {
        let judge = Eye::judge(3, 10);
        let ctx = EstimationContext::default();
        let boy = Eye::boy(5, 20, 110, 150);
        let a = CoachEye::read(&boy, &judge, &ctx, Eye::today());
        let b = CoachEye::read(&boy, &judge, &ctx, Eye::today());
        assert_eq!(a, b);
    }

    /// A settled opinion with a monthly mood: the read may move between
    /// months, but never by more than the mood's share of the band.
    #[test]
    fn opinion_drifts_slowly_month_to_month() {
        let judge = Eye::judge(4, 8);
        let ctx = EstimationContext::default();
        let later = Eye::today().checked_add_days(Days::new(40)).unwrap();
        for id in 1..=30u32 {
            let boy = Eye::boy(id, 18, 80, 140);
            let now = CoachEye::read(&boy, &judge, &ctx, Eye::today()) as f32;
            let then = CoachEye::read(&boy, &judge, &ctx, later) as f32;
            let width = CoachEye::width(8.0 / 20.0, 18, 1.0, 0);
            let allowed = 2.0 * (1.0 - CoachEye::SETTLED_SHARE) * width + 1.5;
            assert!(
                (now - then).abs() <= allowed,
                "player {id}: {now} -> {then} moved more than {allowed}"
            );
        }
    }

    /// With no judge the market still reads the ceiling, not the floor.
    #[test]
    fn consensus_tracks_the_ceiling_without_a_judge() {
        let high = Eye::boy(77, 17, 60, 180);
        let low = Eye::boy(77, 17, 60, 80);
        let high_read = CoachEye::consensus(&high, Eye::today());
        let low_read = CoachEye::consensus(&low, Eye::today());
        assert!(
            high_read as i16 - low_read as i16 >= 60,
            "consensus high {high_read} vs low {low_read}"
        );
        assert!(high_read >= PotentialEstimator::visible_ability(&high));
    }
}
