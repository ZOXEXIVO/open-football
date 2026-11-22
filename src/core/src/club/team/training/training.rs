use crate::club::player::development::SessionWork;
use crate::club::player::language::Language;
use crate::club::player::training::result::PlayerTrainingResult;
use crate::club::staff::CoachPlayerBond;
use crate::{
    ChangeType, ConflictLocation, HappinessEventCause, HappinessEventContext,
    HappinessEventEvidence, HappinessEventFollowUp, HappinessEventScope, HappinessEventSeverity,
    HappinessEventType, Player, PlayerFieldPositionGroup, PlayerHappiness, PlayerPositionType,
    PlayerTraining, RelationshipChange, Staff, Team, TeamTrainingResult, TeammateConflictContext,
    TeammateConflictReason,
};
use chrono::Duration;
use chrono::{Datelike, NaiveDate, NaiveDateTime, Weekday};
use rustc_hash::FxHashMap;

/// Deterministic pair rolls for the weekly training loop.
struct PairRoll;

impl PairRoll {
    /// Pseudo-random roll in `[0.0, 1.0)` for an unordered pair of player
    /// ids on a given date. Same pair + date always returns the same number
    /// — keeps weekly tests stable. The `salt` parameter lets us run
    /// independent bond / friction rolls that don't collide.
    fn roll(a: u32, b: u32, salt: u32, date: NaiveDate) -> f32 {
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
}

impl TrainingType {
    /// True when a recovering (Lmp) player can safely take part in this
    /// session. Heavy physical or high-tempo tactical work is excluded —
    /// those need full match fitness, not rehab fitness.
    fn is_recovery_compatible(&self) -> bool {
        matches!(
            self,
            TrainingType::Recovery
                | TrainingType::LightRecovery
                | TrainingType::Rehabilitation
                | TrainingType::RestDay
                | TrainingType::VideoAnalysis
                | TrainingType::Positioning
        )
    }
}

/// Coach-player bond → training-gain multiplier. Encapsulates the
/// polish task #4 spec formula so the team training loop reads as one
/// named call and the calibration lives in a single place we can tune.
struct TrainingBondGate;

impl TrainingBondGate {
    /// Spec multiplier: `0.90 + training_receptiveness * 0.20`, clamped
    /// to `0.90..1.10`. A neutral bond (receptiveness ≈ 0.5) lands at
    /// 1.0; a fully-bought-in player extracts 10% more from each
    /// session; a coach the player has lost extracts 10% less.
    #[inline]
    fn multiplier(bond: &CoachPlayerBond) -> f32 {
        (0.90 + bond.training_receptiveness * 0.20).clamp(0.90, 1.10)
    }

    /// Spec multiplier for tactical-familiarity drills:
    /// `0.90 + tactical_buy_in * 0.20`, clamped to 0.90..1.10. Used by
    /// tactical-cohesion / formation sessions where the gain comes from
    /// the player believing in the system, not just absorbing skill
    /// reps.
    #[inline]
    pub(crate) fn tactical_familiarity_multiplier(bond: &CoachPlayerBond) -> f32 {
        (0.90 + bond.tactical_buy_in * 0.20).clamp(0.90, 1.10)
    }

    /// True for sessions whose primary gain is tactical/system
    /// understanding rather than skill reps — these get the extra
    /// `tactical_familiarity_multiplier` layer applied on top of the
    /// general training-receptiveness multiplier.
    #[inline]
    fn is_tactical(session_type: &TrainingType) -> bool {
        matches!(
            session_type,
            TrainingType::Positioning
                | TrainingType::TeamShape
                | TrainingType::PressingDrills
                | TrainingType::TransitionPlay
                | TrainingType::SetPiecesDefensive
                | TrainingType::SetPieces
                | TrainingType::VideoAnalysis
                | TrainingType::OpponentSpecific
        )
    }
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

        let coach = team.staffs.training_coach(&team.team_type);

        // Determine periodization phase based on season progress
        let phase = Self::determine_phase(date);

        // Build the day's plan from the real fixture window cached on
        // the team by the country/league pipeline. Falls back to match
        // history (and a Weekday-only path) when no upcoming fixture
        // is available — keeping unit tests with no scheduled fixtures
        // running cleanly.
        let today = date.date();
        let next_match_date = team.fixture_window.next_after(today);
        let prev_match_date = team.fixture_window.previous_before(today).or_else(|| {
            team.match_history
                .items()
                .iter()
                .rev()
                .find(|m| m.date.date() <= today)
                .map(|m| m.date.date())
        });
        let fixtures_this_week = if team.fixture_window.refreshed.is_some() {
            team.fixture_window.fixtures_this_week(today)
        } else {
            Self::matches_last_7_days(team, date)
        };
        let sessions = WeeklyTrainingPlan::sessions_for_date(
            today,
            prev_match_date,
            next_match_date,
            fixtures_this_week,
            phase,
            &Self::get_coach_philosophy(coach),
        );

        for session in &sessions {
            let session_results =
                Self::execute_training_session(team, session, date, facility_quality);
            result.player_results.extend(session_results);
        }

        // Apply team cohesion effects
        Self::apply_team_cohesion_effects(team, &result, date.date());

        result
    }

    /// Run one session. The coach is resolved per participant rather than
    /// per session, because a keeper's day belongs to the goalkeeping coach
    /// whatever is on the outfield programme.
    fn execute_training_session(
        team: &Team,
        session: &TrainingSession,
        date: NaiveDateTime,
        facility_quality: f32,
    ) -> Vec<PlayerTrainingResult> {
        let participants = Self::select_participants(team, session);

        let mut results = Vec::with_capacity(participants.len());

        // Team-level chemistry is now the canonical "dressing room mood"
        // signal — read from the weekly-refreshed snapshot instead of
        // each player's local per-relation chemistry. Per polish task
        // #6: prefer `team.team_chemistry()` over
        // `player.relations.get_team_chemistry()` in team-aware paths;
        // the local read is reserved for player-only adaptation.
        let team_chem = (team.team_chemistry() / 100.0).clamp(0.0, 1.0);
        let chem_factor = 0.90 + team_chem * 0.20;
        let today = date.date();

        let is_tactical_session = TrainingBondGate::is_tactical(&session.session_type);

        for player in participants {
            // Who actually takes him. A keeper works with the goalkeeping
            // coach; everybody else with the squad's training coach, which
            // is the `coach` argument. See
            // [`StaffCollection::training_coach_for`].
            let coach = team
                .staffs
                .training_coach_for(&team.team_type, player.position().position_group());
            let mut r = PlayerTraining::train(player, coach, session, date, facility_quality);
            r.effects.work.scale_quality(chem_factor);
            // Coach-player bond gates absorption per polish task #4 —
            // spec multiplier `0.90 + training_receptiveness * 0.20`
            // clamped to 0.90..1.10. Applied at the team layer (not
            // inside `PlayerTraining::train`) so the existing thousands
            // of unit-test callers don't have to thread a bond through.
            //
            // Double-count note: `PlayerTraining::train` already reads
            // rapport directly. The bond's receptiveness axis is only
            // 30% rapport (the rest is staff-relation receptiveness,
            // coach-memory training trust, personal bond, and promise
            // credibility) so the overlap is ~3% of the multiplier band
            // — small enough to live with for the cleaner caller API.
            let bond = CoachPlayerBond::build(player, coach, today);
            r.effects
                .work
                .scale_quality(TrainingBondGate::multiplier(&bond));

            // Tactical-shape sessions additionally scale by the
            // player's belief in the plan (polish task #5). A player
            // who doesn't buy in tunes out the chalkboard; one who
            // does extracts a small extra gain.
            if is_tactical_session {
                r.effects
                    .work
                    .scale_quality(TrainingBondGate::tactical_familiarity_multiplier(&bond));
            }
            results.push(r);
        }

        results
    }

    /// Credit each coach with specialization days for the position groups
    /// he actually took. Called after training, so a coach develops deep
    /// expertise over time in whichever groups he spends most sessions on —
    /// and a goalkeeping coach, who takes only one group ever, becomes a
    /// keeper specialist within a couple of seasons.
    fn accrue_coach_specialization(
        team: &mut Team,
        coach_by_player: &FxHashMap<u32, (u32, f32)>,
        fallback_coach_id: u32,
        participant_ids: &[u32],
        roster_index: &[Option<usize>],
    ) {
        // Which coach ran which group today. One day each, however many
        // players were in the group — what matters is whether he ran it.
        let mut ran: Vec<(u32, PlayerFieldPositionGroup)> = Vec::new();
        for (pid, index) in participant_ids.iter().zip(roster_index) {
            if let Some(index) = *index {
                let group = team.players.players[index].position().position_group();
                let coach_id = coach_by_player
                    .get(pid)
                    .map(|(id, _)| *id)
                    .unwrap_or(fallback_coach_id);
                if !ran.contains(&(coach_id, group)) {
                    ran.push((coach_id, group));
                }
            }
        }
        for (coach_id, group) in ran {
            if let Some(coach) = team.staffs.find_mut(coach_id) {
                coach.accrue_specialization(group, 1);
            }
        }
    }

    fn select_participants<'a>(team: &'a Team, session: &TrainingSession) -> Vec<&'a Player> {
        let mut participants = Vec::new();

        // If specific participants are listed, use those
        if !session.participants.is_empty() {
            for player_id in &session.participants {
                if let Some(player) = team.players.find(*player_id)
                    && Self::can_participate(player, session)
                {
                    participants.push(player);
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
        // Suspension costs matches, not development — banned players
        // still train with the squad.
        if player.player_attributes.is_injured {
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
            && !session.session_type.is_recovery_compatible()
        {
            return false;
        }
        true
    }

    fn apply_team_cohesion_effects(
        team: &mut Team,
        training_results: &TeamTrainingResult,
        sim_date: NaiveDate,
    ) {
        // Players training together build relationships
        let participant_ids: Vec<u32> = training_results
            .player_results
            .iter()
            .map(|r| r.player_id)
            .collect();
        // Where each participant stands on the roster, resolved once for
        // every pass below.
        let roster_index: Vec<Option<usize>> = participant_ids
            .iter()
            .map(|&id| team.players.players.iter().position(|p| p.id == id))
            .collect();

        // Per-pair bonding/friction. The old "+0.01 across the board" was
        // unrealistic — striker rivals don't bond at the same rate as a
        // mentor pair. We compute a deterministic bond/friction chance per
        // pair from shared language, position group, mentorship, morale,
        // and personality, then apply a meaningful relation magnitude only
        // when the chance passes a per-pair pseudo-random threshold.
        // Determinism: chance comparison uses a seeded function of the two
        // ids so tests stay stable across runs.
        Self::apply_pairwise_bonding(
            team,
            &participant_ids,
            &roster_index,
            training_results,
            sim_date,
        );

        // Coach-player relationship updates based on training quality.
        // Must be the SAME coach who ran the sessions (`training_coach`),
        // because rapport and specialization are read back through that
        // coach's id in `PlayerTraining::train` / `CoachPlayerBond` — if
        // the accrual landed on the head coach instead, a club with a
        // dedicated training coach would never close either feedback loop.
        //
        // Resolved per player, not per squad: a keeper's coach is the
        // goalkeeping specialist, so his rapport has to accrue with the man
        // he actually spends the morning with.
        let coach_for = |index: usize| -> (u32, f32) {
            let group = team.players.players[index].position().position_group();
            let coach = team.staffs.training_coach_for(&team.team_type, group);
            (coach.id, coach.recent_performance.training_effectiveness)
        };
        let coach_by_player: FxHashMap<u32, (u32, f32)> = participant_ids
            .iter()
            .zip(&roster_index)
            .filter_map(|(&id, index)| index.map(|index| (id, coach_for(index))))
            .collect();
        let session_coach = team.staffs.training_coach(&team.team_type);
        let coach_id = session_coach.id;
        let coach_effectiveness = session_coach.recent_performance.training_effectiveness;

        // Rapport accrual: every participant spent a day with the coach.
        // Small positive drift + shared_days increment, even if no other
        // events fire this tick.
        for (&player_id, index) in participant_ids.iter().zip(&roster_index) {
            let with = coach_by_player
                .get(&player_id)
                .map(|(id, _)| *id)
                .unwrap_or(coach_id);
            if let Some(index) = *index {
                team.players.players[index]
                    .rapport
                    .accrue_training_day(with, sim_date, 1);
            }
        }

        // Coach specialization: credit the coach with one day per position
        // group covered in this session. Over hundreds of sessions a coach
        // organically becomes a "midfield specialist" or "striker clinic"
        // without any manual assignment.
        Self::accrue_coach_specialization(
            team,
            &coach_by_player,
            coach_id,
            &participant_ids,
            &roster_index,
        );

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

        // Positive training session: update coach-player relationships —
        // each player's with the coach who actually took him.
        if avg_morale > 0.0 {
            for (&player_id, index) in participant_ids.iter().zip(&roster_index) {
                let (with, effectiveness) = coach_by_player
                    .get(&player_id)
                    .copied()
                    .unwrap_or((coach_id, coach_effectiveness));
                let relationship_boost = 0.01 + effectiveness * 0.02; // 0.01 to 0.03
                if let Some(index) = *index {
                    let change = RelationshipChange::positive(
                        ChangeType::CoachingSuccess,
                        relationship_boost,
                    );
                    team.players.players[index]
                        .relations
                        .update_staff_relationship(with, change, sim_date);
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
        roster_index: &[Option<usize>],
        training_results: &TeamTrainingResult,
        sim_date: NaiveDate,
    ) {
        // Index morale changes by player id so we don't iterate the result
        // vector once per pair.
        let mut morale_change: FxHashMap<u32, f32> = FxHashMap::with_capacity_and_hasher(
            training_results.player_results.len(),
            Default::default(),
        );
        for r in &training_results.player_results {
            morale_change.insert(r.player_id, r.effects.morale_change);
        }

        let snaps = Self::bond_snapshots(team, participant_ids, roster_index);
        let effects = Self::bond_effects(&snaps, &morale_change, sim_date);
        Self::apply_bond_effects(team, effects, sim_date);
    }

    /// Snapshot per-player bonding inputs (immutable) so we can apply
    /// mutations afterwards without aliasing the player borrow.
    fn bond_snapshots(
        team: &Team,
        participant_ids: &[u32],
        roster_index: &[Option<usize>],
    ) -> Vec<BondSnapshot> {
        participant_ids
            .iter()
            .zip(roster_index)
            .filter_map(|(id, index)| {
                let index = (*index)?;
                let p = &team.players.players[index];
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
                Some(BondSnapshot {
                    id: *id,
                    index,
                    position_group: group,
                    primary_lang,
                    languages,
                    morale: p.happiness.morale,
                    controversy: p.attributes.controversy,
                    professionalism: p.attributes.professionalism,
                    mentor_target,
                })
            })
            .collect()
    }

    /// Pairwise pass — collect changes first, then apply, so we don't hold a
    /// long mutable borrow on `team`.
    fn bond_effects(
        snaps: &[BondSnapshot],
        morale_change: &FxHashMap<u32, f32>,
        sim_date: NaiveDate,
    ) -> Vec<BondEffect> {
        let mut effects: Vec<BondEffect> = Vec::new();
        for i in 0..snaps.len() {
            for j in (i + 1)..snaps.len() {
                effects.extend(Self::pair_bond_effects(
                    &snaps[i],
                    &snaps[j],
                    morale_change,
                    sim_date,
                ));
            }
        }
        effects
    }

    fn pair_bond_effects(
        a: &BondSnapshot,
        b: &BondSnapshot,
        morale_change: &FxHashMap<u32, f32>,
        sim_date: NaiveDate,
    ) -> Vec<BondEffect> {
        let same_group = a.position_group == b.position_group;
        let direct_rivals = same_group;
        // Shared language if either side has the other side's primary language
        // at proficiency ≥ 60 (or native).
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
        bond_chance += f32::from(same_group as u8) * 0.04;
        bond_chance += f32::from(shared_language as u8) * 0.05;
        bond_chance += f32::from(mentor_pair as u8) * 0.08;
        bond_chance += f32::from(both_pro as u8) * 0.03;
        bond_chance += f32::from(positive_session as u8) * 0.04;
        bond_chance -= f32::from(direct_rivals as u8) * 0.04;
        bond_chance -= f32::from(either_low_morale as u8) * 0.03;
        bond_chance -= f32::from(either_high_controversy as u8) * 0.03;

        // Friction chance.
        let mut friction_chance = 0.01;
        friction_chance += f32::from(direct_rivals as u8) * 0.04;
        friction_chance += f32::from(either_low_pro as u8) * 0.03;
        friction_chance += f32::from(either_high_controversy as u8) * 0.03;

        // Deterministic per-pair "roll" — same hash twice in the same week
        // resolves identically. Independent rolls for bond and friction so
        // the same pair can't trigger both.
        let bond_roll = PairRoll::roll(a.id, b.id, 0xB0_4D, sim_date);
        let friction_roll = PairRoll::roll(a.id, b.id, 0xF1_2A, sim_date);

        if bond_chance > 0.0 && bond_roll < bond_chance {
            let mut mag: f32 = 0.08;
            if mentor_pair {
                mag += 0.10;
            }
            if shared_language {
                mag += 0.05;
            }
            mag = mag.clamp(0.08, 0.25);
            return vec![
                BondEffect {
                    from: a.index,
                    to: b.id,
                    relation_change: mag,
                    bond: true,
                },
                BondEffect {
                    from: b.index,
                    to: a.id,
                    relation_change: mag,
                    bond: true,
                },
            ];
        }
        if friction_chance > 0.0 && friction_roll < friction_chance {
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
            return vec![
                BondEffect {
                    from: a.index,
                    to: b.id,
                    relation_change: -mag,
                    bond: false,
                },
                BondEffect {
                    from: b.index,
                    to: a.id,
                    relation_change: -mag,
                    bond: false,
                },
            ];
        }
        Vec::new()
    }

    fn apply_bond_effects(team: &mut Team, effects: Vec<BondEffect>, sim_date: NaiveDate) {
        for eff in effects {
            team.players.players[eff.from].apply_training_bond_effect(&eff, sim_date);
        }
    }
}

struct BondSnapshot {
    id: u32,
    index: usize,
    position_group: PlayerFieldPositionGroup,
    primary_lang: Option<Language>,
    languages: Vec<(Language, u8)>,
    morale: f32,
    controversy: f32,
    professionalism: f32,
    mentor_target: Option<u32>,
}

struct BondEffect {
    from: usize,
    to: u32,
    relation_change: f32,
    bond: bool,
}

impl Player {
    fn apply_training_bond_effect(&mut self, eff: &BondEffect, sim_date: NaiveDate) {
        // Apply effects. Event emission thresholds chosen to match the
        // bond/friction magnitude bands above:
        //   bonding magnitudes land in 0.08..0.25 - emit at >= 0.20
        //   friction magnitudes land in 0.10..0.35 - emit at >= 0.25
        // A 14-day per-partner cooldown stops a chronic flashpoint pair from
        // spamming the timeline every training week.
        const BOND_EVENT_THRESHOLD: f32 = 0.20;
        const FRICTION_EVENT_THRESHOLD: f32 = 0.25;

        let change_type = if eff.bond {
            ChangeType::TrainingBonding
        } else {
            ChangeType::TrainingFriction
        };
        self.relations
            .update_with_type(eff.to, eff.relation_change, change_type, sim_date);

        let mag_abs = eff.relation_change.abs();
        let qualifies = if eff.bond {
            mag_abs >= BOND_EVENT_THRESHOLD
        } else {
            mag_abs >= FRICTION_EVENT_THRESHOLD
        };
        if !qualifies {
            return;
        }
        if eff.bond {
            self.emit_training_bond(eff.to);
        } else {
            self.emit_training_friction(eff.to);
        }
    }

    fn emit_training_bond(&mut self, partner_id: u32) {
        const PAIR_COOLDOWN_DAYS: u16 = 14;
        let mag = 0.6;
        let relations = &self.relations;
        self.happiness
            .try_add_partner_context_with_same_tick_budget(
                HappinessEventType::TeammateBonding,
                mag,
                partner_id,
                || {
                    let mut ctx = HappinessEventContext::new(
                        HappinessEventCause::TrainingPartnership,
                        HappinessEventSeverity::from_magnitude(mag),
                        HappinessEventScope::TrainingGround,
                    )
                    .with_evidence(HappinessEventEvidence::TrainingStandardsMismatch)
                    .with_follow_up(HappinessEventFollowUp::TrendImproving);
                    if let Some((level, trust, friendship, prof)) = relations
                        .get_player(partner_id)
                        .map(|r| (r.level, r.trust, r.friendship, r.professional_respect))
                    {
                        ctx = ctx
                            .with_relationship_levels(level, level)
                            .with_relationship_axes(trust, friendship, prof);
                    }
                    ctx
                },
                PAIR_COOLDOWN_DAYS,
                PlayerHappiness::MAX_TEAMMATE_BONDING_PER_TICK,
            );
    }

    fn emit_training_friction(&mut self, partner_id: u32) {
        const PAIR_COOLDOWN_DAYS: u16 = 14;
        let mag = -0.8;
        let relations = &self.relations;
        self.happiness
            .try_add_partner_context_with_same_tick_budget(
                HappinessEventType::ConflictWithTeammate,
                mag,
                partner_id,
                || {
                    let mut ctx = HappinessEventContext::new(
                        HappinessEventCause::TrainingFriction,
                        HappinessEventSeverity::from_magnitude(mag),
                        HappinessEventScope::TrainingGround,
                    )
                    .with_evidence(HappinessEventEvidence::TrainingStandardsMismatch)
                    .with_follow_up(HappinessEventFollowUp::LikelyToSettle)
                    .with_teammate_conflict_context(
                        TeammateConflictContext::new(
                            TeammateConflictReason::TrainingStandards,
                            ConflictLocation::TrainingGround,
                        ),
                    );
                    if let Some((level, trust, friendship, prof)) = relations
                        .get_player(partner_id)
                        .map(|r| (r.level, r.trust, r.friendship, r.professional_respect))
                    {
                        ctx = ctx
                            .with_relationship_levels(level, level)
                            .with_relationship_axes(trust, friendship, prof);
                        if trust <= 35.0 {
                            ctx = ctx.with_evidence(HappinessEventEvidence::LowTrust);
                        }
                        if level <= -25.0 {
                            ctx = ctx
                                .with_evidence(HappinessEventEvidence::AlreadyStrainedRelationship);
                        } else if level.abs() < 25.0 {
                            ctx = ctx.with_evidence(HappinessEventEvidence::WeakExistingBond);
                        }
                    }
                    ctx
                },
                PAIR_COOLDOWN_DAYS,
                PlayerHappiness::MAX_CONFLICT_WITH_TEAMMATE_PER_TICK,
            );
    }
}

impl TeamTraining {
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

    /// Count matches in the last seven days from match history. Used as
    /// a fallback when the league-side fixture window has not been
    /// written yet (early simulation tick or unit test).
    fn matches_last_7_days(team: &Team, date: NaiveDateTime) -> u8 {
        let cutoff = date.date() - Duration::days(7);
        team.match_history
            .items()
            .iter()
            .filter(|m| m.date.date() > cutoff && m.date.date() <= date.date())
            .count()
            .min(u8::MAX as usize) as u8
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

    // Unit and mental training
    Defending,
    DecisionMaking,

    // Goalkeeper Training (focus_positions = [Goalkeeper])
    GoalkeeperTraining,

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
    pub sessions: FxHashMap<Weekday, Vec<TrainingSession>>,
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

/// Each unit's own craft on a training day: keepers in goal, defenders
/// defending, midfielders on the ball, attackers shooting, and on the eve of
/// a match the wide players crossing.
struct UnitWork;

impl UnitWork {
    const KEEPERS: [PlayerPositionType; 1] = [PlayerPositionType::Goalkeeper];
    const DEFENDERS: [PlayerPositionType; 8] = [
        PlayerPositionType::Sweeper,
        PlayerPositionType::DefenderLeft,
        PlayerPositionType::DefenderCenterLeft,
        PlayerPositionType::DefenderCenter,
        PlayerPositionType::DefenderCenterRight,
        PlayerPositionType::DefenderRight,
        PlayerPositionType::WingbackLeft,
        PlayerPositionType::WingbackRight,
    ];
    const MIDFIELDERS: [PlayerPositionType; 6] = [
        PlayerPositionType::DefensiveMidfielder,
        PlayerPositionType::MidfielderLeft,
        PlayerPositionType::MidfielderCenterLeft,
        PlayerPositionType::MidfielderCenter,
        PlayerPositionType::MidfielderCenterRight,
        PlayerPositionType::MidfielderRight,
    ];
    const ATTACKERS: [PlayerPositionType; 7] = [
        PlayerPositionType::AttackingMidfielderLeft,
        PlayerPositionType::AttackingMidfielderCenter,
        PlayerPositionType::AttackingMidfielderRight,
        PlayerPositionType::ForwardLeft,
        PlayerPositionType::ForwardCenter,
        PlayerPositionType::ForwardRight,
        PlayerPositionType::Striker,
    ];
    const WIDE: [PlayerPositionType; 10] = [
        PlayerPositionType::DefenderLeft,
        PlayerPositionType::DefenderRight,
        PlayerPositionType::WingbackLeft,
        PlayerPositionType::WingbackRight,
        PlayerPositionType::MidfielderLeft,
        PlayerPositionType::MidfielderRight,
        PlayerPositionType::AttackingMidfielderLeft,
        PlayerPositionType::AttackingMidfielderRight,
        PlayerPositionType::ForwardLeft,
        PlayerPositionType::ForwardRight,
    ];

    fn sessions(day: Weekday, eve_of_match: bool) -> Vec<TrainingSession> {
        let intensity = if eve_of_match {
            TrainingIntensity::Light
        } else {
            TrainingIntensity::Moderate
        };
        let midfield = match day {
            Weekday::Mon | Weekday::Thu => TrainingType::Passing,
            Weekday::Tue | Weekday::Fri => TrainingType::BallControl,
            _ => TrainingType::DecisionMaking,
        };
        let block = |session_type, focus: &[PlayerPositionType]| TrainingSession {
            session_type,
            intensity: intensity.clone(),
            duration_minutes: 30,
            focus_positions: focus.to_vec(),
            participants: vec![],
        };
        let mut sessions = vec![
            block(TrainingType::GoalkeeperTraining, &Self::KEEPERS),
            block(TrainingType::Defending, &Self::DEFENDERS),
            block(midfield, &Self::MIDFIELDERS),
            block(TrainingType::Shooting, &Self::ATTACKERS),
        ];
        if eve_of_match {
            sessions.push(block(TrainingType::Crossing, &Self::WIDE));
        }
        sessions
    }
}

impl WeeklyTrainingPlan {
    /// Generate a realistic weekly training plan based on match schedule.
    /// Backwards-compatible shim — internally it calls the
    /// context-aware path with `recent_matches = 0` (no congestion).
    pub fn generate_weekly_plan(
        next_match_day: Option<Weekday>,
        previous_match_day: Option<Weekday>,
        phase: PeriodizationPhase,
        coach_philosophy: &CoachingPhilosophy,
    ) -> Self {
        Self::generate_weekly_plan_with_context(
            next_match_day,
            previous_match_day,
            0,
            phase,
            coach_philosophy,
        )
    }

    /// Generate a weekly training plan with full fixture context.
    ///
    /// `recent_matches_14d` lets the plan dampen high-intensity work in
    /// double-match weeks — pressing drills and Speed sessions get
    /// replaced with technical / tactical work when the legs need
    /// protection.
    ///
    /// The plan reads `previous_match_day` and `next_match_day` to
    /// react to real match proximity:
    ///   * MD+1 / MD+2 → recovery / video / light technical
    ///   * MD-1        → set pieces / opponent specific / light
    ///   * MD-2        → match preparation / team shape
    ///   * 3+ days from a match → main physical / tactical load
    ///
    /// When no fixture is in range, the plan defaults to a full
    /// training week rather than assuming a Saturday match.
    pub fn generate_weekly_plan_with_context(
        next_match_day: Option<Weekday>,
        previous_match_day: Option<Weekday>,
        recent_matches_14d: u8,
        phase: PeriodizationPhase,
        coach_philosophy: &CoachingPhilosophy,
    ) -> Self {
        let mut sessions = FxHashMap::default();
        let match_days = vec![next_match_day, previous_match_day]
            .into_iter()
            .flatten()
            .collect();
        let congested = recent_matches_14d >= 3;

        for &day in &[
            Weekday::Mon,
            Weekday::Tue,
            Weekday::Wed,
            Weekday::Thu,
            Weekday::Fri,
            Weekday::Sat,
            Weekday::Sun,
        ] {
            let day_sessions = Self::sessions_for_day(
                day,
                next_match_day,
                previous_match_day,
                phase,
                coach_philosophy,
                congested,
            );
            sessions.insert(day, day_sessions);
        }

        WeeklyTrainingPlan {
            sessions,
            match_days,
            periodization_phase: phase,
        }
    }

    /// Pick the right band of sessions for `day` given fixture context.
    /// Single dispatcher so every weekday respects the same MD-relative
    /// rules — no fragile per-day branching.
    fn sessions_for_day(
        day: Weekday,
        next_match: Option<Weekday>,
        previous_match: Option<Weekday>,
        phase: PeriodizationPhase,
        philosophy: &CoachingPhilosophy,
        congested: bool,
    ) -> Vec<TrainingSession> {
        let mut sessions = Self::team_sessions_for_day(
            day,
            next_match,
            previous_match,
            phase,
            philosophy,
            congested,
        );
        let md_plus = previous_match.map(|m| Self::weekday_distance(m, day));
        let md_minus = next_match.map(|m| Self::weekday_distance(day, m));
        if Some(day) != next_match && md_plus != Some(1) {
            sessions.extend(UnitWork::sessions(day, md_minus == Some(1)));
        }
        sessions
    }

    /// The whole squad's sessions for `day`.
    fn team_sessions_for_day(
        day: Weekday,
        next_match: Option<Weekday>,
        previous_match: Option<Weekday>,
        phase: PeriodizationPhase,
        philosophy: &CoachingPhilosophy,
        congested: bool,
    ) -> Vec<TrainingSession> {
        // Match day itself — no training session.
        if Some(day) == next_match {
            return vec![];
        }

        // Days since previous match (1..6, or None when too far back).
        let md_plus = previous_match.map(|m| Self::weekday_distance(m, day));
        // Days until next match (1..6, or None when no fixture).
        let md_minus = next_match.map(|m| Self::weekday_distance(day, m));

        // MD+1 → recovery + video. Universally true regardless of weekday.
        if md_plus == Some(1) {
            return vec![
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
            ];
        }
        // MD+2 → light recovery + technical maintenance.
        if md_plus == Some(2) {
            return vec![
                TrainingSession {
                    session_type: TrainingType::LightRecovery,
                    intensity: TrainingIntensity::Light,
                    duration_minutes: 45,
                    focus_positions: vec![],
                    participants: vec![],
                },
                TrainingSession {
                    session_type: TrainingType::Passing,
                    intensity: TrainingIntensity::Light,
                    duration_minutes: 30,
                    focus_positions: vec![],
                    participants: vec![],
                },
                TrainingSession {
                    session_type: TrainingType::BallControl,
                    intensity: TrainingIntensity::Light,
                    duration_minutes: 30,
                    focus_positions: vec![],
                    participants: vec![],
                },
            ];
        }
        // MD-1 → set pieces / opponent specific / light.
        if md_minus == Some(1) {
            return vec![
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
            ];
        }
        // MD-2 → match preparation / team shape.
        if md_minus == Some(2) {
            return vec![
                TrainingSession {
                    session_type: TrainingType::MatchPreparation,
                    intensity: TrainingIntensity::Light,
                    duration_minutes: 60,
                    focus_positions: vec![],
                    participants: vec![],
                },
                TrainingSession {
                    session_type: TrainingType::TeamShape,
                    intensity: TrainingIntensity::Moderate,
                    duration_minutes: 45,
                    focus_positions: vec![],
                    participants: vec![],
                },
            ];
        }

        // Otherwise — at least 3 days from any match — run the main
        // physical / tactical load week, dampened in congested periods.
        Self::main_load_sessions(day, phase, philosophy, congested, true)
    }

    /// Whole-day distance between two weekdays in the forward direction
    /// (Mon-Wed = 2, Sat-Mon = 2, Sat-Sat = 7).
    fn weekday_distance(from: Weekday, to: Weekday) -> u8 {
        let f = from.num_days_from_monday() as i32;
        let t = to.num_days_from_monday() as i32;
        let mut d = t - f;
        if d <= 0 {
            d += 7;
        }
        d as u8
    }

    /// Generate a plan whose `today` slot is computed from real
    /// calendar distance to the next/previous fixture, and whose other
    /// weekday slots fall back to the standard main-load layout. The
    /// production training pipeline only consumes today's slot, so the
    /// other days are inspectable in tests but never executed against
    /// a fictional fixture.
    pub fn generate_for_date(
        today: NaiveDate,
        previous_match: Option<NaiveDate>,
        next_match: Option<NaiveDate>,
        fixtures_this_week: u8,
        phase: PeriodizationPhase,
        philosophy: &CoachingPhilosophy,
    ) -> Self {
        let mut sessions = FxHashMap::default();
        let match_days = vec![
            next_match.map(|d| d.weekday()),
            previous_match.map(|d| d.weekday()),
        ]
        .into_iter()
        .flatten()
        .collect();
        let congested = Self::is_congested(fixtures_this_week);

        let today_sessions = Self::sessions_for_date(
            today,
            previous_match,
            next_match,
            fixtures_this_week,
            phase,
            philosophy,
        );
        for &day in &[
            Weekday::Mon,
            Weekday::Tue,
            Weekday::Wed,
            Weekday::Thu,
            Weekday::Fri,
            Weekday::Sat,
            Weekday::Sun,
        ] {
            if day == today.weekday() {
                sessions.insert(day, today_sessions.clone());
            } else {
                sessions.insert(
                    day,
                    Self::main_load_sessions(day, phase, philosophy, congested, true),
                );
            }
        }

        WeeklyTrainingPlan {
            sessions,
            match_days,
            periodization_phase: phase,
        }
    }

    /// Today's slot of [`Self::generate_for_date`] on its own — all the
    /// training pipeline executes.
    pub fn sessions_for_date(
        today: NaiveDate,
        previous_match: Option<NaiveDate>,
        next_match: Option<NaiveDate>,
        fixtures_this_week: u8,
        phase: PeriodizationPhase,
        philosophy: &CoachingPhilosophy,
    ) -> Vec<TrainingSession> {
        Self::sessions_for_real_date(
            today,
            previous_match,
            next_match,
            phase,
            philosophy,
            Self::is_congested(fixtures_this_week),
        )
    }

    /// Two competitive matches inside the week around today is the
    /// canonical congestion threshold for top-flight schedules.
    fn is_congested(fixtures_this_week: u8) -> bool {
        fixtures_this_week >= 2
    }

    /// Pick the right band of sessions for `today` based on real
    /// calendar distance to the next/previous fixture. Strict on the
    /// "match day = no training" rule, so a kickoff date never picks
    /// up a phantom session — including once the day's match is played
    /// and already filed as the previous one, which is how clubs see it:
    /// the league plays before the clubs train.
    fn sessions_for_real_date(
        today: NaiveDate,
        previous_match: Option<NaiveDate>,
        next_match: Option<NaiveDate>,
        phase: PeriodizationPhase,
        philosophy: &CoachingPhilosophy,
        congested: bool,
    ) -> Vec<TrainingSession> {
        let mut sessions = Self::team_sessions_for_real_date(
            today,
            previous_match,
            next_match,
            phase,
            philosophy,
            congested,
        );
        let match_day = Some(today) == next_match || Some(today) == previous_match;
        let day_after = previous_match.is_some_and(|d| (today - d).num_days() == 1);
        let eve = next_match.is_some_and(|d| (d - today).num_days() == 1);
        if !match_day && !day_after {
            sessions.extend(UnitWork::sessions(today.weekday(), eve));
        }
        sessions
    }

    /// The whole squad's sessions for `today`.
    fn team_sessions_for_real_date(
        today: NaiveDate,
        previous_match: Option<NaiveDate>,
        next_match: Option<NaiveDate>,
        phase: PeriodizationPhase,
        philosophy: &CoachingPhilosophy,
        congested: bool,
    ) -> Vec<TrainingSession> {
        if Some(today) == next_match || Some(today) == previous_match {
            return vec![];
        }
        let md_plus = previous_match
            .map(|d| (today - d).num_days())
            .filter(|n| *n >= 0);
        let md_minus = next_match
            .map(|d| (d - today).num_days())
            .filter(|n| *n > 0);

        if md_plus == Some(1) {
            return vec![
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
            ];
        }
        if md_plus == Some(2) {
            return vec![
                TrainingSession {
                    session_type: TrainingType::LightRecovery,
                    intensity: TrainingIntensity::Light,
                    duration_minutes: 45,
                    focus_positions: vec![],
                    participants: vec![],
                },
                TrainingSession {
                    session_type: TrainingType::Passing,
                    intensity: TrainingIntensity::Light,
                    duration_minutes: 30,
                    focus_positions: vec![],
                    participants: vec![],
                },
                TrainingSession {
                    session_type: TrainingType::BallControl,
                    intensity: TrainingIntensity::Light,
                    duration_minutes: 30,
                    focus_positions: vec![],
                    participants: vec![],
                },
            ];
        }
        if md_minus == Some(1) {
            return vec![
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
            ];
        }
        if md_minus == Some(2) {
            return vec![
                TrainingSession {
                    session_type: TrainingType::MatchPreparation,
                    intensity: TrainingIntensity::Light,
                    duration_minutes: 60,
                    focus_positions: vec![],
                    participants: vec![],
                },
                TrainingSession {
                    session_type: TrainingType::TeamShape,
                    intensity: TrainingIntensity::Moderate,
                    duration_minutes: 45,
                    focus_positions: vec![],
                    participants: vec![],
                },
            ];
        }
        // 3+ days from any match - run the main load. Strength and agility
        // work take turns week by week.
        let strength_week = today.iso_week().week() % 2 == 1;
        Self::main_load_sessions(today.weekday(), phase, philosophy, congested, strength_week)
    }

    /// Default load week — used when no fixture is within ±2 days. The
    /// `congested` flag drops Speed / Pressing in favour of technical
    /// work to protect a tired squad. In season an uncongested week holds
    /// one strength or agility block, on its first main-load day.
    fn main_load_sessions(
        day: Weekday,
        phase: PeriodizationPhase,
        philosophy: &CoachingPhilosophy,
        congested: bool,
        strength_week: bool,
    ) -> Vec<TrainingSession> {
        let base = match phase {
            PeriodizationPhase::PreSeason => TrainingIntensity::High,
            PeriodizationPhase::MidSeason => TrainingIntensity::Moderate,
            PeriodizationPhase::LateSeason => TrainingIntensity::Light,
            PeriodizationPhase::OffSeason => TrainingIntensity::VeryLight,
            _ => TrainingIntensity::Moderate,
        };
        match day {
            Weekday::Mon => Self::monday_sessions(base),
            Weekday::Tue => {
                Self::tuesday_sessions(base, phase, philosophy, congested, strength_week)
            }
            Weekday::Wed => Self::wednesday_sessions(base, congested),
            Weekday::Thu => Self::thursday_sessions(congested),
            Weekday::Fri => Self::friday_sessions(congested),
            Weekday::Sat => Self::saturday_sessions(),
            Weekday::Sun => Self::sunday_sessions(),
        }
    }

    fn monday_sessions(base: TrainingIntensity) -> Vec<TrainingSession> {
        vec![
            TrainingSession {
                session_type: TrainingType::Endurance,
                intensity: base,
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

    fn tuesday_sessions(
        base: TrainingIntensity,
        phase: PeriodizationPhase,
        philosophy: &CoachingPhilosophy,
        congested: bool,
        strength_week: bool,
    ) -> Vec<TrainingSession> {
        vec![
            TrainingSession {
                session_type: match philosophy.tactical_focus {
                    TacticalFocus::Possession => TrainingType::Passing,
                    TacticalFocus::Pressing => {
                        if congested {
                            TrainingType::Positioning
                        } else {
                            TrainingType::PressingDrills
                        }
                    }
                    TacticalFocus::Counter => TrainingType::TransitionPlay,
                    _ => TrainingType::TeamShape,
                },
                intensity: if congested {
                    TrainingIntensity::Light
                } else {
                    base.clone()
                },
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
        .into_iter()
        .chain(
            (!congested
                && matches!(
                    phase,
                    PeriodizationPhase::EarlySeason
                        | PeriodizationPhase::MidSeason
                        | PeriodizationPhase::LateSeason
                ))
            .then(|| {
                [
                    TrainingSession {
                        session_type: if strength_week {
                            TrainingType::Strength
                        } else {
                            TrainingType::Agility
                        },
                        intensity: TrainingIntensity::Moderate,
                        duration_minutes: 30,
                        focus_positions: vec![],
                        participants: vec![],
                    },
                    TrainingSession {
                        session_type: TrainingType::Endurance,
                        intensity: base.clone(),
                        duration_minutes: 30,
                        focus_positions: vec![],
                        participants: vec![],
                    },
                ]
            })
            .into_iter()
            .flatten(),
        )
        .collect()
    }

    fn wednesday_sessions(base: TrainingIntensity, congested: bool) -> Vec<TrainingSession> {
        vec![
            TrainingSession {
                session_type: TrainingType::Positioning,
                intensity: base,
                duration_minutes: 60,
                focus_positions: vec![],
                participants: vec![],
            },
            TrainingSession {
                session_type: if congested {
                    TrainingType::BallControl
                } else {
                    TrainingType::Speed
                },
                intensity: TrainingIntensity::Moderate,
                duration_minutes: 30,
                focus_positions: vec![],
                participants: vec![],
            },
            TrainingSession {
                session_type: TrainingType::SetPiecesDefensive,
                intensity: TrainingIntensity::Light,
                duration_minutes: 30,
                focus_positions: vec![],
                participants: vec![],
            },
        ]
    }

    fn thursday_sessions(congested: bool) -> Vec<TrainingSession> {
        vec![
            TrainingSession {
                session_type: if congested {
                    TrainingType::BallControl
                } else {
                    TrainingType::Speed
                },
                intensity: if congested {
                    TrainingIntensity::Moderate
                } else {
                    TrainingIntensity::High
                },
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

    fn friday_sessions(congested: bool) -> Vec<TrainingSession> {
        vec![
            TrainingSession {
                session_type: TrainingType::BallControl,
                intensity: TrainingIntensity::Moderate,
                duration_minutes: 60,
                focus_positions: vec![],
                participants: vec![],
            },
            TrainingSession {
                session_type: if congested {
                    TrainingType::Passing
                } else {
                    TrainingType::TransitionPlay
                },
                intensity: if congested {
                    TrainingIntensity::Light
                } else {
                    TrainingIntensity::High
                },
                duration_minutes: 45,
                focus_positions: vec![],
                participants: vec![],
            },
        ]
    }

    fn saturday_sessions() -> Vec<TrainingSession> {
        vec![TrainingSession {
            session_type: TrainingType::MatchPreparation,
            intensity: TrainingIntensity::High,
            duration_minutes: 90,
            focus_positions: vec![],
            participants: vec![],
        }]
    }

    fn sunday_sessions() -> Vec<TrainingSession> {
        vec![TrainingSession {
            session_type: TrainingType::RestDay,
            intensity: TrainingIntensity::VeryLight,
            duration_minutes: 0,
            focus_positions: vec![],
            participants: vec![],
        }]
    }
}

// ============== Training Effects System ==============

#[derive(Debug, Clone)]
pub struct TrainingEffects {
    /// What the session practised, and how well: repetitions per attribute
    /// and their quality. The player's development decides what it does.
    pub work: SessionWork,
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

#[derive(Debug, Clone)]
pub struct IndividualTrainingPlan {
    pub player_id: u32,
    pub focus_areas: Vec<TrainingFocus>,
    pub intensity_modifier: f32, // 0.5 to 1.5
    pub special_instructions: Vec<SpecialInstruction>,
    /// When the coaching staff set the plan. Drives the monthly
    /// progress tick and the natural expiry of a plan that has run
    /// its course.
    pub started: Option<NaiveDate>,
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

#[cfg(test)]
mod bond_consumer_tests {
    //! Polish task #11 spec tests covering the team-training consumers
    //! added in polish tasks #4 and #5: `training_receptiveness` and
    //! `tactical_buy_in`. The pure multiplier functions are tested
    //! directly so we don't need to stand up a whole team training
    //! cycle to prove the spec curves.
    use super::*;

    fn bond(training_receptiveness: f32, tactical_buy_in: f32) -> CoachPlayerBond {
        CoachPlayerBond {
            training_receptiveness,
            tactical_buy_in,
            ..Default::default()
        }
    }

    #[test]
    fn training_relationship_multiplier_matches_spec_band() {
        // Spec: 0.90 + receptiveness * 0.20, clamped 0.90..1.10.
        // Neutral 0.5 → 1.0; max 1.0 → 1.10; min 0.0 → 0.90.
        assert!((TrainingBondGate::multiplier(&bond(0.5, 0.5)) - 1.0).abs() < 1e-3);
        assert!((TrainingBondGate::multiplier(&bond(1.0, 0.5)) - 1.10).abs() < 1e-3);
        assert!((TrainingBondGate::multiplier(&bond(0.0, 0.5)) - 0.90).abs() < 1e-3);
        // Clamps at the band edges even if upstream produces noise above 1.0.
        assert!(TrainingBondGate::multiplier(&bond(1.4, 0.5)) <= 1.10 + 1e-3);
    }

    #[test]
    fn low_tactical_buy_in_reduces_tactical_familiarity_multiplier() {
        // Polish task #11 spec test: "low tactical_buy_in reduces
        // tactical familiarity gain." The multiplier must be strictly
        // below the neutral 1.0 when buy_in drops below 0.5.
        let neutral = TrainingBondGate::tactical_familiarity_multiplier(&bond(0.5, 0.5));
        let low = TrainingBondGate::tactical_familiarity_multiplier(&bond(0.5, 0.1));
        let high = TrainingBondGate::tactical_familiarity_multiplier(&bond(0.5, 0.9));
        assert!(
            (neutral - 1.0).abs() < 1e-3,
            "neutral should be 1.0, got {}",
            neutral
        );
        assert!(low < 1.0, "low buy_in should produce < 1.0 ({})", low);
        assert!(high > 1.0, "high buy_in should produce > 1.0 ({})", high);
        assert!(low < neutral && neutral < high);
    }

    #[test]
    fn tactical_session_classification_covers_team_shape_family() {
        // Sanity: the sessions targeted by polish task #5 must be
        // flagged as tactical so the snapshot's tactical-buy-in
        // multiplier actually fires for them.
        assert!(TrainingBondGate::is_tactical(&TrainingType::TeamShape));
        assert!(TrainingBondGate::is_tactical(&TrainingType::Positioning));
        assert!(TrainingBondGate::is_tactical(&TrainingType::PressingDrills));
        assert!(TrainingBondGate::is_tactical(&TrainingType::TransitionPlay));
        assert!(TrainingBondGate::is_tactical(
            &TrainingType::SetPiecesDefensive
        ));
        // And the skill-rep sessions must NOT — applying the tactical
        // multiplier to a pure shooting drill would double-dip.
        assert!(!TrainingBondGate::is_tactical(&TrainingType::Shooting));
        assert!(!TrainingBondGate::is_tactical(&TrainingType::BallControl));
        assert!(!TrainingBondGate::is_tactical(&TrainingType::Endurance));
    }
}

#[cfg(test)]
mod unit_week_tests {
    //! The training week by unit: every training day but the day after a
    //! match gives each unit its own craft.
    use super::*;
    use crate::club::player::builder::PlayerBuilder;
    use crate::club::player::development::SkillKey;
    use crate::shared::fullname::FullName;
    use crate::{
        PersonAttributes, PlayerAttributes, PlayerPosition, PlayerPositions, PlayerSkills,
        PlayerTraining, StaffStub,
    };

    fn philosophy() -> CoachingPhilosophy {
        CoachingPhilosophy {
            tactical_focus: TacticalFocus::Balanced,
            training_intensity: TrainingIntensityPreference::Medium,
            youth_focus: false,
            rotation_preference: RotationPreference::Moderate,
        }
    }

    /// Every session a player of `position` attends between `from` and `to`
    /// (exclusive) for a club playing on `fixtures`.
    fn attended(
        position: PlayerPositionType,
        fixtures: &[NaiveDate],
        from: NaiveDate,
        to: NaiveDate,
    ) -> Vec<TrainingSession> {
        let mut out = Vec::new();
        let mut day = from;
        while day < to {
            let previous = fixtures.iter().filter(|d| **d <= day).max().copied();
            let next = fixtures.iter().filter(|d| **d > day).min().copied();
            let this_week = fixtures
                .iter()
                .filter(|d| (**d - day).num_days().abs() <= 3)
                .count() as u8;
            for session in WeeklyTrainingPlan::sessions_for_date(
                day,
                previous,
                next,
                this_week,
                PeriodizationPhase::MidSeason,
                &philosophy(),
            ) {
                if session.focus_positions.is_empty() || session.focus_positions.contains(&position)
                {
                    out.push(session);
                }
            }
            day = day.succ_opt().unwrap();
        }
        out
    }

    fn saturdays(weeks: i64) -> Vec<NaiveDate> {
        let first = NaiveDate::from_ymd_opt(2026, 10, 3).unwrap();
        (0..=weeks).map(|w| first + Duration::weeks(w)).collect()
    }

    fn one_week(position: PlayerPositionType) -> Vec<TrainingSession> {
        let fixtures = saturdays(1);
        attended(
            position,
            &fixtures,
            fixtures[0].succ_opt().unwrap(),
            fixtures[1],
        )
    }

    fn count(sessions: &[TrainingSession], kind: TrainingType) -> usize {
        sessions.iter().filter(|s| s.session_type == kind).count()
    }

    #[test]
    fn a_keepers_one_match_week() {
        let week = one_week(PlayerPositionType::Goalkeeper);
        assert!(count(&week, TrainingType::GoalkeeperTraining) >= 4);
    }

    #[test]
    fn a_defenders_one_match_week() {
        let week = one_week(PlayerPositionType::DefenderCenter);
        assert!(count(&week, TrainingType::Defending) >= 2);
    }

    #[test]
    fn a_wingers_one_match_week() {
        let week = one_week(PlayerPositionType::AttackingMidfielderRight);
        assert!(count(&week, TrainingType::Shooting) >= 1);
        assert!(count(&week, TrainingType::Crossing) >= 1);
    }

    #[test]
    fn a_midfielders_one_match_week() {
        let week = one_week(PlayerPositionType::MidfielderCenter);
        let craft = count(&week, TrainingType::Passing)
            + count(&week, TrainingType::BallControl)
            + count(&week, TrainingType::DecisionMaking);
        assert!(
            craft >= 4,
            "a central midfielder had {craft} sessions of his craft"
        );
    }

    #[test]
    fn a_congested_week_still_has_unit_work() {
        let saturday = NaiveDate::from_ymd_opt(2026, 10, 3).unwrap();
        let fixtures = [
            saturday,
            saturday + Duration::days(4),
            saturday + Duration::days(7),
        ];
        for (position, crafts) in [
            (
                PlayerPositionType::Goalkeeper,
                &[TrainingType::GoalkeeperTraining][..],
            ),
            (
                PlayerPositionType::DefenderCenter,
                &[TrainingType::Defending][..],
            ),
            (
                PlayerPositionType::MidfielderCenter,
                &[
                    TrainingType::Passing,
                    TrainingType::BallControl,
                    TrainingType::DecisionMaking,
                ][..],
            ),
            (PlayerPositionType::Striker, &[TrainingType::Shooting][..]),
        ] {
            let week = attended(
                position,
                &fixtures,
                saturday.succ_opt().unwrap(),
                fixtures[2],
            );
            assert!(
                week.iter().any(|s| crafts.contains(&s.session_type)),
                "{position:?} had no session of his craft in a congested week"
            );
        }
    }

    #[test]
    fn strength_in_season() {
        for weeks in [1, 2] {
            let fixtures = saturdays(weeks);
            let last_week = attended(
                PlayerPositionType::MidfielderCenter,
                &fixtures,
                fixtures[weeks as usize - 1].succ_opt().unwrap(),
                fixtures[weeks as usize],
            );
            assert!(
                count(&last_week, TrainingType::Strength)
                    + count(&last_week, TrainingType::Agility)
                    >= 1
            );
        }
    }

    /// Over two ordinary one-match weeks a player of every exact position
    /// attends a session practising each attribute his position's ability
    /// scoring weights above the mean.
    #[test]
    fn every_position_every_relied_on_attribute() {
        use PlayerPositionType::*;
        let coach = StaffStub::build();
        let fixtures = saturdays(2);
        let mut missing = Vec::new();
        for position in [
            Goalkeeper,
            Sweeper,
            DefenderLeft,
            DefenderCenter,
            DefenderRight,
            DefensiveMidfielder,
            WingbackLeft,
            WingbackRight,
            MidfielderLeft,
            MidfielderCenter,
            MidfielderRight,
            AttackingMidfielderLeft,
            AttackingMidfielderCenter,
            AttackingMidfielderRight,
            ForwardLeft,
            ForwardCenter,
            ForwardRight,
            Striker,
        ] {
            let player = PlayerBuilder::new()
                .id(1)
                .full_name(FullName::new("Unit".to_string(), "Week".to_string()))
                .birth_date(NaiveDate::from_ymd_opt(2000, 1, 1).unwrap())
                .country_id(1)
                .attributes(PersonAttributes::default())
                .skills(PlayerSkills::flat_for_ability(100))
                .positions(PlayerPositions {
                    positions: vec![PlayerPosition {
                        position,
                        level: 20,
                    }],
                })
                .player_attributes(PlayerAttributes::default())
                .build()
                .unwrap();
            let sessions = attended(
                position,
                &fixtures,
                fixtures[0].succ_opt().unwrap(),
                fixtures[2],
            );
            let at = NaiveDate::from_ymd_opt(2026, 10, 10)
                .unwrap()
                .and_hms_opt(0, 0, 0)
                .unwrap();
            let mut practised = [0.0f32; 50];
            for session in &sessions {
                let work = PlayerTraining::train(&player, &coach, session, at, 0.5)
                    .effects
                    .work;
                for (i, p) in practised.iter_mut().enumerate() {
                    *p += work.reps(SkillKey::ALL[i]);
                }
            }
            let prices = PlayerSkills::ability_weights(position);
            let priced: Vec<usize> = (0..50).filter(|i| prices[*i] > 0.0).collect();
            let mean = priced.iter().map(|i| prices[*i]).sum::<f32>() / priced.len() as f32;
            for i in priced {
                if prices[i] > mean && practised[i] <= 0.0 {
                    missing.push(format!(
                        "{position:?}: {:?} (weight {:.3} over a mean of {mean:.3})",
                        SkillKey::ALL[i],
                        prices[i]
                    ));
                }
            }
        }
        assert!(
            missing.is_empty(),
            "never practised:
{}",
            missing.join(
                "
"
            )
        );
    }
}
