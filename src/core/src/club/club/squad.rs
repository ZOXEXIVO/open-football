use super::Club;
use super::graduation_salary;
use crate::club::player::calculators::{
    AutomaticReleaseEligibility, FreeAgentReleaseReason, ReleaseEligibilityContext,
};
use crate::club::ClubPhilosophy;
use crate::club::staff::perception::{AbilityEstimator, CoachProfile, DevelopmentFormEvidence};
use crate::club::team::squad::SquadAssetContext;
use crate::{
    ContractType, Person, Player, PlayerClubContract, PlayerFieldPositionGroup, PlayerStatusType,
    TeamType,
};
use chrono::{Datelike, NaiveDate};
use log::debug;

/// Minimum players a youth/reserve team should keep to remain functional.
const MIN_YOUTH_SQUAD: usize = 11;
/// Minimum players the main team should keep before allowing demotions.
const MIN_MAIN_SQUAD: usize = 22;

impl Club {
    /// Every force-selected player across the club, regardless of the
    /// team they're rostered on. Callers pass these straight to the
    /// squad selector as the first reserves so the +1000 selection
    /// bonus pins them into the match-day XI before the usual scoring
    /// logic decides anything else.
    pub fn get_force_selected_players(&self) -> Vec<&Player> {
        self.teams
            .teams
            .iter()
            .flat_map(|t| t.players.iter())
            .filter(|p| p.is_force_match_selection)
            .collect()
    }

    /// Weekly squad rebalance across all teams.
    ///
    /// Evaluates every player's current team placement and moves them when
    /// the fit is wrong. A single pass handles three situations:
    ///
    /// 1. **Overage** — player too old for their age-limited team (U18/U19),
    ///    move to the next team in progression or main.
    /// 2. **Talent promotion** — youth/reserve player whose ability is
    ///    competitive with the main squad gets pulled up.
    /// 3. **Squad deficit** — if the main team is still below minimum after
    ///    the above, backfill with the best available youth.
    pub(super) fn rebalance_squads(&mut self, date: NaiveDate) {
        let main_idx = match self.teams.main_index() {
            Some(idx) => idx,
            None => return,
        };

        // ── Phase 1: collect every player that needs to move ─────────
        //
        // We scan all non-main teams and decide, per player, whether they
        // should stay, step up one level, or jump to the main team.

        struct PendingMove {
            from: usize,
            to: usize,
            player_id: u32,
            reason: &'static str,
        }

        // Per-position promotion floor on the main team. Using a single
        // global "bottom-3" floor across all positions caused a keeper
        // ping-pong: a youth GK at 82 cleared the global floor (~75)
        // even when the main team already had three senior keepers at
        // 100+, so the depth-cap demoted a keeper every pass and another
        // youth GK got promoted the next week. The position-aware floor
        // below is the real signal — "does this youth displace an actual
        // peer at the same role?" — and it resolves the churn without
        // special-casing the goalkeeper position. Also enforces a minimum
        // depth per group: if main has fewer than `MIN_MAIN_DEPTH` at a
        // position (retirement, transfer, release), any youth above
        // `DEPTH_GAP_FLOOR` is eligible to plug the gap.
        //
        // Both sides of the comparison read the coach-observable level
        // (visible skill + results + training), never the hidden CA
        // digit — a promotion is a staff judgement on what they can see,
        // consistent with the surplus trim below.
        const MIN_MAIN_DEPTH: &[(PlayerFieldPositionGroup, usize)] = &[
            (PlayerFieldPositionGroup::Goalkeeper, 2),
            (PlayerFieldPositionGroup::Defender, 6),
            (PlayerFieldPositionGroup::Midfielder, 6),
            (PlayerFieldPositionGroup::Forward, 4),
        ];
        const DEPTH_GAP_FLOOR: u8 = 60;

        let group_stats = |group: PlayerFieldPositionGroup| -> (usize, u8) {
            let (count, worst) = self.teams.teams[main_idx]
                .players
                .iter()
                .filter(|p| p.position().position_group() == group)
                .map(|p| AbilityEstimator::observable_level(p))
                .fold((0usize, u8::MAX), |(c, w), a| (c + 1, w.min(a)));
            (count, if count == 0 { 0 } else { worst })
        };

        let promotion_threshold = |group: PlayerFieldPositionGroup| -> u8 {
            let (count, worst) = group_stats(group);
            let min_depth = MIN_MAIN_DEPTH
                .iter()
                .find(|(g, _)| *g == group)
                .map(|(_, d)| *d)
                .unwrap_or(0);
            if count < min_depth {
                DEPTH_GAP_FLOOR
            } else {
                // Strictly greater than the current worst — equal CA wouldn't
                // improve depth but would still trigger the demotion cycle.
                worst.saturating_add(1)
            }
        };

        let mut moves: Vec<PendingMove> = Vec::new();

        // Club-level promotion aggressiveness, computed once: a head coach
        // who judges potential well moves on a prospect earlier, and a
        // develop-and-sell club earlier still. The per-player senior-cameo
        // discount stacks on top inside the loop.
        let head_coach_profile =
            CoachProfile::from_staff(self.teams.teams[main_idx].staffs.head_coach());
        let club_discount = PromotionEvidence::club_discount(&head_coach_profile, &self.philosophy);

        for (ti, team) in self.teams.iter().enumerate() {
            if ti == main_idx || team.team_type == TeamType::Main {
                continue;
            }

            // Graduate-out age for a development squad — use
            // `development_age_cap` (U18→18 … U23→23), NOT `max_age`, which
            // bounds only U18/U19. With `max_age` a U20/U21/U23 player was
            // never flagged overage, so a keeper (or anyone) not good enough
            // for a talent promotion could sit in a youth squad into his
            // mid-20s: never playing senior football, and invisible to every
            // ambition audit (which cover Main / B / Reserve / Second only).
            let graduate_out_age = team.team_type.development_age_cap();

            for p in team.players.iter() {
                let age = p.age(date);
                let level = AbilityEstimator::observable_level(p);
                let overage = graduate_out_age.map_or(false, |limit| age > limit);

                // Never promote players marked for departure
                let listed =
                    p.statuses.has(PlayerStatusType::Lst) || p.statuses.has(PlayerStatusType::Loa);

                // Senior cameos already earned via matchday call-ups are
                // direct evidence the player belongs — each one buys the
                // bar down, so the staged pipeline (call-up → cameos →
                // promotion) converges instead of waiting for the kid to
                // out-level a senior on the training pitch alone.
                let cameo_discount = PromotionEvidence::cameo_discount(p, team.team_type);
                let floor = promotion_threshold(p.position().position_group())
                    .saturating_sub(club_discount + cameo_discount);
                if level >= floor && !listed {
                    moves.push(PendingMove {
                        from: ti,
                        to: main_idx,
                        player_id: p.id,
                        reason: "skill level ready for first team",
                    });
                    continue;
                }

                // Overage → move to next team in progression (or main)
                if overage {
                    let next = self.find_next_youth_team(team.team_type, age);
                    // Listed players: only move within youth progression, not to main
                    let dest = if listed {
                        match next {
                            Some(idx) => idx,
                            None => continue, // no youth team available, skip
                        }
                    } else {
                        // Too old for any youth tier → a senior reserve
                        // (Reserve / B / Second) so he keeps playing
                        // competitive football and the reserve-ambition audit
                        // can act on his case; only when the club has no
                        // reserve at all does he land on the main bench, where
                        // the positional-surplus pass then loans or lists him.
                        next.or_else(|| self.find_demotion_target(age))
                            .unwrap_or(main_idx)
                    };
                    moves.push(PendingMove {
                        from: ti,
                        to: dest,
                        player_id: p.id,
                        reason: "overage for current team",
                    });
                }
            }
        }

        // ── Phase 1b: positional-surplus demotion from main ──────────
        //
        // Enforce a depth cap per position group on the main team.
        // Players ranked beyond the cap (by current ability) are
        // surplus: stamp Loa and push them down the progression so
        // they can get match practice in reserve/youth. Loan-ins
        // count against the cap (they occupy a slot) but are never
        // demoted — they belong to another club.
        const MAIN_DEPTH: &[(PlayerFieldPositionGroup, usize)] = &[
            (PlayerFieldPositionGroup::Goalkeeper, 3),
            (PlayerFieldPositionGroup::Defender, 9),
            (PlayerFieldPositionGroup::Midfielder, 9),
            (PlayerFieldPositionGroup::Forward, 6),
        ];

        let mut surplus: Vec<(u32, u8)> = Vec::new();
        for (group, depth) in MAIN_DEPTH {
            let mut ranked: Vec<(u32, u8, u8, bool, bool)> = self.teams.teams[main_idx]
                .players
                .iter()
                .filter(|p| p.position().position_group() == *group)
                .map(|p| {
                    (
                        p.id,
                        p.player_attributes.current_ability,
                        p.age(date),
                        p.is_on_loan(),
                        p.is_force_match_selection,
                    )
                })
                .collect();
            ranked.sort_by(|a, b| b.1.cmp(&a.1));
            for (player_id, _, age, is_loan_in, is_locked) in ranked.into_iter().skip(*depth) {
                if is_loan_in || is_locked {
                    continue;
                }
                surplus.push((player_id, age));
            }
        }

        for &(player_id, age) in &surplus {
            // Where can we send them? Single-team clubs (Maltese top
            // flight, San Marino, etc.) often return None here because
            // there's no reserve/youth team to absorb the demotion.
            let demotion_target = self.find_demotion_target(age);

            if let Some(p) = self.teams.teams[main_idx].players.find_mut(player_id) {
                // No reserve/youth to demote to AND the player is too
                // old to loan? Flag for transfer instead so the surplus
                // can actually leave the club. Without this, single-
                // team clubs accumulate veterans indefinitely (see
                // Gzira: 4× 33-35 GKs sitting on the main roster
                // because they can't loan and can't demote).
                //
                // Convention: club-scoped listers set
                // `contract.is_transfer_listed` only — the country
                // listing pass owns the market listing + `Lst` status.
                // Stamping `Lst` (or `Loa`) here trips that pass's
                // already-listed guard, so the veteran showed as
                // "Listed" while never actually reaching the market.
                if demotion_target.is_none() && age >= 30 {
                    let newly_flagged = p
                        .contract
                        .as_mut()
                        .map(|c| {
                            let first = !c.is_transfer_listed;
                            c.is_transfer_listed = true;
                            first
                        })
                        .unwrap_or(false);
                    if newly_flagged {
                        p.decision_history.add(
                            date,
                            "dec_transfer_listed".to_string(),
                            "dec_reason_surplus_squad".to_string(),
                            "dec_decided_board".to_string(),
                        );
                    }
                } else if !p.statuses.has(PlayerStatusType::Loa) {
                    p.statuses.add(date, PlayerStatusType::Loa);
                }
            }
            if let Some(dest) = demotion_target {
                moves.push(PendingMove {
                    from: main_idx,
                    to: dest,
                    player_id,
                    reason: "surplus at position",
                });
            }
        }

        // ── Phase 2: execute moves, respecting squad-size guards ─────

        // Sort: talent promotions (to main) first, then overage moves,
        // and within each group highest ability first.
        moves.sort_by(|a, b| {
            let a_main = (a.to == main_idx) as u8;
            let b_main = (b.to == main_idx) as u8;
            b_main.cmp(&a_main) // main-team moves first
        });

        // Track how many players we've taken from each source team
        // so we don't drain any team below minimum.
        let mut taken: Vec<usize> = vec![0; self.teams.teams.len()];

        for m in &moves {
            let source_size = self.teams.teams[m.from].players.players.len();
            let already_taken = taken[m.from];

            // Don't drain any team below its minimum viable squad,
            // unless the player is overage or a positional-surplus
            // demotion (both must leave regardless — backfill will
            // restore the size from youth).
            let min_for_source = if m.from == main_idx {
                MIN_MAIN_SQUAD
            } else {
                MIN_YOUTH_SQUAD
            };
            if m.reason != "overage for current team"
                && m.reason != "surplus at position"
                && source_size.saturating_sub(already_taken) <= min_for_source
            {
                continue;
            }

            let from_info = self.teams.teams[m.from].history_info();
            let to_info = self.teams.teams[m.to].history_info();
            let from_senior = self.teams.teams[m.from].team_type.is_own_team();
            let to_senior = self.teams.teams[m.to].team_type.is_own_team();

            if let Some(mut player) = self.teams.teams[m.from].players.take_player(&m.player_id) {
                // Upgrade youth contract to full when promoting to main
                if m.to == main_idx {
                    ProfessionalContractPromotion::upgrade(
                        &mut player,
                        date,
                        self.teams.teams[main_idx].reputation.world,
                    );
                    // Career-defining promotion to senior football. Long
                    // cooldown (effectively one-shot per spell) keeps the
                    // event scarce — a player who yo-yos between reserve
                    // and main shouldn't get a fresh "breakthrough" each
                    // bounce.
                    player.on_youth_breakthrough(date);
                }

                // Close the previous spell and open one on the destination
                // team so future official matches accumulate against the
                // team the player actually plays for. Without this, B-team
                // appearances kept being recorded under the Main row.
                player.on_intra_club_move(&from_info, &to_info, from_senior, to_senior, date);

                debug!(
                    "squad rebalance: {} (CA={}, age={}) {} → {} ({})",
                    player.full_name,
                    player.player_attributes.current_ability,
                    player.age(date),
                    from_info.name,
                    to_info.name,
                    m.reason,
                );
                self.teams.teams[m.to].players.add(player);
                taken[m.from] += 1;
            }
        }

        // ── Phase 3: backfill if main team is still short ────────────

        let main_count = self.teams.teams[main_idx].players.players.len();

        if main_count < MIN_MAIN_SQUAD {
            let deficit = MIN_MAIN_SQUAD - main_count;
            let mut candidates: Vec<(usize, u32, u8)> = Vec::new();

            for (ti, team) in self.teams.iter().enumerate() {
                if ti == main_idx || team.team_type == TeamType::Main {
                    continue;
                }
                let available = team.players.len().saturating_sub(taken[ti]);
                if available <= MIN_YOUTH_SQUAD && team.team_type.max_age().is_some() {
                    continue;
                }
                for p in team.players.iter() {
                    if p.statuses.has(PlayerStatusType::Lst)
                        || p.statuses.has(PlayerStatusType::Loa)
                    {
                        continue;
                    }
                    if p.is_force_match_selection {
                        continue;
                    }
                    candidates.push((ti, p.id, AbilityEstimator::observable_level(p)));
                }
            }

            candidates.sort_by(|a, b| b.2.cmp(&a.2));
            candidates.truncate(deficit);

            for (team_idx, player_id, _) in candidates {
                let from_info = self.teams.teams[team_idx].history_info();
                let to_info = self.teams.teams[main_idx].history_info();
                let from_senior = self.teams.teams[team_idx].team_type.is_own_team();
                let to_senior = self.teams.teams[main_idx].team_type.is_own_team();
                if let Some(mut player) = self.teams.teams[team_idx].players.take_player(&player_id)
                {
                    ProfessionalContractPromotion::upgrade(
                        &mut player,
                        date,
                        self.teams.teams[main_idx].reputation.world,
                    );
                    player.on_youth_breakthrough(date);
                    player.on_intra_club_move(&from_info, &to_info, from_senior, to_senior, date);
                    debug!(
                        "backfill to main: {} (CA={}, age={}) from {}",
                        player.full_name,
                        player.player_attributes.current_ability,
                        player.age(date),
                        from_info.name
                    );
                    self.teams.teams[main_idx].players.add(player);
                }
            }
        }
    }

    /// Weekly: award a first professional contract to youth-team players
    /// whose form has earned it, without waiting for a main-team
    /// promotion.
    ///
    /// Previously a youth contract only became a full one when the player
    /// was good enough to be pulled into the senior squad (see
    /// [`ProfessionalContractPromotion::upgrade`] at the promotion sites).
    /// That meant a standout 17/18-year-old stayed on youth terms — and,
    /// because the
    /// utilization audit skips youth contracts, stayed invisible to the
    /// loan market — until he could already displace a senior. Real clubs
    /// hand a promising academy player pro terms on the back of good
    /// results and *then* send him out on loan to develop. This pass
    /// models that: a youth player with a real run of strong games is
    /// upgraded in place, in his current youth/reserve squad.
    pub(super) fn review_youth_contracts(&mut self, date: NaiveDate) {
        let main_idx = match self.teams.main_index() {
            Some(idx) => idx,
            None => return,
        };
        let club_rep = self.teams.teams[main_idx].reputation.world;

        // Collect first, mutate second: the scan borrows the team
        // collection immutably, so we can't upgrade in the same loop.
        let mut earned: Vec<(usize, u32)> = Vec::new();
        for (ti, team) in self.teams.iter().enumerate() {
            if ti == main_idx {
                continue;
            }
            for player in team.players.iter() {
                if ProfessionalContractPromotion::is_earned(player, date) {
                    earned.push((ti, player.id));
                }
            }
        }

        for (ti, player_id) in earned {
            let team_name = self.teams.teams[ti].name.clone();
            if let Some(player) = self.teams.teams[ti].players.find_mut(player_id) {
                ProfessionalContractPromotion::upgrade(player, date, club_rep);
                // First pro deal — a genuine career milestone for the
                // player, distinct from the senior-debut breakthrough.
                player.on_professional_contract_awarded();
                debug!(
                    "youth → pro contract on merit: {} (CA={}, age={}) at {}",
                    player.full_name,
                    player.player_attributes.current_ability,
                    player.age(date),
                    team_name,
                );
            }
        }
    }

    /// Find the next youth team in progression (U18→U19→U20→U21→U23)
    /// that exists in this club and can accept a player of the given age.
    fn find_next_youth_team(&self, current_type: TeamType, player_age: u8) -> Option<usize> {
        let progression = TeamType::YOUTH_PROGRESSION;

        let current_pos = progression.iter().position(|t| *t == current_type)?;

        for next_type in &progression[current_pos + 1..] {
            // Skip a tier the player has already outgrown (graduate-out age),
            // so an overage player lands on the youngest tier that still fits
            // — or, if too old for all of them, `None` falls through to a
            // senior squad at the call site.
            let age_ok = match next_type.development_age_cap() {
                Some(cap) => player_age <= cap,
                None => true,
            };
            if age_ok {
                if let Some(idx) = self.teams.index_of_type(*next_type) {
                    return Some(idx);
                }
            }
        }

        None
    }

    /// Best non-main destination for a demoted main-team player.
    /// Adult teams (Reserve, B) come first so surplus seniors keep
    /// playing competitive matches; absent those, fall back to the
    /// youth team that fits the player's age.
    fn find_demotion_target(&self, age: u8) -> Option<usize> {
        for t in [TeamType::Reserve, TeamType::B, TeamType::Second] {
            if let Some(idx) = self.teams.index_of_type(t) {
                return Some(idx);
            }
        }
        self.find_youth_team_for_age(age)
    }

    /// Find the best-fitting youth team for a player of the given age.
    /// Returns the youngest team the player is eligible for.
    fn find_youth_team_for_age(&self, player_age: u8) -> Option<usize> {
        let targets: [(TeamType, u8); 5] = [
            (TeamType::U18, 18),
            (TeamType::U19, 19),
            (TeamType::U20, 20),
            (TeamType::U21, 21),
            (TeamType::U23, 23),
        ];

        for (team_type, max_age) in targets {
            if player_age <= max_age {
                if let Some(idx) = self.teams.index_of_type(team_type) {
                    return Some(idx);
                }
            }
        }
        None
    }

    /// Move players without a contract (loan returnees) from main team to reserve.
    /// Loan returns land on teams[0] (main) — staff then moves them to reserve for assessment.
    pub(super) fn move_loan_returns_to_reserve(&mut self, date: NaiveDate) {
        let main_idx = match self.teams.main_index() {
            Some(idx) => idx,
            None => return,
        };

        let reserve_idx = self
            .teams
            .index_of_type(TeamType::Reserve)
            .or_else(|| self.teams.index_of_type(TeamType::B))
            .or_else(|| self.teams.index_of_type(TeamType::Second));

        let reserve_idx = match reserve_idx {
            Some(idx) => idx,
            None => return, // no reserve team, stay on main
        };

        // Find main team players with no contract (returned from loan).
        // Force-selected players stay on main even if their contract slot
        // is empty — the manager has pinned them in.
        let to_move: Vec<u32> = self.teams.teams[main_idx]
            .players
            .iter()
            .filter(|p| p.contract.is_none() && !p.is_force_match_selection)
            .map(|p| p.id)
            .collect();

        // Close the Main spell and open one on the reserve/Second team so
        // the player's appearances there land under the right history row
        // instead of leaking into the stale active Main entry.
        let from_info = self.teams.teams[main_idx].history_info();
        let to_info = self.teams.teams[reserve_idx].history_info();
        let from_senior = self.teams.teams[main_idx].team_type.is_own_team();
        let to_senior = self.teams.teams[reserve_idx].team_type.is_own_team();

        for player_id in to_move {
            if let Some(mut player) = self.teams.teams[main_idx].players.take_player(&player_id) {
                debug!(
                    "loan return -> reserve: {} moved to {}",
                    player.full_name, self.teams.teams[reserve_idx].name
                );
                player.on_intra_club_move(&from_info, &to_info, from_senior, to_senior, date);
                self.teams.teams[reserve_idx].players.add(player);
            }
        }
    }

    /// Trim excess players at over-represented positions. Caps scale
    /// per-team so a club with main + reserve + U18 is allowed the depth a
    /// real club carries; previously a flat cross-club cap deleted
    /// legitimate squad members every season start.
    ///
    /// Surplus players are no longer all released outright: only those
    /// passing `AutomaticReleaseEligibility` (clearly below team level,
    /// negligible market value, affordable severance) become free agents
    /// on the roster (contract cleared, Frt set) for the country-level
    /// free-agent pipeline. Everyone else is flagged for sale instead —
    /// the country listing pass picks `is_transfer_listed` up and creates
    /// the market listing with a real asking price.
    pub(super) fn trim_positional_surplus(&mut self, date: NaiveDate) {
        // Per-team caps. Total ~30 per team covers a realistic first-team
        // squad + cover depth; multiplied across main + reserve + U18
        // (typically 3 teams) this is ~90, in line with real clubs.
        let team_count = self.teams.teams.len().max(1);
        let limits: [(PlayerFieldPositionGroup, usize); 4] = [
            (PlayerFieldPositionGroup::Goalkeeper, 4 * team_count),
            (PlayerFieldPositionGroup::Defender, 10 * team_count),
            (PlayerFieldPositionGroup::Midfielder, 10 * team_count),
            (PlayerFieldPositionGroup::Forward, 7 * team_count),
        ];

        // Club-level context for the release-eligibility gate, computed
        // once. League reputation isn't reachable from club scope — use
        // the main team's world reputation as the pricing proxy, the same
        // trade-off the renewal pass documents in `simulate`.
        let (squad_avg_ability, club_reputation, league_reputation_proxy) = match self.teams.main()
        {
            Some(main) => (
                main.players.current_ability_avg(),
                main.reputation.market_value_score(),
                main.reputation.world,
            ),
            None => return,
        };
        let annual_wage_bill: u32 = self.teams.iter().map(|t| t.get_annual_salary()).sum();

        // Central squad-asset classification, built once against the pre-trim
        // squad. Threaded into the release gate so a player who is core /
        // first-team / recognised / merely-unevaluated is never walked for
        // free — only genuine surplus is. Owns its data, so the mutable trim
        // loop below holds no borrow on it.
        let asset_ctx = SquadAssetContext::build(self, date);

        for (group, max_count) in &limits {
            // Active players only: if a player has already been released
            // (no contract, Frt) they shouldn't count against the cap, or
            // we'd re-release them every season until someone signs.
            // Rank by the coach-observable level (visible skill + results +
            // training), not the hidden CA digit: who is "surplus to squad
            // requirements" is a judgement on how a player performs and
            // applies himself, so the worst-first order the trim walks is his
            // assessed standing. Each candidate still passes the observable
            // `classify` + release gate below before anything happens to him.
            let mut players_at_pos: Vec<(usize, u32, u8)> = Vec::new();
            for (ti, team) in self.teams.iter().enumerate() {
                for p in team.players.iter() {
                    if p.contract.is_none() || p.is_on_loan() {
                        continue;
                    }
                    if p.position().position_group() == *group {
                        players_at_pos.push((ti, p.id, AbilityEstimator::observable_level(p)));
                    }
                }
            }

            if players_at_pos.len() <= *max_count {
                continue;
            }

            // Sort by observable level ascending — move the worst out first
            players_at_pos.sort_by_key(|&(_, _, level)| level);

            let to_trim = players_at_pos.len() - max_count;
            for &(team_idx, player_id, _) in players_at_pos.iter().take(to_trim) {
                if self.teams.teams[team_idx].team_type.max_age().is_some()
                    && self.teams.teams[team_idx].players.players.len() <= MIN_YOUTH_SQUAD
                {
                    continue;
                }
                let team_name = self.teams.teams[team_idx].name.clone();
                if let Some(player) = self.teams.teams[team_idx]
                    .players
                    .players
                    .iter_mut()
                    .find(|p| p.id == player_id)
                {
                    // Already on the market from an earlier pass — the
                    // transfer pipeline owns this player now.
                    let already_listed = player.statuses.has(PlayerStatusType::Lst)
                        || player
                            .contract
                            .as_ref()
                            .map(|c| c.is_transfer_listed)
                            .unwrap_or(false);
                    if already_listed {
                        continue;
                    }
                    let market_value = player.value(date, league_reputation_proxy, club_reputation);
                    let termination_cost = player
                        .contract
                        .as_ref()
                        .map(|c| c.termination_cost(date))
                        .unwrap_or(0);
                    let release_ctx = ReleaseEligibilityContext {
                        date,
                        squad_avg_ability,
                        market_value,
                        annual_wage_bill,
                        asset_class: asset_ctx.classify(player, date),
                    };
                    match AutomaticReleaseEligibility::assess(player, &release_ctx) {
                        None => {
                            debug!(
                                "positional surplus release: {} (id={}, {:?}, CA={} vs avg {}, \
                                 value={:.0}, severance={}) from {}",
                                player.full_name,
                                player.id,
                                group,
                                player.player_attributes.current_ability,
                                squad_avg_ability,
                                market_value,
                                termination_cost,
                                team_name
                            );
                            player.contract = None;
                            if !player.statuses.has(PlayerStatusType::Frt) {
                                player.statuses.add(date, PlayerStatusType::Frt);
                            }
                            // Stamp the explicit exit so the free-agent
                            // sweep records a "squad surplus" free release,
                            // distinct from a negotiated mutual termination.
                            player.set_release_reason(FreeAgentReleaseReason::SurplusFreeRelease);
                            player.decision_history.add(
                                date,
                                "dec_free_transfer_listed".to_string(),
                                FreeAgentReleaseReason::SurplusFreeRelease
                                    .history_reason()
                                    .to_string(),
                                "dec_decided_board".to_string(),
                            );
                        }
                        Some(block) => {
                            // Worth keeping under contract: flag for sale. Only
                            // the contract flag is set — the country listing
                            // pass turns `is_transfer_listed` into a market
                            // listing + `Lst` status; stamping `Lst` here would
                            // trip its already-listed guard and strand the
                            // player off-market. This entry is the single
                            // decision-history record for the listing — the
                            // country pass deliberately skips writing one for
                            // pre-flagged players.
                            if let Some(contract) = player.contract.as_mut() {
                                contract.is_transfer_listed = true;
                            }
                            player.decision_history.add(
                                date,
                                "dec_transfer_listed".to_string(),
                                "dec_reason_surplus_squad".to_string(),
                                "dec_decided_board".to_string(),
                            );
                            debug!(
                                "positional surplus listed instead of released: {} (id={}, {:?}, \
                                 CA={} vs avg {}, value={:.0}, severance={}) from {} — blocked: {:?}",
                                player.full_name,
                                player.id,
                                group,
                                player.player_attributes.current_ability,
                                squad_avg_ability,
                                market_value,
                                termination_cost,
                                team_name,
                                block
                            );
                        }
                    }
                }
            }
        }
    }
}

/// Evidence-based adjustments to the first-team promotion bar in
/// [`Club::rebalance_squads`]. Real clubs don't wait for a prospect to
/// out-train the worst senior: senior cameos already played, a coach who
/// reads potential well, and a development-first club identity all bring
/// the decision forward. Every input is observable — appearance counters
/// and staff profile, never hidden attributes.
struct PromotionEvidence;

impl PromotionEvidence {
    /// Bar discount per senior appearance already made (observable-level
    /// points on the 1..200 scale).
    const PER_SENIOR_APP: u16 = 3;
    /// Cap on the cameo discount — four-plus senior games are a made
    /// case; more cameos shouldn't erode the bar indefinitely.
    const CAMEO_DISCOUNT_CAP: u16 = 12;
    /// Reach of the head coach's potential judgement on the bar.
    const JUDGEMENT_SPAN: f32 = 4.0;
    /// Extra aggressiveness for a develop-and-sell club.
    const DEVELOP_AND_SELL_DISCOUNT: u8 = 4;

    /// Senior appearances a youth-rostered player has already made.
    /// Youth-league fixtures are friendly-flagged and book into the
    /// friendly bucket, so for an age-restricted squad the official
    /// league + cup counters only move when the player turns out for a
    /// senior side — they ARE his senior-cameo record. Senior reserve
    /// squads (B/Second/Reserve) play official football of their own, so
    /// no such read exists for them.
    fn cameo_discount(player: &Player, team_type: TeamType) -> u8 {
        if !team_type.is_youth() {
            return 0;
        }
        let senior_apps = player.statistics.played
            + player.statistics.played_subs
            + player.cup_statistics.played
            + player.cup_statistics.played_subs;
        (senior_apps * Self::PER_SENIOR_APP).min(Self::CAMEO_DISCOUNT_CAP) as u8
    }

    /// Club-level bar discount from the head coach's potential judgement
    /// and the club philosophy.
    fn club_discount(profile: &CoachProfile, philosophy: &ClubPhilosophy) -> u8 {
        let judgement =
            (profile.potential_accuracy.clamp(0.0, 1.0) * Self::JUDGEMENT_SPAN).round() as u8;
        let identity = match philosophy {
            ClubPhilosophy::DevelopAndSell => Self::DEVELOP_AND_SELL_DISCOUNT,
            _ => 0,
        };
        judgement + identity
    }
}

/// Youth → professional contract promotion on merit: deciding whether a
/// youth-contract player has earned pro terms, and performing the upgrade.
/// Both the promotion sites in [`Club::rebalance_squads`] and the weekly
/// [`Club::review_youth_contracts`] pass route through here.
struct ProfessionalContractPromotion;

impl ProfessionalContractPromotion {
    /// Meaningful sample — mirrors the "established regular" bar used by
    /// the stalled-prospect pathway.
    const MIN_GAMES: u16 = 8;
    /// Clearly above the ~6.6 positional neutral: a standout youth
    /// season, not merely "featured".
    const MIN_RATING: f32 = 7.0;
    /// Real-world floor for signing professional terms.
    const MIN_AGE: u8 = 16;

    /// Has this youth-contract player earned a first professional
    /// contract? A pure *results* signal, never raw ability or potential:
    /// a real sample of matches at a reliability-adjusted rating clearly
    /// above the positional neutral (~6.6). The realistic average already
    /// regresses small samples toward neutral, so a hot three-game streak
    /// can't trigger it — the player must sustain the form.
    ///
    /// Reads [`DevelopmentFormEvidence`] — the season bucket that actually
    /// carries the player's football. Youth-league fixtures book into the
    /// friendly bucket, so judging only the official counters (as this
    /// gate originally did) made academy-league form invisible and the
    /// merit upgrade effectively fired only off borrowed senior minutes.
    fn is_earned(player: &Player, date: NaiveDate) -> bool {
        let is_youth = player
            .contract
            .as_ref()
            .map(|c| c.contract_type == ContractType::Youth)
            .unwrap_or(false);
        if !is_youth {
            return false;
        }

        // Don't upgrade a player already earmarked to leave.
        if player.statuses.has(PlayerStatusType::Lst) || player.statuses.has(PlayerStatusType::Loa)
        {
            return false;
        }

        if player.age(date) < Self::MIN_AGE {
            return false;
        }
        if DevelopmentFormEvidence::games(player) < Self::MIN_GAMES {
            return false;
        }

        DevelopmentFormEvidence::regressed_rating(player) >= Self::MIN_RATING
    }

    /// Upgrade a youth contract to a full professional contract. Used both
    /// when a player is promoted to the main team and when a youth-team
    /// player earns pro terms on form. `club_rep` is the main team's world
    /// reputation — it scales the graduation salary so the same ability
    /// earns far more at a big club.
    fn upgrade(player: &mut Player, date: NaiveDate, club_rep: u16) {
        let is_youth = player
            .contract
            .as_ref()
            .map(|c| c.contract_type == ContractType::Youth)
            .unwrap_or(false);
        if !is_youth {
            return;
        }

        let expiration = NaiveDate::from_ymd_opt(date.year() + 3, date.month(), date.day().min(28))
            .unwrap_or(date);
        let salary = graduation_salary(player.player_attributes.current_ability, club_rep);
        let mut upgraded = PlayerClubContract::new(salary, expiration);
        // Anchor the new senior contract to today so the wage-envy
        // grace window correctly treats a freshly graduated youngster
        // as "just signed" — otherwise the monthly audit fires
        // SalaryGapNoticed on a player who only just earned a senior
        // wage in the first place.
        upgraded.started = Some(date);
        player.contract = Some(upgraded);
    }
}

#[cfg(test)]
mod trim_surplus_tests {
    use super::*;
    use crate::academy::ClubAcademy;
    use crate::club::player::core::builder::PlayerBuilder;
    use crate::country::result::CountryResult;
    use crate::country::result::transfers::types::TransferActivitySummary;
    use crate::league::{DayMonthPeriod, League, LeagueCollection, LeagueSettings};
    use crate::shared::Location;
    use crate::shared::fullname::FullName;
    use crate::{
        ClubColors, ClubFacilities, ClubFinances, ClubStatus, Country, PersonAttributes,
        PlayerAttributes, PlayerCollection, PlayerPosition, PlayerPositionType, PlayerPositions,
        PlayerSkills, StaffCollection, TeamBuilder, TeamCollection, TeamReputation,
        TrainingSchedule,
    };
    use chrono::Duration;

    /// Fixtures for the surplus-trim gate. A single Main team means the
    /// goalkeeper cap is 4, so a fifth keeper is always the trim
    /// candidate; ability/salary/length of that candidate then steers
    /// which side of the release-vs-list decision fires.
    struct Fixture;

    impl Fixture {
        fn date() -> NaiveDate {
            NaiveDate::from_ymd_opt(2026, 6, 12).unwrap()
        }

        fn goalkeeper(id: u32, ability: u8, age: u8, salary: u32, contract_months: u32) -> Player {
            let date = Self::date();
            let expiration = date + Duration::days(contract_months as i64 * 30);
            let mut attrs = PlayerAttributes::default();
            attrs.current_ability = ability;
            attrs.potential_ability = ability;
            PlayerBuilder::new()
                .id(id)
                .full_name(FullName::new("Test".to_string(), format!("Keeper{}", id)))
                .birth_date(NaiveDate::from_ymd_opt(date.year() - age as i32, 1, 1).unwrap())
                .country_id(1)
                .attributes(PersonAttributes::default())
                .skills(PlayerSkills::flat_for_ability(ability))
                .positions(PlayerPositions {
                    positions: vec![PlayerPosition {
                        position: PlayerPositionType::Goalkeeper,
                        level: 20,
                    }],
                })
                .player_attributes(attrs)
                .contract(Some(PlayerClubContract::new(salary, expiration)))
                .build()
                .unwrap()
        }

        fn training_schedule() -> TrainingSchedule {
            use chrono::NaiveTime;
            TrainingSchedule::new(
                NaiveTime::from_hms_opt(9, 0, 0).unwrap(),
                NaiveTime::from_hms_opt(15, 0, 0).unwrap(),
            )
        }

        fn club(players: Vec<Player>) -> Club {
            let team = TeamBuilder::new()
                .id(10)
                .league_id(Some(1))
                .club_id(100)
                .name("Main".to_string())
                .slug("main".to_string())
                .team_type(TeamType::Main)
                .players(PlayerCollection::new(players))
                .staffs(StaffCollection::new(Vec::new()))
                .reputation(TeamReputation::new(500, 500, 500))
                .training_schedule(Self::training_schedule())
                .build()
                .unwrap();
            Club::new(
                100,
                "Club".to_string(),
                Location::new(1),
                ClubFinances::new(10_000_000, Vec::new()),
                ClubAcademy::new(3),
                ClubStatus::Professional,
                ClubColors::default(),
                TeamCollection::new(vec![team]),
                ClubFacilities::default(),
            )
        }

        fn find_player(club: &Club, id: u32) -> &Player {
            club.teams.teams[0]
                .players
                .players
                .iter()
                .find(|p| p.id == id)
                .expect("trimmed player must stay on the roster")
        }

        /// Two-squad club for the cross-system tests: Main (id 10, league
        /// 1) plus a Reserve roster (id 20, no league of its own).
        fn club_with_reserve(main_players: Vec<Player>, reserve_players: Vec<Player>) -> Club {
            let main = TeamBuilder::new()
                .id(10)
                .league_id(Some(1))
                .club_id(100)
                .name("Main".to_string())
                .slug("main".to_string())
                .team_type(TeamType::Main)
                .players(PlayerCollection::new(main_players))
                .staffs(StaffCollection::new(Vec::new()))
                .reputation(TeamReputation::new(500, 500, 500))
                .training_schedule(Self::training_schedule())
                .build()
                .unwrap();
            let reserve = TeamBuilder::new()
                .id(20)
                .league_id(None)
                .club_id(100)
                .name("Reserve".to_string())
                .slug("reserve".to_string())
                .team_type(TeamType::Reserve)
                .players(PlayerCollection::new(reserve_players))
                .staffs(StaffCollection::new(Vec::new()))
                .reputation(TeamReputation::new(300, 300, 300))
                .training_schedule(Self::training_schedule())
                .build()
                .unwrap();
            Club::new(
                100,
                "Club".to_string(),
                Location::new(1),
                ClubFinances::new(10_000_000, Vec::new()),
                ClubAcademy::new(3),
                ClubStatus::Professional,
                ClubColors::default(),
                TeamCollection::new(vec![main, reserve]),
                ClubFacilities::default(),
            )
        }

        fn country(club: Club) -> Country {
            let league = League::new(
                1,
                "L".to_string(),
                "l".to_string(),
                1,
                500,
                LeagueSettings {
                    season_starting_half: DayMonthPeriod::new(1, 8, 31, 12),
                    season_ending_half: DayMonthPeriod::new(1, 1, 31, 5),
                    tier: 1,
                    promotion_spots: 0,
                    relegation_spots: 0,
                    league_group: None,
                },
                false,
            );
            Country::builder()
                .id(1)
                .code("EN".to_string())
                .slug("en".to_string())
                .name("England".to_string())
                .continent_id(1)
                .leagues(LeagueCollection::new(vec![league]))
                .clubs(vec![club])
                .build()
                .unwrap()
        }
    }

    #[test]
    fn near_level_surplus_is_listed_not_released() {
        // Fifth keeper at CA 95 vs squad avg ~99 — surplus, but nowhere
        // near the -25 release gap. He must be flagged for sale, not
        // walked for free.
        let mut club = Fixture::club(vec![
            Fixture::goalkeeper(1, 100, 28, 50_000, 12),
            Fixture::goalkeeper(2, 100, 28, 50_000, 12),
            Fixture::goalkeeper(3, 100, 28, 50_000, 12),
            Fixture::goalkeeper(4, 100, 28, 50_000, 12),
            Fixture::goalkeeper(5, 95, 28, 50_000, 12),
        ]);

        club.trim_positional_surplus(Fixture::date());

        let trimmed = Fixture::find_player(&club, 5);
        let contract = trimmed
            .contract
            .as_ref()
            .expect("near-level surplus must keep his contract");
        assert!(
            contract.is_transfer_listed,
            "near-level surplus must be flagged for the transfer market"
        );
        assert!(
            !trimmed.statuses.has(PlayerStatusType::Frt),
            "near-level surplus must not be marked released"
        );
        assert!(
            trimmed
                .decision_history
                .items
                .iter()
                .any(|d| d.movement == "dec_transfer_listed"),
            "listing must be explained in decision history"
        );
        // The four keepers inside the cap are untouched.
        for id in 1..=4 {
            let keeper = Fixture::find_player(&club, id);
            assert!(keeper.contract.is_some());
            assert!(!keeper.contract.as_ref().unwrap().is_transfer_listed);
        }
    }

    #[test]
    fn cheap_fringe_veteran_is_released() {
        // CA 40 at age 36 on a tiny, nearly-expired deal — every gate
        // passes, so the classic mutual release fires.
        let mut club = Fixture::club(vec![
            Fixture::goalkeeper(1, 100, 28, 50_000, 12),
            Fixture::goalkeeper(2, 100, 28, 50_000, 12),
            Fixture::goalkeeper(3, 100, 28, 50_000, 12),
            Fixture::goalkeeper(4, 100, 28, 50_000, 12),
            Fixture::goalkeeper(5, 40, 36, 15_000, 3),
        ]);

        club.trim_positional_surplus(Fixture::date());

        let trimmed = Fixture::find_player(&club, 5);
        assert!(
            trimmed.contract.is_none(),
            "cheap fringe veteran must have the contract cleared"
        );
        assert!(
            trimmed.statuses.has(PlayerStatusType::Frt),
            "release must stamp Frt for the free-agent sweep"
        );
        assert!(
            trimmed
                .decision_history
                .items
                .iter()
                .any(|d| d.movement == "dec_free_transfer_listed"
                    && d.decision == "dec_reason_released_surplus"),
            "release must be explained in decision history as a squad-surplus free release"
        );
        assert_eq!(
            trimmed.release_reason(),
            Some(FreeAgentReleaseReason::SurplusFreeRelease),
            "trimmed surplus player must carry the explicit surplus release reason"
        );
    }

    #[test]
    fn blocked_reserve_surplus_reaches_market_with_single_decision_entry() {
        // End-to-end across the two systems sharing the surplus flow: the
        // season-start trim flags a reserve keeper it must not release
        // (near team level → `is_transfer_listed` only), and the country
        // listing pass then turns the flag into a real market listing
        // carrying the reserve team's id. The trim's decision-history
        // entry must stay the only one — the listing pass deliberately
        // writes none for pre-flagged players.
        let date = Fixture::date();
        // GK cap is 4 per team × 2 teams = 8; nine keepers under contract
        // put the worst one (id 5, CA 95, rostered on the Reserve squad)
        // over the cap.
        let main: Vec<Player> = (1..=4)
            .map(|id| Fixture::goalkeeper(id, 100, 28, 50_000, 12))
            .collect();
        let reserve: Vec<Player> = (5..=9)
            .map(|id| Fixture::goalkeeper(id, 90 + id as u8, 28, 50_000, 12))
            .collect();
        let club = Fixture::club_with_reserve(main, reserve);
        let mut country = Fixture::country(club);

        country.clubs[0].trim_positional_surplus(date);

        {
            let flagged = country.clubs[0].teams.teams[1]
                .players
                .players
                .iter()
                .find(|p| p.id == 5)
                .expect("trimmed reserve keeper must stay on the roster");
            assert!(
                flagged
                    .contract
                    .as_ref()
                    .expect("near-level surplus keeps his contract")
                    .is_transfer_listed,
                "near-level reserve surplus must be flagged for sale"
            );
            assert!(
                !flagged.statuses.has(PlayerStatusType::Frt),
                "blocked release candidate must not be marked released"
            );
        }

        let mut summary = TransferActivitySummary::new();
        CountryResult::list_players_from_pipeline(&mut country, date, &mut summary);

        let listing = country
            .transfer_market
            .listings
            .iter()
            .find(|l| l.player_id == 5)
            .expect("flagged reserve player must reach the country transfer market");
        assert_eq!(
            listing.team_id, 20,
            "the market listing must carry the player's real (reserve) team"
        );

        let player = country.clubs[0].teams.teams[1]
            .players
            .players
            .iter()
            .find(|p| p.id == 5)
            .unwrap();
        assert!(
            player.statuses.has(PlayerStatusType::Lst),
            "the listing pass must stamp Lst once the listing is live"
        );
        assert_eq!(
            player
                .decision_history
                .items
                .iter()
                .filter(|d| d.movement == "dec_transfer_listed")
                .count(),
            1,
            "the trim owns the decision entry — the listing pass must not duplicate it"
        );
    }

    #[test]
    fn expensive_full_time_surplus_is_listed_not_released() {
        // Same fringe quality, but two years of a 2M salary left —
        // severance is far beyond the club's tolerance, so the player is
        // listed instead of paid off.
        let mut club = Fixture::club(vec![
            Fixture::goalkeeper(1, 100, 28, 50_000, 12),
            Fixture::goalkeeper(2, 100, 28, 50_000, 12),
            Fixture::goalkeeper(3, 100, 28, 50_000, 12),
            Fixture::goalkeeper(4, 100, 28, 50_000, 12),
            Fixture::goalkeeper(5, 40, 36, 2_000_000, 24),
        ]);

        club.trim_positional_surplus(Fixture::date());

        let trimmed = Fixture::find_player(&club, 5);
        let contract = trimmed
            .contract
            .as_ref()
            .expect("expensive contract must not be torn up automatically");
        assert!(contract.is_transfer_listed);
        assert!(!trimmed.statuses.has(PlayerStatusType::Frt));
    }
}

#[cfg(test)]
mod promotion_evidence_tests {
    use super::*;
    use crate::academy::ClubAcademy;
    use crate::club::player::core::builder::PlayerBuilder;
    use crate::shared::Location;
    use crate::shared::fullname::FullName;
    use crate::{
        ClubColors, ClubFacilities, ClubFinances, ClubStatus, PersonAttributes, PlayerAttributes,
        PlayerCollection, PlayerPosition, PlayerPositionType, PlayerPositions, PlayerSkills,
        StaffCollection, TeamBuilder, TeamCollection, TeamReputation, TrainingSchedule,
    };
    use chrono::{Datelike, NaiveTime};

    struct Fx;

    impl Fx {
        fn date() -> NaiveDate {
            NaiveDate::from_ymd_opt(2026, 10, 1).unwrap()
        }

        fn schedule() -> TrainingSchedule {
            TrainingSchedule::new(
                NaiveTime::from_hms_opt(9, 0, 0).unwrap(),
                NaiveTime::from_hms_opt(15, 0, 0).unwrap(),
            )
        }

        fn player(id: u32, position: PlayerPositionType, ability: u8, age: u8) -> Player {
            let date = Self::date();
            let mut attrs = PlayerAttributes::default();
            attrs.current_ability = ability;
            attrs.potential_ability = ability;
            attrs.condition = 10_000;
            PlayerBuilder::new()
                .id(id)
                .full_name(FullName::new("T".to_string(), format!("P{id}")))
                .birth_date(NaiveDate::from_ymd_opt(date.year() - age as i32, 1, 1).unwrap())
                .country_id(1)
                .attributes(PersonAttributes::default())
                .skills(PlayerSkills::flat_for_ability(ability))
                .positions(PlayerPositions {
                    positions: vec![PlayerPosition {
                        position,
                        level: 18,
                    }],
                })
                .player_attributes(attrs)
                .contract(Some(PlayerClubContract::new(
                    20_000,
                    NaiveDate::from_ymd_opt(2029, 6, 30).unwrap(),
                )))
                .build()
                .unwrap()
        }

        /// 22 seniors at observable ~120, groups inside both the minimum
        /// depth and the surplus caps, so neither the depth-gap floor nor
        /// the demotion pass interferes with the promotion bar under test.
        fn main_roster() -> Vec<Player> {
            let mut players = Vec::new();
            let mut id = 100u32;
            let mut push = |pos: PlayerPositionType, n: usize, id: &mut u32, out: &mut Vec<Player>| {
                for _ in 0..n {
                    out.push(Self::player(*id, pos, 120, 27));
                    *id += 1;
                }
            };
            push(PlayerPositionType::Goalkeeper, 2, &mut id, &mut players);
            push(PlayerPositionType::DefenderCenter, 8, &mut id, &mut players);
            push(PlayerPositionType::MidfielderCenter, 6, &mut id, &mut players);
            push(PlayerPositionType::Striker, 6, &mut id, &mut players);
            players
        }

        /// U19 squad of twelve: the candidate plus eleven fillers far
        /// below any promotion bar, keeping the squad-minimum guard open.
        fn u19_roster(candidate: Player) -> Vec<Player> {
            let mut players = vec![candidate];
            let mut id = 300u32;
            for _ in 0..4 {
                players.push(Self::player(id, PlayerPositionType::DefenderCenter, 50, 17));
                id += 1;
            }
            for _ in 0..4 {
                players.push(Self::player(id, PlayerPositionType::MidfielderCenter, 50, 17));
                id += 1;
            }
            for _ in 0..3 {
                players.push(Self::player(id, PlayerPositionType::Striker, 50, 17));
                id += 1;
            }
            players
        }

        fn club(candidate: Player) -> Club {
            let main = TeamBuilder::new()
                .id(10)
                .league_id(Some(1))
                .club_id(100)
                .name("Main".to_string())
                .slug("main".to_string())
                .team_type(TeamType::Main)
                .players(PlayerCollection::new(Self::main_roster()))
                .staffs(StaffCollection::new(Vec::new()))
                .reputation(TeamReputation::new(500, 500, 500))
                .training_schedule(Self::schedule())
                .build()
                .unwrap();
            let u19 = TeamBuilder::new()
                .id(19)
                .league_id(None)
                .club_id(100)
                .name("U19".to_string())
                .slug("u19".to_string())
                .team_type(TeamType::U19)
                .players(PlayerCollection::new(Self::u19_roster(candidate)))
                .staffs(StaffCollection::new(Vec::new()))
                .reputation(TeamReputation::new(300, 300, 300))
                .training_schedule(Self::schedule())
                .build()
                .unwrap();
            Club::new(
                100,
                "Club".to_string(),
                Location::new(1),
                ClubFinances::new(10_000_000, Vec::new()),
                ClubAcademy::new(3),
                ClubStatus::Professional,
                ClubColors::default(),
                TeamCollection::new(vec![main, u19]),
                ClubFacilities::default(),
            )
        }

        fn on_team(club: &Club, team_idx: usize, id: u32) -> bool {
            club.teams.teams[team_idx]
                .players
                .players
                .iter()
                .any(|p| p.id == id)
        }
    }

    /// The staged pipeline converging: a near-senior U19 midfielder who
    /// has already collected senior cameos (official appearances while
    /// youth-rostered) clears the discounted promotion bar and moves up.
    #[test]
    fn senior_cameos_accelerate_promotion() {
        let mut candidate = Fx::player(1, PlayerPositionType::MidfielderCenter, 112, 17);
        candidate.statistics.played = 5;
        for _ in 0..5 {
            candidate.statistics.record_match_rating(7.0, 90, true);
        }
        let mut club = Fx::club(candidate);

        club.rebalance_squads(Fx::date());

        assert!(
            Fx::on_team(&club, 0, 1),
            "senior-cameo evidence promotes the near-level prospect to the first team"
        );
    }

    /// The same prospect without a single senior appearance stays in the
    /// academy — the bar only comes down on evidence.
    #[test]
    fn no_cameo_evidence_keeps_prospect_in_the_academy() {
        let candidate = Fx::player(1, PlayerPositionType::MidfielderCenter, 112, 17);
        let mut club = Fx::club(candidate);

        club.rebalance_squads(Fx::date());

        assert!(
            !Fx::on_team(&club, 0, 1),
            "without cameo evidence the promotion bar holds"
        );
        assert!(Fx::on_team(&club, 1, 1), "the prospect stays with the U19s");
    }
}

#[cfg(test)]
mod overage_graduation_tests {
    use super::*;
    use crate::academy::ClubAcademy;
    use crate::club::player::core::builder::PlayerBuilder;
    use crate::shared::Location;
    use crate::shared::fullname::FullName;
    use crate::{
        ClubColors, ClubFacilities, ClubFinances, ClubStatus, PersonAttributes, PlayerAttributes,
        PlayerCollection, PlayerPosition, PlayerPositionType, PlayerPositions, PlayerSkills,
        StaffCollection, TeamBuilder, TeamCollection, TeamReputation, TrainingSchedule,
    };
    use chrono::{Datelike, NaiveTime};

    struct Fx;

    impl Fx {
        fn date() -> NaiveDate {
            NaiveDate::from_ymd_opt(2026, 10, 1).unwrap()
        }

        fn schedule() -> TrainingSchedule {
            TrainingSchedule::new(
                NaiveTime::from_hms_opt(9, 0, 0).unwrap(),
                NaiveTime::from_hms_opt(15, 0, 0).unwrap(),
            )
        }

        fn player(id: u32, position: PlayerPositionType, ability: u8, age: u8) -> Player {
            let date = Self::date();
            let mut attrs = PlayerAttributes::default();
            attrs.current_ability = ability;
            attrs.potential_ability = ability;
            attrs.condition = 10_000;
            PlayerBuilder::new()
                .id(id)
                .full_name(FullName::new("T".to_string(), format!("P{id}")))
                .birth_date(NaiveDate::from_ymd_opt(date.year() - age as i32, 1, 1).unwrap())
                .country_id(1)
                .attributes(PersonAttributes::default())
                .skills(PlayerSkills::flat_for_ability(ability))
                .positions(PlayerPositions {
                    positions: vec![PlayerPosition {
                        position,
                        level: 18,
                    }],
                })
                .player_attributes(attrs)
                .contract(Some(PlayerClubContract::new(
                    20_000,
                    NaiveDate::from_ymd_opt(2029, 6, 30).unwrap(),
                )))
                .build()
                .unwrap()
        }

        /// 22 seniors at observable ~120 — the main GK slots sit far above
        /// any promotion bar the candidate could clear, so only the overage
        /// path can move him.
        fn main_roster() -> Vec<Player> {
            let mut players = Vec::new();
            let mut id = 100u32;
            let push =
                |pos: PlayerPositionType, n: usize, id: &mut u32, out: &mut Vec<Player>| {
                    for _ in 0..n {
                        out.push(Self::player(*id, pos, 120, 27));
                        *id += 1;
                    }
                };
            push(PlayerPositionType::Goalkeeper, 2, &mut id, &mut players);
            push(PlayerPositionType::DefenderCenter, 8, &mut id, &mut players);
            push(PlayerPositionType::MidfielderCenter, 6, &mut id, &mut players);
            push(PlayerPositionType::Striker, 6, &mut id, &mut players);
            players
        }

        /// U20 squad: the overage keeper plus eleven age-appropriate fillers.
        fn u20_roster(candidate: Player) -> Vec<Player> {
            let mut players = vec![candidate];
            let mut id = 300u32;
            for _ in 0..4 {
                players.push(Self::player(id, PlayerPositionType::DefenderCenter, 50, 18));
                id += 1;
            }
            for _ in 0..4 {
                players.push(Self::player(id, PlayerPositionType::MidfielderCenter, 50, 18));
                id += 1;
            }
            for _ in 0..3 {
                players.push(Self::player(id, PlayerPositionType::Striker, 50, 18));
                id += 1;
            }
            players
        }

        /// Eleven senior reserves so the Second team is a valid demotion
        /// target (its own size never blocks an incoming move).
        fn second_roster() -> Vec<Player> {
            let mut players = Vec::new();
            let mut id = 500u32;
            for _ in 0..11 {
                players.push(Self::player(id, PlayerPositionType::DefenderCenter, 70, 24));
                id += 1;
            }
            players
        }

        fn team(id: u32, slug: &str, tt: TeamType, players: Vec<Player>) -> crate::Team {
            TeamBuilder::new()
                .id(id)
                .league_id(if tt == TeamType::U20 { None } else { Some(1) })
                .club_id(100)
                .name(slug.to_string())
                .slug(slug.to_string())
                .team_type(tt)
                .players(PlayerCollection::new(players))
                .staffs(StaffCollection::new(Vec::new()))
                .reputation(TeamReputation::new(400, 400, 400))
                .training_schedule(Self::schedule())
                .build()
                .unwrap()
        }

        fn club(candidate: Player, with_second: bool) -> Club {
            let mut teams = vec![
                Self::team(10, "main", TeamType::Main, Self::main_roster()),
                Self::team(20, "u20", TeamType::U20, Self::u20_roster(candidate)),
            ];
            if with_second {
                teams.push(Self::team(80, "second", TeamType::Second, Self::second_roster()));
            }
            Club::new(
                100,
                "Club".to_string(),
                Location::new(1),
                ClubFinances::new(10_000_000, Vec::new()),
                ClubAcademy::new(3),
                ClubStatus::Professional,
                ClubColors::default(),
                TeamCollection::new(teams),
                ClubFacilities::default(),
            )
        }

        fn team_idx_of(club: &Club, tt: TeamType) -> Option<usize> {
            club.teams.teams.iter().position(|t| t.team_type == tt)
        }

        fn on_team(club: &Club, tt: TeamType, id: u32) -> bool {
            Self::team_idx_of(club, tt)
                .map(|idx| club.teams.teams[idx].players.players.iter().any(|p| p.id == id))
                .unwrap_or(false)
        }
    }

    /// The reported bug: a modest keeper too old for a talent promotion but
    /// past the U20 age cap must not rot in the youth squad forever. He
    /// graduates out — to the senior reserve when one exists.
    #[test]
    fn overage_keeper_graduates_out_of_u20_to_senior_reserve() {
        let candidate = Fx::player(1, PlayerPositionType::Goalkeeper, 50, 25);
        let mut club = Fx::club(candidate, /* with_second */ true);

        club.rebalance_squads(Fx::date());

        assert!(
            !Fx::on_team(&club, TeamType::U20, 1),
            "an overage keeper must not stay parked in the U20 squad"
        );
        assert!(
            Fx::on_team(&club, TeamType::Second, 1),
            "he graduates to the senior reserve where he plays competitive football"
        );
    }

    /// With no senior reserve, the overage player still leaves the youth
    /// squad — onto the main bench, where the surplus/loan machinery owns him.
    #[test]
    fn overage_keeper_leaves_u20_even_without_a_reserve() {
        let candidate = Fx::player(1, PlayerPositionType::Goalkeeper, 50, 25);
        let mut club = Fx::club(candidate, /* with_second */ false);

        club.rebalance_squads(Fx::date());

        assert!(
            !Fx::on_team(&club, TeamType::U20, 1),
            "with no reserve he still must not be stuck in the U20 squad"
        );
        assert!(
            Fx::on_team(&club, TeamType::Main, 1),
            "absent a reserve, he lands on the main roster for the surplus pass to route"
        );
    }
}

#[cfg(test)]
mod youth_contract_review_tests {
    use super::*;
    use crate::academy::ClubAcademy;
    use crate::club::player::core::builder::PlayerBuilder;
    use crate::shared::Location;
    use crate::shared::fullname::FullName;
    use crate::{
        ClubColors, ClubFacilities, ClubFinances, ClubStatus, HappinessEventType, PersonAttributes,
        PlayerAttributes, PlayerCollection, PlayerPosition, PlayerPositionType, PlayerPositions,
        PlayerSkills, StaffCollection, TeamBuilder, TeamCollection, TeamReputation,
        TrainingSchedule,
    };
    use chrono::{Datelike, NaiveDate, NaiveTime};

    struct Fx;

    impl Fx {
        fn date() -> NaiveDate {
            NaiveDate::from_ymd_opt(2026, 4, 1).unwrap()
        }

        fn schedule() -> TrainingSchedule {
            TrainingSchedule::new(
                NaiveTime::from_hms_opt(9, 0, 0).unwrap(),
                NaiveTime::from_hms_opt(15, 0, 0).unwrap(),
            )
        }

        /// A youth-contract central midfielder with `games` starts each
        /// rated `rating`, so `average_rating_realistic` is deterministic.
        /// `record_match_rating` builds the minutes-weighted ledger;
        /// `played` is set separately because the ledger doesn't count
        /// appearances.
        fn youth(id: u32, age: u8, games: u16, rating: f32) -> Player {
            let mut attrs = PlayerAttributes::default();
            attrs.current_ability = 90;
            attrs.potential_ability = 140;
            attrs.condition = 10_000;
            let contract = PlayerClubContract::new_youth(
                10_000,
                NaiveDate::from_ymd_opt(2029, 6, 30).unwrap(),
            );
            let mut p = PlayerBuilder::new()
                .id(id)
                .full_name(FullName::new("Y".into(), format!("P{id}")))
                .birth_date(
                    NaiveDate::from_ymd_opt(Self::date().year() - age as i32, 1, 1).unwrap(),
                )
                .country_id(1)
                .attributes(PersonAttributes::default())
                .skills(PlayerSkills::default())
                .positions(PlayerPositions {
                    positions: vec![PlayerPosition {
                        position: PlayerPositionType::MidfielderCenter,
                        level: 18,
                    }],
                })
                .player_attributes(attrs)
                .contract(Some(contract))
                .build()
                .unwrap();
            p.statistics.played = games;
            for _ in 0..games {
                p.statistics.record_match_rating(rating, 90, true);
            }
            p
        }

        /// Club with an (empty) Main team — only needed so `main_index`
        /// resolves and the salary scale has a reputation — plus a U19
        /// squad holding the youth players under test.
        fn club(youth: Vec<Player>) -> Club {
            let main = TeamBuilder::new()
                .id(10)
                .league_id(Some(1))
                .club_id(100)
                .name("Main".into())
                .slug("main".into())
                .team_type(TeamType::Main)
                .players(PlayerCollection::new(Vec::new()))
                .staffs(StaffCollection::new(Vec::new()))
                .reputation(TeamReputation::new(2_000, 2_000, 2_000))
                .training_schedule(Self::schedule())
                .build()
                .unwrap();
            let u19 = TeamBuilder::new()
                .id(19)
                .league_id(None)
                .club_id(100)
                .name("U19".into())
                .slug("u19".into())
                .team_type(TeamType::U19)
                .players(PlayerCollection::new(youth))
                .staffs(StaffCollection::new(Vec::new()))
                .reputation(TeamReputation::new(800, 800, 800))
                .training_schedule(Self::schedule())
                .build()
                .unwrap();
            Club::new(
                100,
                "Club".into(),
                Location::new(1),
                ClubFinances::new(10_000_000, Vec::new()),
                ClubAcademy::new(3),
                ClubStatus::Professional,
                ClubColors::default(),
                TeamCollection::new(vec![main, u19]),
                ClubFacilities::default(),
            )
        }

        fn contract_type(club: &Club, id: u32) -> ContractType {
            club.teams
                .teams
                .iter()
                .flat_map(|t| t.players.players.iter())
                .find(|p| p.id == id)
                .and_then(|p| p.contract.as_ref())
                .map(|c| c.contract_type.clone())
                .expect("player must exist with a contract")
        }
    }

    /// A youth player with a real run of strong games earns a full
    /// professional contract — in place, still on the U19 roster.
    #[test]
    fn good_form_youth_earns_pro_contract() {
        let mut club = Fx::club(vec![Fx::youth(1, 17, 12, 7.8)]);
        club.review_youth_contracts(Fx::date());

        assert_eq!(
            Fx::contract_type(&club, 1),
            ContractType::FullTime,
            "a youth player on strong form must be upgraded to a full contract"
        );
        // Upgraded in place — not moved to the main team.
        assert!(
            club.teams.teams[1]
                .players
                .players
                .iter()
                .any(|p| p.id == 1),
            "the upgraded player stays in his youth squad"
        );
        // The milestone fires a positive contract event.
        let fired = club.teams.teams[1]
            .players
            .players
            .iter()
            .find(|p| p.id == 1)
            .unwrap()
            .happiness
            .recent_events
            .iter()
            .any(|e| e.event_type == HappinessEventType::ContractRenewal);
        assert!(fired, "earning pro terms must register a contract event");
    }

    /// Strong ratings but too small a sample: no upgrade yet.
    #[test]
    fn insufficient_games_stays_youth() {
        let mut club = Fx::club(vec![Fx::youth(2, 17, 4, 8.5)]);
        club.review_youth_contracts(Fx::date());
        assert_eq!(
            Fx::contract_type(&club, 2),
            ContractType::Youth,
            "a tiny sample must not trigger a pro contract, however high the rating"
        );
    }

    /// A full season of merely average ratings doesn't earn pro terms.
    #[test]
    fn mediocre_form_stays_youth() {
        let mut club = Fx::club(vec![Fx::youth(3, 18, 15, 6.7)]);
        club.review_youth_contracts(Fx::date());
        assert_eq!(
            Fx::contract_type(&club, 3),
            ContractType::Youth,
            "around-average form must not earn a professional contract"
        );
    }

    /// Youth-league fixtures are friendly-flagged and book into the
    /// friendly bucket — a standout academy-league season must count as
    /// merit evidence even with zero official (senior) minutes.
    #[test]
    fn youth_league_form_earns_pro_contract() {
        let mut prospect = Fx::youth(9, 17, 0, 0.0);
        prospect.friendly_statistics.played = 12;
        for _ in 0..12 {
            prospect.friendly_statistics.record_match_rating(7.8, 90, true);
        }
        let mut club = Fx::club(vec![prospect]);
        club.review_youth_contracts(Fx::date());
        assert_eq!(
            Fx::contract_type(&club, 9),
            ContractType::FullTime,
            "academy-league form must earn a professional contract"
        );
    }

    /// Even outstanding form is gated by the real-world minimum age for
    /// signing professional terms.
    #[test]
    fn below_min_age_stays_youth() {
        let mut club = Fx::club(vec![Fx::youth(4, 15, 12, 7.8)]);
        club.review_youth_contracts(Fx::date());
        assert_eq!(
            Fx::contract_type(&club, 4),
            ContractType::Youth,
            "a player below the professional-contract age floor must stay on youth terms"
        );
    }
}
