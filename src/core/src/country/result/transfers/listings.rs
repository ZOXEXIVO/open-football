use super::types::{SquadAnalysis, TransferActivitySummary};
use crate::club::player::behaviour_config::HappinessConfig;
use crate::club::player::calculators::FreeAgentReleaseReason;
use crate::club::player::contract::{AffordabilityInput, ContractStalemate};
use crate::club::player::transfer::processing::UNHAPPY_LISTING_MIN_DAYS;
use crate::club::staff::perception::PotentialEstimator;
use crate::club::team::squad::{SquadAssetClass, SquadAssetProtection, SquadEvidenceContext};
use crate::shared::{Currency, CurrencyValue};
use crate::transfers::TransferWindowManager;
use crate::transfers::loan::guard::LoanAssetGuard;
use crate::transfers::pipeline::approach::ApproachPass;
use crate::transfers::pipeline::{LoanOutReason, TransferTrace};
use crate::transfers::value::PlayerValuationCalculator;
use crate::transfers::{
    NegotiationStatus, TransferListing, TransferListingOrigin, TransferListingStatus,
    TransferListingType, TransferMarket,
};
use crate::{
    Club, ClubLevelAnchor, ContractType, Country, HappinessEventType, Person, Player,
    PlayerFieldPositionGroup, PlayerPositionType, PlayerSquadStatus, PlayerStatusType,
    ReputationLevel, Team,
};
use chrono::{Datelike, NaiveDate, Weekday};
use log::debug;
use std::collections::{HashMap, HashSet};

#[cfg_attr(test, derive(Debug))]
pub(crate) enum ListingDecision {
    Keep,
    Transfer {
        reason: String,
    },
    Loan {
        reason: String,
    },
    FreeTransfer,
    /// The player's live loan listing becomes a permanent listing in
    /// place — same row, same `listed_date`, so the unsold-exit valve's
    /// clock keeps the time already served on the loan list.
    UpgradeLoanToTransfer,
}

/// A loan listing that has found no borrower for this long, on a player
/// the club has also decided to sell, upgrades in place to a permanent
/// listing. Shared by the main-squad decision and the reserve branch so
/// the two squads keep one clock.
pub(crate) const LOAN_UNSOLD_UPGRADE_DAYS: i64 = 180;

/// What the country market already holds for a player, read once per
/// player before the listing decision. The decision is made on rows, not
/// on `Lst` / `Loa` badges: a badge is a claim about the market, and the
/// listing pass is what makes it true.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct MarketPresence {
    /// An Available / InNegotiation permanent listing exists.
    pub for_sale: bool,
    /// `listed_date` of the oldest Available / InNegotiation loan listing.
    pub loan_listed_since: Option<NaiveDate>,
}

impl MarketPresence {
    pub(crate) fn of(market: &TransferMarket, player_id: u32) -> Self {
        let mut presence = Self::default();
        for listing in market.listings.iter().filter(|l| {
            l.player_id == player_id
                && matches!(
                    l.status,
                    TransferListingStatus::Available | TransferListingStatus::InNegotiation
                )
        }) {
            match listing.listing_type {
                TransferListingType::Transfer => presence.for_sale = true,
                TransferListingType::Loan => {
                    presence.loan_listed_since = Some(
                        presence
                            .loan_listed_since
                            .map_or(listing.listed_date, |d| d.min(listing.listed_date)),
                    );
                }
                TransferListingType::EndOfContract => {}
            }
        }
        presence
    }
}

struct PendingListing {
    player_id: u32,
    club_id: u32,
    team_id: u32,
    asking_price: CurrencyValue,
    listing_type: TransferListingType,
    reason: String,
    decided_by: String,
}

/// What one club's listing pass reads for every player it looks at: the
/// pricing inputs that do not change between players, and the coach whose
/// name goes on each decision.
struct ClubListingScope<'c> {
    club: &'c Club,
    date: NaiveDate,
    price_level: f32,
    league_reputation: u16,
    club_reputation: u16,
    decided_by: String,
}

/// What the numeric listing gates measure a player against: his own
/// numbers, his squad's, his club's standing, and what the club can hold.
/// Read once, at the top of the evaluation, so every gate below asks the
/// same questions of the same reading.
#[derive(Clone, Copy)]
struct ListingReading {
    age: u8,
    ca_i: i16,
    avg: i16,
    is_promising_youth: bool,
    rep_level: ReputationLevel,
    parent_holds: bool,
    affordability: AffordabilityInput,
}

/// Who the club puts on the market, and the rows that says so.
pub struct ListingPass;

impl ListingPass {
    /// List players for transfer based on pipeline decisions and staff evaluations.
    pub(crate) fn list_players_from_pipeline(
        country: &mut Country,
        date: NaiveDate,
        summary: &mut TransferActivitySummary,
    ) {
        let mut listings_to_add: Vec<PendingListing> = Vec::new();
        // Loan listings that have been shopped for months with no borrower
        // while the club has separately flagged the player for sale
        // (`contract.is_transfer_listed`) — upgraded in place to permanent
        // listings so the unsold-exit valve can finally reach them.
        let mut listings_to_upgrade: Vec<(u32, CurrencyValue)> = Vec::new();
        // Loan intents the weekly rebalance withdrew because it promoted
        // the player instead. The club has already stripped the badge and
        // the candidate row; the live market row is this pass's to pull.
        Self::withdraw_cancelled_loan_listings(country, date);
        let price_level = country.settings.pricing.price_level;
        let window_mgr = TransferWindowManager::for_country(country.id, &country.code, date);
        let current_window = window_mgr.current_window_dates(country.id, date);

        for club in &country.clubs {
            let squad_analysis = Self::analyze_squad_needs(club, date);

            if club.teams.teams.is_empty() {
                continue;
            }

            let main_team = &club.teams.teams[0];
            let league_reputation = main_team
                .league_id
                .and_then(|lid| country.leagues.leagues.iter().find(|l| l.id == lid))
                .map(|l| l.reputation)
                .unwrap_or(0);
            // Blend home/national/world rather than reading just `world` —
            // a club with strong domestic standing but limited continental
            // exposure should still command a domestic premium.
            let club_reputation = main_team.reputation.market_value_score();
            let scope = ClubListingScope {
                club,
                date,
                price_level,
                league_reputation,
                club_reputation,
                decided_by: main_team.staffs.head_coach_name(),
            };

            Self::main_squad_listings(
                country,
                &scope,
                main_team,
                &squad_analysis,
                current_window,
                &mut listings_to_add,
                &mut listings_to_upgrade,
            );

            Self::reserve_squad_listings(
                country,
                &scope,
                &mut listings_to_add,
                &mut listings_to_upgrade,
            );
        }

        // Cap club-decided listings so no position group on a main team
        // drops below a minimum. Player-initiated (REQ/UNH) listings are
        // honoured even when this leaves the group short — the player
        // wants out and the club must replace him.
        let (listings_to_add, capped_out) =
            Self::enforce_position_group_minimums(country, listings_to_add);

        // A club-decided listing the depth cap refused is not a listing
        // that happens later — it is a decision the club could not act on.
        // Leaving `is_transfer_listed` set would strand the player: the
        // flag blocks contract renewal and coach-agreed termination while
        // no market row exists for anyone to buy him from, so he runs his
        // deal down with no offers and no exit. Releasing the intent puts
        // him back in the squad properly; the surplus sweeps re-raise it
        // the moment the group has the depth to sell him.
        Self::release_capped_listing_intent(country, &capped_out);

        if !listings_to_add.is_empty() {
            debug!(
                "Transfer market: listing {} players for transfer/loan",
                listings_to_add.len()
            );
        }

        Self::apply_listings(country, date, summary, listings_to_add);

        Self::upgrade_stale_loan_listings(country, date, summary, listings_to_upgrade);

        // Self-healing: clear any `Lst` / `Loa` badge no longer backed by a
        // live market listing or a pending listing intent, so the flag and
        // the status can never drift into a stale "Transfer Listed".
        Self::reconcile_stale_market_statuses(country);
    }

    /// Pull the live loan listings of players whose club withdrew the loan
    /// intent — the weekly rebalance promoted them into the first team
    /// instead ([`crate::Club::rebalance_squads`]).
    ///
    /// The club can strip the badge and drop the candidate row on its own,
    /// but the market row lives on the country and would otherwise keep
    /// advertising a player his club has just promoted — and a listing
    /// riding a live negotiation is a deal in flight, so those are left
    /// alone and the withdrawal simply lapses.
    fn withdraw_cancelled_loan_listings(country: &mut Country, date: NaiveDate) {
        let withdrawn: Vec<u32> = country
            .clubs
            .iter_mut()
            .flat_map(|club| club.transfer_plan.loan_withdrawals.drain(..))
            .collect();
        if withdrawn.is_empty() {
            return;
        }
        let before = country.transfer_market.listings.len();
        country.transfer_market.listings.retain(|listing| {
            !(listing.listing_type == TransferListingType::Loan
                && listing.status == TransferListingStatus::Available
                && withdrawn.contains(&listing.player_id))
        });
        let pulled = before - country.transfer_market.listings.len();
        debug!("loan withdrawals: {} rows pulled on {date}", pulled);
        for player_id in withdrawn {
            if TransferTrace::is(player_id) {
                TransferTrace::line(
                    player_id,
                    "list",
                    format!("pass=loan_withdrawal reason=promoted_instead date={date}"),
                );
            }
        }
    }

    /// Clear a player's `Lst` / `Loa` market badge when it no longer reflects
    /// any market state — the self-healing counterpart to the listing pass
    /// above. The genuine delist events already clear these (a sale via
    /// `reset_on_club_change`, a free release via the free-agent sweep), so
    /// this is a guard against drift: a badge left behind when a listing is
    /// resolved or an intent flag cleared. Conservative and double-gated — a
    /// badge is removed ONLY when there is no active market listing of its
    /// type AND no pending intent (`is_transfer_listed` for `Lst`, a live
    /// loan-out candidate for `Loa`) — so a genuinely-listed player is never
    /// stripped off the market by mistake.
    fn reconcile_stale_market_statuses(country: &mut Country) {
        let mut transfer_listed: HashSet<u32> = HashSet::new();
        let mut loan_listed: HashSet<u32> = HashSet::new();
        for listing in &country.transfer_market.listings {
            if listing.status != TransferListingStatus::Available {
                continue;
            }
            match listing.listing_type {
                TransferListingType::Loan => {
                    loan_listed.insert(listing.player_id);
                }
                TransferListingType::Transfer => {
                    transfer_listed.insert(listing.player_id);
                }
                TransferListingType::EndOfContract => {}
            }
        }

        for club in country.clubs.iter_mut() {
            // Owned snapshot so the immutable borrow of `transfer_plan` ends
            // before the mutable walk of `teams` (disjoint `club` fields, but
            // the set outlives the borrow this way).
            let loan_candidates: HashSet<u32> = club
                .transfer_plan
                .loan_out_candidates
                .iter()
                .map(|c| c.player_id)
                .collect();
            for team in club.teams.teams.iter_mut() {
                for player in team.players.players.iter_mut() {
                    if player.statuses.has(PlayerStatusType::Lst) {
                        let flagged = player
                            .contract
                            .as_ref()
                            .map(|c| c.is_transfer_listed)
                            .unwrap_or(false);
                        if !flagged && !transfer_listed.contains(&player.id) {
                            player.statuses.remove(PlayerStatusType::Lst);
                        }
                    }
                    if player.statuses.has(PlayerStatusType::Loa)
                        && !loan_listed.contains(&player.id)
                        && !loan_candidates.contains(&player.id)
                    {
                        player.statuses.remove(PlayerStatusType::Loa);
                    }
                }
            }
        }
    }

    /// Escape valve for players stranded on the transfer list. A listing
    /// the market has ignored for a full year — asking price decayed, the
    /// scouts' availability push exhausted, no live negotiation — stops
    /// being a sale in progress and becomes a stalemate the player
    /// refuses to live with: he pushes for a termination, the club
    /// (already paying wages for a player it decided to sell) agrees,
    /// pays the severance, and he leaves on a free. Without this valve a
    /// dissatisfied player could sit listed for five seasons.
    ///
    /// Contracts already inside their final half-year are left to lapse
    /// naturally instead — the renewal gate guarantees no new offer, so
    /// expiry is the cheaper exit and needs no severance. Weekly cadence;
    /// window-independent (tearing up a contract is legal year-round).
    /// The transfer window has just closed with these players still on
    /// the market — the moment the limbo becomes real: nothing can
    /// change until the next window. One mood note per genuinely
    /// listed, unsold player. The availability broadcast and the
    /// free-exit valve stay the machinery that resolves the listing;
    /// this is the player feeling the door shut.
    pub(crate) fn emit_window_close_limbo(country: &mut Country, date: NaiveDate) {
        let _ = date;
        // Pass 1 (read): genuine, still-open seller listings. Synthetic
        // rows and expiring contracts aren't a player waiting on a move.
        let listed_ids: HashSet<u32> = country
            .transfer_market
            .listings
            .iter()
            .filter(|l| {
                l.listing_type == TransferListingType::Transfer
                    && l.origin == TransferListingOrigin::SellerListed
                    && l.status == TransferListingStatus::Available
            })
            .map(|l| l.player_id)
            .collect();
        if listed_ids.is_empty() {
            return;
        }
        // Pass 2 (mutate): land the mood on every listed player still
        // rostered in this country. Cooldown 100d — long enough to fire
        // once per window close, never twice inside the same window.
        let magnitude = HappinessConfig::default().catalog.unsold_window_closed;
        for club in country.clubs.iter_mut() {
            for team in club.teams.teams.iter_mut() {
                for player in team.players.players.iter_mut() {
                    if !listed_ids.contains(&player.id) || player.is_on_loan() {
                        continue;
                    }
                    player.happiness.add_event_with_cooldown(
                        HappinessEventType::UnsoldWindowClosed,
                        magnitude,
                        100,
                    );
                }
            }
        }
    }

    pub(crate) fn release_unsold_listed_players(country: &mut Country, date: NaiveDate) {
        if date.weekday() != Weekday::Mon {
            return;
        }
        const UNSOLD_EXIT_DAYS: i64 = 365;
        const MIN_REMAINING_DAYS: i64 = 180;
        // Stagger the valve so a save with a long-stale backlog doesn't
        // dump every stranded player into the free-agent pool in one tick.
        const MAX_EXITS_PER_CLUB_PER_PASS: usize = 2;

        // LIVE negotiations only: resolved rows are retained ~30 days for
        // diagnostics, and a status-blind check let a bid rejected weeks
        // ago keep deferring the valve (and, worse, mis-ordered a player's
        // exit against a genuinely pending deal).
        let in_negotiation: HashSet<u32> = country
            .transfer_market
            .negotiations
            .values()
            .filter(|n| {
                matches!(
                    n.status,
                    NegotiationStatus::Pending | NegotiationStatus::Countered
                )
            })
            .map(|n| n.player_id)
            .collect();

        // Pass 1 (read): genuine seller listings that have gone unsold
        // past the threshold, bounded per club.
        let mut exits: Vec<(u32, u32)> = Vec::new(); // (player_id, club_id)
        let mut per_club: HashMap<u32, usize> = HashMap::new();
        for listing in &country.transfer_market.listings {
            if listing.listing_type != TransferListingType::Transfer
                || listing.origin != TransferListingOrigin::SellerListed
                || listing.status != TransferListingStatus::Available
            {
                continue;
            }
            if (date - listing.listed_date).num_days() < UNSOLD_EXIT_DAYS {
                continue;
            }
            if in_negotiation.contains(&listing.player_id) {
                continue;
            }
            let taken = per_club.entry(listing.club_id).or_insert(0);
            if *taken >= MAX_EXITS_PER_CLUB_PER_PASS {
                continue;
            }
            let Some(player) = country
                .clubs
                .iter()
                .filter(|c| c.id == listing.club_id)
                .flat_map(|c| c.teams.teams.iter())
                .flat_map(|t| t.players.players.iter())
                .find(|p| p.id == listing.player_id)
            else {
                continue;
            };
            // A loaned-out or pinned player isn't the valve's to release;
            // a near-expiry deal just runs out (renewals are blocked).
            if player.is_on_loan() || player.is_force_match_selection {
                continue;
            }
            let Some(contract) = player.contract.as_ref() else {
                continue;
            };
            if (contract.expiration - date).num_days() < MIN_REMAINING_DAYS {
                continue;
            }
            *taken += 1;
            exits.push((listing.player_id, listing.club_id));
        }
        if exits.is_empty() {
            return;
        }

        // Pass 2 (mut clubs): tear up the contract, pay the severance,
        // drop the club-side asking-price entry.
        for &(player_id, club_id) in &exits {
            let Some(club) = country.clubs.iter_mut().find(|c| c.id == club_id) else {
                continue;
            };
            let mut payout: u32 = 0;
            for team in &mut club.teams.teams {
                team.transfer_list.remove(player_id);
                if let Some(player) = team.players.players.iter_mut().find(|p| p.id == player_id) {
                    payout = player
                        .contract
                        .as_ref()
                        .map(|c| c.termination_cost(date))
                        .unwrap_or(0);
                    player.on_contract_terminated(date, FreeAgentReleaseReason::UnsoldListingExit);
                    debug!(
                        "Unsold-listing exit: player {} leaves club {} for free after a year on the list (severance {})",
                        player_id, club_id, payout
                    );
                }
            }
            if payout > 0 {
                club.finance
                    .balance
                    .push_expense_player_wages(payout as i64);
            }
        }

        // Pass 3 (mut market): retire the listing rows and drop every
        // club's standing interest — the player is bound for the
        // free-agent pool, where the pool machinery owns his market.
        for &(player_id, _) in &exits {
            for listing in country
                .transfer_market
                .listings
                .iter_mut()
                .filter(|l| l.player_id == player_id)
            {
                listing.status = TransferListingStatus::Cancelled;
            }
            ApproachPass::clear_player_interest(country, player_id);
        }
    }

    /// Drop club-decided listings that would push a position group on the
    /// main team below a minimum. Player-initiated listings (REQ/UNH) and
    /// free-transfer releases for under-16s bypass the cap — those must
    /// be honoured regardless of depth.
    ///
    /// Without this, the pipeline's below-average / surplus / aging /
    /// contract-expiring paths can independently flag every goalkeeper
    /// in a club whose squad-wide CA average sits above the keepers', and
    /// the result is a team with zero recognised goalkeepers.
    /// Clear the sell intent for players whose listing the depth cap
    /// rejected. Returns them to normal squad handling: renewal offers,
    /// coach evaluation and the `Lst` badge reconciliation all key off
    /// `is_transfer_listed`, and all three are dead while it is stuck on
    /// with no listing behind it.
    fn release_capped_listing_intent(country: &mut Country, capped_out: &[PendingListing]) {
        if capped_out.is_empty() {
            return;
        }
        for dropped in capped_out {
            let Some(club) = country.clubs.iter_mut().find(|c| c.id == dropped.club_id) else {
                continue;
            };
            for team in club.teams.teams.iter_mut() {
                let Some(player) = team
                    .players
                    .players
                    .iter_mut()
                    .find(|p| p.id == dropped.player_id)
                else {
                    continue;
                };
                if let Some(contract) = player.contract.as_mut() {
                    contract.is_transfer_listed = false;
                }
                // The badge is the visible half of the same intent; a
                // player the club cannot sell is not "transfer listed", and
                // a player it cannot loan is not "loan listed" — the badge
                // would silence his own playing-time complaints and hide
                // him from the next audit for a listing that never comes.
                player.statuses.remove(PlayerStatusType::Lst);
                if dropped.listing_type == TransferListingType::Loan {
                    player.statuses.remove(PlayerStatusType::Loa);
                }
                break;
            }
        }
    }

    /// Returns `(kept, capped_out)` — the listings that survive the depth
    /// cap, and the club-decided ones it refused. Callers must resolve the
    /// refused intents rather than dropping them, or the player is left
    /// flagged-for-sale with no market row.
    fn enforce_position_group_minimums(
        country: &Country,
        listings: Vec<PendingListing>,
    ) -> (Vec<PendingListing>, Vec<PendingListing>) {
        use std::collections::HashMap;

        const EXEMPT_REASONS: &[&str] = &[
            "dec_reason_player_requested",
            "dec_reason_player_unhappy",
            "dec_reason_under16_release",
        ];

        let (exempt, capped): (Vec<PendingListing>, Vec<PendingListing>) = listings
            .into_iter()
            .partition(|l| EXEMPT_REASONS.contains(&l.reason.as_str()));

        let find_main = |club_id: u32| {
            country
                .clubs
                .iter()
                .find(|c| c.id == club_id)
                .and_then(|c| c.teams.main())
        };

        let player_group = |club_id: u32, player_id: u32| {
            find_main(club_id).and_then(|t| {
                t.players
                    .players
                    .iter()
                    .find(|p| p.id == player_id)
                    .map(|p| p.position().position_group())
            })
        };

        let player_ca = |club_id: u32, player_id: u32| {
            find_main(club_id)
                .and_then(|t| t.players.players.iter().find(|p| p.id == player_id))
                .map(|p| p.player_attributes.current_ability)
                .unwrap_or(0)
        };

        let mut groups: HashMap<(u32, PlayerFieldPositionGroup), Vec<PendingListing>> =
            HashMap::new();
        let mut off_main: Vec<PendingListing> = Vec::new();
        for listing in capped {
            if let Some(group) = player_group(listing.club_id, listing.player_id) {
                groups
                    .entry((listing.club_id, group))
                    .or_default()
                    .push(listing);
            } else {
                // Not on the main roster (reserve/youth club listing):
                // selling him can't thin the main team, so the depth cap
                // doesn't apply. These used to fall out of `groups` and
                // get silently dropped, stranding non-main listings.
                off_main.push(listing);
            }
        }

        let mut result = exempt;
        result.append(&mut off_main);
        let mut capped_out: Vec<PendingListing> = Vec::new();

        for ((club_id, group), mut group_listings) in groups {
            let current_count = find_main(club_id)
                .map(|t| {
                    t.players
                        .iter()
                        .filter(|p| !p.is_on_loan())
                        .filter(|p| p.position().position_group() == group)
                        .count()
                })
                .unwrap_or(0);

            let exempt_in_group = result
                .iter()
                .filter(|l| l.club_id == club_id)
                .filter(|l| player_group(l.club_id, l.player_id) == Some(group))
                .count();

            // State-derived throttle: count players in this group that are
            // ALREADY on a transfer / loan / free-transfer list from
            // earlier passes. Each one occupies a "selling slot" until it
            // moves on, so the cap emerges naturally from squad state
            // instead of a hard-coded per-pass maximum. A club that has
            // already put two backups on the market can't list a third
            // this month; once one clears (either sells or gets delisted),
            // a new slot opens next cycle. Exempt listings (REQ / UNH)
            // aren't subject to this throttle — when the player wants out,
            // he goes regardless of how full the selling queue is. This
            // pass's own candidates are not "already listed": the board
            // audit stamps the badge the day it decides, so a candidate
            // used to count against his own slot.
            let in_this_pass: HashSet<u32> = result
                .iter()
                .chain(group_listings.iter())
                .map(|l| l.player_id)
                .collect();
            let already_listed_in_group = find_main(club_id)
                .map(|t| {
                    t.players
                        .iter()
                        .filter(|p| p.position().position_group() == group)
                        .filter(|p| !in_this_pass.contains(&p.id))
                        .filter(|p| {
                            p.statuses.has(PlayerStatusType::Lst)
                                || p.statuses.has(PlayerStatusType::Loa)
                                || p.statuses.has(PlayerStatusType::Frt)
                        })
                        .count()
                })
                .unwrap_or(0);

            let min_to_keep = ListingBars::min_squad(group);
            let slots_after_min = current_count.saturating_sub(min_to_keep);
            let max_can_list = slots_after_min
                .saturating_sub(exempt_in_group)
                .saturating_sub(already_listed_in_group);

            // Worst-CA players get listed first
            group_listings.sort_by_key(|l| player_ca(l.club_id, l.player_id));

            let mut kept = group_listings;
            let refused = kept.split_off(max_can_list.min(kept.len()));
            result.extend(kept);
            capped_out.extend(refused);
        }

        (result, capped_out)
    }

    pub(crate) fn analyze_squad_needs(club: &Club, current_date: NaiveDate) -> SquadAnalysis {
        if club.teams.teams.is_empty() {
            return SquadAnalysis {
                surplus_positions: vec![],
                needed_positions: vec![],
                average_age: 25.0,
                quality_level: 50,
            };
        }

        let team = &club.teams.teams[0];
        let players = &team.players.players;

        if players.is_empty() {
            return SquadAnalysis {
                surplus_positions: vec![],
                needed_positions: vec![],
                average_age: 25.0,
                quality_level: 50,
            };
        }

        let mut group_counts: HashMap<PlayerFieldPositionGroup, u32> = HashMap::new();
        let mut total_ability: u32 = 0;
        let mut total_age: u32 = 0;
        for player in players {
            let group = player.position().position_group();
            *group_counts.entry(group).or_insert(0) += 1;
            total_ability += player.player_attributes.current_ability as u32;
            total_age += player.age(current_date) as u32;
        }

        let avg_ability = (total_ability / players.len() as u32) as u8;
        let avg_age = total_age as f32 / players.len() as f32;

        let mut surplus = Vec::new();
        let mut needed = Vec::new();

        // Over-/under-stocked is the same judgement the weekly squad
        // rebalance and the buy-side squad-fit gate already make, so ask the
        // position group itself rather than keep a private, stricter copy of
        // the table. The old local numbers (GK > 2, DEF > 7) called a
        // perfectly normal three-keeper squad surplus at goalkeeper, and —
        // because `DefensiveMidfielder` counts in the Defender group —
        // flagged practically every senior squad in the world as surplus at
        // the back, which is the trigger that then lists anyone sitting a
        // point below the squad average.
        for (group, representative) in [
            (
                PlayerFieldPositionGroup::Goalkeeper,
                PlayerPositionType::Goalkeeper,
            ),
            (
                PlayerFieldPositionGroup::Defender,
                PlayerPositionType::DefenderCenter,
            ),
            (
                PlayerFieldPositionGroup::Midfielder,
                PlayerPositionType::MidfielderCenter,
            ),
            (
                PlayerFieldPositionGroup::Forward,
                PlayerPositionType::Striker,
            ),
        ] {
            let count = *group_counts.get(&group).unwrap_or(&0) as usize;
            if group.is_over_stocked(count) {
                surplus.push(representative);
            }
            if group.is_under_stocked(count) {
                needed.push(representative);
            }
        }

        SquadAnalysis {
            surplus_positions: surplus,
            needed_positions: needed,
            average_age: avg_age,
            quality_level: avg_ability,
        }
    }

    pub(crate) fn evaluate_player_listing(
        player: &Player,
        analysis: &SquadAnalysis,
        club: &Club,
        date: NaiveDate,
        current_window: Option<(NaiveDate, NaiveDate)>,
        presence: MarketPresence,
    ) -> ListingDecision {
        if let Some(decision) = Self::already_settled(player, current_window, presence) {
            return decision;
        }
        let flagged_for_sale = player
            .contract
            .as_ref()
            .is_some_and(|c| c.is_transfer_listed);
        let labelled_not_needed = player
            .contract
            .as_ref()
            .is_some_and(|c| matches!(c.squad_status, PlayerSquadStatus::NotNeeded));
        if let Some(decision) = Self::market_row_verdict(
            player,
            date,
            presence,
            flagged_for_sale,
            labelled_not_needed,
        ) {
            return decision;
        }

        let age = player.age(date);
        let ca = player.player_attributes.current_ability;
        // Clubs can't see biological PA — listing decisions read the
        // observable ceiling (visible ability + age/mentals projection).
        let pa = PotentialEstimator::observable_ceiling(player, date);
        let ca_i = ca as i16;
        let avg = analysis.quality_level as i16;
        let is_promising_youth = age <= 23 && pa > ca + 10;

        let rep_level = club
            .teams
            .teams
            .first()
            .map(|t| t.reputation.level())
            .unwrap_or(ReputationLevel::Amateur);

        // Affordability evidence for the contract-stalemate trigger at the
        // end of this function.
        let affordability = AffordabilityInput {
            wage_budget_headroom: club
                .board
                .season_targets
                .as_ref()
                .map(|t| t.wage_budget as u32)
                .map(|budget| {
                    let total_wages: u32 = club.teams.iter().map(|t| t.get_annual_salary()).sum();
                    budget.saturating_sub(total_wages)
                }),
            current_salary: player.contract.as_ref().map(|c| c.salary).unwrap_or(0),
        };

        if let Some(decision) = Self::plan_and_request_verdict(player, club) {
            return decision;
        }

        // Would the club entertain a loan of this man at all? Read once —
        // every loan arm below consults it, and a sale never does: a club
        // may decide to SELL its starter, it does not lend him out.
        let parent_holds = LoanAssetGuard::parent_holds_for(club, player, date);

        let reading = ListingReading {
            age,
            ca_i,
            avg,
            is_promising_youth,
            rep_level,
            parent_holds,
            affordability,
        };

        if let Some(decision) = Self::unhappiness_verdict(player, club, date) {
            return decision;
        }

        if let Some(decision) = Self::club_decision_verdict(
            player,
            club,
            date,
            flagged_for_sale,
            labelled_not_needed,
            reading,
        ) {
            return decision;
        }

        // Squad members the club wouldn't move on pure maths. Runs after
        // explicit decisions (NotNeeded / club-listed / REQ / UNH) so those
        // still dictate, but before numeric triggers so a club captain with
        // a few rating points below the squad mean isn't auto-sold.
        if Self::is_squad_protected(player, club, date) {
            return ListingDecision::Keep;
        }

        if let Some(decision) = Self::below_club_level(player, club, date, reading) {
            return decision;
        }

        if let Some(decision) =
            Self::numeric_listing_triggers(player, analysis, club, date, reading)
        {
            return decision;
        }

        ListingDecision::Keep
    }

    /// Decide between Transfer and Loan based on player profile and club context.
    fn decide_listing_type(
        player: &Player,
        rep_level: &ReputationLevel,
        avg: i16,
        date: NaiveDate,
        parent_holds: bool,
        base_reason: String,
    ) -> ListingDecision {
        let age = player.age(date);
        let ca = player.player_attributes.current_ability;
        // Observable ceiling, not hidden PA — same rule as the listing
        // evaluation above.
        let pa = PotentialEstimator::observable_ceiling(player, date);

        // Under 16: free transfer
        if age < 16 {
            return ListingDecision::FreeTransfer;
        }

        // Young with development potential → loan for match practice.
        // Never the club's own first choice in that shirt: `parent_holds`
        // is the standing read, and a starter needs no match practice
        // elsewhere. Every loan arm below answers to it — the transfer
        // arms do not, because a sale is a decision the club is entitled
        // to make about anybody.
        if age <= 23 && pa > ca + 10 && !parent_holds {
            return ListingDecision::Loan {
                reason: "dec_reason_young_needs_practice".to_string(),
            };
        }

        // At wealthy club, young enough and decent quality → loan to preserve asset
        if age <= 25
            && !parent_holds
            && matches!(
                rep_level,
                ReputationLevel::Elite | ReputationLevel::Continental
            )
            && (ca as i16) >= avg - 20
        {
            return ListingDecision::Loan {
                reason: "dec_reason_blocked_top_club".to_string(),
            };
        }

        // Aging AND peaked → transfer. "Aging" scales with position group
        // so a 30-year-old GK isn't treated the same as a 30-year-old
        // winger. Requires both conditions — the previous OR labelled any
        // 27-year-old who'd reached his potential as "peaked or declining",
        // which is simply a mature player, not a selling point.
        let peaked_age = ListingBars::aging(player.position().position_group()).saturating_sub(2);
        if age >= peaked_age && pa <= ca {
            return ListingDecision::Transfer {
                reason: "dec_reason_peaked_declining".to_string(),
            };
        }

        // Mid-career at wealthy club → loan to preserve value
        if age <= 27
            && !parent_holds
            && matches!(
                rep_level,
                ReputationLevel::Elite | ReputationLevel::Continental
            )
        {
            return ListingDecision::Loan {
                reason: "dec_reason_loan_playing_time".to_string(),
            };
        }

        // Default: transfer
        ListingDecision::Transfer {
            reason: base_reason,
        }
    }

    /// Is this a player the club would keep on non-numeric grounds?
    ///
    /// Real-world squad management keeps players whose value isn't
    /// captured by a CA/PA spreadsheet: formal squad-core designation,
    /// dressing-room leadership, and long-serving pros still contributing
    /// on the pitch. Player-initiated departures (REQ/UNH) and explicit
    /// club decisions (NotNeeded, club-listed) are evaluated earlier and
    /// bypass this — the club can still sell, the player can still ask
    /// out, but routine below-average/surplus/aging sweeps don't touch
    /// this tier.
    /// Official appearances at or below which a fit player has been frozen
    /// out rather than merely rotated.
    const FROZEN_OUT_APPEARANCE_BAR: u16 = 3;
    /// Share of the club's official matches (per cent) below which a
    /// player the club has outgrown counts as no longer being picked —
    /// the rotation bar the label's own honesty cap reads. Only ever
    /// consulted together with the club-level test, in
    /// [`Self::outgrown_and_barely_playing`].
    const STANDING_SHARE_PCT: u32 = 15;

    /// Whether a protection resting on STANDING rather than numbers — long
    /// service, dressing-room authority, a veteran keeper's mentoring role
    /// — is still earned.
    ///
    /// Each of those is a real reason clubs keep players a spreadsheet
    /// would move on, but every one of them assumes the player is still
    /// part of the team. Applied unconditionally they were permanent: a
    /// six-year servant or a 30-year-old keeper stayed protected through
    /// season after season of not playing, which blocked the routine
    /// sweeps while the renewal and release paths were closed to him too.
    /// Once a season has produced a readable sample and a fit player still
    /// has essentially no minutes, the club is paying for someone it does
    /// not use, and the sweeps should be free to move him on. Injury and
    /// suspension are excused — being unavailable is not being unwanted.
    fn standing_protection_still_earned(player: &Player, club: &Club, date: NaiveDate) -> bool {
        let sample = SquadEvidenceContext::current_season_sample(date, club);
        if sample.is_early_season() {
            return true;
        }
        if player.player_attributes.is_injured
            || player.player_attributes.is_banned
            || player.player_attributes.is_in_recovery()
        {
            return true;
        }
        let appearances = player.statistics.played
            + player.statistics.played_subs
            + player.cup_statistics.played
            + player.cup_statistics.played_subs;
        appearances > Self::FROZEN_OUT_APPEARANCE_BAR
    }

    /// A player the club has outgrown who is also barely being picked.
    ///
    /// Long service, dressing-room authority and a veteran keeper's
    /// mentoring role are real reasons to keep a man a spreadsheet would
    /// move on — but they are reasons to keep a SQUAD player, not one the
    /// club has outgrown and stopped selecting. This pairs the two tests
    /// so the shields lapse for exactly that man.
    ///
    /// The share test is deliberately confined to this pair. Applied on
    /// its own inside [`Self::standing_protection_still_earned`] it was
    /// measured over a season, twice, and rejected: it stripped
    /// protection from ordinary fringe players at every club in the world
    /// and took transfer listings up 38 % and standing unhappiness up
    /// 72 %, which is a different change from the one this fixes.
    fn outgrown_and_barely_playing(player: &Player, club: &Club, date: NaiveDate) -> bool {
        let Some(level) = Self::club_level(club) else {
            return false;
        };
        let group = player.position().position_group();
        if !level.is_below_rotation_band(player.player_attributes.current_ability, group) {
            return false;
        }
        let sample = SquadEvidenceContext::current_season_sample(date, club);
        if sample.is_early_season() {
            return false;
        }
        if player.player_attributes.is_injured
            || player.player_attributes.is_banned
            || player.player_attributes.is_in_recovery()
        {
            return false;
        }
        let appearances = player.statistics.played
            + player.statistics.played_subs
            + player.cup_statistics.played
            + player.cup_statistics.played_subs;
        u32::from(appearances) * 100
            < u32::from(sample.club_matches_proxy()) * Self::STANDING_SHARE_PCT
    }

    fn is_squad_protected(player: &Player, club: &Club, date: NaiveDate) -> bool {
        // Central squad-asset policy: a core / first-team-useful player —
        // formally designated OR inferred from CA rank, squad-relative
        // reputation, and prior-season minutes even while his monthly squad
        // status is still `NotYetSet` — is kept by the routine numeric
        // sweeps. Explicit player-driven departures (REQ / UNH) and club
        // decisions (NotNeeded, club-listed) are evaluated BEFORE this in
        // `evaluate_player_listing`, so they still override and the club can
        // always sell when it (or the player) actually wants to.
        if SquadAssetProtection::classify(player, club, date).is_first_team_protected() {
            return true;
        }

        // Club has formally labelled the player as core to the project.
        if let Some(ref c) = player.contract {
            if matches!(
                c.squad_status,
                PlayerSquadStatus::KeyPlayer
                    | PlayerSquadStatus::FirstTeamRegular
                    | PlayerSquadStatus::HotProspectForTheFuture
            ) {
                return true;
            }
        }

        // Highest-CA player in his position group on the main team — i.e.
        // the de facto starter. squad_status is updated monthly, so at
        // simulation start (or before the first-of-month tick on a fresh
        // save) every player still has `NotYetSet` and can't be protected
        // via the formal-designation branch above. Without this fallback,
        // the starting goalkeeper at every club was fair game for the
        // numeric listing paths on day one.
        if let Some(main_team) = club.teams.teams.first() {
            let group = player.position().position_group();
            let player_ca = player.player_attributes.current_ability;
            let group_top_ca = main_team
                .players
                .iter()
                .filter(|p| p.position().position_group() == group)
                .filter(|p| !p.is_on_loan())
                .map(|p| p.player_attributes.current_ability)
                .max()
                .unwrap_or(0);
            if player_ca == group_top_ca && group_top_ca > 0 {
                return true;
            }
        }

        let age = player.age(date);

        // Dressing-room leader — strong leadership attribute + seasoned.
        // Skills are on the 1-20 scale; >=15 is genuine locker-room
        // authority, not just any veteran.
        let standing_earned = Self::standing_protection_still_earned(player, club, date)
            && !Self::outgrown_and_barely_playing(player, club, date);

        if age >= 26 && player.skills.mental.leadership >= 15.0 && standing_earned {
            return true;
        }

        // Long-serving pro still delivering: tenure AND last-season form.
        let tenure_years = player
            .contract
            .as_ref()
            .and_then(|c| c.started)
            .map(|start| (date - start).num_days() / 365)
            .unwrap_or(0);

        // Sample-size-regressed: "still delivering" is a season-long
        // judgement; a 5-app farewell season at raw 7.0 shouldn't earn
        // long-tenure protection that a regressed 6.7 wouldn't.
        let pos = player.position().position_group();
        let last_rating = player
            .statistics_history
            .items
            .last()
            .map(|h| h.statistics.average_rating_realistic(pos))
            .unwrap_or(0.0);

        // …and still part of the team: a rating from a handful of games
        // is not "still delivering", it is a farewell season being read
        // as a form line.
        if tenure_years >= 4 && last_rating >= 6.9 && standing_earned {
            return true;
        }

        // Club stalwart — 6+ years regardless of recent form. Deep-backup
        // roles naturally produce thin playing records (and thus no form
        // data or low ratings from few appearances); the tenure+form
        // branch above punishes them unfairly. Six-year loyalty earned
        // patience from the dressing room and, typically, the boardroom.
        if tenure_years >= 6 && standing_earned {
            return true;
        }

        // Experienced goalkeeper — keepers have the longest careers of
        // any position and #2/#3 veterans are kept on specifically to
        // mentor the starter, cover injuries, and anchor the dressing
        // room. Pure CA-vs-squad-average maths lists them every season;
        // real clubs do the opposite. Antonio Chimenti spent eight years
        // as Juventus backup without being listed. Equivalent carve-outs
        // for outfield positions aren't warranted — those roles turn
        // over much faster.
        //
        // Bounded by the same standing test as the branches above, and by
        // the asset classifier's own veteran-keeper rescue, which already
        // checks that he fits inside the club's normal keeper complement
        // and has made peace with deputising. Unconditional, this shield
        // protected the fourth keeper of a glut as firmly as Juventus's
        // long-serving number two.
        let group = player.position().position_group();
        if group == PlayerFieldPositionGroup::Goalkeeper && age >= 30 && standing_earned {
            return true;
        }

        false
    }

    /// What this club expects of a starter, read off its main team's
    /// reputation — the same anchor the recruitment brief shops against.
    fn club_level(club: &Club) -> Option<ClubLevelAnchor> {
        club.teams
            .main()
            .or_else(|| club.teams.teams.first())
            .map(|team| ClubLevelAnchor::for_reputation(team.reputation.overall_score()))
    }

    /// True when somebody in this group on the main team clears the club's
    /// key-player floor — the club has a starter at its own level, so a
    /// squad-mate far below it is depth it can cycle out, not the best it
    /// has.
    fn group_has_starter_at_level(
        club: &Club,
        group: PlayerFieldPositionGroup,
        level: &ClubLevelAnchor,
    ) -> bool {
        club.teams
            .main()
            .or_else(|| club.teams.teams.first())
            .is_some_and(|team| {
                team.players
                    .iter()
                    .filter(|p| !p.is_on_loan())
                    .filter(|p| p.position().position_group() == group)
                    .any(|p| level.meets_key_level(p.player_attributes.current_ability, group))
            })
    }

    /// Returns true if the player's position group already has enough players.
    fn position_group_has_depth(club: &Club, player: &Player, _date: NaiveDate) -> bool {
        let team = match club.teams.teams.first() {
            Some(t) => t,
            None => return false,
        };

        let group = player.position().position_group();
        let group_count = team
            .players
            .iter()
            .filter(|p| p.position().position_group() == group)
            .count();

        let min_to_keep = match group {
            PlayerFieldPositionGroup::Goalkeeper => 2,
            PlayerFieldPositionGroup::Defender => 4,
            PlayerFieldPositionGroup::Midfielder => 4,
            PlayerFieldPositionGroup::Forward => 2,
        };

        group_count > min_to_keep
    }

    fn calculate_asking_price(
        player: &Player,
        club: &Club,
        date: NaiveDate,
        price_level: f32,
        league_reputation: u16,
        club_reputation: u16,
    ) -> CurrencyValue {
        let base_value = PlayerValuationCalculator::calculate_value_with_price_level(
            player,
            date,
            price_level,
            league_reputation,
            club_reputation,
        );

        let multiplier =
            PlayerValuationCalculator::seller_distress_multiplier(club.finance.balance.balance);

        CurrencyValue {
            amount: base_value.amount * multiplier,
            currency: base_value.currency,
        }
    }

    /// The main roster's own numeric evaluation — the one pass allowed to
    /// decide, on the club's behalf, that a player it never flagged is for
    /// sale, on loan, or released.
    fn main_squad_listings(
        country: &Country,
        scope: &ClubListingScope<'_>,
        main_team: &Team,
        squad_analysis: &SquadAnalysis,
        current_window: Option<(NaiveDate, NaiveDate)>,
        listings_to_add: &mut Vec<PendingListing>,
        listings_to_upgrade: &mut Vec<(u32, CurrencyValue)>,
    ) {
        let club = scope.club;
        let date = scope.date;
        let price_level = scope.price_level;
        let league_reputation = scope.league_reputation;
        let club_reputation = scope.club_reputation;
        let decided_by = &scope.decided_by;

        for player in &main_team.players.players {
            let presence = MarketPresence::of(&country.transfer_market, player.id);
            match ListingPass::evaluate_player_listing(
                player,
                &squad_analysis,
                club,
                date,
                current_window,
                presence,
            ) {
                ListingDecision::Keep => {}
                ListingDecision::UpgradeLoanToTransfer => {
                    let asking_price = ListingPass::calculate_asking_price(
                        player,
                        club,
                        date,
                        price_level,
                        league_reputation,
                        club_reputation,
                    );
                    listings_to_upgrade.push((player.id, asking_price));
                }
                ListingDecision::FreeTransfer => {
                    let free_price = CurrencyValue {
                        amount: 0.0,
                        currency: Currency::Usd,
                    };
                    listings_to_add.push(PendingListing {
                        player_id: player.id,
                        club_id: club.id,
                        team_id: main_team.id,
                        asking_price: free_price,
                        listing_type: TransferListingType::EndOfContract,
                        reason: "dec_reason_under16_release".to_string(),
                        decided_by: decided_by.clone(),
                    });
                }
                ListingDecision::Transfer { reason } => {
                    let asking_price = ListingPass::calculate_asking_price(
                        player,
                        club,
                        date,
                        price_level,
                        league_reputation,
                        club_reputation,
                    );
                    listings_to_add.push(PendingListing {
                        player_id: player.id,
                        club_id: club.id,
                        team_id: main_team.id,
                        asking_price,
                        listing_type: TransferListingType::Transfer,
                        reason,
                        decided_by: decided_by.clone(),
                    });
                }
                ListingDecision::Loan { reason } => {
                    listings_to_add.push(PendingListing {
                        player_id: player.id,
                        club_id: club.id,
                        team_id: main_team.id,
                        asking_price: CurrencyValue {
                            amount: 0.0,
                            currency: Currency::Usd,
                        },
                        listing_type: TransferListingType::Loan,
                        reason,
                        decided_by: decided_by.clone(),
                    });
                }
            }
        }
    }

    /// Explicit club listings outside the main squad. The numeric evaluation
    /// deliberately reads only the main roster — its triggers measure players
    /// against main-squad analysis and must not auto-list reserve or youth
    /// players — but a badge another system stamped, and a formal request a
    /// senior reserve made, must still become a market row, or the player is
    /// stranded flagged-but-invisible to every buyer.
    fn reserve_squad_listings(
        country: &Country,
        scope: &ClubListingScope<'_>,
        listings_to_add: &mut Vec<PendingListing>,
        listings_to_upgrade: &mut Vec<(u32, CurrencyValue)>,
    ) {
        let club = scope.club;
        let date = scope.date;
        let price_level = scope.price_level;
        let league_reputation = scope.league_reputation;
        let club_reputation = scope.club_reputation;
        let decided_by = &scope.decided_by;

        // Explicit club listings outside the main squad. The evaluation
        // above deliberately reads only the main roster — its numeric
        // triggers measure players against main-squad analysis and must
        // not auto-list reserve/youth players — but other systems (the
        // season-start surplus trim) flag players across every team via
        // `contract.is_transfer_listed`. Those flags must still become
        // market listings, carrying the player's real team, or the
        // player is stranded flagged-but-invisible to every buyer.
        for team in club.teams.teams.iter().skip(1) {
            for player in &team.players.players {
                if player.is_on_loan() || player.is_force_match_selection {
                    continue;
                }
                // Board loan flag (`Loa`) on a reserve/youth player —
                // stamped by the squad-utilization audit, the surplus
                // demotion, or an accepted loan-request talk — must
                // become a real loan listing, or the badge is cosmetic
                // and no club can ever bid (the numeric evaluation above
                // reads only the main roster, so these players are
                // otherwise stranded off-market). Idempotent via the
                // existing-listing guard; the flag-setter already wrote
                // the decision-history entry, so `dec_reason_club_listed`
                // suppresses a duplicate. Fee mirrors the main-squad
                // board loan listing (zero — the borrower-side scan sets
                // the actual terms), keeping the path consistent.
                if player.statuses.has(PlayerStatusType::Loa) && player.contract.is_some() {
                    match country.transfer_market.get_listing_by_player(player.id) {
                        None => {
                            listings_to_add.push(PendingListing {
                                player_id: player.id,
                                club_id: club.id,
                                team_id: team.id,
                                asking_price: CurrencyValue {
                                    amount: 0.0,
                                    currency: Currency::Usd,
                                },
                                listing_type: TransferListingType::Loan,
                                reason: "dec_reason_club_listed".to_string(),
                                decided_by: decided_by.clone(),
                            });
                        }
                        Some(existing) => {
                            // A loan listing that found no borrower for
                            // half a year, on a player the club has ALSO
                            // flagged for permanent sale, upgrades to a
                            // real transfer listing. Without this the
                            // `Loa` badge was a life sentence: the loan
                            // row never expires, the flagged-for-sale
                            // branch below skips loan-listed players, and
                            // the unsold-exit valve only reads permanent
                            // listings — so a warehoused reserve (the
                            // 29-keeper U20 case) could never leave by
                            // any route. The original listed date is
                            // kept, so a long-stranded player reaches the
                            // valve's one-year clock immediately instead
                            // of restarting it.
                            let flagged_for_sale = player
                                .contract
                                .as_ref()
                                .map(|c| c.is_transfer_listed)
                                .unwrap_or(false);
                            if existing.listing_type == TransferListingType::Loan
                                && flagged_for_sale
                                && (date - existing.listed_date).num_days()
                                    >= LOAN_UNSOLD_UPGRADE_DAYS
                            {
                                let asking_price = ListingPass::calculate_asking_price(
                                    player,
                                    club,
                                    date,
                                    price_level,
                                    league_reputation,
                                    club_reputation,
                                );
                                listings_to_upgrade.push((player.id, asking_price));
                            }
                        }
                    }
                    continue;
                }

                // Player-initiated departures from a squad below the
                // first team. The numeric evaluation above deliberately
                // reads only the main roster — its triggers measure a
                // player against main-squad analysis and must not
                // auto-list reserves — but a formal request is not a
                // numeric trigger. It is the player's own decision, and
                // a senior reserve squad is exactly where the
                // reserve-ambition audit produces one.
                //
                // Without this the request was a closed loop: the audit
                // fired, the manager talk failed, `Req` was stamped, no
                // listing pass could see it, and the weekly desire tick
                // then cleared the status again for want of a live
                // reason. The player asked to leave every month for
                // years and nothing ever happened.
                //
                // Youth squads stay out of it: a boy asking for football
                // is a development-loan case, which the pathway owns.
                let is_senior_reserve = team.team_type.is_senior_reserve();
                let is_youth_contract = player
                    .contract
                    .as_ref()
                    .map(|c| c.contract_type == ContractType::Youth)
                    .unwrap_or(false);
                if is_senior_reserve && !is_youth_contract && player.contract.is_some() {
                    let requested = player.statuses.has(PlayerStatusType::Req);
                    let long_unhappy = player
                        .statuses
                        .held_for_days(PlayerStatusType::Unh, date)
                        .is_some_and(|days| days >= UNHAPPY_LISTING_MIN_DAYS);
                    let already_on_market = player.statuses.has(PlayerStatusType::Lst)
                        || player.statuses.has(PlayerStatusType::Frt);
                    if (requested || long_unhappy) && !already_on_market {
                        let asking_price = ListingPass::calculate_asking_price(
                            player,
                            club,
                            date,
                            price_level,
                            league_reputation,
                            club_reputation,
                        );
                        listings_to_add.push(PendingListing {
                            player_id: player.id,
                            club_id: club.id,
                            team_id: team.id,
                            asking_price,
                            listing_type: TransferListingType::Transfer,
                            reason: if requested {
                                "dec_reason_player_requested".to_string()
                            } else {
                                "dec_reason_player_unhappy".to_string()
                            },
                            decided_by: decided_by.clone(),
                        });
                        continue;
                    }
                }

                // Explicit permanent club listings: the season-start
                // surplus trim flags players across every team via
                // `contract.is_transfer_listed`. Those flags must still
                // become market listings, carrying the player's real
                // team, or the player is stranded flagged-but-invisible.
                let flagged = player
                    .contract
                    .as_ref()
                    .map(|c| c.is_transfer_listed)
                    .unwrap_or(false);
                if !flagged {
                    continue;
                }
                if player.statuses.has(PlayerStatusType::Lst)
                    || player.statuses.has(PlayerStatusType::Loa)
                    || player.statuses.has(PlayerStatusType::Frt)
                {
                    continue;
                }
                let asking_price = ListingPass::calculate_asking_price(
                    player,
                    club,
                    date,
                    price_level,
                    league_reputation,
                    club_reputation,
                );
                listings_to_add.push(PendingListing {
                    player_id: player.id,
                    club_id: club.id,
                    team_id: team.id,
                    asking_price,
                    listing_type: TransferListingType::Transfer,
                    reason: "dec_reason_club_listed".to_string(),
                    decided_by: decided_by.clone(),
                });
            }
        }
    }

    /// Put the collected decisions on the market: a row, a badge, the
    /// decision-history line the flag-setter did not already write, and — for
    /// a club-decided sale nobody asked for — the conversation it deserves.
    fn apply_listings(
        country: &mut Country,
        date: NaiveDate,
        summary: &mut TransferActivitySummary,
        listings_to_add: Vec<PendingListing>,
    ) {
        // Apply listings
        for listing_data in listings_to_add {
            let status_type = match listing_data.listing_type {
                TransferListingType::Loan => PlayerStatusType::Loa,
                TransferListingType::EndOfContract => PlayerStatusType::Frt,
                _ => PlayerStatusType::Lst,
            };

            let movement = match listing_data.listing_type {
                TransferListingType::Loan => "dec_loan_listed",
                TransferListingType::EndOfContract => "dec_free_transfer_listed",
                _ => "dec_transfer_listed",
            };

            // Captured before `listing_data.listing_type` is moved into the
            // listing below — an end-of-contract listing is the under-16
            // free release and is the only producer of this listing type.
            let is_under16_release = matches!(
                listing_data.listing_type,
                TransferListingType::EndOfContract
            );

            let listing = TransferListing::new(
                listing_data.player_id,
                listing_data.club_id,
                listing_data.team_id,
                listing_data.asking_price,
                date,
                listing_data.listing_type,
            );

            country.transfer_market.add_listing(listing);
            summary.total_listings += 1;

            for club in &mut country.clubs {
                for team in &mut club.teams.teams {
                    if let Some(player) = team
                        .players
                        .players
                        .iter_mut()
                        .find(|p| p.id == listing_data.player_id)
                    {
                        if !player.statuses.has(status_type) {
                            player.statuses.add(date, status_type);
                        }
                        // An end-of-contract listing is the under-16 free
                        // release. Record the explicit origin so when the
                        // deal lapses the free-agent sweep labels the
                        // departure as an under-16 release rather than
                        // falling back to a generic mutual agreement.
                        if is_under16_release {
                            player.set_release_reason(FreeAgentReleaseReason::Under16Release);
                        }
                        // `dec_reason_club_listed` materializes a flag another
                        // system set (`contract.is_transfer_listed`) — that
                        // system already wrote the decision-history entry with
                        // the real reason (surplus trim, salary fallback) when
                        // it flagged the player. Adding a second, vaguer entry
                        // here duplicated the decision on the player page; the
                        // flag-setter owns the history.
                        if listing_data.reason != "dec_reason_club_listed" {
                            player.decision_history.add(
                                date,
                                movement.to_string(),
                                listing_data.reason.clone(),
                                listing_data.decided_by.clone(),
                            );
                        }
                        // A CLUB-decision transfer listing for a player who
                        // never asked out is the "you're not in my plans"
                        // conversation — say it to his face instead of
                        // letting him find out from the transfer page.
                        // Player-initiated listings (his own request, his
                        // own hardened unhappiness) need no telling.
                        let player_initiated = listing_data.reason == "dec_reason_player_requested"
                            || listing_data.reason == "dec_reason_player_unhappy";
                        if status_type == PlayerStatusType::Lst
                            && !player_initiated
                            && !is_under16_release
                        {
                            let magnitude = HappinessConfig::default().catalog.told_not_in_plans;
                            player.happiness.add_event_with_cooldown(
                                HappinessEventType::ToldNotInPlans,
                                magnitude,
                                180,
                            );
                        }
                    }
                }
            }
        }
    }

    /// Upgrade stale loan listings to permanent listings. In-place: same row,
    /// same `listed_date` — only the type, origin and asking price change, so
    /// the unsold-exit valve's clock keeps the time already served on the
    /// loan list.
    fn upgrade_stale_loan_listings(
        country: &mut Country,
        date: NaiveDate,
        summary: &mut TransferActivitySummary,
        listings_to_upgrade: Vec<(u32, CurrencyValue)>,
    ) {
        // Upgrade stale loan listings to permanent listings (see the
        // collection above). In-place: same row, same `listed_date` — only
        // the type, origin and asking price change, so the unsold-exit
        // valve's clock keeps the time already served on the loan list.
        for (player_id, asking_price) in listings_to_upgrade {
            let Some(listing) = country.transfer_market.listings.iter_mut().find(|l| {
                l.player_id == player_id
                    && l.listing_type == TransferListingType::Loan
                    && l.status == TransferListingStatus::Available
            }) else {
                continue;
            };
            listing.listing_type = TransferListingType::Transfer;
            listing.origin = TransferListingOrigin::SellerListed;
            listing.asking_price = asking_price.clone();
            listing.original_asking_price = asking_price;
            summary.total_listings += 1;
            for club in &mut country.clubs {
                for team in &mut club.teams.teams {
                    if let Some(player) =
                        team.players.players.iter_mut().find(|p| p.id == player_id)
                    {
                        if !player.statuses.has(PlayerStatusType::Lst) {
                            player.statuses.add(date, PlayerStatusType::Lst);
                        }
                        // No decision-history entry: the flag-setter (surplus
                        // trim / salary fallback) already recorded the listing
                        // decision when it stamped `is_transfer_listed`.
                    }
                }
            }
        }
    }

    /// The four states in which the question does not arise at all: he belongs
    /// to somebody else, the manager pinned him, he arrived in this very
    /// window, or he is already on the market.
    fn already_settled(
        player: &Player,
        current_window: Option<(NaiveDate, NaiveDate)>,
        presence: MarketPresence,
    ) -> Option<ListingDecision> {
        // Loan players belong to another club — cannot be listed by the loan club
        if player.is_on_loan() {
            return Some(ListingDecision::Keep);
        }

        // Manager has pinned this player to the squad — never auto-list.
        // The pin only protects contracted players; once the contract
        // ends the player is a free agent and must be free to move.
        if player.is_force_match_selection && player.contract.is_some() {
            return Some(ListingDecision::Keep);
        }

        // Same-window protection: signed during this open window → can't be listed
        if let (Some(transfer_date), Some((window_start, window_end))) =
            (player.last_transfer_date, current_window)
        {
            if transfer_date >= window_start && transfer_date <= window_end {
                return Some(ListingDecision::Keep);
            }
        }

        // Already on the market — read the market rows, not the badges.
        // `Lst` / `Loa` are claims about the market and this pass is what
        // makes them true: the board audit stamps the badge the day it
        // decides. Treating the badge as proof of a row stranded every
        // board-listed main-squad player — badge, no row, nothing for a
        // buyer, the seller push or the unsold-exit valve to read — while
        // the reconcile stripped the badge again and the renewal manager
        // saw a clean player. A free-transfer release is a decision
        // already made.
        if presence.for_sale || player.statuses.has(PlayerStatusType::Frt) {
            return Some(ListingDecision::Keep);
        }

        None
    }

    /// He already has a row, or a plan protecting him from getting one.
    fn market_row_verdict(
        player: &Player,
        date: NaiveDate,
        presence: MarketPresence,
        flagged_for_sale: bool,
        labelled_not_needed: bool,
    ) -> Option<ListingDecision> {
        if let Some(loan_listed_since) = presence.loan_listed_since {
            // On the loan market. The row upgrades in place to a permanent
            // listing — the reserve branch's rule, on the same clock —
            // when the player himself wants out, or when the club has also
            // decided to sell and half a year has found no borrower.
            // Otherwise the loan market keeps him.
            let player_wants_out = player.statuses.has(PlayerStatusType::Req)
                || player
                    .statuses
                    .held_for_days(PlayerStatusType::Unh, date)
                    .is_some_and(|days| days >= UNHAPPY_LISTING_MIN_DAYS);
            let club_wants_sale = flagged_for_sale
                || player.statuses.has(PlayerStatusType::Lst)
                || labelled_not_needed;
            let unsold_for_months =
                (date - loan_listed_since).num_days() >= LOAN_UNSOLD_UPGRADE_DAYS;
            if player_wants_out || (club_wants_sale && unsold_for_months) {
                return Some(ListingDecision::UpgradeLoanToTransfer);
            }
            return Some(ListingDecision::Keep);
        }

        // Club signing plan: the club bought this player with intent and is
        // still inside the evaluation window it committed to. Same helper
        // the weekly rebalance / season trim / idle-days audit consult, so
        // every automatic surplus mechanism honours one patience clock.
        if player.signing_protection_active(date) {
            return Some(ListingDecision::Keep);
        }

        None
    }

    /// What the evaluation pipeline and the player himself have already
    /// settled: a standing loan-out candidacy, or a formal transfer request.
    /// The request outranks any persisted club decision - the handler also
    /// sets `contract.is_transfer_listed`, so reading the flag first used to
    /// mislabel a player's own exit as a club one.
    fn plan_and_request_verdict(player: &Player, club: &Club) -> Option<ListingDecision> {
        // Check if evaluation pipeline already identified as loan candidate
        let loan_candidate = club
            .transfer_plan
            .loan_out_candidates
            .iter()
            .find(|c| c.player_id == player.id);

        if let Some(candidate) = loan_candidate {
            // The board audit stamps `Loa` and writes the decision-history
            // row when it adds the candidate; materialising that badge here
            // must not add a second, vaguer entry.
            if player.statuses.has(PlayerStatusType::Loa) {
                return Some(ListingDecision::Loan {
                    reason: "dec_reason_club_listed".to_string(),
                });
            }
            let reason = match &candidate.reason {
                LoanOutReason::NeedsGameTime => "dec_reason_needs_game_time",
                LoanOutReason::BlockedByBetterPlayer => "dec_reason_blocked_by_better",
                LoanOutReason::Surplus => "dec_reason_surplus_tactical",
                LoanOutReason::FinancialRelief => "dec_reason_financial_relief",
                LoanOutReason::LackOfPlayingTime => "dec_reason_lack_playing_time",
                LoanOutReason::PostInjuryFitness => "dec_reason_post_injury_fitness",
                LoanOutReason::DevelopmentPathway => "dec_reason_development_pathway",
                // Stalled-prospect pathway reasons carry their own keys so
                // UI diagnostics can tell "blocked by depth" from "needs
                // first-team minutes" from "protecting resale value".
                LoanOutReason::BlockedByDepth => "dec_reason_blocked_by_depth",
                LoanOutReason::NeedsFirstTeamMinutes => "dec_reason_needs_first_team_minutes",
                LoanOutReason::AssetValueProtection => "dec_reason_asset_value_protection",
                LoanOutReason::UnsettledAbroad => "dec_reason_unsettled_abroad",
            };
            return Some(ListingDecision::Loan {
                reason: reason.to_string(),
            });
        }

        // Player-initiated departures outrank persisted club decisions: a
        // player who formally requested out (or hardened into Unh) is
        // listed under his own reason and exempted from the position-group
        // minimums. The transfer-request handler also sets
        // `contract.is_transfer_listed`, so checking the flag first used to
        // mislabel these as "club listed".
        if player.statuses.has(PlayerStatusType::Req) {
            return Some(ListingDecision::Transfer {
                reason: "dec_reason_player_requested".to_string(),
            });
        }

        None
    }

    /// Unhappiness is not, on its own, a reason to sell - the formal `Unh`
    /// status is also reached by playing-time frustration, and shipping such
    /// a player out is the wrong response. Only a grievance the club has had
    /// a full half-season to fix escalates; before then it routes by squad
    /// value.
    fn unhappiness_verdict(
        player: &Player,
        club: &Club,
        date: NaiveDate,
    ) -> Option<ListingDecision> {
        // Unhappiness is not, on its own, a reason to sell. The formal
        // `Unh` status is also reached by playing-time frustration — a
        // benched but still-useful squad member — and shipping such a
        // player out is the wrong response: the manager-talk and loan
        // paths own him. We only treat the unhappiness as a sell signal
        // once it has held for 6+ months (`UNHAPPY_LISTING_MIN_DAYS`)
        // without resolving — a sustained grievance the club has had a
        // full half-season to fix. The same threshold gates the player's
        // own transfer request, so the two systems escalate together.
        // Before then a playing-time complaint routes by squad value:
        // useful seniors / rotation and not-yet-evaluated players are kept,
        // a development-profile youngster is loaned for minutes, and only a
        // genuinely surplus unhappy player is actually transfer-listed.
        if player.statuses.has(PlayerStatusType::Unh) {
            let unhappy_days = player
                .statuses
                .held_for_days(PlayerStatusType::Unh, date)
                .unwrap_or(0);
            if unhappy_days >= UNHAPPY_LISTING_MIN_DAYS {
                return Some(ListingDecision::Transfer {
                    reason: "dec_reason_player_unhappy".to_string(),
                });
            }
            return Some(match SquadAssetProtection::classify(player, club, date) {
                SquadAssetClass::CorePlayer
                | SquadAssetClass::FirstTeamUseful
                | SquadAssetClass::RotationUseful
                | SquadAssetClass::UnknownNeedsEvaluation => ListingDecision::Keep,
                // A grumbling prospect goes out for minutes — but the
                // club's own first choice in that shirt is not a prospect
                // whatever his birth year says, and unhappiness is not the
                // club's cue to lend him away.
                SquadAssetClass::ProspectDevelopment => {
                    if LoanAssetGuard::parent_holds_for(club, player, date) {
                        ListingDecision::Keep
                    } else {
                        ListingDecision::Loan {
                            reason: "dec_reason_young_needs_practice".to_string(),
                        }
                    }
                }
                SquadAssetClass::TrueSurplus => ListingDecision::Transfer {
                    reason: "dec_reason_player_unhappy".to_string(),
                },
            });
        }

        None
    }

    /// Decisions the club has already made and this pass turns into a market
    /// row - held back while a just-appointed head coach reviews the squad,
    /// because the new manager gets to disown the old regime's exits.
    fn club_decision_verdict(
        player: &Player,
        club: &Club,
        date: NaiveDate,
        flagged_for_sale: bool,
        labelled_not_needed: bool,
        reading: ListingReading,
    ) -> Option<ListingDecision> {
        let avg = reading.avg;
        let rep_level = reading.rep_level;
        let parent_holds = reading.parent_holds;

        // A just-appointed head coach reviews the squad before honouring
        // the old regime's exit decisions — no NEW club-driven listings
        // during the review window. The player-initiated paths above
        // (formal request, long unhappiness) keep their course: the new
        // manager can't make a player un-ask to leave.
        if club
            .transfer_plan
            .manager_review_until
            .map(|until| date < until)
            .unwrap_or(false)
        {
            return Some(ListingDecision::Keep);
        }

        // Club decisions recorded on the player but not yet on the market:
        // the contract flag (surplus trim, salary fallback, the board audit)
        // or a bare badge the board audit stamped the day it decided.
        // `dec_reason_club_listed` writes no history — the decider already
        // did. Checked before the `NotNeeded` label: a badge is a concrete
        // listing verdict, the label is what this pass turns into one when
        // nobody else has.
        if flagged_for_sale || player.statuses.has(PlayerStatusType::Lst) {
            return Some(ListingDecision::Transfer {
                reason: "dec_reason_club_listed".to_string(),
            });
        }
        if player.statuses.has(PlayerStatusType::Loa) {
            return Some(ListingDecision::Loan {
                reason: "dec_reason_club_listed".to_string(),
            });
        }
        if labelled_not_needed {
            return Some(ListingPass::decide_listing_type(
                player,
                &rep_level,
                avg,
                date,
                parent_holds,
                "dec_reason_surplus_squad".to_string(),
            ));
        }

        None
    }

    /// Every gate but this one measures a player against his squad-mates, and
    /// none of them asks what the club expects of a starter. The buy side
    /// asks exactly that when it briefs a shirt, so a giant could decide its
    /// striker was below the standard it recruits at, buy better, and then
    /// keep the man for years because he was never twenty-five points under
    /// the squad mean.
    fn below_club_level(
        player: &Player,
        club: &Club,
        date: NaiveDate,
        reading: ListingReading,
    ) -> Option<ListingDecision> {
        let ca = reading.ca_i as u8;
        let avg = reading.avg;
        let is_promising_youth = reading.is_promising_youth;
        let rep_level = reading.rep_level;
        let parent_holds = reading.parent_holds;

        // Below the CLUB's level. Every gate from here down measures a
        // player against his squad-mates — the squad average, a surplus
        // position, his age — and none of them asked what this club
        // expects of a starter. The buy side asks exactly that when it
        // briefs a shirt, so a giant could decide its striker was below
        // the standard it recruits at, buy better, and then keep the man
        // for years because he was never twenty-five points under the
        // squad mean. Same anchor the brief shops against; listable only
        // once the club has somebody in the group who does clear its
        // level (a squad that is weak everywhere has nothing to cycle
        // him out for), once the season has produced a sample, and never
        // below the group's depth floor.
        if let Some(level) = ListingPass::club_level(club) {
            let group = player.position().position_group();
            if !is_promising_youth
                && level.is_below_rotation_band(ca, group)
                && !SquadEvidenceContext::current_season_sample(date, club).is_early_season()
                && ListingPass::group_has_starter_at_level(club, group, &level)
                && ListingPass::position_group_has_depth(club, player, date)
            {
                return Some(ListingPass::decide_listing_type(
                    player,
                    &rep_level,
                    avg,
                    date,
                    parent_holds,
                    "dec_reason_below_club_level".to_string(),
                ));
            }
        }

        None
    }

    /// The measurements that can list a player nobody decided anything about:
    /// too far below the squad, surplus in his group, aging past the point
    /// his club cycles, one of too many bodies, or a renewal that has failed.
    fn numeric_listing_triggers(
        player: &Player,
        analysis: &SquadAnalysis,
        club: &Club,
        date: NaiveDate,
        reading: ListingReading,
    ) -> Option<ListingDecision> {
        let age = reading.age;
        let ca_i = reading.ca_i;
        let avg = reading.avg;
        let is_promising_youth = reading.is_promising_youth;
        let rep_level = reading.rep_level;
        let parent_holds = reading.parent_holds;
        let affordability = reading.affordability;

        // Wealth-aware quality gap threshold — shared with the buy-side
        // squad-fit gate so selling and buying agree on what "too far
        // below the squad" means.
        let quality_gap_threshold: i16 = rep_level.surplus_quality_gap();

        // Well below squad average
        if analysis.quality_level > 15 && ca_i < avg - quality_gap_threshold && !is_promising_youth
        {
            if !ListingPass::position_group_has_depth(club, player, date) {
                return Some(ListingDecision::Keep);
            }
            return Some(ListingPass::decide_listing_type(
                player,
                &rep_level,
                avg,
                date,
                parent_holds,
                "dec_reason_well_below_avg".to_string(),
            ));
        }

        // Surplus position and below average
        let player_group = player.position().position_group();
        for surplus_pos in &analysis.surplus_positions {
            if surplus_pos.position_group() == player_group {
                if ca_i < avg && !is_promising_youth {
                    return Some(ListingPass::decide_listing_type(
                        player,
                        &rep_level,
                        avg,
                        date,
                        parent_holds,
                        "dec_reason_below_avg_surplus".to_string(),
                    ));
                }
            }
        }

        // Aging players past their prime — only top clubs cycle aging
        // squad-average players out. Smaller clubs keep them to the end of
        // their careers: loyalty, shorter shopping lists, a 35-year-old
        // stalwart at a regional club is a feature, not a problem.
        if rep_level.cycles_aging_squad() {
            let aging_threshold = ListingBars::aging(player.position().position_group());
            if age >= aging_threshold && ca_i < avg + 5 {
                return Some(ListingDecision::Transfer {
                    reason: "dec_reason_aging_declining".to_string(),
                });
            }
        }

        // Below-average players in large squads — wealth-aware threshold
        let squad_size = club
            .teams
            .teams
            .first()
            .map(|t| t.players.players.len())
            .unwrap_or(0);
        let max_comfortable_squad = match rep_level {
            ReputationLevel::Elite => 45,
            ReputationLevel::Continental => 40,
            ReputationLevel::National => 32,
            ReputationLevel::Regional => 26,
            _ => 22,
        };

        if squad_size > max_comfortable_squad && ca_i < avg - 10 && !is_promising_youth {
            return Some(ListingPass::decide_listing_type(
                player,
                &rep_level,
                avg,
                date,
                parent_holds,
                "dec_reason_squad_oversized".to_string(),
            ));
        }

        // Contract stalemate. The renewal manager has already had its
        // window to lock this player down; if it has tried and failed
        // (rejections in the last 365 days) we treat that — not the
        // bare expiry date — as the listing trigger. Pure expiry
        // without failed renewal evidence is intentionally NOT a
        // listing reason: that would conflict with the AI transfer-list
        // prompt and pre-empt the renewal flow on players the club
        // actually wants to keep.
        let stalemate = ContractStalemate::assess(player, date, affordability);
        if stalemate.rejections_12m > 0 && stalemate.permits_listing() {
            return Some(ListingDecision::Transfer {
                reason: "dec_reason_contract_stalemate".to_string(),
            });
        }

        None
    }
}

/// The two per-group bars the listing pass is written against: when a
/// player is old enough to be let go, and how thin a group may get before
/// the club stops letting anyone go at all.
struct ListingBars;

impl ListingBars {
    /// Age at which a mid-tier player at or below squad average is
    /// considered "past his prime" for transfer-listing purposes. Mirrors
    /// real-world career lengths: keepers last longest, forwards
    /// (speed-dependent) decline first, defenders and holding midfielders
    /// sit in between.
    fn aging(group: PlayerFieldPositionGroup) -> u8 {
        match group {
            PlayerFieldPositionGroup::Goalkeeper => 37,
            PlayerFieldPositionGroup::Defender => 34,
            PlayerFieldPositionGroup::Midfielder => 33,
            PlayerFieldPositionGroup::Forward => 32,
        }
    }

    /// Minimum number of main-team players a club must retain per position
    /// group after any club-decided transfer/loan listings in a single
    /// pass. Player-initiated listings (REQ/UNH) bypass this cap.
    fn min_squad(group: PlayerFieldPositionGroup) -> usize {
        match group {
            PlayerFieldPositionGroup::Goalkeeper => 2,
            PlayerFieldPositionGroup::Defender => 6,
            PlayerFieldPositionGroup::Midfielder => 6,
            PlayerFieldPositionGroup::Forward => 3,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::academy::ClubAcademy;
    use crate::club::player::core::builder::PlayerBuilder;
    use crate::league::{DayMonthPeriod, League, LeagueCollection, LeagueSettings, Season};
    use crate::shared::Location;
    use crate::shared::fullname::FullName;
    use crate::transfers::pipeline::{LoanDestinationPreference, LoanOutCandidate, LoanOutStatus};
    use crate::{
        ClubColors, ClubFacilities, ClubFinances, ClubStatus, PersonAttributes, PlayerAttributes,
        PlayerClubContract, PlayerCollection, PlayerPosition, PlayerPositionType, PlayerPositions,
        PlayerSkills, PlayerStatistics, PlayerStatisticsHistoryItem, StaffCollection, Team,
        TeamBuilder, TeamCollection, TeamReputation, TeamType, TrainingSchedule,
    };
    use chrono::{NaiveDate, NaiveTime};

    /// Fixtures for the listing pass: one club (id 100) in one league
    /// (id 1); teams and rosters vary per scenario.
    struct Fixture;

    impl Fixture {
        fn date(y: i32, m: u32, day: u32) -> NaiveDate {
            NaiveDate::from_ymd_opt(y, m, day).unwrap()
        }

        fn training_schedule() -> TrainingSchedule {
            TrainingSchedule::new(
                NaiveTime::from_hms_opt(9, 0, 0).unwrap(),
                NaiveTime::from_hms_opt(15, 0, 0).unwrap(),
            )
        }

        fn team(id: u32, slug: &str, team_type: TeamType, players: Vec<Player>) -> Team {
            TeamBuilder::new()
                .id(id)
                .league_id(Some(1))
                .club_id(100)
                .name(slug.to_string())
                .slug(slug.to_string())
                .team_type(team_type)
                .players(PlayerCollection::new(players))
                .staffs(StaffCollection::new(Vec::new()))
                .reputation(TeamReputation::new(500, 500, 500))
                .training_schedule(Self::training_schedule())
                .build()
                .unwrap()
        }

        fn club(teams: Vec<Team>) -> Club {
            Club::new(
                100,
                "Club".to_string(),
                Location::new(1),
                ClubFinances::new(1_000_000, Vec::new()),
                ClubAcademy::new(3),
                ClubStatus::Professional,
                ClubColors::default(),
                TeamCollection::new(teams),
                ClubFacilities::default(),
            )
        }

        fn country(club: Club) -> Country {
            let league = League::new(
                1,
                "L".to_string(),
                "l".to_string(),
                1,
                500,
                LeagueSettings {
                    season_starting_half: DayMonthPeriod::new(1, 8, 31, 12),
                    season_ending_half: DayMonthPeriod::new(1, 1, 31, 5),
                    tier: 1,
                    promotion_spots: 0,
                    relegation_spots: 0,
                    league_group: None,
                    split_season: false,
                },
                false,
            );
            Country::builder()
                .id(1)
                .code("EN".to_string())
                .slug("en".to_string())
                .name("England".to_string())
                .continent_id(1)
                .leagues(LeagueCollection::new(vec![league]))
                .clubs(vec![club])
                .build()
                .unwrap()
        }

        fn player(id: u32) -> Player {
            let mut attrs = PlayerAttributes::default();
            attrs.current_ability = 130;
            attrs.potential_ability = 140;
            let mut contract = PlayerClubContract::new(50_000, Self::date(2026, 9, 1));
            contract.squad_status = PlayerSquadStatus::FirstTeamRegular;
            PlayerBuilder::new()
                .id(id)
                .full_name(FullName::new("Test".into(), format!("Player{}", id)))
                .birth_date(Self::date(1995, 1, 1))
                .country_id(1)
                .attributes(PersonAttributes::default())
                .skills(PlayerSkills::default())
                .positions(PlayerPositions {
                    positions: vec![PlayerPosition {
                        position: PlayerPositionType::MidfielderCenter,
                        level: 20,
                    }],
                })
                .player_attributes(attrs)
                .contract(Some(contract))
                .build()
                .unwrap()
        }
    }

    /// A `Lst` badge no longer backed by a listing or the
    /// `is_transfer_listed` flag is cleared, so a delisted player never shows
    /// a stale "Transfer Listed" — the guard behind the "11× Transfer Listed"
    /// report. Genuinely-listed players (flagged, or with a live listing) keep
    /// their badge.
    #[test]
    fn reconcile_clears_stale_transfer_badge_but_keeps_backed_ones() {
        let today = Fixture::date(2026, 5, 1);
        let mut stale = Fixture::player(101);
        stale.statuses.add(today, PlayerStatusType::Lst);
        let mut flagged = Fixture::player(102);
        flagged.statuses.add(today, PlayerStatusType::Lst);
        flagged.contract.as_mut().unwrap().is_transfer_listed = true;
        let mut listed = Fixture::player(103);
        listed.statuses.add(today, PlayerStatusType::Lst);

        let club = Fixture::club(vec![Fixture::team(
            10,
            "main",
            TeamType::Main,
            vec![stale, flagged, listed],
        )]);
        let mut country = Fixture::country(club);
        country.transfer_market.add_listing(TransferListing::new(
            103,
            100,
            10,
            CurrencyValue::new(1_000_000.0, Currency::Usd),
            today,
            TransferListingType::Transfer,
        ));

        ListingPass::reconcile_stale_market_statuses(&mut country);

        let has = |id: u32, s: PlayerStatusType| {
            country.clubs[0].teams.teams[0]
                .players
                .players
                .iter()
                .find(|p| p.id == id)
                .unwrap()
                .statuses
                .has(s)
        };
        assert!(
            !has(101, PlayerStatusType::Lst),
            "an unbacked badge must be cleared"
        );
        assert!(
            has(102, PlayerStatusType::Lst),
            "a flagged player keeps his badge"
        );
        assert!(
            has(103, PlayerStatusType::Lst),
            "an actively-listed player keeps his badge"
        );
    }

    /// `Loa` mirrors `Lst`: a badge with no loan listing and no live loan-out
    /// candidate is stale and cleared; a candidate or an active loan listing
    /// keeps it.
    #[test]
    fn reconcile_clears_stale_loan_badge_but_keeps_candidates_and_listings() {
        let today = Fixture::date(2026, 5, 1);
        let mut stale = Fixture::player(201);
        stale.statuses.add(today, PlayerStatusType::Loa);
        let mut candidate = Fixture::player(202);
        candidate.statuses.add(today, PlayerStatusType::Loa);
        let mut listed = Fixture::player(203);
        listed.statuses.add(today, PlayerStatusType::Loa);

        let mut club = Fixture::club(vec![Fixture::team(
            10,
            "main",
            TeamType::Main,
            vec![stale, candidate, listed],
        )]);
        club.transfer_plan
            .loan_out_candidates
            .push(LoanOutCandidate {
                player_id: 202,
                reason: LoanOutReason::LackOfPlayingTime,
                status: LoanOutStatus::Listed,
                loan_fee: 0.0,
                preferred_destination: LoanDestinationPreference::Any,
            });
        let mut country = Fixture::country(club);
        country.transfer_market.add_listing(TransferListing::new(
            203,
            100,
            10,
            CurrencyValue::new(0.0, Currency::Usd),
            today,
            TransferListingType::Loan,
        ));

        ListingPass::reconcile_stale_market_statuses(&mut country);

        let has = |id: u32, s: PlayerStatusType| {
            country.clubs[0].teams.teams[0]
                .players
                .players
                .iter()
                .find(|p| p.id == id)
                .unwrap()
                .statuses
                .has(s)
        };
        assert!(
            !has(201, PlayerStatusType::Loa),
            "an unbacked loan badge must be cleared"
        );
        assert!(
            has(202, PlayerStatusType::Loa),
            "a loan-out candidate keeps his badge"
        );
        assert!(
            has(203, PlayerStatusType::Loa),
            "an actively loan-listed player keeps his badge"
        );
    }

    /// Regression guard: a contract that's about to expire is NOT, on its
    /// own, a reason to transfer-list the player. The contract-stalemate
    /// path requires actual renewal-rejection history (recorded in
    /// `decision_history`), and bare proximity to expiry does not satisfy
    /// that condition.
    #[test]
    fn pure_expiry_without_rejection_history_does_not_list() {
        let today = Fixture::date(2026, 5, 1);
        let player = Fixture::player(101);
        let club = Fixture::club(vec![Fixture::team(
            10,
            "main",
            TeamType::Main,
            vec![player],
        )]);
        let analysis = ListingPass::analyze_squad_needs(&club, today);
        let player_ref = &club.teams.teams[0].players.players[0];
        let decision = ListingPass::evaluate_player_listing(
            player_ref,
            &analysis,
            &club,
            today,
            None,
            MarketPresence::default(),
        );
        assert!(
            matches!(decision, ListingDecision::Keep),
            "pure expiry must not list — saw {:?}",
            decision
        );
    }

    #[test]
    fn new_manager_review_pauses_club_driven_listings() {
        let today = Fixture::date(2026, 6, 12);
        let mut player = Fixture::player(101);
        player.contract.as_mut().unwrap().is_transfer_listed = true;
        let mut club = Fixture::club(vec![Fixture::team(
            10,
            "main",
            TeamType::Main,
            vec![player],
        )]);
        club.transfer_plan.manager_review_until = Some(Fixture::date(2026, 7, 15));
        let analysis = ListingPass::analyze_squad_needs(&club, today);
        let player_ref = &club.teams.teams[0].players.players[0];
        let decision = ListingPass::evaluate_player_listing(
            player_ref,
            &analysis,
            &club,
            today,
            None,
            MarketPresence::default(),
        );
        assert!(
            matches!(decision, ListingDecision::Keep),
            "the old regime's listing flag waits for the new manager's review — saw {:?}",
            decision
        );
    }

    #[test]
    fn review_window_does_not_silence_player_requests() {
        // A formal transfer request stays on course even mid-review —
        // the new manager can't make a player un-ask to leave.
        let today = Fixture::date(2026, 6, 12);
        let mut player = Fixture::player(101);
        player.statuses.add(today, PlayerStatusType::Req);
        let mut club = Fixture::club(vec![Fixture::team(
            10,
            "main",
            TeamType::Main,
            vec![player],
        )]);
        club.transfer_plan.manager_review_until = Some(Fixture::date(2026, 7, 15));
        let analysis = ListingPass::analyze_squad_needs(&club, today);
        let player_ref = &club.teams.teams[0].players.players[0];
        let decision = ListingPass::evaluate_player_listing(
            player_ref,
            &analysis,
            &club,
            today,
            None,
            MarketPresence::default(),
        );
        assert!(
            matches!(decision, ListingDecision::Transfer { .. }),
            "a formal request is listed even during the review window — saw {:?}",
            decision
        );
    }

    #[test]
    fn club_decision_listing_tells_the_player_to_his_face() {
        let today = Fixture::date(2026, 6, 12);
        let mut player = Fixture::player(101);
        player.contract.as_mut().unwrap().is_transfer_listed = true;
        // Enough same-group depth that the position floor doesn't veto
        // the club's listing decision.
        let club = Fixture::club(vec![Fixture::team(
            10,
            "main",
            TeamType::Main,
            vec![
                player,
                Fixture::player(102),
                Fixture::player(103),
                Fixture::player(104),
                Fixture::player(105),
                Fixture::player(106),
                Fixture::player(107),
            ],
        )]);
        let mut country = Fixture::country(club);
        let mut summary = TransferActivitySummary::new();
        ListingPass::list_players_from_pipeline(&mut country, today, &mut summary);
        assert_eq!(
            country
                .transfer_market
                .listings
                .iter()
                .filter(|l| l.player_id == 101)
                .count(),
            1,
            "the club-listed flag must materialize as a market listing"
        );
        let p = country.clubs[0].teams.teams[0]
            .players
            .players
            .iter()
            .find(|p| p.id == 101)
            .unwrap();
        let told = p
            .happiness
            .recent_events
            .iter()
            .filter(|e| e.event_type == HappinessEventType::ToldNotInPlans)
            .count();
        assert_eq!(
            told, 1,
            "a club-decision listing must come with the conversation"
        );
    }

    #[test]
    fn player_requested_listing_needs_no_telling() {
        let today = Fixture::date(2026, 6, 12);
        let mut player = Fixture::player(101);
        player.statuses.add(today, PlayerStatusType::Req);
        let club = Fixture::club(vec![Fixture::team(
            10,
            "main",
            TeamType::Main,
            vec![player],
        )]);
        let mut country = Fixture::country(club);
        let mut summary = TransferActivitySummary::new();
        ListingPass::list_players_from_pipeline(&mut country, today, &mut summary);
        let p = &country.clubs[0].teams.teams[0].players.players[0];
        let told = p
            .happiness
            .recent_events
            .iter()
            .filter(|e| e.event_type == HappinessEventType::ToldNotInPlans)
            .count();
        assert_eq!(
            told, 0,
            "he asked to leave — there is nothing to break to him"
        );
    }

    #[test]
    fn flagged_reserve_player_reaches_market_with_own_team_id() {
        // A player flagged `is_transfer_listed` on a non-main squad must
        // still reach the country market — historically the listing pass
        // only read the main roster and stranded him flagged-but-
        // invisible. The listing must carry his real team id.
        let today = Fixture::date(2026, 6, 12);
        let main_player = Fixture::player(101);
        let mut reserve_player = Fixture::player(201);
        {
            let contract = reserve_player.contract.as_mut().unwrap();
            contract.is_transfer_listed = true;
            contract.squad_status = PlayerSquadStatus::MainBackupPlayer;
        }
        let club = Fixture::club(vec![
            Fixture::team(10, "main", TeamType::Main, vec![main_player]),
            Fixture::team(11, "reserve", TeamType::Reserve, vec![reserve_player]),
        ]);
        let mut country = Fixture::country(club);
        let mut summary = TransferActivitySummary::new();

        ListingPass::list_players_from_pipeline(&mut country, today, &mut summary);

        let listing = country
            .transfer_market
            .listings
            .iter()
            .find(|l| l.player_id == 201)
            .expect("flagged reserve player must reach the country market");
        assert_eq!(
            listing.team_id, 11,
            "the listing must carry the player's real (reserve) team"
        );
        let player = country.clubs[0].teams.teams[1]
            .players
            .players
            .iter()
            .find(|p| p.id == 201)
            .unwrap();
        assert!(player.statuses.has(PlayerStatusType::Lst));
        assert_eq!(
            player
                .decision_history
                .items
                .iter()
                .filter(|d| d.movement == "dec_transfer_listed")
                .count(),
            0,
            "the listing pass must not write history for pre-flagged players — \
             the flag-setter owns the entry"
        );
    }

    #[test]
    fn pre_flagged_main_player_listing_does_not_duplicate_history() {
        // The surplus trim (or salary fallback) flags the contract AND
        // writes the decision-history entry; when the listing pass later
        // materializes the flag into a market listing it must not add a
        // second, vaguer "club listed" entry.
        let today = Fixture::date(2026, 5, 1);
        // Seven midfielders so the position-group minimum (6) leaves one
        // listing slot for the flagged player.
        let mut players: Vec<Player> = (101..=107).map(Fixture::player).collect();
        {
            let flagged = &mut players[0];
            let contract = flagged.contract.as_mut().unwrap();
            contract.is_transfer_listed = true;
            contract.squad_status = PlayerSquadStatus::MainBackupPlayer;
            flagged.decision_history.add(
                Fixture::date(2026, 4, 30),
                "dec_transfer_listed".to_string(),
                "dec_reason_surplus_squad".to_string(),
                "dec_decided_board".to_string(),
            );
        }
        let club = Fixture::club(vec![Fixture::team(10, "main", TeamType::Main, players)]);
        let mut country = Fixture::country(club);
        let mut summary = TransferActivitySummary::new();

        ListingPass::list_players_from_pipeline(&mut country, today, &mut summary);

        let listing = country
            .transfer_market
            .listings
            .iter()
            .find(|l| l.player_id == 101)
            .expect("pre-flagged main-team player must reach the market");
        assert_eq!(listing.team_id, 10);
        let player = country.clubs[0].teams.teams[0]
            .players
            .players
            .iter()
            .find(|p| p.id == 101)
            .unwrap();
        assert!(player.statuses.has(PlayerStatusType::Lst));
        assert_eq!(
            player
                .decision_history
                .items
                .iter()
                .filter(|d| d.movement == "dec_transfer_listed")
                .count(),
            1,
            "exactly one listing decision — written when the player was flagged"
        );
    }

    // ── Badges are claims; the pass makes the rows ──────────────

    /// The Sokolic case. The board audit stamps `Lst` the day it decides,
    /// and the pass used to treat that badge as proof of a market row —
    /// so the row was never made, nothing could buy him, the unsold-exit
    /// valve never saw him, and the reconcile stripped the badge again so
    /// the renewal manager re-signed him. The badge is now materialised
    /// into a permanent listing with no duplicate history (the board wrote
    /// its own), and a candidate's own badge does not occupy the selling
    /// slot the depth cap is counting.
    #[test]
    fn a_board_badge_without_a_market_row_reaches_the_market() {
        let today = Fixture::date(2026, 5, 1);
        // Seven midfielders: the position-group minimum (6) leaves exactly
        // one listing slot, which the candidate's own badge used to fill.
        let mut players: Vec<Player> = (101..=107).map(Fixture::player).collect();
        {
            let listed = &mut players[0];
            listed
                .statuses
                .add(Fixture::date(2026, 4, 1), PlayerStatusType::Lst);
            listed.decision_history.add(
                Fixture::date(2026, 4, 1),
                "dec_board_transfer_listed".to_string(),
                "dec_reason_underutilized".to_string(),
                "dec_decided_board".to_string(),
            );
        }
        let club = Fixture::club(vec![Fixture::team(10, "main", TeamType::Main, players)]);
        let mut country = Fixture::country(club);
        let mut summary = TransferActivitySummary::new();

        ListingPass::list_players_from_pipeline(&mut country, today, &mut summary);

        let listing = country
            .transfer_market
            .listings
            .iter()
            .find(|l| l.player_id == 101)
            .expect("a board-listed main-squad player must reach the market");
        assert_eq!(listing.listing_type, TransferListingType::Transfer);
        assert_eq!(listing.status, TransferListingStatus::Available);
        let player = country.clubs[0].teams.teams[0]
            .players
            .players
            .iter()
            .find(|p| p.id == 101)
            .unwrap();
        assert!(
            player.statuses.has(PlayerStatusType::Lst),
            "the badge is backed by a row now and survives the reconcile"
        );
        assert_eq!(
            player
                .decision_history
                .items
                .iter()
                .filter(|d| d.movement == "dec_transfer_listed")
                .count(),
            0,
            "the board wrote the decision; the pass must not add a second"
        );
    }

    /// A main-squad player on a loan row nobody has taken for half a year,
    /// whom the club has also decided to sell, is upgraded in place — the
    /// row keeps its date so the unsold-exit valve's clock keeps the time
    /// already served. The reserve branch had this rule; the main squad
    /// did not, which is how a keeper sat on a five-year-old loan row.
    #[test]
    fn a_stale_loan_row_on_a_main_squad_player_the_club_wants_sold_upgrades_in_place() {
        let today = Fixture::date(2026, 5, 1);
        let listed_on = Fixture::date(2025, 10, 1);
        let mut players: Vec<Player> = (101..=107).map(Fixture::player).collect();
        {
            let warehoused = &mut players[0];
            warehoused.statuses.add(listed_on, PlayerStatusType::Loa);
            warehoused.contract.as_mut().unwrap().squad_status = PlayerSquadStatus::NotNeeded;
        }
        let club = Fixture::club(vec![Fixture::team(10, "main", TeamType::Main, players)]);
        let mut country = Fixture::country(club);
        country.transfer_market.add_listing(TransferListing::new(
            101,
            100,
            10,
            CurrencyValue {
                amount: 0.0,
                currency: Currency::Usd,
            },
            listed_on,
            TransferListingType::Loan,
        ));
        let mut summary = TransferActivitySummary::new();

        ListingPass::list_players_from_pipeline(&mut country, today, &mut summary);

        let rows: Vec<&TransferListing> = country
            .transfer_market
            .listings
            .iter()
            .filter(|l| l.player_id == 101)
            .collect();
        assert_eq!(rows.len(), 1, "upgraded in place, not duplicated");
        assert_eq!(rows[0].listing_type, TransferListingType::Transfer);
        assert_eq!(
            rows[0].listed_date, listed_on,
            "the valve's clock keeps the time served on the loan list"
        );
        let player = country.clubs[0].teams.teams[0]
            .players
            .players
            .iter()
            .find(|p| p.id == 101)
            .unwrap();
        assert!(player.statuses.has(PlayerStatusType::Lst));
    }

    /// A loan row the market has only just seen is left to the loan
    /// market, sale intent or not.
    #[test]
    fn a_fresh_loan_row_is_left_to_the_loan_market() {
        let today = Fixture::date(2026, 5, 1);
        let listed_on = Fixture::date(2026, 4, 1);
        let mut players: Vec<Player> = (101..=107).map(Fixture::player).collect();
        {
            let loaned = &mut players[0];
            loaned.statuses.add(listed_on, PlayerStatusType::Loa);
            loaned.contract.as_mut().unwrap().squad_status = PlayerSquadStatus::NotNeeded;
        }
        let club = Fixture::club(vec![Fixture::team(10, "main", TeamType::Main, players)]);
        let mut country = Fixture::country(club);
        country.transfer_market.add_listing(TransferListing::new(
            101,
            100,
            10,
            CurrencyValue {
                amount: 0.0,
                currency: Currency::Usd,
            },
            listed_on,
            TransferListingType::Loan,
        ));
        let mut summary = TransferActivitySummary::new();

        ListingPass::list_players_from_pipeline(&mut country, today, &mut summary);

        let rows: Vec<&TransferListing> = country
            .transfer_market
            .listings
            .iter()
            .filter(|l| l.player_id == 101)
            .collect();
        assert_eq!(rows.len(), 1);
        assert_eq!(
            rows[0].listing_type,
            TransferListingType::Loan,
            "a month on the loan list is not a stalemate"
        );
    }

    // ── Unsold-listing escape valve ─────────────────────────────

    /// Fixtures for `release_unsold_listed_players`: one listed player
    /// (id 101) on the main team whose market listing's age varies per
    /// scenario.
    struct ValveFx;

    impl ValveFx {
        /// 2026-06-01 is a Monday — the valve's weekly cadence day.
        fn monday() -> NaiveDate {
            Fixture::date(2026, 6, 1)
        }

        fn listed_country(listed_date: NaiveDate) -> Country {
            let mut player = Fixture::player(101);
            {
                let contract = player.contract.as_mut().unwrap();
                contract.expiration = Fixture::date(2029, 6, 30);
                contract.is_transfer_listed = true;
            }
            let club = Fixture::club(vec![Fixture::team(
                10,
                "main",
                TeamType::Main,
                vec![player],
            )]);
            let mut country = Fixture::country(club);
            country.transfer_market.add_listing(TransferListing::new(
                101,
                100,
                10,
                CurrencyValue {
                    amount: 500_000.0,
                    currency: Currency::Usd,
                },
                listed_date,
                TransferListingType::Transfer,
            ));
            country
        }

        fn player(country: &Country) -> &Player {
            &country.clubs[0].teams.teams[0].players.players[0]
        }
    }

    #[test]
    fn year_unsold_listing_forces_free_exit() {
        let today = ValveFx::monday();
        // Listed 396 days ago — past the year threshold, no negotiation.
        let mut country = ValveFx::listed_country(Fixture::date(2025, 5, 1));
        ListingPass::release_unsold_listed_players(&mut country, today);

        let player = ValveFx::player(&country);
        assert!(player.contract.is_none(), "the deal must be torn up");
        assert!(
            player.statuses.has(PlayerStatusType::Frt),
            "the free-agent sweep must be able to collect him"
        );
        assert_eq!(
            player.release_reason(),
            Some(FreeAgentReleaseReason::UnsoldListingExit),
            "the exit must carry the unsold-listing narrative"
        );
        assert!(
            country
                .transfer_market
                .listings
                .iter()
                .filter(|l| l.player_id == 101)
                .all(|l| l.status == TransferListingStatus::Cancelled),
            "the stranded listing row must be retired"
        );
    }

    #[test]
    fn recent_listing_is_not_torn_up() {
        let today = ValveFx::monday();
        // Listed ~3 months ago — a live sale, not a stalemate.
        let mut country = ValveFx::listed_country(Fixture::date(2026, 3, 1));
        ListingPass::release_unsold_listed_players(&mut country, today);
        assert!(
            ValveFx::player(&country).contract.is_some(),
            "a listing months old is still a sale in progress"
        );
    }

    #[test]
    fn window_close_lands_limbo_on_listed_players() {
        let mut country = ValveFx::listed_country(Fixture::date(2026, 5, 1));
        ListingPass::emit_window_close_limbo(&mut country, ValveFx::monday());
        let unsold = ValveFx::player(&country)
            .happiness
            .recent_events
            .iter()
            .filter(|e| e.event_type == HappinessEventType::UnsoldWindowClosed)
            .count();
        assert_eq!(
            unsold, 1,
            "a listed, unsold player must feel the window shut"
        );
    }

    #[test]
    fn window_close_ignores_synthetic_listings() {
        let mut country = ValveFx::listed_country(Fixture::date(2026, 5, 1));
        country.transfer_market.listings[0].origin = TransferListingOrigin::SyntheticUnsolicited;
        ListingPass::emit_window_close_limbo(&mut country, ValveFx::monday());
        let unsold = ValveFx::player(&country)
            .happiness
            .recent_events
            .iter()
            .filter(|e| e.event_type == HappinessEventType::UnsoldWindowClosed)
            .count();
        assert_eq!(
            unsold, 0,
            "a synthetic anchor row is not a player waiting on a move"
        );
    }

    #[test]
    fn near_expiry_listed_contract_lapses_instead_of_terminating() {
        let today = ValveFx::monday();
        let mut country = ValveFx::listed_country(Fixture::date(2025, 5, 1));
        // Final half-year of the deal — natural expiry is the cheaper
        // exit; the renewal gate guarantees no new offer arrives.
        country.clubs[0].teams.teams[0].players.players[0]
            .contract
            .as_mut()
            .unwrap()
            .expiration = Fixture::date(2026, 9, 1);
        ListingPass::release_unsold_listed_players(&mut country, today);
        assert!(
            ValveFx::player(&country).contract.is_some(),
            "final-half-year deals run out on their own — no severance needed"
        );
    }

    #[test]
    fn loa_flagged_reserve_player_reaches_loan_market_with_own_team_id() {
        // A reserve/youth player carrying the board loan badge (`Loa`) —
        // stamped by the squad-utilization audit — must become a real loan
        // listing on the country market, or the badge is cosmetic and no
        // club can ever bid. The listing must carry his real team id and be
        // a loan (not transfer) listing.
        let today = Fixture::date(2026, 6, 12);
        let main_player = Fixture::player(101);
        let mut reserve_player = Fixture::player(202);
        reserve_player.statuses.add(today, PlayerStatusType::Loa);
        {
            let contract = reserve_player.contract.as_mut().unwrap();
            contract.squad_status = PlayerSquadStatus::MainBackupPlayer;
        }
        let club = Fixture::club(vec![
            Fixture::team(10, "main", TeamType::Main, vec![main_player]),
            Fixture::team(11, "reserve", TeamType::Reserve, vec![reserve_player]),
        ]);
        let mut country = Fixture::country(club);
        let mut summary = TransferActivitySummary::new();

        ListingPass::list_players_from_pipeline(&mut country, today, &mut summary);

        let listing = country
            .transfer_market
            .listings
            .iter()
            .find(|l| l.player_id == 202)
            .expect("a Loa-flagged reserve player must reach the loan market");
        assert_eq!(
            listing.listing_type,
            TransferListingType::Loan,
            "the board loan badge must produce a loan listing, not a transfer listing"
        );
        assert_eq!(
            listing.team_id, 11,
            "the loan listing must carry the player's real (reserve) team"
        );
    }

    #[test]
    fn loa_flagged_reserve_player_is_loan_listed_once_across_passes() {
        // The listing pass runs every day a window is open; a Loa-flagged
        // reserve player must be listed exactly once, not re-listed daily.
        let today = Fixture::date(2026, 6, 12);
        let main_player = Fixture::player(101);
        let mut reserve_player = Fixture::player(203);
        reserve_player.statuses.add(today, PlayerStatusType::Loa);
        let club = Fixture::club(vec![
            Fixture::team(10, "main", TeamType::Main, vec![main_player]),
            Fixture::team(11, "reserve", TeamType::Reserve, vec![reserve_player]),
        ]);
        let mut country = Fixture::country(club);
        let mut summary = TransferActivitySummary::new();

        ListingPass::list_players_from_pipeline(&mut country, today, &mut summary);
        ListingPass::list_players_from_pipeline(&mut country, today, &mut summary);

        let loan_listings = country
            .transfer_market
            .listings
            .iter()
            .filter(|l| l.player_id == 203 && l.listing_type == TransferListingType::Loan)
            .count();
        assert_eq!(
            loan_listings, 1,
            "a Loa-flagged player must be loan-listed exactly once, not re-listed each pass"
        );
    }

    /// A useful first-team regular flagged `Unh` purely from a lack of
    /// minutes must NOT be auto-transfer-listed — the manager-talk / loan
    /// paths own him.
    #[test]
    fn unhappy_regular_playing_time_only_is_kept() {
        let today = Fixture::date(2026, 5, 1);
        let mut player = Fixture::player(101); // FirstTeamRegular, CA 130
        player.statuses.add(today, PlayerStatusType::Unh);
        player.happiness.factors.playing_time = -15.0;
        let club = Fixture::club(vec![Fixture::team(
            10,
            "main",
            TeamType::Main,
            vec![player],
        )]);
        let analysis = ListingPass::analyze_squad_needs(&club, today);
        let player_ref = &club.teams.teams[0].players.players[0];
        let decision = ListingPass::evaluate_player_listing(
            player_ref,
            &analysis,
            &club,
            today,
            None,
            MarketPresence::default(),
        );
        assert!(
            matches!(decision, ListingDecision::Keep),
            "unhappy-but-useful regular frustrated only by minutes must be kept — saw {:?}",
            decision
        );
    }

    /// A first-team regular with zero current-season appearances early in
    /// the season is kept.
    #[test]
    fn first_team_regular_zero_apps_is_kept() {
        let today = Fixture::date(2026, 5, 1);
        let player = Fixture::player(106); // FirstTeamRegular, no apps
        let club = Fixture::club(vec![Fixture::team(
            10,
            "main",
            TeamType::Main,
            vec![player],
        )]);
        let analysis = ListingPass::analyze_squad_needs(&club, today);
        let player_ref = &club.teams.teams[0].players.players[0];
        let decision = ListingPass::evaluate_player_listing(
            player_ref,
            &analysis,
            &club,
            today,
            None,
            MarketPresence::default(),
        );
        assert!(
            matches!(decision, ListingDecision::Keep),
            "a first-team regular with no minutes early-season must be kept — saw {:?}",
            decision
        );
    }

    /// A credible rotation player flagged `Unh` over minutes is kept,
    /// not sold — `RotationUseful` is routed to keep.
    #[test]
    fn unhappy_rotation_player_is_kept() {
        let today = Fixture::date(2026, 5, 1);
        let mut player = Fixture::player(102);
        player.contract.as_mut().unwrap().squad_status = PlayerSquadStatus::FirstTeamSquadRotation;
        player.statuses.add(today, PlayerStatusType::Unh);
        player.happiness.factors.playing_time = -12.0;
        let club = Fixture::club(vec![Fixture::team(
            10,
            "main",
            TeamType::Main,
            vec![player],
        )]);
        let analysis = ListingPass::analyze_squad_needs(&club, today);
        let player_ref = &club.teams.teams[0].players.players[0];
        let decision = ListingPass::evaluate_player_listing(
            player_ref,
            &analysis,
            &club,
            today,
            None,
            MarketPresence::default(),
        );
        assert!(
            matches!(decision, ListingDecision::Keep),
            "an unhappy rotation player must be kept, not listed — saw {:?}",
            decision
        );
    }

    /// An `Unh` player who has carried the status for 6+ months is put up
    /// for sale — sustained, unresolved unhappiness is a durable sell
    /// signal even for an otherwise-useful squad member.
    #[test]
    fn unhappy_for_six_months_is_listed() {
        let today = Fixture::date(2026, 5, 1);
        let mut player = Fixture::player(103);
        // Unhappy since well over six months ago.
        player
            .statuses
            .add(today - chrono::Duration::days(200), PlayerStatusType::Unh);
        let club = Fixture::club(vec![Fixture::team(
            10,
            "main",
            TeamType::Main,
            vec![player],
        )]);
        let analysis = ListingPass::analyze_squad_needs(&club, today);
        let player_ref = &club.teams.teams[0].players.players[0];
        let decision = ListingPass::evaluate_player_listing(
            player_ref,
            &analysis,
            &club,
            today,
            None,
            MarketPresence::default(),
        );
        assert!(
            matches!(decision, ListingDecision::Transfer { ref reason } if reason == "dec_reason_player_unhappy"),
            "an unhappy player past six months must be listed — saw {:?}",
            decision
        );
    }

    /// A player who only recently became `Unh` is NOT listed on
    /// unhappiness alone — even with a deep ambition mismatch on the books,
    /// the manager-talk / loan paths own him until the mood has held for
    /// six months. A useful rotation player is kept.
    #[test]
    fn recently_unhappy_player_is_kept_until_six_months() {
        let today = Fixture::date(2026, 5, 1);
        let mut player = Fixture::player(106);
        player.contract.as_mut().unwrap().squad_status = PlayerSquadStatus::FirstTeamSquadRotation;
        // Unhappy for two months — well short of the six-month listing gate.
        player
            .statuses
            .add(today - chrono::Duration::days(60), PlayerStatusType::Unh);
        player.happiness.factors.ambition_fit = -10.0;
        let club = Fixture::club(vec![Fixture::team(
            10,
            "main",
            TeamType::Main,
            vec![player],
        )]);
        let analysis = ListingPass::analyze_squad_needs(&club, today);
        let player_ref = &club.teams.teams[0].players.players[0];
        let decision = ListingPass::evaluate_player_listing(
            player_ref,
            &analysis,
            &club,
            today,
            None,
            MarketPresence::default(),
        );
        assert!(
            matches!(decision, ListingDecision::Keep),
            "a recently-unhappy useful player must be kept until six months — saw {:?}",
            decision
        );
    }

    /// Regression: an explicit `NotNeeded` surplus player is still actioned.
    #[test]
    fn not_needed_surplus_is_still_listed() {
        let today = Fixture::date(2026, 5, 1);
        let mut player = Fixture::player(104);
        player.contract.as_mut().unwrap().squad_status = PlayerSquadStatus::NotNeeded;
        let club = Fixture::club(vec![Fixture::team(
            10,
            "main",
            TeamType::Main,
            vec![player],
        )]);
        let analysis = ListingPass::analyze_squad_needs(&club, today);
        let player_ref = &club.teams.teams[0].players.players[0];
        let decision = ListingPass::evaluate_player_listing(
            player_ref,
            &analysis,
            &club,
            today,
            None,
            MarketPresence::default(),
        );
        assert!(
            !matches!(decision, ListingDecision::Keep),
            "an explicit NotNeeded surplus player must still be actioned — saw {:?}",
            decision
        );
    }

    /// Regression: a formal transfer request still lists.
    #[test]
    fn requested_player_is_still_listed() {
        let today = Fixture::date(2026, 5, 1);
        let mut player = Fixture::player(105);
        player.statuses.add(today, PlayerStatusType::Req);
        let club = Fixture::club(vec![Fixture::team(
            10,
            "main",
            TeamType::Main,
            vec![player],
        )]);
        let analysis = ListingPass::analyze_squad_needs(&club, today);
        let player_ref = &club.teams.teams[0].players.players[0];
        let decision = ListingPass::evaluate_player_listing(
            player_ref,
            &analysis,
            &club,
            today,
            None,
            MarketPresence::default(),
        );
        assert!(
            matches!(decision, ListingDecision::Transfer { ref reason } if reason == "dec_reason_player_requested"),
            "a formal transfer request must still list — saw {:?}",
            decision
        );
    }

    /// Fixtures reproducing the squad shape that sent a top-flight regular
    /// out on loan: a deep, top-heavy senior squad where the "Defender" group
    /// also holds the club's holding midfielders.
    struct TopFlightSquad;

    impl TopFlightSquad {
        /// A senior with the given ability, age and position. Skills carry the
        /// ability so the observable-level classifiers read him correctly;
        /// `current_ability` is stamped to match for the CA-based sweeps.
        fn player(id: u32, ca: u8, age: u8, position: PlayerPositionType) -> Player {
            let mut attrs = PlayerAttributes::default();
            attrs.current_ability = ca;
            attrs.potential_ability = ca;
            attrs.current_reputation = 3000;
            attrs.home_reputation = 3000;
            let mut contract = PlayerClubContract::new(500_000, Fixture::date(2029, 6, 30));
            contract.squad_status = PlayerSquadStatus::NotYetSet;
            contract.started = Some(Fixture::date(2025, 7, 1));
            PlayerBuilder::new()
                .id(id)
                .full_name(FullName::new("P".into(), format!("{id}")))
                .birth_date(Fixture::date(2026 - age as i32, 1, 1))
                .country_id(1)
                .attributes(PersonAttributes::default())
                .skills(PlayerSkills::flat_for_ability(ca))
                .positions(PlayerPositions {
                    positions: vec![PlayerPosition {
                        position,
                        level: 20,
                    }],
                })
                .player_attributes(attrs)
                .contract(Some(contract))
                .build()
                .unwrap()
        }

        /// The roster, mirroring a real top-flight senior squad: five of the
        /// "defenders" are actually holding midfielders, which is what buries
        /// a genuine centre-back down the position group's ability ranking.
        fn roster() -> Vec<Player> {
            use PlayerPositionType as P;
            [
                (1u32, 136u8, 27u8, P::MidfielderCenter),
                (2, 136, 27, P::DefensiveMidfielder),
                (3, 132, 30, P::MidfielderRight),
                (4, 130, 24, P::Striker),
                (5, 130, 28, P::Striker),
                (6, 128, 25, P::MidfielderRight),
                (7, 128, 29, P::DefensiveMidfielder),
                (8, 128, 26, P::MidfielderLeft),
                (9, 128, 31, P::DefenderCenter),
                (10, 126, 26, P::DefensiveMidfielder),
                (11, 126, 30, P::DefenderCenter),
                (12, 124, 28, P::Goalkeeper),
                (13, 122, 24, P::DefenderCenter),
                (14, 122, 28, P::DefenderRight),
                (16, 120, 26, P::DefensiveMidfielder),
                (17, 120, 32, P::DefenderRight),
                (18, 120, 30, P::MidfielderLeft),
                (19, 120, 23, P::DefenderRight),
                (20, 120, 28, P::DefenderLeft),
                (21, 118, 35, P::Striker),
                (22, 118, 29, P::Goalkeeper),
                (23, 116, 22, P::WingbackRight),
                (24, 116, 30, P::DefenderCenter),
                (25, 114, 25, P::DefenderRight),
                (26, 100, 22, P::MidfielderCenter),
                (27, 94, 39, P::Goalkeeper),
            ]
            .into_iter()
            .map(|(id, ca, age, position)| Self::player(id, ca, age, position))
            .collect()
        }

        /// The squad plus one centre-back who started 22 league games last
        /// season and whom the monthly pass has just labelled `status`. His
        /// ability sits a single point under the squad mean — the razor-thin
        /// margin the numeric surplus trigger reads.
        fn club_with_regular(status: PlayerSquadStatus, ca: u8) -> Club {
            let mut players = Self::roster();
            let mut regular = Self::player(99, ca, 24, PlayerPositionType::DefenderCenter);
            regular.contract.as_mut().unwrap().squad_status = status;
            let mut stats = PlayerStatistics::default();
            stats.played = 22;
            regular
                .statistics_history
                .items
                .push(PlayerStatisticsHistoryItem {
                    season: Season::new(2025),
                    team_name: "Main".into(),
                    team_slug: "main".into(),
                    team_reputation: 7600,
                    league_name: "L".into(),
                    league_slug: "l".into(),
                    is_loan: false,
                    transfer_fee: None,
                    statistics: stats,
                    seq_id: 2025,
                });
            players.push(regular);

            let mut team = Fixture::team(10, "main", TeamType::Main, players);
            team.reputation = TeamReputation::new(7600, 7600, 7600);
            Fixture::club(vec![team])
        }
    }

    /// The Litvinov regression, end to end. A centre-back who started 22
    /// league games last season, one ability point below his squad's mean,
    /// freshly relabelled rotation by the monthly CA-rank pass, must not be
    /// loan-listed by the numeric surplus sweep three weeks into the new
    /// season. Before the fix this returned
    /// `Loan { "dec_reason_blocked_top_club" }` — the decision that sent a
    /// Premier League regular to a second-division club.
    #[test]
    fn top_flight_regular_is_not_loan_listed_as_positional_surplus() {
        let today = Fixture::date(2026, 8, 2);
        for status in [
            PlayerSquadStatus::NotYetSet,
            PlayerSquadStatus::FirstTeamSquadRotation,
            PlayerSquadStatus::MainBackupPlayer,
        ] {
            let club = TopFlightSquad::club_with_regular(status.clone(), 119);
            let analysis = ListingPass::analyze_squad_needs(&club, today);
            let player = club.teams.teams[0]
                .players
                .players
                .iter()
                .find(|p| p.id == 99)
                .unwrap();
            let decision = ListingPass::evaluate_player_listing(
                player,
                &analysis,
                &club,
                today,
                None,
                MarketPresence::default(),
            );
            assert!(
                matches!(decision, ListingDecision::Keep),
                "last season's regular must be kept under {:?} — saw {:?}",
                status,
                decision
            );
        }
    }

    /// A three-keeper squad is a normal squad. The old local threshold
    /// (`gk > 2`) called every one of them surplus at goalkeeper, which then
    /// listed any keeper sitting below the squad average.
    #[test]
    fn three_keepers_are_not_a_surplus_position() {
        let today = Fixture::date(2026, 8, 2);
        let club = TopFlightSquad::club_with_regular(PlayerSquadStatus::NotYetSet, 122);
        let analysis = ListingPass::analyze_squad_needs(&club, today);
        assert!(
            !analysis
                .surplus_positions
                .contains(&PlayerPositionType::Goalkeeper),
            "three keepers is normal depth, not surplus — saw {:?}",
            analysis.surplus_positions
        );
    }

    /// Regression guard for the flagged-but-unlisted limbo.
    ///
    /// `is_transfer_listed` blocks contract renewal and coach-agreed
    /// termination. If the depth cap then refuses to create the market row,
    /// the player keeps the flag with nothing behind it: the club has
    /// quietly decided to sell someone it cannot sell, cannot renew and
    /// cannot release, so he runs his deal down with no offers and no exit.
    /// The invariant is that the two always agree — after the listing pass,
    /// nobody still carries the sell intent without a market row.
    #[test]
    fn a_capped_listing_releases_the_sell_intent() {
        let today = Fixture::date(2026, 6, 12);
        // A two-keeper group sits at the minimum the depth cap protects, so
        // the club's decision to sell one of them cannot be honoured.
        let mut keeper = TopFlightSquad::player(201, 100, 29, PlayerPositionType::Goalkeeper);
        keeper.contract.as_mut().unwrap().is_transfer_listed = true;
        let deputy = TopFlightSquad::player(202, 120, 27, PlayerPositionType::Goalkeeper);

        let club = Fixture::club(vec![Fixture::team(
            10,
            "main",
            TeamType::Main,
            vec![keeper, deputy],
        )]);
        let mut country = Fixture::country(club);
        let mut summary = TransferActivitySummary::new();
        ListingPass::list_players_from_pipeline(&mut country, today, &mut summary);

        let rows = country
            .transfer_market
            .listings
            .iter()
            .filter(|l| l.player_id == 201)
            .count();
        let still_flagged = country.clubs[0].teams.teams[0]
            .players
            .players
            .iter()
            .find(|p| p.id == 201)
            .and_then(|p| p.contract.as_ref())
            .map(|c| c.is_transfer_listed)
            .unwrap_or(false);

        assert!(
            !(still_flagged && rows == 0),
            "sell intent must never outlive its market row: flagged={still_flagged} rows={rows}"
        );
    }
}
