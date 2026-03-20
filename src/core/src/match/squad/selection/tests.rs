use super::*;
use crate::{
    IntegerUtils, MatchTacticType, PeopleNameGeneratorData, PlayerCollection, PlayerGenerator,
    StaffCollection, TeamBuilder, TeamReputation, TeamType, TrainingSchedule,
};
use chrono::{NaiveTime, Utc};

fn test_names() -> PeopleNameGeneratorData {
    PeopleNameGeneratorData {
        first_names: vec!["Test".to_string()],
        last_names: vec!["Player".to_string()],
        nicknames: Vec::new(),
    }
}

#[test]
fn test_squad_selection_always_produces_11() {
    let team = generate_test_team();
    let staff = generate_test_staff();

    let result = SquadSelector::select(&team, &staff);

    assert_eq!(result.main_squad.len(), 11);
    assert!(!result.substitutes.is_empty());
    assert!(result.substitutes.len() <= helpers::DEFAULT_BENCH_SIZE);
}

#[test]
fn test_squad_always_has_goalkeeper() {
    let team = generate_test_team();
    let staff = generate_test_staff();

    let result = SquadSelector::select(&team, &staff);

    let has_gk = result.main_squad.iter().any(|p| {
        p.tactical_position.current_position == PlayerPositionType::Goalkeeper
    });
    assert!(has_gk, "Starting 11 must always have a goalkeeper");
}

#[test]
fn test_squad_no_duplicate_players() {
    let team = generate_test_team();
    let staff = generate_test_staff();

    let result = SquadSelector::select(&team, &staff);

    let mut all_ids: Vec<u32> = result.main_squad.iter().map(|p| p.id).collect();
    all_ids.extend(result.substitutes.iter().map(|p| p.id));

    let unique_count = {
        let mut sorted = all_ids.clone();
        sorted.sort();
        sorted.dedup();
        sorted.len()
    };
    assert_eq!(all_ids.len(), unique_count, "No player should appear twice");
}

#[test]
fn test_position_group_matching() {
    let score = helpers::position_fit_score(
        &generate_defender_center(),
        PlayerPositionType::DefenderCenterLeft,
        crate::club::PlayerFieldPositionGroup::Defender,
    );
    assert!(score > 5.0, "Same-group player should score well: {}", score);
}

// ========== Test helpers ==========

fn generate_test_team() -> crate::Team {
    let mut team = TeamBuilder::new()
        .id(1)
        .league_id(Some(1))
        .club_id(1)
        .name("Test Team".to_string())
        .slug("test-team".to_string())
        .team_type(TeamType::Main)
        .training_schedule(TrainingSchedule::new(
            NaiveTime::from_hms_opt(10, 0, 0).unwrap(),
            NaiveTime::from_hms_opt(17, 0, 0).unwrap(),
        ))
        .reputation(TeamReputation::new(100, 100, 100))
        .players(PlayerCollection::new(generate_test_players()))
        .staffs(StaffCollection::new(Vec::new()))
        .tactics(Some(Tactics::new(MatchTacticType::T442)))
        .build()
        .expect("Failed to build test team");

    team.tactics = Some(Tactics::new(MatchTacticType::T442));
    team
}

fn generate_test_staff() -> Staff {
    crate::StaffStub::default()
}

fn generate_test_players() -> Vec<Player> {
    let mut players = Vec::new();
    let names = test_names();

    for &position in &[
        PlayerPositionType::Goalkeeper,
        PlayerPositionType::DefenderLeft,
        PlayerPositionType::DefenderCenter,
        PlayerPositionType::DefenderRight,
        PlayerPositionType::MidfielderLeft,
        PlayerPositionType::MidfielderCenter,
        PlayerPositionType::MidfielderRight,
        PlayerPositionType::Striker,
    ] {
        for _ in 0..3 {
            let level = IntegerUtils::random(15, 20) as u8;
            let player =
                PlayerGenerator::generate(1, Utc::now().date_naive(), position, level, &names);
            players.push(player);
        }
    }

    players
}

fn generate_defender_center() -> Player {
    let names = test_names();
    PlayerGenerator::generate(
        1,
        Utc::now().date_naive(),
        PlayerPositionType::DefenderCenter,
        18,
        &names,
    )
}
