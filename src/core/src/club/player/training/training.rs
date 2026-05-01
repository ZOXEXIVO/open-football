use crate::club::player::training::result::PlayerTrainingResult;
use crate::{
    MentalGains, Person, PhysicalGains, Player, Staff, TechnicalGains, TrainingEffects,
    TrainingIntensity, TrainingSession, TrainingType,
};
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
        facility_quality: f32,
    ) -> PlayerTrainingResult {
        let mut effects = TrainingEffects {
            physical_gains: PhysicalGains::default(),
            technical_gains: TechnicalGains::default(),
            mental_gains: MentalGains::default(),
            fatigue_change: 0.0,
            injury_risk: 0.0,
            morale_change: 0.0,
            physical_load_units: 0.0,
            high_intensity_share: 0.0,
            readiness_change: 0.0,
        };

        // Base effectiveness factors
        let base_coach_quality = Self::calculate_coach_effectiveness(coach, &session.session_type);
        // Coach specialization: a coach who has spent hundreds of sessions
        // with this player's position group extracts better gains than a
        // generalist. Scales from 1.0 to 1.40.
        let player_group = player.position().position_group();
        let specialization_bonus = coach.specialization_bonus(player_group);
        let coach_quality = base_coach_quality * specialization_bonus;
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

        // Calculate gains based on training type. Each arm now also fills
        // in the load profile (fatigue cost, physical-load units booked
        // into PlayerLoad, HI-share, and per-session readiness gain).
        // The previous model treated almost every non-physical session as
        // "net recovery" which made the squad permanently fresh on a
        // light-tactical week — and any negative fatigue blanket-gifted
        // +2 readiness, equating passive video with hard match-prep.
        //
        // New shape:
        //   * Endurance / Strength / Speed → cost condition, build fitness
        //   * Pressing / Transition / MatchPrep → high readiness gain
        //   * Recovery / RestDay / Video → restore condition, ~zero sharpness
        //   * Rehab / LightRecovery → small readiness rebuild
        match session.session_type {
            TrainingType::Endurance => {
                effects.physical_gains.stamina =
                    0.05 * coach_quality * player_receptiveness * age_factor;
                effects.physical_gains.natural_fitness =
                    0.03 * coach_quality * player_receptiveness * age_factor;
                effects.fatigue_change = 100.0 * intensity_multiplier; // Real cost — endurance is hard
                effects.injury_risk = 0.002 * intensity_multiplier;
                effects.physical_load_units = 30.0 * intensity_multiplier;
                effects.high_intensity_share = 0.20;
                effects.readiness_change = 0.4;
            }
            TrainingType::Strength => {
                effects.physical_gains.strength =
                    0.04 * coach_quality * player_receptiveness * age_factor;
                effects.physical_gains.jumping =
                    0.02 * coach_quality * player_receptiveness * age_factor;
                effects.fatigue_change = 100.0 * intensity_multiplier;
                effects.injury_risk = 0.003 * intensity_multiplier;
                effects.physical_load_units = 22.0 * intensity_multiplier;
                effects.high_intensity_share = 0.15;
                effects.readiness_change = 0.3;
            }
            TrainingType::Speed => {
                effects.physical_gains.pace =
                    0.03 * coach_quality * player_receptiveness * age_factor;
                effects.physical_gains.agility =
                    0.04 * coach_quality * player_receptiveness * age_factor;
                effects.fatigue_change = 150.0 * intensity_multiplier;
                effects.injury_risk = 0.004 * intensity_multiplier;
                effects.physical_load_units = 32.0 * intensity_multiplier;
                effects.high_intensity_share = 0.55;
                effects.readiness_change = 0.5;
            }
            TrainingType::BallControl => {
                effects.technical_gains.first_touch = 0.05 * coach_quality * player_receptiveness;
                effects.technical_gains.technique = 0.04 * coach_quality * player_receptiveness;
                effects.technical_gains.dribbling = 0.03 * coach_quality * player_receptiveness;
                effects.fatigue_change = 15.0 * intensity_multiplier;
                effects.injury_risk = 0.001 * intensity_multiplier;
                effects.physical_load_units = 12.0 * intensity_multiplier;
                effects.high_intensity_share = 0.10;
                effects.readiness_change = 0.4;
            }
            TrainingType::Passing => {
                effects.technical_gains.passing = 0.06 * coach_quality * player_receptiveness;
                effects.mental_gains.vision = 0.02 * coach_quality * player_receptiveness;
                effects.fatigue_change = 10.0 * intensity_multiplier;
                effects.injury_risk = 0.001 * intensity_multiplier;
                effects.physical_load_units = 10.0 * intensity_multiplier;
                effects.high_intensity_share = 0.05;
                effects.readiness_change = 0.4;
            }
            TrainingType::Shooting => {
                effects.technical_gains.finishing = 0.05 * coach_quality * player_receptiveness;
                effects.technical_gains.technique = 0.02 * coach_quality * player_receptiveness;
                effects.mental_gains.decisions = 0.01 * coach_quality * player_receptiveness;
                effects.fatigue_change = 50.0 * intensity_multiplier;
                effects.injury_risk = 0.002 * intensity_multiplier;
                effects.physical_load_units = 16.0 * intensity_multiplier;
                effects.high_intensity_share = 0.20;
                effects.readiness_change = 0.5;
            }
            TrainingType::Positioning => {
                effects.mental_gains.positioning = 0.06 * coach_quality * player_receptiveness;
                effects.mental_gains.concentration = 0.03 * coach_quality * player_receptiveness;
                effects.mental_gains.decisions = 0.02 * coach_quality * player_receptiveness;
                // Tactical walkthrough — slight net recovery, real
                // sharpness gain (you're rehearsing match shape).
                effects.fatigue_change = -30.0 * intensity_multiplier;
                effects.injury_risk = 0.0005 * intensity_multiplier;
                effects.physical_load_units = 4.0 * intensity_multiplier;
                effects.high_intensity_share = 0.0;
                effects.readiness_change = 0.4;
            }
            TrainingType::TeamShape => {
                effects.mental_gains.teamwork = 0.05 * coach_quality * player_receptiveness;
                effects.mental_gains.positioning = 0.04 * coach_quality * player_receptiveness;
                effects.mental_gains.work_rate = 0.02 * coach_quality * player_receptiveness;
                effects.fatigue_change = 30.0 * intensity_multiplier;
                effects.injury_risk = 0.001 * intensity_multiplier;
                effects.morale_change = 0.1; // Team activities boost morale
                effects.physical_load_units = 14.0 * intensity_multiplier;
                effects.high_intensity_share = 0.10;
                effects.readiness_change = 0.6;
            }
            TrainingType::Recovery => {
                effects.fatigue_change = -800.0; // Strong recovery — main condition restoration
                effects.injury_risk = -0.002;
                effects.morale_change = 0.05;
                effects.physical_load_units = 0.0;
                effects.high_intensity_share = 0.0;
                effects.readiness_change = 0.1;
            }
            TrainingType::VideoAnalysis => {
                effects.mental_gains.decisions = 0.03 * coach_quality;
                effects.mental_gains.positioning = 0.02 * coach_quality;
                effects.mental_gains.vision = 0.02 * coach_quality;
                effects.fatigue_change = -120.0; // Sat in a meeting room — body recovers a touch
                effects.injury_risk = 0.0;
                effects.physical_load_units = 0.0;
                effects.high_intensity_share = 0.0;
                effects.readiness_change = 0.1; // Tactical sharpness, not match sharpness
            }
            TrainingType::RestDay => {
                effects.fatigue_change = -600.0;
                effects.injury_risk = -0.003;
                effects.morale_change = 0.03;
                effects.physical_load_units = 0.0;
                effects.high_intensity_share = 0.0;
                effects.readiness_change = 0.0;
            }
            TrainingType::LightRecovery => {
                effects.fatigue_change = -500.0;
                effects.injury_risk = -0.001;
                effects.morale_change = 0.02;
                effects.physical_load_units = 2.0;
                effects.high_intensity_share = 0.0;
                effects.readiness_change = 0.2;
            }
            TrainingType::Rehabilitation => {
                effects.fatigue_change = -300.0;
                effects.injury_risk = -0.002;
                effects.physical_load_units = 4.0;
                effects.high_intensity_share = 0.0;
                effects.readiness_change = 0.3;
            }
            TrainingType::PressingDrills => {
                effects.mental_gains.work_rate = 0.04 * coach_quality * player_receptiveness;
                effects.mental_gains.teamwork = 0.03 * coach_quality * player_receptiveness;
                effects.fatigue_change = 180.0 * intensity_multiplier;
                effects.injury_risk = 0.004 * intensity_multiplier;
                effects.physical_load_units = 38.0 * intensity_multiplier;
                effects.high_intensity_share = 0.50;
                effects.readiness_change = 1.0;
            }
            TrainingType::TransitionPlay => {
                effects.technical_gains.passing = 0.03 * coach_quality * player_receptiveness;
                effects.mental_gains.decisions = 0.03 * coach_quality * player_receptiveness;
                effects.fatigue_change = 160.0 * intensity_multiplier;
                effects.injury_risk = 0.003 * intensity_multiplier;
                effects.physical_load_units = 34.0 * intensity_multiplier;
                effects.high_intensity_share = 0.45;
                effects.readiness_change = 1.0;
            }
            TrainingType::MatchPreparation => {
                effects.mental_gains.concentration = 0.03 * coach_quality * player_receptiveness;
                effects.mental_gains.positioning = 0.02 * coach_quality * player_receptiveness;
                effects.fatigue_change = 30.0 * intensity_multiplier;
                effects.injury_risk = 0.0015 * intensity_multiplier;
                effects.physical_load_units = 18.0 * intensity_multiplier;
                effects.high_intensity_share = 0.20;
                effects.readiness_change = 1.2; // Top-up sharpness without burning condition
            }
            TrainingType::SetPieces => {
                effects.technical_gains.crossing = 0.03 * coach_quality * player_receptiveness;
                effects.technical_gains.heading = 0.02 * coach_quality * player_receptiveness;
                effects.fatigue_change = 20.0 * intensity_multiplier;
                effects.injury_risk = 0.0008 * intensity_multiplier;
                effects.physical_load_units = 8.0 * intensity_multiplier;
                effects.high_intensity_share = 0.10;
                effects.readiness_change = 0.4;
            }
            TrainingType::OpponentSpecific => {
                effects.mental_gains.decisions = 0.02 * coach_quality;
                effects.mental_gains.vision = 0.02 * coach_quality;
                effects.fatigue_change = 20.0 * intensity_multiplier;
                effects.injury_risk = 0.001 * intensity_multiplier;
                effects.physical_load_units = 10.0 * intensity_multiplier;
                effects.high_intensity_share = 0.10;
                effects.readiness_change = 0.6;
            }
            _ => {
                // Default minimal load for unspecified training types
                effects.fatigue_change = 10.0 * intensity_multiplier;
                effects.injury_risk = 0.001 * intensity_multiplier;
                effects.physical_load_units = 8.0 * intensity_multiplier;
                effects.high_intensity_share = 0.10;
                effects.readiness_change = 0.2;
            }
        }

        // ===== FACILITY QUALITY EFFECTS =====
        // Training facilities directly multiply all skill gains.
        // Poor (0.05) → 0.55x gains, Average (0.35) → 0.85x, Good (0.55) → 1.0x,
        // Excellent (0.75) → 1.15x, Best (1.0) → 1.30x
        let facility_modifier = 0.50 + facility_quality * 0.80; // Range: 0.54 to 1.30

        // Better facilities reduce injury risk (better medical, better surfaces)
        // Poor → 1.4x injury, Average → 1.1x, Good → 1.0x, Excellent → 0.85x, Best → 0.70x
        let facility_injury_mod = 1.4 - facility_quality * 0.70; // Range: 1.4 to 0.70

        // Better facilities improve recovery (better recovery rooms, pools, medical staff)
        // Only applies to recovery sessions (negative fatigue_change)
        let facility_recovery_mod = 0.70 + facility_quality * 0.60; // Range: 0.73 to 1.30

        // Apply facility modifier to all skill gains
        effects.physical_gains = Self::scale_physical(effects.physical_gains, facility_modifier);
        effects.technical_gains = Self::scale_technical(effects.technical_gains, facility_modifier);
        effects.mental_gains = Self::scale_mental(effects.mental_gains, facility_modifier);

        // Apply facility modifier to injury risk
        effects.injury_risk *= facility_injury_mod;

        // Apply facility modifier to recovery (only when recovering, not when fatiguing)
        if effects.fatigue_change < 0.0 {
            effects.fatigue_change *= facility_recovery_mod; // More negative = faster recovery
        }

        // Apply player condition modifiers
        let condition_factor = player.player_attributes.condition_percentage() as f32 / 100.0;
        if condition_factor < 0.7 {
            effects.injury_risk *= 1.5; // Higher injury risk when tired
            effects.fatigue_change *= 1.2; // Get tired faster when already fatigued
        }

        // Apply professionalism bonus to gains
        let professionalism_bonus = player.attributes.professionalism / 20.0;
        effects.physical_gains =
            Self::apply_bonus_to_physical(effects.physical_gains, professionalism_bonus);
        effects.technical_gains =
            Self::apply_bonus_to_technical(effects.technical_gains, professionalism_bonus);
        effects.mental_gains =
            Self::apply_bonus_to_mental(effects.mental_gains, professionalism_bonus);

        // Apply potential development factor to all skill gains
        effects.physical_gains = Self::scale_physical(effects.physical_gains, potential_factor);
        effects.technical_gains = Self::scale_technical(effects.technical_gains, potential_factor);
        effects.mental_gains = Self::scale_mental(effects.mental_gains, potential_factor);

        // Physical maturity gate: applies *only* to physical_gains so a
        // 14-year-old's strength/stamina session doesn't pour senior-grade
        // physical CA onto an early-puberty body. Technical and mental
        // training are unaffected — drilling ball control or video review
        // remains useful at every age.
        let physical_maturity = Self::calculate_physical_maturity_factor(player.age(date.date()));
        if physical_maturity < 1.0 {
            effects.physical_gains = Self::scale_physical(effects.physical_gains, physical_maturity);
        }

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
            + player.skills.mental.determination)
            / 3.0;

        // Execution randomness: even good players have bad days
        let execution_roll = rand::random::<f32>() * 4.0 - 2.0; // -2 to +2

        // Coach synergy: good coach-player fit produces better sessions
        let synergy = coach_quality * player_receptiveness;

        let session_performance =
            (gain_score + effort_score * 0.5 + synergy * 2.0 + execution_roll).clamp(1.0, 20.0);

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
                (coach.staff_attributes.coaching.attacking
                    + coach.staff_attributes.coaching.defending
                    + coach.staff_attributes.coaching.tactical
                    + coach.staff_attributes.coaching.technical) as f32
                    / 80.0
            }
        };

        // Add determination factor
        let determination_factor = coach.staff_attributes.mental.determination as f32 / 20.0;

        (base_effectiveness * 0.7 + determination_factor * 0.3).min(1.0)
    }

    fn calculate_player_receptiveness(
        player: &Player,
        coach: &Staff,
        sim_date: chrono::NaiveDate,
    ) -> f32 {
        // Base receptiveness from player attributes
        let base = (player.attributes.professionalism + player.attributes.ambition) / 40.0;

        // Relationship with coach affects receptiveness
        let relationship_bonus = if coach.relations.is_favorite_player(player.id) {
            0.2
        } else if coach
            .relations
            .get_player(player.id)
            .map_or(false, |r| r.level < -50.0)
        {
            -0.2
        } else {
            0.0
        };

        // Rapport: tracked explicitly per (coach, player) and persisted
        // across seasons. Broken rapport (≤ -30) severely blunts training,
        // while strong rapport (≥ +60) amplifies it. This is the key
        // "beyond FM" lever for long-term coach-player relationships.
        let rapport_mult = player.rapport.training_multiplier(coach.id);
        let rapport_delta = rapport_mult - 1.0; // -0.15..+0.20

        // Age affects receptiveness (younger players learn faster)
        let age_bonus = match player.age(sim_date) {
            16..=20 => 0.3,
            21..=24 => 0.2,
            25..=28 => 0.1,
            29..=32 => 0.0,
            _ => -0.1,
        };

        (base + relationship_bonus + age_bonus + rapport_delta).clamp(0.1, 1.6)
    }

    fn calculate_age_training_factor(age: u8) -> f32 {
        // Under-16 has its own band: technical/mental sessions still
        // matter for a kid in the academy, but the multiplier is well
        // below the 16-18 peak. The physical maturity gate (applied
        // separately to physical_gains in `train`) keeps strength /
        // stamina / pace work from running away even when the session
        // type is technical.
        match age {
            0..=13 => 0.4,
            14..=15 => 0.7,
            16..=18 => 1.5, // Youth develop quickly
            19..=21 => 1.3,
            22..=24 => 1.1,
            25..=27 => 1.0,
            28..=30 => 0.8,
            31..=33 => 0.5,
            34..=36 => 0.3,
            _ => 0.1, // Very old players barely improve
        }
    }

    /// Physical maturity dampener for training. Applied *only* to
    /// `physical_gains` so that strength/stamina/pace work doesn't
    /// inflate adolescent CA. Technical and mental gains track the
    /// regular age training factor.
    ///
    /// Football rationale: an academy kid can drill ball control all day
    /// and improve, but bench-pressing him into a senior strength frame
    /// is a multi-year process that no training program can shortcut.
    fn calculate_physical_maturity_factor(age: u8) -> f32 {
        match age {
            0..=13 => 0.20,
            14 => 0.30,
            15 => 0.45,
            16 => 0.70,
            17 => 0.88,
            _ => 1.0,
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

        // Age multiplier. Under-16s deliberately get a *smaller* gap
        // boost than 16-18s — the previous 1.5 lumped 14-year-olds in
        // with 18-year-olds and let elite-PA kids accelerate at a peer
        // 18yo's pace. The biological clock can't be shortcut by PA, so
        // the under-16 multiplier sits below the developmental window.
        let age = player.age(sim_date);
        let age_mult = match age {
            0..=13 => 0.4,
            14..=15 => 0.7,
            16..=18 => 1.5,
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

#[cfg(test)]
mod training_load_tests {
    use super::*;
    use crate::club::player::builder::PlayerBuilder;
    use crate::club::staff::staff_stub::StaffStub;
    use crate::shared::fullname::FullName;
    use crate::{
        PersonAttributes, PlayerAttributes, PlayerPosition, PlayerPositionType, PlayerPositions,
        PlayerSkills, TrainingIntensity, TrainingSession, TrainingType,
    };
    use chrono::NaiveDate;

    fn d(y: i32, m: u32, day: u32) -> chrono::NaiveDateTime {
        NaiveDate::from_ymd_opt(y, m, day)
            .unwrap()
            .and_hms_opt(10, 0, 0)
            .unwrap()
    }

    fn build_player(pos: PlayerPositionType) -> Player {
        build_player_with_birth(pos, NaiveDate::from_ymd_opt(2000, 1, 1).unwrap(), 100)
    }

    fn build_player_with_birth(pos: PlayerPositionType, birth: NaiveDate, pa: u8) -> Player {
        let mut attrs = PlayerAttributes::default();
        attrs.condition = 9_000;
        attrs.fitness = 9_000;
        attrs.potential_ability = pa;
        attrs.current_ability = (pa as f32 * 0.5) as u8;
        let mut skills = PlayerSkills::default();
        skills.physical.natural_fitness = 14.0;
        skills.physical.match_readiness = 12.0;
        skills.physical.strength = 10.0;
        skills.physical.stamina = 10.0;
        skills.physical.pace = 10.0;
        skills.physical.agility = 10.0;
        skills.physical.jumping = 10.0;
        skills.mental.work_rate = 10.0;
        skills.mental.determination = 10.0;
        let mut person = PersonAttributes::default();
        person.professionalism = 10.0;
        person.ambition = 10.0;
        PlayerBuilder::new()
            .id(7)
            .full_name(FullName::new("T".to_string(), "P".to_string()))
            .birth_date(birth)
            .country_id(1)
            .attributes(person)
            .skills(skills)
            .positions(PlayerPositions {
                positions: vec![PlayerPosition {
                    position: pos,
                    level: 18,
                }],
            })
            .player_attributes(attrs)
            .build()
            .unwrap()
    }

    fn session(t: TrainingType, intensity: TrainingIntensity) -> TrainingSession {
        TrainingSession {
            session_type: t,
            intensity,
            duration_minutes: 60,
            focus_positions: vec![],
            participants: vec![],
        }
    }

    #[test]
    fn endurance_costs_condition_not_recovers_it() {
        let player = build_player(PlayerPositionType::MidfielderCenter);
        let coach = StaffStub::default();
        let s = session(TrainingType::Endurance, TrainingIntensity::Moderate);
        let r = PlayerTraining::train(&player, &coach, &s, d(2025, 9, 14), 0.6);
        // Endurance must be a positive (costing) fatigue change.
        assert!(
            r.effects.fatigue_change > 0.0,
            "endurance should cost condition, got {}",
            r.effects.fatigue_change
        );
        // It should also book a non-trivial physical_load.
        assert!(r.effects.physical_load_units > 10.0);
    }

    #[test]
    fn recovery_session_restores_condition_with_little_sharpness() {
        let player = build_player(PlayerPositionType::MidfielderCenter);
        let coach = StaffStub::default();
        let s = session(TrainingType::Recovery, TrainingIntensity::VeryLight);
        let r = PlayerTraining::train(&player, &coach, &s, d(2025, 9, 14), 0.6);
        assert!(r.effects.fatigue_change < -300.0);
        // Recovery sharpness should be tiny — far below match-prep / pressing.
        assert!(
            r.effects.readiness_change < 0.3,
            "recovery readiness should be small, got {}",
            r.effects.readiness_change
        );
    }

    #[test]
    fn pressing_drills_load_higher_than_video_analysis() {
        let player = build_player(PlayerPositionType::ForwardLeft);
        let coach = StaffStub::default();
        let pressing = PlayerTraining::train(
            &player,
            &coach,
            &session(TrainingType::PressingDrills, TrainingIntensity::High),
            d(2025, 9, 14),
            0.6,
        );
        let video = PlayerTraining::train(
            &player,
            &coach,
            &session(TrainingType::VideoAnalysis, TrainingIntensity::VeryLight),
            d(2025, 9, 14),
            0.6,
        );
        assert!(
            pressing.effects.fatigue_change > video.effects.fatigue_change + 200.0,
            "pressing fatigue {} vs video fatigue {}",
            pressing.effects.fatigue_change,
            video.effects.fatigue_change,
        );
        assert!(pressing.effects.physical_load_units > video.effects.physical_load_units + 20.0);
        assert!(pressing.effects.readiness_change > video.effects.readiness_change + 0.5);
    }

    #[test]
    fn match_preparation_gains_more_sharpness_than_rest_day() {
        let player = build_player(PlayerPositionType::MidfielderCenter);
        let coach = StaffStub::default();
        let prep = PlayerTraining::train(
            &player,
            &coach,
            &session(TrainingType::MatchPreparation, TrainingIntensity::Light),
            d(2025, 9, 14),
            0.6,
        );
        let rest = PlayerTraining::train(
            &player,
            &coach,
            &session(TrainingType::RestDay, TrainingIntensity::VeryLight),
            d(2025, 9, 14),
            0.6,
        );
        assert!(
            prep.effects.readiness_change > rest.effects.readiness_change + 0.5,
            "prep readiness {} vs rest readiness {}",
            prep.effects.readiness_change,
            rest.effects.readiness_change
        );
    }

    // ── Maturity-aware training gates ────────────────────────────

    #[test]
    fn under_15_physical_session_caps_strength_gain_below_adult() {
        // 14-year-old vs 22-year-old, identical PA, same Strength
        // session. The physical maturity gate must drag the youth's
        // strength gain materially below the adult's.
        let coach = StaffStub::default();
        let s = session(TrainingType::Strength, TrainingIntensity::High);
        let date = d(2025, 9, 14);

        let young = build_player_with_birth(
            PlayerPositionType::ForwardLeft,
            NaiveDate::from_ymd_opt(2011, 1, 1).unwrap(), // 14
            150,
        );
        let adult = build_player_with_birth(
            PlayerPositionType::ForwardLeft,
            NaiveDate::from_ymd_opt(2003, 1, 1).unwrap(), // 22
            150,
        );

        let young_r = PlayerTraining::train(&young, &coach, &s, date, 0.6);
        let adult_r = PlayerTraining::train(&adult, &coach, &s, date, 0.6);

        assert!(
            young_r.effects.physical_gains.strength * 1.5
                < adult_r.effects.physical_gains.strength,
            "14yo strength gain {} too close to 22yo {} — physical maturity gate not biting",
            young_r.effects.physical_gains.strength,
            adult_r.effects.physical_gains.strength
        );
    }

    #[test]
    fn under_15_technical_session_not_dampened_below_adult_baseline() {
        // The maturity gate is *physical-only*. A 14yo doing a passing
        // session can still progress technically (Henry, Wenger drilled
        // technique into 13yos profitably). The age training factor
        // sits at 0.7 vs 1.1 for a 22yo, so the adult still gains more,
        // but the youth value should be a meaningful fraction of it
        // (not a near-zero like physical).
        let coach = StaffStub::default();
        let s = session(TrainingType::Passing, TrainingIntensity::Moderate);
        let date = d(2025, 9, 14);

        let young = build_player_with_birth(
            PlayerPositionType::MidfielderCenter,
            NaiveDate::from_ymd_opt(2011, 1, 1).unwrap(),
            150,
        );
        let adult = build_player_with_birth(
            PlayerPositionType::MidfielderCenter,
            NaiveDate::from_ymd_opt(2003, 1, 1).unwrap(),
            150,
        );

        let young_r = PlayerTraining::train(&young, &coach, &s, date, 0.6);
        let adult_r = PlayerTraining::train(&adult, &coach, &s, date, 0.6);

        let ratio = young_r.effects.technical_gains.passing
            / adult_r.effects.technical_gains.passing.max(1e-6);
        assert!(
            ratio > 0.4,
            "14yo passing gain {} too small relative to 22yo {} (ratio {})",
            young_r.effects.technical_gains.passing,
            adult_r.effects.technical_gains.passing,
            ratio
        );
        assert!(
            ratio < 1.0,
            "14yo passing gain {} should still be below 22yo {}",
            young_r.effects.technical_gains.passing,
            adult_r.effects.technical_gains.passing
        );
    }
}
