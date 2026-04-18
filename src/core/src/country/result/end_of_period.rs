use chrono::{Datelike, NaiveDate};
use log::{debug, info};
use super::CountryResult;
use crate::utils::{DateUtils, IntegerUtils};
use crate::club::team::reputation::{Achievement, AchievementType};
use crate::{
    ClubResult, Country, Person, Player, PlayerHappiness, PlayerStatusType, TeamInfo, TeamType,
};
use crate::simulator::SimulatorData;
use std::collections::HashMap;

struct LoanReturnEvent {
    player_id: u32,
    borrowing_club_id: u32,
    parent_club_id: u32,
    borrowing_info: TeamInfo,
}

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
                Self::process_season_awards(country, club_results, date);
                // NOTE: loan returns are handled in a separate phase (process_loan_returns)
                // that runs AFTER club results, so ClubResult references remain valid
                Self::process_player_retirements(country, date);
            }
        }

        // Monthly check: retire players who are past max retirement age
        // so they don't linger on teams until season end
        if DateUtils::is_month_beginning(date) && date.month() as u8 != season.end_month {
            if let Some(country) = data.country_mut(country_id) {
                Self::process_overdue_retirements(country, date);
            }
        }

        // Monthly reputation decay — teams that aren't achieving anything
        // drift back toward the mean. Runs on the 1st regardless of season.
        if DateUtils::is_month_beginning(date) {
            if let Some(country) = data.country_mut(country_id) {
                for club in &mut country.clubs {
                    for team in club.teams.iter_mut() {
                        team.on_month_tick();
                    }
                }
            }
        }

        // Promotion/relegation: runs on the 1st of the month AFTER the latest
        // non-friendly league in the country has finished its season. Using
        // the tier-1 end date alone can fire before lower tiers are done,
        // leaving their final_table empty and silently skipping the swap.
        let latest_end_month = data.country(country_id)
            .map(|c| {
                c.leagues.leagues.iter()
                    .filter(|l| !l.friendly)
                    .map(|l| l.settings.season_ending_half.to_month)
                    .max()
                    .unwrap_or(season.end_month)
            })
            .unwrap_or(season.end_month);
        let promo_month = if latest_end_month == 12 { 1u8 } else { latest_end_month + 1 };
        if DateUtils::is_month_beginning(date) && date.month() as u8 == promo_month {
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

    fn process_season_awards(
        country: &mut Country,
        _club_results: &[ClubResult],
        date: NaiveDate,
    ) {
        debug!("Processing season awards");

        // Build team_id -> club index mapping
        let team_to_club: HashMap<u32, usize> = country.clubs.iter()
            .enumerate()
            .flat_map(|(ci, club)| {
                club.teams.iter().map(move |t| (t.id, ci))
            })
            .collect();

        // Trophy reputation boost: league champions and promoted sides get
        // a durable rep bump that lingers for 2 seasons (see Achievement).
        // Collected before the prize-money loop so we don't mix concerns.
        let mut trophy_awards: Vec<(u32, AchievementType)> = Vec::new();
        for league in &country.leagues.leagues {
            if league.friendly {
                continue;
            }
            let table = match &league.final_table {
                Some(t) if !t.is_empty() => t,
                _ => continue,
            };
            if let Some(champion) = table.first() {
                trophy_awards.push((champion.team_id, AchievementType::LeagueTitle));
            }
            let promo_slots = league.settings.promotion_spots as usize;
            if promo_slots > 0 {
                // The top `promo_slots` rows are already the title winners
                // plus those promoted; skip index 0 since that's the title
                // event already recorded.
                for row in table.iter().take(promo_slots).skip(1) {
                    trophy_awards.push((row.team_id, AchievementType::Promotion));
                }
            }
        }
        for (team_id, ach_type) in trophy_awards {
            for club in &mut country.clubs {
                if club.teams.iter().any(|t| t.id == team_id) {
                    // Reputation achievement on the team side; long-term
                    // vision tracker on the board side.
                    if let Some(team) = club.teams.iter_mut().find(|t| t.id == team_id) {
                        team.on_season_trophy(Achievement::new(ach_type.clone(), date, 8));
                    }
                    club.board.on_achievement(ach_type.clone());

                    // Trophy-triggered renewal bump. The board rewards
                    // the manager directly — bigger silverware = bigger
                    // bump, chairman loyalty also lifts. Fires in addition
                    // to the season-start renewal offer so a winning
                    // campaign is recognised even when the contract still
                    // has >18 months left.
                    let (salary_bump_pct, extension_years, loyalty_lift): (f32, i32, u8) =
                        match ach_type {
                            AchievementType::ContinentalTrophy => (0.25, 3, 20),
                            AchievementType::LeagueTitle => (0.20, 2, 15),
                            AchievementType::CupWin => (0.08, 1, 6),
                            AchievementType::Promotion => (0.10, 1, 8),
                            _ => (0.0, 0, 0),
                        };
                    if salary_bump_pct > 0.0 {
                        let cur = club.board.chairman.manager_loyalty as u16;
                        club.board.chairman.manager_loyalty =
                            (cur + loyalty_lift as u16).min(100) as u8;
                        if let Some(main_team) = club.teams.main_mut() {
                            if let Some(mgr) = main_team
                                .staffs
                                .find_mut_by_position(crate::StaffPosition::Manager)
                            {
                                if let Some(contract) = mgr.contract.as_mut() {
                                    contract.salary =
                                        ((contract.salary as f32) * (1.0 + salary_bump_pct)) as u32;
                                    if extension_years > 0 {
                                        let new_exp = contract
                                            .expired
                                            .with_year(contract.expired.year() + extension_years)
                                            .unwrap_or(contract.expired);
                                        if new_exp > contract.expired {
                                            contract.expired = new_exp;
                                        }
                                    }
                                    mgr.job_satisfaction =
                                        (mgr.job_satisfaction + 12.0).clamp(0.0, 100.0);
                                }
                            }
                        }
                    }
                    break;
                }
            }
        }

        // Collect awards per club: (club_idx, prize_money, tv_money)
        let mut club_awards: HashMap<usize, (i64, i64)> = HashMap::new();

        for league in &country.leagues.leagues {
            if league.friendly {
                continue;
            }

            let table = match &league.final_table {
                Some(t) if !t.is_empty() => t,
                _ => continue,
            };

            let total_teams = table.len();
            let prize_pool = league.financials.prize_pool;
            let tv_deal = league.financials.tv_deal_total;

            if prize_pool == 0 && tv_deal == 0 {
                continue;
            }

            // Calculate normalization factor: sum of (1 - i/n)^2
            let total_shares: f64 = (0..total_teams)
                .map(|i| {
                    let r = i as f64 / (total_teams - 1).max(1) as f64;
                    (1.0 - r).powi(2)
                })
                .sum();

            for (position, row) in table.iter().enumerate() {
                let club_idx = match team_to_club.get(&row.team_id) {
                    Some(&idx) => idx,
                    None => continue,
                };

                // Prize money: quadratic decay by position (top-heavy)
                let share = if total_teams > 1 {
                    let pos_ratio = position as f64 / (total_teams - 1) as f64;
                    (1.0 - pos_ratio).powi(2)
                } else {
                    1.0
                };

                let prize_amount = (prize_pool as f64 * share / total_shares) as i64;

                // TV money: 50% equal + 50% merit-based
                let tv_equal = tv_deal / (2 * total_teams as i64);
                let tv_merit = (tv_deal as f64 * 0.5 * share / total_shares) as i64;
                let tv_amount = tv_equal + tv_merit;

                let entry = club_awards.entry(club_idx).or_insert((0, 0));
                entry.0 += prize_amount;
                entry.1 += tv_amount;
            }
        }

        // Apply awards to clubs
        for (club_idx, (prize, tv)) in club_awards {
            let club = &mut country.clubs[club_idx];
            if prize > 0 {
                club.finance.balance.push_income_prize_money(prize);
            }
            if tv > 0 {
                club.finance.balance.push_income_tv(tv);
            }
            debug!("Season awards: {} - prize: ${}, TV: ${}", club.name, prize, tv);
        }
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

        // Phase 1: Scan — collect expired loans as lightweight events
        let events = Self::scan_expired_loans(data, country_id, date);

        // Phase 2: Execute — move players by club ID (country-agnostic)
        for event in events {
            Self::execute_loan_return(data, event, date);
        }
    }

    /// Read-only scan: find players with expired loan contracts in this country.
    fn scan_expired_loans(
        data: &SimulatorData,
        country_id: u32,
        date: NaiveDate,
    ) -> Vec<LoanReturnEvent> {
        let country = match data.country(country_id) {
            Some(c) => c,
            None => return Vec::new(),
        };

        let leagues = &country.leagues.leagues;
        let league_info = |lid: u32| -> (&str, &str) {
            leagues
                .iter()
                .find(|l| l.id == lid)
                .map(|l| (l.name.as_str(), l.slug.as_str()))
                .unwrap_or(("", ""))
        };

        let mut events = Vec::new();

        for club in &country.clubs {
            // Cheap short-circuit: most days most clubs have zero expiring
            // loans, and we'd otherwise clone the main team + league strings
            // unconditionally. Scan first, then allocate.
            let has_expiring = club.teams.iter().any(|team| {
                team.players.iter().any(|player| {
                    player
                        .contract_loan
                        .as_ref()
                        .map(|lc| lc.expiration <= date && lc.loan_from_club_id.is_some())
                        .unwrap_or(false)
                })
            });
            if !has_expiring {
                continue;
            }

            // Build the main-team / league strings once per club, not once
            // per player that needs returning.
            let main_team = club.teams.main();
            let main_name = main_team.map(|t| t.name.clone());
            let main_slug = main_team.map(|t| t.slug.clone());
            let main_reputation = main_team.map(|t| t.reputation.world);
            let (main_league_name, main_league_slug) = main_team
                .and_then(|t| t.league_id)
                .map(|lid| {
                    let (n, s) = league_info(lid);
                    (n.to_owned(), s.to_owned())
                })
                .unwrap_or_default();

            for team in club.teams.iter() {
                let (team_name, team_slug, team_reputation) = if team.team_type == TeamType::Main
                    || main_name.is_none()
                {
                    (team.name.clone(), team.slug.clone(), team.reputation.world)
                } else {
                    (
                        main_name.clone().unwrap_or_default(),
                        main_slug.clone().unwrap_or_default(),
                        main_reputation.unwrap_or(0),
                    )
                };

                for player in team.players.iter() {
                    if let Some(ref loan_contract) = player.contract_loan {
                        if loan_contract.expiration <= date {
                            if let Some(parent_club_id) = loan_contract.loan_from_club_id {
                                events.push(LoanReturnEvent {
                                    player_id: player.id,
                                    borrowing_club_id: club.id,
                                    parent_club_id,
                                    borrowing_info: TeamInfo {
                                        name: team_name.clone(),
                                        slug: team_slug.clone(),
                                        reputation: team_reputation,
                                        league_name: main_league_name.clone(),
                                        league_slug: main_league_slug.clone(),
                                    },
                                });
                            }
                        }
                    }
                }
            }
        }

        events
    }

    /// Execute a single loan return: take player from borrowing club, place at parent club.
    /// Both clubs are resolved globally by ID — works for domestic and cross-country.
    fn execute_loan_return(
        data: &mut SimulatorData,
        event: LoanReturnEvent,
        date: NaiveDate,
    ) {
        // Find parent club location first — abort early if missing
        let parent_pos = data.find_club_main_team(event.parent_club_id);
        if parent_pos.is_none() {
            debug!("Loan return skipped: parent club {} not found for player {}",
                event.parent_club_id, event.player_id);
            return;
        }

        // Build parent TeamInfo before mutating data — needed by record_loan_return
        // to create a current-season entry if one was drained by season-end snapshot.
        let (pci, pcoi, pcli, pti) = parent_pos.unwrap();
        let parent_info = {
            let country = &data.continents[pci].countries[pcoi];
            let club = &country.clubs[pcli];
            let team = &club.teams.teams[pti];
            let league_info = team.league_id
                .and_then(|lid| country.leagues.leagues.iter().find(|l| l.id == lid))
                .map(|l| (l.name.clone(), l.slug.clone()))
                .unwrap_or_default();
            TeamInfo {
                name: team.name.clone(),
                slug: team.slug.clone(),
                reputation: team.reputation.world,
                league_name: league_info.0,
                league_slug: league_info.1,
            }
        };

        // Take player from wherever they are
        let player_pos = data.find_player_position(event.player_id);
        let mut player = match player_pos {
            Some((ci, coi, cli, ti)) => {
                match data.continents[ci].countries[coi].clubs[cli]
                    .teams.teams[ti].players.take_player(&event.player_id)
                {
                    Some(p) => p,
                    None => return,
                }
            }
            None => return,
        };

        player.on_loan_return(&event.borrowing_info, &parent_info, date);
        player.contract_loan = None;
        player.happiness = PlayerHappiness::new();
        player.statuses.statuses.clear();

        // Place at parent club
        info!("Loan return: player {} from club {} back to club {}",
            event.player_id, event.borrowing_club_id, event.parent_club_id);
        data.continents[pci].countries[pcoi].clubs[pcli]
            .teams.teams[pti].players.add(player);
    }

    /// Monthly check: retire players who are clearly past max retirement age
    /// or already have the Ret status. Does NOT do probabilistic retirement
    /// (that stays at season end in process_player_retirements).
    fn process_overdue_retirements(country: &mut Country, date: NaiveDate) {
        let mut to_retire: Vec<(usize, usize, u32)> = Vec::new();

        for (club_idx, club) in country.clubs.iter().enumerate() {
            for (team_idx, team) in club.teams.iter().enumerate() {
                for player in team.players.iter() {
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
    /// Uses the same CA/position/jitter logic as should_retire for consistency.
    fn must_retire(player: &Player, date: NaiveDate) -> bool {
        if player.statuses.get().contains(&PlayerStatusType::Ret) {
            return true;
        }

        let age = player.age(date);
        let ca = player.player_attributes.current_ability;
        let position = player.position();

        let max_retire_age = match ca {
            0..=39   => 37u8,
            40..=69  => 38,
            70..=99  => 39,
            100..=129 => 40,
            130..=159 => 41,
            160..=179 => 42,
            _         => 43,
        };

        let position_offset: i8 = if position.is_goalkeeper() {
            2
        } else if position.is_defender() {
            1
        } else if position.is_forward() {
            -1
        } else {
            0
        };

        let id_jitter: i8 = match player.id % 3 {
            0 => -1,
            1 => 0,
            _ => 1,
        };

        let max_age = (max_retire_age as i16 + position_offset as i16 + id_jitter as i16)
            .clamp(35, 47) as u8;

        // Players still getting games get a 1-year grace period
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
            for (team_idx, team) in club.teams.iter().enumerate() {
                for player in team.players.iter() {
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

    fn should_retire(player: &Player, date: NaiveDate) -> bool {
        let age = player.age(date);
        let ca = player.player_attributes.current_ability;

        // Already marked for retirement
        if player.statuses.get().contains(&PlayerStatusType::Ret) {
            return true;
        }

        let position = player.position();
        let is_gk = position.is_goalkeeper();

        // Wider age windows with finer CA granularity (5-6 year spread)
        // This prevents mass retirement at a single age boundary
        let (min_retire_age, max_retire_age) = match ca {
            0..=39   => (31u8, 36u8),
            40..=69  => (32, 37),
            70..=99  => (33, 38),
            100..=129 => (34, 39),
            130..=159 => (35, 40),
            160..=179 => (36, 41),
            _         => (37, 42),
        };

        // Position adjustments: GK +2, defenders +1, forwards -1
        let position_offset: i8 = if is_gk {
            2
        } else if position.is_defender() {
            1
        } else if position.is_forward() {
            -1
        } else {
            0
        };

        // Per-player jitter based on player ID: spreads ±1 year
        // so players of the same age/ability don't all retire together
        let id_jitter: i8 = match player.id % 3 {
            0 => -1,
            1 => 0,
            _ => 1,
        };

        let min_age = (min_retire_age as i16 + position_offset as i16 + id_jitter as i16)
            .clamp(30, 44) as u8;
        let max_age = (max_retire_age as i16 + position_offset as i16 + id_jitter as i16)
            .clamp(34, 46) as u8;

        // Too young to retire
        if age < min_age {
            return false;
        }

        // Past max age — forced retirement
        if age >= max_age {
            return true;
        }

        // Still playing regularly? Reduce chance but don't fully block
        let current_season_games = player.statistics.total_games();
        let last_season_games = player.statistics_history.items
            .last()
            .map(|h| h.statistics.total_games())
            .unwrap_or(0);

        let total_recent_games = current_season_games + last_season_games;

        // Base retirement probability: gentle ramp across the window
        // Starts at 5%, increases quadratically to ~60% at max_age-1
        let range = (max_age - min_age).max(1) as f32;
        let years_over = (age - min_age) as f32;
        let progress = years_over / range; // 0.0 at min_age, ~1.0 at max_age
        let mut chance: f32 = 5.0 + 55.0 * progress * progress;

        // === Personality modifiers (continuous, not stepped) ===

        // Ambition: 0-20 scale, high ambition reduces chance
        let ambition = player.attributes.ambition;
        chance -= (ambition - 10.0) * 1.5; // -15 to +15

        // Determination: high determination = persist longer
        let determination = player.skills.mental.determination;
        chance -= (determination - 10.0) * 1.0; // -10 to +10

        // === Game time modifiers (graduated) ===
        if total_recent_games >= 30 {
            chance -= 25.0; // Regular starter across both seasons
        } else if total_recent_games >= 15 {
            chance -= 15.0;
        } else if total_recent_games >= 5 {
            chance -= 5.0;
        } else if total_recent_games == 0 {
            chance += 15.0; // No games at all — likely to retire
        }

        // === Ability decline modifier ===
        let pa = player.player_attributes.potential_ability;
        if pa > 0 {
            let decline_ratio = 1.0 - (ca as f32 / pa as f32);
            if decline_ratio > 0.5 {
                chance += 10.0; // Severely declined
            } else if decline_ratio > 0.3 {
                chance += 5.0;
            }
        }

        // Birth month adds sub-year variance:
        // players born later in the year get slight chance reduction
        // (they are effectively slightly younger within the same age bracket)
        let birth_month = player.birth_date.month() as f32;
        chance += (birth_month - 6.0) * 0.5; // -2.5 to +3.0

        chance = chance.clamp(3.0, 85.0);
        IntegerUtils::random(0, 100) < chance as i32
    }

    fn process_year_end_finances(country: &mut Country) {
        debug!("Processing year-end finances");

        for club in &mut country.clubs {
            let balance = club.finance.balance.balance;
            if balance > 0 {
                // 1% return on positive balance, capped at 2M per year.
                // Prevents infinite wealth compounding — clubs can't earn
                // meaningful interest on $500M+ balances.
                let interest = ((balance as f64 * 0.01) as i64).min(2_000_000);
                club.finance.balance.push_income(interest);
            } else if balance < 0 {
                // 5% penalty on negative balance (debt interest)
                let penalty = (balance.abs() as f64 * 0.05) as i64;
                club.finance.balance.push_outcome(penalty);
            }
        }
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

            let nominal_swap = relegation_spots.min(promotion_spots) as usize;

            // Read final tables
            let relegated_candidates: Vec<u32> = country
                .leagues
                .leagues
                .iter()
                .find(|l| l.id == tier1_id)
                .and_then(|l| l.final_table.as_ref())
                .map(|table| {
                    table.iter().rev().take(nominal_swap).map(|r| r.team_id).collect()
                })
                .unwrap_or_default();

            let promoted_candidates: Vec<u32> = country
                .leagues
                .leagues
                .iter()
                .find(|l| l.id == tier2_id)
                .and_then(|l| l.final_table.as_ref())
                .map(|table| {
                    table.iter().take(nominal_swap).map(|r| r.team_id).collect()
                })
                .unwrap_or_default();

            // Must balance: never relegate more than we promote (or vice versa)
            // or the top league silently shrinks each season.
            let swap_count = relegated_candidates.len().min(promoted_candidates.len());
            if swap_count == 0 {
                continue;
            }
            if swap_count < nominal_swap {
                info!(
                    "⚠️ Promotion/relegation pair {}→{} truncated: wanted {}, got {} (missing final_table entries)",
                    tier1_id, tier2_id, nominal_swap, swap_count
                );
            }
            let relegated_team_ids: Vec<u32> = relegated_candidates.into_iter().take(swap_count).collect();
            let promoted_team_ids: Vec<u32> = promoted_candidates.into_iter().take(swap_count).collect();

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

                // Move sub-teams to the matching youth league of the new main league
                if let Some(new_league_id) = new_main_league_id {
                    for team in &mut club.teams.teams {
                        if team.team_type != TeamType::Main {
                            let type_offset = match team.team_type {
                                TeamType::U18 => 100000,
                                TeamType::U19 => 110000,
                                TeamType::U20 => 120000,
                                TeamType::U21 => 130000,
                                TeamType::U23 => 140000,
                                _ => 200000, // B/Reserve teams use generic friendly league
                            };
                            team.league_id = Some(new_league_id + type_offset);
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
