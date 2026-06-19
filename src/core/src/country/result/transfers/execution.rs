use super::types::{DeferredTransfer, can_club_accept_player};
use crate::club::Person;
use crate::club::player::calculators::WageCalculator;
use crate::club::player::events::{LoanCompletion, TransferCompletion};
use crate::club::player::language::Language;
use crate::simulator::SimulatorData;
use crate::transfers::TransferRoutePolicy;
use crate::transfers::TransferWindowManager;
use crate::transfers::market::{ClauseTrigger, TransferMarket};
use crate::transfers::offer::{PersonalTermsOffer, PromisedSquadStatus, TransferClause};
use crate::transfers::pipeline::{
    LoanOutCandidate, LoanOutReason, LoanOutStatus, PipelineProcessor,
};
use crate::{
    Club, ClubPhilosophy, Country, Player, PlayerClubContract, PlayerFieldPositionGroup,
    PlayerPlanRole, PlayerSquadStatus, ReputationLevel, TeamInfo, TeamType,
};
use chrono::Duration;
use chrono::{Datelike, NaiveDate};
use log::debug;

/// Default contract length used to amortize a transfer fee on the buying
/// club's P&L when a more specific length isn't available at execution
/// time. Matches the IFRS football-finance norm.
const DEFAULT_AMORTIZATION_YEARS: u8 = 4;

/// Stateless helpers for the transfer execution path — roster placement
/// and fee-structure math. Grouped on a unit struct (rather than free
/// functions) so the execution surface reads as one discoverable namespace.
struct TransferExecution;

impl TransferExecution {
    /// Add a player to a club's **Main** team. Resolves the main team by
    /// type (the main team is NOT guaranteed to be `teams[0]`); falls back
    /// to the first team only when no Main team exists so a signing is
    /// never silently dropped. Replaces the historical `teams.teams[0]`
    /// inserts, which landed arrivals on whatever squad happened to be
    /// first in the collection.
    fn add_to_main_team(club: &mut Club, player: Player) {
        let idx = club.teams.main_index().unwrap_or(0);
        if let Some(team) = club.teams.teams.get_mut(idx) {
            team.players.add(player);
        }
    }

    /// Upfront cash portion of a permanent transfer fee after carving out
    /// any installment tranches. Installments are a *payment structure*,
    /// not an additional surcharge: the headline `fee` is split into an
    /// upfront payment now plus deferred tranches that the daily settlement
    /// walk pays buyer → seller over time. Without this split the buyer
    /// paid the full fee at completion AND the tranches on top (~155% of
    /// the headline) — money creation on every structured deal. Loans (no
    /// installments) return the full fee unchanged.
    fn upfront_fee(transfer: &DeferredTransfer) -> f64 {
        let fee = transfer.fee.max(0.0);
        // Loans carry no installments; cross-country deals pay the fee
        // upfront because the daily settlement walk runs on the buyer's
        // country market and can't route a deferred tranche to a foreign
        // seller (mirrors how cross-country sell-ons settle upfront).
        if transfer.is_loan || transfer.selling_country_id != transfer.buying_country_id {
            return fee;
        }
        let deferred: f64 = transfer
            .offer_clauses
            .iter()
            .filter_map(|clause| match clause {
                TransferClause::Installments(amount, years) if *years > 0 => {
                    Some(amount.amount.max(0.0))
                }
                _ => None,
            })
            .sum();
        (fee - deferred.min(fee)).max(0.0)
    }
}

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

/// Snapshot of the post-transfer profile of an arriving player. Used by
/// the new-signing-threat pass on the buying club's existing roster so
/// the cross-country and within-country paths share one detection /
/// emit shape.
#[derive(Debug, Clone)]
struct ArrivalThreatProfile {
    player_id: u32,
    position_group: crate::PlayerFieldPositionGroup,
    ability: u8,
    age: u8,
    squad_status: PlayerSquadStatus,
    wage: u32,
}

impl ArrivalThreatProfile {
    fn from_player(player: &Player, date: NaiveDate) -> Self {
        Self {
            player_id: player.id,
            position_group: player.position().position_group(),
            ability: player.player_attributes.current_ability,
            age: player.age(date),
            squad_status: player
                .contract
                .as_ref()
                .map(|c| c.squad_status.clone())
                .unwrap_or(PlayerSquadStatus::FirstTeamRegular),
            wage: player.contract.as_ref().map(|c| c.salary).unwrap_or(0),
        }
    }
}

/// Walk the buying club's roster and fire `ThreatenedByNewSigning` for
/// every same-position existing player who reads the new arrival as
/// direct competition. Gated to avoid noise — only same positional
/// group AND at least one sharp threat axis (status overlap, ability
/// bump, wage shock, fringe status) qualifies.
fn fire_new_signing_threats(
    buying_club: &mut crate::Club,
    arrival: &ArrivalThreatProfile,
    date: NaiveDate,
) {
    for team in &mut buying_club.teams.teams {
        for existing in team.players.iter_mut() {
            if existing.id == arrival.player_id {
                continue;
            }
            let existing_group = existing.position().position_group();
            if existing_group != arrival.position_group {
                continue;
            }
            let existing_status = existing
                .contract
                .as_ref()
                .map(|c| c.squad_status.clone())
                .unwrap_or(PlayerSquadStatus::FirstTeamRegular);
            let existing_age = existing.age(date);
            let existing_ability = existing.player_attributes.current_ability;
            let existing_wage = existing.contract.as_ref().map(|c| c.salary).unwrap_or(0);

            let mut reasons: Vec<crate::NewSigningThreatReason> = Vec::new();
            reasons.push(crate::NewSigningThreatReason::SamePosition);
            if existing_status == arrival.squad_status {
                reasons.push(crate::NewSigningThreatReason::SimilarSquadStatus);
            }
            if arrival.ability as i32 >= existing_ability as i32 + 8 {
                reasons.push(crate::NewSigningThreatReason::HigherAbility);
            }
            if existing_wage > 0 && arrival.wage as f32 >= (existing_wage as f32) * 1.40 {
                reasons.push(crate::NewSigningThreatReason::LargerWageDeal);
            }
            if arrival.age + 3 <= existing_age {
                reasons.push(crate::NewSigningThreatReason::YoungerAndHighPotential);
            }
            if matches!(
                existing_status,
                PlayerSquadStatus::FirstTeamSquadRotation
                    | PlayerSquadStatus::MainBackupPlayer
                    | PlayerSquadStatus::DecentYoungster
            ) {
                reasons.push(crate::NewSigningThreatReason::AlreadyFringe);
            }

            let sharp = reasons.iter().any(|r| {
                matches!(
                    r,
                    crate::NewSigningThreatReason::SimilarSquadStatus
                        | crate::NewSigningThreatReason::HigherAbility
                        | crate::NewSigningThreatReason::LargerWageDeal
                        | crate::NewSigningThreatReason::YoungerAndHighPotential
                        | crate::NewSigningThreatReason::AlreadyFringe
                )
            });
            if !sharp {
                continue;
            }
            let primary = reasons
                .iter()
                .find(|r| !matches!(r, crate::NewSigningThreatReason::SamePosition))
                .copied()
                .unwrap_or(crate::NewSigningThreatReason::SamePosition);
            let mut ctx = crate::NewSigningThreatContext::new(arrival.player_id, primary)
                .with_player_status(existing_status.clone())
                .with_rival_status(arrival.squad_status.clone())
                .with_player_age(existing_age)
                .with_rival_age(arrival.age);
            for r in reasons.iter().skip(1) {
                ctx = ctx.with_reason(*r);
            }
            existing.on_new_signing_threat(ctx);
        }
    }
}

/// Fire `EncouragedBySquadInvestment` on ambitious / senior teammates
/// after a high-quality arrival. Treats CA ≥ 145 or a club-record fee
/// as "meaningful" enough to count — a fringe depth signing doesn't
/// fire the row. Cooldown on the emit path keeps the same window from
/// double-firing if several quality arrivals land in a few days.
fn fire_squad_investment_signal(
    buying_club: &mut crate::Club,
    arrival: &ArrivalThreatProfile,
    fee: f64,
) {
    let meaningful = arrival.ability >= 145 || fee >= 30_000_000.0;
    if !meaningful {
        return;
    }
    let evidence = if fee >= 50_000_000.0 {
        crate::ClubDirectionEvidence::BoardInvestmentVisible
    } else {
        crate::ClubDirectionEvidence::MeaningfulSigningArrived
    };
    for team in &mut buying_club.teams.teams {
        for existing in team.players.iter_mut() {
            if existing.id == arrival.player_id {
                continue;
            }
            // Filter to players who actually care about squad direction
            // — ambitious or senior pros. Bench fillers don't read the
            // window like a Key Player does.
            let status = existing
                .contract
                .as_ref()
                .map(|c| c.squad_status.clone())
                .unwrap_or(PlayerSquadStatus::FirstTeamRegular);
            let cares = existing.attributes.ambition >= 14.0
                || matches!(
                    status,
                    PlayerSquadStatus::KeyPlayer | PlayerSquadStatus::FirstTeamRegular
                );
            if !cares {
                continue;
            }
            let mut ctx = crate::ClubDirectionContext::new(crate::ClubDirectionKind::Encouragement)
                .with_focal_player(arrival.player_id)
                .with_evidence(evidence);
            if existing.attributes.ambition >= 15.0 {
                ctx = ctx.with_evidence(crate::ClubDirectionEvidence::HighAmbition);
            }
            if matches!(status, PlayerSquadStatus::KeyPlayer) {
                ctx = ctx.with_evidence(crate::ClubDirectionEvidence::HighInfluence);
            }
            existing.on_club_direction_encouragement(ctx);
        }
    }
}

/// Fire `ConcernedByClubDirection` on ambitious / senior teammates
/// after a meaningful departure. Caller flags the departing player as
/// "important" (key player / high reputation) before invoking — depth
/// sales never qualify. Cooldown 120d.
fn fire_squad_concern_signal(selling_club: &mut crate::Club, departing: &DepartingPlayerInfo) {
    if !departing.high_reputation {
        return;
    }
    for team in &mut selling_club.teams.teams {
        for existing in team.players.iter_mut() {
            if existing.id == departing.id {
                continue;
            }
            let status = existing
                .contract
                .as_ref()
                .map(|c| c.squad_status.clone())
                .unwrap_or(PlayerSquadStatus::FirstTeamRegular);
            let cares = existing.attributes.ambition >= 14.0
                || matches!(
                    status,
                    PlayerSquadStatus::KeyPlayer | PlayerSquadStatus::FirstTeamRegular
                );
            if !cares {
                continue;
            }
            let mut ctx = crate::ClubDirectionContext::new(crate::ClubDirectionKind::Concern)
                .with_focal_player(departing.id)
                .with_evidence(crate::ClubDirectionEvidence::KeyPlayerSoldUnreplaced)
                .with_evidence(crate::ClubDirectionEvidence::SquadQualityWeakened);
            if existing.attributes.ambition >= 15.0 {
                ctx = ctx.with_evidence(crate::ClubDirectionEvidence::HighAmbition);
            }
            if matches!(status, PlayerSquadStatus::KeyPlayer) {
                ctx = ctx.with_evidence(crate::ClubDirectionEvidence::HighInfluence);
            }
            existing.on_club_direction_concern(ctx);
        }
    }
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

    // Country-pair route policy: this is the final chokepoint every AI
    // and stale-negotiation path flows through, so a closed route
    // (Russia ↔ Ukraine from 2022-02-24 onwards) is refused here even
    // if a prior gate let the transfer reach DeferredTransfer staging.
    // Domestic moves resolve into the same code path with
    // `selling_country_id == buying_country_id`, but the route policy
    // is symmetric and only matches cross-country pairs, so the
    // domestic case is automatically inert.
    if selling_country_id != buying_country_id {
        let selling_country_code = data
            .country(selling_country_id)
            .map(|c| c.code.clone())
            .unwrap_or_default();
        let buying_country_code = data
            .country(buying_country_id)
            .map(|c| c.code.clone())
            .unwrap_or_default();
        if !selling_country_code.is_empty()
            && !buying_country_code.is_empty()
            && TransferRoutePolicy::is_blocked(&selling_country_code, &buying_country_code, date)
        {
            debug!(
                "Blocked by country-pair route policy: player {} from country {} ({}) to country {} ({}) on {} (is_loan={})",
                player_id,
                selling_country_id,
                selling_country_code,
                buying_country_id,
                buying_country_code,
                date,
                is_loan
            );
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
        // Development pathway: a young Development-plan signing at a big
        // club may go straight onto the loan list for first-team minutes.
        // Runs AFTER the interest sweep — the sweep completes every open
        // listing for the player, which would kill the fresh loan listing
        // the staging creates. Data-level so the hoarding cap also counts
        // the buyer's cross-country loanees.
        if !is_loan {
            DevelopmentLoanPathway::stage_after_purchase_global(
                data,
                buying_country_id,
                buying_club_id,
                player_id,
                transfer.personal_terms.as_ref(),
                date,
            );
        }
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
    // Only the upfront portion moves at completion; any installment tranches
    // are paid over time by the settlement walk (see `upfront_fee`).
    let upfront = TransferExecution::upfront_fee(transfer);
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

        // Only credit income when player was actually found and taken.
        // Credit the upfront portion now; deferred installment tranches
        // arrive over time via the settlement walk.
        if player.is_some() {
            selling_club.finance.add_transfer_income(upfront);
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
            // Squad-direction concern: a high-reputation player leaving
            // the squad reads as a worrying signal for ambitious /
            // senior teammates. Cooldowned 120d so a fire-sale doesn't
            // emit a row per outgoing player.
            fire_squad_concern_signal(selling_club, info);
        }
    }

    if let Some(mut player) = player {
        if let Some(ref mut from) = from_info {
            let (league_name, league_slug) =
                resolve_selling_league_labels(country, selling_league_id);
            from.league_name = league_name;
            from.league_slug = league_slug;
        }

        // Check squad capacity AND affordability BEFORE recording history —
        // otherwise a rejected transfer creates a phantom career entry with no
        // matching transfer record, or (worse) a player moves with the fee
        // never debited because the budget couldn't cover it.
        let to_info = resolve_buying_club_info(country, buying_club_id);
        let can_accept = country
            .clubs
            .iter()
            .find(|c| c.id == buying_club_id)
            .map(|c| can_club_accept_player(c) && c.finance.can_afford_transfer(upfront))
            .unwrap_or(false);

        if !can_accept {
            debug!(
                "Transfer rejected: club {} squad full or unaffordable, returning player {}",
                buying_club_id, player_id
            );
            if let Some(selling_club) = country.clubs.iter_mut().find(|c| c.id == selling_club_id) {
                TransferExecution::add_to_main_team(selling_club, player);
                // Reverse the upfront credit booked above.
                selling_club.finance.add_transfer_income(-upfront);
            }
            return false;
        }

        // Always record history — use fallback TeamInfo if club info couldn't be resolved
        let from = from_info.unwrap_or_else(empty_team_info);
        let to = to_info.unwrap_or_else(empty_team_info);
        // Drain existing sell-on obligations now — they pay previous
        // beneficiaries out of the selling club's proceeds on this sale.
        let obligations = player.drain_sell_on_obligations();
        let selling_league_reputation =
            resolve_selling_league_reputation(country, selling_league_id);
        player.complete_transfer(TransferCompletion {
            from: &from,
            to: &to,
            fee,
            date,
            selling_club_id,
            buying_club_id,
            agreed_wage: transfer.agreed_annual_wage,
            buying_league_reputation: transfer.buying_league_reputation,
            selling_league_reputation,
            record_sell_on: transfer.sell_on_percentage,
            personal_terms: transfer.personal_terms.clone(),
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
                beneficiary.finance.adjust_cash(payout);
            }
            if let Some(seller) = country.clubs.iter_mut().find(|c| c.id == selling_club_id) {
                seller.finance.adjust_cash(-payout);
            }
        }

        // The arriving player's nationality is needed for the post-move
        // CompatriotJoined pass; capture before move-out borrows.
        let arrival_country_id = player.country_id;
        let club_country_id = country.id;
        let club_country_code = country.code.clone();
        // Snapshot the arrival's positional / status / wage profile so
        // the post-move `ThreatenedByNewSigning` pass can compare each
        // existing teammate without needing a second &mut on the new
        // arrival's row (which still lives inside the same Vec).
        let arrival_threat = ArrivalThreatProfile::from_player(&player, date);

        if let Some(buying_club) = country.clubs.iter_mut().find(|c| c.id == buying_club_id) {
            // Only the upfront portion leaves now; deferred installment
            // tranches are paid over time by the settlement walk. P&L spread
            // across DEFAULT_AMORTIZATION_YEARS. Affordability was pre-checked
            // above, so this debit always succeeds.
            buying_club
                .finance
                .register_transfer_purchase(upfront, DEFAULT_AMORTIZATION_YEARS);
            // Agent fee — separate one-off cash cost on top of the headline
            // fee. A pure cash movement (not sale income), so it must not
            // perturb the transfer budget.
            if let Some(terms) = transfer.personal_terms.as_ref() {
                if let Some(amount) = terms.agent_fee {
                    if amount > 0 {
                        buying_club.finance.adjust_cash(-(amount as f64));
                    }
                }
            }
            TransferExecution::add_to_main_team(buying_club, player);

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

            // Direct-competition pass: any existing player in the same
            // positional group AND a sharp threat axis (status overlap,
            // ability bump, wage shock, fringe status) hears the
            // signing as competition. Shared with the cross-country
            // path via `fire_new_signing_threats`.
            fire_new_signing_threats(buying_club, &arrival_threat, date);
            // Squad-investment signal: ambitious / senior teammates
            // feel encouraged when a high-quality signing lands.
            fire_squad_investment_signal(buying_club, &arrival_threat, fee);
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
        // Development-pathway staging runs at the `execute_transfer`
        // level (after the global interest sweep) so the hoarding cap
        // can count cross-country loanees too.
        //
        // Schedule any installment / performance / promotion clauses
        // so the buyer pays the seller over time as the events fire.
        TransferClauseScheduler::schedule_for_transfer(
            &mut country.transfer_market,
            transfer,
            date,
        );

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
                TransferExecution::add_to_main_team(selling_club, player);
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
        // young or was signed as a development project. The plan role is
        // the club's own stated intent — clubs can't see biological PA.
        let parent_desire = if player.age(date) <= 22
            || player
                .plan
                .as_ref()
                .map(|p| p.role == PlayerPlanRole::Development)
                .unwrap_or(false)
        {
            0.7
        } else {
            0.3
        };
        let loan_contract = build_loan_contract(
            loan_fee,
            loan_end,
            date,
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
        let parent_league_reputation =
            resolve_selling_league_reputation(country, selling_league_id);
        player.complete_loan(LoanCompletion {
            from: &from,
            to: &to,
            loan_fee,
            date,
            loan_contract,
            borrowing_club_id: buying_club_id,
            parent_league_reputation,
        });

        if let Some(buying_club) = country.clubs.iter_mut().find(|c| c.id == buying_club_id) {
            buying_club.finance.pay_loan_fee(loan_fee);
            TransferExecution::add_to_main_team(buying_club, player);
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
    // Installments defer part of the fee; only the upfront portion is paid
    // (and gated for affordability) at completion. See `upfront_fee` —
    // cross-country deals collapse to the full fee upfront.
    let upfront = TransferExecution::upfront_fee(transfer);

    let can_accept = data
        .country(buying_country_id)
        .and_then(|c| c.clubs.iter().find(|club| club.id == buying_club_id))
        .map(|club| can_club_accept_player(club) && club.finance.can_afford_transfer(upfront))
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
        upfront,
        false,
    );

    let (mut player, from_info, selling_league_id, _) = match taken {
        Some(v) => v,
        None => {
            debug!(
                "Transfer failed: player {} not found in country {}",
                player_id, selling_country_id
            );
            return false;
        }
    };

    // Resolve the source league rep BEFORE crossing into the buyer's
    // borrow — needed for the TransferEnvironmentProfile in
    // `process_transfer_shock`.
    let selling_league_reputation = data
        .country(selling_country_id)
        .map(|c| resolve_selling_league_reputation(c, selling_league_id))
        .unwrap_or(0);

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
                fire_squad_concern_signal(selling_club, info);
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
                upfront,
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
        selling_league_reputation,
        record_sell_on: transfer.sell_on_percentage,
        personal_terms: transfer.personal_terms.clone(),
    });

    let arrival_country_id = player.country_id;
    let buying_country_code = buying_country.code.clone();
    let buying_country_id_local = buying_country.id;
    // Capture profile before `players.add(player)` moves ownership in.
    let arrival_threat = ArrivalThreatProfile::from_player(&player, date);

    if let Some(buying_club) = buying_country
        .clubs
        .iter_mut()
        .find(|c| c.id == buying_club_id)
    {
        // Only the upfront portion leaves now; deferred installment tranches
        // are paid over time by the settlement walk. Affordability was
        // pre-checked above, so this debit always succeeds.
        buying_club
            .finance
            .register_transfer_purchase(upfront, DEFAULT_AMORTIZATION_YEARS);
        // Agent fee — a pure cash movement (not sale income), so it must not
        // perturb the transfer budget.
        if let Some(terms) = transfer.personal_terms.as_ref() {
            if let Some(amount) = terms.agent_fee {
                if amount > 0 {
                    buying_club.finance.adjust_cash(-(amount as f64));
                }
            }
        }
        TransferExecution::add_to_main_team(buying_club, player);

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

        // Direct-competition pass — same emit shape as the within-
        // country path; the existing teammates feel the threat from
        // the new arrival's positional / status / wage profile.
        fire_new_signing_threats(buying_club, &arrival_threat, date);
        fire_squad_investment_signal(buying_club, &arrival_threat, fee);
    }

    // Development-pathway staging runs at the `execute_transfer` level
    // (after the global interest sweep) so the hoarding cap can count
    // cross-country loanees too.

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

    // Schedule future clauses on the BUYER's country market (where the
    // daily settlement walk runs against the buyer's club). Cross-
    // country sells still route payouts via `credit_club_globally`
    // when the time comes, so the seller's country doesn't matter for
    // bookkeeping — only the buyer's does.
    if let Some(buying_country) = data.country_mut(buying_country_id) {
        TransferClauseScheduler::schedule_for_transfer(
            &mut buying_country.transfer_market,
            transfer,
            date,
        );
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
                club.finance.adjust_cash(amount);
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

    // Snapshot the parent's league rep BEFORE `take_player_from_selling_country`
    // grabs the mut-borrow — the rep feeds `LoanCompletion` so the
    // transfer-environment profile in `process_transfer_shock` can score
    // the cross-country move.
    let parent_league_reputation = data
        .country(selling_country_id)
        .map(|c| resolve_selling_league_reputation(c, selling_league_id))
        .unwrap_or(0);

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
    // Same observable rule as the within-country path: plan role and
    // age, never hidden PA.
    let parent_desire = if player.age(date) <= 22
        || player
            .plan
            .as_ref()
            .map(|p| p.role == PlayerPlanRole::Development)
            .unwrap_or(false)
    {
        0.7
    } else {
        0.3
    };
    let loan_contract = build_loan_contract(
        loan_fee,
        loan_end,
        date,
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
        parent_league_reputation,
    });

    if let Some(buying_club) = buying_country
        .clubs
        .iter_mut()
        .find(|c| c.id == buying_club_id)
    {
        buying_club.finance.pay_loan_fee(loan_fee);
        TransferExecution::add_to_main_team(buying_club, player);
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

/// Look up the league reputation (0–10000) for the selling side.
/// Friendly leagues are ignored — the player's competitive league is
/// the one that defines their tier. Returns 0 when unresolved.
fn resolve_selling_league_reputation(country: &Country, selling_league_id: Option<u32>) -> u16 {
    selling_league_id
        .and_then(|lid| country.leagues.leagues.iter().find(|l| l.id == lid))
        .and_then(|l| {
            if l.friendly {
                country.leagues.leagues.iter().find(|ml| !ml.friendly)
            } else {
                Some(l)
            }
        })
        .map(|l| l.reputation)
        .unwrap_or(0)
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
    signing_date: NaiveDate,
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
        WageCalculator::loan_wage_split_v2(parent_wage, borrower_score, parent_desire_to_develop);

    // Wage-contribution percentage = borrower share of total. Computed
    // back from borrower_wage / parent_wage so it stays consistent with
    // the split helper. Capped at 100; floored at 0.
    let contribution_pct = ((borrower_wage as f64 / parent_wage.max(1) as f64) * 100.0)
        .round()
        .clamp(0.0, 100.0) as u8;

    // Minimum-appearances scales with borrower size AND with how much of the
    // season the loan actually covers. Bigger borrowers promise the parent
    // more minutes; small borrowers can't commit. A January (half-season)
    // loan can't demand a full campaign's appearances, so the base figure is
    // pro-rated by the fraction of a ~10-month season still to play from the
    // signing date: a summer loan keeps the full bar, a winter loan ~half.
    let base_min_apps: u16 = if borrower_score >= 0.7 {
        15
    } else if borrower_score >= 0.4 {
        10
    } else {
        6
    };
    let months_remaining = (loan_end - signing_date).num_days().max(0) as f32 / 30.0;
    let season_fraction = (months_remaining / 10.0).clamp(0.4, 1.0);
    let min_apps = ((base_min_apps as f32 * season_fraction).round() as u16).max(1);

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
            .checked_sub_signed(Duration::days(90))
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
            TransferExecution::add_to_main_team(selling_club, player);
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

/// Translates the offer's clause snapshot on a finalized transfer into
/// `PendingTransferClause` entries on the buying country's market. The
/// market settlement walk then resolves them daily (installments / date
/// triggers) or at end-of-season events (appearances / goals /
/// promotion). Loans don't pay installments — the loan-fee is a single
/// cash transfer, and any future-fee option/obligation is already
/// recorded on the contract; this scheduler is a no-op for them.
pub(crate) struct TransferClauseScheduler;

impl TransferClauseScheduler {
    pub(crate) fn schedule_for_transfer(
        market: &mut TransferMarket,
        transfer: &DeferredTransfer,
        date: NaiveDate,
    ) {
        if transfer.is_loan {
            return;
        }
        let buyer = transfer.buying_club_id;
        let seller = transfer.selling_club_id;
        let player = transfer.player_id;

        for clause in &transfer.offer_clauses {
            match clause {
                TransferClause::Installments(amount, years) => {
                    // Cross-country installments are collapsed into the
                    // upfront fee at completion (the settler runs on the
                    // buyer's country market and can't route a tranche to a
                    // foreign seller), so only schedule deferred tranches for
                    // domestic deals.
                    if transfer.selling_country_id != transfer.buying_country_id {
                        continue;
                    }
                    let deferred = amount.amount.max(0.0);
                    if deferred <= 0.0 || *years == 0 {
                        continue;
                    }
                    market.schedule_installments(buyer, seller, player, deferred, *years, date);
                }
                TransferClause::AppearanceFee(amount, target) => {
                    let payout = amount.amount.max(0.0);
                    if payout <= 0.0 || *target == 0 {
                        continue;
                    }
                    market.schedule_performance_addon(
                        buyer,
                        seller,
                        player,
                        ClauseTrigger::AppearanceMilestone {
                            target_appearances: *target,
                        },
                        payout,
                        // Performance add-ons fall away after 4 seasons
                        // — long enough for realistic milestones, short
                        // enough to keep the queue from accumulating
                        // forever on players who never quite reach the
                        // threshold.
                        Some(
                            date.checked_add_signed(Duration::days(365 * 4))
                                .unwrap_or(date),
                        ),
                    );
                }
                TransferClause::GoalBonus(amount, target) => {
                    let payout = amount.amount.max(0.0);
                    if payout <= 0.0 || *target == 0 {
                        continue;
                    }
                    market.schedule_performance_addon(
                        buyer,
                        seller,
                        player,
                        ClauseTrigger::GoalMilestone {
                            target_goals: *target,
                        },
                        payout,
                        Some(
                            date.checked_add_signed(Duration::days(365 * 4))
                                .unwrap_or(date),
                        ),
                    );
                }
                TransferClause::PromotionBonus(amount) => {
                    let payout = amount.amount.max(0.0);
                    if payout <= 0.0 {
                        continue;
                    }
                    market.schedule_performance_addon(
                        buyer,
                        seller,
                        player,
                        ClauseTrigger::Promotion,
                        payout,
                        // Promotion bonus expires after one season —
                        // it only pays for the *next* promotion the
                        // buying club achieves, not a future one years
                        // down the line.
                        Some(date.checked_add_signed(Duration::days(400)).unwrap_or(date)),
                    );
                }
                // Sell-on / loan-option/obligation are already routed
                // via dedicated fields on DeferredTransfer and don't
                // belong in the pending-clauses queue.
                TransferClause::SellOnClause(_)
                | TransferClause::LoanOptionToBuy(_)
                | TransferClause::LoanObligationToBuy(_) => {}
            }
        }
    }
}

/// Post-purchase development loan staging. When a big club buys a young
/// player whose signing plan is `Development`, the buyer may list him
/// for a development loan in the SAME window — the Chelsea / Man City /
/// Benfica model: own the asset, farm out the minutes. Candidates carry
/// `LoanOutReason::DevelopmentPathway`, the only reason allowed past the
/// same-window loan-out protection. All readiness checks use observable
/// data (current ability vs the buyer's tier baseline, squad depth);
/// hidden potential is never consulted.
pub(crate) struct DevelopmentLoanPathway;

impl DevelopmentLoanPathway {
    /// Per-group ceiling on simultaneous loan-outs (active loans plus
    /// already-staged candidates). Hoarding control: a club can't farm
    /// half a position group at once.
    fn group_loan_out_cap(group: PlayerFieldPositionGroup) -> usize {
        match group {
            PlayerFieldPositionGroup::Goalkeeper => 2,
            _ => 3,
        }
    }

    /// Number of clearly-better senior teammates at the prospect's
    /// position group required before the club concludes "no minutes
    /// here" and sends him out. Below this depth the prospect stays —
    /// he is needed at home.
    fn blocked_depth_for(group: PlayerFieldPositionGroup) -> usize {
        match group {
            PlayerFieldPositionGroup::Goalkeeper => 1,
            PlayerFieldPositionGroup::Defender => 4,
            PlayerFieldPositionGroup::Midfielder => 4,
            PlayerFieldPositionGroup::Forward => 2,
        }
    }

    /// Players the club has farmed out at `group` in countries OTHER
    /// than `home_country_id`. Cross-country loanees live on foreign
    /// rosters and are invisible to the single-country scan inside
    /// [`Self::stage_after_purchase`] — without this a parent could
    /// stack a whole position group abroad past its loan-out cap.
    pub(crate) fn count_foreign_loanees_in_group(
        data: &SimulatorData,
        home_country_id: u32,
        club_id: u32,
        group: PlayerFieldPositionGroup,
    ) -> usize {
        data.continents
            .iter()
            .flat_map(|c| c.countries.iter())
            .filter(|country| country.id != home_country_id)
            .flat_map(|country| country.clubs.iter())
            .flat_map(|c| c.teams.teams.iter())
            .flat_map(|t| t.players.players.iter())
            .filter(|p| {
                p.contract_loan.as_ref().and_then(|cl| cl.loan_from_club_id) == Some(club_id)
                    && p.position().position_group() == group
            })
            .count()
    }

    /// Data-level entry used by the deferred Phase-C executor: resolves
    /// the buyer's country, counts the club's foreign loanees for the
    /// hoarding cap, and runs the country-level staging.
    pub(crate) fn stage_after_purchase_global(
        data: &mut SimulatorData,
        buying_country_id: u32,
        buying_club_id: u32,
        player_id: u32,
        personal_terms: Option<&PersonalTermsOffer>,
        date: NaiveDate,
    ) {
        let Some(group) = data
            .country(buying_country_id)
            .and_then(|country| country.clubs.iter().find(|c| c.id == buying_club_id))
            .and_then(|club| {
                club.teams
                    .teams
                    .iter()
                    .flat_map(|t| t.players.players.iter())
                    .find(|p| p.id == player_id)
            })
            .map(|p| p.position().position_group())
        else {
            return;
        };
        let foreign_loanees =
            Self::count_foreign_loanees_in_group(data, buying_country_id, buying_club_id, group);
        if let Some(country) = data.country_mut(buying_country_id) {
            Self::stage_after_purchase(
                country,
                buying_club_id,
                player_id,
                personal_terms,
                date,
                foreign_loanees,
            );
        }
    }

    /// Evaluate the just-completed permanent transfer and stage a
    /// `DevelopmentPathway` loan-out candidate on the buying club when
    /// the prospect profile fits. No-op for seniors, promised starters,
    /// small buyers, prospects near the buyer's first-team level, thin
    /// position groups, or clubs at their loan-out cap.
    ///
    /// `foreign_loanees_in_group` is the club's loanee count in OTHER
    /// countries at the prospect's position group (the country-level
    /// scan below only sees domestic loanees). Callers without
    /// SimulatorData access pass 0 — the domestic count still applies.
    pub(crate) fn stage_after_purchase(
        country: &mut Country,
        buying_club_id: u32,
        player_id: u32,
        personal_terms: Option<&PersonalTermsOffer>,
        date: NaiveDate,
        foreign_loanees_in_group: usize,
    ) {
        // The buyer promised immediate regular football — honouring the
        // promise excludes a same-window loan-out. Only an explicit
        // prospect framing (or no promise at all) keeps the path open.
        if let Some(terms) = personal_terms {
            if matches!(
                terms.squad_status_promise,
                Some(PromisedSquadStatus::KeyPlayer)
                    | Some(PromisedSquadStatus::FirstTeamRegular)
                    | Some(PromisedSquadStatus::FirstTeamSquadRotation)
            ) {
                return;
            }
        }

        // Phase 1: immutable gate evaluation.
        let group = {
            let Some(club) = country.clubs.iter().find(|c| c.id == buying_club_id) else {
                return;
            };
            let Some(team) = club.teams.main().or(club.teams.teams.first()) else {
                return;
            };
            let Some(player) = team.players.players.iter().find(|p| p.id == player_id) else {
                return;
            };

            // Development-plan youngsters only — the plan role is the
            // club's own stated intent, set at signing from observable
            // facts (age, fee).
            let is_development_plan = player
                .plan
                .as_ref()
                .map(|p| p.role == PlayerPlanRole::Development)
                .unwrap_or(false);
            if !is_development_plan || player.age(date) > 22 {
                return;
            }

            // Buyer profile: development loans are a big-club / selling-
            // academy instrument. Small Balanced clubs buy teenagers to
            // field them, not to farm them out.
            let rep_level = team.reputation.level();
            let big_buyer = matches!(
                rep_level,
                ReputationLevel::Elite | ReputationLevel::Continental | ReputationLevel::National
            ) || matches!(club.philosophy, ClubPhilosophy::DevelopAndSell);
            if !big_buyer {
                return;
            }

            let group = player.position().position_group();
            let player_ca = player.player_attributes.current_ability;

            // First-team readiness: a prospect already near the buyer's
            // tier baseline stays — he can compete for minutes now.
            let rep_score = team.reputation.overall_score();
            let baseline = PipelineProcessor::tier_starter_ca_score(rep_score, group);
            if player_ca >= baseline.saturating_sub(10) {
                return;
            }

            // Senior depth: enough clearly-better teammates at the same
            // group that the prospect realistically won't play.
            let blocked_by = team
                .players
                .players
                .iter()
                .filter(|p| {
                    p.id != player_id
                        && p.position().position_group() == group
                        && p.player_attributes.current_ability >= player_ca.saturating_add(5)
                })
                .count();
            if blocked_by < Self::blocked_depth_for(group) {
                return;
            }

            // Loan-out cap: players already farmed out at this group —
            // domestic loanees (visible in this country) plus the
            // caller-supplied foreign count — plus staged candidates
            // still on the roster.
            let active_out = country
                .clubs
                .iter()
                .flat_map(|c| c.teams.teams.iter())
                .flat_map(|t| t.players.players.iter())
                .filter(|p| {
                    p.contract_loan.as_ref().and_then(|cl| cl.loan_from_club_id)
                        == Some(buying_club_id)
                        && p.position().position_group() == group
                })
                .count();
            let pending = club
                .transfer_plan
                .loan_out_candidates
                .iter()
                .filter(|c| {
                    club.teams
                        .teams
                        .iter()
                        .flat_map(|t| t.players.players.iter())
                        .find(|p| p.id == c.player_id)
                        .map(|p| p.position().position_group() == group)
                        .unwrap_or(false)
                })
                .count();
            if active_out + foreign_loanees_in_group + pending >= Self::group_loan_out_cap(group) {
                return;
            }

            group
        };

        // Phase 2: stage the candidate (deduped by player id) and stamp
        // the decision on the player so the UI shows WHY the club is
        // loaning out a player it just bought.
        let mut staged = false;
        if let Some(club) = country.clubs.iter_mut().find(|c| c.id == buying_club_id) {
            let already_staged = club
                .transfer_plan
                .loan_out_candidates
                .iter()
                .any(|c| c.player_id == player_id);
            if !already_staged {
                debug!(
                    "Development pathway: club {} stages player {} ({:?}) for a development loan",
                    buying_club_id, player_id, group
                );
                club.transfer_plan
                    .loan_out_candidates
                    .push(LoanOutCandidate {
                        player_id,
                        reason: LoanOutReason::DevelopmentPathway,
                        status: LoanOutStatus::Identified,
                        loan_fee: 0.0,
                    });
                for team in &mut club.teams.teams {
                    if let Some(player) =
                        team.players.players.iter_mut().find(|p| p.id == player_id)
                    {
                        player.decision_history.add(
                            date,
                            "dec_board_loan_listed".to_string(),
                            "dec_reason_development_pathway".to_string(),
                            "dec_decided_board".to_string(),
                        );
                        break;
                    }
                }
                staged = true;
            }
        }

        // Same-window timing: the deferred Phase-C executor runs AFTER
        // today's pipeline pass, so without an immediate listing the
        // candidate would wait for the next open-window tick — too late
        // for a deadline-day purchase. List now while the window is
        // open; `list_loan_out_candidate` dedupes, so tomorrow's
        // process_loan_out_listings pass won't double-list.
        if staged {
            let window_open = TransferWindowManager::for_country(country, date)
                .current_window_dates(country.id, date)
                .is_some();
            if window_open {
                PipelineProcessor::list_loan_out_candidate(
                    country,
                    buying_club_id,
                    player_id,
                    date,
                );
            } else {
                // Deliberate deferral: registration windows are closed, so
                // the loan listing waits for the next window's evaluation.
                debug!(
                    "Development pathway: window closed — loan listing for player {} deferred",
                    player_id
                );
            }
        }
    }
}

#[cfg(test)]
mod country_pair_execution_tests {
    use super::*;
    use crate::academy::ClubAcademy;
    use crate::club::player::builder::PlayerBuilder;
    use crate::competitions::global::GlobalCompetitions;
    use crate::continent::Continent;
    use crate::league::{DayMonthPeriod, League, LeagueCollection, LeagueSettings};
    use crate::shared::Location;
    use crate::shared::fullname::FullName;
    use crate::transfers::offer::PersonalTermsOffer;
    use crate::{
        Club, ClubColors, ClubFacilities, ClubFinances, ClubStatus, Country, PersonAttributes,
        PlayerAttributes, PlayerCollection, PlayerPosition, PlayerPositionType, PlayerPositions,
        PlayerSkills, StaffCollection, Team, TeamCollection, TeamReputation, TrainingSchedule,
    };
    use chrono::NaiveTime;

    /// Fixture builder for the cross-country route-block tests. Grouped on
    /// a unit struct rather than free functions to follow the project's
    /// "no global helpers" convention.
    struct CountryPairFixtures;

    impl CountryPairFixtures {
        fn d(y: i32, m: u32, day: u32) -> NaiveDate {
            NaiveDate::from_ymd_opt(y, m, day).unwrap()
        }

        fn player(id: u32, country_id: u32) -> crate::Player {
            PlayerBuilder::new()
                .id(id)
                .full_name(FullName::new("Test".to_string(), format!("P{id}")))
                .birth_date(Self::d(2005, 1, 1))
                .country_id(country_id)
                .attributes(PersonAttributes::default())
                .skills(PlayerSkills::default())
                .positions(PlayerPositions {
                    positions: vec![PlayerPosition {
                        position: PlayerPositionType::Striker,
                        level: 18,
                    }],
                })
                .player_attributes(PlayerAttributes::default())
                .build()
                .unwrap()
        }

        fn training_schedule() -> TrainingSchedule {
            TrainingSchedule::new(
                NaiveTime::from_hms_opt(9, 0, 0).unwrap(),
                NaiveTime::from_hms_opt(15, 0, 0).unwrap(),
            )
        }

        fn team(
            id: u32,
            club_id: u32,
            name: &str,
            slug: &str,
            league_id: u32,
            players: Vec<crate::Player>,
        ) -> Team {
            Team::builder()
                .id(id)
                .league_id(Some(league_id))
                .club_id(club_id)
                .name(name.to_string())
                .slug(slug.to_string())
                .team_type(TeamType::Main)
                .players(PlayerCollection::new(players))
                .staffs(StaffCollection::new(Vec::new()))
                .reputation(TeamReputation::new(2000, 2000, 4000))
                .training_schedule(Self::training_schedule())
                .build()
                .unwrap()
        }

        fn club(id: u32, name: &str, main_team: Team) -> Club {
            Club::new(
                id,
                name.to_string(),
                Location::new(1),
                ClubFinances::new(1_000_000, Vec::new()),
                ClubAcademy::new(3),
                ClubStatus::Professional,
                ClubColors::default(),
                TeamCollection::new(vec![main_team]),
                ClubFacilities::default(),
            )
        }

        fn league(id: u32, slug: &str) -> League {
            League::new(
                id,
                "L".to_string(),
                slug.to_string(),
                1,
                5500,
                LeagueSettings {
                    season_starting_half: DayMonthPeriod::new(1, 8, 31, 12),
                    season_ending_half: DayMonthPeriod::new(1, 1, 31, 5),
                    tier: 1,
                    promotion_spots: 0,
                    relegation_spots: 0,
                    league_group: None,
                },
                false,
            )
        }

        fn country(id: u32, code: &str, slug: &str, league_id: u32, clubs: Vec<Club>) -> Country {
            Country::builder()
                .id(id)
                .code(code.to_string())
                .slug(slug.to_string())
                .name(slug.to_string())
                .continent_id(1)
                .reputation(5500)
                .leagues(LeagueCollection::new(vec![Self::league(league_id, slug)]))
                .clubs(clubs)
                .build()
                .unwrap()
        }

        /// Build a minimal two-country world: RU (Spartak, player 200) and
        /// UA (Dynamo Kyiv, no players). Returns `(data, transfer)` where
        /// the transfer represents Spartak loaning the prospect to Dynamo
        /// Kyiv. Caller can flip `is_loan` and the source/destination ids.
        fn ru_to_ua_world(date: NaiveDate) -> (SimulatorData, DeferredTransfer) {
            let spartak_player = Self::player(200, 100);
            let spartak_main = Self::team(
                11,
                100,
                "Spartak Moscow",
                "spartak",
                10,
                vec![spartak_player],
            );
            let spartak = Self::club(100, "Spartak Moscow", spartak_main);
            let ru = Self::country(1, "ru", "russia", 10, vec![spartak]);

            let dynamo_main = Self::team(21, 200, "Dynamo Kyiv", "dynamo-kyiv", 20, Vec::new());
            let dynamo = Self::club(200, "Dynamo Kyiv", dynamo_main);
            let ua = Self::country(2, "ua", "ukraine", 20, vec![dynamo]);

            let continent = Continent::new(1, "Europe".to_string(), vec![ru, ua], Vec::new());
            let data = SimulatorData::new(
                date.and_hms_opt(12, 0, 0).unwrap(),
                vec![continent],
                GlobalCompetitions::new(Vec::new()),
            );

            let transfer = DeferredTransfer {
                player_id: 200,
                selling_country_id: 1,
                selling_club_id: 100,
                buying_country_id: 2,
                buying_club_id: 200,
                fee: 0.0,
                is_loan: true,
                has_option_to_buy: false,
                agreed_annual_wage: Some(100_000),
                buying_league_reputation: 5500,
                sell_on_percentage: None,
                loan_future_fee: None,
                personal_terms: None as Option<PersonalTermsOffer>,
                offer_clauses: Vec::new(),
            };
            (data, transfer)
        }
    }

    /// Original report regression. A 19yo Spartak Moscow prospect loaned
    /// to Dynamo Kyiv after the 2022 cutoff must NOT complete: the
    /// execution chokepoint refuses cross-country RU↔UA on or after
    /// 2022-02-24, no matter how clean the staged DeferredTransfer is.
    /// The player stays at Spartak; the DreamMove framing therefore
    /// never reaches `process_transfer_shock` either, because no signing
    /// is staged.
    #[test]
    fn spartak_loan_to_dynamo_kyiv_after_2022_does_not_complete() {
        let date = CountryPairFixtures::d(2026, 3, 1);
        let (mut data, transfer) = CountryPairFixtures::ru_to_ua_world(date);
        let success = execute_transfer(&mut data, &transfer, date);
        assert!(!success, "RU→UA loan must be refused after the 2022 cutoff");

        // Player stays at Spartak Moscow.
        let ru = data.country(1).expect("RU country present");
        let spartak = ru.clubs.iter().find(|c| c.id == 100).expect("Spartak");
        assert!(
            spartak.teams.contains_player(200),
            "blocked loan must leave the player on Spartak's roster"
        );

        // Player must not have arrived in Ukraine.
        let ua = data.country(2).expect("UA country present");
        let dynamo = ua.clubs.iter().find(|c| c.id == 200).expect("Dynamo");
        for team in &dynamo.teams.teams {
            assert!(
                !team.players.players.iter().any(|p| p.id == 200),
                "Dynamo Kyiv must not receive the loaned prospect"
            );
        }

        // Spartak's finances must not have been credited with the loan
        // fee — the move was refused before any money changed hands.
        assert_eq!(
            spartak.finance.balance.balance, 1_000_000,
            "refused loan must not credit the selling club"
        );
    }

    /// Permanent counterpart: a permanent transfer over the same route
    /// after the cutoff must also be refused, with the player staying
    /// at the selling club and the destination roster untouched.
    #[test]
    fn permanent_ru_to_ua_transfer_after_2022_does_not_complete() {
        let date = CountryPairFixtures::d(2026, 3, 1);
        let (mut data, mut transfer) = CountryPairFixtures::ru_to_ua_world(date);
        transfer.is_loan = false;
        transfer.fee = 5_000_000.0;
        let success = execute_transfer(&mut data, &transfer, date);
        assert!(
            !success,
            "RU→UA permanent transfer must be refused after the 2022 cutoff"
        );

        let ru = data.country(1).unwrap();
        let spartak = ru.clubs.iter().find(|c| c.id == 100).unwrap();
        assert!(
            spartak.teams.contains_player(200),
            "blocked permanent transfer must leave the player on the source roster"
        );
    }

    /// Symmetric variant: UA→RU is also refused after the cutoff.
    #[test]
    fn ua_to_ru_loan_after_2022_does_not_complete() {
        let date = CountryPairFixtures::d(2026, 3, 1);
        let (mut data, mut transfer) = CountryPairFixtures::ru_to_ua_world(date);
        // Flip the direction so the source player sits in UA. The
        // fixture doesn't put a player in Dynamo's squad — synthesise
        // the inverse transfer by swapping ids only after seeding a
        // player into Dynamo's roster.
        let ua_player = CountryPairFixtures::player(201, 200);
        if let Some(country) = data.country_mut(2) {
            if let Some(club) = country.clubs.iter_mut().find(|c| c.id == 200) {
                if let Some(team) = club.teams.teams.first_mut() {
                    team.players.add(ua_player);
                }
            }
        }
        transfer.player_id = 201;
        transfer.selling_country_id = 2;
        transfer.selling_club_id = 200;
        transfer.buying_country_id = 1;
        transfer.buying_club_id = 100;
        let success = execute_transfer(&mut data, &transfer, date);
        assert!(
            !success,
            "UA→RU loan must be refused after the 2022 cutoff (symmetric)"
        );
    }

    /// Pre-cutoff: the route is open, so a clean DeferredTransfer goes
    /// through and the player ends up at the destination. Guards the
    /// no-historical-regression intent — the policy is date-gated.
    #[test]
    fn ru_to_ua_loan_before_2022_completes() {
        let date = CountryPairFixtures::d(2021, 6, 1);
        let (mut data, transfer) = CountryPairFixtures::ru_to_ua_world(date);
        let success = execute_transfer(&mut data, &transfer, date);
        assert!(
            success,
            "RU→UA loan must complete on a pre-2022 simulation date"
        );

        // Player should now live in Ukraine.
        let ua = data.country(2).unwrap();
        let dynamo = ua.clubs.iter().find(|c| c.id == 200).unwrap();
        let arrived = dynamo
            .teams
            .teams
            .iter()
            .any(|t| t.players.players.iter().any(|p| p.id == 200));
        assert!(arrived, "pre-cutoff loan must place the player at Dynamo");
    }

    /// Pre-cutoff permanent counterpart for the same reason.
    #[test]
    fn ru_to_ua_permanent_before_2022_completes() {
        let date = CountryPairFixtures::d(2021, 6, 1);
        let (mut data, mut transfer) = CountryPairFixtures::ru_to_ua_world(date);
        transfer.is_loan = false;
        transfer.fee = 3_000_000.0;
        let success = execute_transfer(&mut data, &transfer, date);
        assert!(
            success,
            "RU→UA permanent transfer must complete on a pre-2022 simulation date"
        );

        let ua = data.country(2).unwrap();
        let dynamo = ua.clubs.iter().find(|c| c.id == 200).unwrap();
        assert!(
            dynamo
                .teams
                .teams
                .iter()
                .any(|t| t.players.players.iter().any(|p| p.id == 200)),
            "pre-cutoff transfer must place the player at the destination"
        );
    }

    /// Non-RU/UA cross-country routes are untouched by the policy at any
    /// date — only the configured RU↔UA pair is closed.
    #[test]
    fn other_cross_country_routes_remain_open_after_2022_cutoff() {
        // Reuse the world but flip the buyer to a country with code "es"
        // (not on the block list). Building a fresh world with Spain
        // would duplicate fixtures; mutating the existing UA country's
        // code to a non-blocked value keeps the test compact and proves
        // the gate predicates on country code only.
        let date = CountryPairFixtures::d(2026, 3, 1);
        let (mut data, transfer) = CountryPairFixtures::ru_to_ua_world(date);
        if let Some(country) = data.country_mut(2) {
            country.code = "es".to_string();
        }
        let success = execute_transfer(&mut data, &transfer, date);
        assert!(
            success,
            "non-RU/UA cross-country route must complete regardless of the cutoff date"
        );
    }
}

#[cfg(test)]
mod development_pathway_tests {
    use super::*;
    use crate::academy::ClubAcademy;
    use crate::club::player::builder::PlayerBuilder;
    use crate::competitions::global::GlobalCompetitions;
    use crate::continent::Continent;
    use crate::league::{DayMonthPeriod, League, LeagueCollection, LeagueSettings};
    use crate::shared::Location;
    use crate::shared::fullname::FullName;
    use crate::transfers::market::{TransferListingStatus, TransferListingType};
    use crate::transfers::negotiation::NegotiationStatus;
    use crate::transfers::pipeline::LoanOutReason as PipelineLoanOutReason;
    use crate::{
        Club, ClubColors, ClubFacilities, ClubFinances, ClubStatus, PersonAttributes,
        PlayerAttributes, PlayerCollection, PlayerPosition, PlayerPositionType, PlayerPositions,
        PlayerSkills, StaffCollection, Team, TeamCollection, TeamReputation, TrainingSchedule,
    };
    use chrono::NaiveTime;

    /// Domestic prospect-purchase world: a National-tier seller holds the
    /// young player; an Elite buyer with a deep forward group buys him.
    /// Wrapped in a unit struct per the project's no-free-helpers rule.
    struct DevPathwayFixtures;

    impl DevPathwayFixtures {
        const SELLER_ID: u32 = 100;
        const BUYER_ID: u32 = 200;
        const PROSPECT_ID: u32 = 500;

        fn d(y: i32, m: u32, day: u32) -> NaiveDate {
            NaiveDate::from_ymd_opt(y, m, day).unwrap()
        }

        fn date() -> NaiveDate {
            Self::d(2026, 7, 5)
        }

        fn player(id: u32, birth_year: i32, ca: u8) -> Player {
            let mut attrs = PlayerAttributes::default();
            attrs.current_ability = ca;
            PlayerBuilder::new()
                .id(id)
                .full_name(FullName::new("Dev".to_string(), format!("P{id}")))
                .birth_date(Self::d(birth_year, 1, 1))
                .country_id(1)
                .attributes(PersonAttributes::default())
                .skills(PlayerSkills::default())
                .positions(PlayerPositions {
                    positions: vec![PlayerPosition {
                        position: PlayerPositionType::Striker,
                        level: 16,
                    }],
                })
                .player_attributes(attrs)
                .build()
                .unwrap()
        }

        fn team(id: u32, club_id: u32, rep: u16, players: Vec<Player>) -> Team {
            Team::builder()
                .id(id)
                .league_id(Some(10))
                .club_id(club_id)
                .name(format!("Team {id}"))
                .slug(format!("team-{id}"))
                .team_type(TeamType::Main)
                .players(PlayerCollection::new(players))
                .staffs(StaffCollection::new(Vec::new()))
                .reputation(TeamReputation::new(rep, rep, rep))
                .training_schedule(TrainingSchedule::new(
                    NaiveTime::from_hms_opt(9, 0, 0).unwrap(),
                    NaiveTime::from_hms_opt(15, 0, 0).unwrap(),
                ))
                .build()
                .unwrap()
        }

        fn club(id: u32, name: &str, team: Team) -> Club {
            Club::new(
                id,
                name.to_string(),
                Location::new(1),
                ClubFinances::new(50_000_000, Vec::new()),
                ClubAcademy::new(3),
                ClubStatus::Professional,
                ClubColors::default(),
                TeamCollection::new(vec![team]),
                ClubFacilities::default(),
            )
        }

        /// Buyer roster: three senior forwards clearly above the
        /// prospect's level, so the "blocked by depth" gate fires.
        fn buyer(rep: u16) -> Club {
            let forwards = vec![
                Self::player(201, 1998, 130),
                Self::player(202, 1999, 128),
                Self::player(203, 2000, 125),
            ];
            Self::club(
                Self::BUYER_ID,
                "Buyer",
                Self::team(21, Self::BUYER_ID, rep, forwards),
            )
        }

        /// A player farmed out by the buyer: lives on a foreign roster
        /// with a borrower-side loan contract pointing back at the buyer.
        fn loaned_out_by_buyer(id: u32) -> Player {
            let mut p = Self::player(id, 1999, 80);
            p.contract_loan = Some(PlayerClubContract::new_loan(
                100_000,
                Self::d(2027, 5, 31),
                Self::BUYER_ID,
                21,
                Self::FARM_CLUB_ID,
            ));
            p
        }

        const FARM_CLUB_ID: u32 = 300;

        /// Second country whose club rosters `loanees` strikers the
        /// buyer has farmed out cross-border.
        fn foreign_country(loanees: usize) -> Country {
            let players: Vec<Player> = (0..loanees)
                .map(|i| Self::loaned_out_by_buyer(900 + i as u32))
                .collect();
            let club = Self::club(
                Self::FARM_CLUB_ID,
                "Farm",
                Self::team(31, Self::FARM_CLUB_ID, 3000, players),
            );
            let league = League::new(
                20,
                "L2".to_string(),
                "league-2".to_string(),
                2,
                3000,
                LeagueSettings {
                    season_starting_half: DayMonthPeriod::new(1, 8, 31, 12),
                    season_ending_half: DayMonthPeriod::new(1, 1, 31, 5),
                    tier: 1,
                    promotion_spots: 0,
                    relegation_spots: 0,
                    league_group: None,
                },
                false,
            );
            Country::builder()
                .id(2)
                .code("es".to_string())
                .slug("spain".to_string())
                .name("spain".to_string())
                .continent_id(1)
                .reputation(4000)
                .leagues(LeagueCollection::new(vec![league]))
                .clubs(vec![club])
                .build()
                .unwrap()
        }

        /// Like [`Self::world`], plus a second country whose club holds
        /// `foreign_loanees` players the buyer already farmed out at the
        /// prospect's position group.
        fn world_with_foreign_loanees(foreign_loanees: usize) -> (SimulatorData, DeferredTransfer) {
            Self::world_impl(
                8500,
                2008,
                None,
                vec![Self::foreign_country(foreign_loanees)],
            )
        }

        /// Build the world and the staged permanent transfer for a
        /// prospect born in `birth_year` (controls age at completion).
        fn world(
            buyer_rep: u16,
            birth_year: i32,
            personal_terms: Option<PersonalTermsOffer>,
        ) -> (SimulatorData, DeferredTransfer) {
            Self::world_impl(buyer_rep, birth_year, personal_terms, Vec::new())
        }

        fn world_impl(
            buyer_rep: u16,
            birth_year: i32,
            personal_terms: Option<PersonalTermsOffer>,
            extra_countries: Vec<Country>,
        ) -> (SimulatorData, DeferredTransfer) {
            let prospect = Self::player(Self::PROSPECT_ID, birth_year, 60);
            let seller = Self::club(
                Self::SELLER_ID,
                "Seller",
                Self::team(11, Self::SELLER_ID, 5000, vec![prospect]),
            );
            let buyer = Self::buyer(buyer_rep);

            let league = League::new(
                10,
                "L".to_string(),
                "league".to_string(),
                1,
                5500,
                LeagueSettings {
                    season_starting_half: DayMonthPeriod::new(1, 8, 31, 12),
                    season_ending_half: DayMonthPeriod::new(1, 1, 31, 5),
                    tier: 1,
                    promotion_spots: 0,
                    relegation_spots: 0,
                    league_group: None,
                },
                false,
            );
            let country = Country::builder()
                .id(1)
                .code("en".to_string())
                .slug("england".to_string())
                .name("england".to_string())
                .continent_id(1)
                .reputation(5500)
                .leagues(LeagueCollection::new(vec![league]))
                .clubs(vec![seller, buyer])
                .build()
                .unwrap();
            let mut countries = vec![country];
            countries.extend(extra_countries);
            let continent = Continent::new(1, "Europe".to_string(), countries, Vec::new());
            let data = SimulatorData::new(
                Self::date().and_hms_opt(12, 0, 0).unwrap(),
                vec![continent],
                GlobalCompetitions::new(Vec::new()),
            );

            let transfer = DeferredTransfer {
                player_id: Self::PROSPECT_ID,
                selling_country_id: 1,
                selling_club_id: Self::SELLER_ID,
                buying_country_id: 1,
                buying_club_id: Self::BUYER_ID,
                fee: 2_000_000.0,
                is_loan: false,
                has_option_to_buy: false,
                agreed_annual_wage: Some(100_000),
                buying_league_reputation: 5500,
                sell_on_percentage: Some(0.15),
                loan_future_fee: None,
                personal_terms,
                offer_clauses: Vec::new(),
            };
            (data, transfer)
        }

        fn dev_candidates(data: &SimulatorData) -> usize {
            data.country(1)
                .unwrap()
                .clubs
                .iter()
                .find(|c| c.id == Self::BUYER_ID)
                .unwrap()
                .transfer_plan
                .loan_out_candidates
                .iter()
                .filter(|c| {
                    c.player_id == Self::PROSPECT_ID
                        && c.reason == PipelineLoanOutReason::DevelopmentPathway
                })
                .count()
        }
    }

    /// Headline pathway: an 18-year-old bought permanently by an Elite
    /// club lands on the buyer's loan list with the DevelopmentPathway
    /// reason — and the candidate survives the post-transfer interest
    /// sweep that `execute_transfer` runs (ownership-aware cleanup).
    #[test]
    fn bought_development_prospect_is_staged_for_loan_out() {
        let (mut data, transfer) = DevPathwayFixtures::world(8500, 2008, None);
        let date = DevPathwayFixtures::date();

        let success = execute_transfer(&mut data, &transfer, date);
        assert!(success, "domestic permanent transfer must complete");

        let country = data.country(1).unwrap();
        let buyer = country
            .clubs
            .iter()
            .find(|c| c.id == DevPathwayFixtures::BUYER_ID)
            .unwrap();
        assert!(
            buyer.teams.contains_player(DevPathwayFixtures::PROSPECT_ID),
            "prospect must be on the buyer's roster"
        );
        assert_eq!(
            DevPathwayFixtures::dev_candidates(&data),
            1,
            "buyer must stage exactly one DevelopmentPathway loan-out candidate"
        );
    }

    /// A 27-year-old bought the same way gets a CompeteForStarting plan,
    /// not Development — the same-window loan-out path stays closed for
    /// normal senior signings.
    #[test]
    fn senior_signing_is_not_staged_for_development_loan() {
        let (mut data, transfer) = DevPathwayFixtures::world(8500, 1999, None);
        let date = DevPathwayFixtures::date();

        assert!(execute_transfer(&mut data, &transfer, date));
        assert_eq!(
            DevPathwayFixtures::dev_candidates(&data),
            0,
            "senior signings must not enter the development-loan pathway"
        );
    }

    /// A promised first-team role blocks the pathway — the buyer
    /// committed minutes, so an immediate loan-out would break the
    /// promise.
    #[test]
    fn promised_regular_football_blocks_staging() {
        let terms = PersonalTermsOffer {
            annual_wage: Some(100_000),
            signing_bonus: None,
            agent_fee: None,
            contract_years: Some(5),
            squad_status_promise: Some(PromisedSquadStatus::FirstTeamRegular),
            release_clause_fee: None,
        };
        let (mut data, transfer) = DevPathwayFixtures::world(8500, 2008, Some(terms));
        let date = DevPathwayFixtures::date();

        assert!(execute_transfer(&mut data, &transfer, date));
        assert_eq!(
            DevPathwayFixtures::dev_candidates(&data),
            0,
            "a promised first-team role must keep the prospect at the club"
        );
    }

    /// A small (Regional) buyer fields the teenager instead of farming
    /// him out — the pathway is a big-club / selling-academy instrument.
    #[test]
    fn small_club_buyer_is_not_staged() {
        let (mut data, transfer) = DevPathwayFixtures::world(3500, 2008, None);
        let date = DevPathwayFixtures::date();

        assert!(execute_transfer(&mut data, &transfer, date));
        assert_eq!(
            DevPathwayFixtures::dev_candidates(&data),
            0,
            "small clubs buy teenagers to play them, not to loan them out"
        );
    }

    /// Re-running the staging for the same player must not duplicate the
    /// candidate.
    #[test]
    fn staging_is_deduplicated_per_player() {
        let (mut data, transfer) = DevPathwayFixtures::world(8500, 2008, None);
        let date = DevPathwayFixtures::date();
        assert!(execute_transfer(&mut data, &transfer, date));
        assert_eq!(DevPathwayFixtures::dev_candidates(&data), 1);

        let country = data.country_mut(1).unwrap();
        DevelopmentLoanPathway::stage_after_purchase(
            country,
            DevPathwayFixtures::BUYER_ID,
            DevPathwayFixtures::PROSPECT_ID,
            None,
            date,
            0,
        );
        assert_eq!(
            DevPathwayFixtures::dev_candidates(&data),
            1,
            "second staging pass must not duplicate the candidate"
        );
    }

    /// Deadline-day purchase: the deferred Phase-C executor runs after
    /// today's pipeline pass, so the staging must list the candidate
    /// immediately while the window is still open — otherwise the loan
    /// dies with the window. Also checks the decision-history surface.
    #[test]
    fn deadline_day_purchase_is_listed_for_loan_immediately() {
        let (mut data, transfer) = DevPathwayFixtures::world(8500, 2008, None);
        let date = DevPathwayFixtures::d(2026, 8, 31); // last day of the summer window

        assert!(execute_transfer(&mut data, &transfer, date));

        let country = data.country(1).unwrap();
        let listing = country
            .transfer_market
            .get_listing_by_player(DevPathwayFixtures::PROSPECT_ID)
            .expect("deadline-day staging must create the loan listing immediately");
        assert_eq!(listing.listing_type, TransferListingType::Loan);

        let buyer = country
            .clubs
            .iter()
            .find(|c| c.id == DevPathwayFixtures::BUYER_ID)
            .unwrap();
        let candidate = buyer
            .transfer_plan
            .loan_out_candidates
            .iter()
            .find(|c| c.player_id == DevPathwayFixtures::PROSPECT_ID)
            .unwrap();
        assert_eq!(candidate.status, LoanOutStatus::Listed);

        let player = buyer
            .teams
            .teams
            .iter()
            .flat_map(|t| t.players.players.iter())
            .find(|p| p.id == DevPathwayFixtures::PROSPECT_ID)
            .unwrap();
        assert!(
            player
                .decision_history
                .items
                .iter()
                .any(|d| d.decision == "dec_reason_development_pathway"),
            "the development-pathway decision must be visible on the player"
        );
    }

    /// A purchase executed outside any registration window stages the
    /// candidate but deliberately defers the listing — the next open
    /// window picks it up (or the window reset re-evaluates).
    #[test]
    fn closed_window_staging_defers_listing_with_candidate_intact() {
        let (mut data, transfer) = DevPathwayFixtures::world(8500, 2008, None);
        let date = DevPathwayFixtures::d(2026, 9, 10); // between windows

        assert!(execute_transfer(&mut data, &transfer, date));

        let country = data.country(1).unwrap();
        assert!(
            country
                .transfer_market
                .get_listing_by_player(DevPathwayFixtures::PROSPECT_ID)
                .is_none(),
            "no registration window open → listing must be deferred"
        );
        let buyer = country
            .clubs
            .iter()
            .find(|c| c.id == DevPathwayFixtures::BUYER_ID)
            .unwrap();
        let candidate = buyer
            .transfer_plan
            .loan_out_candidates
            .iter()
            .find(|c| c.player_id == DevPathwayFixtures::PROSPECT_ID)
            .unwrap();
        assert_eq!(
            candidate.status,
            LoanOutStatus::Identified,
            "deferred candidate stays Identified for the next listing pass"
        );
    }

    /// The next daily pipeline pass must not double-list the candidate
    /// the Phase-C staging already listed.
    #[test]
    fn next_pipeline_pass_does_not_duplicate_listing() {
        let (mut data, transfer) = DevPathwayFixtures::world(8500, 2008, None);
        let date = DevPathwayFixtures::d(2026, 8, 31);
        assert!(execute_transfer(&mut data, &transfer, date));

        let country = data.country_mut(1).unwrap();
        PipelineProcessor::process_loan_out_listings(country, date);

        let listings = country
            .transfer_market
            .listings
            .iter()
            .filter(|l| l.player_id == DevPathwayFixtures::PROSPECT_ID)
            .count();
        assert_eq!(
            listings, 1,
            "re-running the listing pass must not duplicate"
        );
    }

    /// End-to-end pathway: an Elite club buys an 18-year-old
    /// DevelopmentSigning target permanently; the staging lists him as a
    /// DevelopmentPathway loan; a realistic smaller club's loan scan
    /// picks him up and opens a loan negotiation; the loan executes;
    /// parent ownership stays with the buyer while the borrower fields
    /// him; and no stale interest rows survive anywhere. (The seller-
    /// side protection bypass at resolve_initial_approach is covered by
    /// `development_pathway_protection_tests` — here the agreed loan is
    /// executed directly so no RNG phase rolls are involved.)
    #[test]
    fn elite_purchase_to_development_loan_full_lifecycle() {
        const BORROWER_ID: u32 = 400;
        let (mut data, purchase) = DevPathwayFixtures::world(8500, 2008, None);
        let date = DevPathwayFixtures::date();

        // A realistic borrower: small (Regional) club whose forwards sit
        // at the prospect's level, so the development loan buys minutes.
        let borrower = DevPathwayFixtures::club(
            BORROWER_ID,
            "Borrower",
            DevPathwayFixtures::team(
                41,
                BORROWER_ID,
                3000,
                vec![
                    DevPathwayFixtures::player(401, 1997, 55),
                    DevPathwayFixtures::player(402, 1998, 52),
                ],
            ),
        );
        {
            let country = data.country_mut(1).unwrap();
            country.clubs.push(borrower);
            let borrower = country
                .clubs
                .iter_mut()
                .find(|c| c.id == BORROWER_ID)
                .unwrap();
            borrower.transfer_plan.initialized = true;
        }

        // 1. Permanent prospect purchase → staged AND listed (window open).
        assert!(execute_transfer(&mut data, &purchase, date));
        assert_eq!(DevPathwayFixtures::dev_candidates(&data), 1);
        assert!(
            data.country(1)
                .unwrap()
                .transfer_market
                .get_listing_by_player(DevPathwayFixtures::PROSPECT_ID)
                .is_some(),
            "purchase inside the window must list the prospect for loan immediately"
        );

        // 2. The small club's loan scan finds the listed prospect and
        // opens a loan negotiation.
        {
            let country = data.country_mut(1).unwrap();
            PipelineProcessor::scan_loan_market(country, date);
            let negotiation = country
                .transfer_market
                .negotiations
                .values()
                .find(|n| n.player_id == DevPathwayFixtures::PROSPECT_ID)
                .expect("borrower's loan scan must open a negotiation for the listed prospect");
            assert!(negotiation.is_loan, "the approach must be a loan");
            assert_eq!(
                negotiation.buying_club_id, BORROWER_ID,
                "the realistic small club is the borrower"
            );
        }

        // 3. Execute the agreed development loan: parent (buyer) → borrower.
        let loan = DeferredTransfer {
            player_id: DevPathwayFixtures::PROSPECT_ID,
            selling_country_id: 1,
            selling_club_id: DevPathwayFixtures::BUYER_ID,
            buying_country_id: 1,
            buying_club_id: BORROWER_ID,
            fee: 50_000.0,
            is_loan: true,
            has_option_to_buy: false,
            agreed_annual_wage: Some(80_000),
            buying_league_reputation: 3000,
            sell_on_percentage: None,
            loan_future_fee: None,
            personal_terms: None,
            offer_clauses: Vec::new(),
        };
        assert!(execute_transfer(&mut data, &loan, date));

        let country = data.country(1).unwrap();

        // Borrower fields him; the parent no longer rosters him.
        let borrower = country.clubs.iter().find(|c| c.id == BORROWER_ID).unwrap();
        let player = borrower
            .teams
            .teams
            .iter()
            .flat_map(|t| t.players.players.iter())
            .find(|p| p.id == DevPathwayFixtures::PROSPECT_ID)
            .expect("borrower must receive the loanee");
        let parent = country
            .clubs
            .iter()
            .find(|c| c.id == DevPathwayFixtures::BUYER_ID)
            .unwrap();
        assert!(
            !parent
                .teams
                .contains_player(DevPathwayFixtures::PROSPECT_ID),
            "the loanee plays at the borrower, not the parent"
        );

        // Parent ownership intact: the permanent contract still points
        // home, the borrower-side contract points back at the parent,
        // and the sell-on owed to the original seller survives the loan.
        assert!(player.is_on_loan());
        let parent_contract = player.contract.as_ref().expect("parent contract survives");
        assert_eq!(parent_contract.loan_to_club_id, Some(BORROWER_ID));
        let loan_contract = player
            .contract_loan
            .as_ref()
            .expect("loan contract installed");
        assert_eq!(
            loan_contract.loan_from_club_id,
            Some(DevPathwayFixtures::BUYER_ID),
            "loan contract must point back at the owning parent"
        );
        assert!(
            player
                .sell_on_obligations
                .iter()
                .any(|o| o.beneficiary_club_id == DevPathwayFixtures::SELLER_ID),
            "the original seller's sell-on must survive purchase + loan"
        );

        // Career history reads cleanly: a permanent move to the buyer,
        // then a loan spell at the borrower.
        assert!(
            player
                .statistics_history
                .current
                .iter()
                .any(|e| e.team_slug == "team-21" && !e.is_loan),
            "history must show the permanent move to the parent"
        );
        assert!(
            player
                .statistics_history
                .current
                .iter()
                .any(|e| e.team_slug == "team-41" && e.is_loan),
            "history must show the development loan at the borrower"
        );

        // No stale market or interest rows anywhere.
        assert!(
            country
                .transfer_market
                .listings
                .iter()
                .filter(|l| l.player_id == DevPathwayFixtures::PROSPECT_ID)
                .all(|l| l.status == TransferListingStatus::Completed),
            "all listings for the loanee must be completed"
        );
        assert!(
            country
                .transfer_market
                .negotiations
                .values()
                .filter(|n| n.player_id == DevPathwayFixtures::PROSPECT_ID)
                .all(|n| n.status != NegotiationStatus::Pending
                    && n.status != NegotiationStatus::Countered),
            "no live negotiation may survive the completed loan"
        );
        for club in &country.clubs {
            assert!(
                club.transfer_plan
                    .loan_out_candidates
                    .iter()
                    .all(|c| c.player_id != DevPathwayFixtures::PROSPECT_ID),
                "club {} keeps a stale loan-out candidate",
                club.id
            );
            assert!(
                club.transfer_plan
                    .scout_monitoring
                    .iter()
                    .all(|m| m.player_id != DevPathwayFixtures::PROSPECT_ID),
                "club {} keeps a stale monitoring row",
                club.id
            );
            assert!(
                club.transfer_plan.shortlists.iter().all(|s| {
                    s.candidates
                        .iter()
                        .all(|c| c.player_id != DevPathwayFixtures::PROSPECT_ID)
                }),
                "club {} keeps a stale shortlist candidate",
                club.id
            );
        }
    }

    /// Cross-country loanees count against the per-group hoarding cap —
    /// a parent with three strikers already farmed out abroad must not
    /// stage a fourth, while two leave room for one more.
    #[test]
    fn foreign_loanees_count_against_group_loan_cap() {
        let (mut data, transfer) = DevPathwayFixtures::world_with_foreign_loanees(3);
        assert!(execute_transfer(
            &mut data,
            &transfer,
            DevPathwayFixtures::date()
        ));
        assert_eq!(
            DevPathwayFixtures::dev_candidates(&data),
            0,
            "three foreign loanees fill the Forward cap — no fourth"
        );

        let (mut data, transfer) = DevPathwayFixtures::world_with_foreign_loanees(2);
        assert!(execute_transfer(
            &mut data,
            &transfer,
            DevPathwayFixtures::date()
        ));
        assert_eq!(
            DevPathwayFixtures::dev_candidates(&data),
            1,
            "two foreign loanees leave room under the cap of three"
        );
    }
}
