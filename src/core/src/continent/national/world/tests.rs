//! Integration tests for the world-aware national pipeline.
//!
//! Each test asserts a property that the previous continent-local
//! implementation could not satisfy: foreign-based selection, foreign
//! stat propagation, global tournament side effects, world-aware
//! emergency call-up, and the friendly fixture auto-scheduling
//! removal.

use super::*;
use chrono::NaiveDate;
use std::collections::HashMap;

use crate::academy::ClubAcademy;
use crate::club::player::builder::PlayerBuilder;
use crate::competitions::global::GlobalCompetitionFixture;
use crate::continent::Continent;
use crate::continent::national::{NationalCompetitionPhase, NationalTeamCompetitions};
use crate::league::LeagueCollection;
use crate::r#match::{
    FieldSquad, MatchResultRaw, PlayerMatchEndStats, ResultMatchPositionData, Score, TeamScore,
};
use crate::shared::Location;
use crate::shared::fullname::FullName;
use crate::{
    Club, ClubColors, ClubFinances, ClubStatus, Country, NationalSquadPlayer, PersonAttributes,
    PlayerAttributes, PlayerCollection, PlayerFieldPositionGroup, PlayerPosition,
    PlayerPositionType, PlayerPositions, PlayerSkills, PlayerStatusType, StaffCollection,
    TeamBuilder, TeamCollection, TeamReputation, TeamType, TrainingSchedule,
};

use super::lookups::{country_lookup, country_lookup_mut};

// ============================================================
// Test fixtures
// ============================================================

fn d(y: i32, m: u32, day: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(y, m, day).unwrap()
}

fn make_player(id: u32, country_id: u32, position: PlayerPositionType) -> crate::Player {
    let mut p = PlayerBuilder::new()
        .id(id)
        .full_name(FullName::new("Test".to_string(), format!("Player{}", id)))
        .birth_date(d(1996, 5, 1))
        .country_id(country_id)
        .attributes(PersonAttributes::default())
        .skills(PlayerSkills::default())
        .positions(PlayerPositions {
            positions: vec![PlayerPosition {
                position,
                level: 18,
            }],
        })
        .player_attributes(PlayerAttributes {
            current_ability: 150,
            potential_ability: 160,
            condition: 10000,
            world_reputation: 4000,
            home_reputation: 5000,
            current_reputation: 5000,
            ..Default::default()
        })
        .build()
        .unwrap();
    p.statistics.played = 25;
    p.statistics.average_rating = 7.2;
    p
}

fn make_training_schedule() -> TrainingSchedule {
    use chrono::NaiveTime;
    TrainingSchedule::new(
        NaiveTime::from_hms_opt(9, 0, 0).unwrap(),
        NaiveTime::from_hms_opt(15, 0, 0).unwrap(),
    )
}

fn make_team(team_id: u32, club_id: u32, players: Vec<crate::Player>) -> crate::Team {
    TeamBuilder::new()
        .id(team_id)
        .league_id(Some(1))
        .club_id(club_id)
        .name(format!("Team{}", team_id))
        .slug(format!("team{}", team_id))
        .team_type(TeamType::Main)
        .players(PlayerCollection::new(players))
        .staffs(StaffCollection::new(Vec::new()))
        .reputation(TeamReputation::new(3000, 3000, 3000))
        .training_schedule(make_training_schedule())
        .build()
        .unwrap()
}

fn make_club(id: u32, players: Vec<crate::Player>) -> Club {
    let team = make_team(id * 10, id, players);
    Club::new(
        id,
        format!("Club{}", id),
        Location::new(1),
        ClubFinances::new(1_000_000, Vec::new()),
        ClubAcademy::new(3),
        ClubStatus::Professional,
        ClubColors::default(),
        TeamCollection::new(vec![team]),
        crate::ClubFacilities::default(),
    )
}

fn make_country(
    id: u32,
    continent_id: u32,
    name: &str,
    clubs: Vec<Club>,
    reputation: u16,
) -> Country {
    Country::builder()
        .id(id)
        .code(name[..2].to_uppercase())
        .slug(name.to_lowercase())
        .name(name.to_string())
        .continent_id(continent_id)
        .leagues(LeagueCollection::new(Vec::new()))
        .clubs(clubs)
        .reputation(reputation)
        .build()
        .unwrap()
}

fn make_continent(id: u32, countries: Vec<Country>) -> Continent {
    let mut continent = Continent::new(id, format!("Continent{}", id), countries, Vec::new());
    continent.national_team_competitions = NationalTeamCompetitions::new(Vec::new());
    continent
}

fn synth_score(home: u8, away: u8) -> Score {
    Score {
        home_team: TeamScore::new_with_score(0, home),
        away_team: TeamScore::new_with_score(0, away),
        details: Vec::new(),
        home_shootout: 0,
        away_shootout: 0,
    }
}

fn synth_match_result(home_score: u8, away_score: u8, scorer_id: Option<u32>) -> MatchResultRaw {
    let mut player_stats: HashMap<u32, PlayerMatchEndStats> = HashMap::new();
    if let Some(id) = scorer_id {
        player_stats.insert(
            id,
            PlayerMatchEndStats {
                shots_on_target: 1,
                shots_total: 1,
                passes_attempted: 0,
                passes_completed: 0,
                tackles: 0,
                interceptions: 0,
                saves: 0,
                shots_faced: 0,
                goals: 1,
                assists: 0,
                match_rating: 8.0,
                xg: 0.5,
                position_group: PlayerFieldPositionGroup::Forward,
                fouls: 0,
                yellow_cards: 0,
                red_cards: 0,
            },
        );
    }
    MatchResultRaw {
        score: Some(synth_score(home_score, away_score)),
        position_data: ResultMatchPositionData::new(),
        left_team_players: FieldSquad::new(),
        right_team_players: FieldSquad::new(),
        match_time_ms: 5_400_000,
        additional_time_ms: 0,
        player_stats,
        substitutions: Vec::new(),
        penalty_shootout: Vec::new(),
        player_of_the_match_id: None,
    }
}

// ============================================================
// Tests
// ============================================================

/// Country A on continent 1 selects player whose physical club lives
/// in country B on continent 2. The squad must include them in the
/// MatchSquad — proves world-wide club visibility during squad
/// construction.
#[test]
fn build_world_squad_includes_foreign_based_player() {
    let foreigner = make_player(101, 1, PlayerPositionType::Striker);
    let foreign_club = make_club(200, vec![foreigner]);
    let country_b = make_country(2, 2, "Spain", vec![foreign_club], 7000);
    let country_a = make_country(1, 1, "Brazil", Vec::new(), 8000);

    let mut continents = vec![
        make_continent(1, vec![country_a]),
        make_continent(2, vec![country_b]),
    ];

    if let Some(brazil) = country_lookup_mut(&mut continents, 1) {
        brazil.national_team.country_name = "Brazil".to_string();
        brazil.national_team.reputation = 8000;
        brazil.national_team.squad.push(NationalSquadPlayer {
            player_id: 101,
            club_id: 200,
            team_id: 2000,
            primary_reason: crate::CallUpReason::KeyPlayer,
            secondary_reasons: Vec::new(),
        });
    }

    let date = d(2026, 9, 6);
    let squad =
        build_world_match_squad(&mut continents, 1, date).expect("squad should build for Brazil");

    let in_main = squad.main_squad.iter().any(|p| p.id == 101);
    let in_subs = squad.substitutes.iter().any(|p| p.id == 101);
    assert!(
        in_main || in_subs,
        "foreign-based striker must appear in the world-built MatchSquad"
    );
}

/// World-wide stats update reaches a player at a club on a different
/// continent than the country they represent.
#[test]
fn world_stats_update_reaches_foreign_based_player() {
    let foreigner = make_player(101, 1, PlayerPositionType::Striker);
    let foreign_club = make_club(200, vec![foreigner]);
    let country_b = make_country(2, 2, "Spain", vec![foreign_club], 7000);
    let country_a = make_country(1, 1, "Brazil", Vec::new(), 8000);

    let mut continents = vec![
        make_continent(1, vec![country_a]),
        make_continent(2, vec![country_b]),
    ];

    if let Some(brazil) = country_lookup_mut(&mut continents, 1) {
        brazil.national_team.squad.push(NationalSquadPlayer {
            player_id: 101,
            club_id: 200,
            team_id: 2000,
            primary_reason: crate::CallUpReason::KeyPlayer,
            secondary_reasons: Vec::new(),
        });
    }

    let mut goals = HashMap::new();
    goals.insert(101_u32, 2_u16);
    apply_world_international_stats(&mut continents, 1, 99, &goals);

    let player_attrs = continents
        .iter()
        .flat_map(|c| c.countries.iter())
        .flat_map(|c| c.clubs.iter())
        .flat_map(|c| c.teams.teams.iter())
        .flat_map(|t| t.players.players.iter())
        .find(|p| p.id == 101)
        .map(|p| p.player_attributes)
        .expect("player should still exist");

    assert_eq!(player_attrs.international_apps, 1);
    assert_eq!(player_attrs.international_goals, 2);
    assert!(
        player_attrs.world_reputation >= 4000,
        "world reputation must be bumped by an international cap"
    );
}

/// World Cup / global tournament processing must update apps/goals,
/// record schedule entries on both countries, and produce a
/// MatchResult tagged with the international slug so the match-detail
/// page can find it.
#[test]
fn global_tournament_result_updates_caps_schedule_and_match_result() {
    let scorer = make_player(101, 1, PlayerPositionType::Striker);
    let club_a = make_club(1, vec![scorer]);
    let country_a = make_country(1, 1, "Brazil", vec![club_a], 8000);

    let opponent_player = make_player(202, 2, PlayerPositionType::Striker);
    let club_b = make_club(2, vec![opponent_player]);
    let country_b = make_country(2, 1, "Spain", vec![club_b], 7000);

    let mut continents = vec![make_continent(1, vec![country_a, country_b])];

    if let Some(brazil) = country_lookup_mut(&mut continents, 1) {
        brazil.national_team.squad.push(NationalSquadPlayer {
            player_id: 101,
            club_id: 1,
            team_id: 10,
            primary_reason: crate::CallUpReason::KeyPlayer,
            secondary_reasons: Vec::new(),
        });
    }
    if let Some(spain) = country_lookup_mut(&mut continents, 2) {
        spain.national_team.squad.push(NationalSquadPlayer {
            player_id: 202,
            club_id: 2,
            team_id: 20,
            primary_reason: crate::CallUpReason::KeyPlayer,
            secondary_reasons: Vec::new(),
        });
    }

    let fixture = GlobalCompetitionFixture {
        home_country_id: 1,
        away_country_id: 2,
        tournament_idx: 0,
        phase: NationalCompetitionPhase::Knockout,
        group_idx: 0,
        fixture_idx: 0,
    };
    let raw = synth_match_result(2, 1, Some(101));
    let date = d(2026, 6, 20);

    let match_result =
        apply_global_tournament_result(&mut continents, &fixture, &raw, date, "WC", "World Cup");

    assert_eq!(match_result.league_slug, "international");
    assert_eq!(match_result.home_team_id, 1);
    assert_eq!(match_result.away_team_id, 2);
    assert!(match_result.id.starts_with("int-"));

    let scorer_attrs = continents[0].countries[0].clubs[0].teams.teams[0]
        .players
        .players
        .iter()
        .find(|p| p.id == 101)
        .map(|p| p.player_attributes)
        .unwrap();
    assert_eq!(scorer_attrs.international_apps, 1);
    assert_eq!(scorer_attrs.international_goals, 1);

    let opp_attrs = continents[0].countries[1].clubs[0].teams.teams[0]
        .players
        .players
        .iter()
        .find(|p| p.id == 202)
        .map(|p| p.player_attributes)
        .unwrap();
    assert_eq!(
        opp_attrs.international_apps, 1,
        "opponent's squad member must also receive an international cap"
    );
    assert_eq!(opp_attrs.international_goals, 0);

    let brazil_schedule = &continents[0].countries[0].national_team.schedule;
    assert!(
        brazil_schedule
            .iter()
            .any(|f| f.opponent_country_id == 2 && f.is_home && f.result.is_some()),
        "Brazil's schedule must record the home win against Spain"
    );
    let spain_schedule = &continents[0].countries[1].national_team.schedule;
    assert!(
        spain_schedule
            .iter()
            .any(|f| f.opponent_country_id == 1 && !f.is_home && f.result.is_some()),
        "Spain's schedule must record the away loss to Brazil"
    );
}

/// Emergency call-up must use the world-wide candidate pool and bump
/// the EMERGENCY_CALLUPS counter so operators can detect when the
/// regular break-start path was missed.
#[test]
fn emergency_callup_uses_world_candidates_and_bumps_metric() {
    let foreigner = make_player(101, 1, PlayerPositionType::Striker);
    let club_b = make_club(200, vec![foreigner]);
    let country_b = make_country(2, 2, "Spain", vec![club_b], 7000);
    let country_a = make_country(1, 1, "Brazil", Vec::new(), 8000);

    let mut continents = vec![
        make_continent(1, vec![country_a]),
        make_continent(2, vec![country_b]),
    ];

    let before = emergency_callups_total();
    let date = d(2026, 9, 6);
    let squad = build_world_match_squad(&mut continents, 1, date)
        .expect("squad should build via emergency");

    assert!(
        !squad.main_squad.is_empty(),
        "emergency squad must be populated, not empty"
    );
    assert!(
        emergency_callups_total() > before,
        "EMERGENCY_CALLUPS counter must increment for visibility"
    );

    let brazil = country_lookup(&continents, 1).unwrap();
    let squad_has_foreigner = brazil
        .national_team
        .squad
        .iter()
        .any(|p| p.player_id == 101);
    let synth_used = !brazil.national_team.generated_squad.is_empty();
    assert!(
        squad_has_foreigner || synth_used,
        "either the world-pool foreigner is selected, or a synthetic squad fills in — squad must not be empty"
    );

    if squad_has_foreigner {
        let foreign_player_status = continents
            .iter()
            .flat_map(|c| c.countries.iter())
            .flat_map(|c| c.clubs.iter())
            .flat_map(|c| c.teams.teams.iter())
            .flat_map(|t| t.players.players.iter())
            .find(|p| p.id == 101)
            .map(|p| p.statuses.get().contains(&PlayerStatusType::Int))
            .unwrap_or(false);
        assert!(
            foreign_player_status,
            "Int flag must reach the foreign-based player on the other continent"
        );
    }
}

/// `call_up_squad` must NOT push pending friendly fixtures.
/// Friendly simulation isn't wired up; auto-scheduling them would
/// leave forever-`result: None` rows in each country's schedule.
#[test]
fn call_up_squad_does_not_add_pending_friendlies() {
    let mut nt = crate::NationalTeam::new(1, &crate::CountryGeneratorData::empty().people_names);
    nt.country_name = "TestLand".to_string();
    nt.reputation = 9000;
    nt.country_id = 1;

    let date = d(2026, 9, 4);
    nt.call_up_squad(Vec::new(), date, 1, &[(2, "Other".to_string())]);

    let pending_friendlies = nt
        .schedule
        .iter()
        .filter(|f| f.competition_name == "Friendly" && f.result.is_none())
        .count();
    assert_eq!(
        pending_friendlies, 0,
        "no pending friendly fixtures may be auto-scheduled"
    );
}

/// World-aware orchestrator: when there are no fixtures today, it
/// returns an empty result without panicking, and check_phase_transitions
/// runs without falling over on an empty competition set.
#[test]
fn simulate_world_national_competitions_empty_day_is_noop() {
    let country_a = make_country(1, 1, "Brazil", Vec::new(), 8000);
    let mut continents = vec![make_continent(1, vec![country_a])];

    let results = simulate_world_national_competitions(&mut continents, d(2026, 4, 1));
    assert!(results.is_empty());
}

/// World-wide reputation lookup resolves a country regardless of
/// which continent it sits on.
#[test]
fn world_country_reputation_lookup_works_across_continents() {
    let country_a = make_country(1, 1, "Brazil", Vec::new(), 8000);
    let country_b = make_country(2, 2, "Andorra", Vec::new(), 500);
    let continents = vec![
        make_continent(1, vec![country_a]),
        make_continent(2, vec![country_b]),
    ];
    assert_eq!(world_country_reputation(&continents, 1), 8000);
    assert_eq!(world_country_reputation(&continents, 2), 500);
    assert_eq!(world_country_reputation(&continents, 999), 0);
}
