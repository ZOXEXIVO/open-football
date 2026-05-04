use super::types::{DeferredTransfer, can_club_accept_player};
use crate::club::Person;
use crate::club::player::events::{LoanCompletion, TransferCompletion};
use crate::club::player::language::Language;
use crate::simulator::SimulatorData;
use crate::transfers::pipeline::PipelineProcessor;
use crate::{Country, Player, PlayerClubContract, TeamInfo, TeamType};
use chrono::{Datelike, NaiveDate};
use log::debug;

/// Default contract length used to amortize a transfer fee on the buying
/// club's P&L when a more specific length isn't available at execution
/// time. Matches the IFRS football-finance norm.
const DEFAULT_AMORTIZATION_YEARS: u8 = 4;

/// Snapshot of a departing player's traits captured BEFORE they leave the
/// selling club. Drives the per-teammate social events (close-friend lost,
/// mentor departed) that fire on the leftover squad.
#[derive(Debug, Clone)]
struct DepartingPlayerInfo {
    id: u32,
    age: u8,
    country_id: u32,
    high_reputation: bool,
}

/// True if the country's primary language(s) are met at functional fluency
/// (proficiency >= 70). Used to gate CompatriotJoined: an integration boost
/// from a same-nationality teammate matters most when the new arrival is
/// linguistically isolated.
fn speaks_local_language(player: &Player, country_code: &str) -> bool {
    let langs = Language::from_country_code(country_code);
    if langs.is_empty() {
        return true;
    }
    langs.iter().any(|l| {
        player
            .languages
            .iter()
            .any(|pl| pl.language == *l && (pl.is_native || pl.proficiency >= 70))
    })
}

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
        debug!(
            "Blocked self-transfer: club {} tried to {} player {} to itself",
            selling_club_id,
            if is_loan { "loan" } else { "transfer" },
            player_id
        );
        return false;
    }

    // Safety: can't loan a player who is already on loan
    if is_loan {
        let already_on_loan = data
            .player(player_id)
            .map(|p| p.is_on_loan())
            .unwrap_or(false);
        if already_on_loan {
            debug!("Blocked re-loan: player {} is already on loan", player_id);
            return false;
        }
    }
    let success = if selling_country_id == buying_country_id {
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
    };

    // Once the player has actually moved, sweep stale transfer interest
    // (scouting, shortlists, monitoring, listings, pending negotiations)
    // across every country — not just the buying country. The negotiation
    // acceptance path already calls `clear_player_interest` on the owning
    // country, but clubs elsewhere keep stale rows until this cleanup.
    if success {
        PipelineProcessor::cleanup_player_transfer_interest(data, player_id);
    }
    success
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

    // Capture departing player's social traits BEFORE removal. Used by
    // the post-move pass to emit per-teammate CloseFriendSold /
    // MentorDeparted events on the leftover squad.
    let departing: Option<DepartingPlayerInfo> = country
        .clubs
        .iter()
        .find(|c| c.id == selling_club_id)
        .and_then(|club| {
            club.teams.iter().find_map(|t| {
                t.players.iter().find(|p| p.id == player_id).map(|p| {
                    DepartingPlayerInfo {
                        id: p.id,
                        age: p.age(date),
                        country_id: p.country_id,
                        // 7000+ world rep is a "household name" threshold.
                        high_reputation: p.player_attributes.world_reputation >= 7000,
                    }
                })
            })
        });

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

        // Emit per-teammate dressing-room events on the leftover squad.
        // Done while we still hold the selling_club mut-borrow but after
        // the departing player has been removed — so we're iterating
        // remaining teammates only.
        if let Some(info) = &departing {
            for team in &mut selling_club.teams.teams {
                for teammate in team.players.iter_mut() {
                    let bond = match teammate.relations.get_player(info.id) {
                        Some(rel) => rel.friendship,
                        None => continue,
                    };
                    let same_nat = teammate.country_id == info.country_id;
                    let teammate_age = teammate.age(date);

                    // Mentor departure: a veteran (30+) leaving a young
                    // (<= 23) teammate with a strong bond. Single event,
                    // not also CloseFriendSold — mentorship is the more
                    // specific framing.
                    let is_mentor_break = info.age >= 30 && teammate_age <= 23 && bond >= 55.0;

                    if is_mentor_break {
                        teammate.on_mentor_departed(info.id, bond, same_nat);
                    } else if bond >= 65.0 {
                        teammate.on_close_friend_sold(
                            info.id,
                            bond,
                            same_nat,
                            info.high_reputation,
                        );
                    }
                }
            }
        }
    }

    if let Some(mut player) = player {
        if let Some(ref mut from) = from_info {
            let (league_name, league_slug) =
                resolve_selling_league_labels(country, selling_league_id);
            from.league_name = league_name;
            from.league_slug = league_slug;
        }

        // Check squad capacity BEFORE recording history — otherwise a rejected
        // transfer creates a phantom career entry with no matching transfer record
        let to_info = resolve_buying_club_info(country, buying_club_id);
        let can_accept = country
            .clubs
            .iter()
            .find(|c| c.id == buying_club_id)
            .map(|c| can_club_accept_player(c))
            .unwrap_or(false);

        if !can_accept {
            debug!(
                "Transfer rejected: club {} squad full, returning player {}",
                buying_club_id, player_id
            );
            if let Some(selling_club) = country.clubs.iter_mut().find(|c| c.id == selling_club_id) {
                if !selling_club.teams.teams.is_empty() {
                    selling_club.teams.teams[0].players.add(player);
                }
                selling_club.finance.add_transfer_income(-fee);
            }
            return false;
        }

        // Always record history — use fallback TeamInfo if club info couldn't be resolved
        let from = from_info.unwrap_or_else(empty_team_info);
        let to = to_info.unwrap_or_else(empty_team_info);
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
            if payout <= 0.0 {
                continue;
            }
            if let Some(beneficiary) = country
                .clubs
                .iter_mut()
                .find(|c| c.id == obligation.beneficiary_club_id)
            {
                beneficiary.finance.add_transfer_income(payout);
            }
            if let Some(seller) = country.clubs.iter_mut().find(|c| c.id == selling_club_id) {
                seller.finance.add_transfer_income(-payout);
            }
        }

        // The arriving player's nationality is needed for the post-move
        // CompatriotJoined pass; capture before move-out borrows.
        let arrival_country_id = player.country_id;
        let club_country_id = country.id;
        let club_country_code = country.code.clone();

        if let Some(buying_club) = country.clubs.iter_mut().find(|c| c.id == buying_club_id) {
            // Cash leaves immediately, P&L spread across DEFAULT_AMORTIZATION_YEARS.
            buying_club
                .finance
                .register_transfer_purchase(fee, DEFAULT_AMORTIZATION_YEARS);
            if !buying_club.teams.teams.is_empty() {
                buying_club.teams.teams[0].players.add(player);
            }

            // Compatriot pass on the buying club's existing roster: any
            // same-nationality teammate gets the integration boost. Skip
            // the new arrival themselves (they're at the front of the list
            // we just pushed onto). The check `id != player_id` is enough.
            //
            // We also count whether at least one same-nationality teammate
            // exists, so the arriving player can fire `CompatriotJoined`
            // themselves — the integration goes both ways. Domestic moves
            // where everyone already shares the local nationality are
            // gated out by `on_compatriot_joined` itself
            // (`country_id == club_country_id` early-returns).
            let mut compatriot_present = false;
            for team in &mut buying_club.teams.teams {
                for existing in team.players.iter_mut() {
                    if existing.id == player_id {
                        continue;
                    }
                    if existing.country_id != arrival_country_id {
                        continue;
                    }
                    compatriot_present = true;
                    let lacks_lang = !speaks_local_language(existing, &club_country_code);
                    existing.on_compatriot_joined(player_id, club_country_id, lacks_lang);
                }
            }
            // Reverse pass: fire on the arrival if compatriots exist.
            // Tag with one of the existing compatriot ids so the link
            // points at a real teammate; pick the first one we find.
            if compatriot_present {
                let mut a_compatriot_id: Option<u32> = None;
                for team in &buying_club.teams.teams {
                    if let Some(found) = team
                        .players
                        .players
                        .iter()
                        .find(|p| p.id != player_id && p.country_id == arrival_country_id)
                    {
                        a_compatriot_id = Some(found.id);
                        break;
                    }
                }
                if let Some(compatriot_id) = a_compatriot_id {
                    for team in &mut buying_club.teams.teams {
                        if let Some(arrival) = team.players.iter_mut().find(|p| p.id == player_id) {
                            let lacks_lang = !speaks_local_language(arrival, &club_country_code);
                            arrival.on_compatriot_joined(
                                compatriot_id,
                                club_country_id,
                                lacks_lang,
                            );
                            break;
                        }
                    }
                }
            }
        }

        country
            .transfer_market
            .complete_listings_for_player(player_id);
        if let Some(selling_club) = country.clubs.iter_mut().find(|c| c.id == selling_club_id) {
            selling_club
                .transfer_plan
                .loan_out_candidates
                .retain(|c| c.player_id != player_id);
        }

        debug!(
            "Transfer completed: player {} from club {} to club {} for {}",
            player_id, selling_club_id, buying_club_id, fee
        );
        true
    } else {
        debug!(
            "Transfer failed: player {} not found at club {}",
            player_id, selling_club_id
        );
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

        from_team_id = selling_club
            .teams
            .find_team_with_player(player_id)
            .map(|t| t.id)
            .unwrap_or(0);

        // Move to reserve before loaning
        let main_idx = selling_club.teams.main_index();
        let reserve_idx = selling_club
            .teams
            .index_of_type(TeamType::Reserve)
            .or_else(|| selling_club.teams.index_of_type(TeamType::B))
            .or_else(|| selling_club.teams.index_of_type(TeamType::Second));

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
            selling_club.finance.receive_loan_fee(loan_fee);
        }
    }

    if let Some(mut player) = player {
        if let Some(ref mut from) = from_info {
            let (league_name, league_slug) =
                resolve_selling_league_labels(country, selling_league_id);
            from.league_name = league_name;
            from.league_slug = league_slug;
        }

        let loan_end = compute_loan_end(selling_league_id, country, date);

        player.ensure_contract_covers_loan_end(loan_end);

        // Check squad capacity BEFORE recording history — otherwise a rejected
        // loan creates a phantom career entry with no matching transfer record
        let to_info = resolve_buying_club_info(country, buying_club_id);
        let can_accept = country
            .clubs
            .iter()
            .find(|c| c.id == buying_club_id)
            .map(|c| can_club_accept_player(c))
            .unwrap_or(false);

        if !can_accept {
            debug!(
                "Loan rejected: club {} squad full, returning player {}",
                buying_club_id, player_id
            );
            if let Some(selling_club) = country.clubs.iter_mut().find(|c| c.id == selling_club_id) {
                if !selling_club.teams.teams.is_empty() {
                    selling_club.teams.teams[0].players.add(player);
                }
                selling_club.finance.refund_loan_fee(loan_fee);
            }
            return false;
        }

        // Always record history — use fallback TeamInfo if club info couldn't be resolved
        let from = from_info.unwrap_or_else(empty_team_info);
        let to = to_info.unwrap_or_else(empty_team_info);
        let borrower_score = country
            .clubs
            .iter()
            .find(|c| c.id == buying_club_id)
            .and_then(|c| c.teams.main())
            .map(|t| t.reputation.world as f32 / 10_000.0)
            .unwrap_or(0.4);
        // Parent develops loanees more aggressively if the player is
        // young or has high potential.
        let parent_desire =
            if player.age(date) <= 22 || player.player_attributes.potential_ability >= 130 {
                0.7
            } else {
                0.3
            };
        let loan_contract = build_loan_contract(
            loan_fee,
            loan_end,
            selling_club_id,
            from_team_id,
            buying_club_id,
            &player,
            transfer.has_option_to_buy,
            transfer.agreed_annual_wage,
            transfer.loan_future_fee,
            borrower_score,
            parent_desire,
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
            buying_club.finance.pay_loan_fee(loan_fee);
            if !buying_club.teams.teams.is_empty() {
                buying_club.teams.teams[0].players.add(player);
            }
        }

        // Remove listing and loan-out candidate so the player can't be loaned again
        country
            .transfer_market
            .complete_listings_for_player(player_id);
        if let Some(selling_club) = country.clubs.iter_mut().find(|c| c.id == selling_club_id) {
            selling_club
                .transfer_plan
                .loan_out_candidates
                .retain(|c| c.player_id != player_id);
        }

        debug!(
            "Loan completed: player {} from club {} to club {}",
            player_id, selling_club_id, buying_club_id
        );
        true
    } else {
        debug!(
            "Loan failed: player {} not found at club {}",
            player_id, selling_club_id
        );
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

    let from_info = selling_club.teams.main().map(|main_team| TeamInfo {
        name: selling_club.name.clone(),
        slug: main_team.slug.clone(),
        reputation: main_team.reputation.world,
        league_name: String::new(),
        league_slug: String::new(),
    })?;

    // For loans: move to reserve first
    if is_loan {
        let main_idx = selling_club.teams.main_index();
        let reserve_idx = selling_club
            .teams
            .index_of_type(TeamType::Reserve)
            .or_else(|| selling_club.teams.index_of_type(TeamType::B))
            .or_else(|| selling_club.teams.index_of_type(TeamType::Second));
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
        if is_loan {
            selling_club.finance.receive_loan_fee(fee);
        } else {
            selling_club.finance.add_transfer_income(fee);
        }
    }

    // Resolve league name
    let mut from_info = from_info;
    let (league_name, league_slug) = resolve_selling_league_labels(country, league_id);
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

    let can_accept = data
        .country(buying_country_id)
        .and_then(|c| c.clubs.iter().find(|club| club.id == buying_club_id))
        .map(can_club_accept_player)
        .unwrap_or(false);
    if !can_accept {
        debug!(
            "Transfer rejected before mutation: club {} cannot accept player {}",
            buying_club_id, player_id
        );
        return false;
    }

    // Snapshot the departing player's social traits BEFORE removal so the
    // selling-country teammates can be ticked with CloseFriendSold /
    // MentorDeparted. Same shape as the within-country path, just routed
    // via SimulatorData since the player's home country is foreign here.
    let departing: Option<DepartingPlayerInfo> = data
        .country(selling_country_id)
        .and_then(|c| c.clubs.iter().find(|club| club.id == selling_club_id))
        .and_then(|club| {
            club.teams.iter().find_map(|t| {
                t.players
                    .iter()
                    .find(|p| p.id == player_id)
                    .map(|p| DepartingPlayerInfo {
                        id: p.id,
                        age: p.age(date),
                        country_id: p.country_id,
                        high_reputation: p.player_attributes.world_reputation >= 7000,
                    })
            })
        });

    let taken = take_player_from_selling_country(
        data,
        player_id,
        selling_country_id,
        selling_club_id,
        fee,
        false,
    );

    let (mut player, from_info, _, _) = match taken {
        Some(v) => v,
        None => {
            debug!(
                "Transfer failed: player {} not found in country {}",
                player_id, selling_country_id
            );
            return false;
        }
    };

    // Selling-side dressing-room pass: the player has been taken out of
    // the squad, the remaining teammates feel the departure.
    if let Some(info) = &departing {
        if let Some(country) = data.country_mut(selling_country_id) {
            if let Some(selling_club) = country.clubs.iter_mut().find(|c| c.id == selling_club_id) {
                for team in &mut selling_club.teams.teams {
                    for teammate in team.players.iter_mut() {
                        let bond = match teammate.relations.get_player(info.id) {
                            Some(rel) => rel.friendship,
                            None => continue,
                        };
                        let same_nat = teammate.country_id == info.country_id;
                        let teammate_age = teammate.age(date);
                        let is_mentor_break = info.age >= 30 && teammate_age <= 23 && bond >= 55.0;
                        if is_mentor_break {
                            teammate.on_mentor_departed(info.id, bond, same_nat);
                        } else if bond >= 65.0 {
                            teammate.on_close_friend_sold(
                                info.id,
                                bond,
                                same_nat,
                                info.high_reputation,
                            );
                        }
                    }
                }
            }
        }
    }

    let buying_country = match data.country_mut(buying_country_id) {
        Some(c) => c,
        None => {
            return_player_to_selling_country(
                data,
                selling_country_id,
                selling_club_id,
                player,
                fee,
                false,
            );
            return false;
        }
    };

    let to_info = resolve_buying_club_info(buying_country, buying_club_id);

    let to = to_info.unwrap_or_else(empty_team_info);
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

    let arrival_country_id = player.country_id;
    let buying_country_code = buying_country.code.clone();
    let buying_country_id_local = buying_country.id;

    if let Some(buying_club) = buying_country
        .clubs
        .iter_mut()
        .find(|c| c.id == buying_club_id)
    {
        buying_club
            .finance
            .register_transfer_purchase(fee, DEFAULT_AMORTIZATION_YEARS);
        if !buying_club.teams.teams.is_empty() {
            buying_club.teams.teams[0].players.add(player);
        }

        // Compatriot integration pass — same shape as the within-country
        // path, but the player has just stepped off a flight rather than
        // a coach across town. Existing same-nationality teammates feel
        // the lift; the arriving player gets the reciprocal boost if at
        // least one compatriot already plays here.
        let mut compatriot_present = false;
        for team in &mut buying_club.teams.teams {
            for existing in team.players.iter_mut() {
                if existing.id == player_id {
                    continue;
                }
                if existing.country_id != arrival_country_id {
                    continue;
                }
                compatriot_present = true;
                let lacks_lang = !speaks_local_language(existing, &buying_country_code);
                existing.on_compatriot_joined(player_id, buying_country_id_local, lacks_lang);
            }
        }
        if compatriot_present {
            // Tag the arrival's reciprocal event with one of the existing
            // compatriots so the link in the events page resolves to a
            // real teammate.
            let mut a_compatriot_id: Option<u32> = None;
            for team in &buying_club.teams.teams {
                if let Some(found) = team
                    .players
                    .players
                    .iter()
                    .find(|p| p.id != player_id && p.country_id == arrival_country_id)
                {
                    a_compatriot_id = Some(found.id);
                    break;
                }
            }
            if let Some(compatriot_id) = a_compatriot_id {
                for team in &mut buying_club.teams.teams {
                    if let Some(arrival) = team.players.iter_mut().find(|p| p.id == player_id) {
                        let lacks_lang = !speaks_local_language(arrival, &buying_country_code);
                        arrival.on_compatriot_joined(
                            compatriot_id,
                            buying_country_id_local,
                            lacks_lang,
                        );
                        break;
                    }
                }
            }
        }
    }

    // Settle obligations across countries: locate each beneficiary globally
    // and credit them. The seller's finance was already incremented by the
    // full fee in `take_player_from_selling_country`, so we debit the share
    // from the seller too.
    for obligation in &obligations {
        let payout = fee * obligation.percentage as f64;
        if payout <= 0.0 {
            continue;
        }
        credit_club_globally(data, obligation.beneficiary_club_id, payout);
        credit_club_globally(data, selling_club_id, -payout);
    }

    debug!(
        "Transfer completed: player {} from country {} to country {} (fee: {})",
        player_id, selling_country_id, buying_country_id, fee
    );
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

    let can_accept = data
        .country(buying_country_id)
        .and_then(|c| c.clubs.iter().find(|club| club.id == buying_club_id))
        .map(can_club_accept_player)
        .unwrap_or(false);
    if !can_accept {
        debug!(
            "Loan rejected before mutation: club {} cannot accept player {}",
            buying_club_id, player_id
        );
        return false;
    }

    // Get loan end date from selling country's league before taking the player
    let selling_league_id = data
        .country(selling_country_id)
        .and_then(|c| c.clubs.iter().find(|cl| cl.id == selling_club_id))
        .and_then(|cl| cl.teams.main())
        .and_then(|t| t.league_id);

    let loan_end = data
        .country(selling_country_id)
        .map(|c| compute_loan_end(selling_league_id, c, date))
        .unwrap_or_else(|| {
            let year = if date.month() >= 6 {
                date.year() + 1
            } else {
                date.year()
            };
            NaiveDate::from_ymd_opt(year, 5, 31).unwrap_or(date)
        });

    let taken = take_player_from_selling_country(
        data,
        player_id,
        selling_country_id,
        selling_club_id,
        loan_fee,
        true,
    );

    let (mut player, from_info, _, parent_team_id) = match taken {
        Some(v) => v,
        None => {
            debug!(
                "Loan failed: player {} not found in country {}",
                player_id, selling_country_id
            );
            return false;
        }
    };

    player.ensure_contract_covers_loan_end(loan_end);

    let buying_country = match data.country_mut(buying_country_id) {
        Some(c) => c,
        None => {
            return_player_to_selling_country(
                data,
                selling_country_id,
                selling_club_id,
                player,
                loan_fee,
                true,
            );
            return false;
        }
    };

    let to_info = resolve_buying_club_info(buying_country, buying_club_id);

    let to = to_info.unwrap_or_else(empty_team_info);
    let borrower_score = buying_country
        .clubs
        .iter()
        .find(|c| c.id == buying_club_id)
        .and_then(|c| c.teams.main())
        .map(|t| t.reputation.world as f32 / 10_000.0)
        .unwrap_or(0.4);
    let parent_desire =
        if player.age(date) <= 22 || player.player_attributes.potential_ability >= 130 {
            0.7
        } else {
            0.3
        };
    let loan_contract = build_loan_contract(
        loan_fee,
        loan_end,
        selling_club_id,
        parent_team_id,
        buying_club_id,
        &player,
        transfer.has_option_to_buy,
        transfer.agreed_annual_wage,
        transfer.loan_future_fee,
        borrower_score,
        parent_desire,
    );
    player.complete_loan(LoanCompletion {
        from: &from_info,
        to: &to,
        loan_fee,
        date,
        loan_contract,
        borrowing_club_id: buying_club_id,
    });

    if let Some(buying_club) = buying_country
        .clubs
        .iter_mut()
        .find(|c| c.id == buying_club_id)
    {
        buying_club.finance.pay_loan_fee(loan_fee);
        if !buying_club.teams.teams.is_empty() {
            buying_club.teams.teams[0].players.add(player);
        }
    }

    debug!(
        "Loan completed: player {} from country {} to country {} (fee: {})",
        player_id, selling_country_id, buying_country_id, loan_fee
    );
    true
}

// ============================================================
// Shared helpers
// ============================================================

/// Empty `TeamInfo` placeholder used as a fallback when club / league
/// lookup fails partway through execution. Centralised so changes to
/// `TeamInfo`'s shape don't have to be mirrored across four call sites.
fn empty_team_info() -> TeamInfo {
    TeamInfo {
        name: String::new(),
        slug: String::new(),
        reputation: 0,
        league_name: String::new(),
        league_slug: String::new(),
    }
}

/// Resolve `(league_name, league_slug)` for the selling side of a transfer.
/// Friendly leagues (preseason / exhibition fixtures) don't represent the
/// player's actual competitive context, so we fall back to the country's
/// first non-friendly league instead. Used to populate the `from.league_*`
/// fields on `TeamInfo` once the player has been taken.
fn resolve_selling_league_labels(
    country: &Country,
    selling_league_id: Option<u32>,
) -> (String, String) {
    selling_league_id
        .and_then(|lid| country.leagues.leagues.iter().find(|l| l.id == lid))
        .and_then(|l| {
            if l.friendly {
                country.leagues.leagues.iter().find(|ml| !ml.friendly)
            } else {
                Some(l)
            }
        })
        .map(|l| (l.name.clone(), l.slug.clone()))
        .unwrap_or_default()
}

fn resolve_buying_club_info(country: &Country, buying_club_id: u32) -> Option<TeamInfo> {
    country
        .clubs
        .iter()
        .find(|c| c.id == buying_club_id)
        .and_then(|c| {
            let main_team = c.teams.main().or(c.teams.teams.first())?;
            let (league_name, league_slug) = main_team
                .league_id
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
    loan_future_fee: Option<(u32, bool)>,
    borrower_score: f32,
    parent_desire_to_develop: f32,
) -> PlayerClubContract {
    // Parent wage drives the loan split: borrower covers the majority,
    // parent keeps paying the rest. Falls back to the player's current
    // contract salary when the pipeline didn't stage an explicit wage.
    let parent_wage = agreed_parent_wage
        .or_else(|| player.contract.as_ref().map(|c| c.salary))
        .unwrap_or(1_000);
    // V2 split scales borrower share with their reputation/appetite and
    // softens it when the parent is loaning the player out for development
    // (a small parent club won't subsidise a Premier League borrower).
    let (borrower_wage, match_fee) =
        crate::club::player::calculators::WageCalculator::loan_wage_split_v2(
            parent_wage,
            borrower_score,
            parent_desire_to_develop,
        );

    // Wage-contribution percentage = borrower share of total. Computed
    // back from borrower_wage / parent_wage so it stays consistent with
    // the split helper. Capped at 100; floored at 0.
    let contribution_pct = ((borrower_wage as f64 / parent_wage.max(1) as f64) * 100.0)
        .round()
        .clamp(0.0, 100.0) as u8;

    // Minimum-appearances scales with borrower size: bigger borrowers
    // promised the parent more minutes; small borrowers can't commit.
    let min_apps = if borrower_score >= 0.7 {
        15
    } else if borrower_score >= 0.4 {
        10
    } else {
        6
    };

    let mut contract = PlayerClubContract::new_loan(
        borrower_wage,
        loan_end,
        parent_club_id,
        parent_team_id,
        buying_club_id,
    )
    .with_loan_match_fee(match_fee)
    .with_loan_wage_contribution(contribution_pct)
    .with_loan_recall(
        loan_end
            .checked_sub_signed(chrono::Duration::days(90))
            .unwrap_or(loan_end),
    )
    .with_loan_min_appearances(min_apps);

    if let Some((future_fee, obligation)) = loan_future_fee {
        contract = contract.with_loan_future_fee(future_fee, obligation);
    }

    contract
}

fn return_player_to_selling_country(
    data: &mut SimulatorData,
    selling_country_id: u32,
    selling_club_id: u32,
    player: Player,
    credited_fee: f64,
    is_loan: bool,
) {
    if let Some(selling_country) = data.country_mut(selling_country_id) {
        if let Some(selling_club) = selling_country
            .clubs
            .iter_mut()
            .find(|c| c.id == selling_club_id)
        {
            if !selling_club.teams.teams.is_empty() {
                selling_club.teams.teams[0].players.add(player);
            }
            if is_loan {
                selling_club.finance.refund_loan_fee(credited_fee);
            } else {
                selling_club.finance.add_transfer_income(-credited_fee);
            }
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
            let year = if date.month() >= 6 {
                date.year() + 1
            } else {
                date.year()
            };
            NaiveDate::from_ymd_opt(year, 5, 31).unwrap_or(date)
        })
}
