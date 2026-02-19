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
}

impl TeamCollection {
    pub fn new(teams: Vec<Team>) -> Self {
        TeamCollection { teams }
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

    /// Weekly squad composition management: demotions, recalls, and youth promotions
    pub fn manage_squad_composition(&mut self, date: NaiveDate) {
        // Need at least a main team and one other team
        if self.teams.len() < 2 {
            return;
        }

        // Find team indices by type
        let main_idx = match self.teams.iter().position(|t| t.team_type == TeamType::Main) {
            Some(idx) => idx,
            None => return,
        };

        let reserve_idx = self.find_reserve_team_index();
        let youth_idx = self.find_youth_team_index();

        // Phase 1: Demotions (main -> reserves)
        if let Some(res_idx) = reserve_idx {
            let demotions = Self::identify_demotions(&self.teams[main_idx], date);
            // Filter out players who are too old for the target team
            let max_age = self.teams[res_idx].team_type.max_age();
            let demotions: Vec<u32> = if let Some(max) = max_age {
                demotions.into_iter().filter(|&pid| {
                    self.teams[main_idx].players.players.iter()
                        .find(|p| p.id == pid)
                        .map(|p| DateUtils::age(p.birth_date, date) <= max)
                        .unwrap_or(false)
                }).collect()
            } else {
                demotions
            };
            if !demotions.is_empty() {
                debug!(
                    "Squad management: demoting {} players to reserves",
                    demotions.len()
                );
                Self::execute_moves(&mut self.teams, main_idx, res_idx, &demotions);
            }
        }

        // Phase 2: Recalls (reserves -> main)
        if let Some(res_idx) = reserve_idx {
            let recalls = Self::identify_recalls(
                &self.teams[main_idx],
                &self.teams[res_idx],
                date,
            );
            if !recalls.is_empty() {
                debug!(
                    "Squad management: recalling {} players from reserves",
                    recalls.len()
                );
                Self::execute_moves(&mut self.teams, res_idx, main_idx, &recalls);
            }
        }

        // Phase 3: Youth promotions (youth -> main, only if still short)
        if let Some(y_idx) = youth_idx {
            let promotions = Self::identify_youth_promotions(
                &self.teams[main_idx],
                &self.teams[y_idx],
                date,
            );
            if !promotions.is_empty() {
                debug!(
                    "Squad management: promoting {} youth players",
                    promotions.len()
                );
                Self::execute_moves(&mut self.teams, y_idx, main_idx, &promotions);
            }
        }
    }

    /// Find the best reserve team: B > U23 > U21 > U19 > U18
    fn find_reserve_team_index(&self) -> Option<usize> {
        self.teams
            .iter()
            .position(|t| t.team_type == TeamType::B)
            .or_else(|| {
                self.teams
                    .iter()
                    .position(|t| t.team_type == TeamType::U23)
            })
            .or_else(|| {
                self.teams
                    .iter()
                    .position(|t| t.team_type == TeamType::U21)
            })
            .or_else(|| {
                self.teams
                    .iter()
                    .position(|t| t.team_type == TeamType::U19)
            })
            .or_else(|| {
                self.teams
                    .iter()
                    .position(|t| t.team_type == TeamType::U18)
            })
    }

    /// Find the best youth team: U18 > U19
    fn find_youth_team_index(&self) -> Option<usize> {
        self.teams
            .iter()
            .position(|t| t.team_type == TeamType::U18)
            .or_else(|| {
                self.teams
                    .iter()
                    .position(|t| t.team_type == TeamType::U19)
            })
    }

    /// Identify players that should be demoted from first team to reserves
    fn identify_demotions(main_team: &Team, _date: NaiveDate) -> Vec<u32> {
        let players = &main_team.players.players;
        let squad_size = players.len();
        let mut demotions = Vec::new();

        if players.is_empty() {
            return demotions;
        }

        let avg_ability: f32 = players
            .iter()
            .map(|p| p.player_attributes.current_ability as f32)
            .sum::<f32>()
            / squad_size as f32;

        for player in players {
            let statuses = player.statuses.get();

            // Transfer listed -> reserves
            if statuses.contains(&PlayerStatusType::Lst) {
                demotions.push(player.id);
                continue;
            }

            // Loan available -> reserves
            if statuses.contains(&PlayerStatusType::Loa) {
                demotions.push(player.id);
                continue;
            }

            // NotNeeded squad status -> reserves
            if let Some(ref contract) = player.contract {
                if matches!(contract.squad_status, PlayerSquadStatus::NotNeeded) {
                    demotions.push(player.id);
                    continue;
                }
            }

            // Players significantly below squad average (>15 points below),
            // but only if squad > 20 and position has 2+ other healthy players
            if squad_size > 20 {
                let ability = player.player_attributes.current_ability as f32;
                if ability < avg_ability - 15.0 {
                    let player_group = player.position().position_group();
                    let others_in_position = players
                        .iter()
                        .filter(|p| {
                            p.id != player.id
                                && p.position().position_group() == player_group
                                && !p.player_attributes.is_injured
                        })
                        .count();
                    if others_in_position >= 2 {
                        demotions.push(player.id);
                        continue;
                    }
                }
            }
        }

        // If squad > 25 after existing demotions, demote lowest-ability surplus
        let remaining = squad_size - demotions.len();
        if remaining > 25 {
            let excess = remaining - 25;
            let mut candidates: Vec<_> = players
                .iter()
                .filter(|p| !demotions.contains(&p.id))
                .map(|p| (p.id, p.player_attributes.current_ability))
                .collect();
            candidates.sort_by_key(|&(_, ability)| ability);
            for (id, _) in candidates.into_iter().take(excess) {
                demotions.push(id);
            }
        }

        demotions
    }

    /// Identify reserve players that should be recalled to the first team
    fn identify_recalls(main_team: &Team, reserve_team: &Team, _date: NaiveDate) -> Vec<u32> {
        let main_players = &main_team.players.players;
        let reserve_players = &reserve_team.players.players;
        let mut recalls = Vec::new();

        if reserve_players.is_empty() {
            return recalls;
        }

        // Build sorted list of recall candidates (best ability first)
        // Skip players with Lst/Loa status or Loan contract type
        let mut candidates: Vec<&Player> = reserve_players
            .iter()
            .filter(|p| {
                let statuses = p.statuses.get();
                !statuses.contains(&PlayerStatusType::Lst)
                    && !statuses.contains(&PlayerStatusType::Loa)
                    && !p.player_attributes.is_injured
                    && !matches!(
                        p.contract.as_ref().map(|c| &c.contract_type),
                        Some(ContractType::Loan)
                    )
            })
            .collect();
        candidates.sort_by(|a, b| {
            b.player_attributes
                .current_ability
                .cmp(&a.player_attributes.current_ability)
        });

        // Count available (non-injured) main team players by position group
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

        // Position gap recalls
        let position_needs = [
            (PlayerFieldPositionGroup::Goalkeeper, gk_count, 2usize),
            (PlayerFieldPositionGroup::Defender, def_count, 4),
            (PlayerFieldPositionGroup::Midfielder, mid_count, 4),
            (PlayerFieldPositionGroup::Forward, fwd_count, 2),
        ];

        for (group, count, min) in &position_needs {
            if *count < *min {
                let needed = min - count;
                let mut recalled = 0;
                for candidate in &candidates {
                    if recalled >= needed {
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

        // Squad below 18 -> recall best available reserves by ability
        let current_main_size = main_players.len() + recalls.len();
        if current_main_size < 18 {
            let needed = 18 - current_main_size;
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

        // Available (non-injured) count below 14 -> emergency recalls
        let total_available = available_main.len() + recalls.len();
        if total_available < 14 {
            let needed = 14 - total_available;
            // For emergency, even consider less ideal candidates
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
                b.player_attributes
                    .current_ability
                    .cmp(&a.player_attributes.current_ability)
            });
            for candidate in emergency_candidates.into_iter().take(needed) {
                recalls.push(candidate.id);
            }
        }

        recalls
    }

    /// Identify youth players that should be promoted to the first team
    fn identify_youth_promotions(
        main_team: &Team,
        youth_team: &Team,
        _date: NaiveDate,
    ) -> Vec<u32> {
        let main_size = main_team.players.players.len();
        let mut promotions = Vec::new();

        // Only promote when first team < 18 players
        if main_size >= 18 {
            return promotions;
        }

        let needed = 18 - main_size;

        let avg_ability: f32 = if main_team.players.players.is_empty() {
            50.0
        } else {
            main_team
                .players
                .players
                .iter()
                .map(|p| p.player_attributes.current_ability as f32)
                .sum::<f32>()
                / main_team.players.players.len() as f32
        };

        // Eligible youth: within 10 ability of average, OR have potential > average + 10
        let mut candidates: Vec<&Player> = youth_team
            .players
            .players
            .iter()
            .filter(|p| {
                let ability = p.player_attributes.current_ability as f32;
                let potential = p.player_attributes.potential_ability as f32;
                !p.player_attributes.is_injured
                    && (ability >= avg_ability - 10.0 || potential > avg_ability + 10.0)
            })
            .collect();

        // Best ability first
        candidates.sort_by(|a, b| {
            b.player_attributes
                .current_ability
                .cmp(&a.player_attributes.current_ability)
        });

        for candidate in candidates.into_iter().take(needed) {
            promotions.push(candidate.id);
        }

        promotions
    }

    /// Move players between teams
    fn execute_moves(teams: &mut [Team], from_idx: usize, to_idx: usize, player_ids: &[u32]) {
        for &player_id in player_ids {
            // Take player from source team
            if let Some(player) = teams[from_idx].players.take_player(&player_id) {
                // Remove from source team's transfer list
                teams[from_idx].transfer_list.remove(player_id);
                // Add to destination team
                teams[to_idx].players.add(player);
            }
        }
    }
}
