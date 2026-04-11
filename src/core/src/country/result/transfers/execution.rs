use chrono::{Datelike, NaiveDate};
use log::debug;
use super::types::can_club_accept_player;
use crate::{
    Country, Person, Player, PlayerClubContract, PlayerHappiness, PlayerPlan, PlayerStatusType,
    TeamInfo, TeamType,
};
use crate::simulator::SimulatorData;

/// Unified transfer execution — handles both domestic and cross-country.
/// When selling_country_id == buying_country_id it's domestic (single country).
/// When different, the player moves between countries.
/// Returns true if the player was successfully placed at the buying club.
pub(crate) fn execute_transfer(
    data: &mut SimulatorData,
    player_id: u32,
    selling_country_id: u32,
    selling_club_id: u32,
    buying_country_id: u32,
    buying_club_id: u32,
    fee: f64,
    is_loan: bool,
    date: NaiveDate,
) -> bool {
    // Safety: never transfer/loan a player to their own club
    if selling_club_id == buying_club_id {
        debug!("Blocked self-transfer: club {} tried to {} player {} to itself",
            selling_club_id, if is_loan { "loan" } else { "transfer" }, player_id);
        return false;
    }

    // Safety: can't loan a player who is already on loan
    if is_loan {
        let already_on_loan = data.player(player_id)
            .map(|p| p.is_on_loan())
            .unwrap_or(false);
        if already_on_loan {
            debug!("Blocked re-loan: player {} is already on loan", player_id);
            return false;
        }
    }
    if selling_country_id == buying_country_id {
        // Domestic — work within a single country
        if let Some(country) = data.country_mut(selling_country_id) {
            if is_loan {
                execute_loan_within_country(country, player_id, selling_club_id, buying_club_id, fee, date)
            } else {
                execute_transfer_within_country(country, player_id, selling_club_id, buying_club_id, fee, date)
            }
        } else {
            false
        }
    } else {
        // Cross-country — take from one country, place in another
        if is_loan {
            execute_loan_across_countries(data, player_id, selling_country_id, selling_club_id, buying_country_id, buying_club_id, fee, date)
        } else {
            execute_transfer_across_countries(data, player_id, selling_country_id, selling_club_id, buying_country_id, buying_club_id, fee, date)
        }
    }
}

// ============================================================
// Internal: domestic (single country)
// ============================================================

pub(crate) fn execute_transfer_within_country(
    country: &mut Country,
    player_id: u32,
    selling_club_id: u32,
    buying_club_id: u32,
    fee: f64,
    date: NaiveDate,
) -> bool {
    let mut player = None;
    let mut from_info: Option<TeamInfo> = None;
    let mut selling_league_id = None;

    if let Some(selling_club) = country.clubs.iter_mut().find(|c| c.id == selling_club_id) {
        if let Some(main_team) = selling_club.teams.main() {
            selling_league_id = main_team.league_id;
            from_info = Some(TeamInfo {
                name: selling_club.name.clone(),
                slug: main_team.slug.clone(),
                reputation: main_team.reputation.world,
                league_name: String::new(),
                league_slug: String::new(),
                            });
        }

        for team in &mut selling_club.teams.teams {
            if let Some(p) = team.players.take_player(&player_id) {
                player = Some(p);
                team.transfer_list.remove(player_id);
                break;
            }
        }

        // Only credit income when player was actually found and taken
        if player.is_some() {
            selling_club.finance.add_transfer_income(fee);
        }
    }

    if let Some(mut player) = player {
        if let Some(ref mut from) = from_info {
            let (league_name, league_slug) = selling_league_id
                .and_then(|lid| country.leagues.leagues.iter().find(|l| l.id == lid))
                .and_then(|l| if l.friendly { country.leagues.leagues.iter().find(|ml| !ml.friendly) } else { Some(l) })
                .map(|l| (l.name.clone(), l.slug.clone()))
                .unwrap_or_default();
            from.league_name = league_name;
            from.league_slug = league_slug;
        }

        // Check squad capacity BEFORE recording history — otherwise a rejected
        // transfer creates a phantom career entry with no matching transfer record
        let to_info = resolve_buying_club_info(country, buying_club_id);
        let can_accept = country.clubs.iter().find(|c| c.id == buying_club_id)
            .map(|c| can_club_accept_player(c))
            .unwrap_or(false);

        if !can_accept {
            debug!("Transfer rejected: club {} squad full, returning player {}", buying_club_id, player_id);
            if let Some(selling_club) = country.clubs.iter_mut().find(|c| c.id == selling_club_id) {
                if !selling_club.teams.teams.is_empty() {
                    selling_club.teams.teams[0].players.add(player);
                }
                selling_club.finance.add_transfer_income(-fee);
            }
            return false;
        }

        // Always record history — use fallback TeamInfo if club info couldn't be resolved
        let from = from_info.unwrap_or_else(|| TeamInfo {
            name: String::new(), slug: String::new(), reputation: 0,
            league_name: String::new(), league_slug: String::new(),
        });
        let to = to_info.unwrap_or_else(|| TeamInfo {
            name: String::new(), slug: String::new(), reputation: 0,
            league_name: String::new(), league_slug: String::new(),
        });
        player.on_transfer(&from, &to, fee, date);
        player.sold_from = Some((selling_club_id, fee));

        clear_transfer_statuses(&mut player);
        assign_new_contract(&mut player, fee, date, false);
        assign_signing_plan(&mut player, fee, date);

        if let Some(buying_club) = country.clubs.iter_mut().find(|c| c.id == buying_club_id) {
            buying_club.finance.spend_from_transfer_budget(fee);
            if !buying_club.teams.teams.is_empty() {
                buying_club.teams.teams[0].players.add(player);
            }
        }

        country.transfer_market.complete_listings_for_player(player_id);
        if let Some(selling_club) = country.clubs.iter_mut().find(|c| c.id == selling_club_id) {
            selling_club.transfer_plan.loan_out_candidates.retain(|c| c.player_id != player_id);
        }

        debug!("Transfer completed: player {} from club {} to club {} for {}", player_id, selling_club_id, buying_club_id, fee);
        true
    } else {
        debug!("Transfer failed: player {} not found at club {}", player_id, selling_club_id);
        false
    }
}

fn execute_loan_within_country(
    country: &mut Country,
    player_id: u32,
    selling_club_id: u32,
    buying_club_id: u32,
    loan_fee: f64,
    date: NaiveDate,
) -> bool {
    let mut player = None;
    let mut from_info: Option<TeamInfo> = None;
    let mut selling_league_id = None;
    let mut from_team_id = 0u32;

    if let Some(selling_club) = country.clubs.iter_mut().find(|c| c.id == selling_club_id) {
        if let Some(main_team) = selling_club.teams.main() {
            selling_league_id = main_team.league_id;
            from_info = Some(TeamInfo {
                name: selling_club.name.clone(),
                slug: main_team.slug.clone(),
                reputation: main_team.reputation.world,
                league_name: String::new(),
                league_slug: String::new(),
                            });
        }

        from_team_id = selling_club.teams.find_team_with_player(player_id)
            .map(|t| t.id)
            .unwrap_or(0);

        // Move to reserve before loaning
        let main_idx = selling_club.teams.main_index();
        let reserve_idx = selling_club.teams.index_of_type(TeamType::Reserve)
            .or_else(|| selling_club.teams.index_of_type(TeamType::B));

        if let (Some(mi), Some(ri)) = (main_idx, reserve_idx) {
            if mi != ri {
                if let Some(p) = selling_club.teams.teams[mi].players.take_player(&player_id) {
                    selling_club.teams.teams[ri].players.add(p);
                }
            }
        }

        for team in &mut selling_club.teams.teams {
            if let Some(p) = team.players.take_player(&player_id) {
                player = Some(p);
                team.transfer_list.remove(player_id);
                break;
            }
        }

        // Only credit income when player was actually found and taken
        if player.is_some() {
            selling_club.finance.add_transfer_income(loan_fee);
        }
    }

    if let Some(mut player) = player {
        if let Some(ref mut from) = from_info {
            let (league_name, league_slug) = selling_league_id
                .and_then(|lid| country.leagues.leagues.iter().find(|l| l.id == lid))
                .and_then(|l| if l.friendly { country.leagues.leagues.iter().find(|ml| !ml.friendly) } else { Some(l) })
                .map(|l| (l.name.clone(), l.slug.clone()))
                .unwrap_or_default();
            from.league_name = league_name;
            from.league_slug = league_slug;
        }

        let loan_end = compute_loan_end(selling_league_id, country, date);

        // Ensure parent contract doesn't expire during the loan.
        // Extend it to at least loan_end + 1 year so the parent club
        // has time to evaluate and renew after the player returns.
        ensure_parent_contract_covers_loan(&mut player, loan_end);

        // Check squad capacity BEFORE recording history — otherwise a rejected
        // loan creates a phantom career entry with no matching transfer record
        let to_info = resolve_buying_club_info(country, buying_club_id);
        let can_accept = country.clubs.iter().find(|c| c.id == buying_club_id)
            .map(|c| can_club_accept_player(c))
            .unwrap_or(false);

        if !can_accept {
            debug!("Loan rejected: club {} squad full, returning player {}", buying_club_id, player_id);
            if let Some(selling_club) = country.clubs.iter_mut().find(|c| c.id == selling_club_id) {
                if !selling_club.teams.teams.is_empty() {
                    selling_club.teams.teams[0].players.add(player);
                }
            }
            return false;
        }

        // Always record history — use fallback TeamInfo if club info couldn't be resolved
        let from = from_info.unwrap_or_else(|| TeamInfo {
            name: String::new(), slug: String::new(), reputation: 0,
            league_name: String::new(), league_slug: String::new(),
        });
        let to = to_info.unwrap_or_else(|| TeamInfo {
            name: String::new(), slug: String::new(), reputation: 0,
            league_name: String::new(), league_slug: String::new(),
        });
        player.on_loan(&from, &to, loan_fee, date);

        clear_transfer_statuses(&mut player);

        if let Some(buying_club) = country.clubs.iter_mut().find(|c| c.id == buying_club_id) {
            let salary = (loan_fee / 50.0).max(200.0) as u32;
            // Match fee: parent club pays ~2% of the loan fee per appearance, min 500
            let match_fee = ((loan_fee * 0.02).max(500.0)) as u32;
            player.contract_loan = Some(
                PlayerClubContract::new_loan(salary, loan_end, selling_club_id, from_team_id, buying_club_id)
                    .with_loan_match_fee(match_fee)
            );

            buying_club.finance.spend_from_transfer_budget(loan_fee);
            if !buying_club.teams.teams.is_empty() {
                buying_club.teams.teams[0].players.add(player);
            }
        }

        // Remove listing and loan-out candidate so the player can't be loaned again
        country.transfer_market.complete_listings_for_player(player_id);
        if let Some(selling_club) = country.clubs.iter_mut().find(|c| c.id == selling_club_id) {
            selling_club.transfer_plan.loan_out_candidates.retain(|c| c.player_id != player_id);
        }

        debug!("Loan completed: player {} from club {} to club {} (fee: {}, match_fee: {})", player_id, selling_club_id, buying_club_id, loan_fee, ((loan_fee * 0.02).max(500.0)) as u32);
        true
    } else {
        debug!("Loan failed: player {} not found at club {}", player_id, selling_club_id);
        false
    }
}

// ============================================================
// Internal: cross-country (player moves between countries)
// ============================================================

fn take_player_from_selling_country(
    data: &mut SimulatorData,
    player_id: u32,
    selling_country_id: u32,
    selling_club_id: u32,
    fee: f64,
    is_loan: bool,
) -> Option<(Player, TeamInfo, Option<u32>)> {
    let country = data.country_mut(selling_country_id)?;

    let selling_club = country.clubs.iter_mut().find(|c| c.id == selling_club_id)?;

    let league_id = selling_club.teams.main().and_then(|t| t.league_id);

    let from_info = selling_club.teams.main()
        .map(|main_team| TeamInfo {
            name: selling_club.name.clone(),
            slug: main_team.slug.clone(),
            reputation: main_team.reputation.world,
            league_name: String::new(),
            league_slug: String::new(),
                    })?;

    // For loans: move to reserve first
    if is_loan {
        let main_idx = selling_club.teams.main_index();
        let reserve_idx = selling_club.teams.index_of_type(TeamType::Reserve)
            .or_else(|| selling_club.teams.index_of_type(TeamType::B));
        if let (Some(mi), Some(ri)) = (main_idx, reserve_idx) {
            if mi != ri {
                if let Some(p) = selling_club.teams.teams[mi].players.take_player(&player_id) {
                    selling_club.teams.teams[ri].players.add(p);
                }
            }
        }
    }

    let mut player = None;
    for team in &mut selling_club.teams.teams {
        if let Some(p) = team.players.take_player(&player_id) {
            player = Some(p);
            team.transfer_list.remove(player_id);
            break;
        }
    }

    // Only credit income when player was actually found and taken
    if player.is_some() {
        selling_club.finance.add_transfer_income(fee);
    }

    // Resolve league name
    let mut from_info = from_info;
    let (league_name, league_slug) = league_id
        .and_then(|lid| country.leagues.leagues.iter().find(|l| l.id == lid))
        .and_then(|l| if l.friendly { country.leagues.leagues.iter().find(|ml| !ml.friendly) } else { Some(l) })
        .map(|l| (l.name.clone(), l.slug.clone()))
        .unwrap_or_default();
    from_info.league_name = league_name;
    from_info.league_slug = league_slug;

    player.map(|p| (p, from_info, league_id))
}

fn execute_transfer_across_countries(
    data: &mut SimulatorData,
    player_id: u32,
    selling_country_id: u32,
    selling_club_id: u32,
    buying_country_id: u32,
    buying_club_id: u32,
    fee: f64,
    date: NaiveDate,
) -> bool {
    let taken = take_player_from_selling_country(data, player_id, selling_country_id, selling_club_id, fee, false);

    let (mut player, from_info, _) = match taken {
        Some(v) => v,
        None => {
            debug!("Transfer failed: player {} not found in country {}", player_id, selling_country_id);
            return false;
        }
    };

    let buying_country = match data.country_mut(buying_country_id) {
        Some(c) => c,
        None => return false,
    };

    // Check squad capacity BEFORE recording history
    let to_info = resolve_buying_club_info(buying_country, buying_club_id);
    let can_accept = buying_country.clubs.iter().find(|c| c.id == buying_club_id)
        .map(|c| can_club_accept_player(c))
        .unwrap_or(false);

    if !can_accept {
        debug!("Transfer rejected: club {} squad full", buying_club_id);
        if let Some(selling_country) = data.country_mut(selling_country_id) {
            if let Some(selling_club) = selling_country.clubs.iter_mut().find(|c| c.id == selling_club_id) {
                if !selling_club.teams.teams.is_empty() {
                    selling_club.teams.teams[0].players.add(player);
                }
            }
        }
        return false;
    }

    let to = to_info.unwrap_or_else(|| TeamInfo {
        name: String::new(), slug: String::new(), reputation: 0,
        league_name: String::new(), league_slug: String::new(),
    });
    player.on_transfer(&from_info, &to, fee, date);
    player.sold_from = Some((selling_club_id, fee));

    clear_transfer_statuses(&mut player);
    assign_new_contract(&mut player, fee, date, false);
    assign_signing_plan(&mut player, fee, date);

    if let Some(buying_club) = buying_country.clubs.iter_mut().find(|c| c.id == buying_club_id) {
        buying_club.finance.spend_from_transfer_budget(fee);
        if !buying_club.teams.teams.is_empty() {
            buying_club.teams.teams[0].players.add(player);
        }
    }

    debug!("Transfer completed: player {} from country {} to country {} (fee: {})", player_id, selling_country_id, buying_country_id, fee);
    true
}

fn execute_loan_across_countries(
    data: &mut SimulatorData,
    player_id: u32,
    selling_country_id: u32,
    selling_club_id: u32,
    buying_country_id: u32,
    buying_club_id: u32,
    loan_fee: f64,
    date: NaiveDate,
) -> bool {
    // Get loan end date from selling country's league before taking the player
    let selling_league_id = data.country(selling_country_id)
        .and_then(|c| c.clubs.iter().find(|cl| cl.id == selling_club_id))
        .and_then(|cl| cl.teams.main())
        .and_then(|t| t.league_id);

    let loan_end = data.country(selling_country_id)
        .map(|c| compute_loan_end(selling_league_id, c, date))
        .unwrap_or_else(|| {
            let year = if date.month() >= 6 { date.year() + 1 } else { date.year() };
            NaiveDate::from_ymd_opt(year, 5, 31).unwrap_or(date)
        });

    let taken = take_player_from_selling_country(data, player_id, selling_country_id, selling_club_id, loan_fee, true);

    let (mut player, from_info, _) = match taken {
        Some(v) => v,
        None => {
            debug!("Loan failed: player {} not found in country {}", player_id, selling_country_id);
            return false;
        }
    };

    // Ensure parent contract doesn't expire during the loan.
    ensure_parent_contract_covers_loan(&mut player, loan_end);

    let buying_country = match data.country_mut(buying_country_id) {
        Some(c) => c,
        None => return false,
    };

    // Check squad capacity BEFORE recording history
    let to_info = resolve_buying_club_info(buying_country, buying_club_id);
    let can_accept = buying_country.clubs.iter().find(|c| c.id == buying_club_id)
        .map(|c| can_club_accept_player(c))
        .unwrap_or(false);

    if !can_accept {
        debug!("Loan rejected: club {} squad full", buying_club_id);
        if let Some(selling_country) = data.country_mut(selling_country_id) {
            if let Some(selling_club) = selling_country.clubs.iter_mut().find(|c| c.id == selling_club_id) {
                if !selling_club.teams.teams.is_empty() {
                    selling_club.teams.teams[0].players.add(player);
                }
            }
        }
        return false;
    }

    let to = to_info.unwrap_or_else(|| TeamInfo {
        name: String::new(), slug: String::new(), reputation: 0,
        league_name: String::new(), league_slug: String::new(),
    });
    player.on_loan(&from_info, &to, loan_fee, date);

    clear_transfer_statuses(&mut player);

    if let Some(buying_club) = buying_country.clubs.iter_mut().find(|c| c.id == buying_club_id) {
        let salary = (loan_fee / 50.0).max(200.0) as u32;
        let match_fee = ((loan_fee * 0.02).max(500.0)) as u32;
        player.contract_loan = Some(
            PlayerClubContract::new_loan(salary, loan_end, selling_club_id, 0, buying_club_id)
                .with_loan_match_fee(match_fee)
        );

        buying_club.finance.spend_from_transfer_budget(loan_fee);
        if !buying_club.teams.teams.is_empty() {
            buying_club.teams.teams[0].players.add(player);
        }
    }

    debug!("Loan completed: player {} from country {} to country {} (fee: {})", player_id, selling_country_id, buying_country_id, loan_fee);
    true
}

// ============================================================
// Shared helpers
// ============================================================

fn resolve_buying_club_info(country: &Country, buying_club_id: u32) -> Option<TeamInfo> {
    country.clubs.iter()
        .find(|c| c.id == buying_club_id)
        .and_then(|c| {
            let main_team = c.teams.main().or(c.teams.teams.first())?;
            let (league_name, league_slug) = main_team.league_id
                .and_then(|lid| country.leagues.leagues.iter().find(|l| l.id == lid))
                .map(|l| (l.name.clone(), l.slug.clone()))
                .unwrap_or_default();
            Some(TeamInfo {
                name: main_team.name.clone(),
                slug: main_team.slug.clone(),
                reputation: main_team.reputation.world,
                league_name,
                league_slug,
            })
        })
}

fn clear_transfer_statuses(player: &mut Player) {
    player.statuses.remove(PlayerStatusType::Lst);
    player.statuses.remove(PlayerStatusType::Loa);
    player.statuses.remove(PlayerStatusType::Frt);
    player.statuses.remove(PlayerStatusType::Req);
    player.statuses.remove(PlayerStatusType::Unh);
    player.statuses.remove(PlayerStatusType::Trn);
    player.statuses.remove(PlayerStatusType::Bid);
    player.statuses.remove(PlayerStatusType::Wnt);
    player.statuses.remove(PlayerStatusType::Sct);
    player.statuses.remove(PlayerStatusType::Enq);
    player.happiness = PlayerHappiness::new();
}

fn assign_new_contract(player: &mut Player, fee: f64, date: NaiveDate, _is_loan: bool) {
    let contract_years = if player.age(date) < 24 { 5 }
    else if player.age(date) < 28 { 4 }
    else if player.age(date) < 32 { 3 }
    else { 2 };
    let expiry = date.checked_add_signed(chrono::Duration::days(contract_years * 365))
        .unwrap_or(date);
    // Salary proportional to fee: ~5% of transfer fee as annual wage, min 500
    // A 10M transfer → 500K/year, a 100K transfer → 5K/year
    let salary = (fee * 0.05).max(500.0) as u32;
    player.contract = Some(PlayerClubContract::new(salary, expiry));
    player.contract_loan = None;
}

/// Assign a signing plan to a permanently transferred player.
/// The plan captures the club's intent: the player gets a fair evaluation
/// window (time + minimum appearances) before they can be listed for sale.
/// Loans don't get plans — they're temporary by nature.
fn assign_signing_plan(player: &mut Player, fee: f64, date: NaiveDate) {
    let age = player.age(date);
    player.plan = Some(PlayerPlan::from_signing(age, fee, date));
}

/// Extend the player's parent contract so it doesn't expire during the loan.
/// If the contract would expire before `loan_end + 1 year`, push it out.
/// Safety net: the parent club will attempt a proper renewal while the player
/// is on loan, but if the player rejects, this ensures the contract still
/// covers the loan period.
fn ensure_parent_contract_covers_loan(player: &mut Player, loan_end: NaiveDate) {
    let min_expiry = loan_end
        .checked_add_signed(chrono::Duration::days(365))
        .unwrap_or(loan_end);

    if let Some(ref mut contract) = player.contract {
        if contract.expiration < min_expiry {
            debug!(
                "Extending parent contract for player {} from {} to {} (loan ends {})",
                player.id, contract.expiration, min_expiry, loan_end
            );
            contract.expiration = min_expiry;
        }
    }
}

fn compute_loan_end(league_id: Option<u32>, country: &Country, date: NaiveDate) -> NaiveDate {
    league_id
        .and_then(|lid| country.leagues.leagues.iter().find(|l| l.id == lid))
        .map(|league| {
            let end = &league.settings.season_ending_half;
            let end_month = end.to_month as u32;
            let end_day = end.to_day as u32;
            let year = if date.month() > end_month
                || (date.month() == end_month && date.day() > end_day)
            {
                date.year() + 1
            } else {
                date.year()
            };
            NaiveDate::from_ymd_opt(year, end_month, end_day).unwrap_or(date)
        })
        .unwrap_or_else(|| {
            let year = if date.month() >= 6 { date.year() + 1 } else { date.year() };
            NaiveDate::from_ymd_opt(year, 5, 31).unwrap_or(date)
        })
}
