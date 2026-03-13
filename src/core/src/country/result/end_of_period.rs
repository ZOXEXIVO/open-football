use chrono::{Datelike, NaiveDate};
use log::{debug, info};
use super::CountryResult;
use crate::utils::IntegerUtils;
use crate::{ClubResult, Country, Person, PlayerStatusType, TeamInfo};
use crate::simulator::SimulatorData;
use std::collections::HashMap;

impl CountryResult {
    pub(super) fn process_end_of_period(
        data: &mut SimulatorData,
        country_id: u32,
        date: NaiveDate,
        club_results: &[ClubResult],
    ) {
        // Get season dates from league settings
        let season = data.country(country_id)
            .map(|c| c.season_dates())
            .unwrap_or_default();

        if season.is_season_end(date) {
            debug!("End of season processing");

            if let Some(country) = data.country_mut(country_id) {
                Self::process_season_awards(country, club_results);
                // NOTE: loan returns are handled in a separate phase (process_loan_returns)
                // that runs AFTER club results, so ClubResult references remain valid
                Self::process_player_retirements(country, date);
            }
        }

        // Monthly check: retire players who are past max retirement age
        // so they don't linger on teams until season end
        if date.day() == 1 && date.month() as u8 != season.end_month {
            if let Some(country) = data.country_mut(country_id) {
                Self::process_overdue_retirements(country, date);
            }
        }

        // Promotion/relegation: runs on the 1st of the month after season end
        let promo_month = if season.end_month == 12 { 1u8 } else { season.end_month + 1 };
        if date.day() == 1 && date.month() as u8 == promo_month {
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
        // Only check on the 1st and last day of each month to avoid daily scans
        if date.day() != 1 && date.day() < 28 {
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

        // Phase 1: Collect expired loan returns
        struct LoanReturn {
            player_id: u32,
            borrowing_club_idx: usize,
            parent_club_id: u32,
            borrowing_info: TeamInfo,
        }

        let mut loan_returns: Vec<LoanReturn> = Vec::new();

        for (club_idx, club) in country.clubs.iter().enumerate() {
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
                    if let Some(ref loan_contract) = player.contract_loan {
                        if loan_contract.expiration <= date {
                            if let Some(parent_club_id) = loan_contract.loan_from_club_id {
                                loan_returns.push(LoanReturn {
                                    player_id: player.id,
                                    borrowing_club_idx: club_idx,
                                    parent_club_id,
                                    borrowing_info: TeamInfo {
                                        name: team_name.clone(),
                                        slug: team_slug.clone(),
                                        reputation: team_reputation,
                                        league_name: main_team_league.0.clone(),
                                        league_slug: main_team_league.1.clone(),
                                    },
                                });
                            }
                        }
                    }
                }
            }
        }

        // Phase 2: Execute loan returns
        for loan_return in loan_returns {
            let parent_exists = country.clubs.iter().any(|c| c.id == loan_return.parent_club_id)
                && country.clubs.iter().any(|c| c.id == loan_return.parent_club_id && !c.teams.teams.is_empty());

            if !parent_exists {
                debug!("Loan return skipped: parent club {} not found in country for player {}", loan_return.parent_club_id, loan_return.player_id);
                continue;
            }

            let mut player_opt = None;
            for team in &mut country.clubs[loan_return.borrowing_club_idx].teams.teams {
                if let Some(p) = team.players.take_player(&loan_return.player_id) {
                    player_opt = Some(p);
                    break;
                }
            }

            if let Some(mut player) = player_opt {
                player.on_loan_return(&loan_return.borrowing_info, date);

                player.contract_loan = None;
                player.happiness = crate::PlayerHappiness::new();
                player.statuses.statuses.clear();

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
                debug!("Overdue retirement: {} (age {})", player.full_name, player.age(date));
                player.statuses.add(date, PlayerStatusType::Ret);
                player.contract = None;
                country.retired_players.push(player);
            }
        }
    }

    /// Deterministic retirement: only for players past absolute max age or with Ret status.
    /// Players still getting games are given a +1 year grace period.
    fn must_retire(player: &crate::Player, date: NaiveDate) -> bool {
        if player.statuses.get().contains(&PlayerStatusType::Ret) {
            return true;
        }

        let age = player.age(date);
        let ca = player.player_attributes.current_ability;
        let is_gk = player.position().is_goalkeeper();

        let max_retire_age = match ca {
            0..=59 => 36u8,
            60..=99 => 37,
            100..=139 => 38,
            140..=169 => 39,
            _ => 40,
        };

        let max_age = if is_gk { max_retire_age + 2 } else { max_retire_age };

        // Players still getting games get a 1-year grace period
        // At season start current stats are 0, so fall back to last season history;
        // if no history exists at all, assume the player is active (benefit of the doubt)
        let has_recent_games = player.statistics.total_games() >= 5
            || player.statistics_history.items
                .last()
                .map(|h| h.statistics.total_games() >= 5)
                .unwrap_or(true);

        let effective_max = if has_recent_games { max_age + 1 } else { max_age };

        age >= effective_max
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
                debug!("Player retired: {} (age {})", player.full_name, player.age(date));
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

        let is_gk = player.position().is_goalkeeper();

        // Base retirement age window by ability
        // Higher ability players have longer careers
        let (min_retire_age, max_retire_age) = match ca {
            0..=59 => (32u8, 35u8),
            60..=99 => (33, 36),
            100..=139 => (34, 37),
            140..=169 => (35, 38),
            _ => (36, 39),
        };

        // Goalkeepers retire ~2 years later
        let (min_age, max_age) = if is_gk {
            (min_retire_age + 2, max_retire_age + 2)
        } else {
            (min_retire_age, max_retire_age)
        };

        // Too young to retire
        if age < min_age {
            return false;
        }

        // Past max age — forced retirement
        if age >= max_age {
            return true;
        }

        // Still playing regularly? Don't retire.
        // Check current season official appearances
        let current_season_games = player.statistics.total_games();
        if current_season_games >= 10 {
            return false;
        }

        // Check last completed season from history
        let last_season_games = player.statistics_history.items
            .last()
            .map(|h| h.statistics.total_games())
            .unwrap_or(0);

        // Players with significant recent game time are much less likely to retire
        if last_season_games >= 15 {
            return false;
        }

        // Base retirement probability: increases with age within the window
        let range = (max_age - min_age).max(1) as i32;
        let years_over = (age - min_age) as i32;
        let mut chance: i32 = 15 + (years_over * 50 / range);

        // Ambitious players want to keep playing
        // ambition is 0.0-20.0
        let ambition = player.attributes.ambition;
        if ambition > 15.0 {
            chance -= 20;
        } else if ambition > 10.0 {
            chance -= 10;
        }

        // High determination makes players persist
        let determination = player.skills.mental.determination;
        if determination > 15.0 {
            chance -= 15;
        } else if determination > 10.0 {
            chance -= 5;
        }

        // Players with no games last season are more likely to retire
        if last_season_games == 0 && current_season_games == 0 {
            chance += 25;
        } else if last_season_games < 5 {
            chance += 10;
        }

        // Declining ability makes retirement more likely
        // If CA is well below PA, player has declined
        let pa = player.player_attributes.potential_ability;
        if pa > 0 && ca < pa / 2 {
            chance += 15;
        }

        chance = chance.clamp(5, 90);
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
