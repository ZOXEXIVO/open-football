use crate::{Person, PlayerClubContract, TeamType};
use chrono::{Datelike, NaiveDate};
use log::debug;
use super::Club;

impl Club {
    /// Move players who exceed their youth team's max age to the next youth team,
    /// or to the main team if no eligible youth team exists.
    pub(super) fn enforce_youth_team_age_limits(&mut self, date: NaiveDate) {
        let main_idx = match self.teams.teams.iter().position(|t| t.team_type == TeamType::Main) {
            Some(idx) => idx,
            None => return,
        };

        // Collect overage players from all youth teams
        let mut overage: Vec<(usize, u32)> = Vec::new(); // (team_idx, player_id)

        for (ti, team) in self.teams.teams.iter().enumerate() {
            if ti == main_idx {
                continue;
            }
            if let Some(max_age) = team.team_type.max_age() {
                for p in &team.players.players {
                    if p.age(date) > max_age {
                        overage.push((ti, p.id));
                    }
                }
            }
        }

        for (team_idx, player_id) in overage {
            let player_age = self.teams.teams[team_idx].players.players
                .iter()
                .find(|p| p.id == player_id)
                .map(|p| p.age(date))
                .unwrap_or(99);

            let current_team_type = self.teams.teams[team_idx].team_type;

            // Find the next youth team in progression that can accept this player
            let next_youth_idx = self.find_next_youth_team(current_team_type, player_age);

            let target_idx = next_youth_idx.unwrap_or(main_idx);

            if let Some(mut player) = self.teams.teams[team_idx].players.take_player(&player_id) {
                // Give full contract only when promoting to main team
                if target_idx == main_idx {
                    if player.contract.as_ref().map(|c| c.contract_type == crate::ContractType::Youth).unwrap_or(false) {
                        let expiration = NaiveDate::from_ymd_opt(
                            date.year() + 3, date.month(), date.day().min(28),
                        ).unwrap_or(date);
                        let club_rep = self.teams.teams[main_idx].reputation.world;
                        let salary = super::graduation_salary(player.player_attributes.current_ability, club_rep);
                        player.contract = Some(PlayerClubContract::new(salary, expiration));
                    }
                }

                debug!("overage promotion: {} (age {}) from {} -> {}",
                    player.full_name, player.age(date),
                    self.teams.teams[team_idx].name, self.teams.teams[target_idx].name);
                self.teams.teams[target_idx].players.add(player);
            }
        }
    }

    /// Find the next youth team in progression (U18→U19→U20→U21→U23)
    /// that exists in this club and can accept a player of the given age.
    fn find_next_youth_team(&self, current_type: TeamType, player_age: u8) -> Option<usize> {
        let progression = TeamType::YOUTH_PROGRESSION;

        // Find current position in progression
        let current_pos = progression.iter().position(|t| *t == current_type)?;

        // Look through subsequent youth team types
        for next_type in &progression[current_pos + 1..] {
            if let Some(max_age) = next_type.max_age() {
                if player_age <= max_age {
                    // Check if this club has this team type
                    if let Some(idx) = self.teams.teams.iter().position(|t| t.team_type == *next_type) {
                        return Some(idx);
                    }
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

    /// If main team has fewer than 22 players, promote best youth players up.
    pub(super) fn promote_youth_to_main_if_needed(&mut self, date: NaiveDate) {
        let main_idx = match self.teams.teams.iter().position(|t| t.team_type == TeamType::Main) {
            Some(idx) => idx,
            None => return,
        };

        let main_count = self.teams.teams[main_idx].players.players.len();
        let min_squad = 22usize;

        if main_count >= min_squad {
            return;
        }

        let deficit = min_squad - main_count;

        // Collect candidates from youth teams
        // Skip youth teams that would drop below minimum viable squad (11 players)
        let min_youth_squad = 11usize;
        let youth_team_indices: Vec<usize> = self.teams.teams.iter()
            .enumerate()
            .filter(|(i, t)| *i != main_idx && t.team_type != TeamType::Main)
            .map(|(i, _)| i)
            .collect();

        let mut candidates: Vec<(usize, u32, u8, u8)> = Vec::new(); // (team_idx, player_id, ability, age)
        for &ti in &youth_team_indices {
            let team_size = self.teams.teams[ti].players.players.len();
            if team_size <= min_youth_squad && self.teams.teams[ti].team_type.max_age().is_some() {
                continue;
            }
            for p in &self.teams.teams[ti].players.players {
                candidates.push((ti, p.id, p.player_attributes.current_ability, p.age(date)));
            }
        }

        // Sort by ability descending
        candidates.sort_by(|a, b| b.2.cmp(&a.2));
        candidates.truncate(deficit);

        let mut promoted = 0;
        for (team_idx, player_id, _ca, _age) in candidates {
            if let Some(mut player) = self.teams.teams[team_idx].players.take_player(&player_id) {
                // Upgrade to full contract if on youth contract
                if player.contract.as_ref().map(|c| c.contract_type == crate::ContractType::Youth).unwrap_or(false) {
                    let expiration = NaiveDate::from_ymd_opt(
                        date.year() + 3, date.month(), date.day().min(28),
                    ).unwrap_or(date);
                    let club_rep = self.teams.teams[main_idx].reputation.world;
                    let salary = super::graduation_salary(player.player_attributes.current_ability, club_rep);
                    player.contract = Some(PlayerClubContract::new(salary, expiration));
                }

                debug!("promote to main: {} (CA={}, age={}) from {}",
                    player.full_name, player.player_attributes.current_ability,
                    player.age(date), self.teams.teams[team_idx].name);
                self.teams.teams[main_idx].players.add(player);
                promoted += 1;
            }
        }

        if promoted > 0 {
            debug!("{}: promoted {} youth players to main team (now {})",
                self.name, promoted, self.teams.teams[main_idx].players.players.len());
        }
    }
}
