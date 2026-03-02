use chrono::{Datelike, NaiveDate};
use log::{debug, info};
use super::CountryResult;
use crate::{ClubResult, Country};
use crate::simulator::SimulatorData;

impl CountryResult {
    pub(super) fn process_end_of_period(
        data: &mut SimulatorData,
        country_id: u32,
        date: NaiveDate,
        club_results: &[ClubResult],
    ) {
        if date.month() == 5 && date.day() == 31 {
            info!("End of season processing");

            if let Some(country) = data.country_mut(country_id) {
                Self::process_season_awards(country, club_results);
                // NOTE: loan returns are handled in a separate phase (process_loan_returns)
                // that runs AFTER club results, so ClubResult references remain valid
                Self::process_player_retirements(country, date);
            }
        }

        if date.month() == 7 && date.day() == 1 {
            if let Some(country) = data.country_mut(country_id) {
                Self::process_promotion_relegation(country);
            }
        }

        if date.month() == 12 && date.day() == 31 {
            if let Some(country) = data.country_mut(country_id) {
                Self::process_year_end_finances(country);
            }
        }
    }

    fn process_season_awards(_country: &mut Country, _club_results: &[ClubResult]) {
        debug!("Processing season awards");
    }

    /// Process loan returns — must run AFTER club_result.process() so that
    /// ClubResult player references remain valid during contract processing.
    pub(super) fn process_loan_returns(
        data: &mut SimulatorData,
        country_id: u32,
        date: NaiveDate,
    ) {
        if !(date.month() == 5 && date.day() == 31) {
            return;
        }

        if let Some(country) = data.country_mut(country_id) {
            Self::process_contract_expirations(country);
        }
    }

    fn process_contract_expirations(country: &mut Country) {
        debug!("Processing contract expirations");

        // Phase 1: Collect expired loan returns (player_id, from_club_idx, to_club_id)
        let mut loan_returns: Vec<(u32, usize, u32)> = Vec::new();

        for (club_idx, club) in country.clubs.iter().enumerate() {
            for team in &club.teams.teams {
                for player in &team.players.players {
                    if let Some(ref contract) = player.contract {
                        if contract.contract_type == crate::ContractType::Loan {
                            if let Some(parent_club_id) = contract.loan_from_club_id {
                                loan_returns.push((player.id, club_idx, parent_club_id));
                            }
                        }
                    }
                }
            }
        }

        // Phase 2: Execute loan returns — only if parent club exists in this country
        for (player_id, borrowing_club_idx, parent_club_id) in loan_returns {
            // Verify parent club exists in this country before removing the player
            let parent_exists = country.clubs.iter().any(|c| c.id == parent_club_id)
                && country.clubs.iter().any(|c| c.id == parent_club_id && !c.teams.teams.is_empty());

            if !parent_exists {
                info!("Loan return skipped: parent club {} not found in country for player {}", parent_club_id, player_id);
                continue;
            }

            // Take player from borrowing club
            let mut player_opt = None;
            for team in &mut country.clubs[borrowing_club_idx].teams.teams {
                if let Some(p) = team.players.take_player(&player_id) {
                    player_opt = Some(p);
                    break;
                }
            }

            if let Some(mut player) = player_opt {
                // Clear loan contract — parent club's original contract was lost during
                // loan creation, so set no contract; the renewal system will offer a new one
                player.contract = None;
                player.statistics = crate::PlayerStatistics::default();
                player.happiness = crate::PlayerHappiness::new();
                player.statuses.statuses.clear();

                // Return to parent club's first team
                if let Some(parent_club) = country.clubs.iter_mut().find(|c| c.id == parent_club_id) {
                    if !parent_club.teams.teams.is_empty() {
                        info!("Loan return: player {} returns to club {}", player_id, parent_club_id);
                        parent_club.teams.teams[0].players.add(player);
                    }
                }
            }
        }
    }

    fn process_player_retirements(_country: &mut Country, _date: NaiveDate) {
        debug!("Processing player retirements");
    }

    fn process_year_end_finances(_country: &mut Country) {
        debug!("Processing year-end finances");
    }

    fn process_promotion_relegation(country: &mut Country) {
        // Collect league info: (league_id, tier, relegation_spots, promotion_spots)
        let league_info: Vec<(u32, u8, u8, u8)> = country
            .leagues
            .leagues
            .iter()
            .map(|l| (l.id, l.settings.tier, l.settings.relegation_spots, l.settings.promotion_spots))
            .collect();

        // For each league with relegation_spots > 0, find its paired league
        for &(tier1_id, tier1_tier, relegation_spots, _) in &league_info {
            if relegation_spots == 0 || tier1_tier == 0 {
                continue;
            }

            // Find paired league: same country, next tier, with promotion_spots > 0
            let paired = league_info.iter().find(|&&(id, tier, _, promo)| {
                id != tier1_id && tier == tier1_tier + 1 && promo > 0
            });

            let &(tier2_id, _, _, promotion_spots) = match paired {
                Some(p) => p,
                None => continue,
            };

            let swap_count = relegation_spots.min(promotion_spots) as usize;

            // Read final tables
            let relegated_team_ids: Vec<u32> = country
                .leagues
                .leagues
                .iter()
                .find(|l| l.id == tier1_id)
                .and_then(|l| l.final_table.as_ref())
                .map(|table| {
                    table.iter().rev().take(swap_count).map(|r| r.team_id).collect()
                })
                .unwrap_or_default();

            let promoted_team_ids: Vec<u32> = country
                .leagues
                .leagues
                .iter()
                .find(|l| l.id == tier2_id)
                .and_then(|l| l.final_table.as_ref())
                .map(|table| {
                    table.iter().take(swap_count).map(|r| r.team_id).collect()
                })
                .unwrap_or_default();

            if relegated_team_ids.is_empty() || promoted_team_ids.is_empty() {
                continue;
            }

            // Swap league_ids on the teams
            for club in &mut country.clubs {
                for team in &mut club.teams.teams {
                    if relegated_team_ids.contains(&team.id) {
                        info!("⬇️ Relegation: team {} ({}) moves to league {}",
                              team.name, team.id, tier2_id);
                        team.league_id = Some(tier2_id);
                    } else if promoted_team_ids.contains(&team.id) {
                        info!("⬆️ Promotion: team {} ({}) moves to league {}",
                              team.name, team.id, tier1_id);
                        team.league_id = Some(tier1_id);
                    }
                }
            }
        }

        // Clear final tables after processing
        for league in &mut country.leagues.leagues {
            league.final_table = None;
        }
    }
}
