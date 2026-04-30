use crate::HappinessEventType;
use crate::club::player::injury::{BodyPart, InjuryType};
use crate::club::player::player::Player;
use crate::club::{PlayerResult, PlayerStatusType};
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

        if self.player_attributes.is_injured {
            // Phase 1: Injured — decrement injury days
            let transitioned = self.player_attributes.recover_injury_day();
            // Elite physio occasionally heals an extra day.
            if !transitioned && medical.bonus_day() && self.player_attributes.is_injured {
                let _ = self.player_attributes.recover_injury_day();
            }

            if self.player_attributes.injury_days_remaining == 0
                && !self.player_attributes.is_injured
            {
                // Injury countdown hit 0 → transition to recovery phase
                self.statuses.remove(PlayerStatusType::Inj);
                self.statuses.add(now, PlayerStatusType::Lmp);
                result.injury_recovered = true;

                // Match readiness takes a hit on return — even a small
                // injury blunts sharpness. Long-term layoffs lose more.
                let recovery_left = self.player_attributes.recovery_days_remaining as f32;
                let readiness_drop = (recovery_left / 30.0 * 8.0).clamp(2.0, 12.0);
                self.skills.physical.match_readiness =
                    (self.skills.physical.match_readiness - readiness_drop).max(0.0);

                // Morale event: being cleared to play lifts morale, more so
                // after a long absence. Recovery days remaining is our proxy
                // for the severity of what they just went through.
                let magnitude = if recovery_left >= 30.0 {
                    8.0 // long-term injury — big lift, been in rehab for weeks
                } else if recovery_left >= 14.0 {
                    5.0
                } else if recovery_left >= 7.0 {
                    3.0
                } else {
                    1.5
                };
                self.happiness
                    .add_event(HappinessEventType::InjuryReturn, magnitude);
            }
        } else if self.player_attributes.is_in_recovery() {
            // Phase 2: Recovery — post-injury low match fitness phase.
            // Setback risk now uses the unified recipe so workload
            // spikes during rehab actually leak into recurrence chance.
            let setback_chance =
                self.compute_injury_risk(crate::club::player::condition::InjuryRiskInputs {
                    base_rate: 0.001,
                    intensity: 0.5,
                    in_recovery: true,
                    medical_multiplier: medical.setback_multiplier(),
                    now,
                });
            if rand::random::<f32>() < setback_chance {
                if let Some(body_part) =
                    BodyPart::from_u8(self.player_attributes.last_injury_body_part)
                {
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
            // Phase 3: Healthy — daily spontaneous injury risk through the
            // unified recipe. Base 0.0001 / day; the recipe applies age,
            // condition, NF, jadedness, workload spike, congestion,
            // proneness and medical multipliers consistently with the
            // match/training paths.
            let injury_chance =
                self.compute_injury_risk(crate::club::player::condition::InjuryRiskInputs {
                    base_rate: 0.0001,
                    intensity: 0.6,
                    in_recovery: false,
                    medical_multiplier: medical.risk_multiplier(),
                    now,
                });

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
