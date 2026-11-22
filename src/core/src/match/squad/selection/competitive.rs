use crate::club::staff::goalkeeping::{KeeperFixture, KeeperSelectionBrief};
use crate::club::staff::perception::AbilityEstimator;
use crate::club::staff::{CoachDecisionEngine, CoachSelectionContext, PlayerMatchIntent};
use crate::club::{PlayerPositionType, Staff};
use crate::r#match::player::MatchPlayer;
use crate::utils::SlotAssignment;
use crate::{Player, Tactics};
use chrono::NaiveDate;
use log::debug;

use super::SelectionCompetition;
use super::balance::{BalanceSwapPass, LineupBalanceScorer, ObjectiveResolver};
use super::bench_scenarios::{BenchScenarioPlan, BenchScenarioScorer};
use super::helpers;
use super::helpers::SlotFit;
use super::lines::PitchLine;
use super::model::{EligibilityEvaluator, MatchSelectionGameModel, MatchTypeSignal};
use super::role_duty::{OpponentMatchupScorer, RoleDutyFitScorer, TacticalDuty};
use super::scoring::{ScoringEngine, SlotScoreBreakdown};
use rustc_hash::{FxHashMap, FxHashSet};
use std::cmp::Ordering;

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
    /// How little room the fixture leaves: `1 - room`.
    pub match_importance: f32,
    /// What the fixture is worth to the side, whatever the week holds.
    pub stakes: f32,
    /// How far football owed may move a shirt in this fixture.
    pub room: f32,
    /// Substitutes the competition lets the side name.
    pub bench_size: usize,
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
    /// Richer per-fixture model — opponent profile, environment,
    /// competition rules, squad state, coach policy. When `None` the
    /// new bounded additive terms (opponent matchup, role/duty,
    /// eligibility rule penalty) collapse to zero so existing legacy
    /// callers behave identically to before this layer existed.
    pub game_model: Option<&'a MatchSelectionGameModel>,
    /// The men the coach's plan is grooming behind an incumbent. Feeds
    /// the coach engine's `SuccessionPlanning` selection read.
    pub succession_heirs: &'a [u32],
    /// The goalkeeping department's word: the declared pecking order and
    /// any live request to play a particular keeper. `None` for a club that
    /// has never reviewed its keeper room, and for every legacy entry point
    /// — keeper selection then behaves exactly as it always did.
    pub keeper_brief: Option<KeeperSelectionBrief>,
}

impl SelectionScoringContext<'_> {
    /// The shortest bench that names a second goalkeeper.
    const SECOND_KEEPER_BENCH: usize = 12;
    /// What a prospect's seat is worth against a senior who would add more
    /// to the match, in a fixture that leaves all of its room.
    const PROSPECT_SEAT: f32 = 3.0;

    /// Whether `player` is a manager-pinned starter this side actually honours.
    /// Force selection is a Main-team pin for *competitive* football, so three
    /// conditions must all hold:
    ///   * the fixture is not a friendly — a pin never drags a player into a
    ///     meaningless friendly, where the squad is rotated on merit/rest;
    ///   * the engine honours the pin (`honor_force_selection` is false for B /
    ///     reserve / youth squads — the pin is a senior-XI override);
    ///   * the player actually carries the pin.
    fn honors_force(&self, player: &Player) -> bool {
        !self.is_friendly && self.engine.honor_force_selection && player.is_force_match_selection
    }

    /// Bounded want-away selection nudge for `player`, 0.0 for a settled
    /// player. Soft transfer-market/psychology signal layered on top of the
    /// quality scoring — never an availability block. Applied at every
    /// selection site (XI, bench, goalkeeper) so a listed player is scored
    /// like any other rather than vanishing from the matchday squad.
    fn want_away_adjustment(&self, player: &Player) -> f32 {
        self.engine
            .want_away_adjustment(player, self.staff, self.date, self.is_friendly)
    }

    /// Select the best starting 11 for competitive matches.
    pub(crate) fn select_starting_eleven(
        &self,
        team_id: u32,
        available: &[&Player],
    ) -> Vec<MatchPlayer> {
        let mut squad: Vec<MatchPlayer> = Vec::with_capacity(helpers::DEFAULT_SQUAD_SIZE);
        let mut used_ids: Vec<u32> = Vec::new();

        self.select_goalkeeper(team_id, available, &mut squad, &mut used_ids);
        self.fill_outfield_slots(team_id, available, &mut squad, &mut used_ids);
        self.fill_remaining_slots(team_id, available, &mut squad, &mut used_ids);
        self.fill_last_resort(team_id, available, &mut squad, &mut used_ids);

        if squad.len() < helpers::DEFAULT_SQUAD_SIZE {
            debug!("Could only select {} of 11 starting players", squad.len());
        }

        // STEP 4.5: Cohesion local-swap pass. The DP scores each slot on its
        // own merit and can't see that a marginally lower-rated candidate would
        // knit the unit together better — so flip only genuinely close calls.
        self.apply_cohesion_swaps(team_id, &mut squad, &mut used_ids, available);

        // STEP 4.6: XI-level balance pass. The DP / cohesion passes optimise
        // per-slot quality and unit chemistry; this pass evaluates the whole
        // XI against the spec's balance bands (defensive security, ball
        // progression, pressing, …) and applies same-group swaps when they
        // improve the balance more than they cost in raw slot score.
        self.apply_balance_swaps(team_id, &mut squad, &mut used_ids, available);

        squad
    }

    fn select_goalkeeper(
        &self,
        team_id: u32,
        available: &[&Player],
        squad: &mut Vec<MatchPlayer>,
        used_ids: &mut Vec<u32>,
    ) {
        // STEP 1: Goalkeeper. Fallback order:
        //   0. Manager-pinned keeper. A force-selected, available keeper is a
        //      hard pin — the manager wants him between the sticks, so he takes
        //      the gloves over the merit pick. The force bonus only ever
        //      reached the outfield slot score, never `goalkeeper_score`, so a
        //      pinned backup keeper used to lose the gloves to the #1 and ride
        //      the bench; this restores the pin for keepers too.
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
            .pick_forced_goalkeeper(available, used_ids)
            .or_else(|| self.pick_best_goalkeeper(available, used_ids, true))
            .or_else(|| Self::pick_any_goalkeeper_fallback(available, used_ids));
        if let Some(gk) = picked_gk {
            squad.push(MatchPlayer::from_player(
                team_id,
                gk,
                PlayerPositionType::Goalkeeper,
                false,
                Some(self.date),
            ));
            used_ids.push(gk.id);
        } else {
            debug!("No goalkeeper found at all — picking any player as GK");
            if let Some(any) = SlotFit::pick_best_unused(available, used_ids) {
                squad.push(MatchPlayer::from_player(
                    team_id,
                    any,
                    PlayerPositionType::Goalkeeper,
                    false,
                    Some(self.date),
                ));
                used_ids.push(any.id);
            }
        }
    }

    fn fill_outfield_slots(
        &self,
        team_id: u32,
        available: &[&Player],
        squad: &mut Vec<MatchPlayer>,
        used_ids: &mut Vec<u32>,
    ) {
        // STEP 2: Fill outfield positions as one assignment problem. This avoids
        // burning a versatile player in an early slot when a specialist is needed
        // later in the shape.
        let outfield_slots: Vec<PlayerPositionType> = self
            .tactics
            .positions()
            .iter()
            .copied()
            .filter(|p| *p != PlayerPositionType::Goalkeeper)
            .collect();
        let assignments = self.assign_outfield_slots(available, used_ids, &outfield_slots);

        for (pos, player) in assignments {
            squad.push(MatchPlayer::from_player(
                team_id,
                player,
                pos,
                false,
                Some(self.date),
            ));
            used_ids.push(player.id);
        }
    }

    fn fill_remaining_slots(
        &self,
        team_id: u32,
        available: &[&Player],
        squad: &mut Vec<MatchPlayer>,
        used_ids: &mut Vec<u32>,
    ) {
        // STEP 3: Fill remaining slots with best available
        while squad.len() < helpers::DEFAULT_SQUAD_SIZE {
            let best = available
                .iter()
                .filter(|p| !used_ids.contains(&p.id))
                .filter(|p| !p.positions.is_goalkeeper())
                .max_by(|a, b| {
                    let score = |p: &Player| {
                        self.engine.overall_quality(
                            p,
                            self.staff,
                            self.tactics,
                            self.date,
                            self.is_friendly,
                        ) + self.engine.fatigue_penalty(p, self.is_friendly)
                            * ScoringEngine::caution(self.match_importance)
                            + self.want_away_adjustment(p)
                            + self.player_intent(p).start_adjustment()
                    };
                    score(a).partial_cmp(&score(b)).unwrap_or(Ordering::Equal)
                })
                .copied();

            match best {
                Some(player) => {
                    let pos = SlotFit::best_tactical_position(player, self.tactics);
                    squad.push(MatchPlayer::from_player(
                        team_id,
                        player,
                        pos,
                        false,
                        Some(self.date),
                    ));
                    used_ids.push(player.id);
                }
                None => break,
            }
        }
    }

    fn fill_last_resort(
        &self,
        team_id: u32,
        available: &[&Player],
        squad: &mut Vec<MatchPlayer>,
        used_ids: &mut Vec<u32>,
    ) {
        // STEP 4: LAST RESORT — use ANY remaining player
        while squad.len() < helpers::DEFAULT_SQUAD_SIZE {
            let best = available
                .iter()
                .filter(|p| !used_ids.contains(&p.id))
                .max_by_key(|p| AbilityEstimator::observable_level(p))
                .copied();

            match best {
                Some(player) => {
                    let pos = SlotFit::best_tactical_position(player, self.tactics);
                    debug!(
                        "Emergency fill: using {} as outfield player",
                        player.full_name
                    );
                    squad.push(MatchPlayer::from_player(
                        team_id,
                        player,
                        pos,
                        false,
                        Some(self.date),
                    ));
                    used_ids.push(player.id);
                }
                None => break,
            }
        }
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
        let player_by_id: FxHashMap<u32, &Player> = available.iter().map(|p| (p.id, *p)).collect();

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
                let current_cohesion = self.engine.cohesion_bonus(current, &others, slot, group);

                let used_set: FxHashSet<u32> = used_ids.iter().copied().collect();

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
                    if SlotFit::position_fit_score(cand, slot) < COHESION_SWAP_MIN_POSITION_FIT {
                        continue;
                    }
                    let cand_base = self.starting_slot_score(cand, slot, available);
                    let base_gap = current_base - cand_base;
                    if base_gap > COHESION_SWAP_MAX_BASE_GAP {
                        continue;
                    }
                    let cand_cohesion = self.engine.cohesion_bonus(cand, &others, slot, group);
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
                    squad[idx] =
                        MatchPlayer::from_player(team_id, new_player, slot, false, Some(self.date));
                    swapped = true;
                }
            }

            if !swapped {
                break;
            }
        }
    }

    /// Post-assignment XI-balance swap pass. Wraps [`BalanceSwapPass`] and
    /// [`LineupBalanceScorer`]. For each starter, looks at same-group bench
    /// candidates and applies the single swap (per pass over the XI) that
    /// lifts the balance score by more than the slot-score it concedes,
    /// within the spec's caps. Goalkeeper and force-selected players are
    /// never touched. No-op when no game model is wired in (the balance
    /// scorer's reading wouldn't be objective-weighted).
    fn apply_balance_swaps(
        &self,
        team_id: u32,
        squad: &mut [MatchPlayer],
        used_ids: &mut Vec<u32>,
        available: &[&Player],
    ) {
        if self.game_model.is_none() {
            return;
        }
        let player_by_id: FxHashMap<u32, &Player> = available.iter().map(|p| (p.id, *p)).collect();
        let pass = BalanceSwapPass::standard();
        let allowed_loss = pass.allowed_slot_loss(self.room);
        let objective = ObjectiveResolver::resolve(self.game_model);
        let baseline = LineupBalanceScorer::score(squad, &player_by_id, objective);

        for idx in 0..squad.len() {
            let slot = squad[idx].tactical_position.current_position;
            if slot == PlayerPositionType::Goalkeeper {
                continue;
            }
            let Some(current) = player_by_id.get(&squad[idx].id).copied() else {
                continue;
            };
            if self.engine.honor_force_selection && current.is_force_match_selection {
                continue;
            }

            let group = slot.position_group();
            let current_base = self.starting_slot_score(current, slot, available);
            let used_set: FxHashSet<u32> = used_ids.iter().copied().collect();

            let mut best: Option<(&Player, f32, f32)> = None;
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
                let cand_base = self.starting_slot_score(cand, slot, available);
                let base_loss = current_base - cand_base;
                if base_loss > allowed_loss {
                    continue;
                }
                // Probe the swap balance.
                let saved = squad[idx].id;
                squad[idx] = MatchPlayer::from_player(team_id, cand, slot, false, Some(self.date));
                let after = LineupBalanceScorer::score(squad, &player_by_id, objective);
                let balance_gain = after - baseline;
                // Restore so we make the swap only once we've picked the best.
                let original = player_by_id.get(&saved).copied();
                if let Some(orig) = original {
                    squad[idx] =
                        MatchPlayer::from_player(team_id, orig, slot, false, Some(self.date));
                }
                if balance_gain < pass.min_balance_gain {
                    continue;
                }
                let net = balance_gain - base_loss.max(0.0);
                if net <= 0.0 {
                    continue;
                }
                if best.map(|(_, n, _)| net > n).unwrap_or(true) {
                    best = Some((cand, net, balance_gain));
                }
            }

            if let Some((new_player, _, _)) = best {
                let old_id = squad[idx].id;
                used_ids.retain(|id| *id != old_id);
                used_ids.push(new_player.id);
                squad[idx] =
                    MatchPlayer::from_player(team_id, new_player, slot, false, Some(self.date));
            }
        }
    }

    /// The bench, in the order a manager fills it: one goalkeeper, a man
    /// for each line of the team on the pitch, a prospect's seat when the
    /// fixture leaves room for one, and the rest on what they would add to
    /// the match and the football the plan owes them.
    pub(crate) fn select_substitutes(
        &self,
        team_id: u32,
        remaining: &[&Player],
    ) -> Vec<MatchPlayer> {
        let size = self.bench_size;
        let mut subs: Vec<MatchPlayer> = Vec::with_capacity(size);
        let mut used_ids: Vec<u32> = Vec::new();
        let seat = |subs: &mut Vec<MatchPlayer>, used: &mut Vec<u32>, player: &Player| {
            let pos = if player.positions.is_goalkeeper() {
                PlayerPositionType::Goalkeeper
            } else {
                SlotFit::best_tactical_position(player, self.tactics)
            };
            subs.push(MatchPlayer::from_player(
                team_id,
                player,
                pos,
                false,
                Some(self.date),
            ));
            used.push(player.id);
        };
        // A second goalkeeper is named only where the bench is long
        // enough to afford one.
        let outfield_seat =
            |p: &Player| size >= Self::SECOND_KEEPER_BENCH || !p.positions.is_goalkeeper();

        // 1. The goalkeeper.
        if size > 0
            && let Some(gk) = self.pick_best_goalkeeper(remaining, &used_ids, false)
        {
            seat(&mut subs, &mut used_ids, gk);
        }

        // 1b. Manager-pinned overflow. A force-selected player the XI had no
        // room for (more pins than slots) still belongs in the squad, ahead
        // of ordinary squad members. Runs after the keeper so his seat is
        // never lost to the pins. No-op outside the overflow case, in
        // friendlies, and for non-Main squads.
        self.place_forced_substitutes(team_id, &mut subs, &mut used_ids, remaining);

        // 2. Cover for each line of the team on the pitch. A versatile man
        // already seated covers every line he plays.
        let by_id: FxHashMap<u32, &Player> = remaining.iter().map(|p| (p.id, *p)).collect();
        for line in PitchLine::on_the_pitch(self.tactics) {
            if subs.len() >= size {
                break;
            }
            let covered = used_ids
                .iter()
                .filter_map(|id| by_id.get(id))
                .any(|p| !p.positions.is_goalkeeper() && line.is_covered_by(p));
            if covered {
                continue;
            }
            let cover = remaining
                .iter()
                .copied()
                .filter(|p| !used_ids.contains(&p.id) && !p.positions.is_goalkeeper())
                .filter(|p| line.is_covered_by(p))
                .max_by(|a, b| self.bench_value(a).total_cmp(&self.bench_value(b)));
            if let Some(player) = cover {
                seat(&mut subs, &mut used_ids, player);
            }
        }

        // 3. A prospect's seat. A man on a development role in the plan is
        // named ahead of a senior who adds more to this match by as much
        // as the fixture leaves room for it: nothing in a must-win.
        if subs.len() < size {
            let best = |prospects: bool| {
                remaining
                    .iter()
                    .copied()
                    .filter(|p| !used_ids.contains(&p.id) && !p.positions.is_goalkeeper())
                    .filter(|p| self.player_intent(p).development == prospects)
                    .map(|p| (p, self.bench_value(p)))
                    .max_by(|a, b| a.1.total_cmp(&b.1))
            };
            if let Some((prospect, score)) = best(true)
                && best(false)
                    .is_none_or(|(_, senior)| score + self.room * Self::PROSPECT_SEAT >= senior)
            {
                seat(&mut subs, &mut used_ids, prospect);
            }
        }

        // 4. Options for the match: a place for each kind of change the
        // manager may want to make.
        for role in self.bench_plan() {
            if subs.len() >= size {
                break;
            }

            // With a surplus of bench options, restrict the role's pick to
            // genuine fits up front, so a high-quality but ill-fitting player
            // can't win the role's score and then be rejected — leaving the role
            // uncovered when a worse-rated specialist was available. When the
            // pool is thin, fall back to the best available so the bench fills.
            let surplus = remaining.len() > size;
            let best = remaining
                .iter()
                .copied()
                .filter(|p| !used_ids.contains(&p.id) && outfield_seat(p))
                .filter(|p| !surplus || self.bench_role_fit(p, role) >= BENCH_ROLE_MIN_FIT_SURPLUS)
                .max_by(|a, b| {
                    self.bench_role_score(a, role)
                        .total_cmp(&self.bench_role_score(b, role))
                });
            if let Some(player) = best {
                seat(&mut subs, &mut used_ids, player);
            }
        }

        // 5. The rest on fit and football owed.
        while subs.len() < size {
            let best = remaining
                .iter()
                .copied()
                .filter(|p| !used_ids.contains(&p.id) && outfield_seat(p))
                .max_by(|a, b| {
                    self.bench_role_score(a, BenchRole::Impact)
                        .total_cmp(&self.bench_role_score(b, BenchRole::Impact))
                });
            match best {
                Some(player) => seat(&mut subs, &mut used_ids, player),
                None => break,
            }
        }

        // 6. Backup-goalkeeper guarantee. The keeper step (1) already benches
        // the next keeper first, so in the normal flow this is a no-op. It
        // backstops odd pools so a matchday bench never goes out keeper-less
        // while a keeper is available anywhere in the pool.
        self.ensure_backup_goalkeeper(team_id, &mut subs, &mut used_ids, remaining);

        subs
    }

    /// Bench the manager-pinned players who couldn't be fitted into the XI,
    /// ahead of ordinary squad members. Only fires in the overflow case (more
    /// pins than starting slots) — the surplus pins still travel in the
    /// matchday eighteen. Best pins first, with a stable id tie-break for a
    /// deterministic bench; capped at the bench size and never duplicating an id
    /// already used. Skipped in friendlies and for non-Main squads via
    /// [`Self::honors_force`].
    fn place_forced_substitutes(
        &self,
        team_id: u32,
        subs: &mut Vec<MatchPlayer>,
        used_ids: &mut Vec<u32>,
        remaining: &[&Player],
    ) {
        let mut forced: Vec<&Player> = remaining
            .iter()
            .copied()
            .filter(|p| self.honors_force(p))
            .filter(|p| !used_ids.contains(&p.id))
            .collect();
        if forced.is_empty() {
            return;
        }
        forced.sort_by(|a, b| {
            self.bench_role_score(b, BenchRole::Impact)
                .partial_cmp(&self.bench_role_score(a, BenchRole::Impact))
                .unwrap_or(Ordering::Equal)
                .then_with(|| a.id.cmp(&b.id))
        });
        for player in forced {
            if subs.len() >= self.bench_size {
                break;
            }
            let pos = SlotFit::best_tactical_position(player, self.tactics);
            subs.push(MatchPlayer::from_player(
                team_id,
                player,
                pos,
                false,
                Some(self.date),
            ));
            used_ids.push(player.id);
        }
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

        let Some(gk) = self.pick_best_goalkeeper(remaining, used_ids.as_slice(), false) else {
            return;
        };

        if subs.len() < self.bench_size {
            subs.push(MatchPlayer::from_player(
                team_id,
                gk,
                PlayerPositionType::Goalkeeper,
                false,
                Some(self.date),
            ));
            used_ids.push(gk.id);
            return;
        }

        // Bench full — swap the most expendable outfield substitute for the
        // keeper so a structural keeper slot is never lost to an outfield
        // luxury option.
        let player_by_id: FxHashMap<u32, &Player> = remaining.iter().map(|p| (p.id, *p)).collect();
        let Some(idx) = self.lowest_value_outfield_sub(subs, &player_by_id) else {
            return;
        };
        let old_id = subs[idx].id;
        used_ids.retain(|id| *id != old_id);
        used_ids.push(gk.id);
        subs[idx] = MatchPlayer::from_player(
            team_id,
            gk,
            PlayerPositionType::Goalkeeper,
            false,
            Some(self.date),
        );
    }

    /// Index of the outfield substitute most expendable for a structural need
    /// (here: naming a backup keeper on a full bench). Force-selected players
    /// are never eligible. Among the rest the lowest `Impact` bench score wins,
    /// but the plan's first choice for a shirt is only considered once nobody
    /// else remains — so a fringe option is always dropped before a rested
    /// regular. Returns `None` when every bench player is the keeper slot or
    /// force-selected.
    fn lowest_value_outfield_sub(
        &self,
        subs: &[MatchPlayer],
        player_by_id: &FxHashMap<u32, &Player>,
    ) -> Option<usize> {
        let is_key = |p: &Player| {
            self.staff
                .squad_plan
                .entry(p.id)
                .is_some_and(|entry| entry.place == 0 && entry.shirt.is_some())
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

    fn assign_outfield_slots<'p>(
        &self,
        available: &[&'p Player],
        used_ids: &[u32],
        slots: &[PlayerPositionType],
    ) -> Vec<(PlayerPositionType, &'p Player)> {
        if slots.is_empty() {
            return Vec::new();
        }

        let candidates: Vec<&'p Player> = available
            .iter()
            .filter(|p| !used_ids.contains(&p.id))
            .filter(|p| !p.positions.is_goalkeeper())
            .copied()
            .collect();

        // Manager-pinned outfielders must be in the XI whenever it is
        // structurally possible — a hard pre-placement rule, not merely the
        // +1000 slot-score nudge. Two regimes:
        //   * pins <= open slots: pin every forced player into the assignment
        //     (the DP may not skip them) and fill the rest of the shape around
        //     them, choosing the slots that maximise the *total* slot score.
        //   * pins > open slots (overflow): only the pinned players contest the
        //     slots, so the DP fields the best feasible XI of them and the
        //     surplus pins fall through to the bench pass.
        // With no forced players this collapses to the original assignment.
        let forced_count = candidates
            .iter()
            .copied()
            .filter(|p| self.honors_force(p))
            .count();
        let overflow = forced_count > slots.len();

        let pool: Vec<&'p Player> = if overflow {
            candidates
                .iter()
                .copied()
                .filter(|p| self.honors_force(p))
                .collect()
        } else {
            candidates
        };

        if pool.len() < slots.len() {
            return Vec::new();
        }

        // A forced player is pinned (the DP can't leave him out) only in the
        // non-overflow regime; in overflow every pooled player is forced, so
        // the assignment is a free contest among them for the slots.
        let must_place: Vec<bool> = if overflow {
            vec![false; pool.len()]
        } else {
            pool.iter().map(|p| self.honors_force(p)).collect()
        };

        self.solve_slot_assignment(&pool, available, slots, &must_place)
    }

    /// Maximum-total slot assignment by bitmask DP. Each pooled player is
    /// assigned to at most one slot and every slot must be filled; the returned
    /// assignment maximises the summed slot score. `must_place[i] == true`
    /// drops player `i`'s "skip" transition, so a manager-pinned starter is
    /// guaranteed a slot whenever the assignment is feasible. With every
    /// `must_place` false this is exactly the original free assignment.
    ///
    /// `available` is the full availability pool the slot score reads for its
    /// same-role / successor checks; `pool` is the subset actually contesting
    /// the slots.
    fn solve_slot_assignment<'p>(
        &self,
        pool: &[&'p Player],
        available: &[&'p Player],
        slots: &[PlayerPositionType],
        must_place: &[bool],
    ) -> Vec<(PlayerPositionType, &'p Player)> {
        if pool.len() < slots.len() {
            return Vec::new();
        }

        // Precompute every (player, slot) score once. The DP revisits each
        // (player, slot) pair across many bitmask states, and `starting_slot_score`
        // — including the future-aware pathway pass — isn't free, so caching it
        // keeps the assignment cost flat instead of multiplying by 2^slots.
        let slot_count = slots.len();
        let score_matrix: Vec<f32> = pool
            .iter()
            .flat_map(|p| {
                slots
                    .iter()
                    .map(|&slot| self.starting_slot_score(p, slot, available))
            })
            .collect();

        SlotAssignment::solve(&score_matrix, slot_count, must_place)
            .into_iter()
            .enumerate()
            .filter_map(|(idx, player)| player.map(|i| (slots[idx], pool[i])))
            .collect()
    }

    fn starting_slot_score(
        &self,
        player: &Player,
        slot: PlayerPositionType,
        available: &[&Player],
    ) -> f32 {
        self.starting_slot_breakdown(player, slot, available)
            .total()
    }

    pub(crate) fn starting_slot_breakdown(
        &self,
        player: &Player,
        slot: PlayerPositionType,
        available: &[&Player],
    ) -> SlotScoreBreakdown {
        let target_group = slot.position_group();
        let (_, mut b) = self.engine.score_player_for_slot_with_breakdown(
            player,
            slot,
            target_group,
            self.staff,
            self.tactics,
            self.date,
            self.is_friendly,
            &[],
        );
        b.condition_floor += self.engine.fatigue_penalty(player, self.is_friendly)
            * ScoringEngine::caution(self.match_importance);
        b.injury_risk =
            -self
                .engine
                .injury_risk_penalty(player, self.match_importance, self.is_friendly);
        b.coach_relationship +=
            self.want_away_adjustment(player) + self.coach_starting_adjustment(player, slot);
        b.opponent_matchup = self.opponent_matchup_adjustment(player, slot);
        b.role_duty_fit = self.role_duty_adjustment(player, slot);
        b.eligibility_rule = self.eligibility_rule_penalty(player);
        b.medical_risk = self.medical_caution_adjustment(player);
        b.football_owed = self.player_intent(player).start_adjustment();
        let _ = available;
        b
    }

    pub(crate) fn player_intent(&self, player: &Player) -> PlayerMatchIntent {
        PlayerMatchIntent::assess(player, self.staff, self.room)
    }

    /// Opponent-matchup nudge, gated by the presence of a richer game
    /// model. Returns 0.0 when no game model is wired in.
    fn opponent_matchup_adjustment(&self, player: &Player, slot: PlayerPositionType) -> f32 {
        let Some(model) = self.game_model else {
            return 0.0;
        };
        OpponentMatchupScorer::score(player, slot, &model.opponent_profile)
    }

    /// Role/duty fit, mapped onto a bounded `[-1.8, 1.8]` signed term —
    /// rewards a player whose attribute profile matches the slot's role
    /// recipe, penalises an obvious miscast even when the position
    /// level alone would clear the slot. Returns 0.0 with no game model.
    fn role_duty_adjustment(&self, player: &Player, slot: PlayerPositionType) -> f32 {
        if self.game_model.is_none() {
            return 0.0;
        }
        let duty = self.duty_for_slot(slot);
        let fit = RoleDutyFitScorer::score(player, slot, duty);
        ((fit - 0.55) * 3.6).clamp(-1.8, 1.8)
    }

    /// The soft eligibility limit: a man returning from injury is still
    /// selectable, at a cost.
    fn eligibility_rule_penalty(&self, player: &Player) -> f32 {
        if self.game_model.is_none() {
            return 0.0;
        }
        EligibilityEvaluator::returning(player)
            .map(|penalty| -penalty)
            .unwrap_or(0.0)
    }

    /// Medical-caution scaler — bumps the injury-risk penalty when the
    /// coach policy reads `medical_caution` high (cautious manager
    /// pulls a fragile star sooner). Returns 0.0 with no game model.
    fn medical_caution_adjustment(&self, player: &Player) -> f32 {
        let Some(model) = self.game_model else {
            return 0.0;
        };
        let base = self
            .engine
            .injury_risk_penalty(player, self.match_importance, self.is_friendly);
        // Extra premium scaled by policy.medical_caution. Capped low so it
        // can shape close calls without doubling the base penalty.
        -base * (model.coach_tendencies.medical_caution - 0.5).max(0.0) * 0.6
    }

    /// Resolve the duty for a slot from the formation context. The
    /// current `Tactics` model doesn't carry per-slot duties yet, so
    /// the helper infers a sensible default from the slot position.
    /// When a richer formation-with-duties model arrives, swap this
    /// out for a lookup; the rest of the slot scoring stays unchanged.
    fn duty_for_slot(&self, slot: PlayerPositionType) -> TacticalDuty {
        use PlayerPositionType::*;
        match slot {
            Goalkeeper | Sweeper | DefenderCenter | DefenderCenterLeft | DefenderCenterRight
            | DefenderLeft | DefenderRight | DefensiveMidfielder => TacticalDuty::Defend,
            WingbackLeft
            | WingbackRight
            | MidfielderCenter
            | MidfielderCenterLeft
            | MidfielderCenterRight => TacticalDuty::Support,
            _ => TacticalDuty::Attack,
        }
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
        let natural_role_fit = (SlotFit::position_fit_score(player, slot) / 20.0).clamp(0.0, 1.0);
        let coach_ctx = CoachSelectionContext {
            date: self.date,
            match_importance: self.match_importance,
            is_friendly: self.is_friendly,
            is_cup: matches!(
                self.competition,
                SelectionCompetition::DomesticCup { .. } | SelectionCompetition::ContinentalCup
            ),
            is_derby: self.game_model.map(|m| m.is_derby()).unwrap_or(false),
            is_continental: matches!(self.competition, SelectionCompetition::ContinentalCup),
            natural_role_fit,
            is_succession_heir: self.succession_heirs,
        };
        coach.score_starting_slot(player, &coach_ctx).adjustment
    }

    /// Memory-aware coach lens on top of the bench-role score.
    fn coach_bench_adjustment(&self, player: &Player) -> f32 {
        let Some(coach) = self.coach else {
            return 0.0;
        };
        let slot = SlotFit::best_tactical_position(player, self.tactics);
        let natural_role_fit = (SlotFit::position_fit_score(player, slot) / 20.0).clamp(0.0, 1.0);
        let coach_ctx = CoachSelectionContext {
            date: self.date,
            match_importance: self.match_importance,
            is_friendly: self.is_friendly,
            is_cup: matches!(
                self.competition,
                SelectionCompetition::DomesticCup { .. } | SelectionCompetition::ContinentalCup
            ),
            is_derby: self.game_model.map(|m| m.is_derby()).unwrap_or(false),
            is_continental: matches!(self.competition, SelectionCompetition::ContinentalCup),
            natural_role_fit,
            is_succession_heir: self.succession_heirs,
        };
        coach.score_bench_role(player, &coach_ctx).adjustment
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
        roles
    }

    fn bench_role_score(&self, player: &Player, role: BenchRole) -> f32 {
        self.bench_value(player) + self.bench_role_fit(player, role) * 4.0
    }

    /// What a man is worth on the bench whatever he would be sent on to
    /// do: his fit, his readiness, the coach's reading of him and the
    /// football the plan owes him.
    fn bench_value(&self, player: &Player) -> f32 {
        self.engine.overall_quality(
            player,
            self.staff,
            self.tactics,
            self.date,
            self.is_friendly,
        ) + self.engine.fatigue_penalty(player, self.is_friendly) * 0.5
            - self
                .engine
                .injury_risk_penalty(player, self.match_importance, self.is_friendly)
                * 0.35
            + self.want_away_adjustment(player)
            + self.coach_bench_adjustment(player)
            + self.bench_scenario_coverage_score(player)
            + self.eligibility_rule_penalty(player)
            + self.player_intent(player).bench_adjustment()
    }

    /// Per-fixture bench scenario coverage bonus. Returns 0 when no
    /// game model is wired in (legacy callers / tests). Otherwise
    /// rewards bench candidates whose profile fits the planned match
    /// scenarios (chase goal, protect lead, fresh legs press, …) by
    /// rolling [`BenchScenarioPlan`] over [`BenchScenarioScorer`].
    fn bench_scenario_coverage_score(&self, player: &Player) -> f32 {
        let Some(model) = self.game_model else {
            return 0.0;
        };
        let plan = BenchScenarioPlan::build(model.match_type, model.tactical_objective);
        let coverage =
            plan.cover_score(|scenario| BenchScenarioScorer::coverage(player, scenario, self.date));
        // Headline scenario coverage carries a slightly bigger weight
        // than the fixed role fit since it's the scenario-aware
        // bench's primary signal.
        let mut bonus = coverage * 3.0;
        // Penalty-taker premium when shootouts matter.
        if matches!(
            model.match_type,
            MatchTypeSignal::CupFinal | MatchTypeSignal::ContinentalKnockout
        ) {
            bonus += BenchScenarioScorer::penalty_value(player) * 1.0;
        }
        bonus
    }

    pub(crate) fn bench_role_fit(&self, player: &Player, role: BenchRole) -> f32 {
        match role {
            BenchRole::DefensiveCover => Self::defensive_cover(player),
            BenchRole::MidfieldControl => Self::midfield_control(player),
            BenchRole::Creator => Self::creator(player),
            BenchRole::WideOption => Self::wide_option(player),
            BenchRole::Striker => Self::striker(player),
            BenchRole::Utility => self.utility(player),
            BenchRole::Impact => Self::impact(player),
        }
    }

    fn has_any(player: &Player, targets: &[PlayerPositionType]) -> bool {
        targets
            .iter()
            .any(|&pos| player.positions.has_position(pos))
    }

    fn defensive_cover(player: &Player) -> f32 {
        let center = Self::has_any(
            player,
            &[
                PlayerPositionType::DefenderCenter,
                PlayerPositionType::DefenderCenterLeft,
                PlayerPositionType::DefenderCenterRight,
                PlayerPositionType::DefensiveMidfielder,
            ],
        );
        let wide = Self::has_any(
            player,
            &[
                PlayerPositionType::DefenderLeft,
                PlayerPositionType::DefenderRight,
                PlayerPositionType::WingbackLeft,
                PlayerPositionType::WingbackRight,
            ],
        );
        if center && wide {
            1.0
        } else if center || wide {
            0.75
        } else {
            0.0
        }
    }

    fn midfield_control(player: &Player) -> f32 {
        if Self::has_any(
            player,
            &[
                PlayerPositionType::DefensiveMidfielder,
                PlayerPositionType::MidfielderCenter,
                PlayerPositionType::MidfielderCenterLeft,
                PlayerPositionType::MidfielderCenterRight,
            ],
        ) {
            1.0
        } else {
            0.0
        }
    }

    fn creator(player: &Player) -> f32 {
        let positional = Self::has_any(
            player,
            &[
                PlayerPositionType::AttackingMidfielderCenter,
                PlayerPositionType::AttackingMidfielderLeft,
                PlayerPositionType::AttackingMidfielderRight,
                PlayerPositionType::MidfielderCenter,
            ],
        );
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

    fn wide_option(player: &Player) -> f32 {
        if Self::has_any(
            player,
            &[
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
            ],
        ) {
            1.0
        } else {
            0.0
        }
    }

    fn striker(player: &Player) -> f32 {
        if Self::has_any(
            player,
            &[
                PlayerPositionType::Striker,
                PlayerPositionType::ForwardCenter,
                PlayerPositionType::ForwardLeft,
                PlayerPositionType::ForwardRight,
            ],
        ) {
            1.0
        } else {
            0.0
        }
    }

    fn utility(&self, player: &Player) -> f32 {
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

    fn impact(player: &Player) -> f32 {
        let attacking = (player.skills.technical.dribbling
            + player.skills.technical.finishing
            + player.skills.mental.flair
            + player.skills.physical.pace)
            / 80.0;
        attacking.clamp(0.0, 1.0)
    }

    /// Best manager-pinned goalkeeper, or `None` when no force-selected keeper
    /// is available to this side. A hard pin: among the force-selected keepers
    /// already passed by the availability gate (so injury / international duty /
    /// condition floor / ban still exclude them upstream), pick the strongest by
    /// the same `goalkeeper_score` the merit picker uses. Returns `None` in a
    /// friendly and for non-Main squads — the same gate as [`Self::honors_force`].
    fn pick_forced_goalkeeper<'p>(
        &self,
        available: &[&'p Player],
        used_ids: &[u32],
    ) -> Option<&'p Player> {
        available
            .iter()
            .copied()
            .filter(|p| !used_ids.contains(&p.id))
            .filter(|p| p.positions.is_goalkeeper())
            .filter(|p| self.honors_force(p))
            .max_by(|a, b| {
                let score = |p: &Player| {
                    self.engine
                        .goalkeeper_score(p, self.staff, self.is_friendly, self.date)
                };
                score(a).partial_cmp(&score(b)).unwrap_or(Ordering::Equal)
            })
    }

    /// The goalkeeper for the fixture, or for the bench behind him.
    ///
    /// A club with a declared order is asked for its keeper: the number
    /// one unless the night is the deputy's by declaration, the manager
    /// acts on a nomination, or a shaken number one is kept out of a big
    /// night; the next man in the order when he cannot play. A club with no
    /// order fields its best eligible keeper by what its coaches can see.
    /// Neither weighs tiredness, football owed, the market or a loan fee.
    fn pick_best_goalkeeper<'p>(
        &self,
        available: &[&'p Player],
        used_ids: &[u32],
        for_starting: bool,
    ) -> Option<&'p Player> {
        let keepers: Vec<&'p Player> = available
            .iter()
            .copied()
            .filter(|p| !used_ids.contains(&p.id))
            .filter(|p| p.positions.is_goalkeeper())
            .collect();
        if let Some(brief) = self.keeper_brief {
            let eligible: Vec<u32> = keepers.iter().map(|p| p.id).collect();
            let asked = if for_starting {
                brief.keeper_for(&self.keeper_fixture(), &eligible)
            } else {
                brief.next_in_order(&eligible)
            };
            if let Some(keeper) = asked.and_then(|id| keepers.iter().copied().find(|p| p.id == id))
            {
                return Some(keeper);
            }
        }
        keepers.into_iter().max_by(|a, b| {
            let score = |p: &Player| {
                self.engine
                    .goalkeeper_score(p, self.staff, self.is_friendly, self.date)
            };
            score(a).partial_cmp(&score(b)).unwrap_or(Ordering::Equal)
        })
    }

    fn keeper_fixture(&self) -> KeeperFixture {
        KeeperFixture {
            stakes: self.stakes,
            domestic_cup_progress: match self.competition {
                SelectionCompetition::DomesticCup {
                    round,
                    total_rounds,
                    ..
                } if total_rounds > 1 => Some(
                    (f32::from(round.saturating_sub(1)) / f32::from(total_rounds - 1)).min(1.0),
                ),
                SelectionCompetition::DomesticCup { .. } => Some(1.0),
                _ => None,
            },
        }
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
}
