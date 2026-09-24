//! The loan market, end to end.
//!
//! The scan and the seller-side broadcast live in this file. [`guard`]
//! prices the destination against the asset, [`home`] is the pull back to
//! a player's own country, and [`interest`] is what a borrower actually
//! wants.

pub mod agreement;
mod broadcast;
mod foreign;
pub mod guard;
pub mod home;
pub mod interest;
pub mod legacy;
mod scan;
#[cfg(test)]
mod tests;

pub use agreement::*;
pub use guard::*;
pub use home::*;

use crate::club::staff::perception::AbilityEstimator;
use crate::transfers::view::player::PlayerView;
use chrono::{Datelike, NaiveDate, Weekday};

use broadcast::ListingBroadcast;
use foreign::ForeignLoanScan;
use scan::LoanMarketScan;

use crate::club::team::squad::SquadAssetClass;
use crate::shared::{Currency, CurrencyValue};
use crate::transfers::MarketMap;
use crate::transfers::deal::negotiation::NegotiationStatus;
use crate::transfers::gate::build::{BuyerPlausibilityContext, TransferPlausibilityBuilder};
use crate::transfers::gate::{EffectivePlayerReputation, TransferPlausibilityEvaluator};
use crate::transfers::loan::legacy::LegacyLoanGuard;
use crate::transfers::market::knowledge::PlacementReachIndex;
use crate::transfers::market::{TransferListing, TransferListingType};
use crate::transfers::pipeline::processor::PlayerSummary;
use crate::transfers::pipeline::trace::MarketSwitches;
use crate::transfers::pipeline::{LoanDestinationPreference, LoanOutStatus};
use crate::transfers::squad::LevelBand;
use crate::transfers::value::PlayerValuationCalculator;
use crate::utils::FormattingUtils;
use crate::{
    Club, Country, Person, Player, PlayerFieldPositionGroup, PlayerStatusType, RoleFamiliarity,
    Team,
};
use rustc_hash::FxHashMap;
use std::collections::HashMap;

#[cfg(test)]
use crate::HappinessEventType;
#[cfg(test)]
use crate::transfers::market::TransferListingOrigin;

// Loans fund short-term development or rotation minutes. Players older
// than this are signed cheap permanent (or as free agents) rather than
// loaned, so loan targeting above it is noise regardless of whether the
// move is request-driven or opportunistic.
/// [`crate::transfers::MarketAffinity::loan_affinity`] a foreign loan target
/// must clear for a club to even consider him. Low, and deliberately below
/// the permanent-move floor: a loan is a cheaper, more speculative piece of
/// business than a purchase, and clubs take flyers on loanees from markets
/// they would not buy in.
///
/// It stays a FLOOR rather than a bar the implausible routes fall under,
/// because there is no bar that separates them: Italy → Romania, the weakest
/// route real loans use, derives below Russia → Japan. What separates those
/// two is the draw, which weights every surviving candidate by this same
/// number — so a thin route is rare rather than forbidden.
const FOREIGN_LOAN_VISIBILITY_FLOOR: f32 = 0.04;

const MAX_LOAN_TARGET_AGE: u8 = 34;

/// The three fields [`LoanPipeline::loan_option_fee`] needs, so the
/// separate per-path action structs (domestic scan, seller broadcast,
/// foreign scan) can share one option-pricing rule instead of each
/// growing its own copy.
trait LoanOptionContext {
    fn player_id(&self) -> u32;
    fn selling_club_id(&self) -> u32;
    fn is_unsolicited(&self) -> bool;
}

/// The loan market: who a club will lend out, who will take him, and on what terms.
pub struct LoanPipeline;

impl LoanPipeline {
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
                PlayerView::find_player_in_country(country, negotiation.player_id)
            {
                map.entry(negotiation.buying_club_id).or_default().push((
                    player.position().position_group(),
                    player.player_attributes.current_ability,
                ));
            }
        }
        map
    }

    /// Incoming loan approaches in flight that would each take one of the
    /// borrower's foreign registration slots, per borrowing club.
    ///
    /// The quota is a SQUAD fact and the squad does not change until the
    /// deal executes, so without this a club with one slot free opens four
    /// approaches on the same slot in one window. Same shape as
    /// [`Self::pending_incoming_loans_by_club`], which does the identical
    /// job for position depth.
    fn pending_foreign_registrations_by_club(country: &Country) -> FxHashMap<u32, u32> {
        let mut map: FxHashMap<u32, u32> = FxHashMap::default();
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
            // Stamped first for the same reason the depth count stamps: a
            // cross-border target cannot be resolved by the in-country walk.
            let passport = negotiation.loan_target_country.or_else(|| {
                PlayerView::find_player_in_country(country, negotiation.player_id)
                    .map(|player| player.country_id)
            });
            if matches!(passport, Some(id) if id != 0 && id != country.id) {
                *map.entry(negotiation.buying_club_id).or_insert(0) += 1;
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
    /// widens to the next tier down. A week: the cascade is how a
    /// parent finds a taker, and a fortnight per rung spent most of a
    /// window walking down the tiers.
    const BROADCAST_RESPONSE_DAYS: i64 = 7;

    /// The days a parent's staff actually work the phones. Twice a week
    /// rather than once — a placement that takes four rungs to find its
    /// club had a month of window to do it in.
    fn is_market_day(date: NaiveDate) -> bool {
        matches!(date.weekday(), Weekday::Mon | Weekday::Thu)
    }

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
        if !Self::is_market_day(date) {
            return;
        }
        ListingBroadcast::loans(country, date);
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
        if date.weekday() != Weekday::Mon {
            return;
        }
        ListingBroadcast::transfers(country, date);
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
        placement_reach: &PlacementReachIndex,
    ) {
        ForeignLoanScan::run(country, foreign_players, date, market_map, placement_reach);
    }

    /// Fraction of a player's market value an option to buy is struck at.
    /// Below his value because the borrowing club is taking the risk and
    /// paying the wages for a season; matching the permanent path's own
    /// option pricing.
    const LOAN_OPTION_VALUE_FRACTION: f64 = 0.7;
    /// Age above which a loan is squad-clearing rather than an investment,
    /// so no option is written. A club will still buy a thirty-two-year-old
    /// it has had for a season; it will not buy one it has not.
    const LOAN_OPTION_MAX_AGE: u8 = 32;

    /// Strike price for an option to buy on this loan, or `None` when the
    /// deal isn't one an option belongs on.
    ///
    /// Cold, unsolicited approaches carry one only when the parent has
    /// already decided to move him on. It never advertised him, so it is
    /// not handing a purchase right to a club it was talked into dealing
    /// with — unless its own pathway says he is a fee, in which case an
    /// option is exactly what it wants written in. Everything the parent
    /// did advertise carries one as a matter of course.
    fn loan_option_fee<A>(country: &Country, action: &A, date: NaiveDate) -> Option<f64>
    where
        A: LoanOptionContext,
    {
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
        if action.is_unsolicited() && !player.pathway_stage().is_terminal() {
            return None;
        }
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
            let Some(player) = PlayerView::find_player_in_club(club, player_id) else {
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
                if let Some(player) = team.players.players.iter_mut().find(|p| p.id == player_id)
                    && !player.statuses.has(PlayerStatusType::Loa)
                {
                    player.statuses.add(date, PlayerStatusType::Loa);
                }
            }
        }

        true
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
        // The observable read is the expensive part and does not depend on
        // the group, so it is taken once per man rather than once per row.
        let observed: Vec<(&Player, PlayerFieldPositionGroup, u8)> = team
            .players
            .iter()
            .map(|p| {
                (
                    p,
                    p.position().position_group(),
                    AbilityEstimator::observable_level(p),
                )
            })
            .collect();
        let rows = PlayerFieldPositionGroup::ALL
            .iter()
            .map(|&group| {
                let max = group.ideal_squad_depth();
                let abilities: Vec<u8> = observed
                    .iter()
                    .filter(|(_, filed, _)| *filed == group)
                    .map(|(_, _, level)| *level)
                    .collect();
                (group, max, abilities)
            })
            .collect();
        let role_rows = PlayerFieldPositionGroup::ALL
            .iter()
            .map(|&group| {
                let abilities: Vec<u8> = observed
                    .iter()
                    .filter_map(|(p, _, level)| {
                        let effective = RoleFamiliarity::best_in_group(&p.positions, *level, group);
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
        let readiness = LevelBand::readiness_of(candidate_ability, parent_best_in_group);
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
    /// Men in this group clearly better than the candidate — the same
    /// competition view the minutes gate reads, as a count rather than
    /// a verdict.
    fn clearly_better_ahead(
        &self,
        group: PlayerFieldPositionGroup,
        candidate_ability: u8,
    ) -> usize {
        self.role_row(group)
            .map(|(_, abilities)| {
                abilities
                    .iter()
                    .filter(|&&a| {
                        a >= candidate_ability.saturating_add(BorrowerAppetite::CLEARLY_BETTER)
                    })
                    .count()
            })
            .unwrap_or(0)
    }

    fn best_in_group(&self, group: PlayerFieldPositionGroup) -> u8 {
        self.role_row(group)
            .and_then(|(_, abilities)| abilities.iter().copied().max())
            .unwrap_or(0)
    }
}
