use crate::club::staff::{CoachDecisionEngine, CoachSelectionContext};
use crate::club::{PlayerPositionType, Staff};
use crate::r#match::player::MatchPlayer;
use crate::utils::DateUtils;
use crate::{Player, PlayerSquadStatus, Tactics};
use chrono::NaiveDate;
use log::debug;

use super::cup_rotation::CupRotation;
use super::helpers;
use super::scoring::ScoringEngine;
use super::{CupStage, DomesticCupContext, SelectionCompetition, SelectionPolicy};
use std::cmp::Ordering;
use std::collections::{HashMap, HashSet};

/// Minimum role fit (0..1) a substitute must clear to be chosen for a specific
/// bench role when the pool has a surplus of options (see `select_substitutes`).
const BENCH_ROLE_MIN_FIT_SURPLUS: f32 = 0.25;

/// Cohesion local-swap pass tuning (see `apply_cohesion_swaps`). The pass flips
/// a starter for a same-group bench option only when the swap concedes at most
/// `MAX_BASE_GAP` of pure slot score, lifts cohesion by at least `MIN_GAIN`, and
/// nets positive overall — across at most `MAX_PASSES` sweeps.
const COHESION_SWAP_MAX_BASE_GAP: f32 = 0.75;
const COHESION_SWAP_MIN_GAIN: f32 = 0.35;
const COHESION_SWAP_MAX_PASSES: usize = 2;
/// Minimum position fit (0..20 scale) a cohesion-swap candidate must clear.
const COHESION_SWAP_MIN_POSITION_FIT: f32 = 12.0;

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
    /// Coach decision engine for the selecting side, when available.
    /// Absent for the legacy public scoring entry points and tests that
    /// don't build it. The slot / bench scorers fold a small coach
    /// adjustment on top of the existing score when present, and
    /// behave exactly as before when absent.
    pub coach: Option<&'a CoachDecisionEngine<'a>>,
    /// Competition the tie sits in. Used by the coach adjustment to
    /// build the per-player `CoachSelectionContext` (drives the big-
    /// match / derby / continental dimension).
    pub competition: SelectionCompetition,
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

        // STEP 4.5: Cohesion local-swap pass. The DP scores each slot on its
        // own merit and can't see that a marginally lower-rated candidate would
        // knit the unit together better — so flip only genuinely close calls.
        self.apply_cohesion_swaps(team_id, &mut squad, &mut used_ids, available);

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

    /// Post-assignment cohesion swap pass. The DP scores each slot
    /// independently, so a slightly lower-rated candidate that knits the unit
    /// together better (a settled centre-back pairing, a fullback who trusts the
    /// winger ahead of him) never gets the nod. This walks the assigned outfield
    /// XI and flips only genuinely close calls: a same-group bench option with a
    /// real position fit, at most `COHESION_SWAP_MAX_BASE_GAP` below the
    /// incumbent on the pure slot score, whose cohesion with the rest of the
    /// selected XI is at least `COHESION_SWAP_MIN_GAIN` higher and outweighs that
    /// small quality loss. The goalkeeper and force-selected players are never
    /// touched. Without relationship data every cohesion read is zero, so the
    /// pass is a no-op — it only ever refines an already-settled squad.
    fn apply_cohesion_swaps(
        &self,
        team_id: u32,
        squad: &mut [MatchPlayer],
        used_ids: &mut Vec<u32>,
        available: &[&Player],
    ) {
        let player_by_id: HashMap<u32, &Player> = available.iter().map(|p| (p.id, *p)).collect();

        for _ in 0..COHESION_SWAP_MAX_PASSES {
            let mut swapped = false;

            for idx in 0..squad.len() {
                let slot = squad[idx].tactical_position.current_position;
                if slot == PlayerPositionType::Goalkeeper {
                    continue;
                }
                let Some(current) = player_by_id.get(&squad[idx].id).copied() else {
                    continue;
                };
                // Never swap out a manager-pinned starter.
                if self.engine.honor_force_selection && current.is_force_match_selection {
                    continue;
                }

                let group = slot.position_group();

                // The rest of the selected XI — the teammates cohesion is read
                // against. Rebuilt each iteration so it reflects earlier swaps.
                let others: Vec<&Player> = squad
                    .iter()
                    .enumerate()
                    .filter(|(i, _)| *i != idx)
                    .filter_map(|(_, mp)| player_by_id.get(&mp.id).copied())
                    .collect();

                let current_base = self.starting_slot_score(current, slot, available);
                let current_cohesion =
                    self.engine.cohesion_bonus(current, &others, slot, group);

                let used_set: HashSet<u32> = used_ids.iter().copied().collect();

                // Best swap = the largest net gain among eligible candidates.
                let mut best: Option<(&Player, f32)> = None;
                for cand in available.iter().copied() {
                    if used_set.contains(&cand.id) {
                        continue;
                    }
                    if cand.positions.is_goalkeeper() {
                        continue;
                    }
                    if cand.position().position_group() != group {
                        continue;
                    }
                    if helpers::position_fit_score(cand, slot, group)
                        < COHESION_SWAP_MIN_POSITION_FIT
                    {
                        continue;
                    }
                    let cand_base = self.starting_slot_score(cand, slot, available);
                    let base_gap = current_base - cand_base;
                    if base_gap > COHESION_SWAP_MAX_BASE_GAP {
                        continue;
                    }
                    let cand_cohesion =
                        self.engine.cohesion_bonus(cand, &others, slot, group);
                    let cohesion_gain = cand_cohesion - current_cohesion;
                    if cohesion_gain < COHESION_SWAP_MIN_GAIN {
                        continue;
                    }
                    // The net gain after the conceded base score must be positive.
                    let total_gain = cohesion_gain - base_gap;
                    if total_gain <= 0.0 {
                        continue;
                    }
                    if best.map(|(_, g)| total_gain > g).unwrap_or(true) {
                        best = Some((cand, total_gain));
                    }
                }

                if let Some((new_player, _)) = best {
                    let old_id = squad[idx].id;
                    used_ids.retain(|id| *id != old_id);
                    used_ids.push(new_player.id);
                    squad[idx] = MatchPlayer::from_player(team_id, new_player, slot, false);
                    swapped = true;
                }
            }

            if !swapped {
                break;
            }
        }
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
        let player_by_id: HashMap<u32, &Player> = available.iter().map(|p| (p.id, *p)).collect();

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
            let used_set: HashSet<u32> = used_ids.iter().copied().collect();
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

            // With a surplus of bench options, restrict the role's pick to
            // genuine fits up front, so a high-quality but ill-fitting player
            // can't win the role's score and then be rejected — leaving the role
            // uncovered when a worse-rated specialist was available. When the
            // pool is thin, fall back to the best available so the bench fills.
            let surplus = remaining.len() > helpers::DEFAULT_BENCH_SIZE;
            let best = remaining
                .iter()
                .filter(|p| !used_ids.contains(&p.id))
                .filter(|p| !surplus || self.bench_role_fit(p, role) >= BENCH_ROLE_MIN_FIT_SURPLUS)
                .max_by(|a, b| {
                    self.bench_role_score(a, role)
                        .partial_cmp(&self.bench_role_score(b, role))
                        .unwrap_or(Ordering::Equal)
                })
                .copied();

            if let Some(player) = best {
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

        // 5. Backup-goalkeeper guarantee. The keeper step (1) already benches
        // the best available backup first, so in the normal flow this is a
        // no-op. It backstops odd pools — and any future reordering of the
        // steps above — so a matchday bench never goes out keeper-less while a
        // keeper is available anywhere in the pool.
        self.ensure_backup_goalkeeper(team_id, &mut subs, &mut used_ids, remaining);

        subs
    }

    /// Final backup-goalkeeper guarantee for the bench. No-op when the bench
    /// already names a keeper, or when no keeper is available in `remaining`.
    /// Otherwise the best available backup keeper is added: appended when the
    /// bench has a free slot, or — when the bench is full — swapped in for the
    /// lowest-value outfield substitute. A force-selected player is never
    /// dropped, and a key player is only displaced when no cheaper outfield sub
    /// exists. Mirrors real football: a manager names a substitute keeper, even
    /// at the cost of an outfield option, whenever one is available.
    pub(crate) fn ensure_backup_goalkeeper(
        &self,
        team_id: u32,
        subs: &mut Vec<MatchPlayer>,
        used_ids: &mut Vec<u32>,
        remaining: &[&Player],
    ) {
        let has_keeper = subs
            .iter()
            .any(|mp| mp.tactical_position.current_position == PlayerPositionType::Goalkeeper);
        if has_keeper {
            return;
        }

        let Some(gk) = self.pick_best_goalkeeper(remaining, used_ids.as_slice()) else {
            return;
        };

        if subs.len() < helpers::DEFAULT_BENCH_SIZE {
            subs.push(MatchPlayer::from_player(
                team_id,
                gk,
                PlayerPositionType::Goalkeeper,
                false,
            ));
            used_ids.push(gk.id);
            return;
        }

        // Bench full — swap the most expendable outfield substitute for the
        // keeper so a structural keeper slot is never lost to an outfield
        // luxury option.
        let player_by_id: HashMap<u32, &Player> = remaining.iter().map(|p| (p.id, *p)).collect();
        let Some(idx) = self.lowest_value_outfield_sub(subs, &player_by_id) else {
            return;
        };
        let old_id = subs[idx].id;
        used_ids.retain(|id| *id != old_id);
        used_ids.push(gk.id);
        subs[idx] = MatchPlayer::from_player(team_id, gk, PlayerPositionType::Goalkeeper, false);
    }

    /// Index of the outfield substitute most expendable for a structural need
    /// (here: naming a backup keeper on a full bench). Force-selected players
    /// are never eligible. Among the rest the lowest `Impact` bench score wins,
    /// but a key player (KeyPlayer / FirstTeamRegular) is only considered once
    /// no non-key outfield substitute remains — so a fringe option is always
    /// dropped before a senior one. Returns `None` when every bench player is
    /// the keeper slot or force-selected.
    fn lowest_value_outfield_sub(
        &self,
        subs: &[MatchPlayer],
        player_by_id: &HashMap<u32, &Player>,
    ) -> Option<usize> {
        let is_key = |p: &Player| {
            p.contract
                .as_ref()
                .map(|c| {
                    matches!(
                        c.squad_status,
                        PlayerSquadStatus::KeyPlayer | PlayerSquadStatus::FirstTeamRegular
                    )
                })
                .unwrap_or(false)
        };

        let pick = |allow_key: bool| -> Option<usize> {
            let mut best_idx: Option<usize> = None;
            let mut best_score = f32::INFINITY;
            for (i, mp) in subs.iter().enumerate() {
                if mp.tactical_position.current_position == PlayerPositionType::Goalkeeper {
                    continue;
                }
                let Some(p) = player_by_id.get(&mp.id).copied() else {
                    continue;
                };
                if self.engine.honor_force_selection && p.is_force_match_selection {
                    continue;
                }
                if !allow_key && is_key(p) {
                    continue;
                }
                let score = self.bench_role_score(p, BenchRole::Impact);
                if score < best_score {
                    best_score = score;
                    best_idx = Some(i);
                }
            }
            best_idx
        };

        pick(false).or_else(|| pick(true))
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
        let player_by_id: HashMap<u32, &Player> = remaining.iter().map(|p| (p.id, *p)).collect();
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

            let used_set: HashSet<u32> = used_ids.iter().copied().collect();
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
            + self.coach_starting_adjustment(player, slot)
    }

    /// Memory-aware coach lens layered on top of the slot score.
    /// Returns 0.0 when no coach engine is wired in (legacy callers /
    /// tests) so the existing scoring is unchanged. Otherwise returns
    /// a small signed nudge — the engine's adjustment is bounded by
    /// [`AssessmentMath::SELECTION_SCALE`] inside the coach module,
    /// so a personality + memory composite cannot dominate raw quality.
    fn coach_starting_adjustment(&self, player: &Player, slot: PlayerPositionType) -> f32 {
        let Some(coach) = self.coach else {
            return 0.0;
        };
        let target_group = slot.position_group();
        let natural_role_fit =
            (helpers::position_fit_score(player, slot, target_group) / 20.0).clamp(0.0, 1.0);
        let coach_ctx = CoachSelectionContext {
            date: self.date,
            match_importance: self.match_importance,
            is_friendly: self.is_friendly,
            is_cup: matches!(
                self.competition,
                SelectionCompetition::DomesticCup { .. } | SelectionCompetition::ContinentalCup
            ),
            is_derby: false,
            is_continental: matches!(self.competition, SelectionCompetition::ContinentalCup),
            natural_role_fit,
            is_succession_heir: &[],
        };
        coach.score_starting_slot(player, &coach_ctx).adjustment
    }

    /// Memory-aware coach lens on top of the bench-role score.
    fn coach_bench_adjustment(&self, player: &Player) -> f32 {
        let Some(coach) = self.coach else {
            return 0.0;
        };
        let slot = helpers::best_tactical_position(player, self.tactics);
        let natural_role_fit =
            (helpers::position_fit_score(player, slot, slot.position_group()) / 20.0).clamp(0.0, 1.0);
        let coach_ctx = CoachSelectionContext {
            date: self.date,
            match_importance: self.match_importance,
            is_friendly: self.is_friendly,
            is_cup: matches!(
                self.competition,
                SelectionCompetition::DomesticCup { .. } | SelectionCompetition::ContinentalCup
            ),
            is_derby: false,
            is_continental: matches!(self.competition, SelectionCompetition::ContinentalCup),
            natural_role_fit,
            is_succession_heir: &[],
        };
        coach.score_bench_role(player, &coach_ctx).adjustment
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
            + self.coach_bench_adjustment(player)
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

    pub(crate) fn bench_role_fit(&self, player: &Player, role: BenchRole) -> f32 {
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
                let age = DateUtils::age(player.birth_date, self.date);
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
        //
        // The future-aware pathway layer is deliberately NOT mixed in here.
        // Keeper minutes are a single-slot, high-variance call: a green young
        // keeper handed a start on a development nudge concedes goals an
        // outfield prospect's positional cameo never would. Rotating a rested
        // backup into an early/dead cup tie is already handled by
        // `domestic_cup_goalkeeper_adjustment`, which is opponent- and
        // stage-gated; the pathway pull would add no realistic signal a #1
        // keeper's CA edge doesn't already encode, only risk. Youth keeper
        // development is served instead by reliably naming them on the bench
        // (see `ensure_backup_goalkeeper`) so they travel with the squad.
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
pub(crate) enum BenchRole {
    DefensiveCover,
    MidfieldControl,
    Creator,
    WideOption,
    Striker,
    Utility,
    Impact,
    Prospect,
}
