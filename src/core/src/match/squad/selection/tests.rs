use super::*;
use crate::{
    IntegerUtils, MatchTacticType, PeopleNameGeneratorData, PlayerCollection, PlayerGenerator,
    PlayerPosition, StaffCollection, TeamBuilder, TeamReputation, TeamType, TrainingSchedule,
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

    let has_gk = result
        .main_squad
        .iter()
        .any(|p| p.tactical_position.current_position == PlayerPositionType::Goalkeeper);
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
    assert!(
        score > 5.0,
        "Same-group player should score well: {}",
        score
    );
}

#[test]
fn test_global_assignment_preserves_specialists() {
    let mut team = generate_test_team();
    let staff = generate_test_staff();
    let date = Utc::now().date_naive();

    let mut players = vec![
        make_test_player(1, &[(PlayerPositionType::Goalkeeper, 20)], 120, date),
        make_test_player(
            2,
            &[
                (PlayerPositionType::DefenderLeft, 20),
                (PlayerPositionType::DefenderCenterLeft, 20),
            ],
            180,
            date,
        ),
        make_test_player(3, &[(PlayerPositionType::DefenderLeft, 18)], 135, date),
        make_test_player(4, &[(PlayerPositionType::DefenderCenterLeft, 5)], 80, date),
    ];

    let filler_positions = [
        PlayerPositionType::DefenderCenterRight,
        PlayerPositionType::DefenderRight,
        PlayerPositionType::MidfielderLeft,
        PlayerPositionType::MidfielderCenterLeft,
        PlayerPositionType::MidfielderCenterRight,
        PlayerPositionType::MidfielderRight,
        PlayerPositionType::ForwardLeft,
        PlayerPositionType::ForwardRight,
    ];
    for (idx, pos) in filler_positions.iter().enumerate() {
        players.push(make_test_player(10 + idx as u32, &[(*pos, 16)], 120, date));
    }

    team.players = PlayerCollection::new(players);
    team.tactics = Some(Tactics::new(MatchTacticType::T442));

    let result = SquadSelector::select(&team, &staff);
    let elite_pos = result
        .main_squad
        .iter()
        .find(|p| p.id == 2)
        .map(|p| p.tactical_position.current_position);
    let specialist_pos = result
        .main_squad
        .iter()
        .find(|p| p.id == 3)
        .map(|p| p.tactical_position.current_position);

    assert_eq!(elite_pos, Some(PlayerPositionType::DefenderCenterLeft));
    assert_eq!(specialist_pos, Some(PlayerPositionType::DefenderLeft));
}

#[test]
fn test_selection_policy_from_context() {
    assert_eq!(
        SelectionPolicy::from_context(&SelectionContext {
            match_importance: 0.9,
            ..SelectionContext::default()
        }),
        SelectionPolicy::BestEleven
    );
    assert_eq!(
        SelectionPolicy::from_context(&SelectionContext {
            match_importance: 0.3,
            ..SelectionContext::default()
        }),
        SelectionPolicy::CupRotation
    );
    assert_eq!(
        SelectionPolicy::from_context(&SelectionContext {
            is_friendly: true,
            ..SelectionContext::default()
        }),
        SelectionPolicy::YouthDevelopment
    );
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

fn make_test_player(
    id: u32,
    positions: &[(PlayerPositionType, u8)],
    current_ability: u8,
    date: chrono::NaiveDate,
) -> Player {
    let mut player =
        PlayerGenerator::generate(1, date, positions[0].0, positions[0].1, &test_names());
    player.id = id;
    player.positions = crate::PlayerPositions {
        positions: positions
            .iter()
            .map(|(position, level)| PlayerPosition {
                position: *position,
                level: *level,
            })
            .collect(),
    };
    player.player_attributes.current_ability = current_ability;
    player.player_attributes.condition = 9500;
    player.player_attributes.fitness = 9000;
    player.player_attributes.days_since_last_match = 7;
    player
}
