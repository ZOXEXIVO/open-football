use chrono::{Datelike, NaiveDate};
use log::{debug, info};
use super::CountryResult;
use crate::league::Season;
use crate::utils::IntegerUtils;
use crate::{ClubResult, Country, Person, PlayerStatisticsHistoryItem, PlayerStatusType};
use crate::simulator::SimulatorData;
use std::collections::HashMap;

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

        // Monthly check: retire players who are past max retirement age
        // so they don't linger on teams until the next May 31
        if date.day() == 1 && date.month() != 5 {
            if let Some(country) = data.country_mut(country_id) {
                Self::process_overdue_retirements(country, date);
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
            Self::process_contract_expirations(country, date);
        }
    }

    fn process_contract_expirations(country: &mut Country, date: NaiveDate) {
        debug!("Processing contract expirations");

        // Build league lookup for history entries
        let league_lookup: HashMap<u32, (String, String)> = country.leagues.leagues.iter()
            .map(|l| (l.id, (l.name.clone(), l.slug.clone())))
            .collect();

        // Phase 1: Collect expired loan returns (player_id, from_club_idx, to_club_id)
        // Also collect borrowing club info for history snapshots
        struct LoanReturn {
            player_id: u32,
            borrowing_club_idx: usize,
            parent_club_id: u32,
            team_name: String,
            team_slug: String,
            team_reputation: u16,
            league_name: String,
            league_slug: String,
        }

        let mut loan_returns: Vec<LoanReturn> = Vec::new();

        for (club_idx, club) in country.clubs.iter().enumerate() {
            // Get main team info for history (show club name, not sub-team)
            let main_team_info: Option<(String, String, u16)> = club.teams.teams.iter()
                .find(|t| t.team_type == crate::TeamType::Main)
                .map(|t| (t.name.clone(), t.slug.clone(), t.reputation.world));

            let main_team_league = club.teams.teams.iter()
                .find(|t| t.team_type == crate::TeamType::Main)
                .and_then(|t| t.league_id)
                .and_then(|lid| league_lookup.get(&lid))
                .cloned()
                .unwrap_or_default();

            for team in &club.teams.teams {
                let (team_name, team_slug, team_reputation) = match (&team.team_type, &main_team_info) {
                    (crate::TeamType::Main, _) | (_, None) => {
                        (team.name.clone(), team.slug.clone(), team.reputation.world)
                    }
                    (_, Some((name, slug, rep))) => {
                        (name.clone(), slug.clone(), *rep)
                    }
                };

                for player in &team.players.players {
                    if let Some(ref contract) = player.contract {
                        if contract.contract_type == crate::ContractType::Loan {
                            if let Some(parent_club_id) = contract.loan_from_club_id {
                                loan_returns.push(LoanReturn {
                                    player_id: player.id,
                                    borrowing_club_idx: club_idx,
                                    parent_club_id,
                                    team_name: team_name.clone(),
                                    team_slug: team_slug.clone(),
                                    team_reputation,
                                    league_name: main_team_league.0.clone(),
                                    league_slug: main_team_league.1.clone(),
                                });
                            }
                        }
                    }
                }
            }
        }

        // Phase 2: Execute loan returns — only if parent club exists in this country
        for loan_return in loan_returns {
            // Verify parent club exists in this country before removing the player
            let parent_exists = country.clubs.iter().any(|c| c.id == loan_return.parent_club_id)
                && country.clubs.iter().any(|c| c.id == loan_return.parent_club_id && !c.teams.teams.is_empty());

            if !parent_exists {
                info!("Loan return skipped: parent club {} not found in country for player {}", loan_return.parent_club_id, loan_return.player_id);
                continue;
            }

            // Take player from borrowing club
            let mut player_opt = None;
            for team in &mut country.clubs[loan_return.borrowing_club_idx].teams.teams {
                if let Some(p) = team.players.take_player(&loan_return.player_id) {
                    player_opt = Some(p);
                    break;
                }
            }

            if let Some(mut player) = player_opt {
                // Merge remaining stats into the existing loan history entry
                // (season-end snapshot may have already captured them)
                let season = Season::from_date(date);
                let remaining_stats = std::mem::take(&mut player.statistics);

                if let Some(existing) = player.statistics_history.items.iter_mut().find(|e| {
                    e.season.start_year == season.start_year
                        && e.team_slug == loan_return.team_slug
                        && e.is_loan
                }) {
                    // Only overwrite if existing has 0 games and remaining has stats
                    let existing_games = existing.statistics.played + existing.statistics.played_subs;
                    let new_games = remaining_stats.played + remaining_stats.played_subs;
                    if existing_games == 0 && new_games > 0 {
                        existing.statistics = remaining_stats;
                    }
                } else {
                    // No existing entry — create one
                    player.statistics_history.push_or_replace(PlayerStatisticsHistoryItem {
                        season,
                        team_name: loan_return.team_name.clone(),
                        team_slug: loan_return.team_slug.clone(),
                        team_reputation: loan_return.team_reputation,
                        league_name: loan_return.league_name.clone(),
                        league_slug: loan_return.league_slug.clone(),
                        is_loan: true,
                        transfer_fee: None,
                        statistics: remaining_stats,
                        created_at: date,
                    });
                }

                // Clear loan contract — parent club's original contract was lost during
                // loan creation, so set no contract; the renewal system will offer a new one
                player.contract = None;
                player.happiness = crate::PlayerHappiness::new();
                player.statuses.statuses.clear();
                player.last_transfer_date = Some(date);

                // Return to parent club's first team — weekly Club::simulate
                // will move them to reserve via move_loan_returns_to_reserve()
                if let Some(parent_club) = country.clubs.iter_mut().find(|c| c.id == loan_return.parent_club_id) {
                    if !parent_club.teams.teams.is_empty() {
                        info!("Loan return: player {} returns to club {}", loan_return.player_id, loan_return.parent_club_id);
                        parent_club.teams.teams[0].players.add(player);
                    }
                }
            }
        }
    }

    /// Monthly check: retire players who are clearly past max retirement age
    /// or already have the Ret status. Does NOT do probabilistic retirement
    /// (that stays at season end in process_player_retirements).
    fn process_overdue_retirements(country: &mut Country, date: NaiveDate) {
        let mut to_retire: Vec<(usize, usize, u32)> = Vec::new();

        for (club_idx, club) in country.clubs.iter().enumerate() {
            for (team_idx, team) in club.teams.teams.iter().enumerate() {
                for player in &team.players.players {
                    if Self::must_retire(player, date) {
                        to_retire.push((club_idx, team_idx, player.id));
                    }
                }
            }
        }

        for (club_idx, team_idx, player_id) in to_retire {
            if let Some(mut player) = country.clubs[club_idx].teams.teams[team_idx]
                .players.take_player(&player_id)
            {
                info!("Overdue retirement: {} (age {})", player.full_name, player.age(date));
                player.statuses.add(date, PlayerStatusType::Ret);
                player.contract = None;
                country.retired_players.push(player);
            }
        }
    }

    /// Deterministic retirement: only for players past max age or with Ret status.
    fn must_retire(player: &crate::Player, date: NaiveDate) -> bool {
        if player.statuses.get().contains(&PlayerStatusType::Ret) {
            return true;
        }

        let age = player.age(date);
        let ca = player.player_attributes.current_ability;
        let is_gk = player.position().is_goalkeeper();

        let max_retire_age = match ca {
            0..=79 => 35u8,
            80..=119 => 37,
            _ => 38,
        };

        let max_age = if is_gk { max_retire_age + 2 } else { max_retire_age };

        age >= max_age
    }

    fn process_player_retirements(country: &mut Country, date: NaiveDate) {
        debug!("Processing player retirements");

        // Collect players to retire: (club_idx, team_idx, player_id)
        let mut to_retire: Vec<(usize, usize, u32)> = Vec::new();

        for (club_idx, club) in country.clubs.iter().enumerate() {
            for (team_idx, team) in club.teams.teams.iter().enumerate() {
                for player in &team.players.players {
                    if Self::should_retire(player, date) {
                        to_retire.push((club_idx, team_idx, player.id));
                    }
                }
            }
        }

        // Execute retirements: remove from team, add Ret status, store in retired_players
        for (club_idx, team_idx, player_id) in to_retire {
            if let Some(mut player) = country.clubs[club_idx].teams.teams[team_idx]
                .players.take_player(&player_id)
            {
                info!("Player retired: {} (age {})", player.full_name, player.age(date));
                player.statuses.add(date, PlayerStatusType::Ret);
                player.contract = None;
                country.retired_players.push(player);
            }
        }
    }

    fn should_retire(player: &crate::Player, date: NaiveDate) -> bool {
        let age = player.age(date);
        let ca = player.player_attributes.current_ability;

        // Already marked for retirement
        if player.statuses.get().contains(&PlayerStatusType::Ret) {
            return true;
        }

        // Goalkeepers retire later
        let is_gk = player.position().is_goalkeeper();

        // Base retirement age ranges:
        // Low ability (CA < 80): retire 33-35
        // Medium ability (80-120): retire 34-37
        // High ability (120+): retire 35-38
        // Goalkeepers: +2 years
        let (min_retire_age, max_retire_age) = match ca {
            0..=79 => (33u8, 35u8),
            80..=119 => (34, 37),
            _ => (35, 38),
        };

        let (min_age, max_age) = if is_gk {
            (min_retire_age + 2, max_retire_age + 2)
        } else {
            (min_retire_age, max_retire_age)
        };

        if age < min_age {
            return false;
        }

        if age >= max_age {
            return true;
        }

        // Between min and max: probability increases with age
        let range = (max_age - min_age) as i32;
        let years_over = (age - min_age) as i32;
        // Chance: 20% at min_age, increasing to 80% at max_age-1
        let chance = 20 + (years_over * 60 / range);
        IntegerUtils::random(0, 100) < chance
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

            // Swap league_ids on teams and move sub-teams to matching friendly league
            for club in &mut country.clubs {
                let mut new_main_league_id: Option<u32> = None;

                for team in &mut club.teams.teams {
                    if relegated_team_ids.contains(&team.id) {
                        info!("⬇️ Relegation: team {} ({}) moves to league {}",
                              team.name, team.id, tier2_id);
                        team.league_id = Some(tier2_id);
                        new_main_league_id = Some(tier2_id);
                    } else if promoted_team_ids.contains(&team.id) {
                        info!("⬆️ Promotion: team {} ({}) moves to league {}",
                              team.name, team.id, tier1_id);
                        team.league_id = Some(tier1_id);
                        new_main_league_id = Some(tier1_id);
                    }
                }

                // Move sub-teams to the friendly league of the new main league
                if let Some(new_league_id) = new_main_league_id {
                    let friendly_league_id = new_league_id + 200000;
                    for team in &mut club.teams.teams {
                        if team.team_type != crate::TeamType::Main {
                            team.league_id = Some(friendly_league_id);
                        }
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
