use crate::club::player::training::result::PlayerTrainingResult;
use crate::{
    ChangeType, Player, PlayerFieldPositionGroup, PlayerPositionType, PlayerTraining,
    RelationshipChange, Staff, Team, TeamTrainingResult,
};
use chrono::{Datelike, NaiveDateTime, Weekday};
use std::collections::HashMap;

/// Deterministic pseudo-random roll in `[0.0, 1.0)` for an unordered pair
/// of player ids on a given date. Same pair + date always returns the same
/// number — keeps weekly tests stable. The `salt` parameter lets us run
/// independent bond / friction rolls that don't collide.
fn pair_roll(a: u32, b: u32, salt: u32, date: chrono::NaiveDate) -> f32 {
    let (lo, hi) = if a < b { (a, b) } else { (b, a) };
    // Cheap hash — wrapping multiplication on a couple of large primes.
    // Determinism over cryptographic strength is what we need here.
    let h = (lo as u64)
        .wrapping_mul(0x9E37_79B9_7F4A_7C15)
        .wrapping_add((hi as u64).wrapping_mul(0xC6BC_279E_9286_5A2B))
        .wrapping_add(salt as u64)
        .wrapping_add(date.num_days_from_ce() as u64);
    let frac = ((h >> 11) as u32 as f32) / (u32::MAX as f32);
    frac.clamp(0.0, 0.999)
}

/// Returns true when a recovering (Lmp) player can safely take part in
/// the given session. Heavy physical or high-tempo tactical work is
/// excluded — those need full match fitness, not rehab fitness.
fn is_recovery_compatible_session(session_type: &TrainingType) -> bool {
    matches!(
        session_type,
        TrainingType::Recovery
            | TrainingType::LightRecovery
            | TrainingType::Rehabilitation
            | TrainingType::RestDay
            | TrainingType::VideoAnalysis
            | TrainingType::Positioning
    )
}

#[derive(Debug, Clone)]
pub struct TeamTraining;

impl TeamTraining {
    pub fn train(
        team: &mut Team,
        date: NaiveDateTime,
        facility_quality: f32,
    ) -> TeamTrainingResult {
        Self::train_with_facilities(team, date, facility_quality)
    }

    fn train_with_facilities(
        team: &mut Team,
        date: NaiveDateTime,
        facility_quality: f32,
    ) -> TeamTrainingResult {
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
                let session_results =
                    Self::execute_training_session(team, coach, session, date, facility_quality);
                result.player_results.extend(session_results);
            }
        }

        // Apply team cohesion effects
        Self::apply_team_cohesion_effects(team, &result, date.date());

        result
    }

    fn execute_training_session(
        team: &Team,
        coach: &Staff,
        session: &TrainingSession,
        date: NaiveDateTime,
        facility_quality: f32,
    ) -> Vec<PlayerTrainingResult> {
        let participants = Self::select_participants(team, session);

        let mut results = Vec::with_capacity(participants.len());

        for player in participants {
            let mut r = PlayerTraining::train(player, coach, session, date, facility_quality);
            // Chemistry multiplier: a player happy in the dressing room
            // gets 10% more out of a session; one in a toxic one gets 10%
            // less. Narrow band so chemistry is meaningful without being
            // decisive — coach quality and intensity still dominate.
            let chem = player.relations.get_team_chemistry() / 100.0; // 0..1
            let factor = 0.90 + chem * 0.20;
            r.effects.scale_gains(factor);
            results.push(r);
        }

        results
    }

    /// Credit the coach with specialization days for each participant's
    /// position group. Called after training, so the coach develops deep
    /// expertise over time in whichever groups they spend most sessions on.
    fn accrue_coach_specialization(team: &mut Team, coach_id: u32, participant_ids: &[u32]) {
        // Build a multiset of groups trained this session.
        let mut group_counts: [u32; 4] = [0; 4];
        for pid in participant_ids {
            if let Some(player) = team.players.find(*pid) {
                let group = player.position().position_group();
                let idx = match group {
                    PlayerFieldPositionGroup::Goalkeeper => 0,
                    PlayerFieldPositionGroup::Defender => 1,
                    PlayerFieldPositionGroup::Midfielder => 2,
                    PlayerFieldPositionGroup::Forward => 3,
                };
                group_counts[idx] += 1;
            }
        }
        // Credit the coach with ONE specialization day for each group that
        // had participants. Multiple players don't double-count — what
        // matters is whether the coach ran that group today.
        if let Some(coach) = team.staffs.find_mut(coach_id) {
            if group_counts[0] > 0 {
                coach.accrue_specialization(PlayerFieldPositionGroup::Goalkeeper, 1);
            }
            if group_counts[1] > 0 {
                coach.accrue_specialization(PlayerFieldPositionGroup::Defender, 1);
            }
            if group_counts[2] > 0 {
                coach.accrue_specialization(PlayerFieldPositionGroup::Midfielder, 1);
            }
            if group_counts[3] > 0 {
                coach.accrue_specialization(PlayerFieldPositionGroup::Forward, 1);
            }
        }
    }

    fn select_participants<'a>(team: &'a Team, session: &TrainingSession) -> Vec<&'a Player> {
        let mut participants = Vec::new();

        // If specific participants are listed, use those
        if !session.participants.is_empty() {
            for player_id in &session.participants {
                if let Some(player) = team.players.find(*player_id) {
                    if Self::can_participate(player, session) {
                        participants.push(player);
                    }
                }
            }
        } else if !session.focus_positions.is_empty() {
            // Select players based on focus positions
            for player in team.players.iter() {
                if Self::can_participate(player, session) {
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
            for player in team.players.iter() {
                if Self::can_participate(player, session) {
                    participants.push(player);
                }
            }
        }

        participants
    }

    fn can_participate(player: &Player, session: &TrainingSession) -> bool {
        if player.player_attributes.is_injured || player.player_attributes.is_banned {
            return false;
        }
        if player.player_attributes.condition_percentage() <= 30 {
            return false;
        }
        // Returning-from-injury (Lmp) players only join light /
        // rehabilitation sessions. Throwing them straight into a
        // pressing drill is the canonical way to reaggravate the
        // injury — not how a real fitness coach manages a return.
        if player.player_attributes.is_in_recovery()
            && !is_recovery_compatible_session(&session.session_type)
        {
            return false;
        }
        true
    }

    fn apply_team_cohesion_effects(
        team: &mut Team,
        training_results: &TeamTrainingResult,
        sim_date: chrono::NaiveDate,
    ) {
        // Players training together build relationships
        let participant_ids: Vec<u32> = training_results
            .player_results
            .iter()
            .map(|r| r.player_id)
            .collect();

        // Per-pair bonding/friction. The old "+0.01 across the board" was
        // unrealistic — striker rivals don't bond at the same rate as a
        // mentor pair. We compute a deterministic bond/friction chance per
        // pair from shared language, position group, mentorship, morale,
        // and personality, then apply a meaningful relation magnitude only
        // when the chance passes a per-pair pseudo-random threshold.
        // Determinism: chance comparison uses a seeded function of the two
        // ids so tests stay stable across runs.
        Self::apply_pairwise_bonding(team, &participant_ids, training_results, sim_date);

        // Coach-player relationship updates based on training quality
        let coach_id = team.staffs.head_coach().id;
        let coach_effectiveness = team
            .staffs
            .head_coach()
            .recent_performance
            .training_effectiveness;

        // Rapport accrual: every participant spent a day with the coach.
        // Small positive drift + shared_days increment, even if no other
        // events fire this tick.
        for &player_id in &participant_ids {
            if let Some(player) = team.players.find_mut(player_id) {
                player.rapport.accrue_training_day(coach_id, sim_date, 1);
            }
        }

        // Coach specialization: credit the coach with one day per position
        // group covered in this session. Over hundreds of sessions a coach
        // organically becomes a "midfield specialist" or "striker clinic"
        // without any manual assignment.
        Self::accrue_coach_specialization(team, coach_id, &participant_ids);

        // Calculate average morale change across training results
        let total_morale: f32 = training_results
            .player_results
            .iter()
            .map(|r| r.effects.morale_change)
            .sum();
        let avg_morale = if !training_results.player_results.is_empty() {
            total_morale / training_results.player_results.len() as f32
        } else {
            0.0
        };

        // Positive training session: update coach-player relationships
        if avg_morale > 0.0 {
            let relationship_boost = 0.01 + coach_effectiveness * 0.02; // 0.01 to 0.03

            for &player_id in &participant_ids {
                if let Some(player) = team.players.find_mut(player_id) {
                    let change = RelationshipChange::positive(
                        ChangeType::CoachingSuccess,
                        relationship_boost,
                    );
                    player
                        .relations
                        .update_staff_relationship(coach_id, change, sim_date);
                }
            }
        }
    }

    /// Pair-wise training bonding/friction. Replaces the universal +0.01
    /// nudge with a richer model: shared language, position group, mentor
    /// pair, professionalism, and morale all influence bond chance; the
    /// flip side fires friction for direct rivals or low-professionalism
    /// pairs. Magnitude is non-trivial when an event lands so chemistry
    /// actually moves — but we gate event emission so the player history
    /// only sees meaningful bonds.
    fn apply_pairwise_bonding(
        team: &mut Team,
        participant_ids: &[u32],
        training_results: &TeamTrainingResult,
        sim_date: chrono::NaiveDate,
    ) {
        use crate::HappinessEventType;

        // Index morale changes by player id so we don't iterate the result
        // vector once per pair.
        let mut morale_change: HashMap<u32, f32> =
            HashMap::with_capacity(training_results.player_results.len());
        for r in &training_results.player_results {
            morale_change.insert(r.player_id, r.effects.morale_change);
        }

        // Snapshot per-player bonding inputs (immutable) so we can apply
        // mutations afterwards without aliasing the player borrow.
        struct Snap {
            id: u32,
            position_group: crate::PlayerFieldPositionGroup,
            primary_lang: Option<crate::club::player::language::Language>,
            languages: Vec<(crate::club::player::language::Language, u8)>,
            morale: f32,
            controversy: f32,
            professionalism: f32,
            mentor_target: Option<u32>,
        }

        let snaps: Vec<Snap> = participant_ids
            .iter()
            .filter_map(|id| {
                let p = team.players.find(*id)?;
                let group = p.position().position_group();
                let primary_lang = p
                    .languages
                    .iter()
                    .find(|l| l.is_native)
                    .or_else(|| p.languages.iter().max_by_key(|l| l.proficiency))
                    .map(|l| l.language);
                let languages: Vec<_> = p
                    .languages
                    .iter()
                    .filter(|l| l.is_native || l.proficiency >= 60)
                    .map(|l| (l.language, l.proficiency))
                    .collect();
                let mentor_target =
                    p.relations
                        .player_relations_iter()
                        .find_map(|(other_id, rel)| {
                            if rel.mentorship.is_some() {
                                Some(*other_id)
                            } else {
                                None
                            }
                        });
                Some(Snap {
                    id: *id,
                    position_group: group,
                    primary_lang,
                    languages,
                    morale: p.happiness.morale,
                    controversy: p.attributes.controversy,
                    professionalism: p.attributes.professionalism,
                    mentor_target,
                })
            })
            .collect();

        // Pairwise pass — collect changes first, then apply, so we don't
        // hold a long mutable borrow on `team`.
        struct Effect {
            from: u32,
            to: u32,
            relation_change: f32, // signed magnitude on the level axis
            bond: bool,
        }
        let mut effects: Vec<Effect> = Vec::new();

        for i in 0..snaps.len() {
            for j in (i + 1)..snaps.len() {
                let a = &snaps[i];
                let b = &snaps[j];
                let same_group = a.position_group == b.position_group;
                let direct_rivals = same_group;
                // Shared language if either side has the other side's
                // primary language at proficiency ≥ 60 (or native).
                let shared_language = match (a.primary_lang, b.primary_lang) {
                    (Some(la), Some(lb)) if la == lb => true,
                    (Some(la), _) => b.languages.iter().any(|(l, _)| *l == la),
                    (_, Some(lb)) => a.languages.iter().any(|(l, _)| *l == lb),
                    _ => false,
                };
                let mentor_pair = a.mentor_target == Some(b.id) || b.mentor_target == Some(a.id);
                let both_pro = a.professionalism >= 14.0 && b.professionalism >= 14.0;
                let avg_session_morale = (morale_change.get(&a.id).copied().unwrap_or(0.0)
                    + morale_change.get(&b.id).copied().unwrap_or(0.0))
                    / 2.0;
                let positive_session = avg_session_morale > 0.0;
                let either_low_morale = a.morale < 35.0 || b.morale < 35.0;
                let either_high_controversy = a.controversy > 14.0 || b.controversy > 14.0;
                let either_low_pro = a.professionalism < 8.0 || b.professionalism < 8.0;

                // Bond chance build-up.
                let mut bond_chance = 0.03;
                if same_group {
                    bond_chance += 0.04;
                }
                if shared_language {
                    bond_chance += 0.05;
                }
                if mentor_pair {
                    bond_chance += 0.08;
                }
                if both_pro {
                    bond_chance += 0.03;
                }
                if positive_session {
                    bond_chance += 0.04;
                }
                if direct_rivals {
                    bond_chance -= 0.04;
                }
                if either_low_morale {
                    bond_chance -= 0.03;
                }
                if either_high_controversy {
                    bond_chance -= 0.03;
                }

                // Friction chance.
                let mut friction_chance = 0.01;
                if direct_rivals {
                    friction_chance += 0.04;
                }
                if either_low_pro {
                    friction_chance += 0.03;
                }
                if either_high_controversy {
                    friction_chance += 0.03;
                }

                // Deterministic per-pair "roll" — same hash twice in the
                // same week resolves identically. Independent rolls for
                // bond and friction so the same pair can't trigger both.
                let bond_roll = pair_roll(a.id, b.id, 0xB0_4D, sim_date);
                let friction_roll = pair_roll(a.id, b.id, 0xF1_2A, sim_date);

                if bond_chance > 0.0 && bond_roll < bond_chance {
                    let mut mag: f32 = 0.08;
                    if mentor_pair {
                        mag += 0.10;
                    }
                    if shared_language {
                        mag += 0.05;
                    }
                    mag = mag.clamp(0.08, 0.25);
                    effects.push(Effect {
                        from: a.id,
                        to: b.id,
                        relation_change: mag,
                        bond: true,
                    });
                    effects.push(Effect {
                        from: b.id,
                        to: a.id,
                        relation_change: mag,
                        bond: true,
                    });
                } else if friction_chance > 0.0 && friction_roll < friction_chance {
                    let mut mag: f32 = 0.10;
                    if direct_rivals {
                        mag += 0.08;
                    }
                    if either_low_pro {
                        mag += 0.07;
                    }
                    if either_high_controversy {
                        mag += 0.05;
                    }
                    mag = mag.clamp(0.10, 0.35);
                    effects.push(Effect {
                        from: a.id,
                        to: b.id,
                        relation_change: -mag,
                        bond: false,
                    });
                    effects.push(Effect {
                        from: b.id,
                        to: a.id,
                        relation_change: -mag,
                        bond: false,
                    });
                }
            }
        }

        // Apply effects. Event emission only on magnitudes ≥ 0.5 to keep
        // the player history readable — small drifts stay silent.
        for eff in effects {
            if let Some(player) = team.players.find_mut(eff.from) {
                let change_type = if eff.bond {
                    crate::ChangeType::TrainingBonding
                } else {
                    crate::ChangeType::TrainingFriction
                };
                player.relations.update_with_type(
                    eff.to,
                    eff.relation_change,
                    change_type,
                    sim_date,
                );

                if eff.relation_change.abs() >= 0.5 {
                    if eff.bond {
                        player.happiness.add_event_with_partner(
                            HappinessEventType::TeammateBonding,
                            0.6,
                            Some(eff.to),
                        );
                    } else {
                        player.happiness.add_event_with_partner(
                            HappinessEventType::ConflictWithTeammate,
                            -0.8,
                            Some(eff.to),
                        );
                    }
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

    fn get_next_match_day(_team: &Team, _date: NaiveDateTime) -> Option<Weekday> {
        // This would check the actual match schedule
        // For now, assume Saturday matches
        Some(Weekday::Sat)
    }

    fn get_previous_match_day(_team: &Team, _date: NaiveDateTime) -> Option<Weekday> {
        // This would check the actual match history
        // For now, return None
        None
    }

    fn get_coach_philosophy(coach: &Staff) -> CoachingPhilosophy {
        // Determine coach philosophy based on attributes
        let tactical_focus = if coach.staff_attributes.coaching.attacking
            > coach.staff_attributes.coaching.defending
        {
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

#[derive(Debug, Clone, PartialEq, Default)]
pub enum TrainingType {
    // Physical Training
    #[default]
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
    VeryLight, // 20-40% max effort - recovery sessions
    Light,     // 40-60% max effort - technical work
    Moderate,  // 60-75% max effort - standard training
    High,      // 75-90% max effort - intense sessions
    VeryHigh,  // 90-100% max effort - match simulation
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
    PreSeason,   // High volume, building fitness
    EarlySeason, // Balancing fitness and tactics
    MidSeason,   // Maintenance and tactical focus
    LateSeason,  // Managing fatigue, focus on recovery
    OffSeason,   // Rest and light maintenance
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
        sessions.insert(
            Weekday::Mon,
            Self::monday_sessions(previous_match_day, phase),
        );

        // Tuesday - Main training day
        sessions.insert(
            Weekday::Tue,
            Self::tuesday_sessions(phase, coach_philosophy),
        );

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

    fn monday_sessions(
        previous_match: Option<Weekday>,
        _phase: PeriodizationPhase,
    ) -> Vec<TrainingSession> {
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
                },
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
                },
            ]
        }
    }

    fn tuesday_sessions(
        phase: PeriodizationPhase,
        philosophy: &CoachingPhilosophy,
    ) -> Vec<TrainingSession> {
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
            },
        ]
    }

    fn wednesday_sessions(_phase: PeriodizationPhase) -> Vec<TrainingSession> {
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
                focus_positions: vec![
                    PlayerPositionType::Striker,
                    PlayerPositionType::ForwardCenter,
                ],
                participants: vec![],
            },
        ]
    }

    fn thursday_sessions(
        next_match: Option<Weekday>,
        _phase: PeriodizationPhase,
    ) -> Vec<TrainingSession> {
        if next_match == Some(Weekday::Sat) {
            // Light preparation for Saturday match
            vec![TrainingSession {
                session_type: TrainingType::MatchPreparation,
                intensity: TrainingIntensity::Light,
                duration_minutes: 60,
                focus_positions: vec![],
                participants: vec![],
            }]
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
                },
            ]
        }
    }

    fn friday_sessions(
        next_match: Option<Weekday>,
        _phase: PeriodizationPhase,
    ) -> Vec<TrainingSession> {
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
                },
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
                },
            ]
        }
    }

    fn saturday_sessions(next_match: Option<Weekday>) -> Vec<TrainingSession> {
        if next_match == Some(Weekday::Sat) {
            vec![] // Match day
        } else {
            vec![TrainingSession {
                session_type: TrainingType::MatchPreparation,
                intensity: TrainingIntensity::High,
                duration_minutes: 90,
                focus_positions: vec![],
                participants: vec![],
            }]
        }
    }

    fn sunday_sessions(next_match: Option<Weekday>) -> Vec<TrainingSession> {
        if next_match == Some(Weekday::Sun) {
            vec![] // Match day
        } else {
            vec![TrainingSession {
                session_type: TrainingType::RestDay,
                intensity: TrainingIntensity::VeryLight,
                duration_minutes: 0,
                focus_positions: vec![],
                participants: vec![],
            }]
        }
    }
}

// ============== Training Effects System ==============

#[derive(Debug, Clone)]
pub struct TrainingEffects {
    pub physical_gains: PhysicalGains,
    pub technical_gains: TechnicalGains,
    pub mental_gains: MentalGains,
    /// Net change to in-match condition. Positive = costs condition,
    /// negative = recovery. Applied after clamping.
    pub fatigue_change: f32,
    pub injury_risk: f32,
    pub morale_change: f32,
    /// Physical-load units booked into `PlayerLoad`. A heavy match-prep
    /// or pressing drill ≈ 30-40 units; passive video session = 0.
    pub physical_load_units: f32,
    /// Share of the load that's high-intensity (0.0..1.0).
    pub high_intensity_share: f32,
    /// Match-readiness delta. Replaces the old "any negative fatigue
    /// gives +2 sharpness" rule — passive recovery now gains nothing,
    /// real match-tempo work gains a lot.
    pub readiness_change: f32,
}

impl TrainingEffects {
    /// Multiply all skill gains by `factor`. Fatigue / injury_risk /
    /// morale are left alone — they're driven by intensity & session
    /// type, not dressing-room chemistry.
    pub fn scale_gains(&mut self, factor: f32) {
        let f = factor.max(0.0);
        let p = &mut self.physical_gains;
        p.stamina *= f;
        p.strength *= f;
        p.pace *= f;
        p.agility *= f;
        p.balance *= f;
        p.jumping *= f;
        p.natural_fitness *= f;
        let t = &mut self.technical_gains;
        t.first_touch *= f;
        t.passing *= f;
        t.crossing *= f;
        t.dribbling *= f;
        t.finishing *= f;
        t.heading *= f;
        t.tackling *= f;
        t.technique *= f;
        let m = &mut self.mental_gains;
        m.concentration *= f;
        m.decisions *= f;
        m.positioning *= f;
        m.teamwork *= f;
        m.vision *= f;
        m.work_rate *= f;
        m.leadership *= f;
    }
}

#[derive(Debug, Clone, Default)]
pub struct PhysicalGains {
    pub stamina: f32,
    pub strength: f32,
    pub pace: f32,
    pub agility: f32,
    pub balance: f32,
    pub jumping: f32,
    pub natural_fitness: f32,
}

impl PhysicalGains {
    pub fn total(&self) -> f32 {
        self.stamina
            + self.strength
            + self.pace
            + self.agility
            + self.balance
            + self.jumping
            + self.natural_fitness
    }
}

#[derive(Debug, Clone, Default)]
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

impl TechnicalGains {
    pub fn total(&self) -> f32 {
        self.first_touch
            + self.passing
            + self.crossing
            + self.dribbling
            + self.finishing
            + self.heading
            + self.tackling
            + self.technique
    }
}

#[derive(Debug, Clone, Default)]
pub struct MentalGains {
    pub concentration: f32,
    pub decisions: f32,
    pub positioning: f32,
    pub teamwork: f32,
    pub vision: f32,
    pub work_rate: f32,
    pub leadership: f32,
}

impl MentalGains {
    pub fn total(&self) -> f32 {
        self.concentration
            + self.decisions
            + self.positioning
            + self.teamwork
            + self.vision
            + self.work_rate
            + self.leadership
    }
}

// ============== Individual Player Training Plans ==============

#[derive(Debug, Clone)]
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

#[derive(Debug, Clone)]
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

#[derive(Debug, Clone)]
pub struct TrainingLoadManager {
    pub player_loads: HashMap<u32, PlayerTrainingLoad>,
}

#[derive(Debug, Clone)]
pub struct PlayerTrainingLoad {
    pub acute_load: f32,   // Last 7 days
    pub chronic_load: f32, // Last 28 days
    pub load_ratio: f32,   // Acute/Chronic ratio
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

    pub fn update_load(
        &mut self,
        session_load: f32,
        intensity: &TrainingIntensity,
        date: NaiveDateTime,
    ) {
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
        if matches!(
            intensity,
            TrainingIntensity::High | TrainingIntensity::VeryHigh
        ) {
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
        self.cumulative_fatigue > 75.0 || self.load_ratio > 1.5 || self.sessions_this_week >= 6
    }

    pub fn weekly_reset(&mut self) {
        self.sessions_this_week = 0;
        self.cumulative_fatigue *= 0.7; // Partial recovery
    }
}
