use crate::{Person, PlayerClubContract, TeamType};
use chrono::{Datelike, NaiveDate};
use log::debug;
use super::Club;

/// Minimum players a youth/reserve team should keep to remain functional.
const MIN_YOUTH_SQUAD: usize = 11;

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
        let main_idx = match self.teams.teams.iter().position(|t| t.team_type == TeamType::Main) {
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

        // Ability floor of the main team — average of bottom 3 players.
        // Youth players above this level belong in the first team.
        let main_ability_floor = {
            let mut abilities: Vec<u8> = self.teams.teams[main_idx]
                .players.players.iter()
                .map(|p| p.player_attributes.current_ability)
                .collect();
            abilities.sort_unstable();
            let n = abilities.len().min(3).max(1);
            (abilities[..n].iter().map(|&a| a as u16).sum::<u16>() / n as u16) as u8
        };

        let mut moves: Vec<PendingMove> = Vec::new();

        for (ti, team) in self.teams.teams.iter().enumerate() {
            if ti == main_idx || team.team_type == TeamType::Main {
                continue;
            }

            let max_age = team.team_type.max_age();

            for p in &team.players.players {
                let age = p.age(date);
                let ca = p.player_attributes.current_ability;
                let overage = max_age.map_or(false, |limit| age > limit);

                // Never promote players marked for departure
                let statuses = p.statuses.get();
                let listed = statuses.contains(&crate::PlayerStatusType::Lst)
                    || statuses.contains(&crate::PlayerStatusType::Loa);

                // High ability → promote straight to main team
                if ca >= main_ability_floor && !listed {
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

            // Don't drain youth/reserve teams below minimum viable squad,
            // unless the player is overage (must leave regardless)
            if m.reason != "overage for current team"
                && source_size.saturating_sub(already_taken) <= MIN_YOUTH_SQUAD
            {
                continue;
            }

            if let Some(mut player) = self.teams.teams[m.from].players.take_player(&m.player_id) {
                // Upgrade youth contract to full when promoting to main
                if m.to == main_idx {
                    upgrade_contract_if_youth(&mut player, date, &self.teams.teams[main_idx]);
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
        let min_squad = 22usize;

        if main_count < min_squad {
            let deficit = min_squad - main_count;
            let mut candidates: Vec<(usize, u32, u8)> = Vec::new();

            for (ti, team) in self.teams.teams.iter().enumerate() {
                if ti == main_idx || team.team_type == TeamType::Main {
                    continue;
                }
                let available = team.players.players.len().saturating_sub(taken[ti]);
                if available <= MIN_YOUTH_SQUAD && team.team_type.max_age().is_some() {
                    continue;
                }
                for p in &team.players.players {
                    let st = p.statuses.get();
                    if st.contains(&crate::PlayerStatusType::Lst) || st.contains(&crate::PlayerStatusType::Loa) {
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
                if let Some(idx) = self.teams.teams.iter().position(|t| t.team_type == *next_type) {
                    return Some(idx);
                }
            }
        }

        None
    }

    /// Move players without a contract (loan returnees) from main team to reserve.
    /// Loan returns land on teams[0] (main) — staff then moves them to reserve for assessment.
    pub(super) fn move_loan_returns_to_reserve(&mut self, _date: NaiveDate) {
        let main_idx = match self.teams.teams.iter().position(|t| t.team_type == TeamType::Main) {
            Some(idx) => idx,
            None => return,
        };

        let reserve_idx = self.teams.teams.iter()
            .position(|t| t.team_type == TeamType::Reserve)
            .or_else(|| self.teams.teams.iter().position(|t| t.team_type == TeamType::B));

        let reserve_idx = match reserve_idx {
            Some(idx) => idx,
            None => return, // no reserve team, stay on main
        };

        // Find main team players with no contract (returned from loan)
        let to_move: Vec<u32> = self.teams.teams[main_idx].players.players.iter()
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

    /// Release excess players at over-represented positions across all teams.
    pub(super) fn trim_positional_surplus(&mut self, _date: NaiveDate) {
        use crate::PlayerFieldPositionGroup;

        // Positional limits across ALL teams combined
        let limits: [(PlayerFieldPositionGroup, usize); 4] = [
            (PlayerFieldPositionGroup::Goalkeeper, 4),
            (PlayerFieldPositionGroup::Defender, 20),
            (PlayerFieldPositionGroup::Midfielder, 20),
            (PlayerFieldPositionGroup::Forward, 16),
        ];

        for (group, max_count) in &limits {
            // Collect all players at this position across all teams
            let mut players_at_pos: Vec<(usize, u32, u8)> = Vec::new(); // (team_idx, player_id, ability)
            for (ti, team) in self.teams.teams.iter().enumerate() {
                for p in &team.players.players {
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
                // Don't drain youth teams below minimum viable squad
                if self.teams.teams[team_idx].team_type.max_age().is_some()
                    && self.teams.teams[team_idx].players.players.len() <= MIN_YOUTH_SQUAD
                {
                    continue;
                }
                if let Some(player) = self.teams.teams[team_idx].players.take_player(&player_id) {
                    log::debug!(
                        "positional surplus release: {} ({:?}, CA={}) from {}",
                        player.full_name, group, player.player_attributes.current_ability,
                        self.teams.teams[team_idx].name
                    );
                    // Player is simply removed — becomes a free agent
                    drop(player);
                }
            }
        }
    }
}

/// Upgrade a youth contract to a full contract when a player is promoted to the main team.
fn upgrade_contract_if_youth(
    player: &mut crate::Player,
    date: NaiveDate,
    main_team: &crate::Team,
) {
    let is_youth = player.contract.as_ref()
        .map(|c| c.contract_type == crate::ContractType::Youth)
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
