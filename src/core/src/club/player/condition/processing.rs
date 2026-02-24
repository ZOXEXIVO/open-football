use crate::club::player::player::Player;
use crate::club::{PlayerStatusType, CONDITION_MAX_VALUE};
use crate::utils::DateUtils;
use chrono::NaiveDate;

impl Player {
    /// Non-injured players slowly recover condition each day, with age and jadedness awareness
    pub(crate) fn process_condition_recovery(&mut self, now: NaiveDate) {
        if self.player_attributes.is_injured {
            return;
        }

        let natural_fitness = self.skills.physical.natural_fitness;
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

        let recovery =
            (base_recovery * age_factor.max(0.5) * jadedness_factor.max(0.5)) as u16;

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

    /// Players not playing lose match sharpness over time
    pub(crate) fn process_match_readiness_decay(&mut self) {
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
