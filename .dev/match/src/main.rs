use core::club::player::Player;
use core::club::player::PlayerPositionType;
use core::club::team::tactics::{MatchTacticType, Tactics};
use core::r#match::player::MatchPlayer;
use core::r#match::FootballEngine;
use core::r#match::MatchSquad;
use core::staff_contract_mod::NaiveDate;
use core::{PeopleNameGeneratorData, PlayerGenerator};
use axum::response::IntoResponse;
use flate2::write::GzEncoder;
use flate2::Compression;
use rand::Rng;
use rayon::prelude::*;
use serde::Serialize;
use std::io::Write;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};

/// Random squad level range when no explicit level is passed. Covers the
/// realistic spread from a lower-league squad (6) to an elite top-flight
/// team (18) — gives us a mix of matchups to stress-test balance across
/// skill gaps rather than always testing 14-vs-14 homogeneous squads.
const RANDOM_LEVEL_MIN: u8 = 6;
const RANDOM_LEVEL_MAX: u8 = 18;

fn random_level() -> u8 {
    rand::rng().random_range(RANDOM_LEVEL_MIN..=RANDOM_LEVEL_MAX)
}

const MATCH_ID: &str = "dev-match-001";
const LEAGUE_SLUG: &str = "dev";
const CHUNK_DURATION_MS: u64 = 300_000;

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

const LAST_NAMES: &[&str] = &[
    "Silva", "Martinez", "Müller", "Rossi", "Dupont",
    "Smith", "Johnson", "Garcia", "Fernandez", "Novak",
    "Petrov", "Andersson", "Tanaka", "Kim", "Santos",
    "Costa", "Richter", "Bernard", "Moretti", "Kowalski",
    "Ivanov", "Schmidt",
];

#[derive(Serialize)]
struct PlayerJson {
    id: u32,
    shirt_number: u8,
    last_name: String,
    position: String,
    is_home: bool,
}

#[derive(Serialize)]
struct GoalJson {
    player_id: u32,
    time: u64,
    is_auto_goal: bool,
}

#[derive(Serialize)]
struct MetadataJson {
    chunk_count: usize,
    chunk_duration_ms: u64,
    total_duration_ms: u64,
}

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

fn make_squad_simple(team_id: u32, level: u8) -> MatchSquad {
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

fn make_squad_viewer(
    team_id: u32,
    team_name: &str,
    level: u8,
    name_offset: usize,
) -> (MatchSquad, Vec<PlayerJson>) {
    let base_id = team_id * 100;
    let mut players_json = Vec::new();

    let main_squad: Vec<MatchPlayer> = POSITIONS_442
        .iter()
        .enumerate()
        .map(|(i, &pos)| {
            let player = generate_player(base_id + i as u32, pos, level);
            let mp = MatchPlayer::from_player(team_id, &player, pos, false);
            players_json.push(PlayerJson {
                id: mp.id,
                shirt_number: (i + 1) as u8,
                last_name: LAST_NAMES[(name_offset + i) % LAST_NAMES.len()].to_string(),
                position: pos.get_short_name().to_string(),
                is_home: team_id == 1,
            });
            mp
        })
        .collect();

    // Bench: one substitute per outfield position + spare keeper, so
    // fatigue-driven force-subs actually have someone to bring on. Without
    // this, mid-match subs would swap a field player for nobody and the
    // viewer's `PLAYERS_DATA` would be missing the sub-in entry (so their
    // sprite never gets created → "ball moving without player" effect).
    let sub_positions: [PlayerPositionType; 7] = [
        PlayerPositionType::Goalkeeper,
        PlayerPositionType::DefenderCenterLeft,
        PlayerPositionType::DefenderCenterRight,
        PlayerPositionType::MidfielderCenterLeft,
        PlayerPositionType::MidfielderCenterRight,
        PlayerPositionType::ForwardLeft,
        PlayerPositionType::ForwardRight,
    ];
    let substitutes: Vec<MatchPlayer> = sub_positions
        .iter()
        .enumerate()
        .map(|(i, &pos)| {
            let sub_id = base_id + 11 + i as u32;
            let player = generate_player(sub_id, pos, level);
            let mp = MatchPlayer::from_player(team_id, &player, pos, true);
            // Register the sub in PLAYERS_DATA too — that's the lookup the
            // viewer uses to build a sprite when a new id appears in
            // position chunks mid-match.
            players_json.push(PlayerJson {
                id: mp.id,
                shirt_number: (12 + i) as u8,
                last_name: LAST_NAMES[(name_offset + 11 + i) % LAST_NAMES.len()].to_string(),
                position: pos.get_short_name().to_string(),
                is_home: team_id == 1,
            });
            mp
        })
        .collect();

    let squad = MatchSquad {
        team_id,
        team_name: team_name.to_string(),
        tactics: Tactics::new(MatchTacticType::T442),
        main_squad,
        substitutes,
        captain_id: None,
        vice_captain_id: None,
        penalty_taker_id: None,
        free_kick_taker_id: None,
    };

    (squad, players_json)
}

struct TeamStats {
    shots: u16,
    on_target: u16,
    goals: u16,
    saves: u16,
    tackles: u16,
    fouls: u16,
    passes_attempted: u32,
    passes_completed: u32,
    interceptions: u32,
    xg: f32,
}

fn team_stats(result: &core::r#match::MatchResultRaw, team_id: u32) -> TeamStats {
    let squad = if result.left_team_players.team_id == team_id {
        &result.left_team_players
    } else {
        &result.right_team_players
    };
    let ids: Vec<u32> = squad.main.iter().chain(&squad.substitutes).copied().collect();
    let mut ts = TeamStats {
        shots: 0, on_target: 0, goals: 0, saves: 0, tackles: 0, fouls: 0,
        passes_attempted: 0, passes_completed: 0, interceptions: 0, xg: 0.0,
    };
    for id in ids {
        if let Some(s) = result.player_stats.get(&id) {
            ts.shots += s.shots_total;
            ts.on_target += s.shots_on_target;
            ts.goals += s.goals;
            ts.saves += s.saves;
            ts.tackles += s.tackles;
            ts.fouls += s.fouls;
            ts.passes_attempted += s.passes_attempted as u32;
            ts.passes_completed += s.passes_completed as u32;
            ts.interceptions += s.interceptions as u32;
            ts.xg += s.xg;
        }
    }
    ts
}

fn save_gzip_json(path: &PathBuf, data: &[u8]) {
    let file = std::fs::File::create(path)
        .unwrap_or_else(|e| panic!("failed to create {}: {}", path.display(), e));
    let mut encoder = GzEncoder::new(file, Compression::default());
    encoder.write_all(data).expect("failed to write gzip data");
    encoder.finish().expect("failed to finish gzip");
}

fn print_usage() {
    eprintln!("Usage:");
    eprintln!("  dev_match                       open browser viewer (random squad levels)");
    eprintln!("  dev_match viewer [lvlA] [lvlB]  open browser viewer — levels random unless given");
    eprintln!("  dev_match stats [N] [lvlA] [lvlB]  run N matches headless; per-match random levels");
    eprintln!("                                      unless BOTH lvlA and lvlB are passed");
    eprintln!();
    eprintln!("Random level range: {}–{} inclusive.", RANDOM_LEVEL_MIN, RANDOM_LEVEL_MAX);
    eprintln!("Viewer serves at http://localhost:18001");
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let mode = args.get(1).map(|s| s.as_str()).unwrap_or("viewer");

    match mode {
        "stats" => {
            let n_matches: usize = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(20);
            let level_a: Option<u8> = args.get(3).and_then(|s| s.parse().ok());
            let level_b: Option<u8> = args.get(4).and_then(|s| s.parse().ok());
            run_stats(n_matches, level_a, level_b);
        }
        "viewer" => {
            let level_a: Option<u8> = args.get(2).and_then(|s| s.parse().ok());
            let level_b: Option<u8> = args.get(3).and_then(|s| s.parse().ok());
            run_viewer(level_a, level_b);
        }
        "--help" | "-h" | "help" => {
            print_usage();
        }
        other => {
            // Legacy: `dev_match N [lvlA] [lvlB]` — first arg numeric treated as
            // stats N, so existing muscle memory keeps working.
            if let Ok(n) = other.parse::<usize>() {
                let level_a: Option<u8> = args.get(2).and_then(|s| s.parse().ok());
                let level_b: Option<u8> = args.get(3).and_then(|s| s.parse().ok());
                run_stats(n, level_a, level_b);
            } else {
                eprintln!("Unknown mode: {}\n", other);
                print_usage();
                std::process::exit(2);
            }
        }
    }
}

fn run_stats(n_matches: usize, level_a: Option<u8>, level_b: Option<u8>) {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("error"))
        .init();

    match (level_a, level_b) {
        (Some(a), Some(b)) => println!("Running {} matches: level {} vs level {}", n_matches, a, b),
        _ => println!(
            "Running {} matches: random squad levels per match ({}–{})",
            n_matches, RANDOM_LEVEL_MIN, RANDOM_LEVEL_MAX
        ),
    }
    println!();
    println!("{:>3} {:>3}v{:>3} {:>3}-{:>3} | {:>3}/{:>3} sh {:>3}/{:>3} ot {:>4}/{:>4} xG {:>3}/{:>3} sv {:>3}/{:>3} tk {:>3}/{:>3} int {:>4}/{:>4} pa {:>2}/{:>2}% acc",
             "#", "lA", "lB", "H", "A",
             "H", "A", "H", "A", "H", "A", "H", "A", "H", "A", "H", "A", "H", "A", "H", "A");

    let mut total_goals = 0u32;
    let mut total_shots = 0u32;
    let mut total_on_target = 0u32;
    let mut total_saves = 0u32;
    let mut total_tackles = 0u32;
    let mut total_interceptions = 0u32;
    let mut total_passes_attempted = 0u32;
    let mut total_passes_completed = 0u32;
    let mut total_fouls = 0u32;
    let mut total_xg = 0.0f32;
    let mut score_histogram: std::collections::BTreeMap<u8, u32> = std::collections::BTreeMap::new();

    let total_start = std::time::Instant::now();
    for i in 0..n_matches {
        let match_level_a = level_a.unwrap_or_else(random_level);
        let match_level_b = level_b.unwrap_or_else(random_level);
        let home = make_squad_simple(1, match_level_a);
        let away = make_squad_simple(2, match_level_b);
        let _start = std::time::Instant::now();
        let result = FootballEngine::<840, 545>::play(home, away, false, false, false);

        let score = result.score.as_ref().unwrap();
        let hg = score.home_team.get();
        let ag = score.away_team.get();
        let h = team_stats(&result, 1);
        let a = team_stats(&result, 2);

        let h_acc = if h.passes_attempted > 0 { h.passes_completed * 100 / h.passes_attempted } else { 0 };
        let a_acc = if a.passes_attempted > 0 { a.passes_completed * 100 / a.passes_attempted } else { 0 };

        println!("{:>3} {:>3}v{:>3} {:>3}-{:>3} | {:>3}/{:>3}    {:>3}/{:>3}    {:>4.1}/{:>4.1}    {:>3}/{:>3}    {:>3}/{:>3}    {:>3}/{:>3}     {:>4}/{:>4}  {:>2}/{:>2}%",
                 i + 1, match_level_a, match_level_b, hg, ag,
                 h.shots, a.shots, h.on_target, a.on_target,
                 h.xg, a.xg,
                 h.saves, a.saves, h.tackles, a.tackles, h.interceptions, a.interceptions,
                 h.passes_attempted, a.passes_attempted, h_acc, a_acc);

        total_goals += hg as u32 + ag as u32;
        total_shots += h.shots as u32 + a.shots as u32;
        total_on_target += h.on_target as u32 + a.on_target as u32;
        total_saves += h.saves as u32 + a.saves as u32;
        total_tackles += h.tackles as u32 + a.tackles as u32;
        total_interceptions += h.interceptions + a.interceptions;
        total_passes_attempted += h.passes_attempted + a.passes_attempted;
        total_passes_completed += h.passes_completed + a.passes_completed;
        total_fouls += h.fouls as u32 + a.fouls as u32;
        total_xg += h.xg + a.xg;
        *score_histogram.entry(hg + ag).or_default() += 1;
    }
    let total_ms = total_start.elapsed().as_millis();

    let n = n_matches as f32;
    println!();
    println!("--- AGGREGATE over {} matches ({} real-world seconds) ---", n_matches, total_ms / 1000);
    println!("goals per match     : {:.2}  (real ~2.5)", total_goals as f32 / n);
    println!("xG per team/match   : {:.2}  (real ~1.3)", total_xg / (2.0 * n));
    println!("goals vs xG delta   : {:+.2}  (real ~0.0)", total_goals as f32 / n - total_xg / n);
    println!("shots per team/match: {:.1}  (real ~13)", total_shots as f32 / (2.0 * n));
    let shots_per_xg = if total_xg > 0.1 { total_shots as f32 / total_xg } else { 0.0 };
    println!("shots per xG        : {:.1}   (real ~10; high = low-quality shots)", shots_per_xg);
    println!("on-target rate      : {:.1}%  (real ~33%)",
             total_on_target as f32 / total_shots.max(1) as f32 * 100.0);
    let conversion = total_goals as f32 / total_on_target.max(1) as f32 * 100.0;
    println!("on-target→goal rate : {:.1}%  (real ~30%)", conversion);
    let saves_vs_ontarget = total_saves as f32 / total_on_target.max(1) as f32 * 100.0;
    println!("saves/on-target     : {:.1}%  (real ~67%)", saves_vs_ontarget);
    println!("passes per team     : {:.0}  (real ~500)", total_passes_attempted as f32 / (2.0 * n));
    let pass_acc = if total_passes_attempted > 0 {
        total_passes_completed as f32 / total_passes_attempted as f32 * 100.0
    } else { 0.0 };
    println!("pass accuracy       : {:.1}%  (real ~85%)", pass_acc);
    println!("tackles per team    : {:.1}  (real ~18)", total_tackles as f32 / (2.0 * n));
    println!("interceptions/team  : {:.1}  (real ~10)", total_interceptions as f32 / (2.0 * n));
    println!("fouls per team      : {:.1}  (real ~12)", total_fouls as f32 / (2.0 * n));
    println!();
    println!("score total distribution (home+away goals per match):");
    for (total, count) in &score_histogram {
        let bar: String = std::iter::repeat('#').take(*count as usize).collect();
        println!("  {:>2}: {:>3} {}", total, count, bar);
    }
}

fn run_viewer(level_a: Option<u8>, level_b: Option<u8>) {
    // Route `log::warn!` from core (notably the ball-stall snapshot) to
    // stderr. Override with `RUST_LOG=info` or `RUST_LOG=debug` for more.
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("warn"))
        .format_timestamp_millis()
        .init();

    // Enable event+state tracking for dev viewer — required so the
    // position data the HTML viewer consumes gets collected.
    core::set_match_events_mode(true);

    let level_a = level_a.unwrap_or_else(random_level);
    let level_b = level_b.unwrap_or_else(random_level);

    let (home_squad, mut players_json) = make_squad_viewer(1, "Home FC", level_a, 0);
    let (away_squad, away_players) = make_squad_viewer(2, "Away United", level_b, 11);
    players_json.extend(away_players);

    println!("Play match... (level {} vs level {})", level_a, level_b);
    let start = std::time::Instant::now();

    let result = FootballEngine::<840, 545>::play(home_squad, away_squad, true, false, false);

    let elapsed = start.elapsed();

    let score = result.score.as_ref().unwrap();
    let home_goals = score.home_team.get();
    let away_goals = score.away_team.get();

    println!("Completed: {}:{}, {}ms", home_goals, away_goals, elapsed.as_millis());

    let goals_json: Vec<GoalJson> = score.detail().iter()
        .filter(|g| g.stat_type == core::r#match::player::statistics::MatchStatisticType::Goal)
        .map(|g| GoalJson {
            player_id: g.player_id,
            time: g.time,
            is_auto_goal: g.is_auto_goal,
        })
        .collect();

    let out_dir = PathBuf::from("match_results").join(LEAGUE_SLUG);
    std::fs::create_dir_all(&out_dir).expect("failed to create output dir");

    let chunks = result.position_data.split_into_chunks(CHUNK_DURATION_MS);
    let chunk_count = chunks.len();

    let save_start = std::time::Instant::now();
    let total_raw = AtomicUsize::new(0);
    let total_gz = AtomicUsize::new(0);

    chunks.par_iter().enumerate().for_each(|(idx, chunk)| {
        let chunk_data = serde_json::to_vec(chunk).expect("failed to serialize chunk");
        let raw_size = chunk_data.len();
        let chunk_path = out_dir.join(format!("{}_chunk_{}.json.gz", MATCH_ID, idx));
        save_gzip_json(&chunk_path, &chunk_data);
        let gz_size = std::fs::metadata(&chunk_path).map(|m| m.len() as usize).unwrap_or(0);

        total_raw.fetch_add(raw_size, Ordering::Relaxed);
        total_gz.fetch_add(gz_size, Ordering::Relaxed);
    });

    let raw = total_raw.load(Ordering::Relaxed) as f64;
    let gz = total_gz.load(Ordering::Relaxed) as f64;
    let ratio = if gz > 0.0 { raw / gz } else { 0.0 };
    println!(
        "Saved {} chunks in {}ms: {:.1}x compression ({:.0} MB -> {:.0} MB)",
        chunk_count,
        save_start.elapsed().as_millis(),
        ratio,
        raw / 1_048_576.0,
        gz / 1_048_576.0,
    );

    let metadata = MetadataJson {
        chunk_count,
        chunk_duration_ms: CHUNK_DURATION_MS,
        total_duration_ms: result.position_data.max_timestamp(),
    };
    let metadata_path = out_dir.join(format!("{}_metadata.json", MATCH_ID));
    std::fs::write(&metadata_path, serde_json::to_string_pretty(&metadata).unwrap())
        .expect("failed to write metadata");

    let page_data = format!(
        "const MATCH_ID=\"{}\";const MATCH_TIME_MS={};const GOALS_DATA={};const PLAYERS_DATA={};const HOME_BG=\"#00307d\";const HOME_FG=\"#ffffff\";const AWAY_BG=\"#b33f00\";const AWAY_FG=\"#ffffff\";const HOME_GOALS={};const AWAY_GOALS={};",
        MATCH_ID,
        result.match_time_ms,
        serde_json::to_string(&goals_json).unwrap(),
        serde_json::to_string(&players_json).unwrap(),
        home_goals,
        away_goals,
    );
    std::fs::write(out_dir.join("page_data.js"), &page_data).expect("failed to write page data");

    println!("\nStarting viewer at http://localhost:18001");

    #[cfg(target_os = "windows")]
    { let _ = std::process::Command::new("cmd").args(["/C", "start", "http://localhost:18001"]).spawn(); }
    #[cfg(target_os = "macos")]
    { let _ = std::process::Command::new("open").arg("http://localhost:18001").spawn(); }
    #[cfg(target_os = "linux")]
    { let _ = std::process::Command::new("xdg-open").arg("http://localhost:18001").spawn(); }

    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(serve());
}

async fn serve() {
    use axum::routing::get;

    let app = axum::Router::new()
        .route("/", get(page_handler))
        .route("/api/match/{match_id}/metadata", get(metadata_handler))
        .route("/api/match/{match_id}/chunk/{chunk_num}", get(chunk_handler))
        .route("/static/images/match/field.svg", get(field_svg_handler))
        .route("/js/pixi.min.js", get(pixi_handler))
        .route("/match_data.js", get(data_handler));

    let listener = tokio::net::TcpListener::bind("0.0.0.0:18001").await.unwrap();
    axum::serve(listener, app).await.unwrap();
}

async fn page_handler() -> axum::response::Html<String> {
    axum::response::Html(include_str!("viewer.html").to_string())
}

async fn data_handler() -> impl axum::response::IntoResponse {
    let path = PathBuf::from("match_results").join(LEAGUE_SLUG).join("page_data.js");
    let data = tokio::fs::read_to_string(&path).await.unwrap_or_default();
    ([(axum::http::header::CONTENT_TYPE, "application/javascript")], data)
}

async fn metadata_handler(
    axum::extract::Path(match_id): axum::extract::Path<String>,
) -> impl axum::response::IntoResponse {
    let path = PathBuf::from("match_results").join(LEAGUE_SLUG).join(format!("{}_metadata.json", match_id));
    match tokio::fs::read_to_string(&path).await {
        Ok(data) => (axum::http::StatusCode::OK, [(axum::http::header::CONTENT_TYPE, "application/json")], data).into_response(),
        Err(_) => (axum::http::StatusCode::NOT_FOUND, "not found").into_response(),
    }
}

async fn chunk_handler(
    axum::extract::Path((match_id, chunk_num)): axum::extract::Path<(String, usize)>,
) -> impl axum::response::IntoResponse {
    let path = PathBuf::from("match_results").join(LEAGUE_SLUG).join(format!("{}_chunk_{}.json.gz", match_id, chunk_num));
    match tokio::fs::read(&path).await {
        Ok(data) => (
            axum::http::StatusCode::OK,
            [
                (axum::http::header::CONTENT_TYPE, "application/gzip"),
                (axum::http::header::CONTENT_ENCODING, "gzip"),
            ],
            data,
        ).into_response(),
        Err(_) => (axum::http::StatusCode::NOT_FOUND, "not found").into_response(),
    }
}

async fn field_svg_handler() -> impl axum::response::IntoResponse {
    let svg = include_str!("../../../src/web/assets/static/images/match/field.svg");
    ([(axum::http::header::CONTENT_TYPE, "image/svg+xml")], svg)
}

async fn pixi_handler() -> impl axum::response::IntoResponse {
    let js = include_bytes!("../../../src/web/assets/static/js/pixi.min.js");
    ([(axum::http::header::CONTENT_TYPE, "application/javascript")], js.as_slice())
}
