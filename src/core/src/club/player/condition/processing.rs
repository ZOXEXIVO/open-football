use crate::club::PlayerStatusType;
use crate::club::player::player::Player;
use crate::league::SeasonPhase;
use crate::utils::DateUtils;
use chrono::NaiveDate;

/// Minimum condition floor for injured players (30%)
const INJURY_CONDITION_FLOOR: i16 = 3000;

/// Minimum fitness floor for injured players (20%)
const INJURY_FITNESS_FLOOR: i16 = 2000;

/// The "normal" condition level that rest/training pushes toward (90%)
const CONDITION_NORMAL_LEVEL: i16 = 9000;

impl Player {
    /// Daily condition processing (rest day — no training scheduled).
    /// This provides slow natural recovery. Training is the main mechanism
    /// for restoring condition back to normal levels (FM-like behavior).
    ///
    /// Rest-only recovery: ~150-300/day (1.5-3%) — slow without training.
    /// With training recovery sessions: much faster (handled in training result).
    pub(crate) fn process_condition_recovery(&mut self, now: NaiveDate) {
        let natural_fitness = self.skills.physical.natural_fitness;

        if self.player_attributes.is_injured {
            self.process_injury_condition_decay(natural_fitness);
            self.player_attributes.days_since_last_match += 1;
            return;
        }

        let age = DateUtils::age(self.birth_date, now);
        let jadedness = self.player_attributes.jadedness;
        let condition = self.player_attributes.condition;

        // Only recover if below normal level — don't overshoot
        if condition < CONDITION_NORMAL_LEVEL {
            // Base recovery: 200-500 per day based on natural_fitness
            // This is rest-only (no training). A player at 60% reaches ~85% in ~5-6 days.
            let base_recovery = 200.0 + (natural_fitness / 20.0) * 300.0;

            // Age penalty: older players recover slower
            let age_factor = if age > 30 {
                1.0 - (age as f32 - 30.0) * 0.05
            } else if age < 23 {
                1.1
            } else {
                1.0
            };

            // Jadedness penalty: jaded players recover slower
            let jadedness_factor = 1.0 - (jadedness as f32 / 10000.0) * 0.3;

            // Rest bonus: more days since match = better recovery
            let rest_bonus = if self.player_attributes.days_since_last_match > 7 {
                1.4
            } else if self.player_attributes.days_since_last_match > 3 {
                1.2
            } else {
                1.0
            };

            // Calendar phase: winter breaks and post-season rest buy extra
            // recovery beyond what day-to-day spacing alone would yield.
            let phase_bonus = SeasonPhase::from_date(now).condition_recovery_multiplier();

            let recovery = (base_recovery
                * age_factor.max(0.5)
                * jadedness_factor.max(0.5)
                * rest_bonus
                * phase_bonus) as u16;

            // Cap recovery so we don't overshoot normal level
            let max_gain = (CONDITION_NORMAL_LEVEL - condition) as u16;
            self.player_attributes.rest(recovery.min(max_gain));
        }

        // Jadedness natural decay: -150/day when no match for 3+ days
        if self.player_attributes.days_since_last_match > 3 {
            self.player_attributes.jadedness = (self.player_attributes.jadedness - 150).max(0);
        }

        // Remove Rst status when jadedness drops below threshold
        if self.player_attributes.jadedness < 4000 {
            self.statuses.remove(PlayerStatusType::Rst);
        }

        // Increment days since last match
        self.player_attributes.days_since_last_match += 1;
    }

    /// Injured players lose condition, fitness, and match readiness daily.
    /// Higher natural_fitness slows the decay.
    /// Condition floors at 30% — even injured players maintain baseline condition.
    fn process_injury_condition_decay(&mut self, natural_fitness: f32) {
        let nf_factor = 1.0 - (natural_fitness / 20.0) * 0.7; // 0.3..1.0

        // Condition decay: ~30-100/day depending on natural_fitness
        // At nf=20: ~30/day, at nf=1: ~98/day
        let condition_decay = (100.0 * nf_factor) as i16;
        self.player_attributes.condition =
            (self.player_attributes.condition - condition_decay).max(INJURY_CONDITION_FLOOR);

        // Fitness decay: ~15-50/day depending on natural_fitness
        let fitness_decay = (50.0 * nf_factor) as i16;
        self.player_attributes.fitness =
            (self.player_attributes.fitness - fitness_decay).max(INJURY_FITNESS_FLOOR);

        // Match readiness decays faster during injury (not training or playing)
        self.skills.physical.match_readiness =
            (self.skills.physical.match_readiness - 0.2).max(0.0);
    }

    /// Players not playing lose match sharpness over time — except in
    /// pre-season, when structured training rebuilds it.
    pub(crate) fn process_match_readiness_decay(&mut self, now: NaiveDate) {
        if self.player_attributes.is_injured {
            // Already handled in process_injury_condition_decay
            return;
        }

        let phase = SeasonPhase::from_date(now);
        let phase_gain = phase.match_readiness_gain();
        if phase_gain > 0.0 {
            // Pre-season / winter-break conditioning actively rebuilds
            // sharpness — override the idle-decay branch entirely.
            self.skills.physical.match_readiness =
                (self.skills.physical.match_readiness + phase_gain).min(20.0);
            return;
        }

        if self.player_attributes.days_since_last_match > 7 {
            // Accelerated decay after a week without matches
            self.skills.physical.match_readiness =
                (self.skills.physical.match_readiness - 0.15).max(0.0);
        } else if self.player_attributes.days_since_last_match > 3 {
            // Gradual decay
            self.skills.physical.match_readiness =
                (self.skills.physical.match_readiness - 0.08).max(0.0);
        }
        // No decay for first 3 days (normal rest period)
    }
}

#[cfg(test)]
mod recovery_tests {
    use crate::club::player::builder::PlayerBuilder;
    use crate::club::player::condition::ConditionLabel;
    use crate::club::player::player::Player;
    use crate::shared::fullname::FullName;
    use crate::{
        PersonAttributes, PlayerAttributes, PlayerPosition, PlayerPositionType, PlayerPositions,
        PlayerSkills,
    };
    use chrono::NaiveDate;

    fn d(y: i32, m: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, day).unwrap()
    }

    fn make_player(birth: NaiveDate, natural_fitness: f32) -> Player {
        let mut attrs = PlayerAttributes::default();
        attrs.condition = 5_000;
        attrs.fitness = 7_000;
        attrs.jadedness = 2_000;
        attrs.days_since_last_match = 1;
        let mut skills = PlayerSkills::default();
        skills.physical.natural_fitness = natural_fitness;
        skills.physical.match_readiness = 13.0;
        PlayerBuilder::new()
            .id(11)
            .full_name(FullName::new("T".to_string(), "P".to_string()))
            .birth_date(birth)
            .country_id(1)
            .attributes(PersonAttributes::default())
            .skills(skills)
            .positions(PlayerPositions {
                positions: vec![PlayerPosition {
                    position: PlayerPositionType::MidfielderCenter,
                    level: 18,
                }],
            })
            .player_attributes(attrs)
            .build()
            .unwrap()
    }

    #[test]
    fn one_match_per_week_mostly_recovers_by_match_day() {
        // 50% condition midfielder with a week's rest. Rest-only
        // recovery is intentionally slow — players rely on training
        // recovery sessions to top up — but a week should still bring
        // condition back to a "selectable" zone (≥75%).
        let mut p = make_player(d(2000, 1, 1), 14.0);
        p.player_attributes.condition = 5_000;
        let start = d(2025, 9, 1);
        for i in 0..7 {
            p.process_condition_recovery(start + chrono::Duration::days(i));
        }
        assert!(
            p.player_attributes.condition >= 7_500,
            "condition after week of rest: {}",
            p.player_attributes.condition
        );
        // And it should NOT have overshot the 9000 normal level.
        assert!(p.player_attributes.condition <= 9_000);
    }

    #[test]
    fn older_player_recovers_slower_than_younger_with_same_nf() {
        let mut young = make_player(d(2003, 1, 1), 14.0); // ~22 in 2025
        let mut old = make_player(d(1990, 1, 1), 14.0); // ~35 in 2025
        young.player_attributes.condition = 5_000;
        old.player_attributes.condition = 5_000;
        // Single day rest — magnitudes are small but the ordering is
        // robust (age_factor = 1.1 for young, ≈0.75 for 35).
        for _ in 0..3 {
            young.process_condition_recovery(d(2025, 9, 1));
            old.process_condition_recovery(d(2025, 9, 1));
        }
        assert!(
            young.player_attributes.condition > old.player_attributes.condition,
            "young {} should recover faster than old {}",
            young.player_attributes.condition,
            old.player_attributes.condition
        );
    }

    #[test]
    fn high_natural_fitness_helps_recovery_but_isnt_immune() {
        let mut elite = make_player(d(2000, 1, 1), 19.0);
        let mut average = make_player(d(2000, 1, 1), 8.0);
        elite.player_attributes.condition = 5_000;
        average.player_attributes.condition = 5_000;
        elite.process_condition_recovery(d(2025, 9, 1));
        average.process_condition_recovery(d(2025, 9, 1));
        assert!(
            elite.player_attributes.condition > average.player_attributes.condition,
            "elite NF {} should beat avg NF {}",
            elite.player_attributes.condition,
            average.player_attributes.condition
        );
        // But neither should jump to full
        assert!(elite.player_attributes.condition < 9_000);
    }

    #[test]
    fn condition_label_returning_from_short_injury() {
        let mut p = make_player(d(2000, 1, 1), 14.0);
        p.player_attributes.is_injured = false;
        p.player_attributes.recovery_days_remaining = 5;
        assert_eq!(p.condition_label(), ConditionLabel::ReturningFromInjury);
    }

    #[test]
    fn condition_label_limited_minutes_for_long_recovery() {
        let mut p = make_player(d(2000, 1, 1), 14.0);
        p.player_attributes.is_injured = false;
        p.player_attributes.recovery_days_remaining = 21;
        assert_eq!(
            p.condition_label(),
            ConditionLabel::LimitedMinutesRecommended
        );
    }

    #[test]
    fn condition_label_heavy_legs_when_debt_is_high() {
        let mut p = make_player(d(2000, 1, 1), 14.0);
        p.load.recovery_debt = 500.0;
        p.player_attributes.condition = 9_500;
        assert_eq!(p.condition_label(), ConditionLabel::HeavyLegs);
    }

    #[test]
    fn condition_label_needs_rest_when_jaded() {
        let mut p = make_player(d(2000, 1, 1), 14.0);
        p.player_attributes.jadedness = 8_000;
        assert_eq!(p.condition_label(), ConditionLabel::NeedsRest);
    }

    #[test]
    fn condition_label_lacking_match_sharpness() {
        let mut p = make_player(d(2000, 1, 1), 14.0);
        p.skills.physical.match_readiness = 6.0;
        p.player_attributes.days_since_last_match = 21;
        assert_eq!(p.condition_label(), ConditionLabel::LackingMatchSharpness);
    }

    #[test]
    fn condition_label_fresh_for_well_rested_sharp_player() {
        let mut p = make_player(d(2000, 1, 1), 14.0);
        p.player_attributes.condition = 9_500;
        p.player_attributes.jadedness = 1_500;
        p.skills.physical.match_readiness = 16.0;
        // No matches in last 7d, no debt
        assert_eq!(p.condition_label(), ConditionLabel::Fresh);
    }
}
