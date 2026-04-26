use crate::club::player::injury::InjuryType;
use crate::{HappinessEventType, MentalGains, PhysicalGains, PlayerStatusType, SimulatorData, TechnicalGains, TrainingEffects};

pub struct PlayerTrainingResult {
    pub player_id: u32,
    pub effects: TrainingEffects,
    /// How well the player executed this session (1.0-20.0)
    pub session_performance: f32,
}

impl PlayerTrainingResult {
    pub fn new(player_id: u32, effects: TrainingEffects) -> Self {
        PlayerTrainingResult {
            player_id,
            effects,
            session_performance: 10.0,
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
                physical_load_units: 0.0,
                high_intensity_share: 0.0,
                readiness_change: 0.0,
            },
            session_performance: 10.0,
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

            // Recalculate current_ability from actual skill values, weighted
            // by the player's primary position. Using the position-weighted
            // path keeps natural development and training in agreement —
            // both feed back through the same ability function so neither
            // path can quietly contradict the other's growth gate.
            let position = player.position();
            player.player_attributes.current_ability =
                player.skills.calculate_ability_for_position(position);

            // Update rolling training performance (exponential moving average)
            // Alpha = 0.3 for first 5 sessions (fast warmup), then 0.15 (slower, more stable)
            let alpha = if player.training.sessions_completed < 5 { 0.3 } else { 0.15 };
            player.training.training_performance = player.training.training_performance * (1.0 - alpha)
                + self.session_performance * alpha;
            player.training.sessions_completed = player.training.sessions_completed.saturating_add(1);

            // Apply fatigue changes
            // Negative fatigue_change = recovery (condition increases)
            // Positive fatigue_change = fatigue (condition decreases)
            // Cap recovery at 90% (normal level) — training restores toward normal, not to 100%
            // Floor at 30% — condition never drops below this
            let new_condition = player.player_attributes.condition as f32 - self.effects.fatigue_change;
            let condition_cap = if self.effects.fatigue_change < 0.0 { 9000.0 } else { 10000.0 };
            player.player_attributes.condition = new_condition.clamp(3000.0, condition_cap) as i16;

            // Workload bookkeeping into PlayerLoad. Heavy sessions add to
            // physical_load_7/30 + recovery_debt; recovery sessions burn
            // off accumulated debt. This is the single signal selection
            // and injury-risk consult — keeping training and matches on
            // the same scale.
            if self.effects.physical_load_units > 0.0 {
                let hi = self.effects.physical_load_units * self.effects.high_intensity_share;
                player
                    .load
                    .record_training_load(self.effects.physical_load_units, hi);
                // Heavy training adds debt at ~20% the session load.
                player
                    .load
                    .add_recovery_debt(self.effects.physical_load_units * 0.20);
            }
            if self.effects.fatigue_change < 0.0 {
                // Recovery sessions drain debt at ~30% of the magnitude
                // of the condition gain (so a Recovery session worth
                // -800 burns ~240 debt — a real bounce-back).
                player
                    .load
                    .consume_recovery_debt(-self.effects.fatigue_change * 0.30);
            }

            // Apply injury risk — unified recipe. Translate the
            // session-type base risk into the shared model so the same
            // workload signals (jadedness, condition, ACWR spike,
            // congestion, recovery phase) are read by spontaneous,
            // training, and match paths consistently.
            let intensity_factor =
                ((self.effects.fatigue_change.abs() + self.effects.physical_load_units) / 60.0)
                    .clamp(0.4, 2.0);
            let in_recovery = player.player_attributes.is_in_recovery();
            let chance = player.compute_injury_risk(
                crate::club::player::condition::InjuryRiskInputs {
                    base_rate: self.effects.injury_risk.max(0.0),
                    intensity: intensity_factor,
                    in_recovery,
                    medical_multiplier: 1.0,
                    now: current_date,
                },
            );
            if rand::random::<f32>() < chance {
                let age = crate::utils::DateUtils::age(player.birth_date, current_date);
                let condition_pct = player.player_attributes.condition_percentage();
                let natural_fitness = player.skills.physical.natural_fitness;
                let injury_proneness = player.player_attributes.injury_proneness;

                let injury = InjuryType::random_training_injury(age, condition_pct, natural_fitness, injury_proneness);
                player.player_attributes.set_injury(injury);
                player.statuses.add(
                    current_date,
                    PlayerStatusType::Inj,
                );
            }

            // Update match readiness from the per-session readiness_change
            // — replaces the old blanket "any negative fatigue gives +2"
            // rule. PressingDrills / MatchPreparation now sharpen players
            // properly; passive video / RestDay barely move the needle.
            if self.effects.readiness_change != 0.0 {
                player.skills.physical.match_readiness = (player
                    .skills
                    .physical
                    .match_readiness
                    + self.effects.readiness_change)
                    .clamp(0.0, 20.0);
            }
            // Very intense sessions on already-tired legs blunt readiness
            // (a hard pressing drill on a Friday with the legs gone).
            if self.effects.fatigue_change > 120.0
                && player.player_attributes.condition_percentage() < 60
            {
                player.skills.physical.match_readiness =
                    (player.skills.physical.match_readiness - 0.5).max(0.0);
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
