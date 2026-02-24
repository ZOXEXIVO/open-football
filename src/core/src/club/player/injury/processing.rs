use crate::club::player::injury::{BodyPart, InjuryType};
use crate::club::player::player::Player;
use crate::club::{PlayerResult, PlayerStatusType};
use crate::utils::DateUtils;
use chrono::NaiveDate;

impl Player {
    /// Process injury lifecycle: injured → recovery → healthy
    pub(crate) fn process_injury(&mut self, result: &mut PlayerResult, now: NaiveDate) {
        let injury_proneness = self.player_attributes.injury_proneness;
        let proneness_modifier = injury_proneness as f32 / 10.0;

        if self.player_attributes.is_injured {
            // Phase 1: Injured — decrement injury days
            let transitioned = self.player_attributes.recover_injury_day();

            if transitioned {
                // Injury countdown hit 0 → transition to recovery phase
                self.statuses.remove(PlayerStatusType::Inj);
                self.statuses.add(now, PlayerStatusType::Lmp);
                result.injury_recovered = true;
            }
        } else if self.player_attributes.is_in_recovery() {
            // Phase 2: Recovery — post-injury low match fitness phase
            // Small setback chance (~0.5%) that re-injures the same body part
            if rand::random::<f32>() < 0.001 * proneness_modifier {
                if let Some(body_part) = BodyPart::from_u8(
                    self.player_attributes.last_injury_body_part,
                ) {
                    let injury = Self::injury_for_body_part(body_part);
                    self.player_attributes.set_injury(injury);
                    self.statuses.remove(PlayerStatusType::Lmp);
                    self.statuses.add(now, PlayerStatusType::Inj);
                    result.injury_occurred = Some(injury);
                    return;
                }
            }

            let fully_fit = self.player_attributes.recover_recovery_day();
            if fully_fit {
                // Fully recovered — remove low match fitness status
                self.statuses.remove(PlayerStatusType::Lmp);
                // Clear the last injury body part after a delay (keep for recurring risk tracking)
                // We clear it after full recovery so it doesn't linger forever
                self.player_attributes.last_injury_body_part = 0;
            }
        } else {
            // Phase 3: Healthy — small daily random injury chance
            let age = DateUtils::age(self.birth_date, now);
            let condition_pct = self.player_attributes.condition_percentage();
            let natural_fitness = self.skills.physical.natural_fitness;
            let jadedness = self.player_attributes.jadedness;

            // Base chance: 0.0001 (0.01%)
            let mut injury_chance: f32 = 0.0001;

            // Age modifier: players 30+ have higher risk
            if age > 30 {
                injury_chance += (age as f32 - 30.0) * 0.00004;
            }

            // Low condition increases risk
            if condition_pct < 50 {
                injury_chance += (50.0 - condition_pct as f32) * 0.00001;
            }

            // Low natural fitness increases risk
            if natural_fitness < 10.0 {
                injury_chance += (10.0 - natural_fitness) * 0.00002;
            }

            // Jadedness increases risk
            if jadedness > 5000 {
                injury_chance += (jadedness as f32 - 5000.0) * 0.000002;
            }

            // Injury proneness multiplier
            injury_chance *= proneness_modifier;

            if rand::random::<f32>() < injury_chance {
                let injury = InjuryType::random_spontaneous_injury(injury_proneness);
                self.player_attributes.set_injury(injury);
                self.statuses.add(now, PlayerStatusType::Inj);
                result.injury_occurred = Some(injury);
            }
        }
    }

    /// Get a typical injury for a given body part (used for setback re-injuries)
    fn injury_for_body_part(body_part: BodyPart) -> InjuryType {
        match body_part {
            BodyPart::Hamstring => InjuryType::HamstringStrain,
            BodyPart::Knee => InjuryType::MinorKnock,
            BodyPart::Ankle => InjuryType::AnkleSprain,
            BodyPart::Calf => InjuryType::CalfStrain,
            BodyPart::Groin => InjuryType::GroinStrain,
            BodyPart::Shoulder => InjuryType::ShoulderDislocation,
            BodyPart::Foot => InjuryType::StressFracture,
            BodyPart::Back => InjuryType::BackSpasm,
            BodyPart::Hip => InjuryType::HipFlexorStrain,
            BodyPart::Head => InjuryType::MinorConcussion,
            BodyPart::Quad => InjuryType::QuadStrain,
            BodyPart::Shin => InjuryType::Bruise,
        }
    }
}
