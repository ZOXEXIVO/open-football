use crate::club::player::player::Player;
use crate::club::{PlayerStatusType, CONDITION_MAX_VALUE};
use crate::utils::DateUtils;
use chrono::NaiveDate;

/// Minimum condition floor for injured players (15%)
const INJURY_CONDITION_FLOOR: i16 = 1500;

/// Minimum fitness floor for injured players (20%)
const INJURY_FITNESS_FLOOR: i16 = 2000;

impl Player {
    /// Daily condition processing.
    /// Injured players lose condition, fitness, and match readiness (like FM).
    /// Healthy players recover condition naturally.
    pub(crate) fn process_condition_recovery(&mut self, now: NaiveDate) {
        let natural_fitness = self.skills.physical.natural_fitness;

        if self.player_attributes.is_injured {
            self.process_injury_condition_decay(natural_fitness);
            self.player_attributes.days_since_last_match += 1;
            return;
        }

        let age = DateUtils::age(self.birth_date, now);
        let jadedness = self.player_attributes.jadedness;

        // Base recovery: 80-250 per day based on natural_fitness
        let base_recovery = 80.0 + (natural_fitness / 20.0) * 170.0;

        // Age penalty: older players recover slower
        let age_factor = if age > 30 {
            1.0 - (age as f32 - 30.0) * 0.06
        } else if age < 23 {
            1.1
        } else {
            1.0
        };

        // Jadedness penalty: jaded players recover slower
        let jadedness_factor = 1.0 - (jadedness as f32 / 10000.0) * 0.3;

        // Rest bonus: players without recent matches recover faster
        let rest_bonus = if self.player_attributes.days_since_last_match > 7 {
            1.5 // Fully rested — 50% faster recovery
        } else if self.player_attributes.days_since_last_match > 3 {
            1.3 // Well rested — 30% faster recovery
        } else {
            1.0 // Recently played — normal recovery
        };

        let recovery =
            (base_recovery * age_factor.max(0.5) * jadedness_factor.max(0.5) * rest_bonus) as u16;

        if self.player_attributes.condition < CONDITION_MAX_VALUE {
            self.player_attributes.rest(recovery);
        }

        // Jadedness natural decay: -100/day when no match for 3+ days
        if self.player_attributes.days_since_last_match > 3 {
            self.player_attributes.jadedness =
                (self.player_attributes.jadedness - 100).max(0);
        }

        // Remove Rst status when jadedness drops below threshold
        if self.player_attributes.jadedness < 4000 {
            self.statuses.remove(PlayerStatusType::Rst);
        }

        // Increment days since last match
        self.player_attributes.days_since_last_match += 1;
    }

    /// Injured players lose condition, fitness, and match readiness daily.
    /// Higher natural_fitness slows the decay (like FM's Natural Fitness attribute).
    fn process_injury_condition_decay(&mut self, natural_fitness: f32) {
        let nf_factor = 1.0 - (natural_fitness / 20.0) * 0.7; // 0.3..1.0

        // Condition decay: ~50-150/day depending on natural_fitness
        // At nf=20: ~50/day, at nf=1: ~147/day
        let condition_decay = (150.0 * nf_factor) as i16;
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

    /// Players not playing lose match sharpness over time
    pub(crate) fn process_match_readiness_decay(&mut self) {
        if self.player_attributes.is_injured {
            // Already handled in process_injury_condition_decay
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
