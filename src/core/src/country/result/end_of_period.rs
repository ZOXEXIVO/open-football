use super::CountryResult;
use crate::club::team::reputation::{Achievement, AchievementType};
use crate::simulator::SimulatorData;
use crate::utils::{DateUtils, IntegerUtils};
use crate::{
    ClubResult, Country, HappinessEventType, Person, Player, PlayerHappiness, PlayerStatusType,
    StaffPosition, TeamInfo, TeamType,
};
use chrono::{Datelike, NaiveDate};
use log::{debug, info};
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
        let season = data
            .country(country_id)
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
        let latest_end_month = data
            .country(country_id)
            .map(|c| {
                c.leagues
                    .leagues
                    .iter()
                    .filter(|l| !l.friendly)
                    .map(|l| l.settings.season_ending_half.to_month)
                    .max()
                    .unwrap_or(season.end_month)
            })
            .unwrap_or(season.end_month);
        let promo_month = if latest_end_month == 12 {
            1u8
        } else {
            latest_end_month + 1
        };
        if DateUtils::is_month_beginning(date) && date.month() as u8 == promo_month {
            if let Some(country) = data.country_mut(country_id) {
                Self::process_promotion_relegation(country, date);
            }
        }

        // Late-season relegation-fear audit — runs once a month, scoped
        // to the second half of the season for tier-1+ leagues. Players
        // in the bottom (relegation_spots + 1) of the live table feel it.
        if DateUtils::is_month_beginning(date) {
            if let Some(country) = data.country_mut(country_id) {
                Self::process_relegation_fear_audit(country, date);
            }
        }

        if date.month() == 12 && date.day() == 31 {
            if let Some(country) = data.country_mut(country_id) {
                Self::process_year_end_finances(country);
            }
        }
    }

    fn process_season_awards(country: &mut Country, _club_results: &[ClubResult], date: NaiveDate) {
        debug!("Processing season awards");

        // Build team_id -> club index mapping
        let team_to_club: HashMap<u32, usize> = country
            .clubs
            .iter()
            .enumerate()
            .flat_map(|(ci, club)| club.teams.iter().map(move |t| (t.id, ci)))
            .collect();

        // Trophy reputation boost: league champions and promoted sides get
        // a durable rep bump that lingers for 2 seasons (see Achievement).
        // Collected before the prize-money loop so we don't mix concerns.
        let mut trophy_awards: Vec<(u32, AchievementType)> = Vec::new();
        // Per-player happiness events triggered by the same final tables.
        // (team_id, event, prestige) — prestige lets a lower-tier league
        // title fire `TrophyWon` at a smaller magnitude so it doesn't
        // compete with the promotion emotion that's the real headline.
        let mut player_team_events: Vec<(u32, HappinessEventType, f32)> = Vec::new();
        for league in &country.leagues.leagues {
            if league.friendly {
                continue;
            }
            let table = match &league.final_table {
                Some(t) if !t.is_empty() => t,
                _ => continue,
            };
            // For lower-tier leagues that *also* promote (Championship-style
            // setups), the title is real silverware but promotion is the
            // career-visible moment. We fire both, but soften `TrophyWon`
            // so the stack reads as "got promoted, also won the league"
            // rather than two huge wins.
            let promo_slots = league.settings.promotion_spots as usize;
            let lower_tier_with_promo = league.settings.tier > 1 && promo_slots > 0;
            if let Some(champion) = table.first() {
                trophy_awards.push((champion.team_id, AchievementType::LeagueTitle));
                let trophy_prestige = if lower_tier_with_promo { 0.6 } else { 1.0 };
                player_team_events.push((
                    champion.team_id,
                    HappinessEventType::TrophyWon,
                    trophy_prestige,
                ));
                if lower_tier_with_promo {
                    // Lower-league champions are *also* promoted — the
                    // promotion emotion is the dominant one.
                    player_team_events.push((
                        champion.team_id,
                        HappinessEventType::PromotionCelebration,
                        1.0,
                    ));
                }
            }
            if promo_slots > 0 {
                // Non-title promoted clubs (positions 2..=promo_slots).
                // Champion already handled above with the dual emit.
                for row in table.iter().take(promo_slots).skip(1) {
                    trophy_awards.push((row.team_id, AchievementType::Promotion));
                    player_team_events.push((
                        row.team_id,
                        HappinessEventType::PromotionCelebration,
                        1.0,
                    ));
                }
            }

            // Continental qualification — only top-tier leagues feed
            // continental competitions. Heuristic, intentionally:
            // `continent::result::competitions::collect_*_qualified_clubs`
            // is the authoritative qualification source, but it indexes by
            // *continental ranking* rather than league reputation, and it
            // runs as part of the autumn draw — there's no clean signal
            // queryable here at end-of-season for which clubs got the
            // letters in the post.
            //
            // The reputation bands below mirror the *spot counts* that
            // collector hands out (top-4 country = 7 European spots, next
            // 4 country tier = 4, smaller top flights = 2), so a player at
            // a club that genuinely qualifies will reliably hit this
            // happiness event. Bottom flights and friendlies skip.
            //
            // Skip position 0 (title winner already gets `TrophyWon`,
            // which subsumes European qualification — no double-counting).
            if league.settings.tier == 1 {
                let mut europe_spots = 0usize;
                if league.reputation >= 7000 {
                    europe_spots = 7; // CL top 4 + EL/UECL slots
                } else if league.reputation >= 5000 {
                    europe_spots = 4; // top-4 cup-tier qualification
                } else if league.reputation >= 3000 {
                    europe_spots = 2; // 2 spots in modest top flights
                }
                for row in table.iter().take(europe_spots).skip(1) {
                    player_team_events.push((
                        row.team_id,
                        HappinessEventType::QualifiedForEurope,
                        1.0,
                    ));
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
                                .find_mut_by_position(StaffPosition::Manager)
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
            debug!(
                "Season awards: {} - prize: ${}, TV: ${}",
                club.name, prize, tv
            );
        }

        // Per-player season events. One pass over the (team_id, event,
        // prestige) queue we built alongside the trophy collection, with
        // a generous 365-day cooldown so the same milestone never fires
        // twice in a season even if end-of-period happens to tick on
        // consecutive days.
        for (team_id, event, prestige) in player_team_events {
            Self::apply_team_squad_event(country, team_id, event, 365, prestige, date);
        }

        // Reset the per-player disciplinary running counter (yellow-
        // card tally). Active suspensions are preserved — a player
        // who picked up a red on the final matchday still serves it
        // in the new season, matching real FA / FIFA conventions.
        for club in &mut country.clubs {
            for team in club.teams.iter_mut() {
                for player in team.players.iter_mut() {
                    player.reset_season_disciplinary_state();
                }
            }
        }
    }

    /// Emit a team-level happiness event to every player on `team_id`,
    /// scaled by `prestige` (1.0 = default — domestic top cup / league
    /// title / promotion). Domestic minor cups should pass 0.7-0.8;
    /// continental trophies 1.2-1.5. Cooldown is forwarded to the
    /// player-side cooldown gate so repeated end-of-period ticks don't
    /// duplicate the event. No-op if the team isn't found in `country`.
    pub(crate) fn apply_team_squad_event(
        country: &mut Country,
        team_id: u32,
        event: HappinessEventType,
        cooldown_days: u16,
        prestige: f32,
        date: NaiveDate,
    ) {
        for club in &mut country.clubs {
            for team in club.teams.iter_mut() {
                if team.id != team_id {
                    continue;
                }
                for player in team.players.iter_mut() {
                    player.on_team_season_event_with_prestige(
                        event.clone(),
                        cooldown_days,
                        prestige,
                        date,
                    );
                }
            }
        }
    }

    /// Process loan returns — must run AFTER club_result.process() so that
    /// ClubResult player references remain valid during contract processing.
    pub(super) fn process_loan_returns(data: &mut SimulatorData, country_id: u32, date: NaiveDate) {
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
                let (team_name, team_slug, team_reputation) =
                    if team.team_type == TeamType::Main || main_name.is_none() {
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
    fn execute_loan_return(data: &mut SimulatorData, event: LoanReturnEvent, date: NaiveDate) {
        // Find parent club location first — abort early if missing
        let parent_pos = data.find_club_main_team(event.parent_club_id);
        if parent_pos.is_none() {
            debug!(
                "Loan return skipped: parent club {} not found for player {}",
                event.parent_club_id, event.player_id
            );
            return;
        }

        // Build parent TeamInfo before mutating data — needed by record_loan_return
        // to create a current-season entry if one was drained by season-end snapshot.
        let (pci, pcoi, pcli, pti) = parent_pos.unwrap();
        let parent_info = {
            let country = &data.continents[pci].countries[pcoi];
            let club = &country.clubs[pcli];
            let team = &club.teams.teams[pti];
            let league_info = team
                .league_id
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
                match data.continents[ci].countries[coi].clubs[cli].teams.teams[ti]
                    .players
                    .take_player(&event.player_id)
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
        debug!(
            "Loan return: player {} from club {} back to club {}",
            event.player_id, event.borrowing_club_id, event.parent_club_id
        );
        data.continents[pci].countries[pcoi].clubs[pcli].teams.teams[pti]
            .players
            .add(player);
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
                .players
                .take_player(&player_id)
            {
                debug!(
                    "Overdue retirement: {} (age {})",
                    player.full_name,
                    player.age(date)
                );
                player.statuses.add(date, PlayerStatusType::Ret);
                player.contract = None;
                player.retired = true;
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
            0..=39 => 37u8,
            40..=69 => 38,
            70..=99 => 39,
            100..=129 => 40,
            130..=159 => 41,
            160..=179 => 42,
            _ => 43,
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

        let max_age =
            (max_retire_age as i16 + position_offset as i16 + id_jitter as i16).clamp(35, 47) as u8;

        // Players still getting games get a 1-year grace period
        let has_recent_games = player.statistics.total_games() >= 5
            || player
                .statistics_history
                .items
                .last()
                .map(|h| h.statistics.total_games() >= 5)
                .unwrap_or(true);

        let effective_max = if has_recent_games {
            max_age + 1
        } else {
            max_age
        };

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
                .players
                .take_player(&player_id)
            {
                debug!(
                    "Player retired: {} (age {})",
                    player.full_name,
                    player.age(date)
                );
                player.statuses.add(date, PlayerStatusType::Ret);
                player.contract = None;
                player.retired = true;
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
            0..=39 => (31u8, 36u8),
            40..=69 => (32, 37),
            70..=99 => (33, 38),
            100..=129 => (34, 39),
            130..=159 => (35, 40),
            160..=179 => (36, 41),
            _ => (37, 42),
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

        let min_age =
            (min_retire_age as i16 + position_offset as i16 + id_jitter as i16).clamp(30, 44) as u8;
        let max_age =
            (max_retire_age as i16 + position_offset as i16 + id_jitter as i16).clamp(34, 46) as u8;

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
        let last_season_games = player
            .statistics_history
            .items
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

    fn process_promotion_relegation(country: &mut Country, date: NaiveDate) {
        // Collect league info: (league_id, tier, relegation_spots, promotion_spots)
        let league_info: Vec<(u32, u8, u8, u8)> = country
            .leagues
            .leagues
            .iter()
            .map(|l| {
                (
                    l.id,
                    l.settings.tier,
                    l.settings.relegation_spots,
                    l.settings.promotion_spots,
                )
            })
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
                    table
                        .iter()
                        .rev()
                        .take(nominal_swap)
                        .map(|r| r.team_id)
                        .collect()
                })
                .unwrap_or_default();

            let promoted_candidates: Vec<u32> = country
                .leagues
                .leagues
                .iter()
                .find(|l| l.id == tier2_id)
                .and_then(|l| l.final_table.as_ref())
                .map(|table| table.iter().take(nominal_swap).map(|r| r.team_id).collect())
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
            let relegated_team_ids: Vec<u32> =
                relegated_candidates.into_iter().take(swap_count).collect();
            let promoted_team_ids: Vec<u32> =
                promoted_candidates.into_iter().take(swap_count).collect();

            // Build a tier-2 club expectation map: clubs that finished
            // inside the promotion window (top promotion_spots + a couple
            // of playoff places) "expected" promotion. NonPromotionRelease
            // only activates for those — finishing 18th in the second tier
            // shouldn't trigger any player's escape clause.
            let promotion_window_team_ids: Vec<u32> = country
                .leagues
                .leagues
                .iter()
                .find(|l| l.id == tier2_id)
                .and_then(|l| l.final_table.as_ref())
                .map(|t| {
                    let window = (promotion_spots as usize + 2).min(t.len());
                    t.iter().take(window).map(|r| r.team_id).collect::<Vec<_>>()
                })
                .unwrap_or_default();

            // Swap league_ids on teams and move sub-teams to matching friendly league
            for club in &mut country.clubs {
                let mut new_main_league_id: Option<u32> = None;
                // Aggregate one-shot bonus payouts owed for this season's
                // outcome. Charged to the club after the team loops so we
                // only hold a single mut borrow at a time.
                let mut bonus_total: i64 = 0;

                for team in &mut club.teams.teams {
                    if relegated_team_ids.contains(&team.id) {
                        info!(
                            "⬇️ Relegation: team {} ({}) moves to league {}",
                            team.name, team.id, tier2_id
                        );
                        team.league_id = Some(tier2_id);
                        new_main_league_id = Some(tier2_id);
                        // Year-defining wound — emit per player. The promo
                        // counterpart already ran in season_awards via the
                        // PromotionCelebration emit; we don't duplicate the
                        // upward case here.
                        // Inline emit (we hold &mut to team here, so the
                        // Self::apply_team_squad_event helper which takes
                        // &mut country would conflict).
                        for player in team.players.iter_mut() {
                            player.on_team_season_event(HappinessEventType::Relegated, 365, date);
                            // Activate any RelegationWageDecrease and
                            // RelegationFeeRelease clauses on the player's
                            // contract. Each helper consumes the matching
                            // clause so subsequent relegations don't double-apply.
                            if let Some(c) = player.contract.as_mut() {
                                let _ = c.apply_relegation_wage_decrease();
                                let _ = c.take_relegation_release();
                            }
                        }
                    } else if promoted_team_ids.contains(&team.id) {
                        info!(
                            "⬆️ Promotion: team {} ({}) moves to league {}",
                            team.name, team.id, tier1_id
                        );
                        team.league_id = Some(tier1_id);
                        new_main_league_id = Some(tier1_id);
                        // Symmetric to the relegation hooks above —
                        // PromotionWageIncrease bumps salary; players
                        // also keep their existing contracts (no clause
                        // for "release on promotion").
                        for player in team.players.iter_mut() {
                            if let Some(c) = player.contract.as_mut() {
                                let _ = c.apply_promotion_wage_increase();
                                // PromotionFee bonus is paid out as a
                                // one-shot lump sum to every player on
                                // the promoted team who has the bonus.
                                for bonus in &c.bonuses {
                                    if matches!(
                                        bonus.bonus_type,
                                        crate::ContractBonusType::PromotionFee
                                    ) && bonus.value > 0
                                    {
                                        bonus_total += bonus.value as i64;
                                    }
                                }
                            }
                        }
                    } else {
                        // Survival in a tier-1 league with relegation
                        // places: pay AvoidRelegationFee bonuses if the
                        // team was at risk this season. Approximated by
                        // "team carries the clause" — the bonus is only
                        // installed on contracts when the club expected
                        // a relegation battle.
                        let in_tier1_with_relegation = team.league_id == Some(tier1_id);
                        if in_tier1_with_relegation {
                            for player in team.players.iter_mut() {
                                if let Some(c) = player.contract.as_mut() {
                                    for bonus in &c.bonuses {
                                        if matches!(
                                            bonus.bonus_type,
                                            crate::ContractBonusType::AvoidRelegationFee
                                        ) && bonus.value > 0
                                        {
                                            bonus_total += bonus.value as i64;
                                        }
                                    }
                                }
                            }
                        }
                    }
                }

                // NonPromotionRelease activates only for clubs that
                // genuinely expected to go up — finished inside the
                // promotion window — and didn't. Mid-table and
                // bottom-of-tier-2 clubs are excluded so a player's
                // escape clause doesn't fire just because the club is
                // in the wrong league.
                let club_was_in_window = club
                    .teams
                    .teams
                    .iter()
                    .any(|t| promotion_window_team_ids.contains(&t.id));
                let club_was_promoted = club
                    .teams
                    .teams
                    .iter()
                    .any(|t| promoted_team_ids.contains(&t.id));
                if club_was_in_window && !club_was_promoted {
                    for team in &mut club.teams.teams {
                        if team.league_id == Some(tier2_id) {
                            for player in team.players.iter_mut() {
                                if let Some(c) = player.contract.as_mut() {
                                    let _ = c.take_non_promotion_release();
                                }
                            }
                        }
                    }
                }

                if bonus_total > 0 {
                    club.finance.balance.push_expense_player_wages(bonus_total);
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

    /// Monthly late-season audit: surface ambient relegation dread for
    /// players whose team is in (or close to) the drop zone. Gated to
    /// season_progress >= 0.6 so an early-season slump doesn't generate
    /// the event — pressure builds with the season trajectory.
    ///
    /// The cooldown on `RelegationFear` is 30 days, so this monthly scan
    /// produces at most one event per player per month even if their team
    /// fluctuates around the line.
    fn process_relegation_fear_audit(country: &mut Country, date: NaiveDate) {
        // Collect (team_id, in_zone, near_zone) from each non-friendly
        // league with a meaningful relegation slot count and enough
        // season progress to register pressure.
        let mut at_risk_teams: Vec<u32> = Vec::new();

        for league in &country.leagues.leagues {
            if league.friendly || league.settings.relegation_spots == 0 {
                continue;
            }
            let total_teams = league.table.rows.len();
            if total_teams == 0 {
                continue;
            }
            let matches_played = league
                .table
                .rows
                .iter()
                .map(|r| r.played)
                .max()
                .unwrap_or(0);
            let total_matches = ((total_teams - 1) * 2) as u8;
            if total_matches == 0 {
                continue;
            }
            let progress = matches_played as f32 / total_matches as f32;
            if progress < 0.6 {
                continue;
            }

            // Bottom (relegation_spots + 1) teams: the in-zone clubs plus
            // the one immediately above. They're the squads doing the
            // morning newspaper math every week.
            let zone = league.settings.relegation_spots as usize + 1;
            let table = &league.table.rows;
            for row in table.iter().rev().take(zone) {
                at_risk_teams.push(row.team_id);
            }
        }

        if at_risk_teams.is_empty() {
            return;
        }

        for club in &mut country.clubs {
            for team in club.teams.iter_mut() {
                if !at_risk_teams.contains(&team.id) {
                    continue;
                }
                for player in team.players.iter_mut() {
                    player.on_team_season_event(HappinessEventType::RelegationFear, 30, date);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::academy::ClubAcademy;
    use crate::club::player::builder::PlayerBuilder;
    use crate::league::{DayMonthPeriod, League, LeagueCollection, LeagueSettings, LeagueTableRow};
    use crate::shared::Location;
    use crate::{
        Club, ClubColors, ClubFinances, ClubStatus, PersonAttributes, PlayerAttributes,
        PlayerCollection, PlayerPositions, PlayerSkills, StaffCollection, TeamBuilder,
        TeamCollection, TeamReputation, TeamType, TrainingSchedule,
    };

    fn d(y: i32, m: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, day).unwrap()
    }

    fn make_player(id: u32) -> Player {
        let mut p = PlayerBuilder::new()
            .id(id)
            .full_name(crate::shared::fullname::FullName::new(
                "Test".to_string(),
                format!("Player{}", id),
            ))
            .birth_date(d(2000, 1, 1))
            .country_id(1)
            .attributes(PersonAttributes::default())
            .skills(PlayerSkills::default())
            .positions(PlayerPositions { positions: vec![] })
            .player_attributes(PlayerAttributes::default())
            .build()
            .unwrap();
        // Regular starter so participation factor = 1.0.
        p.statistics.played = 30;
        p
    }

    fn make_training_schedule() -> TrainingSchedule {
        use chrono::NaiveTime;
        TrainingSchedule::new(
            NaiveTime::from_hms_opt(9, 0, 0).unwrap(),
            NaiveTime::from_hms_opt(15, 0, 0).unwrap(),
        )
    }

    fn make_team(id: u32, club_id: u32, league_id: u32, players: Vec<Player>) -> crate::Team {
        TeamBuilder::new()
            .id(id)
            .league_id(Some(league_id))
            .club_id(club_id)
            .name(format!("Team{}", id))
            .slug(format!("team{}", id))
            .team_type(TeamType::Main)
            .players(PlayerCollection::new(players))
            .staffs(StaffCollection::new(Vec::new()))
            .reputation(TeamReputation::new(100, 100, 200))
            .training_schedule(make_training_schedule())
            .build()
            .unwrap()
    }

    fn make_club(id: u32, teams: Vec<crate::Team>) -> Club {
        Club::new(
            id,
            format!("Club{}", id),
            Location::new(1),
            ClubFinances::new(1_000_000, Vec::new()),
            ClubAcademy::new(3),
            ClubStatus::Professional,
            ClubColors::default(),
            TeamCollection::new(teams),
            crate::ClubFacilities::default(),
        )
    }

    fn make_league_with_table(id: u32, rep: u16, rows: Vec<(u32, u8, u8)>) -> League {
        let mut league = League::new(
            id,
            format!("League{}", id),
            format!("league{}", id),
            1,
            rep,
            LeagueSettings {
                season_starting_half: DayMonthPeriod::new(1, 8, 31, 12),
                season_ending_half: DayMonthPeriod::new(1, 1, 31, 5),
                tier: 1,
                promotion_spots: 0,
                relegation_spots: 3,
                league_group: None,
            },
            false,
        );
        league.table.rows = rows
            .into_iter()
            .map(|(team_id, played, points)| LeagueTableRow {
                team_id,
                played,
                win: 0,
                draft: 0,
                lost: 0,
                goal_scored: 0,
                goal_concerned: 0,
                points,
                points_deduction: 0,
            })
            .collect();
        league
    }

    fn build_country(clubs: Vec<Club>, leagues: Vec<League>) -> Country {
        Country::builder()
            .id(1)
            .code("EN".to_string())
            .slug("england".to_string())
            .name("England".to_string())
            .continent_id(1)
            .leagues(LeagueCollection::new(leagues))
            .clubs(clubs)
            .build()
            .unwrap()
    }

    fn happiness_event_count(player: &Player, kind: &HappinessEventType) -> usize {
        player
            .happiness
            .recent_events
            .iter()
            .filter(|e| e.event_type == *kind)
            .count()
    }

    #[test]
    fn relegation_fear_fires_for_bottom_team_late_season() {
        // 20-team league, total_matches = 38. matches_played = 30 → 78%
        // progress (>= 60% gate). Team 5 finishes second-to-bottom of the
        // simulated 5-team table, so it falls inside the (relegation_spots
        // + 1 = 4) audit window.
        let p1 = make_player(1);
        let p2 = make_player(2);
        let team = make_team(5, 50, 1, vec![p1, p2]);
        let club = make_club(50, vec![team]);

        // 5 rows; bottom 4 (relegation_spots 3 + 1) eligible.
        let rows = vec![
            (1, 30, 70),
            (2, 30, 50),
            (3, 30, 35),
            (5, 30, 20),
            (4, 30, 10),
        ];
        let league = make_league_with_table(1, 7000, rows);
        let mut country = build_country(vec![club], vec![league]);

        CountryResult::process_relegation_fear_audit(&mut country, d(2032, 4, 1));

        let players = &country.clubs[0].teams.teams[0].players.players;
        assert_eq!(
            happiness_event_count(&players[0], &HappinessEventType::RelegationFear),
            1
        );
        assert_eq!(
            happiness_event_count(&players[1], &HappinessEventType::RelegationFear),
            1
        );
    }

    #[test]
    fn relegation_fear_silent_for_safe_team() {
        let p1 = make_player(1);
        let team = make_team(1, 50, 1, vec![p1]);
        let club = make_club(50, vec![team]);
        // Team 1 is top of the table — way out of the drop zone.
        let rows = vec![
            (1, 30, 70),
            (2, 30, 50),
            (3, 30, 35),
            (4, 30, 20),
            (5, 30, 10),
        ];
        let league = make_league_with_table(1, 7000, rows);
        let mut country = build_country(vec![club], vec![league]);

        CountryResult::process_relegation_fear_audit(&mut country, d(2032, 4, 1));

        let player = &country.clubs[0].teams.teams[0].players.players[0];
        assert_eq!(
            happiness_event_count(player, &HappinessEventType::RelegationFear),
            0
        );
    }

    #[test]
    fn relegation_fear_silent_too_early_in_season() {
        let p1 = make_player(1);
        let team = make_team(5, 50, 1, vec![p1]);
        let club = make_club(50, vec![team]);
        // 5-team league → 8 matches per team. Stop at 4 played (50%) so
        // we sit *under* the 60% audit gate. Drop-zone position alone
        // shouldn't trigger fear in early autumn.
        let rows = vec![(1, 4, 10), (2, 4, 8), (3, 4, 6), (5, 4, 2), (4, 4, 1)];
        let league = make_league_with_table(1, 7000, rows);
        let mut country = build_country(vec![club], vec![league]);

        CountryResult::process_relegation_fear_audit(&mut country, d(2031, 11, 1));

        let player = &country.clubs[0].teams.teams[0].players.players[0];
        assert_eq!(
            happiness_event_count(player, &HappinessEventType::RelegationFear),
            0
        );
    }

    #[test]
    fn apply_team_squad_event_emits_to_each_player() {
        let p1 = make_player(1);
        let p2 = make_player(2);
        let team = make_team(7, 70, 1, vec![p1, p2]);
        let club = make_club(70, vec![team]);
        // No relegation_spots needed — using the helper directly.
        let league = make_league_with_table(1, 5000, vec![]);
        let mut country = build_country(vec![club], vec![league]);

        CountryResult::apply_team_squad_event(
            &mut country,
            7,
            HappinessEventType::TrophyWon,
            365,
            1.0,
            d(2032, 5, 30),
        );

        let players = &country.clubs[0].teams.teams[0].players.players;
        assert_eq!(
            happiness_event_count(&players[0], &HappinessEventType::TrophyWon),
            1
        );
        assert_eq!(
            happiness_event_count(&players[1], &HappinessEventType::TrophyWon),
            1
        );
    }

    #[test]
    fn apply_team_squad_event_silent_for_unknown_team() {
        let p1 = make_player(1);
        let team = make_team(1, 50, 1, vec![p1]);
        let club = make_club(50, vec![team]);
        let league = make_league_with_table(1, 5000, vec![]);
        let mut country = build_country(vec![club], vec![league]);

        // team_id 999 doesn't exist — must no-op.
        CountryResult::apply_team_squad_event(
            &mut country,
            999,
            HappinessEventType::TrophyWon,
            365,
            1.0,
            d(2032, 5, 30),
        );

        let player = &country.clubs[0].teams.teams[0].players.players[0];
        assert_eq!(
            happiness_event_count(player, &HappinessEventType::TrophyWon),
            0
        );
    }

    #[test]
    fn apply_team_squad_event_prestige_scales_magnitude() {
        let p1 = make_player(1);
        let team_a = make_team(10, 100, 1, vec![p1]);
        let club_a = make_club(100, vec![team_a]);
        let p2 = make_player(2);
        let team_b = make_team(20, 200, 1, vec![p2]);
        let club_b = make_club(200, vec![team_b]);
        let league = make_league_with_table(1, 5000, vec![]);
        let mut country = build_country(vec![club_a, club_b], vec![league]);

        CountryResult::apply_team_squad_event(
            &mut country,
            10,
            HappinessEventType::TrophyWon,
            365,
            1.0,
            d(2032, 5, 30),
        );
        CountryResult::apply_team_squad_event(
            &mut country,
            20,
            HappinessEventType::TrophyWon,
            365,
            1.5,
            d(2032, 5, 30),
        );

        let mag_a = country.clubs[0].teams.teams[0].players.players[0]
            .happiness
            .recent_events
            .iter()
            .find(|e| e.event_type == HappinessEventType::TrophyWon)
            .unwrap()
            .magnitude;
        let mag_b = country.clubs[1].teams.teams[0].players.players[0]
            .happiness
            .recent_events
            .iter()
            .find(|e| e.event_type == HappinessEventType::TrophyWon)
            .unwrap()
            .magnitude;
        assert!(
            mag_b > mag_a,
            "prestige 1.5 ({}) should exceed prestige 1.0 ({})",
            mag_b,
            mag_a
        );
    }

    #[test]
    fn apply_team_squad_event_cooldown_blocks_repeat() {
        let p1 = make_player(1);
        let team = make_team(7, 70, 1, vec![p1]);
        let club = make_club(70, vec![team]);
        let league = make_league_with_table(1, 5000, vec![]);
        let mut country = build_country(vec![club], vec![league]);

        let date = d(2032, 5, 30);
        CountryResult::apply_team_squad_event(
            &mut country,
            7,
            HappinessEventType::TrophyWon,
            365,
            1.0,
            date,
        );
        // Second emit on the same date — cooldown must absorb it.
        CountryResult::apply_team_squad_event(
            &mut country,
            7,
            HappinessEventType::TrophyWon,
            365,
            1.0,
            date,
        );

        let player = &country.clubs[0].teams.teams[0].players.players[0];
        assert_eq!(
            happiness_event_count(player, &HappinessEventType::TrophyWon),
            1
        );
    }

    #[test]
    fn relegation_fear_cooldown_blocks_repeat_audit() {
        let p1 = make_player(1);
        let team = make_team(5, 50, 1, vec![p1]);
        let club = make_club(50, vec![team]);
        let rows = vec![
            (1, 30, 70),
            (2, 30, 50),
            (3, 30, 35),
            (5, 30, 20),
            (4, 30, 10),
        ];
        let league = make_league_with_table(1, 7000, rows);
        let mut country = build_country(vec![club], vec![league]);

        // First audit fires; second consecutive monthly audit must respect
        // the 30-day cooldown and silently skip.
        CountryResult::process_relegation_fear_audit(&mut country, d(2032, 4, 1));
        CountryResult::process_relegation_fear_audit(&mut country, d(2032, 4, 1));

        let player = &country.clubs[0].teams.teams[0].players.players[0];
        assert_eq!(
            happiness_event_count(player, &HappinessEventType::RelegationFear),
            1
        );
    }

    fn make_league_with_settings(
        id: u32,
        tier: u8,
        promo: u8,
        rel: u8,
        rows: Vec<(u32, u8, u8)>,
    ) -> League {
        let mut league = League::new(
            id,
            format!("League{}", id),
            format!("league{}", id),
            1,
            5000,
            LeagueSettings {
                season_starting_half: DayMonthPeriod::new(1, 8, 31, 12),
                season_ending_half: DayMonthPeriod::new(1, 1, 31, 5),
                tier,
                promotion_spots: promo,
                relegation_spots: rel,
                league_group: None,
            },
            false,
        );
        let table_rows: Vec<LeagueTableRow> = rows
            .into_iter()
            .map(|(team_id, played, points)| LeagueTableRow {
                team_id,
                played,
                win: 0,
                draft: 0,
                lost: 0,
                goal_scored: 0,
                goal_concerned: 0,
                points,
                points_deduction: 0,
            })
            .collect();
        league.table.rows = table_rows.clone();
        league.final_table = Some(table_rows);
        league
    }

    fn make_simple_team(id: u32, club_id: u32, league_id: u32) -> crate::Team {
        TeamBuilder::new()
            .id(id)
            .league_id(Some(league_id))
            .club_id(club_id)
            .name(format!("Team{}", id))
            .slug(format!("team{}", id))
            .team_type(TeamType::Main)
            .players(PlayerCollection::new(Vec::new()))
            .staffs(StaffCollection::new(Vec::new()))
            .reputation(TeamReputation::new(100, 100, 200))
            .training_schedule(make_training_schedule())
            .build()
            .unwrap()
    }

    #[test]
    fn promotion_relegation_swaps_teams_between_adjacent_tiers() {
        // Tier 1 has 4 teams (10..=13), bottom 2 relegate.
        // Tier 2 has 4 teams (20..=23), top 2 promote.
        let tier1_clubs: Vec<Club> = (10..=13)
            .map(|id| make_club(id as u32, vec![make_simple_team(id as u32, id as u32, 1)]))
            .collect();
        let tier2_clubs: Vec<Club> = (20..=23)
            .map(|id| make_club(id as u32, vec![make_simple_team(id as u32, id as u32, 2)]))
            .collect();
        let tier1 = make_league_with_settings(
            1,
            1,
            0,
            2,
            vec![(10, 30, 70), (11, 30, 60), (12, 30, 30), (13, 30, 20)],
        );
        let tier2 = make_league_with_settings(
            2,
            2,
            2,
            0,
            vec![(20, 30, 80), (21, 30, 70), (22, 30, 40), (23, 30, 25)],
        );

        let mut all_clubs = tier1_clubs;
        all_clubs.extend(tier2_clubs);
        let mut country = build_country(all_clubs, vec![tier1, tier2]);

        CountryResult::process_promotion_relegation(&mut country, d(2032, 6, 1));

        // Bottom-2 of tier 1 (12, 13) drop to tier 2.
        let team_12_league = country
            .clubs
            .iter()
            .find(|c| c.id == 12)
            .and_then(|c| c.teams.iter().next())
            .map(|t| t.league_id);
        assert_eq!(team_12_league, Some(Some(2)));
        let team_13_league = country
            .clubs
            .iter()
            .find(|c| c.id == 13)
            .and_then(|c| c.teams.iter().next())
            .map(|t| t.league_id);
        assert_eq!(team_13_league, Some(Some(2)));

        // Top-2 of tier 2 (20, 21) rise to tier 1.
        let team_20_league = country
            .clubs
            .iter()
            .find(|c| c.id == 20)
            .and_then(|c| c.teams.iter().next())
            .map(|t| t.league_id);
        assert_eq!(team_20_league, Some(Some(1)));
        let team_21_league = country
            .clubs
            .iter()
            .find(|c| c.id == 21)
            .and_then(|c| c.teams.iter().next())
            .map(|t| t.league_id);
        assert_eq!(team_21_league, Some(Some(1)));

        // Mid-table teams stay put — stable team IDs verification.
        let team_10_league = country
            .clubs
            .iter()
            .find(|c| c.id == 10)
            .and_then(|c| c.teams.iter().next())
            .map(|t| t.league_id);
        assert_eq!(team_10_league, Some(Some(1)));
        let team_22_league = country
            .clubs
            .iter()
            .find(|c| c.id == 22)
            .and_then(|c| c.teams.iter().next())
            .map(|t| t.league_id);
        assert_eq!(team_22_league, Some(Some(2)));

        // final_table is cleared after processing so the next season
        // doesn't double-apply.
        assert!(country.leagues.leagues.iter().all(|l| l.final_table.is_none()));
    }

    #[test]
    fn promotion_relegation_is_a_noop_when_no_adjacent_tier_exists() {
        // Single tier-1 league with relegation_spots > 0 but no tier-2
        // counterpart — must not mutate any team's league_id.
        let clubs: Vec<Club> = (10..=12)
            .map(|id| make_club(id as u32, vec![make_simple_team(id as u32, id as u32, 1)]))
            .collect();
        let tier1 = make_league_with_settings(
            1,
            1,
            0,
            1,
            vec![(10, 30, 70), (11, 30, 50), (12, 30, 20)],
        );
        let mut country = build_country(clubs, vec![tier1]);

        CountryResult::process_promotion_relegation(&mut country, d(2032, 6, 1));

        for club in &country.clubs {
            for team in &club.teams.teams {
                assert_eq!(team.league_id, Some(1));
            }
        }
    }
}
