use crate::training::result::PlayerTrainingResult;
use crate::{MentalGains, Person, PhysicalGains, Player, Staff, TechnicalGains, TrainingEffects};
use chrono::NaiveDateTime;

#[derive(Debug)]
pub struct PlayerTraining {
   
}

impl Default for PlayerTraining {
    fn default() -> Self {
        Self::new()
    }
}

impl PlayerTraining {
    pub fn new() -> Self {
        PlayerTraining {}
    }

    pub fn train(player: &Player, coach: &Staff, now: NaiveDateTime) -> PlayerTrainingResult {
        let now_date = now.date();

        // Calculate training effects based on player's current state
        let effects = Self::calculate_individual_training_effects(
            player,
            coach,
            now_date
        );

        // Return the result with effects that will be applied later
        PlayerTrainingResult::new(player.id, effects)
    }

    /// Calculate individual training effects based on player attributes
    fn calculate_individual_training_effects(
        player: &Player,
        coach: &Staff,
        now_date: chrono::NaiveDate,
    ) -> TrainingEffects {
        let mut effects = TrainingEffects {
            physical_gains: PhysicalGains::default(),
            technical_gains: TechnicalGains::default(),
            mental_gains: MentalGains::default(),
            fatigue_change: 0.0,
            injury_risk: 0.0,
            morale_change: 0.0,
        };

        // Calculate base effectiveness factors
        let coach_quality = Self::calculate_coach_effectiveness(coach);
        let player_receptiveness = Self::calculate_player_receptiveness(player, coach);
        let age_factor = Self::calculate_age_training_factor(player.age(now_date));

        // Check player's training history to determine focus
        let weeks_since_training = player.training_history.weeks_since_last_training(now_date);

        // If player hasn't trained recently, apply recovery effects
        if weeks_since_training > 2 {
            effects.fatigue_change = -20.0; // Recovery
            effects.injury_risk = -0.01;
            return effects;
        }

        // Apply position-specific training
        Self::apply_position_specific_training(
            &mut effects,
            player,
            coach_quality,
            player_receptiveness,
            age_factor
        );

        // Apply individual focus areas if any
        if let Some(ref focus) = coach.focus {
            Self::apply_coach_focus_training(
                &mut effects,
                &focus,
                coach_quality,
                player_receptiveness
            );
        }

        // Apply player condition modifiers
        Self::apply_condition_modifiers(&mut effects, player);

        // Apply professionalism bonus
        let professionalism_bonus = player.attributes.professionalism / 20.0;
        Self::apply_professionalism_bonus(&mut effects, professionalism_bonus);

        effects
    }

    fn apply_position_specific_training(
        effects: &mut TrainingEffects,
        player: &Player,
        coach_quality: f32,
        player_receptiveness: f32,
        age_factor: f32,
    ) {
        let position = player.position();

        match position {
            pos if pos.is_goalkeeper() => {
                // Goalkeeper specific training
                effects.physical_gains.agility = 0.04 * coach_quality * player_receptiveness * age_factor;
                effects.physical_gains.jumping = 0.03 * coach_quality * player_receptiveness * age_factor;
                effects.technical_gains.first_touch = 0.02 * coach_quality * player_receptiveness;
                effects.mental_gains.concentration = 0.04 * coach_quality * player_receptiveness;
                effects.mental_gains.positioning = 0.05 * coach_quality * player_receptiveness;
            }
            pos if pos.is_defender() => {
                // Defender specific training
                effects.physical_gains.strength = 0.04 * coach_quality * player_receptiveness * age_factor;
                effects.physical_gains.jumping = 0.03 * coach_quality * player_receptiveness * age_factor;
                effects.technical_gains.tackling = 0.05 * coach_quality * player_receptiveness;
                effects.technical_gains.heading = 0.04 * coach_quality * player_receptiveness;
                effects.mental_gains.positioning = 0.05 * coach_quality * player_receptiveness;
                effects.mental_gains.concentration = 0.03 * coach_quality * player_receptiveness;
            }
            pos if pos.is_midfielder() => {
                // Midfielder specific training
                effects.physical_gains.stamina = 0.04 * coach_quality * player_receptiveness * age_factor;
                effects.technical_gains.passing = 0.05 * coach_quality * player_receptiveness;
                effects.technical_gains.first_touch = 0.04 * coach_quality * player_receptiveness;
                effects.mental_gains.vision = 0.04 * coach_quality * player_receptiveness;
                effects.mental_gains.decisions = 0.03 * coach_quality * player_receptiveness;
            }
            pos if pos.is_forward() => {
                // Forward specific training
                effects.physical_gains.pace = 0.03 * coach_quality * player_receptiveness * age_factor;
                effects.technical_gains.finishing = 0.05 * coach_quality * player_receptiveness;
                effects.technical_gains.dribbling = 0.04 * coach_quality * player_receptiveness;
                effects.mental_gains.positioning = 0.04 * coach_quality * player_receptiveness;
                effects.mental_gains.decisions = 0.03 * coach_quality * player_receptiveness;
            }
            _ => {
                // General training for undefined positions
                effects.physical_gains.stamina = 0.02 * coach_quality * player_receptiveness * age_factor;
                effects.technical_gains.technique = 0.03 * coach_quality * player_receptiveness;
                effects.mental_gains.teamwork = 0.03 * coach_quality * player_receptiveness;
            }
        }

        // Base fatigue and injury risk for position training
        effects.fatigue_change = 15.0;
        effects.injury_risk = 0.02;
    }

    fn apply_coach_focus_training(
        effects: &mut TrainingEffects,
        focus: &crate::CoachFocus,
        coach_quality: f32,
        player_receptiveness: f32,
    ) {
        use crate::{TechnicalFocusType, MentalFocusType, PhysicalFocusType};

        // Apply technical focus
        for tech_focus in &focus.technical_focus {
            match tech_focus {
                TechnicalFocusType::Passing => effects.technical_gains.passing += 0.02 * coach_quality * player_receptiveness,
                TechnicalFocusType::Dribbling => effects.technical_gains.dribbling += 0.02 * coach_quality * player_receptiveness,
                TechnicalFocusType::Finishing => effects.technical_gains.finishing += 0.02 * coach_quality * player_receptiveness,
                TechnicalFocusType::Crossing => effects.technical_gains.crossing += 0.02 * coach_quality * player_receptiveness,
                TechnicalFocusType::Heading => effects.technical_gains.heading += 0.02 * coach_quality * player_receptiveness,
                TechnicalFocusType::Tackling => effects.technical_gains.tackling += 0.02 * coach_quality * player_receptiveness,
                TechnicalFocusType::Technique => effects.technical_gains.technique += 0.02 * coach_quality * player_receptiveness,
                TechnicalFocusType::FirstTouch => effects.technical_gains.first_touch += 0.02 * coach_quality * player_receptiveness,
                TechnicalFocusType::FreeKicks => effects.technical_gains.technique += 0.01 * coach_quality * player_receptiveness,
                TechnicalFocusType::LongShots => effects.technical_gains.technique += 0.01 * coach_quality * player_receptiveness,
                TechnicalFocusType::LongThrows => effects.technical_gains.technique += 0.01 * coach_quality * player_receptiveness,
                TechnicalFocusType::Marking => effects.technical_gains.tackling += 0.01 * coach_quality * player_receptiveness,
                TechnicalFocusType::PenaltyTaking => effects.technical_gains.finishing += 0.01 * coach_quality * player_receptiveness,
                TechnicalFocusType::Corners => effects.technical_gains.crossing += 0.01 * coach_quality * player_receptiveness,
            }
        }

        // Apply mental focus
        for mental_focus in &focus.mental_focus {
            match mental_focus {
                MentalFocusType::Concentration => effects.mental_gains.concentration += 0.02 * coach_quality * player_receptiveness,
                MentalFocusType::Decisions => effects.mental_gains.decisions += 0.02 * coach_quality * player_receptiveness,
                MentalFocusType::Positioning => effects.mental_gains.positioning += 0.02 * coach_quality * player_receptiveness,
                MentalFocusType::Teamwork => effects.mental_gains.teamwork += 0.02 * coach_quality * player_receptiveness,
                MentalFocusType::Vision => effects.mental_gains.vision += 0.02 * coach_quality * player_receptiveness,
                MentalFocusType::Leadership => effects.mental_gains.leadership += 0.02 * coach_quality * player_receptiveness,
                MentalFocusType::WorkRate => effects.mental_gains.work_rate += 0.02 * coach_quality * player_receptiveness,
                MentalFocusType::OffTheBall => effects.mental_gains.positioning += 0.01 * coach_quality * player_receptiveness,
                _ => {} // Handle other mental focus types as needed
            }
        }

        // Apply physical focus
        for physical_focus in &focus.physical_focus {
            match physical_focus {
                PhysicalFocusType::Stamina => effects.physical_gains.stamina += 0.02 * coach_quality * player_receptiveness,
                PhysicalFocusType::Strength => effects.physical_gains.strength += 0.02 * coach_quality * player_receptiveness,
                PhysicalFocusType::Pace => effects.physical_gains.pace += 0.02 * coach_quality * player_receptiveness,
                PhysicalFocusType::Agility => effects.physical_gains.agility += 0.02 * coach_quality * player_receptiveness,
                PhysicalFocusType::Balance => effects.physical_gains.balance += 0.02 * coach_quality * player_receptiveness,
                PhysicalFocusType::Jumping => effects.physical_gains.jumping += 0.02 * coach_quality * player_receptiveness,
                PhysicalFocusType::NaturalFitness => effects.physical_gains.natural_fitness += 0.02 * coach_quality * player_receptiveness,
                _ => {} // Handle other physical focus types as needed
            }
        }
    }

    fn apply_condition_modifiers(effects: &mut TrainingEffects, player: &Player) {
        let condition_factor = player.player_attributes.condition_percentage() as f32 / 100.0;

        if condition_factor < 0.7 {
            // Tired players have higher injury risk and get more fatigued
            effects.injury_risk *= 1.5;
            effects.fatigue_change *= 1.2;

            // Reduced training effectiveness when tired
            effects.physical_gains.stamina *= 0.8;
            effects.physical_gains.strength *= 0.8;
            effects.physical_gains.pace *= 0.8;
        } else if condition_factor > 0.9 {
            // Well-rested players train more effectively
            effects.injury_risk *= 0.8;
            effects.morale_change += 0.05;
        }
    }

    fn apply_professionalism_bonus(effects: &mut TrainingEffects, bonus: f32) {
        // Apply bonus to all gains
        effects.physical_gains.stamina *= 1.0 + bonus;
        effects.physical_gains.strength *= 1.0 + bonus;
        effects.physical_gains.pace *= 1.0 + bonus;
        effects.physical_gains.agility *= 1.0 + bonus;
        effects.physical_gains.balance *= 1.0 + bonus;
        effects.physical_gains.jumping *= 1.0 + bonus;
        effects.physical_gains.natural_fitness *= 1.0 + bonus;

        effects.technical_gains.first_touch *= 1.0 + bonus;
        effects.technical_gains.passing *= 1.0 + bonus;
        effects.technical_gains.crossing *= 1.0 + bonus;
        effects.technical_gains.dribbling *= 1.0 + bonus;
        effects.technical_gains.finishing *= 1.0 + bonus;
        effects.technical_gains.heading *= 1.0 + bonus;
        effects.technical_gains.tackling *= 1.0 + bonus;
        effects.technical_gains.technique *= 1.0 + bonus;

        effects.mental_gains.concentration *= 1.0 + bonus;
        effects.mental_gains.decisions *= 1.0 + bonus;
        effects.mental_gains.positioning *= 1.0 + bonus;
        effects.mental_gains.teamwork *= 1.0 + bonus;
        effects.mental_gains.vision *= 1.0 + bonus;
        effects.mental_gains.work_rate *= 1.0 + bonus;
        effects.mental_gains.leadership *= 1.0 + bonus;
    }

    fn calculate_coach_effectiveness(coach: &Staff) -> f32 {
        // Average of relevant coaching attributes
        let coaching = &coach.staff_attributes.coaching;
        let avg = (coaching.attacking + coaching.defending +
            coaching.tactical + coaching.technical +
            coaching.mental + coaching.fitness) as f32 / 120.0;

        // Add determination factor
        let determination = coach.staff_attributes.mental.determination as f32 / 20.0;

        (avg * 0.7 + determination * 0.3).min(1.0)
    }

    fn calculate_player_receptiveness(player: &Player, coach: &Staff) -> f32 {
        use crate::Person;

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

        // Age affects receptiveness
        let age = player.age(chrono::Local::now().date_naive());
        let age_bonus = match age {
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
}
