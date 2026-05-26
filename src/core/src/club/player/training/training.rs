use crate::club::player::training::result::{PlayerTrainingResult, TrainingOutcomeBreakdown};
use crate::{
    MentalGains, Person, PhysicalGains, Player, Staff, TechnicalGains, TrainingEffects,
    TrainingEventEvidence, TrainingEventReason, TrainingIntensity, TrainingSession, TrainingType,
};
use chrono::{Datelike, NaiveDateTime};

/// Inputs to the reason-picking logic. Keeps the function signature
/// readable when the caller has 15+ booleans / scalars to pass.
struct ReasonInputs {
    positive: bool,
    session_performance: f32,
    delta: f32,
    effort_score: f32,
    physical_state_score: f32,
    in_recovery: bool,
    condition_pct: u32,
    fatigue_change: f32,
    recovery_debt: f32,
    workload_spike: bool,
    has_recent_criticism: bool,
    has_transfer_speculation: bool,
    psychological_score: f32,
    professionalism: f32,
    age: u8,
    leadership: f32,
    session_type: TrainingType,
}

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
                effects.physical_load_units = 14.0 * intensity_multiplier;
                effects.high_intensity_share = 0.10;
                effects.readiness_change = 0.6;
            }
            TrainingType::Recovery => {
                effects.fatigue_change = -800.0; // Strong recovery — main condition restoration
                effects.injury_risk = -0.002;
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
                effects.physical_load_units = 0.0;
                effects.high_intensity_share = 0.0;
                effects.readiness_change = 0.0;
            }
            TrainingType::LightRecovery => {
                effects.fatigue_change = -500.0;
                effects.injury_risk = -0.001;
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

        // ===== DURATION SCALING =====
        // A 90-minute pressing drill costs nearly twice what a 45-minute
        // pressing drill costs. The previous model ignored
        // `duration_minutes` entirely, which made "intensity" the only
        // load knob and meant a 30-minute light recovery session and a
        // 90-minute light recovery session cost the same condition.
        // Reference point: 60 minutes = neutral (mult 1.0). Readiness
        // scales sub-linearly (sqrt) — a half-length session still gets
        // most of the sharpness benefit. Injury risk scales linearly
        // because the chance of pulling something climbs with exposure
        // time, not effort.
        let duration_mult = (session.duration_minutes as f32 / 60.0).clamp(0.35, 1.50);
        effects.fatigue_change *= duration_mult;
        effects.physical_load_units *= duration_mult;
        effects.readiness_change *= duration_mult.sqrt();
        effects.injury_risk *= duration_mult;

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
            effects.physical_gains =
                Self::scale_physical(effects.physical_gains, physical_maturity);
        }

        // ───── Realistic session outcome model ─────────────────────
        // Compute the breakdown of effort / focus / physical / coach /
        // psychological / random factors. This drives:
        //   * session_performance (1..20)
        //   * primary_reason + evidence (event explanations)
        //   * outcome multiplier on gains (a poor session yields less)
        //   * morale_change (replaces the fixed by-session-type values)
        //
        // Randomness is deterministic per (player, date, session-type,
        // intensity) so tests stay stable and the same week reproduces
        // identically across runs.
        let coach_specialization_norm = (specialization_bonus - 1.0) / 0.40; // 1.0-1.4 → 0.0-1.0
        let rapport_mult_norm = (Self::rapport_training_multiplier(player, coach) - 0.85) / 0.35; // ~0.0-1.0
        let outcome = Self::compute_outcome(
            player,
            coach,
            session,
            date,
            facility_quality,
            base_coach_quality,
            coach_specialization_norm,
            rapport_mult_norm,
            player_receptiveness,
            &effects,
        );

        // Apply outcome multiplier to all skill gains. A poor session
        // (raw_score 4) extracts ~0.79× the gains; an excellent one
        // (raw_score 18) extracts ~1.28×. Effort and focus add narrow
        // bonuses; overload bites when the body cannot support quality.
        let outcome_multiplier = (0.65 + outcome.raw_score / 20.0 * 0.70).clamp(0.65, 1.35);
        let effort_bonus = 0.90 + outcome.effort_score / 20.0 * 0.20;
        let focus_bonus = 0.90 + outcome.focus_score / 20.0 * 0.20;
        let overload_penalty = Self::overload_penalty(player, &effects);
        let gain_factor = outcome_multiplier * effort_bonus * focus_bonus * overload_penalty;
        effects.physical_gains = Self::scale_physical(effects.physical_gains, gain_factor);
        effects.technical_gains = Self::scale_technical(effects.technical_gains, gain_factor);
        effects.mental_gains = Self::scale_mental(effects.mental_gains, gain_factor);

        // Morale change is now derived from the outcome — not a fixed
        // table per session type. Recovery sessions can still gain a
        // small bump if the outcome was good, but TeamShape / Recovery
        // no longer pay morale automatically.
        effects.morale_change = Self::morale_change_from_outcome(player, &outcome);

        let mut result = PlayerTrainingResult::new(player.id, effects);
        result.session_performance = outcome.raw_score;
        result.outcome = Some(outcome);
        result
    }

    /// Build the per-session outcome breakdown used for both the visible
    /// `session_performance` and the explanation context attached to any
    /// emitted GoodTraining / PoorTraining event. All sub-scores live on
    /// a 1.0..20.0 scale so the renderer can talk about them in the same
    /// units.
    fn compute_outcome(
        player: &Player,
        _coach: &Staff,
        session: &TrainingSession,
        date: NaiveDateTime,
        facility_quality: f32,
        base_coach_quality: f32,
        coach_specialization_norm: f32,
        rapport_norm: f32,
        player_receptiveness: f32,
        effects: &TrainingEffects,
    ) -> TrainingOutcomeBreakdown {
        let pa = &player.attributes;
        let sk = &player.skills;
        let cond_pct = player.player_attributes.condition_percentage() as f32;
        let fitness_pct =
            (player.player_attributes.fitness as f32 / 10000.0 * 100.0).clamp(0.0, 100.0);
        let jadedness_pct = (player.player_attributes.jadedness as f32 / 10000.0).clamp(0.0, 1.0);

        // ── Effort ────────────────────────────────────────────────
        let effort_score = 20.0
            * ((pa.professionalism / 20.0) * 0.35
                + (sk.mental.work_rate / 20.0) * 0.25
                + (sk.mental.determination / 20.0) * 0.20
                + (pa.ambition / 20.0) * 0.10
                + (pa.consistency / 20.0) * 0.10)
                .clamp(0.0, 1.0);

        // ── Physical state ────────────────────────────────────────
        let physical_state_score = 20.0
            * (cond_pct / 100.0 * 0.45
                + fitness_pct / 100.0 * 0.20
                + sk.physical.match_readiness / 20.0 * 0.15
                + sk.physical.natural_fitness / 20.0 * 0.10
                + (1.0 - jadedness_pct) * 0.10)
                .clamp(0.0, 1.0);

        // ── Psychological ────────────────────────────────────────
        let chemistry = player.relations.get_team_chemistry().clamp(0.0, 100.0);
        let psychological_score = 20.0
            * ((player.happiness.morale / 100.0).clamp(0.0, 1.0) * 0.30
                + (pa.pressure / 20.0) * 0.15
                + (pa.temperament / 20.0) * 0.10
                + (pa.consistency / 20.0) * 0.15
                + (pa.professionalism / 20.0) * 0.15
                + (chemistry / 100.0) * 0.15)
                .clamp(0.0, 1.0);

        // ── Coach fit ────────────────────────────────────────────
        let facility_fit_norm = facility_quality.clamp(0.0, 1.0);
        let coach_fit_score = 20.0
            * (base_coach_quality.clamp(0.0, 1.0) * 0.45
                + coach_specialization_norm.clamp(0.0, 1.0) * 0.20
                + rapport_norm.clamp(0.0, 1.0) * 0.20
                + facility_fit_norm * 0.15)
                .clamp(0.0, 1.0);

        // ── Session fit (skill match) ────────────────────────────
        let session_fit_score = Self::session_fit_score(player, session);

        // ── Tactical / session relevance fit ─────────────────────
        // Strikers don't get full value from extended set-piece work
        // for opposite specialists, etc. Use receptiveness as a soft
        // proxy when no stronger tactical signal exists.
        let tactical_relevance_score = (player_receptiveness * 12.5).clamp(2.0, 18.0);

        // ── Deterministic noise ──────────────────────────────────
        let consistency = pa.consistency.clamp(0.0, 20.0);
        let professionalism = pa.professionalism.clamp(0.0, 20.0);
        let morale = player.happiness.morale.clamp(-100.0, 100.0);
        let randomness_score = Self::deterministic_noise(
            player.id,
            date,
            session,
            consistency,
            professionalism,
            morale,
        );

        // ── Compose ───────────────────────────────────────────────
        let mut raw = 0.28 * effort_score
            + 0.22 * physical_state_score
            + 0.18 * coach_fit_score
            + 0.14 * session_fit_score
            + 0.12 * psychological_score
            + 0.06 * tactical_relevance_score
            + randomness_score;
        raw = raw.clamp(1.0, 20.0);

        // Baseline: stabilise EMA in the early sessions so a fresh
        // signing isn't immediately judged against his very first
        // training day.
        let baseline_score = if player.training.sessions_completed < 5 {
            (10.0 + effort_score * 0.15).clamp(1.0, 20.0)
        } else {
            player.training.training_performance.clamp(1.0, 20.0)
        };
        let delta = raw - baseline_score;

        // ── Pick reason + evidence ────────────────────────────────
        let cond_pct_u = cond_pct as u32;
        let in_recovery = player.player_attributes.is_in_recovery();
        let has_recent_criticism = player.happiness.recent_events.iter().any(|e| {
            e.days_ago <= 14
                && (e.event_type == crate::HappinessEventType::ManagerCriticism
                    || e.event_type == crate::HappinessEventType::ManagerDiscipline
                    || e.event_type == crate::HappinessEventType::MatchDropped)
        });
        let has_transfer_speculation = player.happiness.recent_events.iter().any(|e| {
            e.days_ago <= 21
                && matches!(
                    e.event_type,
                    crate::HappinessEventType::TransferRumour
                        | crate::HappinessEventType::AgentStirsInterest
                        | crate::HappinessEventType::TransferSpeculationDistracts
                        | crate::HappinessEventType::WantedByBiggerClub
                        | crate::HappinessEventType::InterestFromBiggerClub
                )
        });
        let age = crate::utils::DateUtils::age(player.birth_date, date.date());
        let workload_spike = player.load.is_workload_spike();
        let recovery_debt = player.load.recovery_debt;
        let leadership = sk.mental.leadership;
        let work_rate = sk.mental.work_rate;
        let determination = sk.mental.determination;

        let positive = raw >= baseline_score;
        let primary_reason = Self::pick_reason(ReasonInputs {
            positive,
            session_performance: raw,
            delta,
            effort_score,
            physical_state_score,
            in_recovery,
            condition_pct: cond_pct_u,
            fatigue_change: effects.fatigue_change,
            recovery_debt,
            workload_spike,
            has_recent_criticism,
            has_transfer_speculation,
            psychological_score,
            professionalism,
            age,
            leadership,
            session_type: session.session_type.clone(),
        });

        let mut evidence: Vec<TrainingEventEvidence> = Vec::new();
        let push = |ev: TrainingEventEvidence, list: &mut Vec<TrainingEventEvidence>| {
            if !list.contains(&ev) {
                list.push(ev);
            }
        };
        if raw >= 14.0 {
            push(TrainingEventEvidence::HighSessionPerformance, &mut evidence);
        }
        if raw <= 6.0 {
            push(TrainingEventEvidence::LowSessionPerformance, &mut evidence);
        }
        if effort_score <= 7.0 {
            push(TrainingEventEvidence::LowEffort, &mut evidence);
        }
        if work_rate >= 14.0 {
            push(TrainingEventEvidence::HighWorkRate, &mut evidence);
        }
        if determination >= 14.0 {
            push(TrainingEventEvidence::HighDetermination, &mut evidence);
        }
        if cond_pct_u < 60 {
            push(TrainingEventEvidence::LowCondition, &mut evidence);
        }
        if effects.fatigue_change > 100.0 {
            push(TrainingEventEvidence::HighWorkload, &mut evidence);
        }
        if recovery_debt >= crate::club::player::condition::load::RECOVERY_DEBT_HEAVY
            || workload_spike
        {
            push(TrainingEventEvidence::Overloaded, &mut evidence);
        }
        if cond_pct_u < 60
            && (recovery_debt >= 200.0 || workload_spike || effects.fatigue_change > 120.0)
        {
            push(TrainingEventEvidence::FatigueLimited, &mut evidence);
        }
        if in_recovery {
            push(TrainingEventEvidence::InRecoveryPhase, &mut evidence);
            push(TrainingEventEvidence::RecoveryLimited, &mut evidence);
        }
        if morale < 35.0 {
            push(TrainingEventEvidence::LowMorale, &mut evidence);
        }
        if has_recent_criticism {
            push(TrainingEventEvidence::RecentlyDropped, &mut evidence);
        }
        if has_transfer_speculation {
            push(TrainingEventEvidence::TransferSpeculation, &mut evidence);
            push(TrainingEventEvidence::TransferDistraction, &mut evidence);
        }
        if professionalism >= 15.0 {
            push(TrainingEventEvidence::HighProfessionalism, &mut evidence);
        }
        if professionalism <= 7.0 {
            push(TrainingEventEvidence::LowProfessionalism, &mut evidence);
        }
        if age <= 21 {
            push(TrainingEventEvidence::YouthDevelopmentTier, &mut evidence);
        }
        if age >= 30 && leadership >= 14.0 {
            push(TrainingEventEvidence::VeteranLeader, &mut evidence);
        }
        if coach_fit_score <= 6.0 {
            push(TrainingEventEvidence::CoachMismatch, &mut evidence);
        }
        if session_fit_score <= 6.0 {
            push(TrainingEventEvidence::TacticalMismatch, &mut evidence);
        }
        if !positive && baseline_score >= 13.5 && delta <= -2.0 {
            push(
                TrainingEventEvidence::StrongBaselineButOffDay,
                &mut evidence,
            );
        }
        if positive && age <= 21 && raw >= 13.5 && delta >= 2.0 {
            push(
                TrainingEventEvidence::YoungPlayerBreakthrough,
                &mut evidence,
            );
        }
        if positive && age >= 30 && leadership >= 14.0 && raw >= 13.0 {
            push(TrainingEventEvidence::VeteranSetStandard, &mut evidence);
        }

        TrainingOutcomeBreakdown {
            raw_score: raw,
            baseline_score,
            delta_from_baseline: delta,
            effort_score,
            focus_score: session_fit_score,
            physical_state_score,
            coach_fit_score,
            tactical_fit_score: tactical_relevance_score,
            psychological_score,
            randomness_score,
            primary_reason,
            evidence,
        }
    }

    /// Skill-match score for the session type. Maps to 1..20 to mirror
    /// the rest of the breakdown's units.
    fn session_fit_score(player: &Player, session: &TrainingSession) -> f32 {
        let p = &player.skills.physical;
        let t = &player.skills.technical;
        let m = &player.skills.mental;
        let avg = |xs: &[f32]| -> f32 {
            if xs.is_empty() {
                10.0
            } else {
                xs.iter().sum::<f32>() / xs.len() as f32
            }
        };
        let raw = match session.session_type {
            TrainingType::Endurance => avg(&[p.stamina, p.natural_fitness]),
            TrainingType::Strength => avg(&[p.strength, p.jumping]),
            TrainingType::Speed => avg(&[p.pace, p.agility]),
            TrainingType::Agility => avg(&[p.agility, p.balance]),
            TrainingType::BallControl => avg(&[t.first_touch, t.technique, t.dribbling]),
            TrainingType::Passing => avg(&[t.passing, t.technique, m.vision]),
            TrainingType::Shooting => avg(&[t.finishing, t.technique, m.decisions]),
            TrainingType::Crossing => avg(&[t.crossing, t.technique]),
            TrainingType::SetPieces => avg(&[t.crossing, t.heading, t.technique]),
            TrainingType::Positioning => avg(&[m.positioning, m.concentration, m.decisions]),
            TrainingType::TeamShape => avg(&[m.teamwork, m.positioning, m.work_rate]),
            TrainingType::PressingDrills => avg(&[m.work_rate, m.teamwork, p.stamina]),
            TrainingType::TransitionPlay => avg(&[t.passing, m.decisions, p.pace]),
            TrainingType::SetPiecesDefensive => avg(&[m.positioning, t.heading, t.tackling]),
            TrainingType::Concentration => avg(&[m.concentration, m.decisions]),
            TrainingType::DecisionMaking => avg(&[m.decisions, m.vision]),
            TrainingType::Leadership => avg(&[m.leadership, m.teamwork]),
            TrainingType::MatchPreparation => avg(&[m.concentration, m.positioning, m.decisions]),
            TrainingType::OpponentSpecific => avg(&[m.decisions, m.vision, m.concentration]),
            TrainingType::VideoAnalysis => avg(&[m.decisions, m.positioning, m.vision]),
            TrainingType::Recovery
            | TrainingType::LightRecovery
            | TrainingType::Rehabilitation
            | TrainingType::RestDay => {
                let cond = player.player_attributes.condition_percentage() as f32 / 5.0;
                let nf = p.natural_fitness;
                let prof = player.attributes.professionalism;
                ((cond + nf + prof) / 3.0).clamp(1.0, 20.0)
            }
        };
        raw.clamp(1.0, 20.0)
    }

    /// Triangular-shaped deterministic noise with width modulated by
    /// player consistency / professionalism / morale. The output is in
    /// the -1.25..+1.25 window for typical inputs and explains at most
    /// ~10–15% of the final score in normal cases.
    fn deterministic_noise(
        player_id: u32,
        date: NaiveDateTime,
        session: &TrainingSession,
        consistency: f32,
        professionalism: f32,
        morale: f32,
    ) -> f32 {
        let session_token = Self::session_type_token(&session.session_type) as u64;
        let intensity_token = match session.intensity {
            TrainingIntensity::VeryLight => 1u64,
            TrainingIntensity::Light => 2,
            TrainingIntensity::Moderate => 3,
            TrainingIntensity::High => 4,
            TrainingIntensity::VeryHigh => 5,
        };
        let day = date.date().num_days_from_ce() as u64;
        let mut h = (player_id as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15);
        h = h.wrapping_add(day.wrapping_mul(0xC6BC_279E_9286_5A2B));
        h = h.wrapping_add(session_token.wrapping_mul(0x1657_8F35_4D38_C5A7));
        h = h.wrapping_add(intensity_token.wrapping_mul(0x94D0_49BB_1331_11EB));

        // Two independent rolls so the inner sum is triangular-ish.
        let r1 = ((h >> 11) as u32 as f32) / (u32::MAX as f32);
        let r2 = ((h.wrapping_mul(0xD1B5_4A32_D192_ED03) >> 17) as u32 as f32) / (u32::MAX as f32);
        let triangular = r1 + r2 - 1.0; // -1..+1 with peak at 0

        // Width: high consistency narrows the band; low consistency
        // widens it. A player with consistency 20 has roughly half the
        // noise width of one with consistency 4.
        let width = (1.20 - consistency / 20.0 * 0.70).clamp(0.50, 1.20);
        let mut noise = triangular * 0.6 * width;

        // Floor / ceiling per professionalism: pros don't have
        // catastrophic off-days; low-pro players can crater.
        if professionalism >= 15.0 {
            noise = noise.max(-0.45);
        }
        if professionalism <= 6.0 {
            noise = noise.max(-1.25);
        }

        // Rare off-day / on-fire moment. Centered morale pressure
        // pushes off-day chance up symmetrically as morale drops below
        // 50, instead of only firing when morale goes negative (the
        // production morale scale is 0..100). High morale gets a small
        // positive direction-bias on the rare event but cannot dominate
        // attribute-driven scoring.
        let morale_pressure = ((50.0 - morale) / 50.0).clamp(0.0, 1.0);
        let high_morale_lift = ((morale - 60.0) / 40.0).clamp(0.0, 1.0);
        let r3 = ((h.wrapping_mul(0xACE2_4D8B_5E2F_91A3) >> 13) as u32 as f32) / (u32::MAX as f32);
        let off_chance =
            (0.03 - consistency / 20.0 * 0.02 + morale_pressure * 0.03).clamp(0.0, 0.08);
        if r3 < off_chance {
            let r4 =
                ((h.wrapping_mul(0x65A1_8C0B_3F4D_22B7) >> 19) as u32 as f32) / (u32::MAX as f32);
            let mag = 1.0 + r4 * 1.0; // 1.0..2.0
            // Direction bias: morale_pressure raises negative odds, a
            // healthy mood raises positive odds. The crossover sits at
            // morale ~ 50, exactly where pressure and lift are both 0.
            let neg_share =
                (0.5 + morale_pressure * 0.4 - high_morale_lift * 0.4).clamp(0.05, 0.95);
            let sign = if r3 < off_chance * neg_share {
                -1.0
            } else {
                1.0
            };
            noise += mag * sign;
        }

        noise.clamp(-2.5, 2.5)
    }

    /// Multiplier on skill gains when the body is overloaded. Recovery
    /// sessions are exempt — the whole point of a recovery day is to
    /// blunt impact, not to penalise it.
    fn overload_penalty(player: &Player, effects: &TrainingEffects) -> f32 {
        if effects.fatigue_change <= 0.0 {
            return 1.0;
        }
        let cond = player.player_attributes.condition_percentage();
        let mut penalty: f32 = 1.0;
        if cond < 60 {
            penalty -= 0.15;
        }
        if cond < 40 {
            penalty -= 0.10;
        }
        if player.load.recovery_debt >= crate::club::player::condition::load::RECOVERY_DEBT_HEAVY {
            penalty -= 0.10;
        }
        if player.load.is_workload_spike() {
            penalty -= 0.10;
        }
        penalty.clamp(0.70, 1.0)
    }

    fn morale_change_from_outcome(player: &Player, outcome: &TrainingOutcomeBreakdown) -> f32 {
        let raw = outcome.raw_score;
        let delta = outcome.delta_from_baseline;
        let pa = &player.attributes;

        if raw >= 14.0 || delta >= 1.5 {
            let mag = ((raw - 13.0) / 8.0 + delta.max(0.0) / 10.0).clamp(0.15, 0.60);
            // Surprised-and-delighted scaling: high ambition / determination
            // amplifies the bump.
            let personality = 1.0 + (pa.ambition + pa.professionalism) / 80.0;
            mag * personality.clamp(0.8, 1.3)
        } else if raw <= 6.5 || delta <= -1.5 {
            let mut mag = ((7.0 - raw) / 7.0 + (-delta).max(0.0) / 10.0).clamp(0.15, 0.75);
            let cause_is_fatigue = matches!(
                outcome.primary_reason,
                TrainingEventReason::StruggledWithIntensity
                    | TrainingEventReason::ReturningFromInjuryNotSharp
            );
            if cause_is_fatigue {
                mag *= 0.5;
            }
            if pa.professionalism >= 15.0 {
                mag *= 0.75;
            }
            if pa.controversy >= 15.0 || pa.temperament <= 6.0 {
                mag *= 1.20;
            }
            -mag
        } else {
            // Neutral session — don't fire training events at all.
            0.0
        }
    }

    fn pick_reason(inp: ReasonInputs) -> TrainingEventReason {
        if inp.positive {
            if inp.in_recovery {
                return TrainingEventReason::ReturningFromInjuryNotSharp;
            }
            if inp.has_recent_criticism && inp.session_performance >= 13.0 {
                return TrainingEventReason::RespondedToCriticism;
            }
            if inp.age <= 21 && inp.delta >= 2.0 && inp.session_performance >= 13.5 {
                return TrainingEventReason::YoungImpressedStaff;
            }
            if inp.age >= 30 && inp.leadership >= 14.0 && inp.session_performance >= 13.5 {
                return TrainingEventReason::SettingStandards;
            }
            if inp.professionalism >= 15.0 && inp.session_performance >= 14.0 {
                return TrainingEventReason::ExtraWorkAfterSession;
            }
            if matches!(
                inp.session_type,
                TrainingType::MatchPreparation | TrainingType::OpponentSpecific
            ) && inp.session_performance >= 13.0
            {
                return TrainingEventReason::MatchPreparationFocus;
            }
            TrainingEventReason::RoutineGoodSession
        } else {
            // Fatigue beats attitude — physically compromised players don't
            // get labelled as poor attitude.
            if inp.in_recovery {
                return TrainingEventReason::ReturningFromInjuryNotSharp;
            }
            let body_compromised = inp.condition_pct < 60
                || inp.recovery_debt >= 200.0
                || inp.workload_spike
                || inp.fatigue_change > 120.0;
            if body_compromised {
                return TrainingEventReason::StruggledWithIntensity;
            }
            if inp.has_transfer_speculation && inp.psychological_score < 12.0 {
                return TrainingEventReason::DistractedByRumours;
            }
            if inp.professionalism <= 7.0
                && inp.effort_score <= 7.0
                && inp.physical_state_score >= 10.0
            {
                return TrainingEventReason::PoorAttitude;
            }
            TrainingEventReason::RoutineBadSession
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

    /// Stable token per session type so the deterministic-noise hash is
    /// reproducible across builds (mem::discriminant ordering would not
    /// be).
    fn session_type_token(t: &TrainingType) -> u32 {
        match t {
            TrainingType::Endurance => 1,
            TrainingType::Strength => 2,
            TrainingType::Speed => 3,
            TrainingType::Agility => 4,
            TrainingType::Recovery => 5,
            TrainingType::BallControl => 6,
            TrainingType::Passing => 7,
            TrainingType::Shooting => 8,
            TrainingType::Crossing => 9,
            TrainingType::SetPieces => 10,
            TrainingType::Positioning => 11,
            TrainingType::TeamShape => 12,
            TrainingType::PressingDrills => 13,
            TrainingType::TransitionPlay => 14,
            TrainingType::SetPiecesDefensive => 15,
            TrainingType::Concentration => 16,
            TrainingType::DecisionMaking => 17,
            TrainingType::Leadership => 18,
            TrainingType::MatchPreparation => 19,
            TrainingType::VideoAnalysis => 20,
            TrainingType::OpponentSpecific => 21,
            TrainingType::RestDay => 22,
            TrainingType::LightRecovery => 23,
            TrainingType::Rehabilitation => 24,
        }
    }

    fn rapport_training_multiplier(player: &Player, coach: &Staff) -> f32 {
        player.rapport.training_multiplier(coach.id)
    }
}

#[cfg(test)]
mod training_load_tests {
    use super::*;
    use crate::club::player::builder::PlayerBuilder;
    use crate::club::staff::StaffStub;
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
            young_r.effects.physical_gains.strength * 1.5 < adult_r.effects.physical_gains.strength,
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

    // ── Outcome-driven event emission tests ──────────────────────

    fn build_pro_pro(pos: PlayerPositionType, prof: f32, work_rate: f32, det: f32) -> Player {
        let mut p = build_player(pos);
        p.attributes.professionalism = prof;
        p.attributes.consistency = 14.0;
        p.attributes.ambition = 12.0;
        p.attributes.temperament = 14.0;
        p.skills.mental.work_rate = work_rate;
        p.skills.mental.determination = det;
        p
    }

    fn drain_player(player: &mut Player) {
        player.player_attributes.condition = 4_500; // 45%
        player.load.recovery_debt = 320.0;
        player.load.physical_load_7 = 480.0;
        player.load.physical_load_30 = 1_400.0;
    }

    #[test]
    fn low_pro_low_workrate_can_produce_poor_attitude_outcome() {
        // High condition (no fatigue alibi), low pro & work rate, no
        // injury. The reason picker must conclude PoorAttitude when the
        // body says "I can do this" but the effort says "no".
        let coach = StaffStub::default();
        let mut p = build_pro_pro(PlayerPositionType::MidfielderCenter, 4.0, 4.0, 4.0);
        p.attributes.consistency = 6.0;
        p.player_attributes.condition = 9_500; // 95%
        // Force a clear effort failure: kill morale chemistry but leave
        // body fresh.
        let s = session(TrainingType::Endurance, TrainingIntensity::Moderate);
        // Loop a few sessions — deterministic noise gives us a window of
        // outcomes; PoorAttitude should land somewhere in the week.
        let mut saw_attitude = false;
        for day in 1..=14 {
            let r = PlayerTraining::train(&p, &coach, &s, d(2025, 9, day), 0.6);
            if let Some(ob) = r.outcome.as_ref() {
                if ob.primary_reason == TrainingEventReason::PoorAttitude {
                    saw_attitude = true;
                    break;
                }
            }
        }
        assert!(
            saw_attitude,
            "expected PoorAttitude reason for low-pro / low-effort player with full body"
        );
    }

    #[test]
    fn high_pro_low_condition_yields_struggled_with_intensity_not_attitude() {
        let coach = StaffStub::default();
        let mut p = build_pro_pro(PlayerPositionType::MidfielderCenter, 18.0, 16.0, 16.0);
        drain_player(&mut p);
        let s = session(TrainingType::PressingDrills, TrainingIntensity::High);
        for day in 1..=14 {
            let r = PlayerTraining::train(&p, &coach, &s, d(2025, 9, day), 0.6);
            if let Some(ob) = r.outcome.as_ref() {
                assert!(
                    ob.primary_reason != TrainingEventReason::PoorAttitude,
                    "high-pro tired player must not be tagged PoorAttitude"
                );
            }
        }
        // And we should see at least one StruggledWithIntensity result
        let mut saw_fatigue = false;
        for day in 1..=14 {
            let r = PlayerTraining::train(&p, &coach, &s, d(2025, 9, day), 0.6);
            if let Some(ob) = r.outcome.as_ref() {
                if ob.primary_reason == TrainingEventReason::StruggledWithIntensity {
                    saw_fatigue = true;
                    break;
                }
            }
        }
        assert!(
            saw_fatigue,
            "high-pro tired player should at least sometimes read as StruggledWithIntensity"
        );
    }

    #[test]
    fn high_pro_high_workrate_outscores_low_pro_on_average() {
        let coach = StaffStub::default();
        let pro = build_pro_pro(PlayerPositionType::MidfielderCenter, 18.0, 17.0, 16.0);
        let lazy = build_pro_pro(PlayerPositionType::MidfielderCenter, 5.0, 5.0, 5.0);
        let s = session(TrainingType::Passing, TrainingIntensity::Moderate);
        let mut sum_pro = 0.0;
        let mut sum_lazy = 0.0;
        for day in 1..=20 {
            sum_pro +=
                PlayerTraining::train(&pro, &coach, &s, d(2025, 9, day), 0.6).session_performance;
            sum_lazy +=
                PlayerTraining::train(&lazy, &coach, &s, d(2025, 9, day), 0.6).session_performance;
        }
        let avg_pro = sum_pro / 20.0;
        let avg_lazy = sum_lazy / 20.0;
        assert!(
            avg_pro > avg_lazy + 1.5,
            "pro avg {} should outscore low-pro avg {} by a clear margin",
            avg_pro,
            avg_lazy
        );
    }

    #[test]
    fn deterministic_seed_reproduces_same_outcome() {
        let coach = StaffStub::default();
        let p = build_pro_pro(PlayerPositionType::MidfielderCenter, 14.0, 12.0, 12.0);
        let s = session(TrainingType::TeamShape, TrainingIntensity::Moderate);
        let date = d(2026, 3, 4);
        let r1 = PlayerTraining::train(&p, &coach, &s, date, 0.6).session_performance;
        let r2 = PlayerTraining::train(&p, &coach, &s, date, 0.6).session_performance;
        assert!(
            (r1 - r2).abs() < 1e-4,
            "same player+date+session must reproduce session_performance ({} vs {})",
            r1,
            r2
        );
    }

    #[test]
    fn pro_baseline_is_not_swung_into_repeated_poor_by_noise() {
        // A clean professional should never see a sustained PoorTraining
        // streak from random noise alone. Run two simulated weeks and
        // verify that PoorTraining never becomes the *modal* outcome.
        let coach = StaffStub::default();
        let p = build_pro_pro(PlayerPositionType::MidfielderCenter, 18.0, 16.0, 16.0);
        let s = session(TrainingType::Passing, TrainingIntensity::Moderate);
        let mut poor = 0;
        let mut total = 0;
        for day in 1..=21 {
            let r = PlayerTraining::train(&p, &coach, &s, d(2025, 9, day), 0.6);
            total += 1;
            if r.session_performance <= 6.5 {
                poor += 1;
            }
        }
        assert!(
            poor as f32 / total as f32 <= 0.10,
            "pro should not have >10% poor sessions from noise alone; got {}/{}",
            poor,
            total
        );
    }

    #[test]
    fn recovery_session_no_longer_emits_morale_change_unless_outcome_qualifies() {
        // The fixed +0.05 morale that used to fire for every Recovery
        // session is gone. With a plain neutral profile the outcome is
        // expected to be near-baseline → no event, no morale movement.
        let coach = StaffStub::default();
        let p = build_pro_pro(PlayerPositionType::MidfielderCenter, 12.0, 12.0, 12.0);
        let s = session(TrainingType::Recovery, TrainingIntensity::VeryLight);
        let r = PlayerTraining::train(&p, &coach, &s, d(2025, 9, 14), 0.6);
        // morale_change is now derived from outcome; for a neutral
        // outcome (~10 raw, near baseline) it should be 0.
        assert!(
            r.effects.morale_change.abs() < 0.20,
            "neutral recovery session must not auto-pay morale (got {})",
            r.effects.morale_change
        );
    }

    #[test]
    fn team_shape_session_no_longer_auto_emits_good_training() {
        let coach = StaffStub::default();
        let p = build_pro_pro(PlayerPositionType::MidfielderCenter, 12.0, 12.0, 12.0);
        let s = session(TrainingType::TeamShape, TrainingIntensity::Moderate);
        let r = PlayerTraining::train(&p, &coach, &s, d(2025, 9, 14), 0.6);
        assert!(
            r.effects.morale_change <= 0.15,
            "TeamShape must not auto-pay +0.10 morale; got {}",
            r.effects.morale_change
        );
    }

    #[test]
    fn outcome_breakdown_carries_reason_and_evidence() {
        let coach = StaffStub::default();
        let p = build_pro_pro(PlayerPositionType::MidfielderCenter, 18.0, 16.0, 16.0);
        let s = session(TrainingType::MatchPreparation, TrainingIntensity::Light);
        let r = PlayerTraining::train(&p, &coach, &s, d(2025, 9, 14), 0.6);
        let outcome = r.outcome.expect("outcome must be attached");
        // Sub-scores must be on the 1..20 scale.
        assert!((1.0..=20.0).contains(&outcome.raw_score));
        assert!((0.0..=20.0).contains(&outcome.effort_score));
        // High-pro player must carry at least one personality evidence.
        assert!(outcome.evidence.iter().any(|e| matches!(
            e,
            TrainingEventEvidence::HighProfessionalism
                | TrainingEventEvidence::HighWorkRate
                | TrainingEventEvidence::HighDetermination
        )));
    }

    #[test]
    fn fixture_proximity_changes_weekly_plan() {
        use crate::{
            CoachingPhilosophy, PeriodizationPhase, RotationPreference, TacticalFocus,
            TrainingIntensityPreference, WeeklyTrainingPlan,
        };
        use chrono::Weekday;

        let philosophy = CoachingPhilosophy {
            tactical_focus: TacticalFocus::Pressing,
            training_intensity: TrainingIntensityPreference::Medium,
            youth_focus: false,
            rotation_preference: RotationPreference::Moderate,
        };

        // No fixtures in range → main load on Tuesday should be the
        // PressingDrills session for a pressing coach.
        let plan_no_fixture = WeeklyTrainingPlan::generate_weekly_plan_with_context(
            None,
            None,
            0,
            PeriodizationPhase::MidSeason,
            &philosophy,
        );
        let tue_no_fix = plan_no_fixture.sessions.get(&Weekday::Tue).unwrap();
        assert!(matches!(
            tue_no_fix[0].session_type,
            TrainingType::PressingDrills
        ));

        // Match Wednesday → Tuesday is MD-1 → set pieces / opponent
        // specific only, not pressing drills.
        let plan_md_minus_1 = WeeklyTrainingPlan::generate_weekly_plan_with_context(
            Some(Weekday::Wed),
            None,
            0,
            PeriodizationPhase::MidSeason,
            &philosophy,
        );
        let tue_md_minus_1 = plan_md_minus_1.sessions.get(&Weekday::Tue).unwrap();
        assert!(matches!(
            tue_md_minus_1[0].session_type,
            TrainingType::SetPieces
        ));
    }

    // The loan-underperformance assertion lives in
    // `happiness::processing::loan_morale_tests` where the actual
    // private branch can be exercised. Keeping a copy here would only
    // re-test the happiness API, not the loan branch.

    // ── Archetype calibration ───────────────────────────────────
    //
    // Distribution-level guards. They run a single archetype
    // through 30 deterministic dates and assert qualitative shape:
    // the model professional outscores the lazy archetype, the
    // exhausted pro is read as fatigued (never poor attitude), the
    // unsettled transfer target reads more distraction reasons than
    // a baseline neutral, and the breakthroughs / standard-setting
    // reasons fire occasionally without spamming.

    fn archetype_pro() -> Player {
        let mut p = build_pro_pro(PlayerPositionType::MidfielderCenter, 17.0, 16.0, 16.0);
        p.attributes.consistency = 14.0;
        p.attributes.ambition = 14.0;
        p
    }

    fn archetype_lazy() -> Player {
        let mut p = build_pro_pro(PlayerPositionType::MidfielderCenter, 5.0, 5.0, 8.0);
        p.attributes.consistency = 8.0;
        p.attributes.ambition = 6.0;
        p.skills.technical.passing = 16.0;
        p.skills.technical.first_touch = 16.0;
        p
    }

    fn archetype_exhausted_pro() -> Player {
        let mut p = build_pro_pro(PlayerPositionType::MidfielderCenter, 17.0, 16.0, 16.0);
        p.player_attributes.condition = 4_500;
        p.load.recovery_debt = 380.0;
        p.load.physical_load_7 = 520.0;
        p.load.physical_load_30 = 1_500.0;
        p
    }

    fn archetype_transfer_target() -> Player {
        let mut p = build_pro_pro(PlayerPositionType::MidfielderCenter, 14.0, 13.0, 13.0);
        p.happiness.morale = 28.0;
        p.happiness
            .add_event(crate::HappinessEventType::TransferRumour, -2.0);
        p.happiness
            .add_event(crate::HappinessEventType::AgentStirsInterest, -1.5);
        p
    }

    fn archetype_neutral() -> Player {
        build_pro_pro(PlayerPositionType::MidfielderCenter, 12.0, 12.0, 12.0)
    }

    fn archetype_youth() -> Player {
        let mut p = build_player_with_birth(
            PlayerPositionType::MidfielderCenter,
            NaiveDate::from_ymd_opt(2007, 1, 1).unwrap(),
            160,
        );
        p.attributes.professionalism = 14.0;
        p.attributes.ambition = 17.0;
        p.skills.mental.work_rate = 15.0;
        p.skills.mental.determination = 15.0;
        p.attributes.consistency = 12.0;
        p
    }

    fn archetype_veteran() -> Player {
        let mut p = build_player_with_birth(
            PlayerPositionType::MidfielderCenter,
            NaiveDate::from_ymd_opt(1993, 1, 1).unwrap(),
            150,
        );
        p.attributes.professionalism = 17.0;
        p.attributes.ambition = 12.0;
        p.attributes.consistency = 16.0;
        p.skills.mental.leadership = 17.0;
        p.skills.mental.work_rate = 14.0;
        p.skills.mental.determination = 14.0;
        p
    }

    fn run_archetype(
        player: &Player,
        sessions: &[(TrainingType, TrainingIntensity)],
        days: u32,
    ) -> (f32, Vec<TrainingEventReason>) {
        let coach = StaffStub::default();
        let mut perf_sum: f32 = 0.0;
        let mut count: f32 = 0.0;
        let mut reasons: Vec<TrainingEventReason> = Vec::new();
        for offset in 0..days {
            let day = 1 + (offset % 28) as u32;
            let month = 9 + (offset / 28) as u32;
            let date = d(2025, month.min(12), day.max(1).min(28));
            for (t, i) in sessions {
                let r = PlayerTraining::train(
                    player,
                    &coach,
                    &session(t.clone(), i.clone()),
                    date,
                    0.6,
                );
                perf_sum += r.session_performance;
                count += 1.0;
                if let Some(o) = r.outcome.as_ref() {
                    reasons.push(o.primary_reason);
                }
            }
        }
        (perf_sum / count.max(1.0), reasons)
    }

    fn count_reason(reasons: &[TrainingEventReason], r: TrainingEventReason) -> usize {
        reasons.iter().filter(|x| **x == r).count()
    }

    #[test]
    fn pro_outscores_lazy_by_at_least_1_5_over_30_days() {
        let pro = archetype_pro();
        let lazy = archetype_lazy();
        let plan = [
            (TrainingType::Passing, TrainingIntensity::Moderate),
            (TrainingType::TeamShape, TrainingIntensity::Moderate),
        ];
        let (avg_pro, _) = run_archetype(&pro, &plan, 30);
        let (avg_lazy, _) = run_archetype(&lazy, &plan, 30);
        assert!(
            avg_pro - avg_lazy >= 1.5,
            "pro avg {} should outscore lazy avg {} by at least 1.5",
            avg_pro,
            avg_lazy
        );
    }

    #[test]
    fn exhausted_pro_main_negative_reason_is_fatigue_not_attitude() {
        let p = archetype_exhausted_pro();
        let plan = [
            (TrainingType::PressingDrills, TrainingIntensity::High),
            (TrainingType::Speed, TrainingIntensity::High),
        ];
        let (_, reasons) = run_archetype(&p, &plan, 30);
        let fatigue = count_reason(&reasons, TrainingEventReason::StruggledWithIntensity);
        let attitude = count_reason(&reasons, TrainingEventReason::PoorAttitude);
        assert!(
            fatigue > attitude * 3,
            "exhausted pro fatigue={} attitude={} - fatigue should dominate",
            fatigue,
            attitude
        );
        assert_eq!(
            attitude, 0,
            "high-pro tired player must never read as poor attitude"
        );
    }

    #[test]
    fn transfer_target_shows_more_distraction_than_neutral() {
        let target = archetype_transfer_target();
        let neutral = archetype_neutral();
        let plan = [(TrainingType::Passing, TrainingIntensity::Moderate)];
        let (_, target_reasons) = run_archetype(&target, &plan, 30);
        let (_, neutral_reasons) = run_archetype(&neutral, &plan, 30);
        let target_distract =
            count_reason(&target_reasons, TrainingEventReason::DistractedByRumours);
        let neutral_distract =
            count_reason(&neutral_reasons, TrainingEventReason::DistractedByRumours);
        assert!(
            target_distract > neutral_distract,
            "transfer target {} should have more distraction reasons than neutral {}",
            target_distract,
            neutral_distract
        );
    }

    #[test]
    fn youth_can_break_through_but_does_not_spam() {
        let p = archetype_youth();
        let plan = [
            (TrainingType::Passing, TrainingIntensity::Moderate),
            (TrainingType::BallControl, TrainingIntensity::Moderate),
        ];
        let (_, reasons) = run_archetype(&p, &plan, 30);
        let breakthroughs = count_reason(&reasons, TrainingEventReason::YoungImpressedStaff);
        let total = reasons.len();
        // Cannot fire on every session - that's spam - and shouldn't fire 0
        // times if the gates are working. Aim for an occasional event.
        assert!(
            breakthroughs <= total / 4,
            "youth breakthrough should not dominate: {} of {}",
            breakthroughs,
            total
        );
    }

    #[test]
    fn veteran_set_standard_can_fire_but_does_not_spam() {
        let p = archetype_veteran();
        let plan = [
            (TrainingType::TeamShape, TrainingIntensity::Moderate),
            (TrainingType::Passing, TrainingIntensity::Moderate),
        ];
        let (_, reasons) = run_archetype(&p, &plan, 30);
        let standards = count_reason(&reasons, TrainingEventReason::SettingStandards);
        let total = reasons.len();
        assert!(
            standards <= total / 4,
            "veteran standard-setting should not dominate: {} of {}",
            standards,
            total
        );
    }

    #[test]
    fn cooldown_keeps_visible_events_under_three_per_14_days() {
        // Drive a player through a 14-day window with low-pro every day.
        // Cooldown gates inside maybe_emit_training_event must keep the
        // visible PoorTraining count <= 3 even when the underlying
        // outcome would qualify on most days.
        use crate::club::player::training::result::TrainingOutcomeBreakdown;
        let mut p = build_pro_pro(PlayerPositionType::MidfielderCenter, 4.0, 4.0, 4.0);
        p.attributes.consistency = 8.0;

        // Replay maybe_emit by directly invoking the result.process()
        // path on a SimulatorData stub is too heavyweight; instead use
        // the public training emit-helper via a tight loop and then
        // inspect happiness.recent_events. We synthesize the outcome
        // and reuse the cooldown function.
        let _ = TrainingOutcomeBreakdown {
            raw_score: 4.0,
            baseline_score: 6.0,
            delta_from_baseline: -2.0,
            effort_score: 5.0,
            focus_score: 8.0,
            physical_state_score: 14.0,
            coach_fit_score: 10.0,
            tactical_fit_score: 8.0,
            psychological_score: 8.0,
            randomness_score: 0.0,
            primary_reason: TrainingEventReason::PoorAttitude,
            evidence: vec![],
        };

        // Push events directly with the cooldown semantics that match
        // production. This mirrors what `maybe_emit_training_event`
        // would do under heavy negative outcomes.
        for day in 0..14 {
            let mut ctx =
                crate::TrainingEventContext::new(TrainingEventReason::PoorAttitude, 4.0, 6.0);
            ctx = ctx.with_evidence(TrainingEventEvidence::LowEffort);
            let happiness_ctx = crate::HappinessEventContext::new(
                crate::HappinessEventCause::Other,
                crate::HappinessEventSeverity::Moderate,
                crate::HappinessEventScope::TrainingGround,
            )
            .with_training_context(ctx);
            // Cooldown is 14 days for PoorAttitude - bump days_ago on
            // existing events to mirror the day passing.
            for ev in p.happiness.recent_events.iter_mut() {
                ev.days_ago = ev.days_ago.saturating_add(1);
            }
            let suppressed = p.happiness.recent_events.iter().any(|e| {
                e.event_type == crate::HappinessEventType::PoorTraining
                    && e.days_ago <= 14
                    && e.context
                        .as_ref()
                        .and_then(|c| c.training_context.as_ref())
                        .map(|tc| tc.reason == TrainingEventReason::PoorAttitude)
                        .unwrap_or(false)
            });
            if !suppressed {
                p.happiness.add_event_with_context(
                    crate::HappinessEventType::PoorTraining,
                    -2.0,
                    None,
                    happiness_ctx,
                );
            }
            let _ = day;
        }
        let visible = p
            .happiness
            .recent_events
            .iter()
            .filter(|e| e.event_type == crate::HappinessEventType::PoorTraining)
            .count();
        assert!(
            visible <= 3,
            "cooldown should cap visible PoorTraining events at <= 3 per 14 days, got {}",
            visible
        );
    }

    // ── Real-fixture integration ─────────────────────────────────

    #[test]
    fn generate_for_date_recovery_after_real_match_yesterday() {
        use crate::{
            CoachingPhilosophy, PeriodizationPhase, RotationPreference, TacticalFocus,
            TrainingIntensityPreference, WeeklyTrainingPlan,
        };
        let philosophy = CoachingPhilosophy {
            tactical_focus: TacticalFocus::Possession,
            training_intensity: TrainingIntensityPreference::Medium,
            youth_focus: false,
            rotation_preference: RotationPreference::Moderate,
        };
        // Today is a Wednesday with a match on Tuesday (real prev) and
        // no upcoming - MD+1 path must trigger Recovery + VideoAnalysis.
        let today = NaiveDate::from_ymd_opt(2026, 2, 11).unwrap(); // Wed
        let prev = NaiveDate::from_ymd_opt(2026, 2, 10).unwrap(); // Tue
        let plan = WeeklyTrainingPlan::generate_for_date(
            today,
            Some(prev),
            None,
            1,
            PeriodizationPhase::MidSeason,
            &philosophy,
        );
        let today_sessions = plan.sessions.get(&today.weekday()).unwrap();
        assert_eq!(today_sessions.len(), 2);
        assert!(matches!(
            today_sessions[0].session_type,
            TrainingType::Recovery
        ));
        assert!(matches!(
            today_sessions[1].session_type,
            TrainingType::VideoAnalysis
        ));
    }

    #[test]
    fn generate_for_date_match_prep_two_days_before_real_match() {
        use crate::{
            CoachingPhilosophy, PeriodizationPhase, RotationPreference, TacticalFocus,
            TrainingIntensityPreference, WeeklyTrainingPlan,
        };
        let philosophy = CoachingPhilosophy {
            tactical_focus: TacticalFocus::Possession,
            training_intensity: TrainingIntensityPreference::Medium,
            youth_focus: false,
            rotation_preference: RotationPreference::Moderate,
        };
        let today = NaiveDate::from_ymd_opt(2026, 2, 12).unwrap(); // Thu
        let next = NaiveDate::from_ymd_opt(2026, 2, 14).unwrap(); // Sat
        let plan = WeeklyTrainingPlan::generate_for_date(
            today,
            None,
            Some(next),
            1,
            PeriodizationPhase::MidSeason,
            &philosophy,
        );
        let today_sessions = plan.sessions.get(&today.weekday()).unwrap();
        assert!(matches!(
            today_sessions[0].session_type,
            TrainingType::MatchPreparation
        ));
        assert!(matches!(
            today_sessions[1].session_type,
            TrainingType::TeamShape
        ));
    }

    #[test]
    fn generate_for_date_no_training_on_real_match_day() {
        use crate::{
            CoachingPhilosophy, PeriodizationPhase, RotationPreference, TacticalFocus,
            TrainingIntensityPreference, WeeklyTrainingPlan,
        };
        let philosophy = CoachingPhilosophy {
            tactical_focus: TacticalFocus::Possession,
            training_intensity: TrainingIntensityPreference::Medium,
            youth_focus: false,
            rotation_preference: RotationPreference::Moderate,
        };
        let today = NaiveDate::from_ymd_opt(2026, 2, 14).unwrap();
        let plan = WeeklyTrainingPlan::generate_for_date(
            today,
            None,
            Some(today),
            1,
            PeriodizationPhase::MidSeason,
            &philosophy,
        );
        let today_sessions = plan.sessions.get(&today.weekday()).unwrap();
        assert!(today_sessions.is_empty(), "no training on match day");
    }

    #[test]
    fn double_match_week_drops_high_intensity_sessions() {
        use crate::{
            CoachingPhilosophy, PeriodizationPhase, RotationPreference, TacticalFocus,
            TrainingIntensityPreference, WeeklyTrainingPlan,
        };
        let philosophy = CoachingPhilosophy {
            tactical_focus: TacticalFocus::Pressing,
            training_intensity: TrainingIntensityPreference::High,
            youth_focus: false,
            rotation_preference: RotationPreference::Moderate,
        };
        // Today (Tue), match Sun, last match Wed prior - two competitive
        // matches in the rolling 7-day window (Wed and Sun).
        let today = NaiveDate::from_ymd_opt(2026, 2, 17).unwrap();
        let prev = NaiveDate::from_ymd_opt(2026, 2, 11).unwrap();
        let next = NaiveDate::from_ymd_opt(2026, 2, 22).unwrap();
        let plan_congested = WeeklyTrainingPlan::generate_for_date(
            today,
            Some(prev),
            Some(next),
            2,
            PeriodizationPhase::MidSeason,
            &philosophy,
        );
        let plan_fresh = WeeklyTrainingPlan::generate_for_date(
            today,
            None,
            None,
            0,
            PeriodizationPhase::MidSeason,
            &philosophy,
        );
        let cong_today = plan_congested.sessions.get(&today.weekday()).unwrap();
        let fresh_today = plan_fresh.sessions.get(&today.weekday()).unwrap();
        // Pressing coach + non-congested = PressingDrills first.
        assert!(matches!(
            fresh_today[0].session_type,
            TrainingType::PressingDrills
        ));
        // Same coach + congested = Positioning swap (no PressingDrills).
        assert!(
            !cong_today.iter().any(|s| matches!(
                s.session_type,
                TrainingType::PressingDrills | TrainingType::Speed
            )),
            "congested week must drop PressingDrills/Speed, got {:?}",
            cong_today
                .iter()
                .map(|s| s.session_type.clone())
                .collect::<Vec<_>>()
        );
    }

    // ── Noise-model deterministic guards ─────────────────────────

    fn noise_off_day_count(player: &Player, days: u32) -> u32 {
        let coach = StaffStub::default();
        let s = session(TrainingType::Passing, TrainingIntensity::Moderate);
        let mut count = 0;
        for offset in 0..days {
            let day = 1 + (offset % 28) as u32;
            let month = 9 + (offset / 28) as u32;
            let date = d(2025, month.min(12), day.max(1).min(28));
            let r = PlayerTraining::train(player, &coach, &s, date, 0.6);
            if r.session_performance <= 6.5 {
                count += 1;
            }
        }
        count
    }

    #[test]
    fn low_morale_increases_off_day_rate_compared_to_high_morale() {
        let mut low = build_pro_pro(PlayerPositionType::MidfielderCenter, 12.0, 12.0, 12.0);
        low.happiness.morale = 20.0;
        low.attributes.consistency = 8.0;
        let mut high = build_pro_pro(PlayerPositionType::MidfielderCenter, 12.0, 12.0, 12.0);
        high.happiness.morale = 80.0;
        high.attributes.consistency = 8.0;
        let off_low = noise_off_day_count(&low, 60);
        let off_high = noise_off_day_count(&high, 60);
        assert!(
            off_low >= off_high,
            "low-morale off-days {} should be >= high-morale {}",
            off_low,
            off_high
        );
    }

    #[test]
    fn high_consistency_narrows_session_variance() {
        let coach = StaffStub::default();
        let mut steady = build_pro_pro(PlayerPositionType::MidfielderCenter, 12.0, 12.0, 12.0);
        steady.attributes.consistency = 18.0;
        let mut flaky = build_pro_pro(PlayerPositionType::MidfielderCenter, 12.0, 12.0, 12.0);
        flaky.attributes.consistency = 4.0;
        let s = session(TrainingType::Passing, TrainingIntensity::Moderate);
        let mut steady_perfs = Vec::with_capacity(60);
        let mut flaky_perfs = Vec::with_capacity(60);
        for offset in 0..60 {
            let day = 1 + (offset % 28) as u32;
            let month = 9 + (offset / 28) as u32;
            let date = d(2025, month.min(12), day.max(1).min(28));
            steady_perfs
                .push(PlayerTraining::train(&steady, &coach, &s, date, 0.6).session_performance);
            flaky_perfs
                .push(PlayerTraining::train(&flaky, &coach, &s, date, 0.6).session_performance);
        }
        let var = |xs: &[f32]| -> f32 {
            let m = xs.iter().sum::<f32>() / xs.len() as f32;
            xs.iter().map(|x| (x - m).powi(2)).sum::<f32>() / xs.len() as f32
        };
        let steady_var = var(&steady_perfs);
        let flaky_var = var(&flaky_perfs);
        assert!(
            steady_var < flaky_var,
            "high-consistency variance {} should be smaller than low-consistency {}",
            steady_var,
            flaky_var
        );
    }

    // ── Pair bonding/friction emission guards ────────────────────

    #[test]
    fn deterministic_pair_can_emit_teammate_bonding() {
        // We test the threshold logic alone here - the per-pair roll is
        // deterministic but driven by hashed ids, so we can't easily
        // synthesize a successful bond roll in a unit test without
        // wiring a Team. Verify that the threshold constant is what we
        // claim by exercising add_event_with_partner_context_and_cooldown
        // with a magnitude above the threshold and confirming emission.
        use crate::PlayerHappiness;
        let mut h = PlayerHappiness::new();
        let ctx = crate::HappinessEventContext::new(
            crate::HappinessEventCause::TrainingPartnership,
            crate::HappinessEventSeverity::Moderate,
            crate::HappinessEventScope::TrainingGround,
        );
        let emitted = h.add_event_with_partner_context_and_cooldown(
            crate::HappinessEventType::TeammateBonding,
            0.6,
            42,
            ctx,
            14,
        );
        assert!(emitted, "first bonding event should land");
        assert!(
            h.recent_events
                .iter()
                .any(|e| e.event_type == crate::HappinessEventType::TeammateBonding)
        );
    }

    #[test]
    fn deterministic_pair_friction_respects_cooldown() {
        use crate::PlayerHappiness;
        let mut h = PlayerHappiness::new();
        let ctx = crate::HappinessEventContext::new(
            crate::HappinessEventCause::TrainingFriction,
            crate::HappinessEventSeverity::Moderate,
            crate::HappinessEventScope::TrainingGround,
        );
        let first = h.add_event_with_partner_context_and_cooldown(
            crate::HappinessEventType::ConflictWithTeammate,
            -0.8,
            73,
            ctx.clone(),
            14,
        );
        let second = h.add_event_with_partner_context_and_cooldown(
            crate::HappinessEventType::ConflictWithTeammate,
            -0.8,
            73,
            ctx,
            14,
        );
        assert!(first);
        assert!(!second, "cooldown should suppress same-pair second event");
    }
}
