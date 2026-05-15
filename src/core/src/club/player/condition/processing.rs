use crate::club::PlayerStatusType;
use crate::club::player::player::Player;
use crate::league::SeasonPhase;
use crate::utils::DateUtils;
use chrono::NaiveDate;

/// Minimum condition floor for injured players (30%)
const INJURY_CONDITION_FLOOR: i16 = 3000;

/// Minimum fitness floor for injured players (20%)
const INJURY_FITNESS_FLOOR: i16 = 2000;

/// Daily condition-recovery model for non-injured players. Bundles the
/// per-day caps and throttles so callers (training result, daily rest
/// recovery) read the same constants and can't drift. All knobs are
/// associated constants / fns so a future "altitude camp" or
/// "fitness-coach diet plan" growth path adds methods without
/// rippling through the codebase.
pub struct ConditionRecoveryModel;

impl ConditionRecoveryModel {
    /// Baseline ceiling (88%) that rest/training pushes toward when
    /// natural_fitness is zero. The dynamic cap lifts this up to
    /// 9500 (95%) for elite endurance horses.
    pub const NORMAL_BASE: i16 = 8800;

    /// Per-NF-point uplift on the dynamic cap. Picked so a 20.0 NF
    /// player recovers toward 9500 (8800 + 20 × 35).
    pub const NF_UPLIFT_PER_POINT: f32 = 35.0;

    /// Recovery-debt level at which the daily-recovery throttle starts
    /// biting. Mirrors the UI "heavy legs" threshold
    /// (`RECOVERY_DEBT_HEAVY` in `load.rs`). Below this the multiplier
    /// is exactly 1.0; above it the smoothstep curve takes over.
    pub const DEBT_THROTTLE_START: f32 = 350.0;

    /// Recovery-debt level at which the throttle reaches its floor.
    /// Between `DEBT_THROTTLE_START` and `DEBT_THROTTLE_FULL` the
    /// multiplier follows a smoothstep curve (3t² − 2t³) so the
    /// transition is C¹-continuous — no cliff at the start, no cliff
    /// at the floor.
    pub const DEBT_THROTTLE_FULL: f32 = 1_500.0;

    /// Lower bound on the daily-recovery multiplier when debt is at or
    /// above `DEBT_THROTTLE_FULL`. The body always recovers something
    /// each day — even a structurally-overloaded pro is not at zero.
    pub const MIN_RECOVERY_MULT: f32 = 0.45;

    /// Slow-drain coefficient applied to the daily natural-recovery
    /// magnitude when consuming recovery_debt outside training. A
    /// player resting on a no-training day still bleeds off some
    /// deep tiredness — just slower than a structured recovery
    /// session would.
    pub const NATURAL_DEBT_DRAIN_FACTOR: f32 = 0.12;

    /// Dynamic ceiling for rest/training condition recovery. Genetic
    /// recovery ceiling: a player with elite `natural_fitness`
    /// recovers toward a higher daily peak than a player born with
    /// average aerobic genetics.
    ///
    /// Semantics: `natural_fitness` is the recovery ceiling — the
    /// "how high can this player bounce back" trait, not the chronic
    /// training base. `player_attributes.fitness` is the in-season
    /// chronic base (built up by training, depleted by inactivity);
    /// it sits BENEATH this cap and is governed by the fitness
    /// adaptation in `PlayerTrainingResult::apply_to_player`. The two
    /// are intentionally distinct: NF is genetics, fitness is training.
    ///
    /// Range: NF 0.0 → 8800 (88%); NF 20.0 → 9500 (95%).
    pub fn dynamic_cap(natural_fitness: f32) -> i16 {
        let nf = natural_fitness.clamp(0.0, 20.0);
        Self::NORMAL_BASE + (nf * Self::NF_UPLIFT_PER_POINT) as i16
    }

    /// Daily-rest condition recovery throttle. Returns 1.0 below the
    /// `DEBT_THROTTLE_START` "heavy legs" threshold and falls smoothly
    /// (smoothstep, 3t² − 2t³) to `MIN_RECOVERY_MULT` (0.45) as debt
    /// reaches `DEBT_THROTTLE_FULL` (1500). The body prioritises
    /// clearing structural fatigue over restoring acute freshness.
    ///
    /// The smoothstep replaces the old linear cliff which dropped the
    /// multiplier from 1.0 to ~0.77 the moment debt crossed 350 —
    /// realistically a player at debt 351 is barely more limited than
    /// one at debt 349.
    pub fn debt_throttle(recovery_debt: f32) -> f32 {
        if recovery_debt <= Self::DEBT_THROTTLE_START {
            return 1.0;
        }
        let span = Self::DEBT_THROTTLE_FULL - Self::DEBT_THROTTLE_START;
        if span <= 0.0 {
            return Self::MIN_RECOVERY_MULT;
        }
        let t = ((recovery_debt - Self::DEBT_THROTTLE_START) / span).clamp(0.0, 1.0);
        let smooth_t = t * t * (3.0 - 2.0 * t);
        1.0 - smooth_t * (1.0 - Self::MIN_RECOVERY_MULT)
    }
}

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

        // Dynamic cap: elite endurance players recover toward a higher
        // ceiling than average pros (88%..95% based on natural fitness).
        let cap = ConditionRecoveryModel::dynamic_cap(natural_fitness);

        // Only recover if below the dynamic cap.
        if condition < cap {
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

            // Recovery-debt throttle: once deep tiredness crosses
            // ~350 ("heavy legs"), daily condition recovery is capped
            // because the body is dealing with a structural backlog,
            // not just acute fatigue. Falls from 1.0 → 0.45 as debt
            // climbs to 1500.
            let recovery_debt_penalty =
                ConditionRecoveryModel::debt_throttle(self.load.recovery_debt);

            let recovery = (base_recovery
                * age_factor.max(0.5)
                * jadedness_factor.max(0.5)
                * rest_bonus
                * phase_bonus
                * recovery_debt_penalty) as u16;

            // Cap recovery so we don't overshoot the dynamic ceiling.
            let max_gain = (cap - condition) as u16;
            self.player_attributes.rest(recovery.min(max_gain));
        }

        // Natural recovery debt drain — independent of training. The
        // body's slow constant-time housekeeping bleeds off some debt
        // even on pure rest days. Phase bonus folds in (off-season /
        // winter-break = bigger drain).
        let phase_bonus = SeasonPhase::from_date(now).condition_recovery_multiplier();
        let natural_debt_reduction = (200.0 + (natural_fitness / 20.0) * 300.0)
            * ConditionRecoveryModel::NATURAL_DEBT_DRAIN_FACTOR
            * phase_bonus;
        self.load.consume_recovery_debt(natural_debt_reduction);

        // Jadedness natural decay: -150/day when no match for 3+ days
        if self.player_attributes.days_since_last_match > 3 {
            self.player_attributes.jadedness = (self.player_attributes.jadedness - 150).max(0);
        }

        // Remove Rst status when jadedness drops below threshold
        if self.player_attributes.jadedness < 4000 {
            self.statuses.remove(PlayerStatusType::Rst);
        }

        // Fitness atrophy during prolonged inactivity. Defined narrowly:
        // > 14 days since last match AND the rolling physical-load
        // window is near zero (i.e. no meaningful training either).
        // Without the load gate we'd decay fitness for any player
        // training every day but with no matches in the calendar —
        // exactly backwards from intent. Decay is 10..30/day depending
        // on age (older players atrophy faster) so a player abandoned
        // on a free transfer or a long-injured pro losing their
        // base fitness reads correctly.
        if self.player_attributes.days_since_last_match > 14 && self.load.physical_load_7 < 15.0 {
            let age_decay_base = if age >= 30 { 30.0 } else { 10.0 + (age as f32 - 16.0).max(0.0) };
            let fitness_decay = age_decay_base.clamp(10.0, 30.0) as i16;
            self.player_attributes.fitness =
                (self.player_attributes.fitness - fitness_decay).max(INJURY_FITNESS_FLOOR);
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
    use super::ConditionRecoveryModel;
    use crate::club::player::builder::PlayerBuilder;
    use crate::club::player::condition::ConditionLabel;
    use crate::club::player::player::Player;
    use crate::shared::fullname::FullName;
    use crate::{
        PersonAttributes, PlayerAttributes, PlayerPosition, PlayerPositionType, PlayerPositions,
        PlayerSkills,
    };
    use chrono::NaiveDate;

    #[test]
    fn debt_throttle_returns_one_at_or_below_threshold() {
        assert_eq!(ConditionRecoveryModel::debt_throttle(0.0), 1.0);
        assert_eq!(ConditionRecoveryModel::debt_throttle(200.0), 1.0);
        assert_eq!(
            ConditionRecoveryModel::debt_throttle(ConditionRecoveryModel::DEBT_THROTTLE_START),
            1.0
        );
    }

    #[test]
    fn debt_throttle_just_above_threshold_stays_close_to_one() {
        // The whole point of the smoothstep replacement: at debt 351
        // the throttle should be effectively 1.0, not the old ~0.77
        // cliff the linear formula produced.
        let just_above = ConditionRecoveryModel::debt_throttle(351.0);
        assert!(
            just_above > 0.999,
            "throttle just above start should be ~1.0, got {}",
            just_above
        );
    }

    #[test]
    fn debt_throttle_floors_at_min_recovery_mult_when_debt_is_extreme() {
        let at_full =
            ConditionRecoveryModel::debt_throttle(ConditionRecoveryModel::DEBT_THROTTLE_FULL);
        assert!(
            (at_full - ConditionRecoveryModel::MIN_RECOVERY_MULT).abs() < 1e-4,
            "throttle at full should equal MIN_RECOVERY_MULT, got {}",
            at_full
        );
        let way_past =
            ConditionRecoveryModel::debt_throttle(ConditionRecoveryModel::DEBT_THROTTLE_FULL * 2.0);
        assert!(
            (way_past - ConditionRecoveryModel::MIN_RECOVERY_MULT).abs() < 1e-4,
            "throttle past full should clamp to MIN_RECOVERY_MULT, got {}",
            way_past
        );
    }

    #[test]
    fn debt_throttle_is_monotonic_decreasing() {
        // The smoothstep must never temporarily reverse direction —
        // a higher debt always means a smaller recovery multiplier.
        let mut prev = ConditionRecoveryModel::debt_throttle(0.0);
        let mut debt = 0.0;
        while debt <= 2_000.0 {
            let next = ConditionRecoveryModel::debt_throttle(debt);
            assert!(
                next <= prev + 1e-5,
                "throttle non-monotonic at debt {}: prev={} next={}",
                debt,
                prev,
                next
            );
            prev = next;
            debt += 25.0;
        }
    }

    #[test]
    fn debt_throttle_mid_band_is_between_endpoints() {
        // A debt halfway through the throttle band should sit between
        // the endpoints (sanity check against accidentally inverting
        // the smoothstep).
        let mid_debt = (ConditionRecoveryModel::DEBT_THROTTLE_START
            + ConditionRecoveryModel::DEBT_THROTTLE_FULL)
            / 2.0;
        let mid = ConditionRecoveryModel::debt_throttle(mid_debt);
        assert!(mid < 1.0);
        assert!(mid > ConditionRecoveryModel::MIN_RECOVERY_MULT);
    }

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
