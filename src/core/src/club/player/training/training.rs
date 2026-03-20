use crate::club::player::training::result::PlayerTrainingResult;
use crate::{MentalGains, Person, PhysicalGains, Player, Staff, TechnicalGains, TrainingEffects, TrainingIntensity, TrainingSession, TrainingType};
use chrono::NaiveDateTime;

#[derive(Debug, Clone)]
pub struct PlayerTraining {
    /// Rolling average of actual training session quality (1.0-20.0).
    /// Measures execution quality, not just effort/personality.
    /// Updated each training session via exponential moving average.
    pub training_performance: f32,
    /// How many sessions this player has completed (for EMA warmup)
    pub sessions_completed: u16,
}

impl Default for PlayerTraining {
    fn default() -> Self {
        Self::new()
    }
}

impl PlayerTraining {
    pub fn new() -> Self {
        PlayerTraining {
            training_performance: 10.0, // Neutral starting point
            sessions_completed: 0,
        }
    }

    pub fn train(
        player: &Player,
        coach: &Staff,
        session: &TrainingSession,
        date: NaiveDateTime,
    ) -> PlayerTrainingResult {
        let mut effects = TrainingEffects {
            physical_gains: PhysicalGains::default(),
            technical_gains: TechnicalGains::default(),
            mental_gains: MentalGains::default(),
            fatigue_change: 0.0,
            injury_risk: 0.0,
            morale_change: 0.0,
        };

        // Base effectiveness factors
        let coach_quality = Self::calculate_coach_effectiveness(coach, &session.session_type);
        let player_receptiveness = Self::calculate_player_receptiveness(player, coach, date.date());
        let age_factor = Self::calculate_age_training_factor(player.age(date.date()));
        let potential_factor = Self::calculate_potential_development_factor(player, date.date());

        // Intensity multipliers
        let intensity_multiplier = match session.intensity {
            TrainingIntensity::VeryLight => 0.3,
            TrainingIntensity::Light => 0.5,
            TrainingIntensity::Moderate => 1.0,
            TrainingIntensity::High => 1.5,
            TrainingIntensity::VeryHigh => 2.0,
        };

        // Calculate gains based on training type
        // Injury risk: real-world training injury rate is ~0.1-0.4% per session.
        // Previous values (1-4%) caused 80%+ of squads to be injured at any time.
        match session.session_type {
            TrainingType::Endurance => {
                effects.physical_gains.stamina = 0.05 * coach_quality * player_receptiveness * age_factor;
                effects.physical_gains.natural_fitness = 0.03 * coach_quality * player_receptiveness * age_factor;
                effects.fatigue_change = -100.0 * intensity_multiplier; // Net recovery — endurance builds fitness
                effects.injury_risk = 0.002 * intensity_multiplier;
            }
            TrainingType::Strength => {
                effects.physical_gains.strength = 0.04 * coach_quality * player_receptiveness * age_factor;
                effects.physical_gains.jumping = 0.02 * coach_quality * player_receptiveness * age_factor;
                effects.fatigue_change = 100.0 * intensity_multiplier; // Tiring — heavy session
                effects.injury_risk = 0.003 * intensity_multiplier;
            }
            TrainingType::Speed => {
                effects.physical_gains.pace = 0.03 * coach_quality * player_receptiveness * age_factor;
                effects.physical_gains.agility = 0.04 * coach_quality * player_receptiveness * age_factor;
                effects.fatigue_change = 150.0 * intensity_multiplier; // Most tiring physical session
                effects.injury_risk = 0.004 * intensity_multiplier;
            }
            TrainingType::BallControl => {
                effects.technical_gains.first_touch = 0.05 * coach_quality * player_receptiveness;
                effects.technical_gains.technique = 0.04 * coach_quality * player_receptiveness;
                effects.technical_gains.dribbling = 0.03 * coach_quality * player_receptiveness;
                effects.fatigue_change = -50.0 * intensity_multiplier; // Light — slight recovery
                effects.injury_risk = 0.001 * intensity_multiplier;
            }
            TrainingType::Passing => {
                effects.technical_gains.passing = 0.06 * coach_quality * player_receptiveness;
                effects.mental_gains.vision = 0.02 * coach_quality * player_receptiveness;
                effects.fatigue_change = -100.0 * intensity_multiplier; // Light — net recovery
                effects.injury_risk = 0.001 * intensity_multiplier;
            }
            TrainingType::Shooting => {
                effects.technical_gains.finishing = 0.05 * coach_quality * player_receptiveness;
                effects.technical_gains.technique = 0.02 * coach_quality * player_receptiveness;
                effects.mental_gains.decisions = 0.01 * coach_quality * player_receptiveness;
                effects.fatigue_change = 50.0 * intensity_multiplier; // Moderate physical load
                effects.injury_risk = 0.002 * intensity_multiplier;
            }
            TrainingType::Positioning => {
                effects.mental_gains.positioning = 0.06 * coach_quality * player_receptiveness;
                effects.mental_gains.concentration = 0.03 * coach_quality * player_receptiveness;
                effects.mental_gains.decisions = 0.02 * coach_quality * player_receptiveness;
                effects.fatigue_change = -150.0 * intensity_multiplier; // Tactical — good recovery
                effects.injury_risk = 0.0005 * intensity_multiplier;
            }
            TrainingType::TeamShape => {
                effects.mental_gains.teamwork = 0.05 * coach_quality * player_receptiveness;
                effects.mental_gains.positioning = 0.04 * coach_quality * player_receptiveness;
                effects.mental_gains.work_rate = 0.02 * coach_quality * player_receptiveness;
                effects.fatigue_change = -50.0 * intensity_multiplier; // Moderate — slight recovery
                effects.injury_risk = 0.001 * intensity_multiplier;
                effects.morale_change = 0.1; // Team activities boost morale
            }
            TrainingType::Recovery => {
                effects.fatigue_change = -800.0; // Strong recovery — main condition restoration
                effects.injury_risk = -0.002;
                effects.morale_change = 0.05;
            }
            TrainingType::VideoAnalysis => {
                effects.mental_gains.decisions = 0.03 * coach_quality;
                effects.mental_gains.positioning = 0.02 * coach_quality;
                effects.mental_gains.vision = 0.02 * coach_quality;
                effects.fatigue_change = -200.0; // Light recovery while watching video
                effects.injury_risk = 0.0;
            }
            TrainingType::RestDay => {
                effects.fatigue_change = -600.0; // Full rest day — good recovery
                effects.injury_risk = -0.003;
                effects.morale_change = 0.03;
            }
            TrainingType::LightRecovery => {
                effects.fatigue_change = -500.0; // Light recovery session
                effects.injury_risk = -0.001;
                effects.morale_change = 0.02;
            }
            TrainingType::Rehabilitation => {
                effects.fatigue_change = -400.0; // Rehab recovery
                effects.injury_risk = -0.002;
            }
            _ => {
                // Default minimal gains for unspecified training types
                effects.fatigue_change = 10.0 * intensity_multiplier;
                effects.injury_risk = 0.001 * intensity_multiplier;
            }
        }

        // Apply player condition modifiers
        let condition_factor = player.player_attributes.condition_percentage() as f32 / 100.0;
        if condition_factor < 0.7 {
            effects.injury_risk *= 1.5; // Higher injury risk when tired
            effects.fatigue_change *= 1.2; // Get tired faster when already fatigued
        }

        // Apply professionalism bonus to gains
        let professionalism_bonus = player.attributes.professionalism / 20.0;
        effects.physical_gains = Self::apply_bonus_to_physical(effects.physical_gains, professionalism_bonus);
        effects.technical_gains = Self::apply_bonus_to_technical(effects.technical_gains, professionalism_bonus);
        effects.mental_gains = Self::apply_bonus_to_mental(effects.mental_gains, professionalism_bonus);

        // Apply potential development factor to all skill gains
        effects.physical_gains = Self::scale_physical(effects.physical_gains, potential_factor);
        effects.technical_gains = Self::scale_technical(effects.technical_gains, potential_factor);
        effects.mental_gains = Self::scale_mental(effects.mental_gains, potential_factor);

        // Calculate session performance score (1-20):
        // How well did the player actually execute this session?
        // Based on: gains achieved + effort + coach synergy + randomness
        let total_gains = effects.physical_gains.total()
            + effects.technical_gains.total()
            + effects.mental_gains.total();

        // Normalize gains to 0-10 scale (typical session total gain is 0.01-0.15)
        let gain_score = (total_gains * 80.0).clamp(0.0, 10.0);

        // Effort component: professionalism + work_rate + determination
        let effort_score = (player.attributes.professionalism
            + player.skills.mental.work_rate
            + player.skills.mental.determination) / 3.0;

        // Execution randomness: even good players have bad days
        let execution_roll = rand::random::<f32>() * 4.0 - 2.0; // -2 to +2

        // Coach synergy: good coach-player fit produces better sessions
        let synergy = coach_quality * player_receptiveness;

        let session_performance = (gain_score + effort_score * 0.5 + synergy * 2.0 + execution_roll)
            .clamp(1.0, 20.0);

        let mut result = PlayerTrainingResult::new(player.id, effects);
        result.session_performance = session_performance;
        result
    }

    fn apply_bonus_to_physical(mut gains: PhysicalGains, bonus: f32) -> PhysicalGains {
        gains.stamina *= 1.0 + bonus;
        gains.strength *= 1.0 + bonus;
        gains.pace *= 1.0 + bonus;
        gains.agility *= 1.0 + bonus;
        gains.balance *= 1.0 + bonus;
        gains.jumping *= 1.0 + bonus;
        gains.natural_fitness *= 1.0 + bonus;
        gains
    }

    fn apply_bonus_to_technical(mut gains: TechnicalGains, bonus: f32) -> TechnicalGains {
        gains.first_touch *= 1.0 + bonus;
        gains.passing *= 1.0 + bonus;
        gains.crossing *= 1.0 + bonus;
        gains.dribbling *= 1.0 + bonus;
        gains.finishing *= 1.0 + bonus;
        gains.heading *= 1.0 + bonus;
        gains.tackling *= 1.0 + bonus;
        gains.technique *= 1.0 + bonus;
        gains
    }

    fn apply_bonus_to_mental(mut gains: MentalGains, bonus: f32) -> MentalGains {
        gains.concentration *= 1.0 + bonus;
        gains.decisions *= 1.0 + bonus;
        gains.positioning *= 1.0 + bonus;
        gains.teamwork *= 1.0 + bonus;
        gains.vision *= 1.0 + bonus;
        gains.work_rate *= 1.0 + bonus;
        gains.leadership *= 1.0 + bonus;
        gains
    }


    fn calculate_coach_effectiveness(coach: &Staff, training_type: &TrainingType) -> f32 {
        let base_effectiveness = match training_type {
            TrainingType::Endurance | TrainingType::Strength | TrainingType::Speed => {
                coach.staff_attributes.coaching.fitness as f32 / 20.0
            }
            TrainingType::BallControl | TrainingType::Passing | TrainingType::Shooting => {
                coach.staff_attributes.coaching.technical as f32 / 20.0
            }
            TrainingType::Positioning | TrainingType::TeamShape => {
                coach.staff_attributes.coaching.tactical as f32 / 20.0
            }
            TrainingType::Concentration | TrainingType::DecisionMaking => {
                coach.staff_attributes.coaching.mental as f32 / 20.0
            }
            _ => {
                // Average of all coaching attributes
                (coach.staff_attributes.coaching.attacking +
                    coach.staff_attributes.coaching.defending +
                    coach.staff_attributes.coaching.tactical +
                    coach.staff_attributes.coaching.technical) as f32 / 80.0
            }
        };

        // Add determination factor
        let determination_factor = coach.staff_attributes.mental.determination as f32 / 20.0;

        (base_effectiveness * 0.7 + determination_factor * 0.3).min(1.0)
    }

    fn calculate_player_receptiveness(player: &Player, coach: &Staff, sim_date: chrono::NaiveDate) -> f32 {
        // Base receptiveness from player attributes
        let base = (player.attributes.professionalism + player.attributes.ambition) / 40.0;

        // Relationship with coach affects receptiveness
        let relationship_bonus = if coach.relations.is_favorite_player(player.id) {
            0.2
        } else if coach.relations.get_player(player.id).map_or(false, |r| r.level < -50.0) {
            -0.2
        } else {
            0.0
        };

        // Age affects receptiveness (younger players learn faster)
        let age_bonus = match player.age(sim_date) {
            16..=20 => 0.3,
            21..=24 => 0.2,
            25..=28 => 0.1,
            29..=32 => 0.0,
            _ => -0.1,
        };

        (base + relationship_bonus + age_bonus).clamp(0.1, 1.5)
    }

    fn calculate_age_training_factor(age: u8) -> f32 {
        match age {
            16..=18 => 1.5,  // Youth develop quickly
            19..=21 => 1.3,
            22..=24 => 1.1,
            25..=27 => 1.0,
            28..=30 => 0.8,
            31..=33 => 0.5,
            34..=36 => 0.3,
            _ => 0.1,         // Very old players barely improve
        }
    }

    /// Players with large gap between potential and current ability develop faster.
    /// The effect is amplified for younger players who have more room to grow.
    fn calculate_potential_development_factor(player: &Player, sim_date: chrono::NaiveDate) -> f32 {
        let pa = player.player_attributes.potential_ability as f32;
        let ca = player.player_attributes.current_ability as f32;

        if pa <= ca || pa == 0.0 {
            return 1.0;
        }

        // Gap ratio: 0.0 (no gap) to 1.0 (CA=0, PA=max)
        let gap_ratio = (pa - ca) / pa;

        // Age multiplier: young players benefit more from high potential
        let age = player.age(sim_date);
        let age_mult = match age {
            0..=18 => 1.5,
            19..=21 => 1.3,
            22..=24 => 1.0,
            25..=27 => 0.6,
            28..=30 => 0.3,
            _ => 0.1,
        };

        // Result: 1.0 (no boost) up to ~2.0 for young high-PA players far from ceiling
        1.0 + gap_ratio * age_mult * 0.7
    }

    fn scale_physical(mut gains: PhysicalGains, factor: f32) -> PhysicalGains {
        gains.stamina *= factor;
        gains.strength *= factor;
        gains.pace *= factor;
        gains.agility *= factor;
        gains.balance *= factor;
        gains.jumping *= factor;
        gains.natural_fitness *= factor;
        gains
    }

    fn scale_technical(mut gains: TechnicalGains, factor: f32) -> TechnicalGains {
        gains.first_touch *= factor;
        gains.passing *= factor;
        gains.crossing *= factor;
        gains.dribbling *= factor;
        gains.finishing *= factor;
        gains.heading *= factor;
        gains.tackling *= factor;
        gains.technique *= factor;
        gains
    }

    fn scale_mental(mut gains: MentalGains, factor: f32) -> MentalGains {
        gains.concentration *= factor;
        gains.decisions *= factor;
        gains.positioning *= factor;
        gains.teamwork *= factor;
        gains.vision *= factor;
        gains.work_rate *= factor;
        gains.leadership *= factor;
        gains
    }
}
