use core::club::player::Player;
use core::club::player::PlayerPositionType;
use core::club::team::tactics::{MatchTacticType, Tactics};
use core::r#match::player::MatchPlayer;
use core::r#match::FootballEngine;
use core::r#match::MatchSquad;
use core::staff_contract_mod::NaiveDate;
use core::{PeopleNameGeneratorData, PlayerGenerator};

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

fn generate_player(id: u32, position: PlayerPositionType, level: u8) -> Player {
    let empty_names = PeopleNameGeneratorData {
        first_names: Vec::new(),
        last_names: Vec::new(),
        nicknames: Vec::new(),
    };
    let mut player = PlayerGenerator::generate(
        1,
        NaiveDate::from_ymd_opt(2000, 1, 1).unwrap(),
        position,
        level,
        &empty_names,
    );
    player.id = id;
    player
}

fn make_squad(team_id: u32, level: u8) -> MatchSquad {
    let base_id = team_id * 100;
    let main_squad: Vec<MatchPlayer> = POSITIONS_442
        .iter()
        .enumerate()
        .map(|(i, &pos)| {
            let player = generate_player(base_id + i as u32, pos, level);
            MatchPlayer::from_player(team_id, &player, pos, false)
        })
        .collect();

    MatchSquad {
        team_id,
        team_name: format!("Team {}", team_id),
        tactics: Tactics::new(MatchTacticType::T442),
        main_squad,
        substitutes: Vec::new(),
        captain_id: None,
        vice_captain_id: None,
        penalty_taker_id: None,
        free_kick_taker_id: None,
    }
}

fn team_stats(result: &core::r#match::MatchResultRaw, team_id: u32) -> (u16, u16, u16, u16, u16, u16) {
    let squad = if result.left_team_players.team_id == team_id {
        &result.left_team_players
    } else {
        &result.right_team_players
    };
    let ids: Vec<u32> = squad.main.iter().chain(&squad.substitutes).copied().collect();
    let mut shots = 0;
    let mut on_target = 0;
    let mut goals = 0;
    let mut saves = 0;
    let mut tackles = 0;
    let mut fouls = 0;
    for id in ids {
        if let Some(s) = result.player_stats.get(&id) {
            shots += s.shots_total;
            on_target += s.shots_on_target;
            goals += s.goals;
            saves += s.saves;
            tackles += s.tackles;
            fouls += s.fouls;
        }
    }
    (shots, on_target, goals, saves, tackles, fouls)
}

fn main() {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("error"))
        .init();

    let args: Vec<String> = std::env::args().collect();
    let n_matches: usize = args
        .get(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(20);
    let level_a: u8 = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(14);
    let level_b: u8 = args.get(3).and_then(|s| s.parse().ok()).unwrap_or(14);

    println!("Running {} matches: level {} vs level {}", n_matches, level_a, level_b);
    println!();
    println!("{:>4} {:>4}-{:>4}  {:>3}/{:>3} shots  {:>3}/{:>3} on-tgt  {:>3}/{:>3} saves  {:>3}/{:>3} tackles  time_ms",
             "#", "H", "A", "H", "A", "H", "A", "H", "A", "H", "A");

    let mut total_goals = 0u32;
    let mut total_shots = 0u32;
    let mut total_on_target = 0u32;
    let mut total_saves = 0u32;
    let mut total_tackles = 0u32;
    let mut score_histogram: std::collections::BTreeMap<u8, u32> = std::collections::BTreeMap::new();

    let total_start = std::time::Instant::now();
    for i in 0..n_matches {
        let home = make_squad(1, level_a);
        let away = make_squad(2, level_b);
        let start = std::time::Instant::now();
        let result = FootballEngine::<840, 545>::play(home, away, false, false, false);
        let ms = start.elapsed().as_millis();

        let score = result.score.as_ref().unwrap();
        let hg = score.home_team.get();
        let ag = score.away_team.get();
        let (hs, hot, _hg, hsv, ht, _) = team_stats(&result, 1);
        let (as_, aot, _ag, asv, at_, _) = team_stats(&result, 2);

        println!("{:>4} {:>4}-{:>4}  {:>3}/{:>3}          {:>3}/{:>3}           {:>3}/{:>3}         {:>3}/{:>3}         {}",
                 i + 1, hg, ag, hs, as_, hot, aot, hsv, asv, ht, at_, ms);

        total_goals += hg as u32 + ag as u32;
        total_shots += hs as u32 + as_ as u32;
        total_on_target += hot as u32 + aot as u32;
        total_saves += hsv as u32 + asv as u32;
        total_tackles += ht as u32 + at_ as u32;
        *score_histogram.entry(hg + ag).or_default() += 1;
    }
    let total_ms = total_start.elapsed().as_millis();

    println!();
    println!("--- AGGREGATE over {} matches ({} real-world seconds) ---", n_matches, total_ms / 1000);
    println!("goals per match     : {:.2}  (real ~2.5)", total_goals as f32 / n_matches as f32);
    println!("shots per team/match: {:.1}  (real ~13)", total_shots as f32 / (2.0 * n_matches as f32));
    println!("on-target rate      : {:.1}%  (real ~33%)",
             total_on_target as f32 / total_shots.max(1) as f32 * 100.0);
    let conversion = total_goals as f32 / total_on_target.max(1) as f32 * 100.0;
    println!("on-target→goal rate : {:.1}%  (real ~30%)", conversion);
    let saves_vs_ontarget = total_saves as f32 / total_on_target.max(1) as f32 * 100.0;
    println!("saves/on-target     : {:.1}%  (real ~67%)", saves_vs_ontarget);
    println!("tackles per team    : {:.1}", total_tackles as f32 / (2.0 * n_matches as f32));
    println!();
    println!("score total distribution (home+away goals per match):");
    for (total, count) in &score_histogram {
        let bar: String = std::iter::repeat('#').take(*count as usize).collect();
        println!("  {:>2}: {:>3} {}", total, count, bar);
    }
}
