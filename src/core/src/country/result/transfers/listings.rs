use super::types::{SquadAnalysis, TransferActivitySummary};
use crate::club::player::calculators::FreeAgentReleaseReason;
use crate::club::player::contract::{AffordabilityInput, ContractStalemate};
use crate::club::player::transfer::processing::UNHAPPY_LISTING_MIN_DAYS;
use crate::club::staff::perception::PotentialEstimator;
use crate::club::team::squad::{SquadAssetClass, SquadAssetProtection};
use crate::country::result::CountryResult;
use crate::shared::{Currency, CurrencyValue};
use crate::transfers::TransferWindowManager;
use crate::transfers::pipeline::{LoanOutReason, PipelineProcessor};
use crate::transfers::window::PlayerValuationCalculator;
use crate::transfers::{
    TransferListing, TransferListingOrigin, TransferListingStatus, TransferListingType,
};
use crate::club::player::behaviour_config::HappinessConfig;
use crate::{
    Club, Country, HappinessEventType, Person, Player, PlayerFieldPositionGroup,
    PlayerPositionType, PlayerSquadStatus, PlayerStatusType, ReputationLevel,
};
use chrono::{Datelike, NaiveDate, Weekday};
use log::debug;
use std::collections::{HashMap, HashSet};

#[cfg_attr(test, derive(Debug))]
pub(crate) enum ListingDecision {
    Keep,
    Transfer { reason: String },
    Loan { reason: String },
    FreeTransfer,
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

impl CountryResult {
    /// List players for transfer based on pipeline decisions and staff evaluations.
    pub(crate) fn list_players_from_pipeline(
        country: &mut Country,
        date: NaiveDate,
        summary: &mut TransferActivitySummary,
    ) {
        let mut listings_to_add: Vec<PendingListing> = Vec::new();
        let price_level = country.settings.pricing.price_level;
        let window_mgr = TransferWindowManager::for_country(country, date);
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
            let decided_by = main_team.staffs.head_coach_name();

            for player in &main_team.players.players {
                match Self::evaluate_player_listing(
                    player,
                    &squad_analysis,
                    club,
                    date,
                    current_window,
                ) {
                    ListingDecision::Keep => {}
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
                        let asking_price = Self::calculate_asking_price(
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
                    let statuses = player.statuses.get();

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
                    if statuses.contains(&PlayerStatusType::Loa)
                        && player.contract.is_some()
                        && country
                            .transfer_market
                            .get_listing_by_player(player.id)
                            .is_none()
                    {
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
                        continue;
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
                    if statuses.contains(&PlayerStatusType::Lst)
                        || statuses.contains(&PlayerStatusType::Loa)
                        || statuses.contains(&PlayerStatusType::Frt)
                    {
                        continue;
                    }
                    let asking_price = Self::calculate_asking_price(
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

        // Cap club-decided listings so no position group on a main team
        // drops below a minimum. Player-initiated (REQ/UNH) listings are
        // honoured even when this leaves the group short — the player
        // wants out and the club must replace him.
        let listings_to_add = Self::enforce_position_group_minimums(country, listings_to_add);

        if !listings_to_add.is_empty() {
            debug!(
                "Transfer market: listing {} players for transfer/loan",
                listings_to_add.len()
            );
        }

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
                        if !player.statuses.get().contains(&status_type) {
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
                        let player_initiated = listing_data.reason
                            == "dec_reason_player_requested"
                            || listing_data.reason == "dec_reason_player_unhappy";
                        if status_type == PlayerStatusType::Lst
                            && !player_initiated
                            && !is_under16_release
                        {
                            let magnitude =
                                HappinessConfig::default().catalog.told_not_in_plans;
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

        let in_negotiation: HashSet<u32> = country
            .transfer_market
            .negotiations
            .values()
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
                if let Some(player) = team
                    .players
                    .players
                    .iter_mut()
                    .find(|p| p.id == player_id)
                {
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
            PipelineProcessor::clear_player_interest(country, player_id);
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
    fn enforce_position_group_minimums(
        country: &Country,
        listings: Vec<PendingListing>,
    ) -> Vec<PendingListing> {
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
            // he goes regardless of how full the selling queue is.
            let already_listed_in_group = find_main(club_id)
                .map(|t| {
                    t.players
                        .iter()
                        .filter(|p| p.position().position_group() == group)
                        .filter(|p| {
                            let s = p.statuses.get();
                            s.contains(&PlayerStatusType::Lst)
                                || s.contains(&PlayerStatusType::Loa)
                                || s.contains(&PlayerStatusType::Frt)
                        })
                        .count()
                })
                .unwrap_or(0);

            let min_to_keep = min_squad_for_group(group);
            let slots_after_min = current_count.saturating_sub(min_to_keep);
            let max_can_list = slots_after_min
                .saturating_sub(exempt_in_group)
                .saturating_sub(already_listed_in_group);

            // Worst-CA players get listed first
            group_listings.sort_by_key(|l| player_ca(l.club_id, l.player_id));

            result.extend(group_listings.into_iter().take(max_can_list));
        }

        result
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

        let gk = *group_counts
            .get(&PlayerFieldPositionGroup::Goalkeeper)
            .unwrap_or(&0);
        let def = *group_counts
            .get(&PlayerFieldPositionGroup::Defender)
            .unwrap_or(&0);
        let mid = *group_counts
            .get(&PlayerFieldPositionGroup::Midfielder)
            .unwrap_or(&0);
        let fwd = *group_counts
            .get(&PlayerFieldPositionGroup::Forward)
            .unwrap_or(&0);

        let mut surplus = Vec::new();
        let mut needed = Vec::new();

        if gk > 2 {
            surplus.push(PlayerPositionType::Goalkeeper);
        }
        if gk < 2 {
            needed.push(PlayerPositionType::Goalkeeper);
        }
        if def > 7 {
            surplus.push(PlayerPositionType::DefenderCenter);
        }
        if def < 4 {
            needed.push(PlayerPositionType::DefenderCenter);
        }
        if mid > 7 {
            surplus.push(PlayerPositionType::MidfielderCenter);
        }
        if mid < 4 {
            needed.push(PlayerPositionType::MidfielderCenter);
        }
        if fwd > 5 {
            surplus.push(PlayerPositionType::Striker);
        }
        if fwd < 2 {
            needed.push(PlayerPositionType::Striker);
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
    ) -> ListingDecision {
        // Loan players belong to another club — cannot be listed by the loan club
        if player.is_on_loan() {
            return ListingDecision::Keep;
        }

        // Manager has pinned this player to the squad — never auto-list.
        // The pin only protects contracted players; once the contract
        // ends the player is a free agent and must be free to move.
        if player.is_force_match_selection && player.contract.is_some() {
            return ListingDecision::Keep;
        }

        // Same-window protection: signed during this open window → can't be listed
        if let (Some(transfer_date), Some((window_start, window_end))) =
            (player.last_transfer_date, current_window)
        {
            if transfer_date >= window_start && transfer_date <= window_end {
                return ListingDecision::Keep;
            }
        }

        let statuses = player.statuses.get();

        // Already listed
        if statuses.contains(&PlayerStatusType::Lst)
            || statuses.contains(&PlayerStatusType::Loa)
            || statuses.contains(&PlayerStatusType::Frt)
        {
            return ListingDecision::Keep;
        }

        // Club signing plan: the club bought this player with intent.
        if let Some(ref plan) = player.plan {
            let total_apps = player.statistics.played + player.statistics.played_subs;
            if !plan.is_evaluated(date, total_apps) && !plan.is_expired(date) {
                return ListingDecision::Keep;
            }
        }

        let age = player.age(date);
        let ca = player.player_attributes.current_ability;
        // Clubs can't see biological PA — listing decisions read the
        // observable ceiling (visible ability + age/mentals projection).
        let pa = PotentialEstimator::observable_ceiling(player, date);
        let ca_i = ca as i16;
        let avg = analysis.quality_level as i16;

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

        // Check if evaluation pipeline already identified as loan candidate
        let loan_candidate = club
            .transfer_plan
            .loan_out_candidates
            .iter()
            .find(|c| c.player_id == player.id);

        if let Some(candidate) = loan_candidate {
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
            };
            return ListingDecision::Loan {
                reason: reason.to_string(),
            };
        }

        // Player-initiated departures outrank persisted club decisions: a
        // player who formally requested out (or hardened into Unh) is
        // listed under his own reason and exempted from the position-group
        // minimums. The transfer-request handler also sets
        // `contract.is_transfer_listed`, so checking the flag first used to
        // mislabel these as "club listed".
        if statuses.contains(&PlayerStatusType::Req) {
            return ListingDecision::Transfer {
                reason: "dec_reason_player_requested".to_string(),
            };
        }

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
        if statuses.contains(&PlayerStatusType::Unh) {
            let unhappy_days = player
                .statuses
                .held_for_days(PlayerStatusType::Unh, date)
                .unwrap_or(0);
            if unhappy_days >= UNHAPPY_LISTING_MIN_DAYS {
                return ListingDecision::Transfer {
                    reason: "dec_reason_player_unhappy".to_string(),
                };
            }
            return match SquadAssetProtection::classify(player, club, date) {
                SquadAssetClass::CorePlayer
                | SquadAssetClass::FirstTeamUseful
                | SquadAssetClass::RotationUseful
                | SquadAssetClass::UnknownNeedsEvaluation => ListingDecision::Keep,
                SquadAssetClass::ProspectDevelopment => ListingDecision::Loan {
                    reason: "dec_reason_young_needs_practice".to_string(),
                },
                SquadAssetClass::TrueSurplus => ListingDecision::Transfer {
                    reason: "dec_reason_player_unhappy".to_string(),
                },
            };
        }

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
            return ListingDecision::Keep;
        }

        // Club decisions persisted on the contract.
        if let Some(ref contract) = player.contract {
            if matches!(contract.squad_status, PlayerSquadStatus::NotNeeded) {
                return Self::decide_listing_type(
                    player,
                    &rep_level,
                    avg,
                    date,
                    "dec_reason_surplus_squad".to_string(),
                );
            }
            if contract.is_transfer_listed {
                return ListingDecision::Transfer {
                    reason: "dec_reason_club_listed".to_string(),
                };
            }
        }

        // Squad members the club wouldn't move on pure maths. Runs after
        // explicit decisions (NotNeeded / club-listed / REQ / UNH) so those
        // still dictate, but before numeric triggers so a club captain with
        // a few rating points below the squad mean isn't auto-sold.
        if Self::is_squad_protected(player, club, date) {
            return ListingDecision::Keep;
        }

        let is_promising_youth = age <= 23 && pa > ca + 10;

        // Wealth-aware quality gap threshold
        let quality_gap_threshold: i16 = match rep_level {
            ReputationLevel::Elite => 25,
            ReputationLevel::Continental => 20,
            ReputationLevel::National => 15,
            ReputationLevel::Regional => 12,
            _ => 10,
        };

        // Well below squad average
        if analysis.quality_level > 15 && ca_i < avg - quality_gap_threshold && !is_promising_youth
        {
            if !Self::position_group_has_depth(club, player, date) {
                return ListingDecision::Keep;
            }
            return Self::decide_listing_type(
                player,
                &rep_level,
                avg,
                date,
                "dec_reason_well_below_avg".to_string(),
            );
        }

        // Surplus position and below average
        let player_group = player.position().position_group();
        for surplus_pos in &analysis.surplus_positions {
            if surplus_pos.position_group() == player_group {
                if ca_i < avg && !is_promising_youth {
                    return Self::decide_listing_type(
                        player,
                        &rep_level,
                        avg,
                        date,
                        "dec_reason_below_avg_surplus".to_string(),
                    );
                }
            }
        }

        // Aging players past their prime — only top clubs cycle aging
        // squad-average players out. Smaller clubs keep them to the end of
        // their careers: loyalty, shorter shopping lists, a 35-year-old
        // stalwart at a regional club is a feature, not a problem.
        if rep_level.cycles_aging_squad() {
            let aging_threshold = aging_listing_threshold(player.position().position_group());
            if age >= aging_threshold && ca_i < avg + 5 {
                return ListingDecision::Transfer {
                    reason: "dec_reason_aging_declining".to_string(),
                };
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
            return Self::decide_listing_type(
                player,
                &rep_level,
                avg,
                date,
                "dec_reason_squad_oversized".to_string(),
            );
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
            return ListingDecision::Transfer {
                reason: "dec_reason_contract_stalemate".to_string(),
            };
        }

        ListingDecision::Keep
    }

    /// Decide between Transfer and Loan based on player profile and club context.
    fn decide_listing_type(
        player: &Player,
        rep_level: &ReputationLevel,
        avg: i16,
        date: NaiveDate,
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

        // Young with development potential → loan for match practice
        if age <= 23 && pa > ca + 10 {
            return ListingDecision::Loan {
                reason: "dec_reason_young_needs_practice".to_string(),
            };
        }

        // At wealthy club, young enough and decent quality → loan to preserve asset
        if age <= 25
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
        let peaked_age =
            aging_listing_threshold(player.position().position_group()).saturating_sub(2);
        if age >= peaked_age && pa <= ca {
            return ListingDecision::Transfer {
                reason: "dec_reason_peaked_declining".to_string(),
            };
        }

        // Mid-career at wealthy club → loan to preserve value
        if age <= 27
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
        if age >= 26 && player.skills.mental.leadership >= 15.0 {
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

        if tenure_years >= 4 && last_rating >= 6.9 {
            return true;
        }

        // Club stalwart — 6+ years regardless of recent form. Deep-backup
        // roles naturally produce thin playing records (and thus no form
        // data or low ratings from few appearances); the tenure+form
        // branch above punishes them unfairly. Six-year loyalty earned
        // patience from the dressing room and, typically, the boardroom.
        if tenure_years >= 6 {
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
        let group = player.position().position_group();
        if group == PlayerFieldPositionGroup::Goalkeeper && age >= 30 {
            return true;
        }

        false
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
}

/// Age at which a mid-tier player at or below squad average is considered
/// "past his prime" for transfer-listing purposes. Mirrors real-world
/// career lengths: keepers last longest, forwards (speed-dependent)
/// decline first, defenders and holding midfielders sit in between.
fn aging_listing_threshold(group: PlayerFieldPositionGroup) -> u8 {
    match group {
        PlayerFieldPositionGroup::Goalkeeper => 37,
        PlayerFieldPositionGroup::Defender => 34,
        PlayerFieldPositionGroup::Midfielder => 33,
        PlayerFieldPositionGroup::Forward => 32,
    }
}

/// Minimum number of main-team players a club must retain per position
/// group after any club-decided transfer/loan listings in a single pass.
/// Player-initiated listings (REQ/UNH) bypass this cap.
fn min_squad_for_group(group: PlayerFieldPositionGroup) -> usize {
    match group {
        PlayerFieldPositionGroup::Goalkeeper => 2,
        PlayerFieldPositionGroup::Defender => 6,
        PlayerFieldPositionGroup::Midfielder => 6,
        PlayerFieldPositionGroup::Forward => 3,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::academy::ClubAcademy;
    use crate::club::player::core::builder::PlayerBuilder;
    use crate::league::{DayMonthPeriod, League, LeagueCollection, LeagueSettings};
    use crate::shared::Location;
    use crate::shared::fullname::FullName;
    use crate::{
        ClubColors, ClubFacilities, ClubFinances, ClubStatus, PersonAttributes, PlayerAttributes,
        PlayerClubContract, PlayerCollection, PlayerPosition, PlayerPositionType, PlayerPositions,
        PlayerSkills, StaffCollection, Team, TeamBuilder, TeamCollection, TeamReputation, TeamType,
        TrainingSchedule,
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
        let analysis = CountryResult::analyze_squad_needs(&club, today);
        let player_ref = &club.teams.teams[0].players.players[0];
        let decision =
            CountryResult::evaluate_player_listing(player_ref, &analysis, &club, today, None);
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
        let analysis = CountryResult::analyze_squad_needs(&club, today);
        let player_ref = &club.teams.teams[0].players.players[0];
        let decision =
            CountryResult::evaluate_player_listing(player_ref, &analysis, &club, today, None);
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
        let analysis = CountryResult::analyze_squad_needs(&club, today);
        let player_ref = &club.teams.teams[0].players.players[0];
        let decision =
            CountryResult::evaluate_player_listing(player_ref, &analysis, &club, today, None);
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
        CountryResult::list_players_from_pipeline(&mut country, today, &mut summary);
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
        CountryResult::list_players_from_pipeline(&mut country, today, &mut summary);
        let p = &country.clubs[0].teams.teams[0].players.players[0];
        let told = p
            .happiness
            .recent_events
            .iter()
            .filter(|e| e.event_type == HappinessEventType::ToldNotInPlans)
            .count();
        assert_eq!(told, 0, "he asked to leave — there is nothing to break to him");
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

        CountryResult::list_players_from_pipeline(&mut country, today, &mut summary);

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
        assert!(player.statuses.get().contains(&PlayerStatusType::Lst));
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

        CountryResult::list_players_from_pipeline(&mut country, today, &mut summary);

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
        assert!(player.statuses.get().contains(&PlayerStatusType::Lst));
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
            let club =
                Fixture::club(vec![Fixture::team(10, "main", TeamType::Main, vec![player])]);
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
        CountryResult::release_unsold_listed_players(&mut country, today);

        let player = ValveFx::player(&country);
        assert!(player.contract.is_none(), "the deal must be torn up");
        assert!(
            player.statuses.get().contains(&PlayerStatusType::Frt),
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
        CountryResult::release_unsold_listed_players(&mut country, today);
        assert!(
            ValveFx::player(&country).contract.is_some(),
            "a listing months old is still a sale in progress"
        );
    }

    #[test]
    fn window_close_lands_limbo_on_listed_players() {
        let mut country = ValveFx::listed_country(Fixture::date(2026, 5, 1));
        CountryResult::emit_window_close_limbo(&mut country, ValveFx::monday());
        let unsold = ValveFx::player(&country)
            .happiness
            .recent_events
            .iter()
            .filter(|e| e.event_type == HappinessEventType::UnsoldWindowClosed)
            .count();
        assert_eq!(unsold, 1, "a listed, unsold player must feel the window shut");
    }

    #[test]
    fn window_close_ignores_synthetic_listings() {
        let mut country = ValveFx::listed_country(Fixture::date(2026, 5, 1));
        country.transfer_market.listings[0].origin = TransferListingOrigin::SyntheticUnsolicited;
        CountryResult::emit_window_close_limbo(&mut country, ValveFx::monday());
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
        CountryResult::release_unsold_listed_players(&mut country, today);
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

        CountryResult::list_players_from_pipeline(&mut country, today, &mut summary);

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

        CountryResult::list_players_from_pipeline(&mut country, today, &mut summary);
        CountryResult::list_players_from_pipeline(&mut country, today, &mut summary);

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
        let analysis = CountryResult::analyze_squad_needs(&club, today);
        let player_ref = &club.teams.teams[0].players.players[0];
        let decision =
            CountryResult::evaluate_player_listing(player_ref, &analysis, &club, today, None);
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
        let analysis = CountryResult::analyze_squad_needs(&club, today);
        let player_ref = &club.teams.teams[0].players.players[0];
        let decision =
            CountryResult::evaluate_player_listing(player_ref, &analysis, &club, today, None);
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
        let analysis = CountryResult::analyze_squad_needs(&club, today);
        let player_ref = &club.teams.teams[0].players.players[0];
        let decision =
            CountryResult::evaluate_player_listing(player_ref, &analysis, &club, today, None);
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
        let analysis = CountryResult::analyze_squad_needs(&club, today);
        let player_ref = &club.teams.teams[0].players.players[0];
        let decision =
            CountryResult::evaluate_player_listing(player_ref, &analysis, &club, today, None);
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
        let analysis = CountryResult::analyze_squad_needs(&club, today);
        let player_ref = &club.teams.teams[0].players.players[0];
        let decision =
            CountryResult::evaluate_player_listing(player_ref, &analysis, &club, today, None);
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
        let analysis = CountryResult::analyze_squad_needs(&club, today);
        let player_ref = &club.teams.teams[0].players.players[0];
        let decision =
            CountryResult::evaluate_player_listing(player_ref, &analysis, &club, today, None);
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
        let analysis = CountryResult::analyze_squad_needs(&club, today);
        let player_ref = &club.teams.teams[0].players.players[0];
        let decision =
            CountryResult::evaluate_player_listing(player_ref, &analysis, &club, today, None);
        assert!(
            matches!(decision, ListingDecision::Transfer { ref reason } if reason == "dec_reason_player_requested"),
            "a formal transfer request must still list — saw {:?}",
            decision
        );
    }
}
