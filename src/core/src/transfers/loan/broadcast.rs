//! The seller's side of the market: a club that has listed somebody and had
//! no answer goes looking for one.
//!
//! Two products, the same four passes each. **Read** who is broadcastable and
//! prune the entries whose player has moved on; **advance** each broadcast one
//! tier wider when the current tier has gone unanswered past its response
//! window; **pick** a counterparty at that tier; **open** the negotiation.
//!
//! The pass split is what the `── Pass N ──` banners inside the two old bodies
//! (590 and 399 lines) already described. Each pass has a different borrow —
//! read the country, mutate the clubs, read again, mutate the market — which is
//! why they were sequential regions of one function rather than anything
//! shareable.

use crate::transfers::loan::LoanPipeline;
use crate::transfers::market::window::MarketCadence;
use crate::transfers::squad::bands::TierBands;
use crate::transfers::view::club::ClubView;
use crate::transfers::view::player::PlayerView;
use chrono::{Duration, NaiveDate};
use log::debug;

use crate::club::player::behaviour_config::HappinessConfig;
use crate::club::player::transfer::MarketResignation;
use crate::club::staff::perception::PotentialEstimator;
use crate::shared::{Currency, CurrencyValue};
use crate::transfers::ScoutingRegion;
use crate::transfers::deal::negotiation::NegotiationStatus;
use crate::transfers::deal::offer::{PersonalTermsOffer, TransferClause, TransferOffer};
use crate::transfers::deal::reason::TransferReason;
use crate::transfers::gate::fit::{SquadFitSnapshot, SquadRegistrationLimits};
use crate::transfers::loan::interest::{DestinationAppeal, InterestDraw, LoanApproachMemory};
use crate::transfers::market::{TransferListingOrigin, TransferListingStatus, TransferListingType};
use crate::transfers::pipeline::trace::MarketSwitches;
use crate::transfers::pipeline::{AvailabilityBroadcast, LoanDestinationPreference};
use crate::transfers::squad::minutes::LoanPromise;
use crate::utils::FormattingUtils;
use crate::{
    Country, HappinessEventCause, HappinessEventContext, HappinessEventScope,
    HappinessEventSeverity, HappinessEventType, Person, PlayerFieldPositionGroup, ReputationLevel,
    RoleFamiliarity,
};
use std::collections::{HashMap, HashSet};

use super::*;

/// The parent's own side of a broadcast: how wide it has widened, and where it
/// has already placed people.
struct LoanPushParent<'a> {
    restrict_tier: Option<ReputationLevel>,
    placements: &'a [(u32, NaiveDate)],
}

/// What every borrower read shares: the calendar, the geography, and who is
/// already carrying a pursuit.
struct LoanPushMarket<'a> {
    is_january: bool,
    domestic_region: ScoutingRegion,
    pending_loans: &'a HashMap<u32, Vec<(PlayerFieldPositionGroup, u8)>>,
}

/// The seller-side broadcast pass.
pub(in crate::transfers::loan) struct ListingBroadcast;

/// A player his parent club has loan-listed and is shopping around.
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

/// A player whose transfer listing has gone stale.
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

/// A loan a parent club is offering around, and to whom.
struct LoanPushAction {
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
impl LoanOptionContext for LoanPushAction {
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

/// A stale listing a club is offering around, and to whom.
struct TransferPushAction {
    buyer_id: u32,
    player_id: u32,
    selling_club_id: u32,
    offer_amount: f64,
}

impl ListingBroadcast {
    /// A parent club that loan-listed somebody and got no answer goes looking.
    pub(in crate::transfers::loan) fn loans(country: &mut Country, date: NaiveDate) {
        let _is_january = MarketCadence::is_mid_season_window_for(&country.code, date);
        // The region this country plays in — every borrower in this pass
        // is in it, so the home term is derived once.
        let _domestic_region = ScoutingRegion::from_country(country.continent_id, &country.code);

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
        let pending_loans = LoanPipeline::pending_incoming_loans_by_club(country);

        let Some((broadcastable, _keep)) = Self::loan_read(country, date, &in_negotiation) else {
            return;
        };
        Self::loan_tiers(country, date, &broadcastable, &in_negotiation);
        let actions = Self::loan_pick(
            country,
            date,
            &broadcastable,
            &pending_loans,
            &in_negotiation,
        );
        Self::loan_open(country, date, actions);
    }

    /// Pass 1 (read) — who is broadcastable, and prune the entries whose player
    /// has moved on.
    fn loan_read(
        country: &mut Country,
        date: NaiveDate,
        _in_negotiation: &HashSet<u32>,
    ) -> Option<(Vec<Broadcastable>, Vec<u32>)> {
        // ── Pass 1 (read): broadcastable players ────────────────────────
        // A player is broadcastable when he carries an Available loan
        // listing (the same source the borrower scan reads — which also
        // guarantees `start_negotiation` has a listing to anchor on) AND
        // his parent club is resource-rich enough to run a push (National+).

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
            let Some(player) = PlayerView::find_player_in_country(country, listing.player_id)
            else {
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
            let guard = LoanPipeline::loan_guard_for(country, parent_club, player, date);
            let is_development =
                LoanPipeline::is_development_loan(guard.as_ref(), player.age(date))
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
                parent_league_rep: LoanPipeline::club_league_reputation(country, parent_club),
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
        LoanPipeline::prune_loan_broadcasts(country, &keep_ids);
        if broadcastable.is_empty() {
            return None;
        }

        Some((broadcastable, keep_ids))
    }

    /// Pass 2 (mut clubs) — widen each broadcast one tier when its current
    /// tier has gone unanswered past the response window.
    fn loan_tiers(
        country: &mut Country,
        date: NaiveDate,
        broadcastable: &[Broadcastable],
        in_negotiation: &HashSet<u32>,
    ) {
        // ── Pass 2 (mut clubs): advance each broadcast's tier ───────────
        // Open a broadcast at the parent's own tier; widen one tier down
        // once the current tier has gone unanswered past the response
        // window. In-negotiation players are left frozen.
        for b in broadcastable {
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
                    if (date - prev.since).num_days() >= LoanPipeline::BROADCAST_RESPONSE_DAYS {
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
    }

    /// Pass 3 (read) — pick a borrower at each broadcast's tier.
    fn loan_pick(
        country: &Country,
        date: NaiveDate,
        broadcastable: &[Broadcastable],
        pending_loans: &HashMap<u32, Vec<(PlayerFieldPositionGroup, u8)>>,
        in_negotiation: &HashSet<u32>,
    ) -> Vec<LoanPushAction> {
        let is_january = MarketCadence::is_mid_season_window_for(&country.code, date);
        let domestic_region = ScoutingRegion::from_country(country.continent_id, &country.code);
        let mut actions: Vec<LoanPushAction> = Vec::new();
        // One borrower must not be handed two same-group loans in a single
        // broadcast tick: per-player `has_active_negotiation_for` and the
        // registered-loan snapshot both miss it, because broadcast actions
        // aren't opened as negotiations until Pass 4.
        let mut claimed_loans: HashSet<(u32, PlayerFieldPositionGroup)> = HashSet::new();
        for b in broadcastable {
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
                if posted_for < LoanPipeline::HOME_FIRST_DAYS {
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
            let destinations = Self::loan_destinations(
                country,
                b,
                date,
                &claimed_loans,
                &LoanPushMarket {
                    is_january,
                    domestic_region,
                    pending_loans,
                },
                &LoanPushParent {
                    restrict_tier,
                    placements: parent_placements,
                },
            );

            if let Some(borrower_id) = InterestDraw::pick(&destinations) {
                actions.push(LoanPushAction {
                    borrower_id,
                    player_id: b.player_id,
                    is_development: b.is_development,
                    selling_club_id: b.parent_club_id,
                    offer_amount: FormattingUtils::round_fee(b.asking * 0.8),
                });
                claimed_loans.insert((borrower_id, b.group));
            }
        }

        actions
    }

    /// Who, at this broadcast's current tier, would take him. Weighed by
    /// [`DestinationAppeal`] rather than reputation alone: a prospect is placed
    /// where he will play and be coached, and no borrower should become a farm
    /// team.
    fn loan_destinations(
        country: &Country,
        b: &Broadcastable,
        date: NaiveDate,
        claimed_loans: &HashSet<(u32, PlayerFieldPositionGroup)>,
        market: &LoanPushMarket<'_>,
        parent: &LoanPushParent<'_>,
    ) -> Vec<(u32, f32)> {
        let restrict_tier = parent.restrict_tier;
        let parent_placements = parent.placements;
        let is_january = market.is_january;
        let domestic_region = market.domestic_region;
        let pending_loans = market.pending_loans;

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
            let borrower_league_rep = LoanPipeline::club_league_reputation(country, club);
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
            if !LoanPipeline::loan_guard_allows(
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

        destinations
    }

    /// Pass 4 (mut market) — open the loan negotiations.
    fn loan_open(country: &mut Country, date: NaiveDate, actions: Vec<LoanPushAction>) {
        // ── Pass 4 (mut market): open the loan negotiations ─────────────
        // The interested club's "response". The player already carries an
        // Available loan listing, so `start_negotiation` has its anchor.
        for action in actions {
            let selling_rep = ClubView::get_club_reputation(country, action.selling_club_id);
            let buying_rep = ClubView::get_club_reputation(country, action.borrower_id);
            let (p_age, p_ambition) =
                PlayerView::get_player_negotiation_data(country, action.player_id, date);

            let mut clauses = Vec::new();
            if let Some(option_fee) = LoanPipeline::loan_option_fee(country, &action, date) {
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
                loan_duration_months: Some(LoanPipeline::loan_duration_to_season_end(
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
                let (p_name, sc_name) = PlayerView::resolve_player_and_club_name(
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

    /// A club whose transfer listing has gone stale offers the player around.
    pub(in crate::transfers::loan) fn transfers(country: &mut Country, date: NaiveDate) {
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

        let Some(sellable) = Self::transfer_read(country, date, &in_negotiation) else {
            return;
        };
        Self::transfer_tiers(country, date, &sellable, &in_negotiation);
        let actions = Self::transfer_pick(country, date, &sellable, &in_negotiation);
        Self::transfer_open(country, date, actions);
    }

    /// Pass 1 (read) — stale genuine seller listings.
    fn transfer_read(
        country: &mut Country,
        date: NaiveDate,
        _in_negotiation: &HashSet<u32>,
    ) -> Option<Vec<Sellable>> {
        // ── Pass 1 (read): stale genuine seller listings ────────────────
        // Synthetic / unsolicited listings never enter the push — only a
        // listing the club actually advertised represents a player it
        // wants moved.

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
            if (date - listing.listed_date).num_days() < LoanPipeline::TRANSFER_BROADCAST_GRACE_DAYS
            {
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
            let Some(player) = PlayerView::find_player_in_country(country, listing.player_id)
            else {
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

        LoanPipeline::prune_transfer_broadcasts(country, &keep_ids);
        if sellable.is_empty() {
            return None;
        }

        Some(sellable)
    }

    /// Passes 2 and 2b — open or widen each broadcast's tier, then put a
    /// first-time entry on the player's own feed as the ask it is.
    fn transfer_tiers(
        country: &mut Country,
        date: NaiveDate,
        sellable: &[Sellable],
        in_negotiation: &HashSet<u32>,
    ) {
        // ── Pass 2 (mut clubs): open / advance each broadcast's tier ────
        // A first-time entry is the player's ask made real — remember him
        // so the player-mut pass below can put the request on his feed.
        let mut newly_asking: Vec<u32> = Vec::new();
        for s in sellable {
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
            let opened =
                s.listed_date + Duration::days(LoanPipeline::TRANSFER_BROADCAST_GRACE_DAYS);
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
            let steps_down = ((date - first_since).num_days().max(0)
                / LoanPipeline::BROADCAST_RESPONSE_DAYS) as u32;
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
    }

    /// Pass 3 (read) — pick a buyer at each broadcast's tier.
    fn transfer_pick(
        country: &Country,
        date: NaiveDate,
        sellable: &[Sellable],
        in_negotiation: &HashSet<u32>,
    ) -> Vec<TransferPushAction> {
        let mut actions: Vec<TransferPushAction> = Vec::new();
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
        for s in sellable {
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
                - LoanPipeline::TRANSFER_BROADCAST_GRACE_DAYS) as f32
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
                let floor = TierBands::tier_starter_ca_score(rep_score, s.group).saturating_sub(20);
                let ceiling = TierBands::tier_target_ceiling_score(rep_score, s.group)
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
                actions.push(TransferPushAction {
                    buyer_id,
                    player_id: s.player_id,
                    selling_club_id: s.parent_club_id,
                    offer_amount,
                });
                claimed.insert((buyer_id, s.group));
            }
        }

        actions
    }

    /// Pass 4 (mut market) — open the transfer negotiations.
    fn transfer_open(country: &mut Country, date: NaiveDate, actions: Vec<TransferPushAction>) {
        // ── Pass 4 (mut market): open the purchase negotiations ─────────
        // The interested club's "response". The player carries an
        // Available seller listing, so `start_negotiation` has its anchor
        // and the ordinary seller-acceptance path resolves the deal.
        for action in actions {
            let selling_rep = ClubView::get_club_reputation(country, action.selling_club_id);
            let buying_rep = ClubView::get_club_reputation(country, action.buyer_id);
            let (p_age, p_ambition) =
                PlayerView::get_player_negotiation_data(country, action.player_id, date);

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
                let (p_name, sc_name) = PlayerView::resolve_player_and_club_name(
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
}
