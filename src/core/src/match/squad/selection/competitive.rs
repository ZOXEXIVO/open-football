use crate::club::{PlayerPositionType, Staff};
use crate::r#match::player::MatchPlayer;
use crate::utils::DateUtils;
use crate::{Player, PlayerSquadStatus, Tactics};
use chrono::NaiveDate;
use log::debug;

use super::cup_rotation::CupRotation;
use super::helpers;
use super::scoring::ScoringEngine;
use super::{CupStage, DomesticCupContext, SelectionPolicy};
use chrono::Utc;
use std::cmp::Ordering;

/// Read-only scoring inputs every selector step needs, bundled so the call
/// chain isn't a wall of positional arguments. All fields are borrows or small
/// `Copy` values, so the context is cheap to pass around. The competitive
/// selection algorithm hangs off this type as methods rather than free
/// functions threading the same arguments everywhere.
#[derive(Clone, Copy)]
pub(crate) struct SelectionScoringContext<'a> {
    pub staff: &'a Staff,
    pub tactics: &'a Tactics,
    pub engine: &'a ScoringEngine,
    pub date: NaiveDate,
    pub is_friendly: bool,
    pub match_importance: f32,
    pub policy: SelectionPolicy,
    pub cup: Option<&'a DomesticCupContext>,
}

impl SelectionScoringContext<'_> {
    /// Domestic-cup opportunity bonus for `player`, or 0.0 outside a cup tie.
    fn cup_opportunity(&self, player: &Player, for_starting: bool) -> f32 {
        self.cup
            .map(|c| {
                self.engine
                    .domestic_cup_opportunity_bonus(player, c, for_starting)
            })
            .unwrap_or(0.0)
    }

    /// Domestic-cup goalkeeper adjustment for `player`, or 0.0 outside a cup tie.
    fn cup_goalkeeper(&self, player: &Player) -> f32 {
        self.cup
            .map(|c| self.engine.domestic_cup_goalkeeper_adjustment(player, c))
            .unwrap_or(0.0)
    }

    /// Future-aware pathway adjustment for a starting-XI slot. `available` is
    /// the full pool, used for the same-role quality / successor checks.
    fn future_pathway_start(
        &self,
        player: &Player,
        slot: PlayerPositionType,
        available: &[&Player],
    ) -> f32 {
        self.engine.future_pathway_adjustment(
            player,
            slot,
            self.match_importance,
            self.date,
            self.cup,
            available,
            true,
        )
    }

    /// Future-aware pathway adjustment for the bench. Deliberately passes an
    /// empty same-role pool: the bench skips the starting gap gate so a
    /// not-quite-ready prospect (who would lose the same-role contest for a
    /// start) still earns a place on the matchday bench.
    fn future_pathway_bench(&self, player: &Player) -> f32 {
        let slot = helpers::best_tactical_position(player, self.tactics);
        self.engine.future_pathway_adjustment(
            player,
            slot,
            self.match_importance,
            self.date,
            self.cup,
            &[],
            false,
        )
    }

    /// Select the best starting 11 for competitive matches.
    pub(crate) fn select_starting_eleven(
        &self,
        team_id: u32,
        available: &[&Player],
    ) -> Vec<MatchPlayer> {
        let mut squad: Vec<MatchPlayer> = Vec::with_capacity(helpers::DEFAULT_SQUAD_SIZE);
        let mut used_ids: Vec<u32> = Vec::new();
        let mut selected_players: Vec<&Player> = Vec::new();
        let required = self.tactics.positions();

        // STEP 1: Goalkeeper. Fallback order:
        //   1. Best available keeper (fit, not injured, not on int duty).
        //   2. Any keeper in the available pool, even if low-condition — we
        //      normally reject those, but a tired keeper still has real
        //      goalkeeping skills and saves far more than an outfielder
        //      pressed into goal. Skipped only if the keeper is actively
        //      injured or banned (those shouldn't play at all).
        //   3. Last resort: outfielder as emergency keeper. Real football
        //      does this but the result is a 5+ goal concession — so we
        //      reach this only when the club has literally no keeper on
        //      the roster.
        //
        // The Goalkeeping struct on an outfielder defaults to all zeros
        // (never trained as a keeper), which previously produced hnd=1
        // ref=1 after the (x-1)/19 scaling clamp — save rate effectively
        // 0%, and the league generated repeatable 10+ goal blowouts. This
        // order keeps a real keeper in goal whenever possible.
        let picked_gk = self
            .pick_best_goalkeeper(available, &used_ids)
            .or_else(|| Self::pick_any_goalkeeper_fallback(available, &used_ids));
        if let Some(gk) = picked_gk {
            squad.push(MatchPlayer::from_player(
                team_id,
                gk,
                PlayerPositionType::Goalkeeper,
                false,
            ));
            used_ids.push(gk.id);
            selected_players.push(gk);
        } else {
            debug!("No goalkeeper found at all — picking any player as GK");
            if let Some(any) = helpers::pick_best_unused(available, &used_ids) {
                squad.push(MatchPlayer::from_player(
                    team_id,
                    any,
                    PlayerPositionType::Goalkeeper,
                    false,
                ));
                used_ids.push(any.id);
                selected_players.push(any);
            }
        }

        // STEP 2: Fill outfield positions as one assignment problem. This avoids
        // burning a versatile player in an early slot when a specialist is needed
        // later in the shape.
        let outfield_slots: Vec<PlayerPositionType> = required
            .iter()
            .copied()
            .filter(|p| *p != PlayerPositionType::Goalkeeper)
            .collect();
        let assignments = self.assign_outfield_slots(available, &used_ids, &outfield_slots);

        for (pos, player) in assignments {
            squad.push(MatchPlayer::from_player(team_id, player, pos, false));
            used_ids.push(player.id);
            selected_players.push(player);
        }

        // STEP 3: Fill remaining slots with best available
        while squad.len() < helpers::DEFAULT_SQUAD_SIZE {
            let best = available
                .iter()
                .filter(|p| !used_ids.contains(&p.id))
                .filter(|p| !p.positions.is_goalkeeper())
                .max_by(|a, b| {
                    let score = |p: &Player| {
                        let slot = helpers::best_tactical_position(p, self.tactics);
                        self.engine.overall_quality(
                            p,
                            self.staff,
                            self.tactics,
                            self.date,
                            self.is_friendly,
                        ) + self
                            .engine
                            .development_minutes_bonus(p, self.match_importance)
                            + self.engine.fatigue_penalty(p, self.is_friendly)
                            + self.cup_opportunity(p, true)
                            + self.future_pathway_start(p, slot, available)
                    };
                    score(a).partial_cmp(&score(b)).unwrap_or(Ordering::Equal)
                })
                .copied();

            match best {
                Some(player) => {
                    let pos = helpers::best_tactical_position(player, self.tactics);
                    squad.push(MatchPlayer::from_player(team_id, player, pos, false));
                    used_ids.push(player.id);
                }
                None => break,
            }
        }

        // STEP 4: LAST RESORT — use ANY remaining player
        while squad.len() < helpers::DEFAULT_SQUAD_SIZE {
            let best = available
                .iter()
                .filter(|p| !used_ids.contains(&p.id))
                .max_by(|a, b| {
                    let sa = a.player_attributes.current_ability;
                    let sb = b.player_attributes.current_ability;
                    sa.cmp(&sb)
                })
                .copied();

            match best {
                Some(player) => {
                    let pos = helpers::best_tactical_position(player, self.tactics);
                    debug!(
                        "Emergency fill: using {} as outfield player",
                        player.full_name
                    );
                    squad.push(MatchPlayer::from_player(team_id, player, pos, false));
                    used_ids.push(player.id);
                }
                None => break,
            }
        }

        if squad.len() < helpers::DEFAULT_SQUAD_SIZE {
            debug!("Could only select {} of 11 starting players", squad.len());
        }

        // STEP 5: Domestic cup rotation target. The DP optimizer pushes back
        // toward the strongest XI when the cup opportunity bias isn't a wide
        // enough margin to flip an individual slot. In real football, a manager
        // who wants to rotate against weak opposition does so deliberately —
        // they pick a deeper rotation than the per-slot score would justify.
        // Walk the XI once and swap a handful of established starters for
        // same-group fringe replacements, under safety constraints.
        if let Some(cup) = self.cup {
            self.apply_cup_rotation_target(team_id, &mut squad, &mut used_ids, available, cup);
        }

        squad
    }

    /// Post-assignment safe-swap pass for early/quarter cup ties. Counts
    /// non-established starters in the XI built by the DP, and if below the
    /// stage/opponent target, swaps established starters out one-by-one for
    /// available non-established replacements under tight safety constraints.
    /// Force-selected players, the goalkeeper slot, and players whose only
    /// realistic replacement is far below them on quality all stay put.
    fn apply_cup_rotation_target(
        &self,
        team_id: u32,
        squad: &mut Vec<MatchPlayer>,
        used_ids: &mut Vec<u32>,
        available: &[&Player],
        cup: &DomesticCupContext,
    ) {
        let stage = cup.stage();
        let opp = cup.opponent_ratio;

        // Stage / opponent → (target non-established starters, max quality
        // gap a swap may concede). Semi and Final fall through with no pass.
        let (target, max_gap) = match stage {
            CupStage::Early => {
                if opp <= 0.70 {
                    (7usize, 5.0f32)
                } else if opp <= 1.15 {
                    (6, 3.5)
                } else {
                    (4, 2.0)
                }
            }
            CupStage::Quarter => {
                if opp <= 1.15 {
                    (4, 2.0)
                } else {
                    (2, 2.0)
                }
            }
            CupStage::Semi | CupStage::Final => return,
        };

        // Player lookup by id — both for re-scoring starters and for the
        // established / force / position checks the swap loop runs.
        let player_by_id: std::collections::HashMap<u32, &Player> = available
            .iter()
            .map(|p| (p.id, *p))
            .collect();

        let is_non_established = |p: &Player| !CupRotation::is_established(p);
        let count_non_established = |sq: &[MatchPlayer]| -> usize {
            sq.iter()
                .filter(|mp| {
                    mp.tactical_position.current_position != PlayerPositionType::Goalkeeper
                })
                .filter_map(|mp| player_by_id.get(&mp.id))
                .filter(|p| is_non_established(p))
                .count()
        };

        if count_non_established(squad) >= target {
            return;
        }

        // Bench pool changes after each swap (the displaced starter joins the
        // available bench but isn't a swap *candidate* — we don't want to put
        // him back in). Limit to the size of the XI to guarantee termination.
        for _ in 0..helpers::DEFAULT_SQUAD_SIZE {
            if count_non_established(squad) >= target {
                break;
            }

            // Eligible replacements: non-established, not GK, not recovering,
            // condition >=70, currently on the bench (not in the XI).
            let used_set: std::collections::HashSet<u32> = used_ids.iter().copied().collect();
            let bench_pool: Vec<&Player> = available
                .iter()
                .copied()
                .filter(|p| !used_set.contains(&p.id))
                .filter(|p| !p.positions.is_goalkeeper())
                .filter(|p| is_non_established(p))
                .filter(|p| !p.player_attributes.is_in_recovery())
                .filter(|p| p.player_attributes.condition_percentage() >= 70)
                .collect();

            if bench_pool.is_empty() {
                break;
            }

            // Best swap = the one that costs the least quality (smallest gap)
            // while passing every safety/fit constraint. Iterate every
            // (established starter, non-established candidate) pair.
            let mut best: Option<(usize, &Player, f32)> = None;
            for (idx, mp) in squad.iter().enumerate() {
                let slot = mp.tactical_position.current_position;
                if slot == PlayerPositionType::Goalkeeper {
                    continue;
                }
                let Some(starter) = player_by_id.get(&mp.id).copied() else {
                    continue;
                };
                if !CupRotation::is_established(starter) {
                    continue;
                }
                // Honor the manager pin: a force-selected player is never
                // swapped out by this pass.
                if self.engine.honor_force_selection && starter.is_force_match_selection {
                    continue;
                }

                let starter_score = self.starting_slot_score(starter, slot, available);
                let slot_group = slot.position_group();

                for &cand in bench_pool.iter() {
                    let fit = helpers::position_fit_score(cand, slot, slot_group);
                    // "0.70 fit" on a 0..20 level scale: a level-14 specialist
                    // at the slot, or a same-group player whose proximity
                    // multiplier × primary level lands at or above 14.
                    if fit < 14.0 {
                        continue;
                    }
                    let cand_score = self.starting_slot_score(cand, slot, available);
                    let gap = starter_score - cand_score;
                    if gap > max_gap {
                        continue;
                    }
                    // Smallest gap wins (least quality conceded).
                    if best.map(|(_, _, g)| gap < g).unwrap_or(true) {
                        best = Some((idx, cand, gap));
                    }
                }
            }

            let Some((idx, new_player, _)) = best else {
                break;
            };
            let old_id = squad[idx].id;
            let slot = squad[idx].tactical_position.current_position;
            used_ids.retain(|id| *id != old_id);
            used_ids.push(new_player.id);
            squad[idx] = MatchPlayer::from_player(team_id, new_player, slot, false);
        }
    }

    /// Select substitutes for competitive matches.
    pub(crate) fn select_substitutes(
        &self,
        team_id: u32,
        remaining: &[&Player],
    ) -> Vec<MatchPlayer> {
        let mut subs: Vec<MatchPlayer> = Vec::with_capacity(helpers::DEFAULT_BENCH_SIZE);
        let mut used_ids: Vec<u32> = Vec::new();

        // 1. Backup goalkeeper
        if let Some(gk) = self.pick_best_goalkeeper(remaining, &used_ids) {
            subs.push(MatchPlayer::from_player(
                team_id,
                gk,
                PlayerPositionType::Goalkeeper,
                false,
            ));
            used_ids.push(gk.id);
        }

        // 2. Role coverage. Real benches are selected for match options, not just
        // broad DEF/MID/FWD buckets.
        for role in self.bench_plan() {
            if subs.len() >= helpers::DEFAULT_BENCH_SIZE {
                break;
            }

            let best = remaining
                .iter()
                .filter(|p| !used_ids.contains(&p.id))
                .max_by(|a, b| {
                    self.bench_role_score(a, role)
                        .partial_cmp(&self.bench_role_score(b, role))
                        .unwrap_or(Ordering::Equal)
                })
                .copied();

            if let Some(player) = best {
                if self.bench_role_fit(player, role) < 0.25
                    && remaining.len() > helpers::DEFAULT_BENCH_SIZE
                {
                    continue;
                }
                let pos = helpers::best_tactical_position(player, self.tactics);
                subs.push(MatchPlayer::from_player(team_id, player, pos, false));
                used_ids.push(player.id);
            }
        }

        // 3. Fill remaining with best available
        while subs.len() < helpers::DEFAULT_BENCH_SIZE {
            let best = remaining
                .iter()
                .filter(|p| !used_ids.contains(&p.id))
                .max_by(|a, b| {
                    self.bench_role_score(a, BenchRole::Impact)
                        .partial_cmp(&self.bench_role_score(b, BenchRole::Impact))
                        .unwrap_or(Ordering::Equal)
                })
                .copied();

            match best {
                Some(player) => {
                    let pos = helpers::best_tactical_position(player, self.tactics);
                    subs.push(MatchPlayer::from_player(team_id, player, pos, false));
                    used_ids.push(player.id);
                }
                None => break,
            }
        }

        // 4. Early-round cup bench guarantee. The bench should carry at least
        // two non-established outfielders into early ties — players who can
        // realistically come on for cameo minutes. If the bench-role scoring
        // overweighted established Impact subs, swap the lowest-impact
        // established outfielder for the best available non-established one.
        if let Some(cup) = self.cup {
            if cup.stage() == CupStage::Early {
                self.ensure_non_established_bench_outfielders(team_id, &mut subs, &mut used_ids, remaining, 2);
            }
        }

        subs
    }

    /// Push the bench toward `min_count` non-established outfielders. Skips
    /// the goalkeeper slot, force-selected players, and stops once no
    /// improvement is possible. Runs only for early-round cup ties — bench
    /// composition for league/managed-minutes matches stays purely
    /// score-driven.
    fn ensure_non_established_bench_outfielders(
        &self,
        team_id: u32,
        subs: &mut [MatchPlayer],
        used_ids: &mut Vec<u32>,
        remaining: &[&Player],
        min_count: usize,
    ) {
        let player_by_id: std::collections::HashMap<u32, &Player> = remaining
            .iter()
            .map(|p| (p.id, *p))
            .collect();
        let is_non_est_outfield = |p: &Player| -> bool {
            !p.positions.is_goalkeeper() && !CupRotation::is_established(p)
        };

        for _ in 0..subs.len() {
            let current = subs
                .iter()
                .filter(|mp| mp.tactical_position.current_position != PlayerPositionType::Goalkeeper)
                .filter_map(|mp| player_by_id.get(&mp.id))
                .filter(|p| is_non_est_outfield(p))
                .count();
            if current >= min_count {
                return;
            }

            let used_set: std::collections::HashSet<u32> = used_ids.iter().copied().collect();
            let candidate = remaining
                .iter()
                .copied()
                .filter(|p| !used_set.contains(&p.id))
                .filter(|p| is_non_est_outfield(p))
                .filter(|p| !p.player_attributes.is_in_recovery())
                .max_by(|a, b| {
                    self.bench_role_score(a, BenchRole::Impact)
                        .partial_cmp(&self.bench_role_score(b, BenchRole::Impact))
                        .unwrap_or(Ordering::Equal)
                });
            let Some(new_player) = candidate else { return };

            // Drop the bench's lowest-impact established outfielder.
            let mut drop_idx: Option<usize> = None;
            let mut drop_score = f32::INFINITY;
            for (i, mp) in subs.iter().enumerate() {
                if mp.tactical_position.current_position == PlayerPositionType::Goalkeeper {
                    continue;
                }
                let Some(p) = player_by_id.get(&mp.id) else { continue };
                if !CupRotation::is_established(p) {
                    continue;
                }
                if self.engine.honor_force_selection && p.is_force_match_selection {
                    continue;
                }
                let s = self.bench_role_score(p, BenchRole::Impact);
                if s < drop_score {
                    drop_score = s;
                    drop_idx = Some(i);
                }
            }
            let Some(idx) = drop_idx else { return };

            let old_id = subs[idx].id;
            used_ids.retain(|id| *id != old_id);
            used_ids.push(new_player.id);
            let pos = helpers::best_tactical_position(new_player, self.tactics);
            subs[idx] = MatchPlayer::from_player(team_id, new_player, pos, false);
        }
    }

    fn assign_outfield_slots<'p>(
        &self,
        available: &[&'p Player],
        used_ids: &[u32],
        slots: &[PlayerPositionType],
    ) -> Vec<(PlayerPositionType, &'p Player)> {
        if slots.is_empty() {
            return Vec::new();
        }

        let players: Vec<&Player> = available
            .iter()
            .filter(|p| !used_ids.contains(&p.id))
            .filter(|p| !p.positions.is_goalkeeper())
            .copied()
            .collect();

        if players.len() < slots.len() {
            return Vec::new();
        }

        // Precompute every (player, slot) score once. The DP revisits each
        // (player, slot) pair across many bitmask states, and `starting_slot_score`
        // — now including the future-aware pathway pass — isn't free, so caching
        // it keeps the assignment cost flat instead of multiplying by 2^slots.
        let score_matrix: Vec<Vec<f32>> = players
            .iter()
            .map(|p| {
                slots
                    .iter()
                    .map(|&slot| self.starting_slot_score(p, slot, available))
                    .collect()
            })
            .collect();

        let slot_count = slots.len();
        let full_mask = (1usize << slot_count) - 1;
        let neg_inf = f32::NEG_INFINITY;
        let mut dp = vec![vec![neg_inf; full_mask + 1]; players.len() + 1];
        let mut prev = vec![vec![None; full_mask + 1]; players.len() + 1];
        dp[0][0] = 0.0;

        for (i, _player) in players.iter().enumerate() {
            for mask in 0..=full_mask {
                let current = dp[i][mask];
                if !current.is_finite() {
                    continue;
                }

                if current > dp[i + 1][mask] {
                    dp[i + 1][mask] = current;
                    prev[i + 1][mask] = Some((mask, None));
                }

                for (slot_idx, _slot) in slots.iter().enumerate() {
                    let bit = 1usize << slot_idx;
                    if mask & bit != 0 {
                        continue;
                    }
                    let score = score_matrix[i][slot_idx];
                    let new_mask = mask | bit;
                    let candidate = current + score;
                    if candidate > dp[i + 1][new_mask] {
                        dp[i + 1][new_mask] = candidate;
                        prev[i + 1][new_mask] = Some((mask, Some(slot_idx)));
                    }
                }
            }
        }

        if !dp[players.len()][full_mask].is_finite() {
            return Vec::new();
        }

        let mut assigned: Vec<Option<&Player>> = vec![None; slot_count];
        let mut mask = full_mask;
        for i in (1..=players.len()).rev() {
            let Some((previous_mask, selected_slot)) = prev[i][mask] else {
                break;
            };
            if let Some(slot_idx) = selected_slot {
                assigned[slot_idx] = Some(players[i - 1]);
            }
            mask = previous_mask;
        }

        assigned
            .into_iter()
            .enumerate()
            .filter_map(|(idx, player)| player.map(|p| (slots[idx], p)))
            .collect()
    }

    fn starting_slot_score(
        &self,
        player: &Player,
        slot: PlayerPositionType,
        available: &[&Player],
    ) -> f32 {
        let target_group = slot.position_group();
        self.engine.score_player_for_slot(
            player,
            slot,
            target_group,
            self.staff,
            self.tactics,
            self.date,
            self.is_friendly,
            &[],
        ) + self
            .engine
            .development_minutes_bonus(player, self.match_importance)
            + self.engine.fatigue_penalty(player, self.is_friendly)
            - self
                .engine
                .injury_risk_penalty(player, self.match_importance, self.is_friendly)
            + self.policy_starting_adjustment(player)
            + self.cup_opportunity(player, true)
            + self.future_pathway_start(player, slot, available)
    }

    fn policy_starting_adjustment(&self, player: &Player) -> f32 {
        let age = DateUtils::age(player.birth_date, self.date);
        let is_key_player = player
            .contract
            .as_ref()
            .map(|c| {
                matches!(
                    c.squad_status,
                    PlayerSquadStatus::KeyPlayer | PlayerSquadStatus::FirstTeamRegular
                )
            })
            .unwrap_or(false);
        let is_development_age = age <= 21;
        let idle = player.player_attributes.days_since_last_match as f32;
        // Use position-weighted physical_load so a 90-min wingback gets
        // rotated where a 90-min keeper isn't. Falls back to minutes_last_7
        // when the new field hasn't accumulated yet (early sim, fresh save).
        let load = player
            .load
            .physical_load_7
            .max(player.load.minutes_last_7 * 0.95);
        let morale = player.happiness.morale;

        (match self.policy {
            SelectionPolicy::BestEleven => {
                let experience = if age >= 24 { 0.35 } else { -0.15 };
                let morale_bonus = ((morale - 50.0) / 50.0).clamp(-1.0, 1.0) * 0.35;
                experience + morale_bonus
            }
            SelectionPolicy::StrongWithRotation => {
                let rest_need = if load > 360.0 { -0.8 } else { 0.0 };
                let fresh_regular = if is_key_player && idle >= 5.0 {
                    0.35
                } else {
                    0.0
                };
                rest_need + fresh_regular
            }
            SelectionPolicy::ManagedMinutes => {
                let underplayed = (idle / 18.0).min(1.0) * 0.9;
                let fatigue = if load > 300.0 { -1.2 } else { 0.0 };
                underplayed + fatigue
            }
            SelectionPolicy::CupRotation => {
                let underplayed = (idle / 14.0).min(1.0) * 1.6;
                let youth = if is_development_age { 0.9 } else { 0.0 };
                let protect_star = if is_key_player && load > 180.0 {
                    -1.4
                } else {
                    0.0
                };
                underplayed + youth + protect_star
            }
            SelectionPolicy::YouthDevelopment => {
                let youth = if is_development_age {
                    2.0
                } else if age <= 23 {
                    1.0
                } else {
                    0.0
                };
                let key_player_rest = if is_key_player { -1.2 } else { 0.0 };
                youth + key_player_rest + (idle / 21.0).min(1.0)
            }
        }) * (1.0 - self.match_importance * 0.25)
    }

    fn bench_plan(&self) -> Vec<BenchRole> {
        let uses_wide_players = self.tactics.positions().iter().any(|p| {
            matches!(
                p,
                PlayerPositionType::DefenderLeft
                    | PlayerPositionType::DefenderRight
                    | PlayerPositionType::WingbackLeft
                    | PlayerPositionType::WingbackRight
                    | PlayerPositionType::MidfielderLeft
                    | PlayerPositionType::MidfielderRight
                    | PlayerPositionType::AttackingMidfielderLeft
                    | PlayerPositionType::AttackingMidfielderRight
                    | PlayerPositionType::ForwardLeft
                    | PlayerPositionType::ForwardRight
            )
        });

        let mut roles = vec![
            BenchRole::DefensiveCover,
            BenchRole::MidfieldControl,
            BenchRole::Creator,
        ];
        if uses_wide_players {
            roles.push(BenchRole::WideOption);
        } else {
            roles.push(BenchRole::Utility);
        }
        roles.push(BenchRole::Striker);
        roles.push(BenchRole::Impact);
        roles.push(match self.policy {
            SelectionPolicy::CupRotation | SelectionPolicy::YouthDevelopment => BenchRole::Prospect,
            _ => BenchRole::Utility,
        });
        roles
    }

    fn bench_role_score(&self, player: &Player, role: BenchRole) -> f32 {
        self.engine.overall_quality(
            player,
            self.staff,
            self.tactics,
            self.date,
            self.is_friendly,
        ) + self
            .engine
            .development_minutes_bonus(player, self.match_importance)
            + self.engine.fatigue_penalty(player, self.is_friendly) * 0.5
            - self
                .engine
                .injury_risk_penalty(player, self.match_importance, self.is_friendly)
                * 0.35
            + self.bench_role_fit(player, role) * 4.0
            + self.bench_policy_adjustment(player)
            + self.cup_opportunity(player, false)
            + self.cup_bench_unseen_bonus(player)
            + self.future_pathway_bench(player)
    }

    /// Extra bench pull for a non-established player who hasn't featured in
    /// this cup competition yet. Stacks on top of the regular cup opportunity
    /// bonus so a fringe player is more likely to actually make the matchday
    /// 18 (and thus get a cameo) on a rotation night.
    fn cup_bench_unseen_bonus(&self, player: &Player) -> f32 {
        let Some(cup) = self.cup else {
            return 0.0;
        };
        let stage = cup.stage();
        let weight = match stage {
            CupStage::Early => 1.2,
            CupStage::Quarter => 0.6,
            _ => return 0.0,
        };
        if CupRotation::is_established(player) {
            return 0.0;
        }
        let cup_apps =
            player.cup_statistics.played + player.cup_statistics.played_subs;
        if cup_apps == 0 { weight } else { 0.0 }
    }

    fn bench_role_fit(&self, player: &Player, role: BenchRole) -> f32 {
        let positions = player.positions.positions();
        let has = |pos: PlayerPositionType| positions.contains(&pos);
        let has_any = |targets: &[PlayerPositionType]| targets.iter().any(|&p| has(p));

        match role {
            BenchRole::DefensiveCover => {
                let center = has_any(&[
                    PlayerPositionType::DefenderCenter,
                    PlayerPositionType::DefenderCenterLeft,
                    PlayerPositionType::DefenderCenterRight,
                    PlayerPositionType::DefensiveMidfielder,
                ]);
                let wide = has_any(&[
                    PlayerPositionType::DefenderLeft,
                    PlayerPositionType::DefenderRight,
                    PlayerPositionType::WingbackLeft,
                    PlayerPositionType::WingbackRight,
                ]);
                if center && wide {
                    1.0
                } else if center || wide {
                    0.75
                } else {
                    0.0
                }
            }
            BenchRole::MidfieldControl => {
                if has_any(&[
                    PlayerPositionType::DefensiveMidfielder,
                    PlayerPositionType::MidfielderCenter,
                    PlayerPositionType::MidfielderCenterLeft,
                    PlayerPositionType::MidfielderCenterRight,
                ]) {
                    1.0
                } else {
                    0.0
                }
            }
            BenchRole::Creator => {
                let positional = has_any(&[
                    PlayerPositionType::AttackingMidfielderCenter,
                    PlayerPositionType::AttackingMidfielderLeft,
                    PlayerPositionType::AttackingMidfielderRight,
                    PlayerPositionType::MidfielderCenter,
                ]);
                let creative = (player.skills.mental.vision
                    + player.skills.technical.passing
                    + player.skills.technical.technique)
                    / 60.0;
                if positional {
                    creative.max(0.45)
                } else {
                    creative * 0.5
                }
            }
            BenchRole::WideOption => {
                if has_any(&[
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
                ]) {
                    1.0
                } else {
                    0.0
                }
            }
            BenchRole::Striker => {
                if has_any(&[
                    PlayerPositionType::Striker,
                    PlayerPositionType::ForwardCenter,
                    PlayerPositionType::ForwardLeft,
                    PlayerPositionType::ForwardRight,
                ]) {
                    1.0
                } else {
                    0.0
                }
            }
            BenchRole::Utility => {
                let covered = self
                    .tactics
                    .positions()
                    .iter()
                    .filter(|&&pos| {
                        pos != PlayerPositionType::Goalkeeper && player.positions.get_level(pos) > 0
                    })
                    .count();
                (covered as f32 / 3.0).clamp(0.0, 1.0)
            }
            BenchRole::Impact => {
                let attacking = (player.skills.technical.dribbling
                    + player.skills.technical.finishing
                    + player.skills.mental.flair
                    + player.skills.physical.pace)
                    / 80.0;
                attacking.clamp(0.0, 1.0)
            }
            BenchRole::Prospect => {
                let age = DateUtils::age(player.birth_date, Utc::now().date_naive());
                if age <= 19 {
                    1.0
                } else if age <= 22 {
                    0.65
                } else {
                    0.0
                }
            }
        }
    }

    fn bench_policy_adjustment(&self, player: &Player) -> f32 {
        let age = DateUtils::age(player.birth_date, self.date);
        match self.policy {
            SelectionPolicy::BestEleven | SelectionPolicy::StrongWithRotation => 0.0,
            SelectionPolicy::ManagedMinutes => {
                (player.player_attributes.days_since_last_match as f32 / 21.0).min(1.0) * 0.7
            }
            SelectionPolicy::CupRotation => {
                let youth = if age <= 21 { 0.8 } else { 0.0 };
                youth + (player.player_attributes.days_since_last_match as f32 / 14.0).min(1.0)
            }
            SelectionPolicy::YouthDevelopment => {
                if age <= 21 {
                    1.6
                } else if age <= 23 {
                    0.8
                } else {
                    0.0
                }
            }
        }
    }

    fn pick_best_goalkeeper<'p>(
        &self,
        available: &[&'p Player],
        used_ids: &[u32],
    ) -> Option<&'p Player> {
        // In real football the #1 keeper plays everything unless injured,
        // genuinely out of form, or the fixture is low priority (early cup
        // rounds, dead rubbers). Injury/suspension is already filtered before
        // we get here; poor form is baked into `goalkeeper_score` via
        // match_readiness + condition_floor_penalty. The missing rotation
        // trigger was fixture importance — `development_minutes_bonus` only
        // fires when match_importance < 0.5, giving an underplayed backup a
        // boost on cup nights but vanishing for league games, so the #1 GK
        // isn't displaced by a workload signal that doesn't apply to keepers.
        available
            .iter()
            .filter(|p| !used_ids.contains(&p.id))
            .filter(|p| p.positions.is_goalkeeper())
            .max_by(|a, b| {
                let score = |p: &Player| {
                    self.engine
                        .goalkeeper_score(p, self.staff, self.is_friendly)
                        + self
                            .engine
                            .development_minutes_bonus(p, self.match_importance)
                        + self.cup_goalkeeper(p)
                };
                score(a).partial_cmp(&score(b)).unwrap_or(Ordering::Equal)
            })
            .copied()
    }

    /// Skill-blind keeper fallback. `pick_best_goalkeeper` uses a full
    /// scoring pipeline (ability × age × form × staff opinion) which can
    /// theoretically return None if every scored keeper produces NaN or
    /// similar edge values. This variant just picks any player in the
    /// available pool whose registered positions include Goalkeeper —
    /// preferring the one with highest combined handling+reflexes so the
    /// walking-wounded keeper with a real goalkeeping profile is picked
    /// over the fresh outfielder with a zeroed one. Used as the second
    /// line of the keeper fallback chain before the outfielder-as-GK
    /// emergency path. Independent of the scoring context, so it's an
    /// associated function rather than a method.
    fn pick_any_goalkeeper_fallback<'p>(
        available: &[&'p Player],
        used_ids: &[u32],
    ) -> Option<&'p Player> {
        available
            .iter()
            .filter(|p| !used_ids.contains(&p.id))
            .filter(|p| p.positions.is_goalkeeper())
            .max_by(|a, b| {
                let sa = a.skills.goalkeeping.handling + a.skills.goalkeeping.reflexes;
                let sb = b.skills.goalkeeping.handling + b.skills.goalkeeping.reflexes;
                sa.partial_cmp(&sb).unwrap_or(Ordering::Equal)
            })
            .copied()
    }
}

#[derive(Debug, Clone, Copy)]
enum BenchRole {
    DefensiveCover,
    MidfieldControl,
    Creator,
    WideOption,
    Striker,
    Utility,
    Impact,
    Prospect,
}
