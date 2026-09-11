//! End-to-end tests for the loan-market scan: a parent club, a borrowing
//! club, and one country wrapping both.
//!
//! Worlds come from [`crate::transfers::tests::kit`]; what stays here is what
//! this module is about — a keeper who is fully fit and whose skills match
//! his ability, because the scan reads both.

use super::super::*;
use crate::shared::{Currency, CurrencyValue};
use crate::transfers::loan::LoanPipeline;
use crate::transfers::market::{TransferListing, TransferListingOrigin, TransferListingType};
use crate::transfers::tests::kit::{TestClub, TestCountry, TestPlayer, TestTeam};
use crate::transfers::{LoanOutCandidate, LoanOutReason, LoanOutStatus};
use crate::{Club, Country, Player, PlayerPositionType, PlayerSquadStatus, Team, TeamType};
use chrono::{Datelike, Duration, NaiveDate, Weekday};

/// Fixtures for the scan, on a unit struct per the project's
/// no-free-helpers convention.
struct Fx;

impl Fx {
    /// A Monday inside a window-agnostic part of the calendar — the
    /// unsolicited pool only builds on Mondays.
    fn monday() -> NaiveDate {
        let d = NaiveDate::from_ymd_opt(2026, 1, 5).unwrap();
        assert_eq!(d.weekday(), Weekday::Mon, "fixture date must be a Monday");
        d
    }

    fn keeper(id: u32, ca: u8, pa: u8, age: u8, youth: bool) -> Player {
        let expiration = NaiveDate::from_ymd_opt(2030, 6, 30).unwrap();
        let player = TestPlayer::new(id)
            .position(PlayerPositionType::Goalkeeper)
            .position_level(18)
            .ability(ca)
            .potential(pa)
            .age(age)
            .on(NaiveDate::from_ymd_opt(2026, 1, 1).unwrap())
            .skills_match_ability()
            .fit()
            .squad_status(PlayerSquadStatus::NotYetSet);
        if youth {
            player.youth_contract_until(10_000, expiration).build()
        } else {
            player.contract_until(20_000, expiration).build()
        }
    }

    fn team(id: u32, club_id: u32, tt: TeamType, world: u16, players: Vec<Player>) -> Team {
        TestTeam::new(id)
            .club_id(club_id)
            .name(&format!("t{id}"))
            .team_type(tt)
            .reputation(world)
            .players(players)
            .build()
    }

    fn club(id: u32, teams: Vec<Team>, balance: i64) -> Club {
        TestClub::new(id)
            .name(&format!("Club{id}"))
            .balance(balance)
            .teams(teams)
            .build()
    }

    fn country(clubs: Vec<Club>) -> Country {
        TestCountry::new(1)
            .code("EN")
            .league_reputation(500)
            .clubs(clubs)
            .build()
    }

    /// Put an Available loan listing on the market for `player_id`,
    /// advertised free (a development loan). This is what the seller-
    /// side broadcast reads — the equivalent of the parent club having
    /// loan-listed the player.
    fn loan_list(country: &mut Country, player_id: u32, club_id: u32, team_id: u32) {
        country
            .transfer_market
            .add_listing(TransferListing::new_with_origin(
                player_id,
                club_id,
                team_id,
                CurrencyValue {
                    amount: 0.0,
                    currency: Currency::Usd,
                },
                Self::monday(),
                TransferListingType::Loan,
                TransferListingOrigin::SellerListed,
            ));
    }
}

/// The headline case: an Elite club's young, unlisted reserve keeper is
/// approached on loan by a Regional club that has a keeper vacancy. This
/// is exactly "no club has interest in my U18/U20 keeper" — and the only
/// way interest registers is an actual negotiation, so the scan must
/// create one.
#[test]
fn regional_club_makes_unsolicited_loan_approach_for_elite_youth_keeper() {
    let date = Fx::monday();

    // Elite parent (world 9000): three senior keepers on the main roster
    // plus one young, unlisted keeper in the reserves.
    let parent_main = Fx::team(
        10,
        1,
        TeamType::Main,
        9000,
        vec![
            Fx::keeper(101, 120, 120, 28, false),
            Fx::keeper(102, 118, 118, 26, false),
            Fx::keeper(103, 115, 115, 30, false),
        ],
    );
    let parent_reserve = Fx::team(
        11,
        1,
        TeamType::Reserve,
        6000,
        vec![Fx::keeper(200, 70, 150, 18, true)],
    );
    let parent = Fx::club(1, vec![parent_main, parent_reserve], 50_000_000);

    // Regional borrower (world 4000) with two weak keepers and budget.
    let borrower_main = Fx::team(
        20,
        2,
        TeamType::Main,
        4000,
        vec![
            Fx::keeper(301, 55, 55, 27, false),
            Fx::keeper(302, 50, 50, 29, false),
        ],
    );
    let mut borrower = Fx::club(2, vec![borrower_main], 5_000_000);
    borrower.transfer_plan.initialized = true;

    let mut country = Fx::country(vec![parent, borrower]);

    LoanPipeline::scan_loan_market(&mut country, date);

    assert!(
        country.transfer_market.has_active_negotiation_for(200, 2),
        "a Regional club should make an unsolicited loan approach for the Elite club's \
         young reserve keeper — this is the interest that was never registering"
    );
}

/// The production-realistic case: the borrowing club is broke. The old
/// `value * 0.10` asking made the loan fee exceed a poor club's tiny
/// `max_loan_fee`, silently filtering the prospect out. A development
/// loan now goes out free, so a cash-strapped club can still take him.
#[test]
fn cash_strapped_borrower_still_approaches_on_a_free_development_loan() {
    let date = Fx::monday();

    let parent_main = Fx::team(
        10,
        1,
        TeamType::Main,
        9000,
        vec![
            Fx::keeper(101, 120, 120, 28, false),
            Fx::keeper(102, 118, 118, 26, false),
            Fx::keeper(103, 115, 115, 30, false),
        ],
    );
    let parent_reserve = Fx::team(
        11,
        1,
        TeamType::Reserve,
        6000,
        vec![Fx::keeper(200, 70, 150, 18, true)],
    );
    let parent = Fx::club(1, vec![parent_main, parent_reserve], 50_000_000);

    // Negative balance → `max_loan_fee` is just 50k. A value-based fee
    // would have blocked the prospect; a free development loan must not.
    let borrower_main = Fx::team(
        20,
        2,
        TeamType::Main,
        4000,
        vec![
            Fx::keeper(301, 55, 55, 27, false),
            Fx::keeper(302, 50, 50, 29, false),
        ],
    );
    let mut borrower = Fx::club(2, vec![borrower_main], -2_000_000);
    borrower.transfer_plan.initialized = true;

    let mut country = Fx::country(vec![parent, borrower]);

    LoanPipeline::scan_loan_market(&mut country, date);

    assert!(
        country.transfer_market.has_active_negotiation_for(200, 2),
        "a cash-strapped club must still take a youngster on a free development loan"
    );
    let listing = country
        .transfer_market
        .listings
        .iter()
        .find(|l| l.player_id == 200)
        .expect("the approach must back itself with a synthetic loan listing");
    assert_eq!(
        listing.asking_price.amount, 0.0,
        "a development loan must be advertised free so a poor club's loan-fee cap can't filter it"
    );
}

/// Fix A end-to-end through the seller push: a full-but-weak GK line no
/// longer blocks a development keeper. The borrower already fields THREE
/// keepers (78 / 75 / 72), so a cover loan would need an impossible 88 to
/// clear best + 10 — but an 85-CA loan-listed prospect is a clear upgrade
/// on the fringe keeper he displaces, so the broadcast now places him
/// where before no club could take him.
#[test]
fn broadcast_places_development_keeper_into_full_but_weak_line() {
    let date = Fx::monday();

    let parent_main = Fx::team(
        10,
        1,
        TeamType::Main,
        9000,
        vec![
            Fx::keeper(101, 120, 120, 28, false),
            Fx::keeper(102, 118, 118, 26, false),
            Fx::keeper(103, 115, 115, 30, false),
        ],
    );
    let parent_reserve = Fx::team(
        11,
        1,
        TeamType::Reserve,
        6000,
        vec![Fx::keeper(200, 85, 150, 19, true)],
    );
    let parent = Fx::club(1, vec![parent_main, parent_reserve], 50_000_000);

    // A FULL three-deep GK line — but weak enough that the prospect
    // clearly beats the fringe keeper.
    let borrower_main = Fx::team(
        20,
        2,
        TeamType::Main,
        4000,
        vec![
            Fx::keeper(301, 78, 78, 27, false),
            Fx::keeper(302, 75, 75, 29, false),
            Fx::keeper(303, 72, 72, 31, false),
        ],
    );
    let mut borrower = Fx::club(2, vec![borrower_main], 5_000_000);
    borrower.transfer_plan.initialized = true;

    let mut country = Fx::country(vec![parent, borrower]);
    Fx::loan_list(&mut country, 200, 1, 11);

    LoanPipeline::broadcast_listed_loans(&mut country, date);

    assert!(
        country.transfer_market.has_active_negotiation_for(200, 2),
        "the broadcast should place a clearly-better development keeper into a full-but-weak \
         GK line — the case Fix A unblocks"
    );
}

/// Seller-side push: a National+ club broadcasts its loan-listed
/// youngster and a same-tier club with a keeper vacancy responds on the
/// first cycle — no waiting for that club to happen to scan.
#[test]
fn broadcast_places_listed_youth_at_a_same_tier_taker() {
    let date = Fx::monday();

    // National parent (world 5500): a blocked young keeper in the
    // reserves, loan-listed.
    let parent_main = Fx::team(
        10,
        1,
        TeamType::Main,
        5500,
        vec![
            Fx::keeper(101, 120, 120, 28, false),
            Fx::keeper(102, 118, 118, 26, false),
            Fx::keeper(103, 115, 115, 30, false),
        ],
    );
    let parent_reserve = Fx::team(
        11,
        1,
        TeamType::Reserve,
        4000,
        vec![Fx::keeper(200, 70, 150, 18, true)],
    );
    let parent = Fx::club(1, vec![parent_main, parent_reserve], 50_000_000);

    // National taker at the same tier (world 5500) with a keeper vacancy.
    let borrower_main = Fx::team(
        20,
        2,
        TeamType::Main,
        5500,
        vec![
            Fx::keeper(301, 55, 55, 27, false),
            Fx::keeper(302, 50, 50, 29, false),
        ],
    );
    let borrower = Fx::club(2, vec![borrower_main], 5_000_000);

    let mut country = Fx::country(vec![parent, borrower]);
    Fx::loan_list(&mut country, 200, 1, 11);

    LoanPipeline::broadcast_listed_loans(&mut country, date);

    assert!(
        country.transfer_market.has_active_negotiation_for(200, 2),
        "a National+ club should broadcast its loan-listed youngster and a same-tier club \
         with a vacancy responds"
    );
}

/// Non-development (surplus) loan: an Elite parent, the only realistic taker
/// a Regional club. A surplus player still cascades — the broadcast opens at
/// the parent's own (Elite) tier and widens one rung per unanswered window,
/// Elite → Continental → National → Regional, before the Regional club is
/// offered him. High reputation first, cascading down. (Development loanees
/// instead skip the cascade and are placed at the best taker immediately —
/// see `broadcast_places_development_loanee_at_best_taker_immediately`.)
#[test]
fn broadcast_cascades_non_development_loan_high_to_low() {
    let d0 = Fx::monday(); // 2026-01-05, a Monday

    let parent_main = Fx::team(
        10,
        1,
        TeamType::Main,
        9000,
        vec![
            Fx::keeper(101, 120, 120, 28, false),
            Fx::keeper(102, 118, 118, 26, false),
            Fx::keeper(103, 115, 115, 30, false),
        ],
    );
    // A 30-year-old surplus keeper (not a development loanee), so the staged
    // high → low cascade applies rather than immediate best-taker placement.
    let parent_reserve = Fx::team(
        11,
        1,
        TeamType::Reserve,
        6000,
        vec![Fx::keeper(200, 70, 150, 30, false)],
    );
    let parent = Fx::club(1, vec![parent_main, parent_reserve], 50_000_000);

    let borrower_main = Fx::team(
        20,
        2,
        TeamType::Main,
        4000,
        vec![
            Fx::keeper(301, 55, 55, 27, false),
            Fx::keeper(302, 50, 50, 29, false),
        ],
    );
    let borrower = Fx::club(2, vec![borrower_main], 5_000_000);

    let mut country = Fx::country(vec![parent, borrower]);
    Fx::loan_list(&mut country, 200, 1, 11);

    // Cycle 1: opens at Elite — no Elite taker exists, nobody responds.
    LoanPipeline::broadcast_listed_loans(&mut country, d0);
    assert!(
        !country.transfer_market.has_active_negotiation_for(200, 2),
        "no Elite club exists to take him on the first broadcast"
    );

    // Widen one tier per 14-day window: Continental, then National.
    LoanPipeline::broadcast_listed_loans(&mut country, d0 + Duration::days(14));
    LoanPipeline::broadcast_listed_loans(&mut country, d0 + Duration::days(28));
    assert!(
        !country.transfer_market.has_active_negotiation_for(200, 2),
        "still being offered above the Regional taker's tier"
    );

    // Fourth window reaches Regional — the club with a vacancy responds.
    LoanPipeline::broadcast_listed_loans(&mut country, d0 + Duration::days(42));
    assert!(
        country.transfer_market.has_active_negotiation_for(200, 2),
        "once the net widens to Regional, the club with a vacancy responds"
    );
}

/// A DEVELOPMENT loanee is shopped to the whole market at once: the parent
/// evaluates every club that would actually play him and places him at the
/// best — here the only realistic — taker on the FIRST broadcast, with no
/// slow tier cascade. Same Elite-parent / Regional-taker shape as the
/// non-development cascade above, but the youngster lands immediately
/// instead of after four unanswered windows.
#[test]
fn broadcast_places_development_loanee_at_best_taker_immediately() {
    let d0 = Fx::monday();

    let parent_main = Fx::team(
        10,
        1,
        TeamType::Main,
        9000,
        vec![
            Fx::keeper(101, 120, 120, 28, false),
            Fx::keeper(102, 118, 118, 26, false),
            Fx::keeper(103, 115, 115, 30, false),
        ],
    );
    // An 18-year-old development keeper: a prospect who needs minutes.
    let parent_reserve = Fx::team(
        11,
        1,
        TeamType::Reserve,
        6000,
        vec![Fx::keeper(200, 70, 150, 18, true)],
    );
    let parent = Fx::club(1, vec![parent_main, parent_reserve], 50_000_000);

    let borrower_main = Fx::team(
        20,
        2,
        TeamType::Main,
        4000,
        vec![
            Fx::keeper(301, 55, 55, 27, false),
            Fx::keeper(302, 50, 50, 29, false),
        ],
    );
    let borrower = Fx::club(2, vec![borrower_main], 5_000_000);

    let mut country = Fx::country(vec![parent, borrower]);
    Fx::loan_list(&mut country, 200, 1, 11);

    LoanPipeline::broadcast_listed_loans(&mut country, d0);
    assert!(
        country.transfer_market.has_active_negotiation_for(200, 2),
        "a development loanee is placed at the best (only) taker on the first \
         broadcast, not after a tier cascade"
    );
}

/// Resource gate: a below-National parent doesn't have the loan-
/// management reach to run a push, so it never broadcasts — it falls
/// back to passive listing no matter what takers exist.
#[test]
fn broadcast_skipped_for_a_below_national_parent() {
    let date = Fx::monday();

    // Regional parent (world 4000) — below the resource threshold.
    let parent_main = Fx::team(
        10,
        1,
        TeamType::Main,
        4000,
        vec![
            Fx::keeper(101, 90, 90, 28, false),
            Fx::keeper(102, 88, 88, 26, false),
        ],
    );
    let parent_reserve = Fx::team(
        11,
        1,
        TeamType::Reserve,
        3000,
        vec![Fx::keeper(200, 60, 120, 18, true)],
    );
    let parent = Fx::club(1, vec![parent_main, parent_reserve], 5_000_000);

    let borrower_main = Fx::team(
        20,
        2,
        TeamType::Main,
        3500,
        vec![Fx::keeper(301, 40, 40, 27, false)],
    );
    let borrower = Fx::club(2, vec![borrower_main], 1_000_000);

    let mut country = Fx::country(vec![parent, borrower]);
    Fx::loan_list(&mut country, 200, 1, 11);

    LoanPipeline::broadcast_listed_loans(&mut country, date);

    assert!(
        !country.transfer_market.has_active_negotiation_for(200, 2),
        "a Regional parent lacks the loan-management resource to run a push"
    );
}

/// B3 — the home-first hold has to release.
///
/// A `HomeCountry` candidate is kept off the domestic push for a
/// fortnight so his own league gets first refusal. But Pass 2 reset
/// the broadcast's `since` on every widen, the widen cadence IS that
/// fortnight, and the hold read `since` — so "posted for" ran 0 → 7 →
/// 0 for ever and the domestic market never once offered him.
/// The hold now reads `posted_since`, stamped when the entry is
/// created and never touched again.
#[test]
fn a_home_first_hold_releases_although_the_tier_widened_on_the_same_day() {
    let d0 = Fx::monday();

    let parent_main = Fx::team(
        10,
        1,
        TeamType::Main,
        5500,
        vec![
            Fx::keeper(101, 120, 120, 28, false),
            Fx::keeper(102, 118, 118, 26, false),
            Fx::keeper(103, 115, 115, 30, false),
        ],
    );
    // A foreign teenager his parent has decided should go HOME.
    let mut prospect = Fx::keeper(200, 70, 150, 18, true);
    prospect.country_id = 55;
    let parent_reserve = Fx::team(11, 1, TeamType::Reserve, 4000, vec![prospect]);
    let mut parent = Fx::club(1, vec![parent_main, parent_reserve], 50_000_000);
    parent
        .transfer_plan
        .loan_out_candidates
        .push(LoanOutCandidate {
            player_id: 200,
            reason: LoanOutReason::UnsettledAbroad,
            status: LoanOutStatus::Identified,
            loan_fee: 0.0,
            preferred_destination: LoanDestinationPreference::HomeCountry,
        });

    // A domestic club at the same tier with a keeper vacancy — it
    // would take him the moment the hold lets go.
    let borrower_main = Fx::team(
        20,
        2,
        TeamType::Main,
        5500,
        vec![
            Fx::keeper(301, 55, 55, 27, false),
            Fx::keeper(302, 50, 50, 29, false),
        ],
    );
    let borrower = Fx::club(2, vec![borrower_main], 5_000_000);

    let mut country = Fx::country(vec![parent, borrower]);
    Fx::loan_list(&mut country, 200, 1, 11);

    // Day 0: posted, and held — his own league gets first refusal.
    LoanPipeline::broadcast_listed_loans(&mut country, d0);
    assert!(
        !country.transfer_market.has_active_negotiation_for(200, 2),
        "a HomeCountry candidate is not shopped domestically on the day he is posted"
    );

    // Day 14: the tier widens on this very tick — which used to reset
    // the clock the hold reads, so it could never elapse.
    LoanPipeline::broadcast_listed_loans(&mut country, d0 + Duration::days(14));
    assert!(
        country.transfer_market.has_active_negotiation_for(200, 2),
        "a fortnight is a head start, not a veto: the domestic push must open"
    );
}
