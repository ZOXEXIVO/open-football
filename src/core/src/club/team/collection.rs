use crate::club::team::coach_perception::{
    CoachDecisionState, RecentMoveType, date_to_week, seeded_decision, sigmoid_probability,
};
use crate::context::GlobalContext;
use crate::utils::{DateUtils, Logging};
use crate::{
    ContractType, Player, PlayerFieldPositionGroup, PlayerSquadStatus, PlayerStatusType, Team,
    TeamResult, TeamType,
};
use chrono::NaiveDate;
use log::debug;

#[derive(Debug)]
pub struct TeamCollection {
    pub teams: Vec<Team>,
    pub coach_state: Option<CoachDecisionState>,
}

impl TeamCollection {
    pub fn new(teams: Vec<Team>) -> Self {
        TeamCollection {
            teams,
            coach_state: None,
        }
    }

    pub fn simulate(&mut self, ctx: GlobalContext<'_>) -> Vec<TeamResult> {
        self.teams
            .iter_mut()
            .map(|team| {
                let message = &format!("simulate team: {}", &team.name);
                Logging::estimate_result(|| team.simulate(ctx.with_team(team.id)), message)
            })
            .collect()
    }

    pub fn by_id(&self, id: u32) -> &Team {
        self.teams
            .iter()
            .find(|t| t.id == id)
            .expect(format!("no team with id = {}", id).as_str())
    }

    pub fn main_team_id(&self) -> Option<u32> {
        self.teams
            .iter()
            .find(|t| t.team_type == TeamType::Main)
            .map(|t| t.id)
    }

    pub fn with_league(&self, league_id: u32) -> Vec<u32> {
        self.teams
            .iter()
            .filter(|t| t.league_id == Some(league_id))
            .map(|t| t.id)
            .collect()
    }

    // ─── Coach state management ──────────────────────────────────────

    fn ensure_coach_state(&mut self, date: NaiveDate) {
        let main_team = match self.teams.iter().find(|t| t.team_type == TeamType::Main) {
            Some(t) => t,
            None => return,
        };

        let head_coach = main_team.staffs.head_coach();
        let coach_id = head_coach.id;

        let needs_rebuild = match &self.coach_state {
            Some(state) => state.coach_id != coach_id,
            None => true,
        };

        if needs_rebuild {
            self.coach_state = Some(CoachDecisionState::new(head_coach, date));
        }

        if let Some(ref mut state) = self.coach_state {
            state.current_week = date_to_week(date);
        }
    }

    /// Updates impressions via Option::take(). Decays emotional heat once per cycle.
    fn update_all_impressions(&mut self, date: NaiveDate) {
        let mut state = match self.coach_state.take() {
            Some(s) => s,
            None => return,
        };

        for team in &self.teams {
            for player in &team.players.players {
                state.update_impression(player, date, &team.team_type);
            }
        }

        // Decay emotional heat once per update cycle (not per player)
        state.emotional_heat *= 0.80;

        self.coach_state = Some(state);
    }

    fn record_moves(&mut self, ids: &[u32], move_type: RecentMoveType, date: NaiveDate) {
        if let Some(ref mut state) = self.coach_state {
            for &id in ids {
                state.record_move(id, move_type, date);
            }
        }
    }

    // ─── Weekly squad composition (trigger-based) ────────────────────

    pub fn manage_squad_composition(&mut self, date: NaiveDate) {
        if self.teams.len() < 2 {
            return;
        }

        let main_idx = match self.teams.iter().position(|t| t.team_type == TeamType::Main) {
            Some(idx) => idx,
            None => return,
        };

        let reserve_idx = self.find_reserve_team_index();
        let youth_idx = self.find_youth_team_index();

        self.ensure_coach_state(date);
        self.update_all_impressions(date);

        // --- Trigger detection & pressure-based gating ---
        let should_act = if let Some(ref mut state) = self.coach_state {
            let main_team = &self.teams[main_idx];
            let satisfaction = compute_squad_satisfaction(main_team, state);
            state.squad_satisfaction = satisfaction;
            state.weeks_since_last_change += 1;

            // Decay trigger pressure each cycle
            state.trigger_pressure *= 0.85;

            // Accumulate triggers from observable events
            let injured_count = main_team.players.players.iter()
                .filter(|p| p.player_attributes.is_injured)
                .count();
            if injured_count >= 4 {
                state.trigger_pressure += 0.3;
            }

            let played_players: Vec<_> = main_team.players.players.iter()
                .filter(|p| p.statistics.played + p.statistics.played_subs > 3)
                .collect();
            if !played_players.is_empty() {
                let avg_form: f32 = played_players.iter()
                    .map(|p| p.statistics.average_rating)
                    .sum::<f32>() / played_players.len() as f32;
                if avg_form < 6.0 {
                    state.trigger_pressure += 0.2;
                    state.emotional_heat = (state.emotional_heat + 0.15).clamp(0.0, 1.0);
                }
            }

            // Position emergency: any group with 0 available
            let available: Vec<_> = main_team.players.players.iter()
                .filter(|p| !p.player_attributes.is_injured)
                .collect();
            let has_gk = available.iter().any(|p| p.position().position_group() == PlayerFieldPositionGroup::Goalkeeper);
            let has_def = available.iter().any(|p| p.position().position_group() == PlayerFieldPositionGroup::Defender);
            let has_mid = available.iter().any(|p| p.position().position_group() == PlayerFieldPositionGroup::Midfielder);
            let has_fwd = available.iter().any(|p| p.position().position_group() == PlayerFieldPositionGroup::Forward);
            if !has_gk || !has_def || !has_mid || !has_fwd {
                state.trigger_pressure += 0.4;
            }

            // Time pressure: not looking at squad for too long builds restlessness
            if state.weeks_since_last_change > 6 {
                state.trigger_pressure += (state.weeks_since_last_change - 6) as f32 * 0.05;
            }
            state.trigger_pressure = state.trigger_pressure.clamp(0.0, 1.0);

            // Compute action drive vs inertia pull
            let action_drive = state.trigger_pressure * (1.0 - state.profile.conservatism * 0.3)
                + (1.0 - satisfaction) * 0.3
                + state.emotional_heat * 0.2;
            let inertia_pull = state.profile.conservatism * 0.3
                + satisfaction * 0.2
                + if state.weeks_since_last_change < 3 { 0.3 } else { 0.0 };

            let action_prob = (action_drive - inertia_pull + 0.15).clamp(0.05, 0.95);
            let squad_size = main_team.players.players.len();

            let seed = state.profile.coach_seed
                .wrapping_mul(state.current_week)
                .wrapping_add(0xFA57);

            // Emergency override: always act if squad too small
            if squad_size < 16 {
                true
            } else if seeded_decision(action_prob, seed) {
                true
            } else {
                debug!(
                    "Squad management: coach not triggered (pressure={:.2}, satisfaction={:.2}, heat={:.2})",
                    state.trigger_pressure, satisfaction, state.emotional_heat
                );
                false
            }
        } else {
            true // no coach state → use legacy
        };

        if !should_act {
            return;
        }

        let mut any_move = false;

        // Phase 1: Demotions (main -> reserves)
        if let Some(res_idx) = reserve_idx {
            let demotions = self.identify_demotions_fuzzy(main_idx, date);
            let max_age = self.teams[res_idx].team_type.max_age();
            let demotions = Self::filter_by_age(demotions, &self.teams[main_idx], max_age, date);
            if !demotions.is_empty() {
                debug!(
                    "Squad management: demoting {} players to reserves",
                    demotions.len()
                );
                Self::execute_moves(&mut self.teams, main_idx, res_idx, &demotions);
                self.record_moves(&demotions, RecentMoveType::DemotedToReserves, date);
                any_move = true;
            }
        }

        // Phase 2: Recalls (reserves -> main)
        if let Some(res_idx) = reserve_idx {
            let recalls = self.identify_recalls_fuzzy(main_idx, res_idx, date);
            if !recalls.is_empty() {
                debug!(
                    "Squad management: recalling {} players from reserves",
                    recalls.len()
                );
                Self::execute_moves(&mut self.teams, res_idx, main_idx, &recalls);
                self.record_moves(&recalls, RecentMoveType::RecalledFromReserves, date);
                any_move = true;
            }
        }

        // Phase 3: Youth promotions
        if let Some(y_idx) = youth_idx {
            let promotions = self.identify_youth_promotions_fuzzy(main_idx, y_idx, date);
            if !promotions.is_empty() {
                debug!(
                    "Squad management: promoting {} youth players",
                    promotions.len()
                );
                Self::execute_moves(&mut self.teams, y_idx, main_idx, &promotions);
                self.record_moves(&promotions, RecentMoveType::YouthPromoted, date);
                any_move = true;
            }
        }

        if any_move {
            if let Some(ref mut state) = self.coach_state {
                state.weeks_since_last_change = 0;
            }
        }
    }

    /// Daily critical squad moves: immediate demotions and ability-based swaps
    pub fn manage_critical_squad_moves(&mut self, date: NaiveDate) {
        if self.teams.len() < 2 {
            return;
        }
        let main_idx = match self.teams.iter().position(|t| t.team_type == TeamType::Main) {
            Some(idx) => idx,
            None => return,
        };
        let reserve_idx = match self.find_reserve_team_index() {
            Some(idx) => idx,
            None => return,
        };

        // Phase 1: Immediate demotions (Lst, Loa, NotNeeded) - deterministic
        let demotions = Self::identify_immediate_demotions(&self.teams[main_idx]);
        let max_age = self.teams[reserve_idx].team_type.max_age();
        let demotions = Self::filter_by_age(demotions, &self.teams[main_idx], max_age, date);
        if !demotions.is_empty() {
            debug!(
                "Daily squad moves: demoting {} players immediately",
                demotions.len()
            );
            Self::execute_moves(&mut self.teams, main_idx, reserve_idx, &demotions);
            self.record_moves(&demotions, RecentMoveType::DemotedToReserves, date);
        }

        // Phase 2: Ability-based swaps (fuzzy, emotional urgency lowers threshold)
        self.ensure_coach_state(date);
        let swaps = self.identify_ability_swaps_fuzzy(main_idx, reserve_idx, date);
        if !swaps.is_empty() {
            let (demote_ids, promote_ids): (Vec<u32>, Vec<u32>) = swaps.into_iter().unzip();
            Self::execute_moves(&mut self.teams, main_idx, reserve_idx, &demote_ids);
            Self::execute_moves(&mut self.teams, reserve_idx, main_idx, &promote_ids);
            self.record_moves(&demote_ids, RecentMoveType::SwappedOut, date);
            self.record_moves(&promote_ids, RecentMoveType::SwappedIn, date);
        }
    }

    // ─── Fuzzy identification functions ──────────────────────────────

    /// Fuzzy demotion with patience snap and staleness blindspot
    fn identify_demotions_fuzzy(&self, main_idx: usize, date: NaiveDate) -> Vec<u32> {
        let main_team = &self.teams[main_idx];
        let players = &main_team.players.players;
        let squad_size = players.len();
        let mut demotions = Vec::new();

        if players.is_empty() {
            return demotions;
        }

        let state = match &self.coach_state {
            Some(s) => s,
            None => return Self::legacy_identify_demotions(main_team, date),
        };

        let profile = &state.profile;
        let emotional_heat = state.emotional_heat;

        let avg_quality: f32 = players
            .iter()
            .map(|p| {
                state
                    .impressions
                    .get(&p.id)
                    .map(|imp| imp.perceived_quality)
                    .unwrap_or_else(|| state.perceived_quality(p, date))
            })
            .sum::<f32>()
            / squad_size as f32;

        for player in players {
            let statuses = player.statuses.get();

            // Administrative demotions stay deterministic
            if statuses.contains(&PlayerStatusType::Lst) {
                demotions.push(player.id);
                continue;
            }
            if statuses.contains(&PlayerStatusType::Loa) {
                demotions.push(player.id);
                continue;
            }
            if let Some(ref contract) = player.contract {
                if matches!(contract.squad_status, PlayerSquadStatus::NotNeeded) {
                    demotions.push(player.id);
                    continue;
                }
            }

            // Inertia protection
            if state.is_protected(
                player.id,
                &[
                    RecentMoveType::PromotedToFirst,
                    RecentMoveType::RecalledFromReserves,
                    RecentMoveType::YouthPromoted,
                    RecentMoveType::SwappedIn,
                ],
            ) {
                continue;
            }

            let perceived = state
                .impressions
                .get(&player.id)
                .map(|imp| imp.perceived_quality)
                .unwrap_or_else(|| state.perceived_quality(player, date));

            let coach_trust = state
                .impressions
                .get(&player.id)
                .map(|imp| imp.coach_trust)
                .unwrap_or(5.0);

            let (sunk_cost, disappointments) = state
                .impressions
                .get(&player.id)
                .map(|imp| (imp.bias.sunk_cost, imp.bias.disappointments))
                .unwrap_or((0.0, 0));

            // Staleness blindspot: coach hasn't observed this player recently
            let observation_staleness = state.impressions.get(&player.id)
                .map(|imp| state.current_week.saturating_sub(imp.bias.last_observation_week))
                .unwrap_or(0);
            let staleness_blindness = if observation_staleness > 4 {
                // Coach not paying attention → less likely to notice player is bad
                1.0 - (observation_staleness.min(12) as f32 / 24.0)
            } else {
                1.0
            };

            let age = DateUtils::age(player.birth_date, date);

            // --- Patience snap: emotional coach turns on disappointing player ---
            if emotional_heat > 0.4 && disappointments >= 2 && squad_size > 18 {
                let snap_intensity = (emotional_heat - 0.3) * (disappointments as f32 / 4.0);
                let snap_prob = (snap_intensity * 0.4).clamp(0.0, 0.5);
                let snap_seed = profile.coach_seed
                    .wrapping_mul(player.id)
                    .wrapping_add(state.current_week.wrapping_mul(0xBAAD));
                if seeded_decision(snap_prob, snap_seed) {
                    let player_group = player.position().position_group();
                    let others = players.iter()
                        .filter(|p| {
                            p.id != player.id
                                && p.position().position_group() == player_group
                                && !p.player_attributes.is_injured
                                && !demotions.contains(&p.id)
                        })
                        .count();
                    if others >= 2 {
                        demotions.push(player.id);
                        continue;
                    }
                }
            }

            // Hot prospects / youngsters below average
            if let Some(ref contract) = player.contract {
                if matches!(
                    contract.squad_status,
                    PlayerSquadStatus::HotProspectForTheFuture
                        | PlayerSquadStatus::DecentYoungster
                ) {
                    let youth_protection = profile.youth_preference * 1.5;
                    let gap = avg_quality - perceived - youth_protection;
                    let steepness = 1.5 - profile.conservatism * 0.5;
                    let prob = sigmoid_probability(gap - 1.0, steepness);
                    let trust_factor = 1.0 - (coach_trust / 10.0) * 0.3;
                    let sunk_cost_factor = 1.0 - (sunk_cost / 10.0) * 0.4;
                    let disappointment_factor = if disappointments >= 3 { 1.3 } else { 1.0 };
                    let final_prob = prob * trust_factor * sunk_cost_factor
                        * disappointment_factor * staleness_blindness;

                    if squad_size > 20 {
                        let seed = profile.coach_seed
                            .wrapping_mul(player.id)
                            .wrapping_add(state.current_week);
                        if seeded_decision(final_prob, seed) {
                            demotions.push(player.id);
                            continue;
                        }
                    }
                }
            }

            // Players significantly below squad average
            if squad_size > 20 {
                let gap_required = 3.0 + profile.conservatism * 1.5;
                let gap = avg_quality - perceived;

                let youth_modifier = if age <= 22 {
                    profile.youth_preference * 1.0
                } else {
                    0.0
                };

                let steepness = 1.0 - profile.conservatism * 0.3;
                let prob = sigmoid_probability(gap - gap_required - youth_modifier, steepness);
                let trust_factor = 1.0 - (coach_trust / 10.0) * 0.3;
                let sunk_cost_factor = 1.0 - (sunk_cost / 10.0) * 0.4;
                let disappointment_factor = if disappointments >= 3 { 1.3 } else { 1.0 };
                let final_prob = prob * trust_factor * sunk_cost_factor
                    * disappointment_factor * staleness_blindness;

                let seed = profile.coach_seed
                    .wrapping_mul(player.id)
                    .wrapping_add(state.current_week.wrapping_mul(3));
                if seeded_decision(final_prob, seed) {
                    let player_group = player.position().position_group();
                    let others_in_position = players
                        .iter()
                        .filter(|p| {
                            p.id != player.id
                                && p.position().position_group() == player_group
                                && !p.player_attributes.is_injured
                                && !demotions.contains(&p.id)
                        })
                        .count();
                    if others_in_position >= 2 {
                        demotions.push(player.id);
                        continue;
                    }
                }
            }
        }

        // Force demote if squad > 25
        let remaining = squad_size - demotions.len();
        if remaining > 25 {
            let excess = remaining - 25;
            let mut candidates: Vec<_> = players
                .iter()
                .filter(|p| !demotions.contains(&p.id))
                .map(|p| {
                    let q = state
                        .impressions
                        .get(&p.id)
                        .map(|imp| imp.perceived_quality)
                        .unwrap_or_else(|| state.perceived_quality(p, date));
                    (p.id, q)
                })
                .collect();
            candidates.sort_by(|a, b| {
                a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal)
            });
            for (id, _) in candidates.into_iter().take(excess) {
                demotions.push(id);
            }
        }

        demotions
    }

    /// Fuzzy recall with visibility-based forgetting
    fn identify_recalls_fuzzy(
        &self,
        main_idx: usize,
        reserve_idx: usize,
        date: NaiveDate,
    ) -> Vec<u32> {
        const MAX_SQUAD_SIZE: usize = 25;

        let main_team = &self.teams[main_idx];
        let reserve_team = &self.teams[reserve_idx];
        let main_players = &main_team.players.players;
        let reserve_players = &reserve_team.players.players;
        let mut recalls = Vec::new();

        if reserve_players.is_empty() {
            return recalls;
        }

        let state = match &self.coach_state {
            Some(s) => s,
            None => return Self::legacy_identify_recalls(main_team, reserve_team, date, &[]),
        };

        let profile = &state.profile;

        let recall_budget = MAX_SQUAD_SIZE.saturating_sub(main_players.len());

        // Candidates with visibility-based remember filter
        let mut candidates: Vec<&Player> = reserve_players
            .iter()
            .filter(|p| {
                let statuses = p.statuses.get();
                if statuses.contains(&PlayerStatusType::Lst)
                    || statuses.contains(&PlayerStatusType::Loa)
                    || p.player_attributes.is_injured
                    || matches!(
                        p.contract.as_ref().map(|c| &c.contract_type),
                        Some(ContractType::Loan)
                    )
                    || p.player_attributes.condition_percentage() <= 40
                    || state.is_protected(
                        p.id,
                        &[RecentMoveType::DemotedToReserves, RecentMoveType::SwappedOut],
                    )
                {
                    return false;
                }

                // Visibility filter: coach only recalls players they remember
                let vis = state.impressions.get(&p.id)
                    .map(|imp| imp.bias.visibility)
                    .unwrap_or(0.3);
                let remember_prob = (vis * 0.5 + 0.5).clamp(0.5, 1.0);
                let rem_seed = profile.coach_seed
                    .wrapping_mul(p.id)
                    .wrapping_add(state.current_week.wrapping_mul(17));
                seeded_decision(remember_prob, rem_seed)
            })
            .collect();

        // Recall score with visibility factor
        let recall_score = |p: &Player| -> f32 {
            let perceived = state
                .impressions
                .get(&p.id)
                .map(|imp| imp.perceived_quality)
                .unwrap_or_else(|| state.perceived_quality(p, date));
            let readiness = state
                .impressions
                .get(&p.id)
                .map(|imp| imp.match_readiness)
                .unwrap_or_else(|| state.match_readiness(p));
            let trust = state
                .impressions
                .get(&p.id)
                .map(|imp| imp.coach_trust)
                .unwrap_or(5.0);
            let visibility = state
                .impressions
                .get(&p.id)
                .map(|imp| imp.bias.visibility)
                .unwrap_or(0.5);

            let status_bonus = match p.contract.as_ref().map(|c| &c.squad_status) {
                Some(PlayerSquadStatus::KeyPlayer) => 3.0,
                Some(PlayerSquadStatus::FirstTeamRegular) => 2.0,
                Some(PlayerSquadStatus::FirstTeamSquadRotation) => 1.0,
                Some(PlayerSquadStatus::MainBackupPlayer) => 0.5,
                Some(PlayerSquadStatus::HotProspectForTheFuture) => {
                    0.3 + profile.youth_preference * 1.0
                }
                Some(PlayerSquadStatus::DecentYoungster) => {
                    0.1 + profile.youth_preference * 0.5
                }
                Some(PlayerSquadStatus::NotNeeded) => -5.0,
                _ => 0.0,
            };

            perceived * 0.4 + readiness * 0.3 + (trust / 10.0) * 3.0 * 0.15
                + status_bonus * 0.15 + visibility * 2.0 * 0.10
        };

        candidates.sort_by(|a, b| {
            recall_score(b)
                .partial_cmp(&recall_score(a))
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        // Position needs
        let available_main: Vec<&Player> = main_players
            .iter()
            .filter(|p| !p.player_attributes.is_injured)
            .collect();

        let count_by_group = |group: PlayerFieldPositionGroup| -> usize {
            available_main
                .iter()
                .filter(|p| p.position().position_group() == group)
                .count()
        };

        let gk_count = count_by_group(PlayerFieldPositionGroup::Goalkeeper);
        let def_count = count_by_group(PlayerFieldPositionGroup::Defender);
        let mid_count = count_by_group(PlayerFieldPositionGroup::Midfielder);
        let fwd_count = count_by_group(PlayerFieldPositionGroup::Forward);

        let tactics = main_team.tactics();
        let positions = tactics.positions();
        let def_need = positions.iter().filter(|p| p.is_defender()).count() + 1;
        let mid_need = positions.iter().filter(|p| p.is_midfielder()).count() + 1;
        let fwd_need = positions.iter().filter(|p| p.is_forward()).count() + 1;

        let position_needs = [
            (PlayerFieldPositionGroup::Goalkeeper, gk_count, 2usize),
            (PlayerFieldPositionGroup::Defender, def_count, def_need),
            (PlayerFieldPositionGroup::Midfielder, mid_count, mid_need),
            (PlayerFieldPositionGroup::Forward, fwd_count, fwd_need),
        ];

        for (group, count, min) in &position_needs {
            if recalls.len() >= recall_budget {
                break;
            }
            if *count < *min {
                let needed = min - count;
                let mut recalled = 0;
                for candidate in &candidates {
                    if recalled >= needed || recalls.len() >= recall_budget {
                        break;
                    }
                    if candidate.position().position_group() == *group
                        && !recalls.contains(&candidate.id)
                    {
                        recalls.push(candidate.id);
                        recalled += 1;
                    }
                }
            }
        }

        // Squad below 18
        let current_main_size = main_players.len() + recalls.len();
        if current_main_size < 18 {
            let needed = (18 - current_main_size).min(recall_budget.saturating_sub(recalls.len()));
            let mut recalled = 0;
            for candidate in &candidates {
                if recalled >= needed {
                    break;
                }
                if !recalls.contains(&candidate.id) {
                    recalls.push(candidate.id);
                    recalled += 1;
                }
            }
        }

        // Emergency recalls (<14 available) — bypasses visibility filter
        let total_available = available_main.len() + recalls.len();
        if total_available < 14 {
            let needed = 14 - total_available;
            let mut emergency_candidates: Vec<&Player> = reserve_players
                .iter()
                .filter(|p| {
                    let statuses = p.statuses.get();
                    !statuses.contains(&PlayerStatusType::Lst)
                        && !statuses.contains(&PlayerStatusType::Loa)
                        && !p.player_attributes.is_injured
                        && !recalls.contains(&p.id)
                        && !matches!(
                            p.contract.as_ref().map(|c| &c.contract_type),
                            Some(ContractType::Loan)
                        )
                })
                .collect();
            emergency_candidates.sort_by(|a, b| {
                recall_score(b)
                    .partial_cmp(&recall_score(a))
                    .unwrap_or(std::cmp::Ordering::Equal)
            });
            for candidate in emergency_candidates.into_iter().take(needed) {
                recalls.push(candidate.id);
            }
        }

        recalls
    }

    /// Fuzzy youth promotion with spotlight moments and philosophy bonus
    fn identify_youth_promotions_fuzzy(
        &self,
        main_idx: usize,
        youth_idx: usize,
        date: NaiveDate,
    ) -> Vec<u32> {
        let main_team = &self.teams[main_idx];
        let youth_team = &self.teams[youth_idx];
        let main_size = main_team.players.players.len();
        let mut promotions = Vec::new();

        let state = match &self.coach_state {
            Some(s) => s,
            None => return Self::legacy_identify_youth_promotions(main_team, youth_team, date),
        };

        let profile = &state.profile;

        let promotion_ceiling = (18.0 + profile.youth_preference * 4.0) as usize;
        if main_size >= promotion_ceiling {
            return promotions;
        }

        let needed = promotion_ceiling - main_size;

        let avg_perceived: f32 = if main_team.players.players.is_empty() {
            10.0
        } else {
            main_team
                .players
                .players
                .iter()
                .map(|p| {
                    state
                        .impressions
                        .get(&p.id)
                        .map(|imp| imp.perceived_quality)
                        .unwrap_or_else(|| state.perceived_quality(p, date))
                })
                .sum::<f32>()
                / main_team.players.players.len() as f32
        };

        // Philosophy bonus: youth-loving coaches with thin squads lower the bar
        let philosophy_bonus = if profile.youth_preference > 0.6 && main_size < 20 {
            (profile.youth_preference - 0.5) * 3.0
        } else {
            0.0
        };

        let threshold = avg_perceived - 2.0 - profile.risk_tolerance * 2.0 - philosophy_bonus;

        // Promotion candidates (spotlight is already in potential_impression)
        let mut candidates: Vec<(&Player, f32)> = youth_team
            .players
            .players
            .iter()
            .filter_map(|p| {
                let age = DateUtils::age(p.birth_date, date);
                if age < 16 || p.player_attributes.is_injured
                    || p.player_attributes.condition_percentage() <= 40
                {
                    return None;
                }

                let potential = state
                    .impressions
                    .get(&p.id)
                    .map(|imp| imp.potential_impression)
                    .unwrap_or_else(|| state.potential_impression(p, date));

                let quality = state
                    .impressions
                    .get(&p.id)
                    .map(|imp| imp.perceived_quality)
                    .unwrap_or_else(|| state.perceived_quality(p, date));

                let training = state
                    .impressions
                    .get(&p.id)
                    .map(|imp| imp.training_impression)
                    .unwrap_or_else(|| state.training_impression(p));

                let score = potential * 0.4 + quality * 0.3 + training * 0.3;

                let steepness = 1.0 + profile.risk_tolerance * 0.5;
                let prob = sigmoid_probability(score - threshold, steepness);

                let seed = profile.coach_seed
                    .wrapping_mul(p.id)
                    .wrapping_add(state.current_week.wrapping_mul(7));

                if seeded_decision(prob, seed) {
                    Some((p, score))
                } else {
                    None
                }
            })
            .collect();

        candidates.sort_by(|a, b| {
            b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal)
        });

        for (candidate, _) in candidates.into_iter().take(needed) {
            promotions.push(candidate.id);
        }

        promotions
    }

    /// Fuzzy ability swap with emotional urgency
    fn identify_ability_swaps_fuzzy(
        &self,
        main_idx: usize,
        reserve_idx: usize,
        date: NaiveDate,
    ) -> Vec<(u32, u32)> {
        let main_team = &self.teams[main_idx];
        let reserve_team = &self.teams[reserve_idx];

        let state = match &self.coach_state {
            Some(s) => s,
            None => return Self::legacy_identify_ability_swaps(main_team, reserve_team, date),
        };

        let profile = &state.profile;

        let max_swaps = (2.0 * (1.0 - profile.conservatism * 0.5)).ceil() as usize;

        // Emotional urgency lowers swap threshold (panic changes)
        let emotional_urgency = state.emotional_heat * 0.5;
        let swap_threshold = (1.5 + profile.conservatism * 1.5 - emotional_urgency).max(0.5);

        let mut swaps = Vec::new();
        let mut used_main = Vec::new();
        let mut used_reserve = Vec::new();

        let reserve_candidates: Vec<&Player> = reserve_team
            .players
            .players
            .iter()
            .filter(|p| {
                let st = p.statuses.get();
                !p.player_attributes.is_injured
                    && !p.player_attributes.is_banned
                    && !st.contains(&PlayerStatusType::Lst)
                    && !st.contains(&PlayerStatusType::Loa)
                    && !matches!(
                        p.contract.as_ref().map(|c| &c.contract_type),
                        Some(ContractType::Loan)
                    )
                    && p.player_attributes.condition_percentage() > 50
            })
            .collect();

        let swap_score = |p: &Player| -> f32 {
            let perceived = state
                .impressions
                .get(&p.id)
                .map(|imp| imp.perceived_quality)
                .unwrap_or_else(|| state.perceived_quality(p, date));
            let readiness = state
                .impressions
                .get(&p.id)
                .map(|imp| imp.match_readiness)
                .unwrap_or_else(|| state.match_readiness(p));
            perceived * 0.7 + readiness * 0.3
        };

        for group in &[
            PlayerFieldPositionGroup::Goalkeeper,
            PlayerFieldPositionGroup::Defender,
            PlayerFieldPositionGroup::Midfielder,
            PlayerFieldPositionGroup::Forward,
        ] {
            if swaps.len() >= max_swaps {
                break;
            }

            let mut main_group: Vec<&Player> = main_team
                .players
                .players
                .iter()
                .filter(|p| {
                    p.position().position_group() == *group
                        && !used_main.contains(&p.id)
                        && !p.statuses.get().contains(&PlayerStatusType::Lst)
                        && !state.is_protected(
                            p.id,
                            &[
                                RecentMoveType::SwappedIn,
                                RecentMoveType::RecalledFromReserves,
                                RecentMoveType::YouthPromoted,
                            ],
                        )
                })
                .collect();
            main_group.sort_by(|a, b| {
                swap_score(a)
                    .partial_cmp(&swap_score(b))
                    .unwrap_or(std::cmp::Ordering::Equal)
            });

            let mut res_group: Vec<&&Player> = reserve_candidates
                .iter()
                .filter(|p| {
                    p.position().position_group() == *group && !used_reserve.contains(&p.id)
                })
                .collect();
            res_group.sort_by(|a, b| {
                swap_score(b)
                    .partial_cmp(&swap_score(a))
                    .unwrap_or(std::cmp::Ordering::Equal)
            });

            for main_p in &main_group {
                if res_group.is_empty() || swaps.len() >= max_swaps {
                    break;
                }
                let best_res = res_group[0];
                let main_score = swap_score(main_p);
                let res_score = swap_score(best_res);
                let gap = res_score - main_score;

                let steepness = 1.0 + (1.0 - profile.conservatism) * 0.5;
                let prob = sigmoid_probability(gap - swap_threshold, steepness);

                let seed = profile.coach_seed
                    .wrapping_mul(main_p.id)
                    .wrapping_add(best_res.id)
                    .wrapping_add(state.current_week.wrapping_mul(11));

                if seeded_decision(prob, seed) {
                    swaps.push((main_p.id, best_res.id));
                    used_main.push(main_p.id);
                    used_reserve.push(best_res.id);
                    res_group.remove(0);
                } else {
                    break;
                }
            }
        }

        swaps
    }

    // ─── Helper functions ────────────────────────────────────────────

    fn find_reserve_team_index(&self) -> Option<usize> {
        self.teams.iter().position(|t| t.team_type == TeamType::B)
            .or_else(|| self.teams.iter().position(|t| t.team_type == TeamType::U23))
            .or_else(|| self.teams.iter().position(|t| t.team_type == TeamType::U21))
            .or_else(|| self.teams.iter().position(|t| t.team_type == TeamType::U19))
            .or_else(|| self.teams.iter().position(|t| t.team_type == TeamType::U18))
    }

    fn find_youth_team_index(&self) -> Option<usize> {
        self.teams.iter().position(|t| t.team_type == TeamType::U18)
            .or_else(|| self.teams.iter().position(|t| t.team_type == TeamType::U19))
    }

    fn identify_immediate_demotions(main_team: &Team) -> Vec<u32> {
        main_team
            .players
            .players
            .iter()
            .filter_map(|player| {
                let statuses = player.statuses.get();
                if statuses.contains(&PlayerStatusType::Lst)
                    || statuses.contains(&PlayerStatusType::Loa)
                {
                    return Some(player.id);
                }
                if let Some(ref contract) = player.contract {
                    if matches!(contract.squad_status, PlayerSquadStatus::NotNeeded) {
                        return Some(player.id);
                    }
                }
                None
            })
            .collect()
    }

    fn filter_by_age(
        ids: Vec<u32>,
        team: &Team,
        max_age: Option<u8>,
        date: NaiveDate,
    ) -> Vec<u32> {
        match max_age {
            Some(max) => ids
                .into_iter()
                .filter(|&pid| {
                    team.players
                        .players
                        .iter()
                        .find(|p| p.id == pid)
                        .map(|p| DateUtils::age(p.birth_date, date) <= max)
                        .unwrap_or(false)
                })
                .collect(),
            None => ids,
        }
    }

    fn execute_moves(teams: &mut [Team], from_idx: usize, to_idx: usize, player_ids: &[u32]) {
        for &player_id in player_ids {
            if let Some(player) = teams[from_idx].players.take_player(&player_id) {
                teams[from_idx].transfer_list.remove(player_id);
                teams[to_idx].players.add(player);
            }
        }
    }

    // ─── Legacy functions ────────────────────────────────────────────

    fn legacy_estimate_player_quality(player: &Player) -> f32 {
        let tech = player.skills.technical.average();
        let mental = player.skills.mental.average();
        let physical = player.skills.physical.average();
        let skill_composite = tech * 0.40 + mental * 0.35 + physical * 0.25;
        let position_level = player.positions.positions.iter()
            .map(|p| p.level).max().unwrap_or(0) as f32;
        let base = skill_composite * 0.75 + position_level * 0.25;
        let form_bonus = if player.statistics.played + player.statistics.played_subs > 3 {
            (player.statistics.average_rating - 6.5).clamp(-1.5, 1.5)
        } else {
            0.0
        };
        let noise = ((player.id.wrapping_mul(2654435761)) >> 24) as f32 / 128.0 - 1.0;
        base + form_bonus + noise
    }

    fn legacy_estimate_youth_potential(player: &Player, date: NaiveDate) -> f32 {
        let quality = Self::legacy_estimate_player_quality(player);
        let age = DateUtils::age(player.birth_date, date);
        let age_bonus = match age {
            0..=15 => 1.5, 16..=17 => 2.5, 18 => 3.0, 19..=20 => 2.0, 21..=22 => 1.0, _ => 0.0,
        };
        let attitude = (player.attributes.professionalism + player.skills.mental.determination) / 2.0;
        let attitude_bonus = (attitude - 10.0).clamp(-1.0, 2.0) * 0.5;
        quality + age_bonus + attitude_bonus
    }

    fn legacy_recall_priority_score(player: &Player) -> f32 {
        let quality = Self::legacy_estimate_player_quality(player);
        let status_bonus = match player.contract.as_ref().map(|c| &c.squad_status) {
            Some(PlayerSquadStatus::KeyPlayer) => 3.0,
            Some(PlayerSquadStatus::FirstTeamRegular) => 2.0,
            Some(PlayerSquadStatus::FirstTeamSquadRotation) => 1.0,
            Some(PlayerSquadStatus::MainBackupPlayer) => 0.5,
            Some(PlayerSquadStatus::HotProspectForTheFuture) => 0.3,
            Some(PlayerSquadStatus::DecentYoungster) => 0.1,
            Some(PlayerSquadStatus::NotNeeded) => -5.0,
            _ => 0.0,
        };
        let condition = (player.player_attributes.condition_percentage() as f32 / 100.0).clamp(0.3, 1.0);
        (quality + status_bonus) * condition
    }

    fn legacy_identify_demotions(main_team: &Team, _date: NaiveDate) -> Vec<u32> {
        let players = &main_team.players.players;
        let squad_size = players.len();
        let mut demotions = Vec::new();
        if players.is_empty() { return demotions; }

        let avg_quality: f32 = players.iter()
            .map(|p| Self::legacy_estimate_player_quality(p))
            .sum::<f32>() / squad_size as f32;

        for player in players {
            let statuses = player.statuses.get();
            if statuses.contains(&PlayerStatusType::Lst) { demotions.push(player.id); continue; }
            if statuses.contains(&PlayerStatusType::Loa) { demotions.push(player.id); continue; }
            if let Some(ref contract) = player.contract {
                if matches!(contract.squad_status, PlayerSquadStatus::NotNeeded) {
                    demotions.push(player.id); continue;
                }
            }
            if let Some(ref contract) = player.contract {
                if matches!(contract.squad_status,
                    PlayerSquadStatus::HotProspectForTheFuture | PlayerSquadStatus::DecentYoungster
                ) {
                    let quality = Self::legacy_estimate_player_quality(player);
                    if quality < avg_quality - 1.0 && squad_size > 20 {
                        demotions.push(player.id); continue;
                    }
                }
            }
            if squad_size > 20 {
                let quality = Self::legacy_estimate_player_quality(player);
                if quality < avg_quality - 3.0 {
                    let player_group = player.position().position_group();
                    let others = players.iter()
                        .filter(|p| p.id != player.id
                            && p.position().position_group() == player_group
                            && !p.player_attributes.is_injured)
                        .count();
                    if others >= 2 { demotions.push(player.id); continue; }
                }
            }
        }

        let remaining = squad_size - demotions.len();
        if remaining > 25 {
            let excess = remaining - 25;
            let mut candidates: Vec<_> = players.iter()
                .filter(|p| !demotions.contains(&p.id))
                .map(|p| (p.id, Self::legacy_estimate_player_quality(p)))
                .collect();
            candidates.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal));
            for (id, _) in candidates.into_iter().take(excess) { demotions.push(id); }
        }
        demotions
    }

    fn legacy_identify_recalls(
        main_team: &Team, reserve_team: &Team, _date: NaiveDate, excluded_ids: &[u32],
    ) -> Vec<u32> {
        const MAX_SQUAD_SIZE: usize = 25;
        let main_players = &main_team.players.players;
        let reserve_players = &reserve_team.players.players;
        let mut recalls = Vec::new();
        if reserve_players.is_empty() { return recalls; }

        let recall_budget = MAX_SQUAD_SIZE.saturating_sub(main_players.len());
        let mut candidates: Vec<&Player> = reserve_players.iter()
            .filter(|p| {
                let statuses = p.statuses.get();
                !statuses.contains(&PlayerStatusType::Lst)
                    && !statuses.contains(&PlayerStatusType::Loa)
                    && !p.player_attributes.is_injured
                    && !matches!(p.contract.as_ref().map(|c| &c.contract_type), Some(ContractType::Loan))
                    && p.player_attributes.condition_percentage() > 40
                    && !excluded_ids.contains(&p.id)
            }).collect();
        candidates.sort_by(|a, b| {
            Self::legacy_recall_priority_score(b).partial_cmp(&Self::legacy_recall_priority_score(a))
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        let available_main: Vec<&Player> = main_players.iter()
            .filter(|p| !p.player_attributes.is_injured).collect();
        let count_by_group = |group: PlayerFieldPositionGroup| -> usize {
            available_main.iter().filter(|p| p.position().position_group() == group).count()
        };
        let gk_count = count_by_group(PlayerFieldPositionGroup::Goalkeeper);
        let def_count = count_by_group(PlayerFieldPositionGroup::Defender);
        let mid_count = count_by_group(PlayerFieldPositionGroup::Midfielder);
        let fwd_count = count_by_group(PlayerFieldPositionGroup::Forward);

        let tactics = main_team.tactics();
        let positions = tactics.positions();
        let def_need = positions.iter().filter(|p| p.is_defender()).count() + 1;
        let mid_need = positions.iter().filter(|p| p.is_midfielder()).count() + 1;
        let fwd_need = positions.iter().filter(|p| p.is_forward()).count() + 1;
        let position_needs = [
            (PlayerFieldPositionGroup::Goalkeeper, gk_count, 2usize),
            (PlayerFieldPositionGroup::Defender, def_count, def_need),
            (PlayerFieldPositionGroup::Midfielder, mid_count, mid_need),
            (PlayerFieldPositionGroup::Forward, fwd_count, fwd_need),
        ];

        for (group, count, min) in &position_needs {
            if recalls.len() >= recall_budget { break; }
            if *count < *min {
                let needed = min - count;
                let mut recalled = 0;
                for candidate in &candidates {
                    if recalled >= needed || recalls.len() >= recall_budget { break; }
                    if candidate.position().position_group() == *group && !recalls.contains(&candidate.id) {
                        recalls.push(candidate.id); recalled += 1;
                    }
                }
            }
        }

        let current_main_size = main_players.len() + recalls.len();
        if current_main_size < 18 {
            let needed = (18 - current_main_size).min(recall_budget.saturating_sub(recalls.len()));
            let mut recalled = 0;
            for candidate in &candidates {
                if recalled >= needed { break; }
                if !recalls.contains(&candidate.id) { recalls.push(candidate.id); recalled += 1; }
            }
        }

        let total_available = available_main.len() + recalls.len();
        if total_available < 14 {
            let needed = 14 - total_available;
            let mut emergency: Vec<&Player> = reserve_players.iter()
                .filter(|p| {
                    let st = p.statuses.get();
                    !st.contains(&PlayerStatusType::Lst) && !st.contains(&PlayerStatusType::Loa)
                        && !p.player_attributes.is_injured && !recalls.contains(&p.id)
                        && !excluded_ids.contains(&p.id)
                        && !matches!(p.contract.as_ref().map(|c| &c.contract_type), Some(ContractType::Loan))
                }).collect();
            emergency.sort_by(|a, b| {
                Self::legacy_estimate_player_quality(b).partial_cmp(&Self::legacy_estimate_player_quality(a))
                    .unwrap_or(std::cmp::Ordering::Equal)
            });
            for candidate in emergency.into_iter().take(needed) { recalls.push(candidate.id); }
        }
        recalls
    }

    fn legacy_identify_youth_promotions(main_team: &Team, youth_team: &Team, date: NaiveDate) -> Vec<u32> {
        let main_size = main_team.players.players.len();
        let mut promotions = Vec::new();
        if main_size >= 18 { return promotions; }
        let needed = 18 - main_size;

        let avg_quality: f32 = if main_team.players.players.is_empty() { 10.0 } else {
            main_team.players.players.iter()
                .map(|p| Self::legacy_estimate_player_quality(p)).sum::<f32>()
                / main_team.players.players.len() as f32
        };

        let mut candidates: Vec<&Player> = youth_team.players.players.iter()
            .filter(|p| {
                let age = DateUtils::age(p.birth_date, date);
                let quality = Self::legacy_estimate_player_quality(p);
                let youth_potential = Self::legacy_estimate_youth_potential(p, date);
                age >= 16 && !p.player_attributes.is_injured
                    && p.player_attributes.condition_percentage() > 40
                    && (quality >= avg_quality - 2.0 || youth_potential > avg_quality + 2.0)
            }).collect();
        candidates.sort_by(|a, b| {
            Self::legacy_estimate_youth_potential(b, date)
                .partial_cmp(&Self::legacy_estimate_youth_potential(a, date))
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        for candidate in candidates.into_iter().take(needed) { promotions.push(candidate.id); }
        promotions
    }

    fn legacy_identify_ability_swaps(main_team: &Team, reserve_team: &Team, _date: NaiveDate) -> Vec<(u32, u32)> {
        const SWAP_THRESHOLD: f32 = 2.0;
        let mut swaps = Vec::new();
        let mut used_main = Vec::new();
        let mut used_reserve = Vec::new();

        let reserve_candidates: Vec<&Player> = reserve_team.players.players.iter()
            .filter(|p| {
                let st = p.statuses.get();
                !p.player_attributes.is_injured && !p.player_attributes.is_banned
                    && !st.contains(&PlayerStatusType::Lst) && !st.contains(&PlayerStatusType::Loa)
                    && !matches!(p.contract.as_ref().map(|c| &c.contract_type), Some(ContractType::Loan))
                    && p.player_attributes.condition_percentage() > 50
            }).collect();

        for group in &[
            PlayerFieldPositionGroup::Goalkeeper, PlayerFieldPositionGroup::Defender,
            PlayerFieldPositionGroup::Midfielder, PlayerFieldPositionGroup::Forward,
        ] {
            let mut main_group: Vec<&Player> = main_team.players.players.iter()
                .filter(|p| p.position().position_group() == *group
                    && !used_main.contains(&p.id)
                    && !p.statuses.get().contains(&PlayerStatusType::Lst))
                .collect();
            main_group.sort_by(|a, b| {
                Self::legacy_estimate_player_quality(a).partial_cmp(&Self::legacy_estimate_player_quality(b))
                    .unwrap_or(std::cmp::Ordering::Equal)
            });
            let mut res_group: Vec<&&Player> = reserve_candidates.iter()
                .filter(|p| p.position().position_group() == *group && !used_reserve.contains(&p.id))
                .collect();
            res_group.sort_by(|a, b| {
                Self::legacy_estimate_player_quality(b).partial_cmp(&Self::legacy_estimate_player_quality(a))
                    .unwrap_or(std::cmp::Ordering::Equal)
            });

            for main_p in &main_group {
                if res_group.is_empty() { break; }
                let best_res = res_group[0];
                if Self::legacy_estimate_player_quality(best_res)
                    > Self::legacy_estimate_player_quality(main_p) + SWAP_THRESHOLD
                {
                    swaps.push((main_p.id, best_res.id));
                    used_main.push(main_p.id); used_reserve.push(best_res.id);
                    res_group.remove(0);
                } else { break; }
            }
        }
        swaps
    }
}

// ─── Free functions ──────────────────────────────────────────────────

fn compute_squad_satisfaction(main_team: &Team, state: &CoachDecisionState) -> f32 {
    let players = &main_team.players.players;
    let squad_size = players.len();

    let size_satisfaction = if (20..=23).contains(&squad_size) { 1.0 }
        else if squad_size >= 18 && squad_size <= 25 { 0.7 }
        else if squad_size >= 14 { 0.4 }
        else { 0.1 };

    let played_players: Vec<&Player> = players.iter()
        .filter(|p| p.statistics.played + p.statistics.played_subs > 3).collect();
    let perf_satisfaction = if played_players.is_empty() { 0.5 } else {
        let avg_rating: f32 = played_players.iter()
            .map(|p| p.statistics.average_rating).sum::<f32>() / played_players.len() as f32;
        ((avg_rating - 5.5) / 2.0).clamp(0.0, 1.0)
    };

    let qualities: Vec<f32> = players.iter()
        .filter_map(|p| state.impressions.get(&p.id).map(|imp| imp.perceived_quality))
        .collect();
    let spread_satisfaction = if qualities.len() >= 2 {
        let max_q = qualities.iter().cloned().fold(f32::NEG_INFINITY, f32::max);
        let min_q = qualities.iter().cloned().fold(f32::INFINITY, f32::min);
        (1.0 - (max_q - min_q) / 10.0).clamp(0.0, 1.0)
    } else { 0.5 };

    let available: Vec<_> = players.iter().filter(|p| !p.player_attributes.is_injured).collect();
    let has_gk = available.iter().any(|p| p.position().position_group() == PlayerFieldPositionGroup::Goalkeeper);
    let has_def = available.iter().filter(|p| p.position().position_group() == PlayerFieldPositionGroup::Defender).count() >= 3;
    let has_mid = available.iter().filter(|p| p.position().position_group() == PlayerFieldPositionGroup::Midfielder).count() >= 2;
    let has_fwd = available.iter().filter(|p| p.position().position_group() == PlayerFieldPositionGroup::Forward).count() >= 1;
    let coverage_satisfaction = if has_gk && has_def && has_mid && has_fwd { 1.0 } else { 0.2 };

    size_satisfaction * 0.25 + perf_satisfaction * 0.35
        + spread_satisfaction * 0.15 + coverage_satisfaction * 0.25
}
