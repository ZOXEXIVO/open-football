// Enhanced training system with realistic football training components

use crate::training::result::PlayerTrainingResult;
use crate::{Person, Player, PlayerPositionType, Staff, Team, TeamTrainingResult};
use chrono::{Datelike, NaiveDateTime, Weekday};
use std::collections::HashMap;
// ============== Training Types ==============

#[derive(Debug, Clone, PartialEq)]
pub enum TrainingType {
    // Physical Training
    Endurance,
    Strength,
    Speed,
    Agility,
    Recovery,

    // Technical Training
    BallControl,
    Passing,
    Shooting,
    Crossing,
    SetPieces,

    // Tactical Training
    Positioning,
    TeamShape,
    PressingDrills,
    TransitionPlay,
    SetPiecesDefensive,

    // Mental Training
    Concentration,
    DecisionMaking,
    Leadership,

    // Match Preparation
    MatchPreparation,
    VideoAnalysis,
    OpponentSpecific,

    // Recovery
    RestDay,
    LightRecovery,
    Rehabilitation,
}

#[derive(Debug, Clone)]
pub struct TrainingSession {
    pub session_type: TrainingType,
    pub intensity: TrainingIntensity,
    pub duration_minutes: u16,
    pub focus_positions: Vec<PlayerPositionType>,
    pub participants: Vec<u32>, // Player IDs
}

#[derive(Debug, Clone, PartialEq)]
pub enum TrainingIntensity {
    VeryLight,  // 20-40% max effort - recovery sessions
    Light,      // 40-60% max effort - technical work
    Moderate,   // 60-75% max effort - standard training
    High,       // 75-90% max effort - intense sessions
    VeryHigh,   // 90-100% max effort - match simulation
}

// ============== Weekly Training Schedule ==============

#[derive(Debug, Clone)]
pub struct WeeklyTrainingPlan {
    pub sessions: HashMap<Weekday, Vec<TrainingSession>>,
    pub match_days: Vec<Weekday>,
    pub periodization_phase: PeriodizationPhase,
}

#[derive(Copy, Clone, Debug, PartialEq)]
pub enum PeriodizationPhase {
    PreSeason,      // High volume, building fitness
    EarlySeason,    // Balancing fitness and tactics
    MidSeason,      // Maintenance and tactical focus
    LateSeason,     // Managing fatigue, focus on recovery
    OffSeason,      // Rest and light maintenance
}

impl WeeklyTrainingPlan {
    /// Generate a realistic weekly training plan based on match schedule
    pub fn generate_weekly_plan(
        next_match_day: Option<Weekday>,
        previous_match_day: Option<Weekday>,
        phase: PeriodizationPhase,
        coach_philosophy: &CoachingPhilosophy,
    ) -> Self {
        let mut sessions = HashMap::new();
        let match_days = vec![next_match_day, previous_match_day]
            .into_iter()
            .flatten()
            .collect();

        // Monday - Recovery or moderate training
        sessions.insert(Weekday::Mon, Self::monday_sessions(previous_match_day, phase));

        // Tuesday - Main training day
        sessions.insert(Weekday::Tue, Self::tuesday_sessions(phase, coach_philosophy));

        // Wednesday - Tactical/Technical focus
        sessions.insert(Weekday::Wed, Self::wednesday_sessions(phase));

        // Thursday - High intensity or recovery based on fixture
        sessions.insert(Weekday::Thu, Self::thursday_sessions(next_match_day, phase));

        // Friday - Match preparation or main training
        sessions.insert(Weekday::Fri, Self::friday_sessions(next_match_day, phase));

        // Saturday - Match day or training
        sessions.insert(Weekday::Sat, Self::saturday_sessions(next_match_day));

        // Sunday - Match day or rest
        sessions.insert(Weekday::Sun, Self::sunday_sessions(next_match_day));

        WeeklyTrainingPlan {
            sessions,
            match_days,
            periodization_phase: phase,
        }
    }

    fn monday_sessions(previous_match: Option<Weekday>, phase: PeriodizationPhase) -> Vec<TrainingSession> {
        if previous_match == Some(Weekday::Sun) || previous_match == Some(Weekday::Sat) {
            // Recovery after weekend match
            vec![
                TrainingSession {
                    session_type: TrainingType::Recovery,
                    intensity: TrainingIntensity::VeryLight,
                    duration_minutes: 45,
                    focus_positions: vec![],
                    participants: vec![],
                },
                TrainingSession {
                    session_type: TrainingType::VideoAnalysis,
                    intensity: TrainingIntensity::VeryLight,
                    duration_minutes: 30,
                    focus_positions: vec![],
                    participants: vec![],
                }
            ]
        } else {
            // Normal training
            vec![
                TrainingSession {
                    session_type: TrainingType::Endurance,
                    intensity: TrainingIntensity::Moderate,
                    duration_minutes: 60,
                    focus_positions: vec![],
                    participants: vec![],
                },
                TrainingSession {
                    session_type: TrainingType::Passing,
                    intensity: TrainingIntensity::Moderate,
                    duration_minutes: 45,
                    focus_positions: vec![],
                    participants: vec![],
                }
            ]
        }
    }

    fn tuesday_sessions(phase: PeriodizationPhase, philosophy: &CoachingPhilosophy) -> Vec<TrainingSession> {
        let base_intensity = match phase {
            PeriodizationPhase::PreSeason => TrainingIntensity::High,
            PeriodizationPhase::MidSeason => TrainingIntensity::Moderate,
            PeriodizationPhase::LateSeason => TrainingIntensity::Light,
            _ => TrainingIntensity::Moderate,
        };

        vec![
            TrainingSession {
                session_type: match philosophy.tactical_focus {
                    TacticalFocus::Possession => TrainingType::Passing,
                    TacticalFocus::Pressing => TrainingType::PressingDrills,
                    TacticalFocus::Counter => TrainingType::TransitionPlay,
                    _ => TrainingType::TeamShape,
                },
                intensity: base_intensity.clone(),
                duration_minutes: 90,
                focus_positions: vec![],
                participants: vec![],
            },
            TrainingSession {
                session_type: TrainingType::SetPieces,
                intensity: TrainingIntensity::Light,
                duration_minutes: 30,
                focus_positions: vec![],
                participants: vec![],
            }
        ]
    }

    fn wednesday_sessions(phase: PeriodizationPhase) -> Vec<TrainingSession> {
        vec![
            TrainingSession {
                session_type: TrainingType::Positioning,
                intensity: TrainingIntensity::Moderate,
                duration_minutes: 75,
                focus_positions: vec![],
                participants: vec![],
            },
            TrainingSession {
                session_type: TrainingType::Shooting,
                intensity: TrainingIntensity::Moderate,
                duration_minutes: 45,
                focus_positions: vec![PlayerPositionType::Striker, PlayerPositionType::ForwardCenter],
                participants: vec![],
            }
        ]
    }

    fn thursday_sessions(next_match: Option<Weekday>, phase: PeriodizationPhase) -> Vec<TrainingSession> {
        if next_match == Some(Weekday::Sat) {
            // Light preparation for Saturday match
            vec![
                TrainingSession {
                    session_type: TrainingType::MatchPreparation,
                    intensity: TrainingIntensity::Light,
                    duration_minutes: 60,
                    focus_positions: vec![],
                    participants: vec![],
                }
            ]
        } else {
            // Full training session
            vec![
                TrainingSession {
                    session_type: TrainingType::Speed,
                    intensity: TrainingIntensity::High,
                    duration_minutes: 45,
                    focus_positions: vec![],
                    participants: vec![],
                },
                TrainingSession {
                    session_type: TrainingType::TeamShape,
                    intensity: TrainingIntensity::Moderate,
                    duration_minutes: 60,
                    focus_positions: vec![],
                    participants: vec![],
                }
            ]
        }
    }

    fn friday_sessions(next_match: Option<Weekday>, phase: PeriodizationPhase) -> Vec<TrainingSession> {
        if next_match == Some(Weekday::Sat) {
            // Final preparation
            vec![
                TrainingSession {
                    session_type: TrainingType::SetPieces,
                    intensity: TrainingIntensity::Light,
                    duration_minutes: 30,
                    focus_positions: vec![],
                    participants: vec![],
                },
                TrainingSession {
                    session_type: TrainingType::OpponentSpecific,
                    intensity: TrainingIntensity::VeryLight,
                    duration_minutes: 45,
                    focus_positions: vec![],
                    participants: vec![],
                }
            ]
        } else {
            // Normal training
            vec![
                TrainingSession {
                    session_type: TrainingType::BallControl,
                    intensity: TrainingIntensity::Moderate,
                    duration_minutes: 60,
                    focus_positions: vec![],
                    participants: vec![],
                },
                TrainingSession {
                    session_type: TrainingType::TransitionPlay,
                    intensity: TrainingIntensity::High,
                    duration_minutes: 45,
                    focus_positions: vec![],
                    participants: vec![],
                }
            ]
        }
    }

    fn saturday_sessions(next_match: Option<Weekday>) -> Vec<TrainingSession> {
        if next_match == Some(Weekday::Sat) {
            vec![] // Match day
        } else {
            vec![
                TrainingSession {
                    session_type: TrainingType::MatchPreparation,
                    intensity: TrainingIntensity::High,
                    duration_minutes: 90,
                    focus_positions: vec![],
                    participants: vec![],
                }
            ]
        }
    }

    fn sunday_sessions(next_match: Option<Weekday>) -> Vec<TrainingSession> {
        if next_match == Some(Weekday::Sun) {
            vec![] // Match day
        } else {
            vec![
                TrainingSession {
                    session_type: TrainingType::RestDay,
                    intensity: TrainingIntensity::VeryLight,
                    duration_minutes: 0,
                    focus_positions: vec![],
                    participants: vec![],
                }
            ]
        }
    }
}

// ============== Training Effects System ==============

#[derive(Debug)]
pub struct TrainingEffects {
    pub physical_gains: PhysicalGains,
    pub technical_gains: TechnicalGains,
    pub mental_gains: MentalGains,
    pub fatigue_change: f32,
    pub injury_risk: f32,
    pub morale_change: f32,
}

#[derive(Debug, Default)]
pub struct PhysicalGains {
    pub stamina: f32,
    pub strength: f32,
    pub pace: f32,
    pub agility: f32,
    pub balance: f32,
    pub jumping: f32,
    pub natural_fitness: f32,
}

#[derive(Debug, Default)]
pub struct TechnicalGains {
    pub first_touch: f32,
    pub passing: f32,
    pub crossing: f32,
    pub dribbling: f32,
    pub finishing: f32,
    pub heading: f32,
    pub tackling: f32,
    pub technique: f32,
}

#[derive(Debug, Default)]
pub struct MentalGains {
    pub concentration: f32,
    pub decisions: f32,
    pub positioning: f32,
    pub teamwork: f32,
    pub vision: f32,
    pub work_rate: f32,
    pub leadership: f32,
}

// ============== Enhanced Team Training ==============

#[derive(Debug)]
pub struct TeamTraining;

impl TeamTraining {
    pub fn train(team: &mut Team, date: NaiveDateTime) -> TeamTrainingResult {
        let mut result = TeamTrainingResult::new();

        // Check if it's training time
        if !team.training_schedule.is_time(date) {
            return result;
        }

        // Get the current training plan
        let current_weekday = date.weekday();
        let coach = team.staffs.training_coach(&team.team_type);

        // Determine periodization phase based on season progress
        let phase = Self::determine_phase(date);

        // Get or generate weekly plan
        let weekly_plan = WeeklyTrainingPlan::generate_weekly_plan(
            Self::get_next_match_day(team, date),
            Self::get_previous_match_day(team, date),
            phase,
            &Self::get_coach_philosophy(coach),
        );

        // Execute today's training sessions
        if let Some(sessions) = weekly_plan.sessions.get(&current_weekday) {
            for session in sessions {
                let session_results = Self::execute_training_session(
                    team,
                    coach,
                    session,
                    date,
                );
                result.player_results.extend(session_results);
            }
        }

        // Apply team cohesion effects
        Self::apply_team_cohesion_effects(team, &result);

        result
    }

    fn execute_training_session(
        team: &Team,
        coach: &Staff,
        session: &TrainingSession,
        date: NaiveDateTime,
    ) -> Vec<PlayerTrainingResult> {
        // Determine participants
        let participants = Self::select_participants(team, session);

        let mut results = Vec::with_capacity(participants.len());

        for player in participants {
            // Calculate training effects based on session type
            let effects = Self::calculate_training_effects(
                player,
                coach,
                session,
                date,
            );

            results.push(PlayerTrainingResult::new(player.id, effects));
        }

        results
    }

    fn calculate_training_effects(
        player: &Player,
        coach: &Staff,
        session: &TrainingSession,
        date: NaiveDateTime,
    ) -> TrainingEffects {
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
        let player_receptiveness = Self::calculate_player_receptiveness(player, coach);
        let age_factor = Self::calculate_age_training_factor(player.age(date.date()));

        // Intensity multipliers
        let intensity_multiplier = match session.intensity {
            TrainingIntensity::VeryLight => 0.3,
            TrainingIntensity::Light => 0.5,
            TrainingIntensity::Moderate => 1.0,
            TrainingIntensity::High => 1.5,
            TrainingIntensity::VeryHigh => 2.0,
        };

        // Calculate gains based on training type
        match session.session_type {
            TrainingType::Endurance => {
                effects.physical_gains.stamina = 0.05 * coach_quality * player_receptiveness * age_factor;
                effects.physical_gains.natural_fitness = 0.03 * coach_quality * player_receptiveness * age_factor;
                effects.fatigue_change = 15.0 * intensity_multiplier;
                effects.injury_risk = 0.02 * intensity_multiplier;
            }
            TrainingType::Strength => {
                effects.physical_gains.strength = 0.04 * coach_quality * player_receptiveness * age_factor;
                effects.physical_gains.jumping = 0.02 * coach_quality * player_receptiveness * age_factor;
                effects.fatigue_change = 20.0 * intensity_multiplier;
                effects.injury_risk = 0.03 * intensity_multiplier;
            }
            TrainingType::Speed => {
                effects.physical_gains.pace = 0.03 * coach_quality * player_receptiveness * age_factor;
                effects.physical_gains.agility = 0.04 * coach_quality * player_receptiveness * age_factor;
                effects.fatigue_change = 25.0 * intensity_multiplier;
                effects.injury_risk = 0.04 * intensity_multiplier;
            }
            TrainingType::BallControl => {
                effects.technical_gains.first_touch = 0.05 * coach_quality * player_receptiveness;
                effects.technical_gains.technique = 0.04 * coach_quality * player_receptiveness;
                effects.technical_gains.dribbling = 0.03 * coach_quality * player_receptiveness;
                effects.fatigue_change = 10.0 * intensity_multiplier;
                effects.injury_risk = 0.01 * intensity_multiplier;
            }
            TrainingType::Passing => {
                effects.technical_gains.passing = 0.06 * coach_quality * player_receptiveness;
                effects.mental_gains.vision = 0.02 * coach_quality * player_receptiveness;
                effects.fatigue_change = 8.0 * intensity_multiplier;
                effects.injury_risk = 0.01 * intensity_multiplier;
            }
            TrainingType::Shooting => {
                effects.technical_gains.finishing = 0.05 * coach_quality * player_receptiveness;
                effects.technical_gains.technique = 0.02 * coach_quality * player_receptiveness;
                effects.mental_gains.decisions = 0.01 * coach_quality * player_receptiveness;
                effects.fatigue_change = 12.0 * intensity_multiplier;
                effects.injury_risk = 0.02 * intensity_multiplier;
            }
            TrainingType::Positioning => {
                effects.mental_gains.positioning = 0.06 * coach_quality * player_receptiveness;
                effects.mental_gains.concentration = 0.03 * coach_quality * player_receptiveness;
                effects.mental_gains.decisions = 0.02 * coach_quality * player_receptiveness;
                effects.fatigue_change = 5.0 * intensity_multiplier;
                effects.injury_risk = 0.005 * intensity_multiplier;
            }
            TrainingType::TeamShape => {
                effects.mental_gains.teamwork = 0.05 * coach_quality * player_receptiveness;
                effects.mental_gains.positioning = 0.04 * coach_quality * player_receptiveness;
                effects.mental_gains.work_rate = 0.02 * coach_quality * player_receptiveness;
                effects.fatigue_change = 10.0 * intensity_multiplier;
                effects.injury_risk = 0.01 * intensity_multiplier;
                effects.morale_change = 0.1; // Team activities boost morale
            }
            TrainingType::Recovery => {
                effects.fatigue_change = -30.0; // Negative means recovery
                effects.injury_risk = -0.02; // Reduces injury risk
                effects.morale_change = 0.05;
            }
            TrainingType::VideoAnalysis => {
                effects.mental_gains.decisions = 0.03 * coach_quality;
                effects.mental_gains.positioning = 0.02 * coach_quality;
                effects.mental_gains.vision = 0.02 * coach_quality;
                effects.fatigue_change = 0.0;
                effects.injury_risk = 0.0;
            }
            _ => {
                // Default minimal gains for unspecified training types
                effects.fatigue_change = 10.0 * intensity_multiplier;
                effects.injury_risk = 0.01 * intensity_multiplier;
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

        effects
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

    fn calculate_player_receptiveness(player: &Player, coach: &Staff) -> f32 {
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
        let age_bonus = match player.age(chrono::Local::now().date_naive()) {
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

    fn select_participants<'a>(team: &'a Team, session: &TrainingSession) -> Vec<&'a Player> {
        let mut participants = Vec::new();

        // If specific participants are listed, use those
        if !session.participants.is_empty() {
            for player_id in &session.participants {
                if let Some(player) = team.players.players.iter().find(|p| p.id == *player_id) {
                    if Self::can_participate(player) {
                        participants.push(player);
                    }
                }
            }
        } else if !session.focus_positions.is_empty() {
            // Select players based on focus positions
            for player in &team.players.players {
                if Self::can_participate(player) {
                    for position in &session.focus_positions {
                        if player.positions.has_position(*position) {
                            participants.push(player);
                            break;
                        }
                    }
                }
            }
        } else {
            // All available players participate
            for player in &team.players.players {
                if Self::can_participate(player) {
                    participants.push(player);
                }
            }
        }

        participants
    }

    fn can_participate(player: &Player) -> bool {
        !player.player_attributes.is_injured &&
            !player.player_attributes.is_banned &&
            player.player_attributes.condition_percentage() > 30
    }

    fn apply_training_effects(player: &mut Player, effects: TrainingEffects) -> PlayerTrainingResult {
        let result = PlayerTrainingResult::new(player.id);

        // Apply physical gains
        player.skills.physical.stamina = (player.skills.physical.stamina + effects.physical_gains.stamina).min(20.0);
        player.skills.physical.strength = (player.skills.physical.strength + effects.physical_gains.strength).min(20.0);
        player.skills.physical.pace = (player.skills.physical.pace + effects.physical_gains.pace).min(20.0);
        player.skills.physical.agility = (player.skills.physical.agility + effects.physical_gains.agility).min(20.0);
        player.skills.physical.balance = (player.skills.physical.balance + effects.physical_gains.balance).min(20.0);
        player.skills.physical.jumping = (player.skills.physical.jumping + effects.physical_gains.jumping).min(20.0);
        player.skills.physical.natural_fitness = (player.skills.physical.natural_fitness + effects.physical_gains.natural_fitness).min(20.0);

        // Apply technical gains
        player.skills.technical.first_touch = (player.skills.technical.first_touch + effects.technical_gains.first_touch).min(20.0);
        player.skills.technical.passing = (player.skills.technical.passing + effects.technical_gains.passing).min(20.0);
        player.skills.technical.crossing = (player.skills.technical.crossing + effects.technical_gains.crossing).min(20.0);
        player.skills.technical.dribbling = (player.skills.technical.dribbling + effects.technical_gains.dribbling).min(20.0);
        player.skills.technical.finishing = (player.skills.technical.finishing + effects.technical_gains.finishing).min(20.0);
        player.skills.technical.heading = (player.skills.technical.heading + effects.technical_gains.heading).min(20.0);
        player.skills.technical.tackling = (player.skills.technical.tackling + effects.technical_gains.tackling).min(20.0);
        player.skills.technical.technique = (player.skills.technical.technique + effects.technical_gains.technique).min(20.0);

        // Apply mental gains
        player.skills.mental.concentration = (player.skills.mental.concentration + effects.mental_gains.concentration).min(20.0);
        player.skills.mental.decisions = (player.skills.mental.decisions + effects.mental_gains.decisions).min(20.0);
        player.skills.mental.positioning = (player.skills.mental.positioning + effects.mental_gains.positioning).min(20.0);
        player.skills.mental.teamwork = (player.skills.mental.teamwork + effects.mental_gains.teamwork).min(20.0);
        player.skills.mental.vision = (player.skills.mental.vision + effects.mental_gains.vision).min(20.0);
        player.skills.mental.work_rate = (player.skills.mental.work_rate + effects.mental_gains.work_rate).min(20.0);
        player.skills.mental.leadership = (player.skills.mental.leadership + effects.mental_gains.leadership).min(20.0);

        // Apply fatigue changes
        let new_condition = player.player_attributes.condition as f32 - effects.fatigue_change;
        player.player_attributes.condition = new_condition.clamp(0.0, 10000.0) as i16;

        // Check for injuries (simplified - you'd want more complex injury system)
        if rand::random::<f32>() < effects.injury_risk {
            // Trigger injury
            player.player_attributes.is_injured = true;
            // You'd want to add injury details, duration, etc.
        }

        // Apply morale changes
        // This would integrate with your happiness system

        result
    }

    fn apply_team_cohesion_effects(team: &mut Team, training_results: &TeamTrainingResult) {
        // Players training together build relationships
        let participant_ids: Vec<u32> = training_results.player_results
            .iter()
            .map(|r| r.player_id)
            .collect();

        // Small relationship improvements between training partners
        for i in 0..participant_ids.len() {
            for j in i + 1..participant_ids.len() {
                if let Some(player_i) = team.players.players.iter_mut().find(|p| p.id == participant_ids[i]) {
                    player_i.relations.update(participant_ids[j], 0.01);
                }
                if let Some(player_j) = team.players.players.iter_mut().find(|p| p.id == participant_ids[j]) {
                    player_j.relations.update(participant_ids[i], 0.01);
                }
            }
        }
    }

    fn determine_phase(date: NaiveDateTime) -> PeriodizationPhase {
        let month = date.month();
        match month {
            6 | 7 => PeriodizationPhase::PreSeason,
            8 | 9 => PeriodizationPhase::EarlySeason,
            10 | 11 | 12 | 1 | 2 => PeriodizationPhase::MidSeason,
            3 | 4 => PeriodizationPhase::LateSeason,
            5 => PeriodizationPhase::OffSeason,
            _ => PeriodizationPhase::MidSeason,
        }
    }

    fn get_next_match_day(team: &Team, date: NaiveDateTime) -> Option<Weekday> {
        // This would check the actual match schedule
        // For now, assume Saturday matches
        Some(Weekday::Sat)
    }

    fn get_previous_match_day(team: &Team, date: NaiveDateTime) -> Option<Weekday> {
        // This would check the actual match history
        // For now, return None
        None
    }

    fn get_coach_philosophy(coach: &Staff) -> CoachingPhilosophy {
        // Determine coach philosophy based on attributes
        let tactical_focus = if coach.staff_attributes.coaching.attacking > coach.staff_attributes.coaching.defending {
            if coach.staff_attributes.coaching.attacking > 15 {
                TacticalFocus::Attacking
            } else {
                TacticalFocus::Possession
            }
        } else if coach.staff_attributes.coaching.defending > 15 {
            TacticalFocus::Defensive
        } else {
            TacticalFocus::Balanced
        };

        let training_intensity = if coach.staff_attributes.coaching.fitness > 15 {
            TrainingIntensityPreference::High
        } else if coach.staff_attributes.coaching.fitness < 10 {
            TrainingIntensityPreference::Low
        } else {
            TrainingIntensityPreference::Medium
        };

        CoachingPhilosophy {
            tactical_focus,
            training_intensity,
            youth_focus: coach.staff_attributes.coaching.working_with_youngsters > 12,
            rotation_preference: RotationPreference::Moderate,
        }
    }
}

// ============== Individual Player Training Plans ==============

#[derive(Debug)]
pub struct IndividualTrainingPlan {
    pub player_id: u32,
    pub focus_areas: Vec<TrainingFocus>,
    pub intensity_modifier: f32, // 0.5 to 1.5
    pub special_instructions: Vec<SpecialInstruction>,
}

#[derive(Debug, Clone)]
pub enum TrainingFocus {
    WeakFootImprovement,
    PositionRetraining(PlayerPositionType),
    SpecificSkill(SkillType),
    InjuryRecovery,
    FitnessBuilding,
    MentalDevelopment,
}

#[derive(Debug, Clone)]
pub enum SkillType {
    FreeKicks,
    Penalties,
    LongShots,
    Heading,
    Tackling,
    Crossing,
    Dribbling,
}

#[derive(Debug, Clone)]
pub enum SpecialInstruction {
    ExtraGymWork,
    DietProgram,
    MentalCoaching,
    MediaTraining,
    LeadershipDevelopment,
    RestMoreOften,
}

// ============== Coaching Philosophy ==============

#[derive(Debug, Clone)]
pub struct CoachingPhilosophy {
    pub tactical_focus: TacticalFocus,
    pub training_intensity: TrainingIntensityPreference,
    pub youth_focus: bool,
    pub rotation_preference: RotationPreference,
}

#[derive(Debug, Clone, PartialEq)]
pub enum TacticalFocus {
    Possession,
    Counter,
    Pressing,
    Attacking,
    Defensive,
    Balanced,
}

#[derive(Debug, Clone, PartialEq)]
pub enum TrainingIntensityPreference {
    Low,    // Focus on technical, less physical
    Medium, // Balanced approach
    High,   // Heavy physical training
}

#[derive(Debug, Clone, PartialEq)]
pub enum RotationPreference {
    Minimal,  // Same XI mostly
    Moderate, // Some rotation
    Heavy,    // Lots of rotation
}

// ============== Training Ground Facilities ==============

#[derive(Debug)]
pub struct TrainingFacilities {
    pub quality: FacilityQuality,
    pub gym_quality: FacilityQuality,
    pub medical_facilities: FacilityQuality,
    pub recovery_facilities: FacilityQuality,
    pub pitches_count: u8,
    pub has_swimming_pool: bool,
    pub has_sports_science: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub enum FacilityQuality {
    Poor,
    Basic,
    Good,
    Excellent,
    WorldClass,
}

impl TrainingFacilities {
    pub fn get_training_modifier(&self) -> f32 {
        let base = match self.quality {
            FacilityQuality::Poor => 0.7,
            FacilityQuality::Basic => 0.85,
            FacilityQuality::Good => 1.0,
            FacilityQuality::Excellent => 1.15,
            FacilityQuality::WorldClass => 1.3,
        };

        let gym_bonus = match self.gym_quality {
            FacilityQuality::Poor => -0.05,
            FacilityQuality::Basic => 0.0,
            FacilityQuality::Good => 0.05,
            FacilityQuality::Excellent => 0.1,
            FacilityQuality::WorldClass => 0.15,
        };

        base + gym_bonus
    }

    pub fn get_injury_risk_modifier(&self) -> f32 {
        let base = match self.quality {
            FacilityQuality::Poor => 1.3,
            FacilityQuality::Basic => 1.15,
            FacilityQuality::Good => 1.0,
            FacilityQuality::Excellent => 0.9,
            FacilityQuality::WorldClass => 0.8,
        };

        let medical_modifier = match self.medical_facilities {
            FacilityQuality::Poor => 1.2,
            FacilityQuality::Basic => 1.1,
            FacilityQuality::Good => 1.0,
            FacilityQuality::Excellent => 0.9,
            FacilityQuality::WorldClass => 0.8,
        };

        base * medical_modifier
    }

    pub fn get_recovery_modifier(&self) -> f32 {
        let base = match self.recovery_facilities {
            FacilityQuality::Poor => 0.7,
            FacilityQuality::Basic => 0.85,
            FacilityQuality::Good => 1.0,
            FacilityQuality::Excellent => 1.2,
            FacilityQuality::WorldClass => 1.4,
        };

        let pool_bonus = if self.has_swimming_pool { 0.1 } else { 0.0 };
        let sports_science_bonus = if self.has_sports_science { 0.15 } else { 0.0 };

        base + pool_bonus + sports_science_bonus
    }
}

// ============== Training Load Management ==============

#[derive(Debug)]
pub struct TrainingLoadManager {
    pub player_loads: HashMap<u32, PlayerTrainingLoad>,
}

#[derive(Debug)]
pub struct PlayerTrainingLoad {
    pub acute_load: f32,        // Last 7 days
    pub chronic_load: f32,      // Last 28 days
    pub load_ratio: f32,        // Acute/Chronic ratio
    pub cumulative_fatigue: f32,
    pub last_high_intensity: Option<NaiveDateTime>,
    pub sessions_this_week: u8,
}

impl PlayerTrainingLoad {
    pub fn new() -> Self {
        PlayerTrainingLoad {
            acute_load: 0.0,
            chronic_load: 0.0,
            load_ratio: 1.0,
            cumulative_fatigue: 0.0,
            last_high_intensity: None,
            sessions_this_week: 0,
        }
    }

    pub fn update_load(&mut self, session_load: f32, intensity: &TrainingIntensity, date: NaiveDateTime) {
        // Update acute load (exponentially weighted)
        self.acute_load = self.acute_load * 0.9 + session_load * 0.1;

        // Update chronic load (slower adaptation)
        self.chronic_load = self.chronic_load * 0.97 + session_load * 0.03;

        // Calculate load ratio
        self.load_ratio = if self.chronic_load > 0.0 {
            self.acute_load / self.chronic_load
        } else {
            1.0
        };

        // Update fatigue
        self.cumulative_fatigue = (self.cumulative_fatigue + session_load * 0.2).min(100.0);

        // Track high intensity sessions
        if matches!(intensity, TrainingIntensity::High | TrainingIntensity::VeryHigh) {
            self.last_high_intensity = Some(date);
        }

        self.sessions_this_week += 1;
    }

    pub fn get_injury_risk_factor(&self) -> f32 {
        // High acute:chronic ratios increase injury risk
        if self.load_ratio > 1.5 {
            1.5
        } else if self.load_ratio > 1.3 {
            1.2
        } else if self.load_ratio < 0.8 {
            1.1 // Too little load can also increase injury risk
        } else {
            1.0
        }
    }

    pub fn needs_rest(&self) -> bool {
        self.cumulative_fatigue > 75.0 ||
            self.load_ratio > 1.5 ||
            self.sessions_this_week >= 6
    }

    pub fn weekly_reset(&mut self) {
        self.sessions_this_week = 0;
        self.cumulative_fatigue *= 0.7; // Partial recovery
    }
}