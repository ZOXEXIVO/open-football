use crate::club::player::player::Player;
use crate::club::PlayerStatusType;
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
            self.player_attributes.jadedness =
                (self.player_attributes.jadedness - 150).max(0);
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
