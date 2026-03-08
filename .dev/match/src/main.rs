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
    name: String,
    home_name: &'static str,
    away_name: &'static str,
    home_goals: u8,
    away_goals: u8,
    elapsed_ms: u128,
}

fn play_match(setup: &MatchSetup, match_recordings: bool) -> MatchResult {
    let home = make_squad(1, setup.home_name, setup.home_level, setup.home_tactic, setup.home_positions);
    let away = make_squad(2, setup.away_name, setup.away_level, setup.away_tactic, setup.away_positions);

    let start = std::time::Instant::now();
    let result = FootballEngine::<840, 545>::play(home, away, match_recordings, true);
    let elapsed = start.elapsed();

    let score = result.score.as_ref().unwrap();

    MatchResult {
        name: setup.name.to_string(),
        home_name: setup.home_name,
        away_name: setup.away_name,
        home_goals: score.home_team.get(),
        away_goals: score.away_team.get(),
        elapsed_ms: elapsed.as_millis(),
    }
}

fn get_process_memory_mb() -> f64 {
    #[cfg(target_os = "windows")]
    {
        use std::mem::MaybeUninit;

        #[repr(C)]
        #[allow(non_snake_case)]
        struct PROCESS_MEMORY_COUNTERS {
            cb: u32,
            PageFaultCount: u32,
            PeakWorkingSetSize: usize,
            WorkingSetSize: usize,
            QuotaPeakPagedPoolUsage: usize,
            QuotaPagedPoolUsage: usize,
            QuotaPeakNonPagedPoolUsage: usize,
            QuotaNonPagedPoolUsage: usize,
            PagefileUsage: usize,
            PeakPagefileUsage: usize,
        }

        unsafe extern "system" {
            fn GetCurrentProcess() -> *mut std::ffi::c_void;
            fn K32GetProcessMemoryInfo(
                process: *mut std::ffi::c_void,
                ppsmemCounters: *mut PROCESS_MEMORY_COUNTERS,
                cb: u32,
            ) -> i32;
        }

        unsafe {
            let mut pmc = MaybeUninit::<PROCESS_MEMORY_COUNTERS>::zeroed().assume_init();
            pmc.cb = std::mem::size_of::<PROCESS_MEMORY_COUNTERS>() as u32;
            let process = GetCurrentProcess();
            if K32GetProcessMemoryInfo(process, &mut pmc, pmc.cb) != 0 {
                pmc.WorkingSetSize as f64 / (1024.0 * 1024.0)
            } else {
                0.0
            }
        }
    }

    #[cfg(not(target_os = "windows"))]
    {
        0.0
    }
}

fn get_peak_memory_mb() -> f64 {
    #[cfg(target_os = "windows")]
    {
        use std::mem::MaybeUninit;

        #[repr(C)]
        #[allow(non_snake_case)]
        struct PROCESS_MEMORY_COUNTERS {
            cb: u32,
            PageFaultCount: u32,
            PeakWorkingSetSize: usize,
            WorkingSetSize: usize,
            QuotaPeakPagedPoolUsage: usize,
            QuotaPagedPoolUsage: usize,
            QuotaPeakNonPagedPoolUsage: usize,
            QuotaNonPagedPoolUsage: usize,
            PagefileUsage: usize,
            PeakPagefileUsage: usize,
        }

        unsafe extern "system" {
            fn GetCurrentProcess() -> *mut std::ffi::c_void;
            fn K32GetProcessMemoryInfo(
                process: *mut std::ffi::c_void,
                ppsmemCounters: *mut PROCESS_MEMORY_COUNTERS,
                cb: u32,
            ) -> i32;
        }

        unsafe {
            let mut pmc = MaybeUninit::<PROCESS_MEMORY_COUNTERS>::zeroed().assume_init();
            pmc.cb = std::mem::size_of::<PROCESS_MEMORY_COUNTERS>() as u32;
            let process = GetCurrentProcess();
            if K32GetProcessMemoryInfo(process, &mut pmc, pmc.cb) != 0 {
                pmc.PeakWorkingSetSize as f64 / (1024.0 * 1024.0)
            } else {
                0.0
            }
        }
    }

    #[cfg(not(target_os = "windows"))]
    {
        0.0
    }
}

fn print_struct_sizes() {
    use core::r#match::ball::Ball;
    use core::r#match::result::ResultMatchPositionData;
    use core::r#match::engine::field::MatchField;
    use core::r#match::engine::context::MatchContext;
    use core::r#match::engine::result::MatchResultRaw;
    use core::r#match::PlayerDistanceClosure;
    use core::r#match::engine::player::positions::closure::PlayerDistanceItem;
    use core::r#match::engine::raycast::space::{Space, SphereCollider};
    use core::r#match::engine::player::context::GameTickContext;
    use core::r#match::result::ResultPositionDataItem;
    use core::club::player::skills::PlayerSkills;
    use core::club::player::attributes::PlayerAttributes;
    use core::club::person::PersonAttributes;

    println!("=== STRUCT SIZES (stack only) ===");
    println!("  MatchPlayer:              {} bytes", std::mem::size_of::<MatchPlayer>());
    println!("  PlayerSkills:             {} bytes", std::mem::size_of::<PlayerSkills>());
    println!("  PlayerAttributes:         {} bytes", std::mem::size_of::<PlayerAttributes>());
    println!("  PersonAttributes:         {} bytes", std::mem::size_of::<PersonAttributes>());
    println!("  Ball:                     {} bytes", std::mem::size_of::<Ball>());
    println!("  MatchField:               {} bytes", std::mem::size_of::<MatchField>());
    println!("  MatchContext:             {} bytes", std::mem::size_of::<MatchContext>());
    println!("  MatchResultRaw:           {} bytes", std::mem::size_of::<MatchResultRaw>());
    println!("  ResultMatchPositionData:  {} bytes", std::mem::size_of::<ResultMatchPositionData>());
    println!("  ResultPositionDataItem:   {} bytes", std::mem::size_of::<ResultPositionDataItem>());
    println!("  PlayerDistanceClosure:    {} bytes", std::mem::size_of::<PlayerDistanceClosure>());
    println!("  PlayerDistanceItem:       {} bytes", std::mem::size_of::<PlayerDistanceItem>());
    println!("  SphereCollider:           {} bytes", std::mem::size_of::<SphereCollider>());
    println!("  Space:                    {} bytes", std::mem::size_of::<Space>());
    println!("  GameTickContext:          {} bytes", std::mem::size_of::<GameTickContext>());
    println!();

    // Calculate per-match position data estimate
    // 45 min * 60 sec * 1000ms / 10ms tick = 270,000 ticks per half, 540,000 total
    let ticks_per_match: u64 = 540_000;
    let item_size = std::mem::size_of::<ResultPositionDataItem>() as u64;
    let players = 22u64;

    let position_data_per_match_mb = (ticks_per_match * (players + 1) * item_size) as f64 / (1024.0 * 1024.0);
    println!("=== POSITION DATA ESTIMATE (match_recordings=true) ===");
    println!("  Ticks per match:          {}", ticks_per_match);
    println!("  ResultPositionDataItem:   {} bytes", item_size);
    println!("  Entities tracked:         {} (22 players + ball)", players + 1);
    println!("  Max position data/match:  {:.1} MB", position_data_per_match_mb);
    println!("  x10 parallel matches:     {:.1} MB", position_data_per_match_mb * 10.0);
    println!("  x50 parallel matches:     {:.1} GB", position_data_per_match_mb * 50.0 / 1024.0);
    println!("  x100 parallel matches:    {:.1} GB", position_data_per_match_mb * 100.0 / 1024.0);
    println!("  x200 parallel matches:    {:.1} GB", position_data_per_match_mb * 200.0 / 1024.0);
    println!();

    // Distance closure per recalculation
    let n = 22usize;
    let dist_items = n * (n - 1);
    let dist_item_size = std::mem::size_of::<PlayerDistanceItem>();
    println!("=== DISTANCE CLOSURE ===");
    println!("  Items per closure:        {} ({}x{})", dist_items, n, n-1);
    println!("  Closure size:             {} bytes", dist_items * dist_item_size);
    println!();

    // Space per tick (clones all MatchPlayers)
    let collider_size = std::mem::size_of::<SphereCollider>();
    let match_player_size = std::mem::size_of::<MatchPlayer>();
    println!("=== SPACE (per tick allocation) ===");
    println!("  SphereCollider size:      {} bytes (contains Option<MatchPlayer>)", collider_size);
    println!("  23 colliders/tick:        {} bytes (stack only, + heap for MatchPlayer Vecs)", 23 * collider_size);
    println!("  MatchPlayer stack size:   {} bytes", match_player_size);
    println!("  22 MatchPlayer clones:    {} bytes (stack only)", 22 * match_player_size);
    println!();
}

fn main() {
    print_struct_sizes();

    let num_matches: usize = std::env::args()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(10);

    let match_recordings: bool = std::env::args()
        .nth(2)
        .map(|s| s == "true" || s == "1")
        .unwrap_or(false);

    println!("=== MEMORY TEST: {} parallel matches, recordings={} ===", num_matches, match_recordings);
    println!();

    // Generate match setups
    let setups: Vec<MatchSetup> = (0..num_matches).map(|_| {
        MatchSetup {
            name: "442 vs 442",
            home_name: "Home",
            home_level: 12,
            home_tactic: MatchTacticType::T442,
            home_positions: &POSITIONS_442,
            away_name: "Away",
            away_level: 12,
            away_tactic: MatchTacticType::T442,
            away_positions: &POSITIONS_442,
        }
    }).collect();

    let mem_before = get_process_memory_mb();
    println!("Memory before:  {:.1} MB", mem_before);

    let total_start = std::time::Instant::now();

    let results: Vec<MatchResult> = setups.par_iter().map(|setup| play_match(setup, match_recordings)).collect();

    let total_elapsed = total_start.elapsed();
    let mem_after = get_process_memory_mb();
    let mem_peak = get_peak_memory_mb();

    println!();
    println!("{:<40} {}", "SCENARIO", "RESULT");
    println!("{}", "-".repeat(80));

    for r in &results {
        println!(
            "{:<40} {} {} - {} {}  ({} ms)",
            r.name, r.home_name, r.home_goals, r.away_goals, r.away_name, r.elapsed_ms,
        );
    }

    println!("{}", "-".repeat(80));
    println!("Total: {} ms (parallel, {} matches)", total_elapsed.as_millis(), num_matches);
    println!();
    println!("=== MEMORY SUMMARY ===");
    println!("  Before matches:   {:.1} MB", mem_before);
    println!("  After matches:    {:.1} MB", mem_after);
    println!("  Peak memory:      {:.1} MB", mem_peak);
    println!("  Delta:            {:.1} MB", mem_after - mem_before);
    println!("  Per match (avg):  {:.1} MB", (mem_after - mem_before) / num_matches as f64);
}
