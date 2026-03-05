use core::club::player::Player;
use core::club::player::PlayerPositionType;
use core::club::team::tactics::{MatchTacticType, Tactics};
use core::r#match::player::MatchPlayer;
use core::r#match::FootballEngine;
use core::r#match::MatchSquad;
use core::staff_contract_mod::NaiveDate;
use core::PlayerGenerator;
use rayon::prelude::*;

const POSITIONS_442: [PlayerPositionType; 11] = [
    PlayerPositionType::Goalkeeper,
    PlayerPositionType::DefenderLeft,
    PlayerPositionType::DefenderCenterLeft,
    PlayerPositionType::DefenderCenterRight,
    PlayerPositionType::DefenderRight,
    PlayerPositionType::MidfielderLeft,
    PlayerPositionType::MidfielderCenterLeft,
    PlayerPositionType::MidfielderCenterRight,
    PlayerPositionType::MidfielderRight,
    PlayerPositionType::ForwardLeft,
    PlayerPositionType::ForwardRight,
];

const POSITIONS_433: [PlayerPositionType; 11] = [
    PlayerPositionType::Goalkeeper,
    PlayerPositionType::DefenderLeft,
    PlayerPositionType::DefenderCenterLeft,
    PlayerPositionType::DefenderCenterRight,
    PlayerPositionType::DefenderRight,
    PlayerPositionType::MidfielderCenterLeft,
    PlayerPositionType::MidfielderCenter,
    PlayerPositionType::MidfielderCenterRight,
    PlayerPositionType::ForwardLeft,
    PlayerPositionType::ForwardCenter,
    PlayerPositionType::ForwardRight,
];

const POSITIONS_352: [PlayerPositionType; 11] = [
    PlayerPositionType::Goalkeeper,
    PlayerPositionType::DefenderCenterLeft,
    PlayerPositionType::DefenderCenter,
    PlayerPositionType::DefenderCenterRight,
    PlayerPositionType::WingbackLeft,
    PlayerPositionType::MidfielderCenterLeft,
    PlayerPositionType::MidfielderCenter,
    PlayerPositionType::MidfielderCenterRight,
    PlayerPositionType::WingbackRight,
    PlayerPositionType::ForwardLeft,
    PlayerPositionType::ForwardRight,
];

struct MatchSetup {
    name: &'static str,
    home_name: &'static str,
    home_level: u8,
    home_tactic: MatchTacticType,
    home_positions: &'static [PlayerPositionType; 11],
    away_name: &'static str,
    away_level: u8,
    away_tactic: MatchTacticType,
    away_positions: &'static [PlayerPositionType; 11],
}

fn generate_player(id: u32, position: PlayerPositionType, level: u8) -> Player {
    let mut player = PlayerGenerator::generate(
        1,
        NaiveDate::from_ymd_opt(2023, 1, 1).unwrap(),
        position,
        level,
        None,
    );
    player.id = id;
    player
}

fn make_squad(
    team_id: u32,
    team_name: &str,
    level: u8,
    tactic_type: MatchTacticType,
    positions: &[PlayerPositionType; 11],
) -> MatchSquad {
    let base_id = team_id * 100;
    let main_squad: Vec<MatchPlayer> = positions
        .iter()
        .enumerate()
        .map(|(i, &pos)| {
            let player = generate_player(base_id + i as u32, pos, level);
            MatchPlayer::from_player(team_id, &player, pos, false)
        })
        .collect();

    MatchSquad {
        team_id,
        team_name: team_name.to_string(),
        tactics: Tactics::new(tactic_type),
        main_squad,
        substitutes: Vec::new(),
        captain_id: None,
        vice_captain_id: None,
        penalty_taker_id: None,
        free_kick_taker_id: None,
    }
}

struct MatchResult {
    name: &'static str,
    home_name: &'static str,
    away_name: &'static str,
    home_goals: u8,
    away_goals: u8,
    elapsed_ms: u128,
}

fn play_match(setup: &MatchSetup) -> MatchResult {
    let home = make_squad(1, setup.home_name, setup.home_level, setup.home_tactic, setup.home_positions);
    let away = make_squad(2, setup.away_name, setup.away_level, setup.away_tactic, setup.away_positions);

    let start = std::time::Instant::now();
    let result = FootballEngine::<840, 545>::play(home, away, false, true);
    let elapsed = start.elapsed();

    let score = result.score.as_ref().unwrap();

    MatchResult {
        name: setup.name,
        home_name: setup.home_name,
        away_name: setup.away_name,
        home_goals: score.home_team.get(),
        away_goals: score.away_team.get(),
        elapsed_ms: elapsed.as_millis(),
    }
}

fn main() {
    let matches = [
        MatchSetup {
            name: "Medium vs Medium (442 vs 442)",
            home_name: "Team A",
            home_level: 12,
            home_tactic: MatchTacticType::T442,
            home_positions: &POSITIONS_442,
            away_name: "Team B",
            away_level: 12,
            away_tactic: MatchTacticType::T442,
            away_positions: &POSITIONS_442,
        },
    ];

    println!("{:<40} {}", "SCENARIO", "RESULT");
    println!("{}", "-".repeat(80));

    let total_start = std::time::Instant::now();

    let results: Vec<MatchResult> = matches.par_iter().map(|setup| play_match(setup)).collect();

    let total_elapsed = total_start.elapsed();

    for r in &results {
        println!(
            "{:<40} {} {} - {} {}  ({} ms)",
            r.name, r.home_name, r.home_goals, r.away_goals, r.away_name, r.elapsed_ms,
        );
    }

    println!("{}", "-".repeat(80));
    println!("Total: {} ms (parallel)", total_elapsed.as_millis());
}
