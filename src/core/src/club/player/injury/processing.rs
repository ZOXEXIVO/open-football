use crate::club::player::injury::{BodyPart, InjuryType};
use crate::club::player::player::Player;
use crate::club::{PlayerResult, PlayerStatusType};
use crate::utils::DateUtils;
use chrono::NaiveDate;

/// Additional factor tracking recovery acceleration.
/// Stored transiently — the number of extra days healed this tick.
pub(crate) struct MedicalStaffQuality {
    /// 0.0-1.0 physiotherapy quality (best staff member's attribute / 20)
    pub physio: f32,
    /// 0.0-1.0 sports science quality (best staff member's attribute / 20)
    pub sports_science: f32,
}

impl MedicalStaffQuality {
    /// Extra days healed this tick on top of the normal one-day decrement.
    /// Elite physio (1.0) occasionally heals an extra day; mediocre (0.35)
    /// basically never does.
    fn bonus_day(&self) -> bool {
        // Physio 0.35 → 0% chance, 1.0 → 30% chance — smoothly interpolated
        let chance = (self.physio - 0.3).max(0.0) * 0.45;
        rand::random::<f32>() < chance
    }

    /// Multiplier on spontaneous injury risk.
    /// Elite sports science halves it; mediocre leaves it alone.
    fn risk_multiplier(&self) -> f32 {
        // 0.35 → 1.0, 0.5 → ~0.95, 0.8 → ~0.7, 1.0 → ~0.5
        let bonus = (self.sports_science - 0.35).max(0.0);
        (1.0 - bonus * 0.77).clamp(0.45, 1.05)
    }

    /// Multiplier on setback / re-injury risk during recovery phase.
    /// Sports science matters more here than on the base risk.
    fn setback_multiplier(&self) -> f32 {
        let ss_bonus = (self.sports_science - 0.35).max(0.0);
        let physio_bonus = (self.physio - 0.35).max(0.0);
        (1.0 - ss_bonus * 0.6 - physio_bonus * 0.3).clamp(0.2, 1.05)
    }
}

impl Player {
    /// Process injury lifecycle: injured → recovery → healthy.
    /// The medical staff quality comes from the parent ClubContext and
    /// modulates recovery speed + spontaneous injury risk.
    pub(crate) fn process_injury(
        &mut self,
        result: &mut PlayerResult,
        now: NaiveDate,
        medical: &MedicalStaffQuality,
    ) {
        let injury_proneness = self.player_attributes.injury_proneness;
        let proneness_modifier = injury_proneness as f32 / 10.0;

        if self.player_attributes.is_injured {
            // Phase 1: Injured — decrement injury days
            let transitioned = self.player_attributes.recover_injury_day();
            // Elite physio occasionally heals an extra day.
            if !transitioned && medical.bonus_day() && self.player_attributes.is_injured {
                let _ = self.player_attributes.recover_injury_day();
            }

            if self.player_attributes.injury_days_remaining == 0 && !self.player_attributes.is_injured {
                // Injury countdown hit 0 → transition to recovery phase
                self.statuses.remove(PlayerStatusType::Inj);
                self.statuses.add(now, PlayerStatusType::Lmp);
                result.injury_recovered = true;
            }
        } else if self.player_attributes.is_in_recovery() {
            // Phase 2: Recovery — post-injury low match fitness phase.
            // Setback chance reduced by medical staff quality.
            let setback_chance = 0.001 * proneness_modifier * medical.setback_multiplier();
            if rand::random::<f32>() < setback_chance {
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
            // Elite physio can shave extra days off the recovery phase too.
            if !fully_fit && medical.bonus_day() {
                let _ = self.player_attributes.recover_recovery_day();
            }
            if fully_fit || self.player_attributes.recovery_days_remaining == 0 {
                // Fully recovered — remove low match fitness status
                self.statuses.remove(PlayerStatusType::Lmp);
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

            // Sports science multiplier — elite staff cuts spontaneous
            // injury risk roughly in half.
            injury_chance *= medical.risk_multiplier();

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
