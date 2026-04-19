use chrono::{Datelike, NaiveDate};
use log::debug;
use super::types::{can_club_accept_player, DeferredTransfer};
use crate::club::player::events::{LoanCompletion, TransferCompletion};
use crate::{Country, Player, PlayerClubContract, TeamInfo, TeamType};
use crate::simulator::SimulatorData;

/// Unified transfer execution — handles both domestic and cross-country.
/// When selling_country_id == buying_country_id it's domestic (single country).
/// When different, the player moves between countries.
/// Returns true if the player was successfully placed at the buying club.
pub(crate) fn execute_transfer(
    data: &mut SimulatorData,
    transfer: &DeferredTransfer,
    date: NaiveDate,
) -> bool {
    let player_id = transfer.player_id;
    let selling_country_id = transfer.selling_country_id;
    let selling_club_id = transfer.selling_club_id;
    let buying_country_id = transfer.buying_country_id;
    let buying_club_id = transfer.buying_club_id;
    let is_loan = transfer.is_loan;

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
                execute_loan_within_country(country, transfer, date)
            } else {
                execute_transfer_within_country(country, transfer, date)
            }
        } else {
            false
        }
    } else {
        // Cross-country — take from one country, place in another
        if is_loan {
            execute_loan_across_countries(data, transfer, date)
        } else {
            execute_transfer_across_countries(data, transfer, date)
        }
    }
}

// ============================================================
// Internal: domestic (single country)
// ============================================================

pub(crate) fn execute_transfer_within_country(
    country: &mut Country,
    transfer: &DeferredTransfer,
    date: NaiveDate,
) -> bool {
    let player_id = transfer.player_id;
    let selling_club_id = transfer.selling_club_id;
    let buying_club_id = transfer.buying_club_id;
    let fee = transfer.fee;
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
        // Drain existing sell-on obligations now — they pay previous
        // beneficiaries out of the selling club's proceeds on this sale.
        let obligations = player.drain_sell_on_obligations();
        player.complete_transfer(TransferCompletion {
            from: &from,
            to: &to,
            fee,
            date,
            selling_club_id,
            buying_club_id,
            agreed_wage: transfer.agreed_annual_wage,
            buying_league_reputation: transfer.buying_league_reputation,
            record_sell_on: transfer.sell_on_percentage,
        });

        for obligation in &obligations {
            let payout = fee * obligation.percentage as f64;
            if payout <= 0.0 { continue; }
            if let Some(beneficiary) = country
                .clubs
                .iter_mut()
                .find(|c| c.id == obligation.beneficiary_club_id)
            {
                beneficiary.finance.add_transfer_income(payout);
            }
            if let Some(seller) = country
                .clubs
                .iter_mut()
                .find(|c| c.id == selling_club_id)
            {
                seller.finance.add_transfer_income(-payout);
            }
        }

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
    transfer: &DeferredTransfer,
    date: NaiveDate,
) -> bool {
    let player_id = transfer.player_id;
    let selling_club_id = transfer.selling_club_id;
    let buying_club_id = transfer.buying_club_id;
    let loan_fee = transfer.fee;
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

        player.ensure_contract_covers_loan_end(loan_end);

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
        let loan_contract = build_loan_contract(
            loan_fee,
            loan_end,
            selling_club_id,
            from_team_id,
            buying_club_id,
            &player,
            transfer.has_option_to_buy,
            transfer.agreed_annual_wage,
        );
        player.complete_loan(LoanCompletion {
            from: &from,
            to: &to,
            loan_fee,
            date,
            loan_contract,
            borrowing_club_id: buying_club_id,
        });

        if let Some(buying_club) = country.clubs.iter_mut().find(|c| c.id == buying_club_id) {
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

        debug!("Loan completed: player {} from club {} to club {}", player_id, selling_club_id, buying_club_id);
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
) -> Option<(Player, TeamInfo, Option<u32>, u32)> {
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
    let mut parent_team_id: u32 = 0;
    for team in &mut selling_club.teams.teams {
        if let Some(p) = team.players.take_player(&player_id) {
            parent_team_id = team.id;
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

    player.map(|p| (p, from_info, league_id, parent_team_id))
}

fn execute_transfer_across_countries(
    data: &mut SimulatorData,
    transfer: &DeferredTransfer,
    date: NaiveDate,
) -> bool {
    let player_id = transfer.player_id;
    let selling_country_id = transfer.selling_country_id;
    let selling_club_id = transfer.selling_club_id;
    let buying_country_id = transfer.buying_country_id;
    let buying_club_id = transfer.buying_club_id;
    let fee = transfer.fee;
    let taken = take_player_from_selling_country(data, player_id, selling_country_id, selling_club_id, fee, false);

    let (mut player, from_info, _, _) = match taken {
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
    // Drain sell-on obligations for cross-country. The beneficiaries may
    // live in a different country from the current seller, so we hold the
    // drained list and settle after returning the player into the buyer
    // country — routing goes via `data.country_mut` lookups.
    let obligations = player.drain_sell_on_obligations();
    player.complete_transfer(TransferCompletion {
        from: &from_info,
        to: &to,
        fee,
        date,
        selling_club_id,
        buying_club_id,
        agreed_wage: transfer.agreed_annual_wage,
        buying_league_reputation: transfer.buying_league_reputation,
        record_sell_on: transfer.sell_on_percentage,
    });

    if let Some(buying_club) = buying_country.clubs.iter_mut().find(|c| c.id == buying_club_id) {
        buying_club.finance.spend_from_transfer_budget(fee);
        if !buying_club.teams.teams.is_empty() {
            buying_club.teams.teams[0].players.add(player);
        }
    }

    // Settle obligations across countries: locate each beneficiary globally
    // and credit them. The seller's finance was already incremented by the
    // full fee in `take_player_from_selling_country`, so we debit the share
    // from the seller too.
    for obligation in &obligations {
        let payout = fee * obligation.percentage as f64;
        if payout <= 0.0 { continue; }
        credit_club_globally(data, obligation.beneficiary_club_id, payout);
        credit_club_globally(data, selling_club_id, -payout);
    }

    debug!("Transfer completed: player {} from country {} to country {} (fee: {})", player_id, selling_country_id, buying_country_id, fee);
    true
}

/// Locate a club anywhere in the simulator and add `amount` to their finance
/// balance. Used for cross-country sell-on routing where the beneficiary
/// sits in a different country from the selling club.
fn credit_club_globally(data: &mut SimulatorData, club_id: u32, amount: f64) {
    for continent in data.continents.iter_mut() {
        for country in continent.countries.iter_mut() {
            if let Some(club) = country.clubs.iter_mut().find(|c| c.id == club_id) {
                club.finance.add_transfer_income(amount);
                return;
            }
        }
    }
}

fn execute_loan_across_countries(
    data: &mut SimulatorData,
    transfer: &DeferredTransfer,
    date: NaiveDate,
) -> bool {
    let player_id = transfer.player_id;
    let selling_country_id = transfer.selling_country_id;
    let selling_club_id = transfer.selling_club_id;
    let buying_country_id = transfer.buying_country_id;
    let buying_club_id = transfer.buying_club_id;
    let loan_fee = transfer.fee;
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

    let (mut player, from_info, _, parent_team_id) = match taken {
        Some(v) => v,
        None => {
            debug!("Loan failed: player {} not found in country {}", player_id, selling_country_id);
            return false;
        }
    };

    player.ensure_contract_covers_loan_end(loan_end);

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
    let loan_contract = build_loan_contract(
        loan_fee,
        loan_end,
        selling_club_id,
        parent_team_id,
        buying_club_id,
        &player,
        transfer.has_option_to_buy,
        transfer.agreed_annual_wage,
    );
    player.complete_loan(LoanCompletion {
        from: &from_info,
        to: &to,
        loan_fee,
        date,
        loan_contract,
        borrowing_club_id: buying_club_id,
    });

    if let Some(buying_club) = buying_country.clubs.iter_mut().find(|c| c.id == buying_club_id) {
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

fn build_loan_contract(
    _loan_fee: f64,
    loan_end: NaiveDate,
    parent_club_id: u32,
    parent_team_id: u32,
    buying_club_id: u32,
    player: &Player,
    _has_option_to_buy: bool,
    agreed_parent_wage: Option<u32>,
) -> PlayerClubContract {
    // Parent wage drives the loan split: borrower covers the majority,
    // parent keeps paying the rest. Falls back to the player's current
    // contract salary when the pipeline didn't stage an explicit wage.
    let parent_wage = agreed_parent_wage
        .or_else(|| player.contract.as_ref().map(|c| c.salary))
        .unwrap_or(1_000);
    let (borrower_wage, match_fee) =
        crate::club::player::calculators::WageCalculator::loan_wage_split(parent_wage);
    PlayerClubContract::new_loan(borrower_wage, loan_end, parent_club_id, parent_team_id, buying_club_id)
        .with_loan_match_fee(match_fee)
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
