//! The loan market, end to end.
//!
//! The scan and the seller-side broadcast live in this file. [`guard`]
//! prices the destination against the asset, [`home`] is the pull back to
//! a player's own country, and [`interest`] is what a borrower actually
//! wants.

pub mod guard;
pub mod home;
pub mod interest;
mod scan;
#[cfg(test)]
mod tests;

pub use guard::*;
pub use home::*;

use chrono::{Datelike, Duration, NaiveDate, Weekday};
use log::debug;

use scan::LoanMarketScan;

use crate::club::player::behaviour_config::HappinessConfig;
use crate::club::player::transfer::MarketResignation;
use crate::club::staff::perception::PotentialEstimator;
use crate::club::team::squad::SquadAssetClass;
use crate::shared::{Currency, CurrencyValue};
use crate::transfers::ScoutingRegion;
use crate::transfers::deal::negotiation::NegotiationStatus;
use crate::transfers::deal::offer::{PersonalTermsOffer, TransferClause, TransferOffer};
use crate::transfers::deal::reason::TransferReason;
use crate::transfers::gate::fit::{SquadFitSnapshot, SquadRegistrationLimits};
use crate::transfers::gate::stance::PlayerStanceBuilder;
use crate::transfers::gate::{
    BuyerPlausibilityContext, EffectivePlayerReputation, TransferPlausibilityBuilder,
    TransferPlausibilityEvaluator, TransferPlausibilityVerdict,
};
use crate::transfers::loan::interest::{
    BorrowerTaste, DestinationAppeal, GroupPressure, InterestDraw, LoanApproachMemory,
    LoanCandidateProfile,
};
use crate::transfers::market::{
    TransferListing, TransferListingOrigin, TransferListingStatus, TransferListingType,
};
use crate::transfers::pipeline::processor::{PipelineProcessor, PlayerSummary};
use crate::transfers::pipeline::trace::{MarketSwitches, TransferTrace};
use crate::transfers::pipeline::{
    AvailabilityBroadcast, LoanDestinationPreference, LoanOutStatus, TransferRequestStatus,
};
use crate::transfers::squad::minutes::LoanPromise;
use crate::transfers::value::PlayerValuationCalculator;
use crate::transfers::{MarketAffinity, MarketAffinityInputs, MarketMap, MoveKind};
use crate::utils::FormattingUtils;
use crate::{
    Club, ClubPhilosophy, Country, HappinessEventCause, HappinessEventContext, HappinessEventScope,
    HappinessEventSeverity, HappinessEventType, Person, Player, PlayerFieldPositionGroup,
    PlayerStatusType, ReputationLevel, RoleFamiliarity, Team,
};
use std::collections::{HashMap, HashSet};

// Loans fund short-term development or rotation minutes. Players older
// than this are signed cheap permanent (or as free agents) rather than
// loaned, so loan targeting above it is noise regardless of whether the
// move is request-driven or opportunistic.
/// Corridor affinity a foreign loan target must clear for a club to even
/// consider him. Low, and deliberately below the permanent-move floor: a
/// loan is a cheaper, more speculative piece of business than a purchase,
/// and clubs take flyers on loanees from markets they would not buy in.
const FOREIGN_LOAN_VISIBILITY_FLOOR: f32 = 0.04;

const MAX_LOAN_TARGET_AGE: u8 = 34;

/// The three fields [`PipelineProcessor::loan_option_fee`] needs, so the
/// separate per-path action structs (domestic scan, seller broadcast,
/// foreign scan) can share one option-pricing rule instead of each
/// growing its own copy.
trait LoanOptionContext {
    fn player_id(&self) -> u32;
    fn selling_club_id(&self) -> u32;
    fn is_unsolicited(&self) -> bool;
}

impl PipelineProcessor {
    /// Pending **incoming-loan** targets per borrowing club, resolved to
    /// `(position group, current ability)`. Folded into every borrower
    /// depth snapshot so a loan already in flight counts against the
    /// position cap — the cap then holds across the broadcast → domestic
    /// → foreign scans within a tick (each registers its own negotiations
    /// before the next runs) and across days while a loan negotiation is
    /// still pending. Without it the depth gate saw only the physical
    /// roster, so a club could open several keeper loans against the same
    /// "1 GK" snapshot and overshoot the depth cap.
    fn pending_incoming_loans_by_club(
        country: &Country,
    ) -> HashMap<u32, Vec<(PlayerFieldPositionGroup, u8)>> {
        let mut map: HashMap<u32, Vec<(PlayerFieldPositionGroup, u8)>> = HashMap::new();
        for negotiation in country.transfer_market.negotiations.values() {
            if !negotiation.is_loan {
                continue;
            }
            if !matches!(
                negotiation.status,
                NegotiationStatus::Pending | NegotiationStatus::Countered
            ) {
                continue;
            }
            // Stamped profile first — a FOREIGN loan target can't be
            // resolved by the in-country walk, which made in-flight
            // cross-border loans invisible to the depth cap (a borrower
            // could stack a domestic keeper loan on top of a pending
            // foreign one against the same "1 GK" snapshot).
            if let Some(profile) = negotiation.loan_target_profile {
                map.entry(negotiation.buying_club_id)
                    .or_default()
                    .push(profile);
            } else if let Some(player) =
                Self::find_player_in_country(country, negotiation.player_id)
            {
                map.entry(negotiation.buying_club_id).or_default().push((
                    player.position().position_group(),
                    player.player_attributes.current_ability,
                ));
            }
        }
        map
    }

    /// Months from `date` to the borrower's league season end — the real
    /// duration of a rest-of-season loan. The flat 10-month stamp put a
    /// 10-month duration on every market-history row, including ~5-month
    /// January loans. Mirrors the executor's `compute_loan_end` season
    /// math (which owns the authoritative contract-side end date).
    fn loan_duration_to_season_end(
        country: &Country,
        borrower_club_id: u32,
        date: NaiveDate,
    ) -> u8 {
        let end = country
            .clubs
            .iter()
            .find(|c| c.id == borrower_club_id)
            .and_then(|c| c.teams.main().or_else(|| c.teams.teams.first()))
            .and_then(|t| t.league_id)
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
            });
        ((end - date).num_days().max(30) / 30).clamp(1, 12) as u8
    }

    // ============================================================
    // Step 6.5: Scan Loan Market — Small clubs proactively seek loans
    // ============================================================

    pub fn scan_loan_market(country: &mut Country, date: NaiveDate) {
        LoanMarketScan::run(country, date);
    }

    // ============================================================
    // Step 7a-bis: Staged Loan-Availability Broadcast (seller-side push)
    // ============================================================

    /// Days a broadcast sits at one reputation tier before, unanswered, it
    /// widens to the next tier down.
    const BROADCAST_RESPONSE_DAYS: i64 = 14;

    /// Days a posted `HomeCountry` candidate is held off the domestic push
    /// so his own league's clubs get first refusal. A fortnight — the same
    /// window one tier of the cascade gets, because that is what the
    /// preference is worth: a head start, not a veto.
    const HOME_FIRST_DAYS: i64 = 14;

    /// Seller-side loan placement. A resource-rich parent club (National
    /// reputation or above) actively offers each loan-listed player to
    /// other clubs instead of only waiting to be scanned: it broadcasts
    /// the player to the highest realistic reputation tier first (its own
    /// level), and an interested club responds by opening a loan
    /// negotiation — the existing seller-acceptance path then resolves it.
    /// If no club at the current tier responds within
    /// [`Self::BROADCAST_RESPONSE_DAYS`], the net widens one tier down and
    /// re-offers, cascading high → low until the player is placed.
    ///
    /// Complements the borrower-driven [`Self::scan_loan_market`] (pull): a
    /// prized prospect at a giant whom no small club happens to scan still
    /// gets placed, because the parent goes looking. Weekly cadence;
    /// domestic only (a foreign push compounds with the cross-border gates
    /// in [`Self::scan_foreign_loan_market`]). Reuses the same realism
    /// gates the borrower side applies — would-get-minutes depth and the
    /// reputation-drop floor — so a push never lands a player on a bench or
    /// somewhere that makes no sporting sense.
    pub fn broadcast_listed_loans(country: &mut Country, date: NaiveDate) {
        // Weekly cadence — the squad-wide push is heavier than the daily
        // listed-market scan, and a placement decision needn't be revisited
        // every day.
        if date.weekday() != Weekday::Mon {
            return;
        }
        // Read the same window the borrower-side scan reads, so the appetite
        // gate below judges a club exactly as its own scan would.
        let is_january = Self::is_mid_season_window_for(country, date);
        // The region this country plays in — every borrower in this pass
        // is in it, so the home term is derived once.
        let domestic_region = ScoutingRegion::from_country(country.continent_id, &country.code);

        // Players with an in-flight negotiation already have a pending
        // response: don't widen their net or open a second approach. Their
        // broadcast entry is preserved (not pruned) so a failed negotiation
        // resumes the cascade where it left off. LIVE negotiations only —
        // resolved rows are retained ~30 days for diagnostics, and the
        // status-blind set froze a player's cascade for weeks after a
        // rejection, contradicting the resume-where-it-left-off contract.
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
        let pending_loans = Self::pending_incoming_loans_by_club(country);

        // ── Pass 1 (read): broadcastable players ────────────────────────
        // A player is broadcastable when he carries an Available loan
        // listing (the same source the borrower scan reads — which also
        // guarantees `start_negotiation` has a listing to anchor on) AND
        // his parent club is resource-rich enough to run a push (National+).
        struct Broadcastable {
            player_id: u32,
            parent_club_id: u32,
            parent_tier: ReputationLevel,
            parent_rep: u16,
            /// Standard of the competition the parent plays in — the level
            /// the push is placing him down from.
            parent_league_rep: u16,
            parent_best_in_group: u8,
            group: PlayerFieldPositionGroup,
            ability: u8,
            /// Carried so the borrower-appetite gate can hold a pushed
            /// candidate to the age band of the request it is answering.
            age: u8,
            is_development: bool,
            asking: f64,
            /// Where the parent decided he would rather go, and enough of
            /// his passport to price it. The ranking and the appraisal have
            /// to agree about a home move, or the parent offers him
            /// somewhere he then refuses (C4).
            preference: LoanDestinationPreference,
            nationality_country_id: u32,
            nationality_region: Option<ScoutingRegion>,
            return_home_desire: f32,
            /// The asset's own price on this move — see [`LoanAssetGuard`].
            guard: Option<LoanAssetGuard>,
        }

        let mut broadcastable: Vec<Broadcastable> = Vec::new();
        // Anchors that must survive this pass although the player can't be
        // pushed right now — his listing is riding a live negotiation.
        // Pruning them made every failed loan bid restart the cascade at
        // the parent's own tier, contradicting the preserved-entry
        // contract documented above.
        let mut keep_ids: Vec<u32> = Vec::new();
        for listing in &country.transfer_market.listings {
            if listing.listing_type != TransferListingType::Loan
                || !matches!(
                    listing.status,
                    TransferListingStatus::Available | TransferListingStatus::InNegotiation
                )
            {
                continue;
            }
            // Never broadcast a synthetic row: the parent did not list this
            // player, so "the parent shops him to the market" is a lie —
            // mirrors the `SellerListed` filter on the transfer broadcast.
            if !listing.is_seller_advertised() {
                continue;
            }
            let Some(parent_club) = country.clubs.iter().find(|c| c.id == listing.club_id) else {
                continue;
            };
            let Some(parent_team) = parent_club
                .teams
                .main()
                .or_else(|| parent_club.teams.teams.first())
            else {
                continue;
            };
            let parent_tier = parent_team.reputation.level();
            // Resource gate: only National-and-above clubs run a push;
            // smaller clubs fall back to passive listing.
            if !parent_tier.runs_loan_broadcast() {
                continue;
            }
            let Some(player) = Self::find_player_in_country(country, listing.player_id) else {
                continue;
            };
            if player.is_on_loan() {
                continue;
            }
            // Same age ceiling every borrower-side scan enforces — the
            // push must not place a listed 35-year-old no scan would take.
            if player.age(date) > MAX_LOAN_TARGET_AGE {
                continue;
            }
            keep_ids.push(listing.player_id);
            if listing.status != TransferListingStatus::Available {
                continue;
            }
            let group = player.position().position_group();
            let parent_best_in_group = parent_team
                .players
                .iter()
                .filter(|p| p.position().position_group() == group)
                .map(|p| p.player_attributes.current_ability)
                .max()
                .unwrap_or(0);
            let guard = Self::loan_guard_for(country, parent_club, player, date);
            let is_development = Self::is_development_loan(guard.as_ref(), player.age(date))
                || parent_club
                    .transfer_plan
                    .loan_out_candidates
                    .iter()
                    .any(|cand| {
                        cand.player_id == listing.player_id
                            && cand.reason.expects_guaranteed_minutes()
                    });
            broadcastable.push(Broadcastable {
                player_id: listing.player_id,
                parent_club_id: listing.club_id,
                parent_tier,
                parent_rep: parent_team.reputation.world,
                parent_league_rep: Self::club_league_reputation(country, parent_club),
                parent_best_in_group,
                group,
                ability: player.player_attributes.current_ability,
                age: player.age(date),
                is_development,
                asking: listing.asking_price.amount,
                preference: parent_club
                    .transfer_plan
                    .loan_out_candidates
                    .iter()
                    .find(|cand| cand.player_id == listing.player_id)
                    .map(|cand| cand.preferred_destination)
                    .unwrap_or_default(),
                nationality_country_id: player.country_id,
                nationality_region: player.home_region(),
                return_home_desire: player.home_pull.desire,
                guard,
            });
        }

        // Prune broadcasts whose player is no longer broadcastable (sold,
        // recalled, loan agreed, parent fell below the resource tier). Runs
        // even when the list is empty so the map never accumulates. Players
        // whose listing is riding a live negotiation stay in the keep-set —
        // their cascade resumes where it left off if the bid collapses.
        Self::prune_loan_broadcasts(country, &keep_ids);
        if broadcastable.is_empty() {
            return;
        }

        // ── Pass 2 (mut clubs): advance each broadcast's tier ───────────
        // Open a broadcast at the parent's own tier; widen one tier down
        // once the current tier has gone unanswered past the response
        // window. In-negotiation players are left frozen.
        for b in &broadcastable {
            if in_negotiation.contains(&b.player_id) {
                continue;
            }
            let Some(club) = country.clubs.iter_mut().find(|c| c.id == b.parent_club_id) else {
                continue;
            };
            let next = match club.transfer_plan.loan_broadcasts.get(&b.player_id) {
                None => AvailabilityBroadcast {
                    tier: b.parent_tier,
                    since: date,
                    posted_since: date,
                },
                Some(prev) => {
                    if (date - prev.since).num_days() >= Self::BROADCAST_RESPONSE_DAYS {
                        // …but only down to the floor the asset's own
                        // standing allows. A listing is consent to a loan,
                        // not consent to any destination: walking a
                        // near-ready first-teamer one tier down every
                        // fortnight is how a fortnight's silence turned
                        // into a third-tier offer. A cascade that reaches
                        // its floor with no taker simply stays there — the
                        // 180-day loan → transfer upgrade owns what
                        // happens next.
                        let floor = b
                            .guard
                            .filter(|_| !MarketSwitches::loan_guard_off())
                            .map(|g| g.parent_reach().cascade_floor(b.parent_tier))
                            .unwrap_or(ReputationLevel::Amateur);
                        AvailabilityBroadcast {
                            tier: prev.tier.next_lower().max(floor),
                            since: date,
                            // The tier moved; the posting did not.
                            posted_since: prev.posted_since,
                        }
                    } else {
                        prev.clone()
                    }
                }
            };
            club.transfer_plan.loan_broadcasts.insert(b.player_id, next);
        }

        // ── Pass 3 (read): pick a borrower at each broadcast's tier ─────
        struct PushAction {
            borrower_id: u32,
            player_id: u32,
            selling_club_id: u32,
            offer_amount: f64,
            /// The parent listed him through the development pathway —
            /// the loan exists to buy minutes. Decides the shirt the
            /// offer promises ([`LoanPromise`]).
            is_development: bool,
        }
        // A broadcast loan is the seller's own initiative — he put the
        // player on the loan list and is shopping him around — so an
        // option to buy always belongs on it.
        impl LoanOptionContext for PushAction {
            fn player_id(&self) -> u32 {
                self.player_id
            }
            fn selling_club_id(&self) -> u32 {
                self.selling_club_id
            }
            fn is_unsolicited(&self) -> bool {
                false
            }
        }
        let mut actions: Vec<PushAction> = Vec::new();
        // One borrower must not be handed two same-group loans in a single
        // broadcast tick: per-player `has_active_negotiation_for` and the
        // registered-loan snapshot both miss it, because broadcast actions
        // aren't opened as negotiations until Pass 4.
        let mut claimed_loans: HashSet<(u32, PlayerFieldPositionGroup)> = HashSet::new();
        for b in &broadcastable {
            if in_negotiation.contains(&b.player_id) {
                continue;
            }
            // ── Home first ──────────────────────────────────────────
            //
            // A man the parent decided should go HOME is not offered
            // around his current country for the first fortnight. It is
            // one more stage at the top of the existing high → low
            // cascade, and it is what makes the preference real: without
            // it the domestic push placed him the same day the parent
            // formed the wish, and the foreign home market — which scans
            // on its own clock — never got a look.
            //
            // After the fortnight everything opens. The preference is a
            // head start, never a veto.
            let wants_home_elsewhere = b.preference == LoanDestinationPreference::HomeCountry
                && b.nationality_country_id != 0
                && b.nationality_country_id != country.id;
            if wants_home_elsewhere {
                // Against the FIRST posting, never against the current
                // tier's stamp: Pass 2 re-stamps `since` on every widen,
                // and the widen cadence IS this window, so the hold could
                // never elapse.
                let posted_for = country
                    .clubs
                    .iter()
                    .find(|c| c.id == b.parent_club_id)
                    .and_then(|c| c.transfer_plan.loan_broadcasts.get(&b.player_id))
                    .map(|br| (date - br.posted_since).num_days())
                    .unwrap_or(0);
                if posted_for < Self::HOME_FIRST_DAYS {
                    continue;
                }
            }
            // A development loanee is shopped to the WHOLE market at once: the
            // parent evaluates every club that would actually play him and sends
            // him to the best (highest-reputation) one — the strongest
            // environment where he still STARTS — instead of cascading down to
            // the first taker. The `would_get_loan_minutes` gate keeps him from
            // being placed too high (a keeper must be the undisputed #1), so the
            // "best passer" naturally falls to a lower club only when no higher
            // one has room. Cover / surplus loans keep the staged high → low
            // cascade and are placed at the current broadcast tier.
            let Some(parent_club) = country.clubs.iter().find(|c| c.id == b.parent_club_id) else {
                continue;
            };
            let restrict_tier = if b.is_development {
                None
            } else {
                match parent_club
                    .transfer_plan
                    .loan_broadcasts
                    .get(&b.player_id)
                    .map(|br| br.tier)
                {
                    Some(t) => Some(t),
                    None => continue,
                }
            };
            let parent_placements = &parent_club.transfer_plan.loan_placements;

            // Score every club that would actually play him, then draw one.
            //
            // This used to be `max_by_key` on `reputation.world`, which is the
            // most static number in the model: for a given loanee the same club
            // won it every Monday, and a failed bid changed nothing, so the
            // parent re-offered him to the club that had just said no. Worse,
            // reputation alone is not what a parent is choosing on — a prospect
            // is placed where he will play and be coached, and no borrower
            // should become a farm team. [`DestinationAppeal`] weighs all of
            // that; the gates below are untouched.
            let mut destinations: Vec<(u32, f32)> = Vec::new();
            for club in &country.clubs {
                if club.id == b.parent_club_id || club.is_rival(b.parent_club_id) {
                    continue;
                }
                // The push must not commit a borrower its own scans would
                // gate out: a full negotiation docket, or a loan fee the
                // club can't fund (the scan's 20%-of-balance / 50k
                // affordability bar) — a broke Regional club was being
                // handed 300k+ fees here. No `initialized` requirement:
                // accepting a pushed loan is a passive response, not a
                // planning action.
                if country
                    .transfer_market
                    .active_negotiation_count_for_club(club.id)
                    >= club.transfer_plan.max_concurrent_negotiations
                {
                    continue;
                }
                let borrower_balance = club.finance.balance.balance;
                let max_loan_fee = if borrower_balance < 0 {
                    50_000.0
                } else {
                    borrower_balance as f64 * 0.20
                };
                if b.asking * 0.8 > max_loan_fee {
                    continue;
                }
                let Some(team) = club.teams.main().or_else(|| club.teams.teams.first()) else {
                    continue;
                };
                if let Some(tier) = restrict_tier {
                    if team.reputation.level() != tier {
                        continue;
                    }
                }
                // …and the borrower has to actually want him. Reputation is
                // what makes a club the most attractive name on the parent's
                // list; it is not consent. Without this the push read a club
                // whose forward line was thin as an invitation, which is
                // precisely backwards at a side whose attack is carried by
                // wide men filed elsewhere.
                //
                // The upgrade exception is the other half of that: a club
                // that "only shops in January" does not turn down a genuine
                // first-team-level loanee in August, and its refusal was
                // exactly what walked the boy down to the tier below.
                let borrower_league_rep = Self::club_league_reputation(country, club);
                let borrower_best_here = team
                    .players
                    .iter()
                    .filter_map(|p| {
                        let effective = RoleFamiliarity::best_in_group(
                            &p.positions,
                            p.player_attributes.current_ability,
                            b.group,
                        );
                        (effective > 0).then_some(effective)
                    })
                    .max()
                    .unwrap_or(0);
                let borrower_profile = LoanBorrowerProfile::of(club, date, borrower_league_rep)
                    .map(|p| p.with_best_in_group(borrower_best_here));
                let upgrade_welcome = borrower_profile
                    .as_ref()
                    .zip(b.guard.as_ref())
                    .map(|(profile, guard)| {
                        let verdict = guard.assess(profile);
                        verdict.allows() && verdict.carry <= LoanAssetGuard::CARRY_MAX
                    })
                    .unwrap_or(false)
                    && !MarketSwitches::loan_guard_off();
                if !LoanBorrowerAppetite::assess(club, team, is_january).accepts_push(
                    club,
                    b.group,
                    b.ability,
                    b.age,
                    borrower_best_here,
                    upgrade_welcome,
                ) {
                    continue;
                }
                if country
                    .transfer_market
                    .has_active_negotiation_for(b.player_id, club.id)
                {
                    continue;
                }
                if claimed_loans.contains(&(club.id, b.group)) {
                    continue;
                }
                // A club that has already gone in for this player and got
                // nowhere is not the place to send him again this month. Same
                // standoff the borrower's own scans respect, read from the
                // other side of the deal — without it the push could re-offer
                // the same name to the same club every Monday, which is the
                // pairing that made one borrower look welded to one target.
                if club
                    .transfer_plan
                    .is_loan_approach_barred(b.player_id, date)
                {
                    continue;
                }
                let borrower_rep = team.reputation.world;
                let depth = BorrowerPositionDepth::snapshot(team)
                    .with_pending_loans(pending_loans.get(&club.id).map_or(&[], |v| v.as_slice()));
                let level = LoanDestinationLevel {
                    ability: b.ability,
                    parent_best_in_group: b.parent_best_in_group,
                    parent_rep: b.parent_rep,
                    borrower_rep,
                    parent_league_rep: b.parent_league_rep,
                    borrower_league_rep,
                    is_development: b.is_development,
                };
                if !depth.has_room_for(b.group, b.ability, b.is_development)
                    || !depth.would_get_loan_minutes(
                        b.group,
                        b.ability,
                        b.is_development,
                        b.parent_best_in_group,
                    )
                    || !level.is_plausible()
                {
                    continue;
                }
                if !Self::loan_guard_allows(
                    b.guard.as_ref(),
                    borrower_profile.as_ref(),
                    b.player_id,
                ) {
                    continue;
                }
                destinations.push((
                    club.id,
                    DestinationAppeal {
                        borrower_rep,
                        borrower_league_rep: level.borrower_league_rep,
                        parent_league_rep: b.parent_league_rep,
                        minutes_headroom: depth.minutes_headroom(b.group, b.ability),
                        training_rating: club.facilities.training.to_rating(),
                        existing_placements: LoanApproachMemory::crowding_at(
                            parent_placements,
                            club.id,
                            date,
                        ),
                        is_development: b.is_development,
                        // Same term the borrower-side scan and the
                        // appraisal read, so all three agree about an
                        // Argentine loaned within Brazil (C4).
                        home_pull: HomeLoanPull::factor(
                            b.nationality_country_id,
                            b.nationality_region,
                            country.id,
                            domestic_region,
                            b.return_home_desire,
                        ),
                    }
                    .score(),
                ));
            }
            if let Some(borrower_id) = InterestDraw::pick(&destinations) {
                actions.push(PushAction {
                    borrower_id,
                    player_id: b.player_id,
                    is_development: b.is_development,
                    selling_club_id: b.parent_club_id,
                    offer_amount: FormattingUtils::round_fee(b.asking * 0.8),
                });
                claimed_loans.insert((borrower_id, b.group));
            }
        }

        // ── Pass 4 (mut market): open the loan negotiations ─────────────
        // The interested club's "response". The player already carries an
        // Available loan listing, so `start_negotiation` has its anchor.
        for action in actions {
            let selling_rep = Self::get_club_reputation(country, action.selling_club_id);
            let buying_rep = Self::get_club_reputation(country, action.borrower_id);
            let (p_age, p_ambition) =
                Self::get_player_negotiation_data(country, action.player_id, date);

            let mut clauses = Vec::new();
            if let Some(option_fee) = Self::loan_option_fee(country, &action, date) {
                clauses.push(TransferClause::LoanOptionToBuy(CurrencyValue {
                    amount: option_fee,
                    currency: Currency::Usd,
                }));
            }

            let offer = TransferOffer {
                base_fee: CurrencyValue {
                    amount: action.offer_amount,
                    currency: Currency::Usd,
                },
                clauses,
                contract_length_years: None,
                loan_duration_months: Some(Self::loan_duration_to_season_end(
                    country,
                    action.borrower_id,
                    date,
                )),
                personal_terms: Some(PersonalTermsOffer {
                    // A loan promises a shirt: minutes are what it is
                    // for (B4 / [`LoanPromise`]). No wage — the borrower
                    // picks up a share of the deal he already has.
                    squad_status_promise: LoanPromise::for_loan(action.is_development),
                    ..PersonalTermsOffer::default()
                }),
                offering_club_id: action.borrower_id,
                offered_date: date,
            };

            if let Some(neg_id) = country.transfer_market.start_negotiation(
                action.player_id,
                action.borrower_id,
                offer,
                date,
                selling_rep,
                buying_rep,
                p_age,
                p_ambition,
            ) {
                let (p_name, sc_name) = Self::resolve_player_and_club_name(
                    country,
                    action.player_id,
                    action.selling_club_id,
                );
                if let Some(negotiation) = country.transfer_market.negotiations.get_mut(&neg_id) {
                    negotiation.is_loan = true;
                    negotiation.reason = TransferReason::key("signing_reason_loan_broadcast");
                    negotiation.player_name = p_name;
                    negotiation.selling_club_name = sc_name;
                }
                // Remember where this went. Read back by `DestinationAppeal`
                // as crowding, so the parent spreads its loanees around
                // instead of re-offering to whoever topped the ranking last
                // week — including when that club had just turned him down.
                if let Some(parent) = country
                    .clubs
                    .iter_mut()
                    .find(|c| c.id == action.selling_club_id)
                {
                    parent
                        .transfer_plan
                        .record_loan_placement(action.borrower_id, date);
                }
                // The borrower has this name in front of it now; its own scans
                // shouldn't independently chase the same player next tick.
                if let Some(borrower) = country
                    .clubs
                    .iter_mut()
                    .find(|c| c.id == action.borrower_id)
                {
                    borrower
                        .transfer_plan
                        .record_loan_approach(action.player_id, date);
                }
                debug!(
                    "Loan broadcast: parent {} placed listed player {} at borrower {}",
                    action.selling_club_id, action.player_id, action.borrower_id
                );
            }
        }
    }

    /// Drop broadcast entries whose player is no longer broadcastable.
    /// `live_ids` is the set still in play this pass; everything else is
    /// removed so the per-club map stays bounded.
    fn prune_loan_broadcasts(country: &mut Country, live_ids: &[u32]) {
        for club in &mut country.clubs {
            if !club.transfer_plan.loan_broadcasts.is_empty() {
                club.transfer_plan
                    .loan_broadcasts
                    .retain(|pid, _| live_ids.contains(pid));
            }
        }
    }

    // ============================================================
    // Step 7a-ter: Staged Transfer-Availability Broadcast (stale listings)
    // ============================================================

    /// Short grace a fresh permanent listing gets on the pull-side market
    /// before the club starts actively shopping him. Replaces the old
    /// 90-day dead zone in which nothing seller-side touched a listing for
    /// three months; the club now offers him to peers within weeks and only
    /// widens the tier reach if he stays unsold (see `broadcast_listed_transfers`).
    const TRANSFER_BROADCAST_GRACE_DAYS: i64 = 21;

    /// Seller-side placement for STALE permanent listings — the
    /// permanent-transfer mirror of [`Self::broadcast_listed_loans`],
    /// kept alongside it so the two cascades share every helper. A
    /// transfer-listed player the pull-side market has ignored past the
    /// grace weeks asks the club to find him a new team (a visible note
    /// on his events feed), and the club's scouts respond by offering
    /// him to other clubs: opening at the club's own reputation tier
    /// and widening one tier down every unanswered response window —
    /// cumulatively, so every level from the club's own down to the
    /// cascade's reach stays in the running — until a club opens a
    /// normal purchase negotiation. Unlike the loan push there is no
    /// National+ resource gate — a stranded listing is a wage problem
    /// for any club. Together with the year-unsold free-exit valve this
    /// guarantees a listing RESOLVES: sold via the push, or — rarely,
    /// when even the widened market wants no part of him — the player
    /// leaves on a free.
    pub fn broadcast_listed_transfers(country: &mut Country, date: NaiveDate) {
        // Weekly cadence, mirroring the loan push.
        if date.weekday() != Weekday::Mon {
            return;
        }

        // In-flight negotiations already carry a pending response — don't
        // widen their net or open a competing approach. LIVE negotiations
        // only, mirroring the loan push: resolved rows are retained ~30
        // days for diagnostics, and a status-blind set froze a player's
        // cascade for weeks after every rejected bid.
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

        // ── Pass 1 (read): stale genuine seller listings ────────────────
        // Synthetic / unsolicited listings never enter the push — only a
        // listing the club actually advertised represents a player it
        // wants moved.
        struct Sellable {
            player_id: u32,
            parent_club_id: u32,
            parent_tier: ReputationLevel,
            group: PlayerFieldPositionGroup,
            ability: u8,
            /// His passport. A responding club at its foreigner quota has no
            /// registration slot for him, loan or purchase alike.
            nationality_country_id: u32,
            /// Age and observable ceiling, carried so a responding club can
            /// run its own squad-fit maths on the candidate. The ceiling is
            /// the staff-free potential proxy — never the hidden PA.
            age: u8,
            observable_ceiling: u8,
            asking: f64,
            listed_date: NaiveDate,
        }

        let mut sellable: Vec<Sellable> = Vec::new();
        // Broadcast anchors that must SURVIVE this pass even though the
        // player can't be pushed right now — his listing is riding a live
        // negotiation. Pruning them lost the cascade's `since` anchor, so
        // every failed bid restarted the tier walk at the seller's own
        // level (contradicting the resume-where-it-left-off contract).
        let mut keep_ids: Vec<u32> = Vec::new();
        for listing in &country.transfer_market.listings {
            if listing.listing_type != TransferListingType::Transfer
                || listing.origin != TransferListingOrigin::SellerListed
                || !matches!(
                    listing.status,
                    TransferListingStatus::Available | TransferListingStatus::InNegotiation
                )
            {
                continue;
            }
            if (date - listing.listed_date).num_days() < Self::TRANSFER_BROADCAST_GRACE_DAYS {
                continue;
            }
            let Some(parent_club) = country.clubs.iter().find(|c| c.id == listing.club_id) else {
                continue;
            };
            let Some(parent_team) = parent_club
                .teams
                .main()
                .or_else(|| parent_club.teams.teams.first())
            else {
                continue;
            };
            let Some(player) = Self::find_player_in_country(country, listing.player_id) else {
                continue;
            };
            if player.is_on_loan() {
                continue;
            }
            keep_ids.push(listing.player_id);
            if listing.status != TransferListingStatus::Available {
                continue;
            }
            sellable.push(Sellable {
                player_id: listing.player_id,
                parent_club_id: listing.club_id,
                parent_tier: parent_team.reputation.level(),
                group: player.position().position_group(),
                ability: player.player_attributes.current_ability,
                nationality_country_id: player.country_id,
                age: player.age(date),
                observable_ceiling: PotentialEstimator::observable_ceiling(player, date),
                asking: listing.asking_price.amount,
                listed_date: listing.listed_date,
            });
        }

        Self::prune_transfer_broadcasts(country, &keep_ids);
        if sellable.is_empty() {
            return;
        }

        // ── Pass 2 (mut clubs): open / advance each broadcast's tier ────
        // A first-time entry is the player's ask made real — remember him
        // so the player-mut pass below can put the request on his feed.
        let mut newly_asking: Vec<u32> = Vec::new();
        for s in &sellable {
            if in_negotiation.contains(&s.player_id) {
                continue;
            }
            let Some(club) = country.clubs.iter_mut().find(|c| c.id == s.parent_club_id) else {
                continue;
            };
            // Anchor the cascade on when the LISTING cleared its grace —
            // stable, never reset — so the tier reach widens continuously
            // with real time on the market. A save loaded with an old
            // unsold listing resumes the cascade at the depth its age has
            // already earned instead of politely re-opening at the
            // seller's own tier; the eligible-tier band below is
            // cumulative (own tier down to the cascade tier), so a deep
            // resume never skips the levels in between.
            let opened = s.listed_date + Duration::days(Self::TRANSFER_BROADCAST_GRACE_DAYS);
            let first_since = club
                .transfer_plan
                .transfer_broadcasts
                .get(&s.player_id)
                .map(|b| b.since)
                .unwrap_or(opened);
            if !club
                .transfer_plan
                .transfer_broadcasts
                .contains_key(&s.player_id)
            {
                newly_asking.push(s.player_id);
            }
            let steps_down =
                ((date - first_since).num_days().max(0) / Self::BROADCAST_RESPONSE_DAYS) as u32;
            let tier = s.parent_tier.step_down(steps_down);
            club.transfer_plan.transfer_broadcasts.insert(
                s.player_id,
                AvailabilityBroadcast {
                    tier,
                    since: first_since,
                    // The transfer cascade already anchors on the date the
                    // listing cleared its grace, so the two agree.
                    posted_since: first_since,
                },
            );
        }

        // ── Pass 2b (mut players): the ask itself ───────────────────────
        // A few weeks unsold on the list — the player (or his agent) formally
        // tells the club to find him a real destination, coinciding with the
        // club opening the seller push. Cooldowned so a cascade re-opened
        // after a failed negotiation doesn't spam.
        for player_id in newly_asking {
            for club in &mut country.clubs {
                for team in &mut club.teams.teams {
                    if let Some(player) =
                        team.players.players.iter_mut().find(|p| p.id == player_id)
                    {
                        let magnitude = HappinessConfig::default()
                            .catalog
                            .asked_club_to_arrange_transfer;
                        let happiness_ctx = HappinessEventContext::new(
                            HappinessEventCause::Other,
                            HappinessEventSeverity::from_magnitude(magnitude),
                            HappinessEventScope::Boardroom,
                        );
                        player.happiness.add_event_with_context_and_cooldown(
                            HappinessEventType::AskedClubToArrangeTransfer,
                            magnitude,
                            None,
                            happiness_ctx,
                            120,
                        );
                    }
                }
            }
        }

        // ── Pass 3 (read): pick a buyer at each broadcast's tier ────────
        struct PushAction {
            buyer_id: u32,
            player_id: u32,
            selling_club_id: u32,
            offer_amount: f64,
        }
        let mut actions: Vec<PushAction> = Vec::new();
        // One buyer must not be handed two same-group purchases in a
        // single broadcast tick — the actions aren't negotiations yet, so
        // `has_active_negotiation_for` can't see them.
        let mut claimed: HashSet<(u32, PlayerFieldPositionGroup)> = HashSet::new();
        // Every club's own surplus maths, per position group in play.
        // Answering a broadcast IS a signing, so it has to clear the same
        // fit test the scouted recruitment paths already apply: a club
        // that would classify the arrival as surplus on day one must not
        // buy him, because its own rebalance and release passes would
        // move him straight back out. Built once here (the tier walk
        // below re-reads the same clubs for every listing) and keyed by
        // club id so the borrow stays read-only.
        let broadcast_groups: HashSet<PlayerFieldPositionGroup> =
            sellable.iter().map(|s| s.group).collect();
        let mut fit_by_club_group: HashMap<(u32, PlayerFieldPositionGroup), SquadFitSnapshot> =
            HashMap::new();
        let registration = SquadRegistrationLimits::new(country.id, &country.regulations);
        for club in &country.clubs {
            for &group in &broadcast_groups {
                fit_by_club_group.insert(
                    (club.id, group),
                    SquadFitSnapshot::build(club, group, date, registration),
                );
            }
        }
        for s in &sellable {
            if in_negotiation.contains(&s.player_id) {
                continue;
            }
            let Some(tier) = country
                .clubs
                .iter()
                .find(|c| c.id == s.parent_club_id)
                .and_then(|c| c.transfer_plan.transfer_broadcasts.get(&s.player_id))
                .map(|br| br.tier)
            else {
                continue;
            };

            // A modest push discount: the unsold weeks already proved the
            // headline asking wrong; the responding club opens just under
            // the (already market-decayed) asking and the normal
            // negotiation resolves the rest (the seller-side fee floors
            // still protect against a giveaway).
            let offer_amount = FormattingUtils::round_fee(s.asking * 0.85);

            // Weeks unsold past the grace, 0..1 across the same ~half
            // season the seller's fee floor and the player's resignation
            // run on. It relaxes the buyer-side CA ceiling below: a
            // stale listing IS the bargain-above-your-level that a lower
            // club stretches for in real markets.
            let staleness = (((date - s.listed_date).num_days()
                - Self::TRANSFER_BROADCAST_GRACE_DAYS) as f32
                / MarketResignation::RAMP_DAYS)
                .clamp(0.0, 1.0);
            let ceiling_relax = (staleness * 35.0).round() as u8;

            // Highest world reputation among qualifying clubs wins — the
            // best home that would actually take him. Eligibility is the
            // CUMULATIVE band from the seller's own tier down to the
            // cascade's current reach: a tier already offered stays in
            // the running while the net widens (the old exact-tier match
            // silently un-offered every level the cascade had passed, so
            // once it saturated at the bottom tier a decent player could
            // never be bought by anyone at all).
            let mut best: Option<(u32, u16)> = None;
            for club in &country.clubs {
                if club.id == s.parent_club_id || club.is_rival(s.parent_club_id) {
                    continue;
                }
                let Some(team) = club.teams.main().or_else(|| club.teams.teams.first()) else {
                    continue;
                };
                if !(tier..=s.parent_tier).contains(&team.reputation.level()) {
                    continue;
                }
                if country
                    .transfer_market
                    .has_active_negotiation_for(s.player_id, club.id)
                {
                    continue;
                }
                if claimed.contains(&(club.id, s.group)) {
                    continue;
                }
                // The buyer must be able to fund the fee from its window
                // plan.
                let plan = &club.transfer_plan;
                let available = plan.total_budget - plan.spent - plan.reserved;
                if offer_amount > available {
                    continue;
                }
                // Tier-window realism: the player must be a plausible
                // squad member at the buyer's level — neither so weak
                // he'd never play (the cascade will reach a lower tier)
                // nor too far above the buyer's target ceiling. The
                // ceiling relaxes continuously with staleness: nobody at
                // his own level wanted him, so the market's real price
                // is a level below the tag, and an ambitious lower club
                // punches up for exactly this kind of deal.
                let rep_score = team.reputation.overall_score();
                let floor = Self::tier_starter_ca_score(rep_score, s.group).saturating_sub(20);
                let ceiling = Self::tier_target_ceiling_score(rep_score, s.group)
                    .saturating_add(ceiling_relax);
                if s.ability < floor || s.ability > ceiling {
                    continue;
                }
                // Wages are deliberately NOT gated here: plausibility
                // waives its wage floor for a genuinely listed player
                // (availability opens the door; the wage question is
                // settled at personal terms, where his reservation
                // collapses to the buyer's tier), and the broadcast
                // holds to the same contract.
                // Depth: no point buying into a full position line.
                let depth = BorrowerPositionDepth::snapshot(team);
                if !depth.has_room_for(s.group, s.ability, false) {
                    continue;
                }
                // …and no point buying a player this club's own systems
                // would classify as surplus the day he lands. The tier
                // window above asks whether he is a plausible body at
                // this LEVEL of football; this asks whether he is a
                // plausible body in THIS squad, which is the question
                // the club actually has to live with. A free keeper slot
                // under the ideal-depth rule is not the same thing as a
                // squad with room for him: the release pass reads the
                // depth cap and the squad average, and if those already
                // say "surplus" then signing him only books the churn.
                if fit_by_club_group
                    .get(&(club.id, s.group))
                    .is_some_and(|fit| {
                        fit.would_be_surplus(s.ability, s.observable_ceiling, s.age)
                        // A loanee occupies a registration slot exactly like
                        // a signing. A club at its foreigner quota has no
                        // room for one, whoever holds the registration.
                        || fit.would_be_unregistrable(s.nationality_country_id)
                    })
                {
                    continue;
                }
                let rep = team.reputation.world;
                match best {
                    Some((_, best_rep)) if rep <= best_rep => {}
                    _ => best = Some((club.id, rep)),
                }
            }
            if let Some((buyer_id, _)) = best {
                actions.push(PushAction {
                    buyer_id,
                    player_id: s.player_id,
                    selling_club_id: s.parent_club_id,
                    offer_amount,
                });
                claimed.insert((buyer_id, s.group));
            }
        }

        // ── Pass 4 (mut market): open the purchase negotiations ─────────
        // The interested club's "response". The player carries an
        // Available seller listing, so `start_negotiation` has its anchor
        // and the ordinary seller-acceptance path resolves the deal.
        for action in actions {
            let selling_rep = Self::get_club_reputation(country, action.selling_club_id);
            let buying_rep = Self::get_club_reputation(country, action.buyer_id);
            let (p_age, p_ambition) =
                Self::get_player_negotiation_data(country, action.player_id, date);

            let offer = TransferOffer {
                base_fee: CurrencyValue {
                    amount: action.offer_amount,
                    currency: Currency::Usd,
                },
                clauses: Vec::new(),
                contract_length_years: None,
                loan_duration_months: None,
                personal_terms: None,
                offering_club_id: action.buyer_id,
                offered_date: date,
            };

            if let Some(neg_id) = country.transfer_market.start_negotiation(
                action.player_id,
                action.buyer_id,
                offer,
                date,
                selling_rep,
                buying_rep,
                p_age,
                p_ambition,
            ) {
                let (p_name, sc_name) = Self::resolve_player_and_club_name(
                    country,
                    action.player_id,
                    action.selling_club_id,
                );
                if let Some(negotiation) = country.transfer_market.negotiations.get_mut(&neg_id) {
                    negotiation.reason = TransferReason::key("signing_reason_listing_broadcast");
                    negotiation.player_name = p_name;
                    negotiation.selling_club_name = sc_name;
                }
                debug!(
                    "Transfer broadcast: club {} offered stale-listed player {} to buyer {}",
                    action.selling_club_id, action.player_id, action.buyer_id
                );
            }
        }
    }

    /// Drop transfer-broadcast entries whose player is no longer
    /// broadcastable (sold, delisted, gone on a free). Mirror of
    /// [`Self::prune_loan_broadcasts`].
    fn prune_transfer_broadcasts(country: &mut Country, live_ids: &[u32]) {
        for club in &mut country.clubs {
            if !club.transfer_plan.transfer_broadcasts.is_empty() {
                club.transfer_plan
                    .transfer_broadcasts
                    .retain(|pid, _| live_ids.contains(pid));
            }
        }
    }

    // ============================================================
    // Step 7b: Loan Market Scanning (other countries)
    // ============================================================

    /// Age band the proactive foreign pickup will take a man in.
    ///
    /// A COMPATRIOT his parent has posted is judged by the band the
    /// posting itself uses ([`UnsettledAbroadScan::MAX_AGE`], the same 25
    /// the manager-talk loan route reads); anybody else is a cold
    /// development pickup and keeps the 23.
    ///
    /// Two bands because they answer different questions. "Would a
    /// smaller club take a stranger's prospect off him?" is a development
    /// question. "Would his own league take back one of its own, whose
    /// club has said he can go?" is not — it is a homecoming, and the two
    /// years between them are exactly the population the posting model
    /// exists for.
    fn home_pickup_age_ok(
        age: u8,
        home_return_wanted: bool,
        nationality_country_id: u32,
        borrower_country_id: u32,
    ) -> bool {
        let coming_home = home_return_wanted
            && nationality_country_id != 0
            && nationality_country_id == borrower_country_id;
        if coming_home {
            age <= UnsettledAbroadScan::MAX_AGE
        } else {
            ForeignUnsolicitedLoanTarget::is_development(age)
        }
    }

    pub fn scan_foreign_loan_market(
        country: &mut Country,
        foreign_players: &[&PlayerSummary],
        date: NaiveDate,
        market_map: &MarketMap,
    ) {
        if foreign_players.is_empty() {
            return;
        }

        let is_january = Self::is_mid_season_window_for(country, date);

        // The scanning country's own region — used to block loans from
        // clearly more prestigious regions (Paraguay can't loan from England).
        let country_id = country.id;
        let club_region = ScoutingRegion::from_country(country.continent_id, &country.code);
        let club_region_prestige = club_region.league_prestige();

        // How plausible each source market is for THIS country, memoised on
        // the PAIR `(passport, league he plays in)`. The borrowing side of
        // the market has the same geography as the buying side: a Turkish
        // club takes loans from the leagues Turkish clubs deal with, not
        // from wherever a loanable body happens to be listed.
        //
        // Keyed on the league alone (the first cut) a Brazilian at Porto and
        // a Portuguese at Porto shared one entry, and the first of the two
        // scored decided the geography for both.
        let mut visibility_cache: HashMap<(u32, u32), f32> = HashMap::new();
        let mut market_visibility = |p: &PlayerSummary| -> f32 {
            if market_map.is_silent() {
                return 1.0;
            }
            let key = (p.nationality_country_id, p.country_id);
            if let Some(cached) = visibility_cache.get(&key) {
                return *cached;
            }
            let affinity = MarketAffinity::affinity(
                market_map,
                MarketAffinityInputs {
                    buyer_country_id: country_id,
                    nationality_country_id: key.0,
                    current_country_id: key.1,
                    kind: MoveKind::Talent,
                    // Scanned per country, one answer for the league.
                    benefactor: 0.0,
                },
            );
            visibility_cache.insert(key, affinity);
            affinity
        };

        // Collect loan-listed foreign players
        // Only consider players from countries with equal or lower reputation,
        // and whose home region isn't far above the scanning club's region.
        let country_rep = country.reputation;
        let foreign_loans: Vec<&PlayerSummary> = foreign_players
            .iter()
            .copied()
            .filter(|p| {
                // Either the parent advertised the loan, or the player is a
                // credible cold target his club isn't building around — the
                // `Loa` badge is not required for a loan approach. Importance
                // is gated below by the staged (unsolicited) plausibility.
                let approachable = p.is_loan_listed
                    || p.home_return_wanted
                    || (p.age <= MAX_LOAN_TARGET_AGE
                        && ForeignUnsolicitedLoanTarget::looks_loanable(
                            p.age,
                            p.skill_ability,
                            p.club_best_in_group,
                        ));
                if !approachable {
                    return false;
                }
                // A player is never a stranger in his own country. The
                // level gates below read truth and stay exactly as they
                // were — what changes is that his own federation is not
                // "a smaller country" to him, and his own continent is a
                // smaller step than a stranger's (L7.3).
                let is_home_country =
                    p.nationality_country_id != 0 && p.nationality_country_id == country_id;
                let is_home_region = !is_home_country && p.nationality_region == Some(club_region);
                // Country-reputation step-down. A player from a more
                // prestigious footballing nation isn't a realistic loan-in
                // for a smaller country — EXCEPT development-profile
                // youngsters, who routinely drop a national tier for
                // guaranteed senior minutes (Russia → Belarus, an Argentine
                // prospect → a smaller league). The region-prestige gate
                // below and the club-rep reality band downstream still bound
                // how far the move can fall.
                if !HomeLoanGates::country_rep_ok(
                    Self::foreign_loan_country_rep_ok(
                        p.country_reputation,
                        country_rep,
                        ForeignUnsolicitedLoanTarget::is_development(p.age),
                    ),
                    is_home_country,
                ) {
                    return false;
                }
                // Cross-region prestige step-down. A settled player won't loan
                // down into a clearly smaller ecosystem for a bit-part role, but
                // a development youngster drops abroad for guaranteed minutes
                // (an Italian U18 → Romania). See `foreign_loan_region_ok`.
                // `p.region` is precomputed at pool-build time.
                if !HomeLoanGates::region_ok(
                    Self::foreign_loan_region_ok(
                        p.region.league_prestige(),
                        club_region_prestige,
                        ForeignUnsolicitedLoanTarget::is_development(p.age),
                    ),
                    p.region.league_prestige(),
                    club_region_prestige,
                    is_home_country,
                    is_home_region,
                ) {
                    return false;
                }
                // Corridor visibility. A borrowing club looks in the
                // markets its league works, the same as a buying one — the
                // loan market is not a separate geography. Home is exempt
                // by construction (affinity 1.0), which is what keeps the
                // loan-home pathway untouched.
                market_visibility(p) >= FOREIGN_LOAN_VISIBILITY_FLOOR
            })
            .collect();

        if foreign_loans.is_empty() {
            return;
        }

        // Posted compatriots, hoisted out of the per-club loop: the ONLY
        // foreign loan market an Elite or Continental club opens, and a
        // country-wide fact that has no business being recomputed per club.
        let compatriots: Vec<&PlayerSummary> = foreign_loans
            .iter()
            .copied()
            .filter(|p| p.home_return_wanted && p.nationality_country_id == country_id)
            .collect();

        // Position-group partition for the request loop below — each
        // request used to walk the whole filtered pool to reject the
        // other three groups candidate by candidate. The proactive
        // development pickup after the request loop crosses groups and
        // keeps using the flat `foreign_loans`. Selection is a weighted
        // draw over the whole qualified slate either way, so the
        // partition is a cost saving and nothing more — it no longer
        // has to preserve an ordering for a tie-break to land on.
        let mut foreign_loans_by_group: [Vec<&PlayerSummary>; PlayerFieldPositionGroup::COUNT] =
            Default::default();
        for p in &foreign_loans {
            foreign_loans_by_group[p.position_group.index()].push(*p);
        }

        struct ForeignLoanAction {
            club_id: u32,
            player: PlayerSummary,
            offer_amount: f64,
            reason: TransferReason,
            /// Age-classified development loan — the borrower cleared the
            /// stricter minutes gate, so the offer promises a regular
            /// shirt ([`LoanPromise`]).
            is_development: bool,
            /// This approach came from the compatriot sweep rather than
            /// the ordinary foreign scan — one per club per window.
            from_compatriot_sweep: bool,
            /// What he is to his current club, and how far the move
            /// falls — both read from the staged plausibility model here,
            /// because at resolution time his club is abroad.
            player_importance: f32,
            sporting_drop: f32,
        }

        let mut actions: Vec<ForeignLoanAction> = Vec::new();
        let pending_loans = Self::pending_incoming_loans_by_club(country);

        // Negotiations are only created AFTER the club loop (the staged
        // `actions` apply below), so the market's active set is frozen for
        // the whole scan — snapshot the per-club counts and (player, club)
        // pairs once instead of linear-scanning `negotiations` per club and
        // per candidate.
        let active_counts = country.transfer_market.active_negotiation_counts();
        let active_pairs = country.transfer_market.active_negotiation_pairs();

        // Rotate first refusal, as the domestic scan does: the per-pass dedup
        // below is "has anybody claimed him yet", so registration order handed
        // the lowest-id club the pick of the continent every tick.
        for club_idx in InterestDraw::visit_order(country.clubs.len()) {
            let club = &country.clubs[club_idx];
            if club.teams.teams.is_empty() {
                continue;
            }

            let plan = &club.transfer_plan;
            if !plan.initialized {
                continue;
            }

            let team = &club.teams.teams[0];
            let rep_level = team.reputation.level();

            // Elite clubs buy — they don't scan loan markets.
            // Continental only in January with negative balance.
            // Local/Amateur don't have the scouting reach for foreign markets.
            // Who runs the ORDINARY foreign loan scan — the request path
            // plus the proactive pickup, over the whole foreign pool.
            let ordinary_foreign_scan = match rep_level {
                ReputationLevel::Elite => false,
                ReputationLevel::Continental => is_january && club.finance.balance.balance < 0,
                ReputationLevel::National | ReputationLevel::Regional => true,
                _ => false, // Local/Amateur
            };
            // …and who runs the COMPATRIOT SWEEP instead.
            //
            // A compatriot the world has been told would come home is a
            // different proposition from a foreign loan market. Flamengo
            // and Palmeiras do not trawl Europe for loanees — and they do
            // sign the boy their own league produced when his club posts
            // him. So the sweep opens exactly that door: the proactive
            // branch alone, over the posted-compatriot slice alone, once
            // per club per window.
            //
            // It used to be a country-wide `any()` that simply flipped
            // `should_scan_foreign`, after which both branches iterated
            // the FULL foreign pool with no compatriot restriction: one
            // posted English 21-year-old at Ajax had every Elite English
            // club running the ordinary foreign scan every pass for as
            // long as he stayed posted.
            let compatriot_sweep = !ordinary_foreign_scan
                && !MarketSwitches::compatriot_sweep_off()
                && matches!(
                    rep_level,
                    ReputationLevel::Elite | ReputationLevel::Continental
                )
                && plan.compatriot_sweeps_this_window == 0
                && !compatriots.is_empty();
            if !ordinary_foreign_scan && !compatriot_sweep {
                continue;
            }
            // The slice each branch actually looks at.
            let scan_pool: &[&PlayerSummary] = if ordinary_foreign_scan {
                &foreign_loans
            } else {
                &compatriots
            };
            // Standard of football on offer here — the division gate reads
            // it against the parent's own competition, which travels on the
            // summary (C12).
            let borrower_league_rep = Self::club_league_reputation(country, club);

            // Check concurrent negotiation limits
            let actual_active = active_counts.get(&club.id).copied().unwrap_or(0);
            if actual_active >= plan.max_concurrent_negotiations {
                continue;
            }

            let balance = club.finance.balance.balance;
            let max_loan_fee = if balance < 0 {
                30_000.0
            } else {
                (balance as f64 * 0.15).min(500_000.0)
            };

            let avg_ability = {
                let avg = team.players.current_ability_avg();
                if avg == 0 { 50 } else { avg }
            };

            // Get scout known regions for this club
            let scout_regions: Vec<ScoutingRegion> = club
                .teams
                .teams
                .iter()
                .flat_map(|t| t.staffs.iter())
                .flat_map(|s| s.staff_attributes.knowledge.known_regions.iter().copied())
                .collect();

            // Check unfulfilled transfer requests. Emergency
            // free-agent depth requests are excluded — they're
            // serviced by the free-agent matcher only, not by loans.
            let unfulfilled = plan.transfer_requests.iter().filter(|r| {
                r.status != TransferRequestStatus::Fulfilled
                    && r.status != TransferRequestStatus::Abandoned
                    && !r.is_emergency_free_agent_depth()
            });

            let mut scans = 0usize;
            let max_scans: usize = match rep_level {
                ReputationLevel::Elite => 3,
                ReputationLevel::Continental => 2,
                _ => 1,
            };

            // Track position groups already targeted to avoid multiple negotiations
            // for the same position (e.g. FormationGap + DepthCover for GK)
            let mut scanned_position_groups: Vec<PlayerFieldPositionGroup> = Vec::new();

            // Borrower-side depth snapshot — the same gate the
            // domestic scan uses to avoid loaning into an already-full
            // position. Building it once here keeps the filter inside
            // `foreign_loans.iter()` cheap.
            let borrower_position_depth = BorrowerPositionDepth::snapshot(team)
                .with_pending_loans(pending_loans.get(&club.id).map_or(&[], |v| v.as_slice()));
            // What this club's year and payroll can carry — the same two
            // money terms the domestic scan prices, which cross a border
            // unchanged: a Segunda club's revenue is a Segunda club's
            // revenue whoever the parent is.
            let foreign_borrower_profile = LoanBorrowerProfile::of(club, date, borrower_league_rep);
            let foreign_borrower_for =
                |group: PlayerFieldPositionGroup| -> Option<LoanBorrowerProfile> {
                    foreign_borrower_profile
                        .map(|p| p.with_best_in_group(borrower_position_depth.best_in_group(group)))
                };

            // Staged-plausibility buyer context, built once per club so the
            // foreign-loan filter can run the same cross-border veto the
            // scouting / permanent paths use. The candidate `PlayerSummary`
            // carries the seller-side context, so no selling-country ref is
            // needed here.
            let buyer_loan_ctx = BuyerPlausibilityContext::build(country, club, date);

            // Same preference model the domestic scan uses. The cross-border
            // branches ranked on `(same_region, skill_ability)`, which made the
            // continent's single strongest listed prospect the one target every
            // eligible club in the region opened with.
            let taste = BorrowerTaste::of(
                club,
                club.teams
                    .teams
                    .iter()
                    .map(|t| {
                        t.staffs
                            .resolve_for_transfers()
                            .best_scout_judging_ability()
                    })
                    .max()
                    .unwrap_or(5),
                avg_ability,
                max_loan_fee,
            );
            let open_request_groups: Vec<PlayerFieldPositionGroup> = plan
                .transfer_requests
                .iter()
                .filter(|r| {
                    r.status != TransferRequestStatus::Fulfilled
                        && r.status != TransferRequestStatus::Abandoned
                })
                .map(|r| r.position.position_group())
                .collect();
            // Cultural proximity stays a preference, as it was — but as a
            // weight on interest rather than a lexicographic key that no
            // amount of quality could outrank.
            let foreign_interest = |p: &PlayerSummary, fee: f64| -> Option<f32> {
                taste
                    .interest_in(&LoanCandidateProfile {
                        player_id: p.player_id,
                        true_ability: p.skill_ability,
                        age: p.age,
                        is_development: ForeignUnsolicitedLoanTarget::is_development(p.age),
                        fee,
                        group_thinness: GroupPressure::thinness(
                            borrower_position_depth.headcount(p.position_group),
                            p.position_group,
                        ),
                        answers_open_request: open_request_groups.contains(&p.position_group),
                    })
                    .map(|score| {
                        // Local supply is real: a club looks in its own
                        // backyard first. Kept exactly as it was — the
                        // home pull competes with it rather than
                        // replacing it, and the census decides.
                        let local = if p.region == club_region { 1.45 } else { 1.0 };
                        // …and a man's own country pulls him back, in
                        // proportion to how much he wants it. Note this
                        // reads his NATIONALITY region against the
                        // borrower's, where `local` above reads his
                        // CLUB's — the two used to be the same field, and
                        // the same-region term therefore worked AGAINST a
                        // return home.
                        score
                            * local
                            * HomeLoanPull::factor(
                                p.nationality_country_id,
                                p.nationality_region,
                                country_id,
                                club_region,
                                p.return_home_desire,
                            )
                    })
            };

            // The request path walks the whole foreign pool by position
            // group, so it belongs to the ordinary scan alone — a
            // compatriot sweep never runs it.
            for request in unfulfilled.filter(|_| ordinary_foreign_scan) {
                if scans >= max_scans {
                    break;
                }

                let pos_group = request.position.position_group();
                if scanned_position_groups.contains(&pos_group) {
                    continue;
                }

                let relaxed_min = request.min_ability.saturating_sub(5);

                // Filter foreign loan players: must match position, ability,
                // be in a scout's known region, and be a realistic move
                // (players don't go from Serie A to the Nigerian league)
                let team_rep = team.reputation.world;
                let qualified: Vec<&&PlayerSummary> = foreign_loans_by_group[pos_group.index()]
                    .iter()
                    .filter(|p| {
                        !club.is_rival(p.club_id)
                            && !plan.is_loan_approach_barred(p.player_id, date)
                            && p.position_group == request.position.position_group()
                            && p.skill_ability >= relaxed_min
                            && p.age <= request
                                .preferred_age_max
                                .saturating_add(3)
                                .min(MAX_LOAN_TARGET_AGE)
                            && p.age >= request.preferred_age_min
                            && p.estimated_value * 0.1 <= max_loan_fee
                            // Squad-average floor is for cover loans; a youth
                            // match-practice loan leans on the minutes gate.
                            && (ForeignUnsolicitedLoanTarget::is_development(p.age)
                                || p.skill_ability >= avg_ability.saturating_sub(10))
                            && !active_pairs.contains(&(p.player_id, club.id))
                            && !actions.iter().any(|a| a.player.player_id == p.player_id)
                            // Reputation reality check: players don't drop more than
                            // ~40% in league level on loan. A rep-8000 player won't
                            // go to a rep-2000 club. This prevents Serie A → Nigeria.
                            && p.home_reputation <= (team_rep as f32 * 2.0) as i16
                            && team_rep >= (p.home_reputation.max(0) as f32 * 0.35) as u16
                            // `p.region` is stamped from the same
                            // (continent, country-code) resolve at pool
                            // build time — no per-candidate re-resolve.
                            && HomeLoanGates::reach_ok(
                                scout_regions.contains(&p.region),
                                p.nationality_country_id,
                                p.nationality_region,
                                country_id,
                                club_region,
                                p.home_return_wanted,
                                ForeignUnsolicitedLoanTarget::is_development(p.age),
                            )
                            // Borrower-depth gate: if the borrower's
                            // squad is already full at this position
                            // group, accept only when the incoming
                            // player is clearly better than what's
                            // already there. Mirrors the domestic
                            // `should_skip_loan` shape so we don't
                            // import a 4th mid-tier GK on top of three.
                            && borrower_position_depth.has_room_for(
                                p.position_group,
                                p.skill_ability,
                                ForeignUnsolicitedLoanTarget::is_development(p.age),
                            )
                            // The loan must buy minutes — not a bench
                            // seat behind a wall of better players.
                            // Young loanees (the development profile)
                            // need the stricter expected-minutes bar.
                            && borrower_position_depth.would_get_loan_minutes(
                                p.position_group,
                                p.skill_ability,
                                ForeignUnsolicitedLoanTarget::is_development(p.age),
                                p.club_best_in_group,
                            )
                            // Parent-club reputation drop — same realism
                            // gate as the domestic scan, anchored on the
                            // player's own club rather than his personal
                            // reputation.
                            && Self::loan_level_ok(
                                team_rep,
                                p.club_world_reputation.max(0) as u16,
                                p.skill_ability,
                                p.club_best_in_group,
                                ForeignUnsolicitedLoanTarget::is_development(p.age),
                                // Both competitions are in hand on a
                                // cross-border loan: his is on the
                                // summary, the borrower's is local (C12).
                                p.seller_ctx.league_reputation,
                                borrower_league_rep,
                            )
                            // Staged cross-border veto: an important player at
                            // a much stronger club abroad isn't a credible
                            // loan target even when loan-listed, unless his
                            // availability genuinely opens the move. Mirrors
                            // the permanent foreign gate.
                            && !matches!(
                                TransferPlausibilityBuilder::evaluate_summary(
                                    &buyer_loan_ctx,
                                    p,
                                    true,
                                    true,
                                    date,
                                        None,
                                ),
                                Some(TransferPlausibilityVerdict::HardReject(_))
                            )
                            // The asset's own price on this destination —
                            // the two money terms cross a border unchanged.
                            && Self::foreign_loan_guard_allows(
                                p,
                                foreign_borrower_for(p.position_group).as_ref(),
                            )
                    })
                    .collect();

                let weighted: Vec<(u32, f32)> = qualified
                    .iter()
                    .enumerate()
                    .filter_map(|(i, p)| {
                        foreign_interest(p, p.estimated_value * 0.1 * 0.8)
                            .map(|score| (i as u32, score))
                    })
                    .collect();

                if let Some(best) = InterestDraw::pick(&weighted).map(|i| *qualified[i as usize]) {
                    let loan_fee = FormattingUtils::round_fee(best.estimated_value * 0.1 * 0.8);
                    // Same shape as the domestic request scan — the empty
                    // `format!` here dropped the request's "why" from every
                    // foreign request-driven loan's history row.
                    let reason = TransferReason::key(request.reason.as_signing_reason_key());
                    let staged = ForeignLoanStance::read(&buyer_loan_ctx, best, date);
                    actions.push(ForeignLoanAction {
                        club_id: club.id,
                        player: (*best).clone(),
                        offer_amount: loan_fee,
                        reason,
                        is_development: ForeignUnsolicitedLoanTarget::is_development(best.age),
                        from_compatriot_sweep: false,
                        player_importance: staged.0,
                        sporting_drop: staged.1,
                    });
                    scanned_position_groups.push(pos_group);
                    scans += 1;
                }
            }

            // ── Proactive foreign development pickup (no request needed) ──
            //
            // The request loop above only signs a foreign loanee when THIS
            // club already asked for the position — so a giant's loan-listed
            // prospect (a River Plate youth keeper no domestic club can field
            // as a #1) has no cross-border outlet, because the seller
            // broadcast is domestic-only. Mirror the domestic opportunistic /
            // unsolicited scan here: a National / Regional club proactively
            // takes a loan-listed DEVELOPMENT prospect from a same-region,
            // equal-or-higher-rep country when it would actually play him.
            // The country-rep / region step-downs (already lifted for
            // development when `foreign_loans` was built) plus the
            // borrower-depth minutes gate keep the drop plausible and
            // guarantee minutes, so prospects circulate across the continent
            // (Argentina → Uruguay / Chile / …) without landing on a bench.
            // Bounded by the same per-club scan budget as the request path.
            let team_rep = team.reputation.world;
            if scans < max_scans {
                let prospects: Vec<&&PlayerSummary> = scan_pool
                    .iter()
                    .filter(|p| {
                        // A posted compatriot is his own advert — his
                        // parent has told the world he would come home,
                        // which is exactly the row a loan listing is.
                        //
                        // …and he is not held to the 23 the ordinary cold
                        // pickup uses. `UnsettledAbroadScan` posts men up
                        // to 25, and so does the manager-talk route, so a
                        // posted 24- or 25-year-old compatriot passed
                        // every pool filter and was then invisible to
                        // every Elite / Continental club in his own
                        // country — reachable only by a National side
                        // with an open request in his position.
                        (p.is_loan_listed || p.home_return_wanted)
                            && Self::home_pickup_age_ok(
                                p.age,
                                p.home_return_wanted,
                                p.nationality_country_id,
                                country_id,
                            )
                            && !club.is_rival(p.club_id)
                            && !plan.is_loan_approach_barred(p.player_id, date)
                            && !scanned_position_groups.contains(&p.position_group)
                            && p.estimated_value * 0.1 <= max_loan_fee
                            && !active_pairs.contains(&(p.player_id, club.id))
                            && !actions.iter().any(|a| a.player.player_id == p.player_id)
                            // Reputation reality band — identical to the
                            // request path, so a prospect still can't drop
                            // into a far smaller ecosystem than his club's.
                            && p.home_reputation <= (team_rep as f32 * 2.0) as i16
                            && team_rep >= (p.home_reputation.max(0) as f32 * 0.35) as u16
                            && HomeLoanGates::reach_ok(
                                scout_regions.contains(&p.region),
                                p.nationality_country_id,
                                p.nationality_region,
                                country_id,
                                club_region,
                                p.home_return_wanted,
                                true,
                            )
                            // Development profile throughout (gated above), so
                            // every borrower-side gate runs at its dev setting:
                            // the relaxed keeper room check (Fix A) lets him
                            // into a full-but-weak line, and the minutes gate
                            // guarantees he competes rather than sits.
                            && borrower_position_depth.has_room_for(
                                p.position_group,
                                p.skill_ability,
                                true,
                            )
                            && borrower_position_depth.would_get_loan_minutes(
                                p.position_group,
                                p.skill_ability,
                                true,
                                p.club_best_in_group,
                            )
                            && Self::loan_level_ok(
                                team_rep,
                                p.club_world_reputation.max(0) as u16,
                                p.skill_ability,
                                p.club_best_in_group,
                                true,
                                // Both competitions are in hand on a
                                // cross-border loan: his is on the
                                // summary, the borrower's is local (C12).
                                p.seller_ctx.league_reputation,
                                borrower_league_rep,
                            )
                            && !matches!(
                                TransferPlausibilityBuilder::evaluate_summary(
                                    &buyer_loan_ctx,
                                    p,
                                    true,
                                    true,
                                    date,
                                        None,
                                ),
                                Some(TransferPlausibilityVerdict::HardReject(_))
                            )
                            // The asset's own price on this destination —
                            // the two money terms cross a border unchanged.
                            && Self::foreign_loan_guard_allows(
                                p,
                                foreign_borrower_for(p.position_group).as_ref(),
                            )
                    })
                    .collect();

                // Same-region prospects still circulate locally first — that
                // preference now lives in the weight, so a genuinely better fit
                // on another continent is reachable instead of unreachable.
                let weighted: Vec<(u32, f32)> = prospects
                    .iter()
                    .enumerate()
                    .filter_map(|(i, p)| {
                        foreign_interest(p, p.estimated_value * 0.1 * 0.8)
                            .map(|score| (i as u32, score))
                    })
                    .collect();

                if let Some(best) = InterestDraw::pick(&weighted).map(|i| *prospects[i as usize]) {
                    // Terminal branch for this club: nothing after this reads
                    // the scan counter or the scanned-groups set, so the loan
                    // action is all that's needed.
                    let loan_fee = FormattingUtils::round_fee(best.estimated_value * 0.1 * 0.8);
                    let staged = ForeignLoanStance::read(&buyer_loan_ctx, best, date);
                    actions.push(ForeignLoanAction {
                        club_id: club.id,
                        player: (*best).clone(),
                        offer_amount: loan_fee,
                        reason: TransferReason::key("signing_reason_loan_foreign_prospect"),
                        is_development: ForeignUnsolicitedLoanTarget::is_development(best.age),
                        player_importance: staged.0,
                        sporting_drop: staged.1,
                        from_compatriot_sweep: compatriot_sweep,
                    });
                }
            }
        }

        if actions.is_empty() {
            return;
        }

        // Create listings and negotiations for foreign loan targets
        for action in actions {
            let asking_price = CurrencyValue {
                amount: action.offer_amount,
                currency: Currency::Usd,
            };

            // Unsolicited foreign approaches target players their club never
            // loan-listed: tag the backing listing synthetic so the resolver
            // withholds the "seller-advertised" acceptance bonus. A genuinely
            // loan-listed foreign target keeps the seller-advertised origin.
            let is_unsolicited = !action.player.is_loan_listed;
            let listing = TransferListing::new_with_origin(
                action.player.player_id,
                action.player.club_id,
                0, // Foreign team — no local team_id
                asking_price.clone(),
                date,
                TransferListingType::Loan,
                if is_unsolicited {
                    TransferListingOrigin::SyntheticUnsolicited
                } else {
                    TransferListingOrigin::LoanOutListed
                },
            );
            country.transfer_market.add_listing(listing);

            let buying_rep = Self::get_club_reputation(country, action.club_id);
            // Use a reasonable estimate for selling club rep
            let selling_rep = (action.player.skill_ability as f32 / 200.0).clamp(0.1, 0.9);

            // Same as the domestic loan path — explicit months-side
            // duration so the market history record matches the actual
            // loan length. The borrower is in the scanning country.
            // A boy going home comes with a buy option. It is how these
            // deals are actually written — Gerson, Kaio Jorge, Matheus
            // Pereira all converted — and it is where the ~30 % conversion
            // band in the census comes from. The option is withheld on an
            // ordinary cold approach (a club does not price a permanent it
            // has not asked about); a posted compatriot is not a cold
            // approach, it is a homecoming his parent advertised.
            let coming_home = action.player.home_return_wanted
                && action.player.nationality_country_id != 0
                && action.player.nationality_country_id == country.id;
            let mut clauses = Vec::new();
            if coming_home && action.player.estimated_value > 0.0 {
                clauses.push(TransferClause::LoanOptionToBuy(CurrencyValue {
                    amount: FormattingUtils::round_fee(
                        action.player.estimated_value * Self::LOAN_OPTION_VALUE_FRACTION,
                    ),
                    currency: Currency::Usd,
                }));
            }
            let offer = TransferOffer {
                base_fee: asking_price,
                clauses,
                contract_length_years: None,
                loan_duration_months: Some(Self::loan_duration_to_season_end(
                    country,
                    action.club_id,
                    date,
                )),
                personal_terms: Some(PersonalTermsOffer {
                    // A loan promises a shirt: minutes are what it is
                    // for (B4 / [`LoanPromise`]). No wage — the borrower
                    // picks up a share of the deal he already has.
                    squad_status_promise: LoanPromise::for_loan(action.is_development),
                    ..PersonalTermsOffer::default()
                }),
                offering_club_id: action.club_id,
                offered_date: date,
            };

            if let Some(neg_id) = country.transfer_market.start_negotiation(
                action.player.player_id,
                action.club_id,
                offer,
                date,
                selling_rep,
                buying_rep,
                action.player.age,
                action.player.determination,
            ) {
                if let Some(negotiation) = country.transfer_market.negotiations.get_mut(&neg_id) {
                    negotiation.is_loan = true;
                    negotiation.has_option_to_buy = coming_home;
                    negotiation.is_unsolicited = is_unsolicited;
                    negotiation.reason = action.reason;
                    negotiation.selling_country_id = Some(action.player.country_id);
                    negotiation.selling_continent_id = Some(action.player.continent_id);
                    negotiation.selling_country_code = action.player.country_code.clone();
                    negotiation.player_name = action.player.player_name.clone();
                    negotiation.selling_club_name = action.player.club_name.clone();
                    // The borrower depth fold can't resolve a foreign
                    // player in-country — the stamped profile keeps this
                    // in-flight loan visible to the position cap.
                    negotiation.loan_target_profile =
                        Some((action.player.position_group, action.player.skill_ability));
                    // The player is in another country's borrow by the time
                    // he is asked, so his side of the appraisal is captured
                    // here from the market summary — which is exactly what
                    // this borrower actually knows about him. Without it a
                    // cross-border loan fell back to the bare prestige wall
                    // that made going home the least likely destination on
                    // the board (L7.6).
                    negotiation.staged_stance = Some(PlayerStanceBuilder::from_summary(
                        &action.player,
                        action.player_importance,
                        action.player.region,
                    ));
                    negotiation.staged_sporting_drop = Some(action.sporting_drop);
                    negotiation.open_salary_at(action.player.salary);
                }

                // Same standoff the domestic scan stamps — a club that has
                // moved for a foreign target doesn't reopen the file the
                // following tick.
                if let Some(buyer) = country.clubs.iter_mut().find(|c| c.id == action.club_id) {
                    buyer
                        .transfer_plan
                        .record_loan_approach(action.player.player_id, date);
                    if action.from_compatriot_sweep {
                        // One homecoming per window. The sweep is a door
                        // for the boy a league produced, not a licence to
                        // shop abroad.
                        buyer.transfer_plan.compatriot_sweeps_this_window = buyer
                            .transfer_plan
                            .compatriot_sweeps_this_window
                            .saturating_add(1);
                    }
                }

                debug!(
                    "Foreign loan scan: Club {} started foreign loan negotiation for player {} from country {}",
                    action.club_id, action.player.player_id, action.player.country_id
                );
            }
        }
    }

    /// Realism gate for the borrower side of a domestic loan: players
    /// don't drop from a giant to a minnow for a bit-part role.
    /// `borrower_rep` / `parent_rep` are main-team world reputations
    /// (0..10000).
    ///
    /// **Readiness, not age, decides how far a player may drop.** The signal is
    /// how far he sits below the parent's best at his position:
    ///   * A genuinely raw player (`very_raw` — 25+ below the best) tolerates
    ///     the biggest drop: for him ANY senior football is the point, and the
    ///     stricter `would_get_loan_minutes` gate already guarantees he plays
    ///     wherever he lands. A raw *development* youngster (the classic
    ///     teenage keeper) has the floor lifted entirely so he can drop to a
    ///     small club where he STARTS; a raw non-development player keeps a
    ///     light 0.12 floor.
    ///   * Everyone else — INCLUDING a young **displaced first-choice** who is
    ///     close to the parent's best — is held to the peer-level 0.25 floor.
    ///     He moves down a tier or two for minutes, not several tiers to a
    ///     minnow's bench. (Previously the age-based `is_development` flag
    ///     lifted the floor for him too, which is exactly what sent polished
    ///     young regulars tumbling far below their level — e.g. a Serie A
    ///     first-choice keeper loaned to Serie C.)
    ///
    /// The club-standing half of [`LoanDestinationLevel`] on its own —
    /// what [`Self::loan_level_ok`] reduces to when neither competition is
    /// known. Every production path now has league context (C12), so this
    /// is the named entry point the gate's own tests measure against.
    #[cfg(test)]
    fn loan_reputation_drop_ok(
        borrower_rep: u16,
        parent_rep: u16,
        player_ability: u8,
        parent_best_in_group: u8,
        is_development: bool,
    ) -> bool {
        Self::loan_level_ok(
            borrower_rep,
            parent_rep,
            player_ability,
            parent_best_in_group,
            is_development,
            0,
            0,
        )
    }

    /// Both halves of [`LoanDestinationLevel`], with the league context a
    /// caller can supply.
    ///
    /// The cross-border scan used to pass 0/0 and stand the division gate
    /// down entirely, although both reputations are in hand: the borrower's
    /// own competition is local, and the parent's rides on the market
    /// summary. Zero on either side still stands the gate down — that is
    /// its own "unknown competition" rule, not a suspension.
    #[allow(clippy::too_many_arguments)]
    fn loan_level_ok(
        borrower_rep: u16,
        parent_rep: u16,
        player_ability: u8,
        parent_best_in_group: u8,
        is_development: bool,
        parent_league_rep: u16,
        borrower_league_rep: u16,
    ) -> bool {
        LoanDestinationLevel {
            ability: player_ability,
            parent_best_in_group,
            parent_rep,
            borrower_rep,
            parent_league_rep,
            borrower_league_rep,
            is_development,
        }
        .is_plausible()
    }

    /// Fraction of a player's market value an option to buy is struck at.
    /// Below his value because the borrowing club is taking the risk and
    /// paying the wages for a season; matching the permanent path's own
    /// option pricing.
    const LOAN_OPTION_VALUE_FRACTION: f64 = 0.7;
    /// Age above which a loan is squad-clearing rather than an investment,
    /// so no option is written. Nobody buys a 31-year-old at the end of a
    /// cover loan.
    const LOAN_OPTION_MAX_AGE: u8 = 30;

    /// Strike price for an option to buy on this loan, or `None` when the
    /// deal isn't one an option belongs on.
    ///
    /// Cold, unsolicited approaches are excluded: the parent never
    /// advertised the player, and a club that hasn't decided to sell him
    /// doesn't hand over a purchase right as part of a loan it was talked
    /// into. Everything else — a player his club put on the loan list — is
    /// exactly the deal that carries one in reality.
    fn loan_option_fee<A>(country: &Country, action: &A, date: NaiveDate) -> Option<f64>
    where
        A: LoanOptionContext,
    {
        if action.is_unsolicited() {
            return None;
        }
        let selling_club = country
            .clubs
            .iter()
            .find(|c| c.id == action.selling_club_id())?;
        let player = selling_club.teams.teams.iter().find_map(|t| {
            t.players
                .players
                .iter()
                .find(|p| p.id == action.player_id())
        })?;
        if player.age(date) > Self::LOAN_OPTION_MAX_AGE {
            return None;
        }
        let league_rep = Self::club_league_reputation(country, selling_club);
        let club_rep = selling_club
            .teams
            .main()
            .map(|t| t.reputation.world)
            .unwrap_or(0);
        let value = player.value(date, league_rep, club_rep);
        if value <= 0.0 {
            return None;
        }
        Some(FormattingUtils::round_fee(
            value * Self::LOAN_OPTION_VALUE_FRACTION,
        ))
    }

    /// Price the parent side of a loan for this player — the same seller
    /// context the valuation and every other sell-side reading resolve, so
    /// the guard and the market quote one number.
    pub(crate) fn loan_guard_for(
        country: &Country,
        club: &Club,
        player: &Player,
        date: NaiveDate,
    ) -> Option<LoanAssetGuard> {
        let (league_rep, club_rep) = PlayerValuationCalculator::seller_context(country, club);
        LoanAssetGuard::for_player(club, player, date, league_rep, club_rep)
    }

    /// Is this a DEVELOPMENT loan — one the player needs because he is
    /// below his club's own level — rather than merely a loan of somebody
    /// young? Falls back to the age band when the parent side could not be
    /// read, and reverts to it entirely on the `OF_LOAN_GUARD_OFF` arm.
    pub(crate) fn is_development_loan(guard: Option<&LoanAssetGuard>, age: u8) -> bool {
        if MarketSwitches::loan_guard_off() {
            return age <= UnsolicitedLoanTarget::DEVELOPMENT_AGE;
        }
        guard
            .map(|g| g.is_development())
            .unwrap_or(age <= UnsolicitedLoanTarget::DEVELOPMENT_AGE)
    }

    /// Would the guard let this loan reach this borrower? `None` on either
    /// side stands the guard down — it never invents a verdict from
    /// missing facts — and so does the `OF_LOAN_GUARD_OFF` arm.
    fn loan_guard_allows(
        guard: Option<&LoanAssetGuard>,
        borrower: Option<&LoanBorrowerProfile>,
        player_id: u32,
    ) -> bool {
        if MarketSwitches::loan_guard_off() {
            return true;
        }
        let (Some(guard), Some(borrower)) = (guard, borrower) else {
            return true;
        };
        let verdict = guard.assess(borrower);
        if TransferTrace::is(player_id) {
            TransferTrace::line(player_id, "loan", guard.diagnostics(borrower, &verdict));
        }
        verdict.allows()
    }

    /// One `loan` trace line per (traced player, candidate borrower):
    /// every destination gate's own reading, side by side, so the funnel
    /// can be read as a table instead of re-derived from file:line.
    ///
    /// Always returns `true` — it is a diagnostic, never a gate — so a
    /// caller folds it into its filter chain wherever it wants the reading
    /// taken. Costs one cached `OnceLock` read when disarmed.
    #[allow(clippy::too_many_arguments)]
    fn trace_loan_destination(
        player_id: u32,
        borrower_name: &str,
        group: PlayerFieldPositionGroup,
        ability: u8,
        is_development: bool,
        parent_best_in_group: u8,
        level: &LoanDestinationLevel,
        depth: &BorrowerPositionDepth,
    ) -> bool {
        if !TransferTrace::is(player_id) {
            return true;
        }
        TransferTrace::line(
            player_id,
            "loan",
            format!(
                "borrower={borrower_name} dev={is_development} readiness={:.2} \
                 division_floor={:.3} rep={}/{} league={}/{} standing_ok={} division_ok={} \
                 room={} minutes={} best_here={}",
                level.readiness(),
                level.division_floor(),
                level.borrower_rep,
                level.parent_rep,
                level.borrower_league_rep,
                level.parent_league_rep,
                level.clears_club_standing(),
                level.clears_division(),
                depth.has_room_for(group, ability, is_development),
                depth.would_get_loan_minutes(group, ability, is_development, parent_best_in_group),
                depth.best_in_group(group),
            ),
        );
        true
    }

    /// [`Self::loan_guard_allows`] across a border, where the parent side
    /// is only reachable through the player's summary.
    fn foreign_loan_guard_allows(
        target: &PlayerSummary,
        borrower: Option<&LoanBorrowerProfile>,
    ) -> bool {
        if MarketSwitches::loan_guard_off() {
            return true;
        }
        let Some(borrower) = borrower else {
            return true;
        };
        let guard = LoanAssetGuard::from_summary(
            target.skill_ability,
            target.age,
            target.position_group,
            target.club_world_reputation,
            target.club_best_in_group,
            target.seller_ctx.league_reputation,
            target.estimated_value,
            target.salary,
            target.is_loan_listed,
            EffectivePlayerReputation::compute(
                target.world_reputation,
                target.current_reputation,
                target.home_reputation,
                false,
            ),
        );
        let verdict = guard.assess(borrower);
        if TransferTrace::is(target.player_id) {
            TransferTrace::line(
                target.player_id,
                "loan",
                format!("foreign {}", guard.diagnostics(borrower, &verdict)),
            );
        }
        verdict.allows()
    }

    /// The guard's verdict alone, for the paths that need the reach rather
    /// than the yes/no — the broadcast cascade's floor tier and the
    /// Continental cold approach, which is peer-level business only.
    fn loan_guard_reach(
        guard: Option<&LoanAssetGuard>,
        borrower: Option<&LoanBorrowerProfile>,
    ) -> Option<LoanReach> {
        if MarketSwitches::loan_guard_off() {
            return None;
        }
        let (guard, borrower) = (guard?, borrower?);
        Some(guard.assess(borrower).reach)
    }

    /// League reputation of a club's main competition, or 0 when the club
    /// plays no league at all (a friendly-only side). Zero suspends the
    /// division gate, which then defers to the club-standing one rather than
    /// inventing a verdict.
    pub(crate) fn club_league_reputation(country: &Country, club: &Club) -> u16 {
        club.teams
            .main()
            .or_else(|| club.teams.teams.first())
            .and_then(|t| t.league_id)
            .and_then(|lid| country.leagues.leagues.iter().find(|l| l.id == lid))
            .map(|l| l.reputation)
            .unwrap_or(0)
    }

    /// Cross-border country-reputation gate for the foreign loan market.
    /// A player from a more prestigious footballing nation isn't a
    /// realistic loan-in for a smaller country: an established fringe
    /// player would rather stay or move sideways than drop a national tier
    /// for a bit-part role abroad, so the borrower's country must be at
    /// least as reputable as the player's.
    ///
    /// `is_development` lifts the gate entirely — the deliberate exception.
    /// A development-profile youngster (≤23) routinely loans DOWN a country
    /// tier for guaranteed senior minutes (Russia → Belarus, an Argentine
    /// prospect → a smaller league); vetoing that on country reputation
    /// alone is exactly what blocked the realistic "go abroad to play"
    /// move. The drop is still bounded downstream by the region-prestige
    /// gate and the club-rep reality band, so this can't launder a
    /// wonderkid into a clearly smaller ecosystem. Mirrors the
    /// `is_development` lift in [`Self::loan_reputation_drop_ok`].
    fn foreign_loan_country_rep_ok(
        player_country_rep: u16,
        borrower_country_rep: u16,
        is_development: bool,
    ) -> bool {
        if is_development {
            return true;
        }
        player_country_rep <= borrower_country_rep
    }

    /// Cross-region prestige step-down for a foreign loan. A settled player from
    /// a more prestigious football region won't loan down into a clearly smaller
    /// ecosystem for a bit-part role — but a development youngster (≤23) accepts
    /// a much larger drop to go abroad for guaranteed minutes (an Italian U18 →
    /// Romania / Russia). Without this lift a Western-European prospect was
    /// region-LOCKED: only borrowers within +0.20 of his own 1.0 prestige
    /// qualify, i.e. only Western Europe, so a giant's youngster never moved
    /// abroad at all. The wider development allowance reaches the mid regions
    /// (Eastern Europe / Scandinavia / South America, ~0.45-0.50) for senior
    /// football but still falls short of the bottom regions, and the downstream
    /// club-rep reality band bounds the actual destination. Mirrors the
    /// development lift in [`Self::foreign_loan_country_rep_ok`].
    fn foreign_loan_region_ok(
        player_region_prestige: f32,
        club_region_prestige: f32,
        is_development: bool,
    ) -> bool {
        // A settled player tolerates only a small step down in region prestige;
        // a development youngster goes much further for minutes.
        let allowance = if is_development { 0.55 } else { 0.20 };
        player_region_prestige <= club_region_prestige + allowance
    }

    /// List loan-out candidates on the transfer market.
    pub(crate) fn process_loan_out_listings(country: &mut Country, date: NaiveDate) {
        let pending: Vec<(u32, u32)> = country
            .clubs
            .iter()
            .flat_map(|club| {
                club.transfer_plan
                    .loan_out_candidates
                    .iter()
                    .filter(|c| c.status == LoanOutStatus::Identified)
                    .map(move |c| (club.id, c.player_id))
            })
            .collect();

        for (club_id, player_id) in pending {
            Self::list_loan_out_candidate(country, club_id, player_id, date);
        }
    }

    /// List ONE identified loan-out candidate on the market. Returns
    /// true when a new listing was created. Deduped against existing
    /// listings, so the daily pipeline pass and the Phase-C development-
    /// pathway staging can both call it without double-listing.
    pub(crate) fn list_loan_out_candidate(
        country: &mut Country,
        club_id: u32,
        player_id: u32,
        date: NaiveDate,
    ) -> bool {
        if country
            .transfer_market
            .get_listing_by_player(player_id)
            .is_some()
        {
            return false;
        }

        let price_level = country.settings.pricing.price_level;
        let listing = {
            let Some(club) = country.clubs.iter().find(|c| c.id == club_id) else {
                return false;
            };
            let Some(candidate) = club
                .transfer_plan
                .loan_out_candidates
                .iter()
                .find(|c| c.player_id == player_id && c.status == LoanOutStatus::Identified)
            else {
                return false;
            };
            let Some(player) = Self::find_player_in_club(club, player_id) else {
                return false;
            };
            if player.is_on_loan() {
                return false;
            }

            let team_id = club.teams.teams.first().map(|t| t.id).unwrap_or(0);

            let asking_price = if candidate.loan_fee > 0.0 {
                CurrencyValue {
                    amount: candidate.loan_fee,
                    currency: Currency::Usd,
                }
            } else {
                // Loan fee is ~10% of player value, not full value
                let (seller_league_rep, seller_club_rep) =
                    PlayerValuationCalculator::seller_context(country, club);
                let full_value = PlayerValuationCalculator::calculate_value_with_price_level(
                    player,
                    date,
                    price_level,
                    seller_league_rep,
                    seller_club_rep,
                );
                CurrencyValue {
                    amount: FormattingUtils::round_fee(full_value.amount * 0.10),
                    currency: full_value.currency,
                }
            };

            TransferListing::new(
                player_id,
                club.id,
                team_id,
                asking_price,
                date,
                TransferListingType::Loan,
            )
        };

        country.transfer_market.add_listing(listing);

        if let Some(club) = country.clubs.iter_mut().find(|c| c.id == club_id) {
            if let Some(candidate) = club
                .transfer_plan
                .loan_out_candidates
                .iter_mut()
                .find(|c| c.player_id == player_id)
            {
                candidate.status = LoanOutStatus::Listed;
            }

            for team in &mut club.teams.teams {
                if let Some(player) = team.players.players.iter_mut().find(|p| p.id == player_id) {
                    if !player.statuses.has(PlayerStatusType::Loa) {
                        player.statuses.add(date, PlayerStatusType::Loa);
                    }
                }
            }
        }

        true
    }
}

/// Eligibility policy for an *unsolicited* domestic loan — a smaller club
/// approaching a bigger one for a player his club has NOT loan-listed. The
/// `Loa` badge is deliberately not required; loan demand should not depend
/// on the parent advertising the player. The realism is in WHO is
/// approachable, decided by the central [`SquadAssetClass`] classifier that
/// the audit and listing paths already share.
/// The level a proposed loan actually asks a player to play at: the standing
/// of the two clubs, the standard of the two competitions, and how close he
/// already is to his parent club's first team.
///
/// Both level realism gates live here so no call site can apply one and skip
/// the other. Reputation alone was never enough: a well-supported
/// second-division club and a top-flight one sit close on reputation while
/// playing in different divisions, so the loan market — which knew only club
/// reputation — happily placed top-flight regulars a tier down.
struct LoanDestinationLevel {
    /// Current ability of the player being loaned.
    ability: u8,
    /// Best current ability in his position group on the parent's main
    /// roster — the standard he is measured against at his own club.
    parent_best_in_group: u8,
    /// Main-team world reputations (0..10000).
    parent_rep: u16,
    borrower_rep: u16,
    /// Reputations of the competitions the two clubs play in. Zero means
    /// "no league" (a friendly-only side), which suspends the division gate
    /// rather than guessing.
    parent_league_rep: u16,
    borrower_league_rep: u16,
    /// The loan exists to buy match practice, so a bigger drop is the point
    /// of the move rather than a demotion.
    is_development: bool,
}

impl LoanDestinationLevel {
    /// Ratio of own ability to the parent's best at the position at which a
    /// player reads as genuinely raw — years away from that shirt.
    const RAW_ABILITY_RATIO: f32 = 0.60;
    /// Ratio at which he reads as ready to compete for it.
    const READY_ABILITY_RATIO: f32 = 0.90;
    /// Share of the parent's league level a raw player may drop to …
    const RAW_LEAGUE_FLOOR: f32 = 0.45;
    /// … and the much tighter share a near-ready one may.
    const READY_LEAGUE_FLOOR: f32 = 0.85;
    /// Room the drop gets for being a drop the player NEEDS, at its
    /// widest — a raw youngster playing every week two divisions down is
    /// the point of his move.
    ///
    /// This used to be a flat multiplier applied to every loan of anybody
    /// aged 23 or under, which made the readiness curve above a fiction:
    /// a parent's own best forward reads readiness 1.0 and a floor of
    /// 0.85, and the blanket allowance dropped it to 0.6375 — under the
    /// 0.707 that separates a top flight from its second division. The
    /// allowance is now continuous in the same readiness the floor is: a
    /// raw player keeps the full width, a first-team-ready one gets none
    /// of it and is held to his parent's own level.
    const RAW_LEAGUE_ALLOWANCE: f32 = 0.25;
    /// Club-standing floor a raw non-development loanee is held to …
    const RAW_STANDING_FLOOR: f32 = 0.12;
    /// … and the extra share a fully first-team-ready one adds to it, so a
    /// ready player's destination is a peer rather than "a quarter of my
    /// club", which is what a flat 0.25 made credible.
    const READY_STANDING_SPAN: f32 = 0.63;

    /// Both gates. A destination has to be a credible club **and** a
    /// credible division.
    fn is_plausible(&self) -> bool {
        self.clears_club_standing() && self.clears_division()
    }

    /// Club-standing gate — see [`PipelineProcessor::loan_reputation_drop_ok`],
    /// which delegates here.
    fn clears_club_standing(&self) -> bool {
        if self.parent_rep == 0 {
            return true;
        }
        let very_raw = self.ability.saturating_add(25) <= self.parent_best_in_group;
        let floor = if very_raw && self.is_development {
            // A development youngster genuinely years off the shirt drops
            // without a reputation floor at all — the minutes gate is the
            // realism check for him.
            0.0
        } else if MarketSwitches::loan_guard_off() {
            if very_raw { 0.12 } else { 0.25 }
        } else {
            // Continuous in how ready he already is: the raw floor at one
            // end, a peer-level club at the other. A flat 0.25 made a club
            // a quarter of the parent's standing a credible home for the
            // parent's own first-choice.
            Self::RAW_STANDING_FLOOR + Self::READY_STANDING_SPAN * self.readiness()
        };
        self.borrower_rep as f32 >= self.parent_rep as f32 * floor
    }

    /// Division gate: is the borrower's **competition** a plausible home?
    ///
    /// The floor is continuous in how ready the player already is for his
    /// parent's own first team — measured, like every other loan gate here,
    /// against the parent's best at his position. A youngster far off that
    /// standard drops a long way to play; someone already competing for the
    /// shirt only moves sideways, because a club with a first-team-standard
    /// player plays him, keeps him as cover, or sells him — it does not park
    /// him a division below. A development loan widens the allowance, so the
    /// "drop a level and play every week" pathway keeps working for the
    /// players it is meant for.
    fn clears_division(&self) -> bool {
        // Unknown competition on either side — the club-standing gate owns
        // the decision rather than this one guessing.
        if self.parent_league_rep == 0 || self.borrower_league_rep == 0 {
            return true;
        }
        self.borrower_league_rep as f32 >= self.parent_league_rep as f32 * self.division_floor()
    }

    /// How ready this player already is for his parent club's own first
    /// team, 0..1 — measured against the parent's best at his position.
    /// Zero (unknown parent standard) reads as fully raw, which is what
    /// stands both floors down.
    fn readiness(&self) -> f32 {
        Self::readiness_of(self.ability, self.parent_best_in_group)
    }

    /// [`Self::readiness`] from the two bare numbers, for the gates that
    /// hold them without building a whole destination.
    fn readiness_of(ability: u8, parent_best_in_group: u8) -> f32 {
        if parent_best_in_group == 0 {
            return 0.0;
        }
        let ability_ratio = ability as f32 / parent_best_in_group as f32;
        ((ability_ratio - Self::RAW_ABILITY_RATIO)
            / (Self::READY_ABILITY_RATIO - Self::RAW_ABILITY_RATIO))
            .clamp(0.0, 1.0)
    }

    /// Share of the parent's league level this loan may drop to.
    fn division_floor(&self) -> f32 {
        if self.parent_best_in_group == 0 {
            return 0.0;
        }
        let readiness = self.readiness();
        let floor = Self::RAW_LEAGUE_FLOOR
            + (Self::READY_LEAGUE_FLOOR - Self::RAW_LEAGUE_FLOOR) * readiness;
        if MarketSwitches::loan_guard_off() {
            return if self.is_development {
                floor * (1.0 - Self::RAW_LEAGUE_ALLOWANCE)
            } else {
                floor
            };
        }
        // The allowance a drop earns for being a drop the player needs,
        // continuous in readiness rather than switched on by his birth
        // year: full width when he is raw, none of it when he is ready.
        floor * (1.0 - Self::RAW_LEAGUE_ALLOWANCE * (1.0 - readiness))
    }
}

struct UnsolicitedLoanTarget;

impl UnsolicitedLoanTarget {
    /// Upper age for a *development* unsolicited loan — a young player a
    /// smaller club takes to get him minutes. Above it, only a genuinely
    /// surplus player is a credible cold loan target (generic cover, not
    /// development).
    const DEVELOPMENT_AGE: u8 = UnsettledAbroadScan::DEVELOPMENT_AGE;

    /// Classify a potential unsolicited loan target. Returns `Some(is_dev)`
    /// when the player may be cold-approached — `is_dev` selecting the
    /// stricter development-minutes gate — or `None` when he is not a
    /// credible target at all.
    ///
    /// Never a first-team contributor (`CorePlayer` / `FirstTeamUseful`),
    /// and never a player the club hasn't even evaluated yet
    /// (`UnknownNeedsEvaluation`). Prospects and young rotation players go
    /// as development loans; genuine surplus goes at any age up to `max_age`.
    fn classify(
        player: &Player,
        age: u8,
        max_age: u8,
        asset_class: SquadAssetClass,
        parent_holds: bool,
    ) -> Option<bool> {
        if player.contract.is_none() || player.is_on_loan() {
            return None;
        }
        // The parent's own first choice is never cold-called, whatever
        // label the monthly rank pass happens to have stamped on him. The
        // asset class below is a good answer to "is he surplus?" and a
        // poor one to "is he ours?" — a nineteen-year-old starter reads
        // `ProspectDevelopment` off a birth year and walks straight
        // through it.
        if parent_holds {
            return None;
        }
        if age > max_age {
            return None;
        }
        // Manager-pinned players are off the table entirely.
        if player.is_force_match_selection {
            return None;
        }
        // Already on a list / heading out under his own steam: those flow
        // through the normal (listed) paths, not a cold loan approach.
        if player.statuses.has(PlayerStatusType::Lst)
            || player.statuses.has(PlayerStatusType::Loa)
            || player.statuses.has(PlayerStatusType::Frt)
        {
            return None;
        }
        match asset_class {
            SquadAssetClass::CorePlayer
            | SquadAssetClass::FirstTeamUseful
            | SquadAssetClass::UnknownNeedsEvaluation => None,
            SquadAssetClass::ProspectDevelopment => Some(true),
            SquadAssetClass::RotationUseful => (age <= Self::DEVELOPMENT_AGE).then_some(true),
            SquadAssetClass::TrueSurplus => Some(age <= Self::DEVELOPMENT_AGE),
        }
    }

    /// Does the target clear the "right level for this borrower" gate?
    ///
    /// For a development loan the realism check is "will he actually play
    /// here", which the caller enforces with the position-aware minutes /
    /// room gates (and, for keepers, the strict plausible-#1 rule). When
    /// that holds, the squad-average floor is actively harmful — it blocks
    /// the signature development move: a big-club youngster dropping to a
    /// smaller club to START. Young keepers are the sharpest case — they
    /// develop late, so a teenage keeper's CA sits far below an outfield-heavy
    /// squad average, which is exactly why none ever moved. So a development
    /// loan skips the squad-average floor. The destination-level floors are
    /// NOT skipped wholesale: [`LoanDestinationLevel`] is itself keyed to
    /// readiness, so a genuinely raw youngster still drops to a club where he
    /// STARTS, but a near-ready player (a displaced first-choice) is held to a
    /// peer-level club in a peer-level division rather than tumbling several
    /// tiers. Cover (non-development) loans keep every floor.
    fn clears_level_gate(borrower_avg_ability: u8, level: &LoanDestinationLevel) -> bool {
        let avg_ok =
            level.is_development || level.ability >= borrower_avg_ability.saturating_sub(5);
        avg_ok && level.is_plausible()
    }
}

/// The two seller-side numbers a cross-border loan has to carry with it.
///
/// Personal terms for a foreign loan are resolved by the BORROWING
/// country's pass, where the player and his club are inside another
/// country's borrow. What he is to that club, and how far the move falls,
/// are therefore read here — from the same staged plausibility model the
/// scan has already run for its own gates, so the two can never disagree
/// (Part VIII, "two importance formulas").
struct ForeignLoanStance;

impl ForeignLoanStance {
    /// `(importance, sporting_drop)`. Falls back to the mid-range read the
    /// foreign fee resolver already uses when the summary cannot be
    /// assessed at all.
    fn read(
        buyer_ctx: &BuyerPlausibilityContext,
        target: &PlayerSummary,
        date: NaiveDate,
    ) -> (f32, f32) {
        // A man his club has loan-listed is not a cold call. Reading every
        // foreign loan as unsolicited held a genuinely advertised target
        // to the stricter unsolicited bar and withheld the
        // seller-advertised read from his own importance.
        let is_unsolicited = !target.is_loan_listed;
        TransferPlausibilityBuilder::from_summary(
            buyer_ctx,
            target,
            true,
            is_unsolicited,
            date,
            None,
        )
        .map(|inputs| {
            (
                TransferPlausibilityEvaluator::player_importance(&inputs),
                TransferPlausibilityEvaluator::sporting_drop(&inputs),
            )
        })
        .unwrap_or((0.55, 0.0))
    }
}

/// Eligibility approximation for an *unsolicited* foreign loan. A cross-
/// country [`PlayerSummary`] doesn't carry the squad-asset classification
/// (it is built without the owning club's full squad context), so the "is
/// he a first-team contributor?" question is approximated from how far the
/// player sits below his club's best at his position: a key man is at/near
/// the top, a prospect or fringe player clearly below it. The staged
/// plausibility gate (run as unsolicited) still applies on top.
struct ForeignUnsolicitedLoanTarget;

impl ForeignUnsolicitedLoanTarget {
    /// Young players up to this age qualify as development targets on a
    /// small gap; older players need a clear surplus gap.
    const DEVELOPMENT_AGE: u8 = UnsettledAbroadScan::DEVELOPMENT_AGE;
    /// CA below his club's best at the position that marks a young player as
    /// a development prospect (not the first-choice).
    const PROSPECT_GAP: u8 = 5;
    /// Larger gap an older player must sit below his club's best to read as
    /// clearly-surplus fringe rather than a contributor.
    const FRINGE_GAP: u8 = 15;

    /// Does this foreign player look like one his club would entertain a
    /// loan out for — i.e. clearly not their first-choice at the position?
    fn looks_loanable(age: u8, skill_ability: u8, club_best_in_group: u8) -> bool {
        let gap = if age <= Self::DEVELOPMENT_AGE {
            Self::PROSPECT_GAP
        } else {
            Self::FRINGE_GAP
        };
        club_best_in_group >= skill_ability.saturating_add(gap)
    }

    /// Whether a development-grade (stricter) minutes gate should apply.
    fn is_development(age: u8) -> bool {
        age <= Self::DEVELOPMENT_AGE
    }
}

/// Whether a club is in the market for a loan at all, and on what terms.
///
/// The borrower-side scan and the seller-side broadcast have to agree about
/// this. They used not to: the scan asked reputation, philosophy, window and
/// squad shortage before it would even look, while the push asked the
/// borrower nothing — the comment there called accepting a pushed loan "a
/// passive response, not a planning action" and skipped every appetite gate.
/// So a Continental club that shops the loan market only in January, and only
/// while in the red, was handed teenagers in August by parents who had simply
/// picked the highest-reputation name that would play them.
struct LoanBorrowerAppetite {
    /// The club runs its own loan scans right now.
    scans: bool,
    /// Some position group is below the level at which the club can field a
    /// balanced matchday squad.
    critical_shortage: bool,
}

impl LoanBorrowerAppetite {
    /// Age slack a pushed candidate gets against a request's band — the same
    /// relaxation [`PipelineProcessor::scan_loan_market`] applies when it
    /// matches its own requests against the listed market.
    const REQUEST_AGE_SLACK: u8 = 3;
    /// Ability slack, likewise mirrored from the scan's `relaxed_min`.
    const REQUEST_ABILITY_SLACK: u8 = 5;
    /// CA over the borrower's own best in the group at which a pushed
    /// loanee stops being "somebody else's player" and becomes an
    /// upgrade any club takes, in any month.
    const UPGRADE_MARGIN: u8 = 5;

    fn assess(club: &Club, team: &Team, is_january: bool) -> Self {
        // Critical-need override: a club whose squad is genuinely short at
        // any position MUST scan the loan market, even outside its usual
        // scanning window. Without this, a National-rep club that loses both
        // senior GKs in October would otherwise wait until January to cover —
        // leaving them fielding youth keepers for two months.
        //
        // Threshold: < 2 at GK, < 4 at DEF/MID, < 2 at FWD. Below these and
        // the team genuinely cannot field a balanced matchday squad.
        let critical_shortage = {
            let mut counts = [0usize; PlayerFieldPositionGroup::COUNT];
            for p in team.players.iter() {
                counts[p.position().position_group().index()] += 1;
            }
            counts[0] < 2 || counts[1] < 4 || counts[2] < 4 || counts[3] < 2
        };

        // Philosophy overrides reputation defaults. LoanFocused clubs always
        // scan; SignToCompete clubs almost never loan.
        let scans = critical_shortage
            || match &club.philosophy {
                ClubPhilosophy::LoanFocused => true,
                ClubPhilosophy::SignToCompete => {
                    // Only loan as emergency cover in January
                    is_january && club.finance.balance.balance < 0
                }
                _ => match team.reputation.level() {
                    ReputationLevel::Regional
                    | ReputationLevel::Local
                    | ReputationLevel::Amateur => true,
                    ReputationLevel::National => is_january || club.finance.balance.balance < 0,
                    ReputationLevel::Continental => is_january && club.finance.balance.balance < 0,
                    ReputationLevel::Elite => false,
                },
            };

        LoanBorrowerAppetite {
            scans,
            critical_shortage,
        }
    }

    /// Will this club entertain a loan somebody else brings to it?
    ///
    /// Answering a knock at the door is more permissive than going out
    /// looking, so an open request at the position counts even for a club
    /// that runs no scans of its own. What it is not is unconditional: the
    /// candidate still has to be someone that request was asking for. A side
    /// shopping for a centre-forward who can lead its line has not thereby
    /// agreed to take any centre-forward alive, and a club with no request at
    /// the position has not asked for anybody at all.
    ///
    /// The one unconditional yes is an UPGRADE: a loanee clearly better
    /// than anything the club has in that group, whose wage it can carry
    /// and who is inside the guard's reach. A Continental club that
    /// "only loans in January, and only in the red" turning that down in
    /// August is not caution — it is the refusal that handed the boy to
    /// the tier below, every time.
    #[allow(clippy::too_many_arguments)]
    fn accepts_push(
        &self,
        club: &Club,
        group: PlayerFieldPositionGroup,
        candidate_ability: u8,
        candidate_age: u8,
        borrower_best_in_group: u8,
        guard_clears: bool,
    ) -> bool {
        if self.scans || self.critical_shortage {
            return true;
        }
        if guard_clears
            && borrower_best_in_group > 0
            && candidate_ability >= borrower_best_in_group.saturating_add(Self::UPGRADE_MARGIN)
        {
            return true;
        }
        club.transfer_plan
            .transfer_requests
            .iter()
            .filter(|r| {
                r.status != TransferRequestStatus::Fulfilled
                    && r.status != TransferRequestStatus::Abandoned
                    && !r.is_emergency_free_agent_depth()
                    && r.position.position_group() == group
            })
            .any(|r| {
                candidate_ability >= r.min_ability.saturating_sub(Self::REQUEST_ABILITY_SLACK)
                    && candidate_age >= r.preferred_age_min
                    && candidate_age <= r.preferred_age_max.saturating_add(Self::REQUEST_AGE_SLACK)
            })
    }
}

/// Snapshot of the borrowing club's position-group depth — per-group
/// ability lists plus squad caps. Used by every loan scan (domestic and
/// foreign) for two realism gates: `has_room_for` stops the borrower
/// piling a fourth mid-tier player onto a position that's already three
/// deep, and `would_get_minutes` rejects destinations where the player
/// would sit behind a wall of clearly better names — a development loan
/// must buy pitch time, not a bench seat.
struct BorrowerPositionDepth {
    /// Headcount view: who is FILED under each group. A man occupies one
    /// shirt in the squad register however many roles he can play, so the
    /// squad-bloat cap counts labels.
    rows: Vec<(PlayerFieldPositionGroup, usize, Vec<u8>)>,
    /// Competition view: what every squad member is worth in each group's
    /// roles, whatever he is filed under, discounted by how natural the role
    /// is to him.
    ///
    /// The minutes gate has to read this one. Asking the label who stood
    /// ahead of an incoming forward counted only the men whose record
    /// happened to lead with a forward position — so a club whose front
    /// line was three outstanding wide forwards filed as midfielders read as
    /// having nobody up front, and became the most attractive destination in
    /// the country for other clubs' teenage strikers precisely because its
    /// attack looked empty.
    role_rows: Vec<(PlayerFieldPositionGroup, Vec<u8>)>,
}

impl BorrowerPositionDepth {
    fn snapshot(team: &Team) -> Self {
        let rows = PlayerFieldPositionGroup::ALL
            .iter()
            .map(|&group| {
                let max = group.ideal_squad_depth();
                let abilities: Vec<u8> = team
                    .players
                    .iter()
                    .filter(|p| p.position().position_group() == group)
                    .map(|p| p.player_attributes.current_ability)
                    .collect();
                (group, max, abilities)
            })
            .collect();
        let role_rows = PlayerFieldPositionGroup::ALL
            .iter()
            .map(|&group| {
                let abilities: Vec<u8> = team
                    .players
                    .iter()
                    .filter_map(|p| {
                        let effective = RoleFamiliarity::best_in_group(
                            &p.positions,
                            p.player_attributes.current_ability,
                            group,
                        );
                        (effective > 0).then_some(effective)
                    })
                    .collect();
                (group, abilities)
            })
            .collect();
        BorrowerPositionDepth { rows, role_rows }
    }

    /// Fold in-flight incoming-loan targets (`(group, ability)`) into the
    /// per-group ability lists, so `has_room_for` / `would_get_loan_minutes`
    /// treat a loan being negotiated as if the player were already on the
    /// roster. Builder form keeps the bare `snapshot` constructor for the
    /// unit tests.
    fn with_pending_loans(mut self, pending: &[(PlayerFieldPositionGroup, u8)]) -> Self {
        for (group, ability) in pending {
            if let Some(row) = self.rows.iter_mut().find(|(g, _, _)| g == group) {
                row.2.push(*ability);
            }
            // A player still being negotiated for is known only by the group
            // he was matched on, so he joins the competition view there at
            // face value — the same conservative reading the headcount takes.
            if let Some(row) = self.role_rows.iter_mut().find(|(g, _)| g == group) {
                row.1.push(*ability);
            }
        }
        self
    }

    fn row(
        &self,
        group: PlayerFieldPositionGroup,
    ) -> Option<&(PlayerFieldPositionGroup, usize, Vec<u8>)> {
        self.rows.iter().find(|(g, _, _)| *g == group)
    }

    /// Competition view for `group` — see [`Self::role_rows`].
    fn role_row(
        &self,
        group: PlayerFieldPositionGroup,
    ) -> Option<&(PlayerFieldPositionGroup, Vec<u8>)> {
        self.role_rows.iter().find(|(g, _)| *g == group)
    }

    /// True when adding a loan player at `group` makes sense — either
    /// there's room (count < max) or the incoming player is clearly
    /// stronger than the existing best in that group.
    ///
    /// `development` relaxes the full-line rule for GOALKEEPERS only. A full
    /// keeper line normally blocks anything short of a clear +10 upgrade, but
    /// that traps a giant's keeper prospects: keepers develop late (so a
    /// prospect's CA is low) and every club already carries its cap of three,
    /// so a River Plate youth keeper is a plausible loanee almost nowhere. For
    /// a development keeper loan the line admits ONE over-cap loanee who is a
    /// genuine upgrade on the group's WEAKEST keeper (the fringe keeper he
    /// displaces); `would_get_loan_minutes` then guarantees he competes for
    /// the shirt, which is the whole point. Bounded to one: once a keeper loan
    /// is already inbound the pending-loan fold pushes the count past the cap
    /// and the strict bar returns, so this never re-opens the loan-in
    /// over-accumulation the cap exists to prevent. Cover loans and every
    /// outfield group keep the strict clear-upgrade bar.
    fn has_room_for(
        &self,
        group: PlayerFieldPositionGroup,
        candidate_ability: u8,
        development: bool,
    ) -> bool {
        match self.row(group) {
            Some((_, max, abilities)) => {
                if abilities.len() < *max {
                    return true;
                }
                let best = abilities.iter().copied().max().unwrap_or(0);
                if development
                    && group == PlayerFieldPositionGroup::Goalkeeper
                    && abilities.len() == *max
                {
                    let worst = abilities.iter().copied().min().unwrap_or(0);
                    return candidate_ability >= worst.saturating_add(8);
                }
                // Group is full — only accept if the incoming player
                // would clearly upgrade the position (≥10 CA over the
                // current best).
                candidate_ability >= best.saturating_add(10)
            }
            None => true,
        }
    }

    /// CA over the borrower's best in the group at which a "minutes" loan
    /// stops being one, at full readiness …
    const OVERQUALIFIED_GAP_READY: f32 = 25.0;
    /// … plus the extra a raw youngster is allowed, because his whole
    /// pathway is dropping below his own level to play.
    const OVERQUALIFIED_GAP_RAW_EXTRA: f32 = 15.0;

    /// Minutes gate with a development-strictness switch. Development
    /// loans exist to buy PLAYING time, so a young development loanee
    /// tolerates at most ONE clearly better outfielder ahead of him;
    /// generic squad/emergency cover keeps the looser bar of two. GK
    /// loans must always arrive as plausible first choice.
    ///
    /// The gate had no UPPER bound: it asked only whether the player would
    /// play here, and a loanee forty points better than the borrower's
    /// best is the strongest possible yes. That is not a minutes loan, it
    /// is a mismatch — the borrower cannot coach him, cannot pay him and
    /// is not the level he needs — so the same reading now closes from
    /// both sides. The bound widens as the loanee gets rawer, because
    /// dropping below his own level IS a raw player's pathway;
    /// `parent_best_in_group` of 0 means the parent's standard is unknown,
    /// which stands the upper bound down rather than guessing.
    fn would_get_loan_minutes(
        &self,
        group: PlayerFieldPositionGroup,
        candidate_ability: u8,
        development: bool,
        parent_best_in_group: u8,
    ) -> bool {
        match self.role_row(group) {
            Some((_, abilities)) => {
                let clearly_better = abilities
                    .iter()
                    .filter(|&&a| a >= candidate_ability.saturating_add(8))
                    .count();
                let plays = match group {
                    PlayerFieldPositionGroup::Goalkeeper => clearly_better == 0,
                    _ => clearly_better < if development { 2 } else { 3 },
                };
                plays && self.is_not_overqualified(group, candidate_ability, parent_best_in_group)
            }
            None => true,
        }
    }

    /// The upper half of the minutes gate — see
    /// [`Self::would_get_loan_minutes`].
    fn is_not_overqualified(
        &self,
        group: PlayerFieldPositionGroup,
        candidate_ability: u8,
        parent_best_in_group: u8,
    ) -> bool {
        if parent_best_in_group == 0 || MarketSwitches::loan_guard_off() {
            return true;
        }
        let best_here = self
            .role_row(group)
            .and_then(|(_, abilities)| abilities.iter().copied().max())
            .unwrap_or(0);
        if best_here == 0 {
            return true;
        }
        let readiness = LoanDestinationLevel::readiness_of(candidate_ability, parent_best_in_group);
        let allowed =
            Self::OVERQUALIFIED_GAP_READY + Self::OVERQUALIFIED_GAP_RAW_EXTRA * (1.0 - readiness);
        (candidate_ability as f32 - best_here as f32) <= allowed
    }

    /// How clearly the candidate would be first choice here, 0..1 — the same
    /// competition view [`Self::would_get_loan_minutes`] gates on, read as a
    /// margin instead of a verdict.
    ///
    /// The gate can only answer "he would play"; a parent choosing between two
    /// clubs that both clear it wants to know *how well*. 1.0 is nobody in the
    /// building within eight CA of him, and the figure falls off continuously
    /// as better players stack up in front — so "walks straight into the side"
    /// outranks "scrapes ahead of the incumbent" without either being barred.
    fn minutes_headroom(&self, group: PlayerFieldPositionGroup, candidate_ability: u8) -> f32 {
        match self.role_row(group) {
            Some((_, abilities)) => {
                let ahead = abilities
                    .iter()
                    .filter(|&&a| a >= candidate_ability.saturating_add(8))
                    .count() as f32;
                let comparable = abilities
                    .iter()
                    .filter(|&&a| {
                        a < candidate_ability.saturating_add(8)
                            && a.saturating_add(8) > candidate_ability
                    })
                    .count() as f32;
                // A man clearly better costs a full place in the queue; a peer
                // costs a third of one, because he is competition rather than a
                // wall.
                (1.0 / (1.0 + ahead + comparable * 0.33)).clamp(0.0, 1.0)
            }
            None => 1.0,
        }
    }

    /// Headcount filed under `group` — the squad-register view, for scarcity
    /// pressure.
    fn headcount(&self, group: PlayerFieldPositionGroup) -> usize {
        self.row(group).map(|(_, _, a)| a.len()).unwrap_or(0)
    }

    /// Best ability the borrower can already field in this group — the
    /// competition view, so a wide forward filed as a midfielder counts
    /// where he actually plays.
    fn best_in_group(&self, group: PlayerFieldPositionGroup) -> u8 {
        self.role_row(group)
            .and_then(|(_, abilities)| abilities.iter().copied().max())
            .unwrap_or(0)
    }
}
