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
use rayon::prelude::*;
use serde::Serialize;
use std::io::Write;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};

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

fn make_squad(
    team_id: u32,
    team_name: &str,
    level: u8,
    positions: &[PlayerPositionType; 11],
    name_offset: usize,
) -> (MatchSquad, Vec<PlayerJson>) {
    let base_id = team_id * 100;
    let mut players_json = Vec::new();

    let main_squad: Vec<MatchPlayer> = positions
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

    let squad = MatchSquad {
        team_id,
        team_name: team_name.to_string(),
        tactics: Tactics::new(MatchTacticType::T442),
        main_squad,
        substitutes: Vec::new(),
        captain_id: None,
        vice_captain_id: None,
        penalty_taker_id: None,
        free_kick_taker_id: None,
    };

    (squad, players_json)
}

fn save_gzip_json(path: &PathBuf, data: &[u8]) {
    let file = std::fs::File::create(path)
        .unwrap_or_else(|e| panic!("failed to create {}: {}", path.display(), e));
    // Default (level 6) over best (9): ~3–5× faster with <2% size difference
    // on already-compact position JSON. Dev iteration favours speed.
    let mut encoder = GzEncoder::new(file, Compression::default());
    encoder.write_all(data).expect("failed to write gzip data");
    encoder.finish().expect("failed to finish gzip");
}

fn main() {
    // Route `log::warn!` from core (notably the ball-stall snapshot in
    // `ball.rs`) to stderr. Default filter `warn` surfaces diagnostics
    // without drowning the terminal in per-tick debug output. Override
    // with `RUST_LOG=info` or `RUST_LOG=debug` for more verbosity.
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("warn"))
        .format_timestamp_millis()
        .init();

    // Enable event+state tracking for dev viewer
    core::set_match_events_mode(true);

    let (home_squad, mut players_json) = make_squad(1, "Home FC", 14, &POSITIONS_442, 0);
    let (away_squad, away_players) = make_squad(2, "Away United", 14, &POSITIONS_442, 11);
    players_json.extend(away_players);

    println!("Play match...");
    let start = std::time::Instant::now();

    let result = FootballEngine::<840, 545>::play(home_squad, away_squad, true, false, false);

    let elapsed = start.elapsed();

    let score = result.score.as_ref().unwrap();
    let home_goals = score.home_team.get();
    let away_goals = score.away_team.get();

    println!("Completed: {}:{}, {}ms", home_goals, away_goals, elapsed.as_millis());

    // Collect goal events
    let goals_json: Vec<GoalJson> = score.detail().iter()
        .filter(|g| g.stat_type == core::r#match::player::statistics::MatchStatisticType::Goal)
        .map(|g| GoalJson {
            player_id: g.player_id,
            time: g.time,
            is_auto_goal: g.is_auto_goal,
        })
        .collect();

    // Split into chunks
    let out_dir = PathBuf::from("match_results").join(LEAGUE_SLUG);
    std::fs::create_dir_all(&out_dir).expect("failed to create output dir");

    let chunks = result.position_data.split_into_chunks(CHUNK_DURATION_MS);
    let chunk_count = chunks.len();

    // Save chunks in parallel. No per-chunk progress print: `\r` updates
    // from rayon threads interleave non-monotonically and mangle the
    // terminal. One line after the join is enough — the save step is
    // seconds long anyway.
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

    // Save metadata
    let metadata = MetadataJson {
        chunk_count,
        chunk_duration_ms: CHUNK_DURATION_MS,
        total_duration_ms: result.position_data.max_timestamp(),
    };
    let metadata_path = out_dir.join(format!("{}_metadata.json", MATCH_ID));
    std::fs::write(&metadata_path, serde_json::to_string_pretty(&metadata).unwrap())
        .expect("failed to write metadata");

    // Generate page data
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
