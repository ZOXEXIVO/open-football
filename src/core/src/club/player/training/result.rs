use crate::club::player::injury::InjuryType;
use crate::{HappinessEventType, MentalGains, PhysicalGains, PlayerStatusType, SimulatorData, TechnicalGains, TrainingEffects};

pub struct PlayerTrainingResult {
    pub player_id: u32,
    pub effects: TrainingEffects,
}

impl PlayerTrainingResult {
    pub fn new(player_id: u32, effects: TrainingEffects) -> Self {
        PlayerTrainingResult {
            player_id,
            effects,
        }
    }

    pub fn empty(player_id: u32) -> Self {
        PlayerTrainingResult {
            player_id,
            effects: TrainingEffects {
                physical_gains: PhysicalGains::default(),
                technical_gains: TechnicalGains::default(),
                mental_gains: MentalGains::default(),
                fatigue_change: 0.0,
                injury_risk: 0.0,
                morale_change: 0.0,
            },
        }
    }

    /// Apply the training effects to the player
    /// This is where the actual skill updates happen with mutable references
    pub fn process(&self, data: &mut SimulatorData) {
        let current_date = data.date.date();
        // Get mutable reference to the player
        if let Some(player) = data.player_mut(self.player_id) {
            // Gate skill gains by ability gap: players near potential barely grow
            let current_ability = player.player_attributes.current_ability as f32;
            let potential_ability = player.player_attributes.potential_ability as f32;
            let growth_factor = if potential_ability <= 0.0 {
                0.05 // No potential set — minimal growth
            } else {
                let gap_ratio = (potential_ability - current_ability) / potential_ability;
                gap_ratio.clamp(0.05, 1.0) // At least 5% gains (tiny beyond potential)
            };

            // Apply physical gains (scaled by growth factor)
            player.skills.physical.stamina = (player.skills.physical.stamina + self.effects.physical_gains.stamina * growth_factor).min(20.0);
            player.skills.physical.strength = (player.skills.physical.strength + self.effects.physical_gains.strength * growth_factor).min(20.0);
            player.skills.physical.pace = (player.skills.physical.pace + self.effects.physical_gains.pace * growth_factor).min(20.0);
            player.skills.physical.agility = (player.skills.physical.agility + self.effects.physical_gains.agility * growth_factor).min(20.0);
            player.skills.physical.balance = (player.skills.physical.balance + self.effects.physical_gains.balance * growth_factor).min(20.0);
            player.skills.physical.jumping = (player.skills.physical.jumping + self.effects.physical_gains.jumping * growth_factor).min(20.0);
            player.skills.physical.natural_fitness = (player.skills.physical.natural_fitness + self.effects.physical_gains.natural_fitness * growth_factor).min(20.0);

            // Apply technical gains (scaled by growth factor)
            player.skills.technical.first_touch = (player.skills.technical.first_touch + self.effects.technical_gains.first_touch * growth_factor).min(20.0);
            player.skills.technical.passing = (player.skills.technical.passing + self.effects.technical_gains.passing * growth_factor).min(20.0);
            player.skills.technical.crossing = (player.skills.technical.crossing + self.effects.technical_gains.crossing * growth_factor).min(20.0);
            player.skills.technical.dribbling = (player.skills.technical.dribbling + self.effects.technical_gains.dribbling * growth_factor).min(20.0);
            player.skills.technical.finishing = (player.skills.technical.finishing + self.effects.technical_gains.finishing * growth_factor).min(20.0);
            player.skills.technical.heading = (player.skills.technical.heading + self.effects.technical_gains.heading * growth_factor).min(20.0);
            player.skills.technical.tackling = (player.skills.technical.tackling + self.effects.technical_gains.tackling * growth_factor).min(20.0);
            player.skills.technical.technique = (player.skills.technical.technique + self.effects.technical_gains.technique * growth_factor).min(20.0);

            // Apply mental gains (scaled by growth factor)
            player.skills.mental.concentration = (player.skills.mental.concentration + self.effects.mental_gains.concentration * growth_factor).min(20.0);
            player.skills.mental.decisions = (player.skills.mental.decisions + self.effects.mental_gains.decisions * growth_factor).min(20.0);
            player.skills.mental.positioning = (player.skills.mental.positioning + self.effects.mental_gains.positioning * growth_factor).min(20.0);
            player.skills.mental.teamwork = (player.skills.mental.teamwork + self.effects.mental_gains.teamwork * growth_factor).min(20.0);
            player.skills.mental.vision = (player.skills.mental.vision + self.effects.mental_gains.vision * growth_factor).min(20.0);
            player.skills.mental.work_rate = (player.skills.mental.work_rate + self.effects.mental_gains.work_rate * growth_factor).min(20.0);
            player.skills.mental.leadership = (player.skills.mental.leadership + self.effects.mental_gains.leadership * growth_factor).min(20.0);

            // Recalculate current_ability from actual skill values
            player.player_attributes.current_ability = player.skills.calculate_ability();

            // Apply fatigue changes
            // Negative fatigue_change = recovery (condition increases)
            // Positive fatigue_change = fatigue (condition decreases)
            // Cap recovery at 90% (normal level) — training restores toward normal, not to 100%
            // Floor at 30% — condition never drops below this
            let new_condition = player.player_attributes.condition as f32 - self.effects.fatigue_change;
            let condition_cap = if self.effects.fatigue_change < 0.0 { 9000.0 } else { 10000.0 };
            player.player_attributes.condition = new_condition.clamp(3000.0, condition_cap) as i16;

            // Apply injury risk — use proper injury system
            let injury_proneness = player.player_attributes.injury_proneness;
            let proneness_modifier = injury_proneness as f32 / 10.0;
            if rand::random::<f32>() < self.effects.injury_risk * proneness_modifier {
                let age = crate::utils::DateUtils::age(player.birth_date, current_date);
                let condition_pct = player.player_attributes.condition_percentage();
                let natural_fitness = player.skills.physical.natural_fitness;

                let injury = InjuryType::random_training_injury(age, condition_pct, natural_fitness, injury_proneness);
                player.player_attributes.set_injury(injury);
                player.statuses.add(
                    current_date,
                    PlayerStatusType::Inj,
                );
            }

            // Update match readiness based on training
            if self.effects.fatigue_change < 0.0 {
                // Recovery training improves match readiness
                player.skills.physical.match_readiness = (player.skills.physical.match_readiness + 2.0).min(20.0);
            } else if self.effects.fatigue_change > 20.0 {
                // Intense training reduces match readiness
                player.skills.physical.match_readiness = (player.skills.physical.match_readiness - 1.0).max(0.0);
            }

            // Apply morale changes to happiness system
            if self.effects.morale_change.abs() > 0.001 {
                let event_type = if self.effects.morale_change > 0.0 {
                    HappinessEventType::GoodTraining
                } else {
                    HappinessEventType::PoorTraining
                };
                player.happiness.add_event(event_type, self.effects.morale_change * 5.0);
                player.happiness.adjust_morale(self.effects.morale_change * 3.0);

                // Good training still has a chance to improve behaviour
                if self.effects.morale_change > 0.0 && rand::random::<f32>() < self.effects.morale_change {
                    player.behaviour.try_increase();
                }
            }
        }
    }
}
