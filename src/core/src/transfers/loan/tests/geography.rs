//! A loan is agreed by three parties, and each of them knows where it
//! goes: the lender's own placement network, the borrower's registration
//! room and reach, and what the player makes of the place.

use std::collections::HashMap;

use chrono::NaiveDate;

use super::super::*;
use crate::transfers::ScoutingRegion;
use crate::transfers::market::knowledge::{
    LoanPlacementKnowledge, LoanPlacementLedger, PlacementReachIndex,
};
use crate::transfers::market::map::{
    CorridorWeight, CountryTransferProfile, MarketCountryFacts, MarketMap,
};
use crate::club::player::mind::CareerPlanView;
use crate::transfers::MarketAffinity;
use crate::PlayerFieldPositionGroup;

/// A three-country world: a lender, the place it has always sent boys,
/// and a place no card on either side names.
struct Geo;

impl Geo {
    /// The lender.
    const HOME: u32 = 1;
    /// Where its nationals go, on both cards.
    const WORKED: u32 = 2;
    /// A league in another region that neither card mentions.
    const STRANGER: u32 = 3;

    fn day() -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 8, 1).unwrap()
    }

    fn map() -> MarketMap {
        let mut facts = HashMap::new();
        for (id, code, continent, top_flight) in [
            (Self::HOME, "nl", 1u32, 6500u16),
            (Self::WORKED, "be", 1, 3500),
            (Self::STRANGER, "jp", 4, 6000),
        ] {
            facts.insert(
                id,
                MarketCountryFacts {
                    id,
                    code: code.to_string(),
                    continent_id: continent,
                    region: ScoutingRegion::from_country(continent, code),
                    reputation: 6000,
                    top_flight_reputation: top_flight,
                    median_top_flight_wage: 100_000,
                },
            );
        }
        let mut profiles = HashMap::new();
        profiles.insert(
            Self::HOME,
            CountryTransferProfile {
                export: vec![CorridorWeight {
                    country_id: Self::WORKED,
                    weight: 1.0,
                    money: false,
                }],
                ..Default::default()
            },
        );
        profiles.insert(
            Self::WORKED,
            CountryTransferProfile {
                import: vec![CorridorWeight {
                    country_id: Self::HOME,
                    weight: 1.0,
                    money: false,
                }],
                ..Default::default()
            },
        );
        MarketMap::new(profiles, facts)
    }
}

#[test]
fn a_route_a_club_has_worked_is_a_route_it_knows() {
    let map = Geo::map();
    let mut ledger = LoanPlacementLedger::default();
    for _ in 0..5 {
        ledger.record_placement(Geo::STRANGER, Geo::day());
    }
    let trust = LoanPlacementKnowledge::of(
        &map,
        Geo::HOME,
        &ledger,
        0,
        Geo::STRANGER,
        Geo::day(),
    );
    assert!(
        trust > 0.9,
        "five boys placed there last month is a placement network: {trust}"
    );
    assert_eq!(
        LoanPlacementKnowledge::of(
            &map,
            Geo::HOME,
            &LoanPlacementLedger::default(),
            0,
            Geo::HOME,
            Geo::day(),
        ),
        1.0,
        "a club always knows its own country"
    );
}

#[test]
fn a_pair_no_card_names_prices_off_the_corridor_and_never_at_zero() {
    let map = Geo::map();
    let empty = LoanPlacementLedger::default();
    let worked = LoanPlacementKnowledge::of(&map, Geo::HOME, &empty, 0, Geo::WORKED, Geo::day());
    let stranger =
        LoanPlacementKnowledge::of(&map, Geo::HOME, &empty, 0, Geo::STRANGER, Geo::day());

    assert!(
        (worked - 0.5).abs() < 0.001,
        "the export card is worth half, exactly as the import card is: {worked}"
    );
    assert!(
        stranger > 0.0,
        "a route that has never existed must stay openable: {stranger}"
    );
    assert!(
        stranger < worked,
        "and it is not the route the country's own card names: {stranger} vs {worked}"
    );
    let corridor = MarketAffinity::corridor_strength(&map, Geo::HOME, Geo::STRANGER);
    assert!(
        stranger <= corridor,
        "the fallback is the corridor at half weight, never more than the corridor"
    );
}

#[test]
fn a_club_with_no_ledger_of_its_own_still_reads_its_countrys_card() {
    let map = Geo::map();
    let index = PlacementReachIndex::default();
    let worked = index.trust(&map, 77, Geo::HOME, Geo::WORKED, Geo::day());
    let stranger = index.trust(&map, 77, Geo::HOME, Geo::STRANGER, Geo::day());
    assert!(
        worked > stranger,
        "an unstaged club is its country's card and nothing else: {worked} vs {stranger}"
    );
}

#[test]
fn a_destination_the_parent_knows_nothing_about_is_rare_rather_than_forbidden() {
    let staged = ParentWillingness::open();
    let known = staged.placed_into(1.0);
    let blind = staged.placed_into(0.0);

    assert_eq!(known.score, staged.score, "a route it works costs nothing");
    assert!(
        blind.score > 0.0,
        "a club will send a boy somewhere new: {}",
        blind.score
    );
    assert!(
        blind.score < 0.3 * staged.score,
        "…and it is the exception: {}",
        blind.score
    );
    assert!(
        staged.placed_into(0.3).score > staged.placed_into(0.1).score,
        "the term is a ramp, not a step"
    );
}

/// The borrower's side: a slot is a scarce thing, and a scarce thing is
/// not an absent one — the shipped world's own squads breach these
/// limits, so a club over its quota borrows rarely rather than never.
#[test]
fn a_full_quota_narrows_the_appetite_without_closing_the_league() {
    let reading = |slot_room: f32| BorrowerReading {
        base_by_tier: 1.0,
        season_phase: 1.0,
        group: PlayerFieldPositionGroup::Defender,
        count: 4,
        ideal_depth: 8,
        best_here: 100,
        candidate: 110,
        clearly_better_ahead: 0,
        allowed_ahead: 2,
        band_here: 0.9,
        band_target: 0.9,
        readiness: 0.5,
        standing_ratio: 0.9,
        league_ratio: 0.9,
        need: 0.8,
        slot_room,
    };
    let open = BorrowerAppetite::of(&reading(1.0));
    let tight = BorrowerAppetite::of(&reading(0.25));
    let full = BorrowerAppetite::of(&reading(0.1));

    assert!(open.score > tight.score && tight.score > full.score);
    assert!(
        full.score > 0.0,
        "a league whose squads are over quota still borrows: {}",
        full.score
    );
    assert_eq!(open.slot_room, 1.0, "and the trace says which term did it");
}

/// The player's side: a year somewhere he knows nothing about is a
/// reason to say no, and a smaller one than dropping two divisions.
#[test]
fn a_strange_country_costs_him_and_a_familiar_one_does_not() {
    let reading = |familiarity: f32, resignation: f32| ConsentReading {
        plan: CareerPlanView::none(),
        band_here: 0.9,
        renown_gap: 0.0,
        renown_band: 2000.0,
        going_home: false,
        resignation,
        familiarity,
    };
    let familiar = PlayerConsent::of(&reading(1.0, 0.0));
    let strange = PlayerConsent::of(&reading(0.0, 0.0));
    let resigned = PlayerConsent::of(&reading(0.0, 1.0));

    assert_eq!(familiar.familiarity_cost, 0.0);
    assert!(strange.score < familiar.score);
    assert!(strange.score > 0.0, "strange is a cost, never a wall");
    assert!(
        resigned.score > strange.score,
        "a man who has asked to go anywhere goes anywhere"
    );
}

#[test]
fn a_man_going_home_reads_familiar_whatever_the_cards_say() {
    let map = Geo::map();
    assert_eq!(
        MarketAffinity::player_affinity(&map, Geo::STRANGER, Geo::STRANGER, 0.0),
        1.0,
        "there is no such thing as a strange homecoming"
    );
    let abroad = MarketAffinity::player_affinity(&map, Geo::HOME, Geo::STRANGER, 0.0);
    assert!(abroad < 1.0, "and a stranger's country is not home: {abroad}");
    assert_eq!(
        MarketAffinity::player_affinity(&map, Geo::HOME, Geo::STRANGER, 1.0),
        1.0,
        "a man who speaks the place has his one strong reason"
    );
}

#[test]
fn the_trace_line_carries_every_new_term() {
    let parent = ParentWillingness::open().placed_into(0.0);
    let borrower = BorrowerAppetite::of(&BorrowerReading {
        base_by_tier: 1.0,
        season_phase: 1.0,
        group: PlayerFieldPositionGroup::Defender,
        count: 4,
        ideal_depth: 8,
        best_here: 100,
        candidate: 110,
        clearly_better_ahead: 0,
        allowed_ahead: 2,
        band_here: 0.9,
        band_target: 0.9,
        readiness: 0.5,
        standing_ratio: 0.9,
        league_ratio: 0.9,
        need: 0.8,
        slot_room: 0.25,
    });
    let player = PlayerConsent::of(&ConsentReading {
        plan: CareerPlanView::none(),
        band_here: 0.9,
        renown_gap: 0.0,
        renown_band: 2000.0,
        going_home: false,
        resignation: 0.0,
        familiarity: 0.0,
    });
    let money = LoanMoney::of(&MoneyReading {
        weight: 0.2,
        carry: 0.3,
        asking: 0.0,
        max_loan_fee: 1_000_000.0,
        development: false,
    });

    let line = LoanAgreement::explain(&parent, &borrower, &player, &money);
    for term in ["placement=", "slots=", "strange="] {
        assert!(line.contains(term), "{term} missing from: {line}");
    }
}
