//! Tests for the two units the staff-recommendation split made reachable.
//!
//! Both were previously only exercisable through a whole-country tick, which
//! is why neither had a test: the guards that decide whether a club is scanned
//! at all, and the cap the commit applies when it writes the staged
//! recommendations back. Extracting them is what pays for the extraction.

use super::*;
use crate::transfers::tests::kit::{TestClub, TestCountry, TestDate, TestPlayer};

/// Fixtures: a club the guards should let through, the tick it is scanned
/// against, a one-club country to scan it in, and a recommendation row.
struct Fx;

impl Fx {
    /// A club whose plan has been built and which is under the cap — the ordinary
    /// case the guards are supposed to let through.
    fn scannable_club(id: u32) -> Club {
        let mut club = TestClub::new(id)
            .players(vec![
                TestPlayer::new(id * 100 + 1).ability(120).age(24).build(),
                TestPlayer::new(id * 100 + 2).ability(110).age(27).build(),
            ])
            .build();
        club.transfer_plan.initialized = true;
        club.transfer_plan.total_budget = 5_000_000.0;
        club
    }

    fn tick(date: NaiveDate) -> AdviceTick {
        AdviceTick {
            date,
            is_january: false,
            price_level: 1.0,
            current_window: None,
        }
    }

    /// Opens a scan against a one-club country, or reports why it refused.
    fn open_against(club: Club) -> Option<()> {
        let date = TestDate::today();
        let country = TestCountry::new(1).clubs(vec![club]).build();
        let lookup = CountryPlayerLookup::build(&country);
        let snapshots: Vec<PlayerSnapshot> = Vec::new();
        ClubAdviceScan::open(
            &country,
            &country.clubs[0],
            &snapshots,
            &lookup,
            Fx::tick(date),
        )
        .map(|_| ())
    }

    fn recommendation(player_id: u32, date: NaiveDate) -> StaffRecommendation {
        StaffRecommendation {
            player_id,
            recommender_staff_id: 7,
            source: RecommendationSource::ScoutNetwork,
            recommendation_type: RecommendationType::ReadyForStepUp,
            assessed_ability: 120,
            assessed_potential: 140,
            confidence: 0.6,
            estimated_fee: 1_000_000.0,
            date_recommended: date,
        }
    }
}

#[test]
fn an_ordinary_club_opens_a_scan() {
    assert!(
        Fx::open_against(Fx::scannable_club(1)).is_some(),
        "a club with a squad, an initialised plan and room under the cap is scannable"
    );
}

#[test]
fn a_club_with_no_squad_is_not_scanned() {
    let mut club = TestClub::new(1).teams(Vec::new()).build();
    club.transfer_plan.initialized = true;

    assert!(
        Fx::open_against(club).is_none(),
        "the scan indexes teams[0]; a club with no squad must be refused before that"
    );
}

#[test]
fn a_club_whose_plan_was_never_built_is_not_scanned() {
    let mut club = Fx::scannable_club(1);
    club.transfer_plan.initialized = false;

    assert!(
        Fx::open_against(club).is_none(),
        "recommendations are staged against a plan; without one there is nothing to stage into"
    );
}

#[test]
fn a_club_already_at_the_recommendation_cap_is_not_scanned() {
    let date = TestDate::today();
    let mut club = Fx::scannable_club(1);
    for player_id in 0..10 {
        club.transfer_plan
            .staff_recommendations
            .push(Fx::recommendation(player_id, date));
    }

    assert!(
        Fx::open_against(club).is_none(),
        "ten is the ceiling the commit applies; scanning past it only produces work to throw away"
    );

    let mut just_under = Fx::scannable_club(1);
    for player_id in 0..9 {
        just_under
            .transfer_plan
            .staff_recommendations
            .push(Fx::recommendation(player_id, date));
    }
    assert!(
        Fx::open_against(just_under).is_some(),
        "nine is under the ceiling, so the club still gets scanned"
    );
}

#[test]
fn the_commit_writes_no_more_than_the_club_rates() {
    let date = TestDate::today();
    let mut country = TestCountry::new(1)
        .clubs(vec![Fx::scannable_club(1)])
        .build();

    let team = country.clubs[0].teams.teams.first().unwrap();
    let cap = PipelineProcessor::staff_recommendation_cap_score(
        team.reputation.level(),
        team.reputation.overall_score(),
    );

    // Twice the cap, staged. The commit is the only thing standing between a
    // productive week of scouting and a plan nobody can read.
    let staged: Vec<Vec<RecommendationAction>> = vec![
        (0..(cap as u32 * 2))
            .map(|player_id| RecommendationAction {
                club_id: 1,
                recommendation: Fx::recommendation(player_id, date),
            })
            .collect(),
    ];

    AdviceCommit::apply(&mut country, staged, date);

    assert_eq!(
        country.clubs[0].transfer_plan.staff_recommendations.len(),
        cap,
        "the commit stops at the cap the club's reputation earns it"
    );
}

#[test]
fn every_committed_recommendation_reaches_the_scout_s_books() {
    let date = TestDate::today();
    let mut country = TestCountry::new(1)
        .clubs(vec![Fx::scannable_club(1)])
        .build();

    let staged: Vec<Vec<RecommendationAction>> = vec![vec![RecommendationAction {
        club_id: 1,
        recommendation: Fx::recommendation(4_242, date),
    }]];

    AdviceCommit::apply(&mut country, staged, date);

    let plan = &country.clubs[0].transfer_plan;
    assert_eq!(plan.staff_recommendations.len(), 1);
    assert!(
        plan.scout_monitoring
            .iter()
            .any(|row| row.player_id == 4_242 && row.scout_staff_id == 7),
        "a recommendation is mirrored into a monitoring row so the recruitment \
         meeting and the UI see the player on that scout's books too"
    );
}
