use crate::{
    ContractType, Person, Player, PlayerClubContract, PlayerFieldPositionGroup, PlayerStatusType,
    Team, TeamType,
};
use chrono::{Datelike, NaiveDate};
use log::debug;
use super::Club;

/// Minimum players a youth/reserve team should keep to remain functional.
const MIN_YOUTH_SQUAD: usize = 11;
/// Minimum players the main team should keep before allowing demotions.
const MIN_MAIN_SQUAD: usize = 22;

impl Club {
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
        // global "bottom-3 CA" floor across all positions caused a keeper
        // ping-pong: a youth GK at CA 82 cleared the global floor (~75)
        // even when the main team already had three senior keepers at
        // 100+, so the depth-cap demoted a keeper every pass and another
        // youth GK got promoted the next week. The position-aware floor
        // below is the real signal — "does this youth displace an actual
        // peer at the same role?" — and it resolves the churn without
        // special-casing the goalkeeper position. Also enforces a minimum
        // depth per group: if main has fewer than `MIN_MAIN_DEPTH` at a
        // position (retirement, transfer, release), any youth above
        // `DEPTH_GAP_FLOOR` is eligible to plug the gap.
        const MIN_MAIN_DEPTH: &[(PlayerFieldPositionGroup, usize)] = &[
            (PlayerFieldPositionGroup::Goalkeeper, 2),
            (PlayerFieldPositionGroup::Defender, 6),
            (PlayerFieldPositionGroup::Midfielder, 6),
            (PlayerFieldPositionGroup::Forward, 4),
        ];
        const DEPTH_GAP_FLOOR: u8 = 60;

        let group_stats = |group: PlayerFieldPositionGroup| -> (usize, u8) {
            let cas: Vec<u8> = self.teams.teams[main_idx]
                .players
                .iter()
                .filter(|p| p.position().position_group() == group)
                .map(|p| p.player_attributes.current_ability)
                .collect();
            let worst = cas.iter().copied().min().unwrap_or(0);
            (cas.len(), worst)
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

        for (ti, team) in self.teams.iter().enumerate() {
            if ti == main_idx || team.team_type == TeamType::Main {
                continue;
            }

            let max_age = team.team_type.max_age();

            for p in team.players.iter() {
                let age = p.age(date);
                let ca = p.player_attributes.current_ability;
                let overage = max_age.map_or(false, |limit| age > limit);

                // Never promote players marked for departure
                let statuses = p.statuses.get();
                let listed = statuses.contains(&PlayerStatusType::Lst)
                    || statuses.contains(&PlayerStatusType::Loa);

                let floor = promotion_threshold(p.position().position_group());
                if ca >= floor && !listed {
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
                        next.unwrap_or(main_idx)
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
            let mut ranked: Vec<(u32, u8, u8, bool)> = self.teams.teams[main_idx]
                .players
                .iter()
                .filter(|p| p.position().position_group() == *group)
                .map(|p| (p.id, p.player_attributes.current_ability, p.age(date), p.is_on_loan()))
                .collect();
            ranked.sort_by(|a, b| b.1.cmp(&a.1));
            for (player_id, _, age, is_loan_in) in ranked.into_iter().skip(*depth) {
                if is_loan_in {
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
                if !p.statuses.get().contains(&PlayerStatusType::Loa) {
                    p.statuses.add(date, PlayerStatusType::Loa);
                }
                // No reserve/youth to demote to AND the player is too
                // old to loan? Tag for transfer instead so the surplus
                // can actually leave the club. Without this, single-
                // team clubs accumulate veterans indefinitely (see
                // Gzira: 4× 33-35 GKs sitting on the main roster
                // because they can't loan and can't demote).
                if demotion_target.is_none() && age >= 30 {
                    if !p.statuses.get().contains(&PlayerStatusType::Lst) {
                        p.statuses.add(date, PlayerStatusType::Lst);
                    }
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
            let min_for_source = if m.from == main_idx { MIN_MAIN_SQUAD } else { MIN_YOUTH_SQUAD };
            if m.reason != "overage for current team"
                && m.reason != "surplus at position"
                && source_size.saturating_sub(already_taken) <= min_for_source
            {
                continue;
            }

            if let Some(mut player) = self.teams.teams[m.from].players.take_player(&m.player_id) {
                // Upgrade youth contract to full when promoting to main
                if m.to == main_idx {
                    upgrade_contract_if_youth(&mut player, date, &self.teams.teams[main_idx]);
                    // Career-defining promotion to senior football. Long
                    // cooldown (effectively one-shot per spell) keeps the
                    // event scarce — a player who yo-yos between reserve
                    // and main shouldn't get a fresh "breakthrough" each
                    // bounce.
                    player.on_youth_breakthrough(date);
                }

                debug!("squad rebalance: {} (CA={}, age={}) {} → {} ({})",
                    player.full_name,
                    player.player_attributes.current_ability,
                    player.age(date),
                    self.teams.teams[m.from].name,
                    self.teams.teams[m.to].name,
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
                    let st = p.statuses.get();
                    if st.contains(&PlayerStatusType::Lst) || st.contains(&PlayerStatusType::Loa) {
                        continue;
                    }
                    candidates.push((ti, p.id, p.player_attributes.current_ability));
                }
            }

            candidates.sort_by(|a, b| b.2.cmp(&a.2));
            candidates.truncate(deficit);

            for (team_idx, player_id, _) in candidates {
                if let Some(mut player) = self.teams.teams[team_idx].players.take_player(&player_id) {
                    upgrade_contract_if_youth(&mut player, date, &self.teams.teams[main_idx]);
                    player.on_youth_breakthrough(date);
                    debug!("backfill to main: {} (CA={}, age={}) from {}",
                        player.full_name, player.player_attributes.current_ability,
                        player.age(date), self.teams.teams[team_idx].name);
                    self.teams.teams[main_idx].players.add(player);
                }
            }
        }
    }

    /// Find the next youth team in progression (U18→U19→U20→U21→U23)
    /// that exists in this club and can accept a player of the given age.
    fn find_next_youth_team(&self, current_type: TeamType, player_age: u8) -> Option<usize> {
        let progression = TeamType::YOUTH_PROGRESSION;

        let current_pos = progression.iter().position(|t| *t == current_type)?;

        for next_type in &progression[current_pos + 1..] {
            let age_ok = match next_type.max_age() {
                Some(max_age) => player_age <= max_age,
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
    pub(super) fn move_loan_returns_to_reserve(&mut self, _date: NaiveDate) {
        let main_idx = match self.teams.main_index() {
            Some(idx) => idx,
            None => return,
        };

        let reserve_idx = self.teams.index_of_type(TeamType::Reserve)
            .or_else(|| self.teams.index_of_type(TeamType::B))
            .or_else(|| self.teams.index_of_type(TeamType::Second));

        let reserve_idx = match reserve_idx {
            Some(idx) => idx,
            None => return, // no reserve team, stay on main
        };

        // Find main team players with no contract (returned from loan)
        let to_move: Vec<u32> = self.teams.teams[main_idx].players.iter()
            .filter(|p| p.contract.is_none())
            .map(|p| p.id)
            .collect();

        for player_id in to_move {
            if let Some(player) = self.teams.teams[main_idx].players.take_player(&player_id) {
                debug!("loan return -> reserve: {} moved to {}",
                    player.full_name, self.teams.teams[reserve_idx].name);
                self.teams.teams[reserve_idx].players.add(player);
            }
        }
    }

    /// Release excess players at over-represented positions. Caps scale
    /// per-team so a club with main + reserve + U18 is allowed the depth a
    /// real club carries; previously a flat cross-club cap deleted
    /// legitimate squad members every season start.
    ///
    /// Released players become free agents on the same roster (contract
    /// cleared, Frt status set) — the country-level free-agent pipeline
    /// then matches them to interested clubs. Crucially the player is
    /// never dropped: the previous version deleted them from the world
    /// entirely, with no transfer record, no free-agent pool entry.
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

        for (group, max_count) in &limits {
            // Active players only: if a player has already been released
            // (no contract, Frt) they shouldn't count against the cap, or
            // we'd re-release them every season until someone signs.
            let mut players_at_pos: Vec<(usize, u32, u8)> = Vec::new();
            for (ti, team) in self.teams.iter().enumerate() {
                for p in team.players.iter() {
                    if p.contract.is_none() || p.is_on_loan() {
                        continue;
                    }
                    if p.position().position_group() == *group {
                        players_at_pos.push((ti, p.id, p.player_attributes.current_ability));
                    }
                }
            }

            if players_at_pos.len() <= *max_count {
                continue;
            }

            // Sort by ability ascending — release the worst first
            players_at_pos.sort_by_key(|&(_, _, ca)| ca);

            let to_release = players_at_pos.len() - max_count;
            for &(team_idx, player_id, _) in players_at_pos.iter().take(to_release) {
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
                    debug!(
                        "positional surplus release: {} ({:?}, CA={}) from {}",
                        player.full_name,
                        group,
                        player.player_attributes.current_ability,
                        team_name
                    );
                    player.contract = None;
                    if !player.statuses.get().contains(&PlayerStatusType::Frt) {
                        player.statuses.add(date, PlayerStatusType::Frt);
                    }
                }
            }
        }
    }
}

/// Upgrade a youth contract to a full contract when a player is promoted to the main team.
fn upgrade_contract_if_youth(
    player: &mut Player,
    date: NaiveDate,
    main_team: &Team,
) {
    let is_youth = player.contract.as_ref()
        .map(|c| c.contract_type == ContractType::Youth)
        .unwrap_or(false);

    if is_youth {
        let expiration = NaiveDate::from_ymd_opt(
            date.year() + 3, date.month(), date.day().min(28),
        ).unwrap_or(date);
        let club_rep = main_team.reputation.world;
        let salary = super::graduation_salary(player.player_attributes.current_ability, club_rep);
        player.contract = Some(PlayerClubContract::new(salary, expiration));
    }
}
