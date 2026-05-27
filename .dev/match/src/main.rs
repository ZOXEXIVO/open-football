use axum::response::IntoResponse;
use core::club::player::Player;
use core::club::player::PlayerPositionType;
use core::club::team::tactics::{MatchTacticType, Tactics};
use core::r#match::FootballEngine;
use core::r#match::MatchSquad;
use core::r#match::player::MatchPlayer;
use core::staff_contract_mod::NaiveDate;
use core::{MatchRuntime, PeopleNameGeneratorData, PlayerGenerator};
use flate2::Compression;
use flate2::write::GzEncoder;
use rand::RngExt;
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
    "Silva",
    "Martinez",
    "Müller",
    "Rossi",
    "Dupont",
    "Smith",
    "Johnson",
    "Garcia",
    "Fernandez",
    "Novak",
    "Petrov",
    "Andersson",
    "Tanaka",
    "Kim",
    "Santos",
    "Costa",
    "Richter",
    "Bernard",
    "Moretti",
    "Kowalski",
    "Ivanov",
    "Schmidt",
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
    // STAR_HOG=1 reproduces a lone-striker shape: one elite forward
    // (ForwardLeft, +5 levels) alongside a much weaker partner
    // (ForwardRight, -4). This mimics a team built around a single
    // focal striker — the scenario that produces the league's 50+ goal
    // top scorers — which the uniform 442 squad otherwise hides.
    let star_hog = std::env::var("STAR_HOG").ok().as_deref() == Some("1");
    // PLAYMAKER injects an elite central midfielder (MidfielderCenterLeft)
    // so the redesign can be measured — uniform squads otherwise can't show
    // whether attacking skill drives an MC's goals.
    //   PLAYMAKER=1 → box-to-box / advanced playmaker (elite off-the-ball,
    //     finishing, long-shots, technique): should project ~10-15/season.
    //   PLAYMAKER=2 → deep regista (elite passing/vision/composure but
    //     low off-the-ball/finishing): should stay ~2-5/season — proving
    //     the model rewards the ATTACKING profile, not midfielders blanket.
    let playmaker = std::env::var("PLAYMAKER")
        .ok()
        .and_then(|v| v.parse::<u8>().ok());
    let main_squad: Vec<MatchPlayer> = POSITIONS_442
        .iter()
        .enumerate()
        .map(|(i, &pos)| {
            let lvl = if star_hog && pos == PlayerPositionType::ForwardLeft {
                (level + 5).min(20)
            } else if star_hog && pos == PlayerPositionType::ForwardRight {
                level.saturating_sub(4).max(1)
            } else {
                level
            };
            let mut player = generate_player(base_id + i as u32, pos, lvl);
            if pos == PlayerPositionType::MidfielderCenterLeft {
                let s = &mut player.skills;
                match playmaker {
                    Some(1) => {
                        // Advanced / box-to-box playmaker.
                        s.technical.finishing = 17.0;
                        s.technical.long_shots = 17.0;
                        s.technical.technique = 17.0;
                        s.technical.dribbling = 16.0;
                        s.technical.passing = 16.0;
                        s.mental.off_the_ball = 18.0;
                        s.mental.composure = 17.0;
                        s.mental.decisions = 16.0;
                        s.mental.vision = 16.0;
                        s.mental.work_rate = 16.0;
                        s.physical.acceleration = 15.0;
                        s.physical.pace = 15.0;
                        s.physical.stamina = 16.0;
                    }
                    Some(2) => {
                        // Deep regista — creates, doesn't finish.
                        s.technical.passing = 18.0;
                        s.technical.technique = 17.0;
                        s.mental.vision = 18.0;
                        s.mental.composure = 17.0;
                        s.mental.decisions = 17.0;
                        s.technical.finishing = 7.0;
                        s.technical.long_shots = 8.0;
                        s.mental.off_the_ball = 7.0;
                        s.mental.work_rate = 8.0;
                    }
                    _ => {}
                }
            }
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
        selection_omissions: Vec::new(),
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
        selection_omissions: Vec::new(),
    };

    (squad, players_json)
}

#[derive(Clone)]
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

/// One match's row of output and aggregates. Produced inside the
/// rayon parallel loop so the only synchronisation point is the
/// global atomic counters inside `core` (shot/tackle/save accounting),
/// which are already lock-free.
#[derive(Clone)]
struct MatchOutcome {
    idx: usize,
    level_a: u8,
    level_b: u8,
    home_goals: u8,
    away_goals: u8,
    home: TeamStats,
    away: TeamStats,
    /// Per-player rows for this match: (player_id, goals, shots, xg, pos_group).
    /// pos_group: 0=GK 1=DEF 2=MID 3=FWD (derived from the 442 id slot).
    /// Used to measure per-player concentration AND per-line goal share.
    per_player: Vec<(u32, u16, u16, f32, u8)>,
}

/// Position group for a player id, using the deterministic 442 slot
/// scheme in make_squad_simple (base_id = team_id*100):
/// 0 GK, 1-4 DEF, 5-8 MID, 9-10 FWD. Stats runs have no substitutes so
/// every id maps cleanly to 0..=10. This is the lens for the GOALS BY
/// LINE diagnostic — the share of goals scored by each positional line,
/// which is what "defenders/midfielders rarely score" is measured against.
fn pos_group_of(id: u32) -> u8 {
    match id % 100 {
        0 => 0,     // GK
        1..=4 => 1, // DEF
        5..=8 => 2, // MID
        _ => 3,     // FWD (9, 10)
    }
}

/// Collect per-player (id, goals, shots, xg, pos_group) rows for both teams.
fn per_player_rows(result: &core::r#match::MatchResultRaw) -> Vec<(u32, u16, u16, f32, u8)> {
    let mut rows = Vec::new();
    for (id, s) in result.player_stats.iter() {
        rows.push((*id, s.goals, s.shots_total, s.xg, pos_group_of(*id)));
    }
    rows
}

fn team_stats(result: &core::r#match::MatchResultRaw, team_id: u32) -> TeamStats {
    let squad = if result.left_team_players.team_id == team_id {
        &result.left_team_players
    } else {
        &result.right_team_players
    };
    let ids: Vec<u32> = squad
        .main
        .iter()
        .chain(&squad.substitutes)
        .copied()
        .collect();
    let mut ts = TeamStats {
        shots: 0,
        on_target: 0,
        goals: 0,
        saves: 0,
        tackles: 0,
        fouls: 0,
        passes_attempted: 0,
        passes_completed: 0,
        interceptions: 0,
        xg: 0.0,
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

// ───────────────────────────────────────────────────────────────────────────
// League season harness — `dev_match league [teams] [rounds] [minLvl] [maxLvl]`
//
// Plays a full round-robin season with clubs spread across a strength range,
// so the season includes genuine strong-vs-weak mismatches. Reports the
// SEASON-LONG top-scorer table (the headline: does the top scorer settle at a
// realistic ~25-30, or inflate?), the league table, and the goals-by-line
// split. Goals include any penalties / set-pieces the engine produced in play
// — the paths a 5-game snapshot can't separate from open-play variance.
// ───────────────────────────────────────────────────────────────────────────

/// Club names for league output flavour (indexed by team slot).
const CLUB_NAMES: &[&str] = &[
    "Inter",
    "Milan",
    "Juventus",
    "Napoli",
    "Roma",
    "Lazio",
    "Atalanta",
    "Fiorentina",
    "Bologna",
    "Torino",
    "Como",
    "Genoa",
    "Udinese",
    "Cagliari",
    "Empoli",
    "Lecce",
    "Verona",
    "Parma",
    "Cremonese",
    "Monza",
    "Sassuolo",
    "Salernitana",
    "Frosinone",
    "Spezia",
];

/// One league club, built ONCE so every player keeps fixed skills across the
/// whole season (regenerating per match would scramble identities and apps).
struct LeagueTeam {
    id: u32,
    name: String,
    level: u8,
    players: Vec<MatchPlayer>,
}

fn build_league_team(id: u32, name: &str, level: u8) -> LeagueTeam {
    let base_id = id * 100;
    let players = POSITIONS_442
        .iter()
        .enumerate()
        .map(|(i, &pos)| {
            let player = generate_player(base_id + i as u32, pos, level);
            MatchPlayer::from_player(id, &player, pos, false)
        })
        .collect();
    LeagueTeam {
        id,
        name: name.to_string(),
        level,
        players,
    }
}

fn league_squad(t: &LeagueTeam) -> MatchSquad {
    MatchSquad {
        team_id: t.id,
        team_name: t.name.clone(),
        tactics: Tactics::new(MatchTacticType::T442),
        main_squad: t.players.clone(),
        substitutes: Vec::new(),
        captain_id: None,
        vice_captain_id: None,
        penalty_taker_id: None,
        free_kick_taker_id: None,
        selection_omissions: Vec::new(),
    }
}

struct LeagueMatch {
    home_idx: usize,
    away_idx: usize,
    home_goals: u8,
    away_goals: u8,
    per_player: Vec<(u32, u16, u16, f32, u8)>,
}

#[derive(Clone, Default)]
struct TableRow {
    played: u32,
    w: u32,
    d: u32,
    l: u32,
    gf: u32,
    ga: u32,
}
impl TableRow {
    fn pts(&self) -> u32 {
        self.w * 3 + self.d
    }
    fn gd(&self) -> i32 {
        self.gf as i32 - self.ga as i32
    }
}

fn run_league(n_teams: usize, rounds: usize, min_lvl: u8, max_lvl: u8) {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("error")).init();

    let n_teams = n_teams.clamp(2, CLUB_NAMES.len());
    let rounds = rounds.clamp(1, 2);
    let (min_lvl, max_lvl) = (min_lvl.min(max_lvl), min_lvl.max(max_lvl));
    let n_threads = rayon::current_num_threads();
    println!(
        "League season: {} teams, {} round(s), club levels {}–{} spread  (parallel: {} threads)",
        n_teams, rounds, min_lvl, max_lvl, n_threads
    );

    // Build clubs with a strength spread so the season has real mismatches.
    let teams: Vec<LeagueTeam> = (0..n_teams)
        .map(|i| {
            let level = if n_teams <= 1 {
                max_lvl
            } else {
                (min_lvl as f32 + (max_lvl - min_lvl) as f32 * (i as f32 / (n_teams - 1) as f32))
                    .round() as u8
            };
            build_league_team((i + 1) as u32, CLUB_NAMES[i], level)
        })
        .collect();

    // Round-robin fixtures (double = home + away, like a real 38-game season).
    let mut fixtures: Vec<(usize, usize)> = Vec::new();
    for a in 0..n_teams {
        for b in (a + 1)..n_teams {
            fixtures.push((a, b));
            if rounds >= 2 {
                fixtures.push((b, a));
            }
        }
    }
    let apps_per_player = ((n_teams - 1) * rounds) as u32;

    let start = std::time::Instant::now();
    let played: Vec<LeagueMatch> = fixtures
        .par_iter()
        .map(|&(h, a)| {
            let home = league_squad(&teams[h]);
            let away = league_squad(&teams[a]);
            let result = FootballEngine::<840, 545>::play(home, away, false, false, false);
            let score = result.score.as_ref().unwrap();
            LeagueMatch {
                home_idx: h,
                away_idx: a,
                home_goals: score.home_team.get(),
                away_goals: score.away_team.get(),
                per_player: per_player_rows(&result),
            }
        })
        .collect();
    let secs = start.elapsed().as_secs();

    // Aggregate the table, per-player tallies, and goals-by-line.
    let mut table = vec![TableRow::default(); n_teams];
    let mut agg: std::collections::HashMap<u32, (u32, u32, f32, u32, u8)> =
        std::collections::HashMap::new();
    let mut group_goals = [0u32; 4];
    let mut total_goals = 0u32;
    for m in &played {
        let (hg, ag) = (m.home_goals as u32, m.away_goals as u32);
        table[m.home_idx].played += 1;
        table[m.away_idx].played += 1;
        table[m.home_idx].gf += hg;
        table[m.home_idx].ga += ag;
        table[m.away_idx].gf += ag;
        table[m.away_idx].ga += hg;
        if hg > ag {
            table[m.home_idx].w += 1;
            table[m.away_idx].l += 1;
        } else if ag > hg {
            table[m.away_idx].w += 1;
            table[m.home_idx].l += 1;
        } else {
            table[m.home_idx].d += 1;
            table[m.away_idx].d += 1;
        }
        total_goals += hg + ag;
        for &(id, g, sh, xg, grp) in &m.per_player {
            let e = agg.entry(id).or_insert((0, 0, 0.0, 0, grp));
            e.0 += g as u32;
            e.1 += sh as u32;
            e.2 += xg;
            e.3 += 1;
            group_goals[grp as usize] += g as u32;
        }
    }

    let n_matches = played.len();
    println!(
        "Played {} matches in {}s — {:.2} goals/match  ({} apps/player over the season)\n",
        n_matches,
        secs,
        total_goals as f32 / n_matches.max(1) as f32,
        apps_per_player
    );

    // League table, sorted by points then goal difference.
    let mut order: Vec<usize> = (0..n_teams).collect();
    order.sort_by(|&a, &b| {
        table[b]
            .pts()
            .cmp(&table[a].pts())
            .then(table[b].gd().cmp(&table[a].gd()))
    });
    println!("--- LEAGUE TABLE ---");
    println!(
        "  {:>2} {:<12} {:>3} {:>3} {:>3} {:>3} {:>3} {:>4} {:>4} {:>4}",
        "#", "club", "lvl", "P", "W", "D", "L", "GF", "GA", "Pts"
    );
    for (rank, &ti) in order.iter().enumerate() {
        let r = &table[ti];
        println!(
            "  {:>2} {:<12} {:>3} {:>3} {:>3} {:>3} {:>3} {:>4} {:>4} {:>4}",
            rank + 1,
            teams[ti].name,
            teams[ti].level,
            r.played,
            r.w,
            r.d,
            r.l,
            r.gf,
            r.ga,
            r.pts()
        );
    }

    // Top scorers — the headline. `Gls` over a full double round-robin IS the
    // season tally (apps == games played), so this is directly comparable to
    // a real Golden Boot (~25-30 in a 38-game league).
    let mut scorers: Vec<(u32, u32, u32, f32, u32, u8)> = agg
        .into_iter()
        .map(|(id, (g, sh, xg, apps, grp))| (id, g, sh, xg, apps, grp))
        .collect();
    scorers.sort_by(|a, b| b.1.cmp(&a.1));
    println!("\n--- TOP SCORERS (full season) ---");
    println!(
        "  {:>2} {:<12} {:<4} {:>4} {:>4} {:>5} {:>6} {:>7}",
        "#", "club", "pos", "Aps", "Gls", "Sh", "xG", "g/game"
    );
    for (rank, (id, g, sh, xg, apps, grp)) in scorers.iter().take(20).enumerate() {
        let team_idx = (*id / 100).saturating_sub(1) as usize;
        let club = teams.get(team_idx).map(|t| t.name.as_str()).unwrap_or("?");
        let pos = match grp {
            1 => "DEF",
            2 => "MID",
            3 => "FWD",
            _ => "GK",
        };
        let per = *g as f32 / (*apps).max(1) as f32;
        println!(
            "  {:>2} {:<12} {:<4} {:>4} {:>4} {:>5} {:>6.1} {:>7.2}",
            rank + 1,
            club,
            pos,
            apps,
            g,
            sh,
            xg,
            per
        );
    }

    // Season goals-by-line — does the SEASON distribution match real life?
    let line_total = group_goals.iter().sum::<u32>().max(1);
    println!("\n--- GOALS BY LINE (full season) ---");
    let labels = ["GK", "DEF", "MID", "FWD"];
    for (i, lab) in labels.iter().enumerate() {
        println!(
            "  {:<4} {:>4}  ({:>4.1}%)",
            lab,
            group_goals[i],
            group_goals[i] as f32 / line_total as f32 * 100.0
        );
    }
    println!("  real-life outfield share ≈ FWD 58% / MID 32% / DEF 10%");
    println!("\n  (Gls = full-season tally; includes penalties / set-pieces the engine produced.)");
}

fn print_usage() {
    eprintln!("Usage:");
    eprintln!("  dev_match                       open browser viewer (random squad levels)");
    eprintln!("  dev_match viewer [lvlA] [lvlB]  open browser viewer — levels random unless given");
    eprintln!(
        "  dev_match stats [N] [lvlA] [lvlB]  run N matches headless; per-match random levels"
    );
    eprintln!("                                      unless BOTH lvlA and lvlB are passed");
    eprintln!("  dev_match league [teams] [rounds] [minLvl] [maxLvl]  full round-robin season");
    eprintln!(
        "                                      defaults: 20 teams, 2 rounds (38 games), levels 8–18"
    );
    eprintln!();
    eprintln!(
        "Random level range: {}–{} inclusive.",
        RANDOM_LEVEL_MIN, RANDOM_LEVEL_MAX
    );
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
        "league" => {
            let teams: usize = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(20);
            let rounds: usize = args.get(3).and_then(|s| s.parse().ok()).unwrap_or(2);
            let min_lvl: u8 = args.get(4).and_then(|s| s.parse().ok()).unwrap_or(8);
            let max_lvl: u8 = args.get(5).and_then(|s| s.parse().ok()).unwrap_or(18);
            run_league(teams, rounds, min_lvl, max_lvl);
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
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("error")).init();

    let n_threads = rayon::current_num_threads();
    match (level_a, level_b) {
        (Some(a), Some(b)) => println!(
            "Running {} matches: level {} vs level {}  (parallel: {} threads)",
            n_matches, a, b, n_threads
        ),
        _ => println!(
            "Running {} matches: random squad levels per match ({}–{})  (parallel: {} threads)",
            n_matches, RANDOM_LEVEL_MIN, RANDOM_LEVEL_MAX, n_threads
        ),
    }
    println!();
    println!(
        "{:>3} {:>3}v{:>3} {:>3}-{:>3} | {:>3}/{:>3} sh {:>3}/{:>3} ot {:>4}/{:>4} xG {:>3}/{:>3} sv {:>3}/{:>3} tk {:>3}/{:>3} int {:>4}/{:>4} pa {:>2}/{:>2}% acc",
        "#",
        "lA",
        "lB",
        "H",
        "A",
        "H",
        "A",
        "H",
        "A",
        "H",
        "A",
        "H",
        "A",
        "H",
        "A",
        "H",
        "A",
        "H",
        "A",
        "H",
        "A"
    );

    // Reset the shot-gate waterfall counters once at run start. They
    // accumulate across all matches (including across threads — the
    // counters are AtomicU64) so we see which gate is suppressing shots
    // at population scale, not match-to-match noise.
    core::shot_gate_stats::reset();
    core::tackle_stats::reset();
    core::save_accounting_stats::reset();
    core::helper_diag::reset();
    core::mid_run_diag::reset();
    {
        use std::sync::atomic::Ordering;
        core::save_accounting_stats::SAVE_TICKS_REACHED.store(0, Ordering::Relaxed);
        core::save_accounting_stats::SAVE_TICKS_OUT_OF_REACH.store(0, Ordering::Relaxed);
        core::save_accounting_stats::SAVE_TICKS_PAST_GOAL_LINE.store(0, Ordering::Relaxed);
        core::save_accounting_stats::SAVE_PHYSICS_FIRED.store(0, Ordering::Relaxed);
        core::save_accounting_stats::SAVE_PHYSICS_PASSED.store(0, Ordering::Relaxed);
    }

    // Pre-roll per-match levels so the parallel work below is a pure
    // function of `i` and the work scheduler can dispatch in any order.
    // (We can't call `random_level()` inside the parallel closure and
    // still match the historical "i-th match's levels" reproducibility
    // expectation if anyone later seeds the RNG — but we still want
    // each level pair to be independent draws when no fixed levels
    // were passed.)
    let level_pairs: Vec<(u8, u8)> = (0..n_matches)
        .map(|_| {
            (
                level_a.unwrap_or_else(random_level),
                level_b.unwrap_or_else(random_level),
            )
        })
        .collect();

    let total_start = std::time::Instant::now();

    // Run all matches in parallel. Rayon's `into_par_iter().map().collect()`
    // preserves input order, so `outcomes` comes back sorted by match
    // index — the per-match table below prints in the same order as
    // the previous serial loop.
    //
    // Thread safety: each match builds its own squads, owns its own
    // RNG state via `rand::rng()` (thread-local), and the engine's
    // global counters (shot_gate / tackle / save_accounting / save
    // pipeline) are all `AtomicU64` so increments compose correctly
    // across threads.
    let outcomes: Vec<MatchOutcome> = level_pairs
        .par_iter()
        .enumerate()
        .map(|(i, &(match_level_a, match_level_b))| {
            let home = make_squad_simple(1, match_level_a);
            let away = make_squad_simple(2, match_level_b);
            let result = FootballEngine::<840, 545>::play(home, away, false, false, false);
            let score = result.score.as_ref().unwrap();
            let hg = score.home_team.get();
            let ag = score.away_team.get();
            let h = team_stats(&result, 1);
            let a = team_stats(&result, 2);
            let per_player = per_player_rows(&result);
            MatchOutcome {
                idx: i,
                level_a: match_level_a,
                level_b: match_level_b,
                home_goals: hg,
                away_goals: ag,
                home: h,
                away: a,
                per_player,
            }
        })
        .collect();
    let total_ms = total_start.elapsed().as_millis();

    // Print per-match rows in match order (single-threaded, so the
    // table is always coherent even though matches ran in parallel).
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
    let mut score_histogram: std::collections::BTreeMap<u8, u32> =
        std::collections::BTreeMap::new();

    for o in &outcomes {
        let h = &o.home;
        let a = &o.away;
        let h_acc = if h.passes_attempted > 0 {
            h.passes_completed * 100 / h.passes_attempted
        } else {
            0
        };
        let a_acc = if a.passes_attempted > 0 {
            a.passes_completed * 100 / a.passes_attempted
        } else {
            0
        };

        println!(
            "{:>3} {:>3}v{:>3} {:>3}-{:>3} | {:>3}/{:>3}    {:>3}/{:>3}    {:>4.1}/{:>4.1}    {:>3}/{:>3}    {:>3}/{:>3}    {:>3}/{:>3}     {:>4}/{:>4}  {:>2}/{:>2}%",
            o.idx + 1,
            o.level_a,
            o.level_b,
            o.home_goals,
            o.away_goals,
            h.shots,
            a.shots,
            h.on_target,
            a.on_target,
            h.xg,
            a.xg,
            h.saves,
            a.saves,
            h.tackles,
            a.tackles,
            h.interceptions,
            a.interceptions,
            h.passes_attempted,
            a.passes_attempted,
            h_acc,
            a_acc,
        );

        total_goals += o.home_goals as u32 + o.away_goals as u32;
        total_shots += h.shots as u32 + a.shots as u32;
        total_on_target += h.on_target as u32 + a.on_target as u32;
        total_saves += h.saves as u32 + a.saves as u32;
        total_tackles += h.tackles as u32 + a.tackles as u32;
        total_interceptions += h.interceptions + a.interceptions;
        total_passes_attempted += h.passes_attempted + a.passes_attempted;
        total_passes_completed += h.passes_completed + a.passes_completed;
        total_fouls += h.fouls as u32 + a.fouls as u32;
        total_xg += h.xg + a.xg;
        *score_histogram
            .entry(o.home_goals + o.away_goals)
            .or_default() += 1;
    }

    let n = n_matches as f32;
    println!();
    println!(
        "--- AGGREGATE over {} matches ({} real-world seconds) ---",
        n_matches,
        total_ms / 1000
    );
    println!(
        "goals per match     : {:.2}  (real ~2.5)",
        total_goals as f32 / n
    );
    println!(
        "xG per team/match   : {:.2}  (real ~1.3)",
        total_xg / (2.0 * n)
    );
    println!(
        "goals vs xG delta   : {:+.2}  (real ~0.0)",
        total_goals as f32 / n - total_xg / n
    );
    println!(
        "shots per team/match: {:.1}  (real ~13)",
        total_shots as f32 / (2.0 * n)
    );
    let shots_per_xg = if total_xg > 0.1 {
        total_shots as f32 / total_xg
    } else {
        0.0
    };
    println!(
        "shots per xG        : {:.1}   (real ~10; high = low-quality shots)",
        shots_per_xg
    );
    println!(
        "on-target rate      : {:.1}%  (real ~33%)",
        total_on_target as f32 / total_shots.max(1) as f32 * 100.0
    );
    let conversion = total_goals as f32 / total_on_target.max(1) as f32 * 100.0;
    println!("on-target→goal rate : {:.1}%  (real ~30%)", conversion);
    let saves_vs_ontarget = total_saves as f32 / total_on_target.max(1) as f32 * 100.0;
    println!(
        "saves/on-target     : {:.1}%  (real ~67%)",
        saves_vs_ontarget
    );
    println!(
        "passes per team     : {:.0}  (real ~500)",
        total_passes_attempted as f32 / (2.0 * n)
    );
    let pass_acc = if total_passes_attempted > 0 {
        total_passes_completed as f32 / total_passes_attempted as f32 * 100.0
    } else {
        0.0
    };
    println!("pass accuracy       : {:.1}%  (real ~85%)", pass_acc);
    println!(
        "tackles per team    : {:.1}  (real ~18)",
        total_tackles as f32 / (2.0 * n)
    );
    println!(
        "interceptions/team  : {:.1}  (real ~10)",
        total_interceptions as f32 / (2.0 * n)
    );
    println!(
        "fouls per team      : {:.1}  (real ~12)",
        total_fouls as f32 / (2.0 * n)
    );
    println!();
    println!("score total distribution (home+away goals per match):");
    for (total, count) in &score_histogram {
        let bar: String = std::iter::repeat('#').take(*count as usize).collect();
        println!("  {:>2}: {:>3} {}", total, count, bar);
    }

    // ── Per-player goal concentration / season projection ──────────────
    // Aggregate goals/shots/xG by player id across all matches. Player
    // ids are stable per position slot, so each id appears once per match
    // (an "appearance"). We project a SEASON_GAMES-game season to compare
    // against the website's top-scorer totals.
    const SEASON_GAMES: f32 = 42.0;
    let mut agg: std::collections::HashMap<u32, (u32, u32, f32, u32, u8)> =
        std::collections::HashMap::new(); // id -> (goals, shots, xg, apps, group)
    // Per-line totals (goals, shots, xg) indexed by group 0=GK 1=DEF 2=MID 3=FWD.
    // This is THE distribution metric the balance work targets.
    let mut group_agg: [(u32, u32, f32); 4] = [(0, 0, 0.0); 4];
    let mut per_match_top_scorer_goals: Vec<u16> = Vec::new();
    for o in &outcomes {
        // Track the single highest-scoring player in this match (any team).
        let mut match_top = 0u16;
        for &(id, goals, shots, xg, grp) in &o.per_player {
            let e = agg.entry(id).or_insert((0, 0, 0.0, 0, grp));
            e.0 += goals as u32;
            e.1 += shots as u32;
            e.2 += xg;
            e.3 += 1;
            e.4 = grp;
            let gi = grp as usize;
            group_agg[gi].0 += goals as u32;
            group_agg[gi].1 += shots as u32;
            group_agg[gi].2 += xg;
            match_top = match_top.max(goals);
        }
        per_match_top_scorer_goals.push(match_top);
    }

    // ── GOALS BY LINE — the headline balance metric ───────────────────
    // Real football outfield goal share ≈ FWD 58% / MID 32% / DEF 10%.
    // A reading of ~FWD 100% / MID 0% / DEF 0% is the concentration bug.
    println!();
    println!(
        "--- GOALS BY LINE (aggregated across {} matches) ---",
        n_matches
    );
    let line_labels = ["GK", "DEF", "MID", "FWD"];
    let line_total_goals: u32 = group_agg.iter().map(|g| g.0).sum::<u32>().max(1);
    let line_total_shots: u32 = group_agg.iter().map(|g| g.1).sum::<u32>().max(1);
    for (i, label) in line_labels.iter().enumerate() {
        let (g, sh, xg) = group_agg[i];
        println!(
            "  {:<4} goals={:>4} ({:>4.1}% of all)  shots={:>5} ({:>4.1}%)  xG={:>6.1}  conv={:>4.1}%",
            label,
            g,
            g as f32 / line_total_goals as f32 * 100.0,
            sh,
            sh as f32 / line_total_shots as f32 * 100.0,
            xg,
            if sh > 0 {
                g as f32 / sh as f32 * 100.0
            } else {
                0.0
            },
        );
    }
    println!("  target outfield goal share ≈ FWD 58% / MID 32% / DEF 10%");

    let mut rows: Vec<(u32, u32, u32, f32, u32, u8)> = agg
        .into_iter()
        .map(|(id, (g, sh, xg, apps, grp))| (id, g, sh, xg, apps, grp))
        .collect();
    rows.sort_by(|a, b| b.1.cmp(&a.1));
    println!();
    println!(
        "--- PER-PLAYER GOALS (aggregated across {} matches) ---",
        n_matches
    );
    println!(
        "  {:>5}  {:>4} {:>4} {:>5} {:>4}  {:>7} {:>7}  {:>5}   {:>9}",
        "id", "G", "Sh", "xG", "Aps", "G/app", "xG/app", "conv%", "proj/42g"
    );
    for (id, g, sh, xg, apps, grp) in rows.iter().take(14) {
        let apps_f = (*apps).max(1) as f32;
        let g_per = *g as f32 / apps_f;
        let xg_per = *xg / apps_f;
        let conv = if *sh > 0 {
            *g as f32 / *sh as f32 * 100.0
        } else {
            0.0
        };
        let tag = match grp {
            1 => "  DEF",
            2 => "  MID",
            3 => "  FWD",
            _ => "  GK",
        };
        println!(
            "  {:>5}  {:>4} {:>4} {:>5.1} {:>4}  {:>7.3} {:>7.3}  {:>4.0}%   {:>7.1}{}",
            id,
            g,
            sh,
            xg,
            apps,
            g_per,
            xg_per,
            conv,
            g_per * SEASON_GAMES,
            tag
        );
    }
    let avg_match_top = per_match_top_scorer_goals
        .iter()
        .map(|&x| x as f32)
        .sum::<f32>()
        / n as f32;
    println!(
        "  per-match top scorer avg: {:.3} goals  → if one player got every such match: {:.1}/season",
        avg_match_top,
        avg_match_top * SEASON_GAMES
    );
    // Goal share: what fraction of all goals went to the single top slot.
    let total_goals_agg: u32 = rows.iter().map(|r| r.1).sum();
    if let Some(top) = rows.first() {
        println!(
            "  top scorer share of ALL goals: {:.1}%  (top slot {} goals of {} total)",
            top.1 as f32 / total_goals_agg.max(1) as f32 * 100.0,
            top.1,
            total_goals_agg
        );
    }

    // Midfielder box-run + cutback redistribution diagnostics. These track
    // the mechanism that funnels chances to arriving central midfielders:
    // how many ticks an elected runner spent in a central shooting position
    // and how many cutbacks were played to them. If MID goal share is low
    // but RUNNER_BOX_TICKS is high, the runners arrive but aren't being fed
    // (distribution problem); if both are low, the runs aren't happening.
    let mr = core::mid_run_diag::snapshot();
    println!();
    println!("--- MID BOX-RUN / CUTBACK ---");
    println!(
        "  runner-in-box ticks={}  fwd cutbacks={}  mid cutbacks={}",
        mr[0], mr[1], mr[2]
    );
    println!(
        "  mid in-range ticks={}  mid box-shot fired={}",
        mr[3], mr[4]
    );
    println!(
        "  corners awarded={}  DEF corner-attack ticks={}  DEF corner headers on goal={}",
        mr[6], mr[7], mr[5]
    );
    println!(
        "  corner crosses sent={}  (to a CB={})  CB header chances={}",
        mr[8], mr[9], mr[10]
    );
    println!(
        "  corner-contest seen={}  fired={}  attacker-won={}",
        mr[11], mr[12], mr[13]
    );
    println!(
        "  block→corner branch fired={}  save-parry→corner branch fired={}",
        mr[14], mr[15]
    );

    // Shot-gate waterfall — each row is the absolute count of forward-has-ball
    // ticks that survived every gate so far. The % drop column is the share
    // of ticks that gate killed, measured against the tick count one row up.
    // The gate with the largest drop is the dominant shot suppressor.
    // Layout: index 3 (PASSED_NOT_POSSESSION) is informational — the
    // engine no longer gates shots on `prefer_possession`, but we still
    // observe how often the team is in tempo-management mode when a
    // forward has the ball in range. Print it separately so the
    // waterfall drops reflect the real gate chain.
    let s = core::shot_gate_stats::snapshot();

    // Helper-diagnostic counters: written by `evaluate_forward_shot_decision`
    // every time a forward state asks "should this be a shot?". `helper_diag`
    // catalogues which gate killed the call (xG floor / pass-EV / clear shot)
    // vs how many actually rolled the willingness die. The avg-at-roll
    // values are the population means of xG and willingness for the calls
    // that *reached* the willingness roll — invaluable when calibrating
    // the floor / willingness-curve coefficients in isolation.
    println!();
    println!("--- HELPER (evaluate_forward_shot_decision) ---");
    println!("  outcomes: shoot={}  pass={}  hold={}", s[9], s[10], s[11]);
    {
        use std::sync::atomic::Ordering;
        let calls = core::helper_diag::CALLS.load(Ordering::Relaxed);
        let h_hg = core::helper_diag::HOLD_HARDGATE.load(Ordering::Relaxed);
        let h_far = core::helper_diag::HOLD_FAR.load(Ordering::Relaxed);
        let h_xg = core::helper_diag::HOLD_XG.load(Ordering::Relaxed);
        let h_i6 = core::helper_diag::HOLD_INSIDE_SIX_XG.load(Ordering::Relaxed);
        let h_nc = core::helper_diag::HOLD_NO_CLEAR.load(Ordering::Relaxed);
        let p_def = core::helper_diag::PASS_DEFERRAL.load(Ordering::Relaxed);
        let reach = core::helper_diag::REACHED_ROLL.load(Ordering::Relaxed);
        let rolled = core::helper_diag::ROLL_PASSED.load(Ordering::Relaxed);
        let sum_xg = core::helper_diag::SUM_XG_X1000.load(Ordering::Relaxed);
        let sum_w = core::helper_diag::SUM_WILLINGNESS_X1000.load(Ordering::Relaxed);
        println!(
            "  calls={}  hold_hardgate={}  hold_far={}  hold_xg={}  hold_inside_six_xg={}  hold_no_clear={}  pass_defer={}  reached_roll={}  rolled_yes={}",
            calls, h_hg, h_far, h_xg, h_i6, h_nc, p_def, reach, rolled
        );
        if reach > 0 {
            let avg_xg = sum_xg as f64 / reach as f64 / 1000.0;
            let avg_w = sum_w as f64 / reach as f64 / 1000.0;
            println!("  avg-at-roll: xG≈{:.3}  willingness≈{:.4}", avg_xg, avg_w);
        }
    }

    let chain_order = [0usize, 1, 2, 4, 5, 6, 7, 8];
    let chain_labels = [
        "has_ball_in_range (dist <= 90)",
        "can_shoot (not on cooldown)",
        "has_settled (ownership >= 30)",
        "!defer_to_teammate",
        "dist <= max_shot_distance",
        "has_clear_shot()",
        "willingness roll passed",
        "FIRED (Shooting state entered)",
    ];
    println!();
    println!("--- SHOT-GATE WATERFALL (cumulative pass counts, all matches) ---");
    let base = s[0].max(1);
    for (row_idx, &i) in chain_order.iter().enumerate() {
        let drop_from_prior = if row_idx == 0 {
            0.0
        } else {
            let prior = s[chain_order[row_idx - 1]] as f64;
            if prior > 0.0 {
                (1.0 - s[i] as f64 / prior) * 100.0
            } else {
                0.0
            }
        };
        let share_of_base = s[i] as f64 / base as f64 * 100.0;
        println!(
            "  {:>10}  ({:>5.1}% of start, drop {:>5.1}%)  {}",
            s[i], share_of_base, drop_from_prior, chain_labels[row_idx]
        );
    }
    // Informational observation, not part of chain.
    let poss_share = s[3] as f64 / base as f64 * 100.0;
    println!(
        "  [info]   {:>5.1}% of in-range ticks had prefer_possession=false",
        poss_share
    );

    // Tackle flow per role: entries (state process() calls), attempts
    // (dice rolled), successes (TacklingBall emitted). The success→stat
    // mapping is 1:1 so the sum of role successes should match the
    // tackles/team column in the AGGREGATE block above.
    let t = core::tackle_stats::snapshot();
    println!();
    println!("--- TACKLE FLOW per role (cumulative, all matches) ---");
    let roles = ["DEF", "MID", "FWD", "GK"];
    let total_entries: u64 = t[0..4].iter().sum();
    let total_attempts: u64 = t[4..8].iter().sum();
    let total_successes: u64 = t[8..12].iter().sum();
    println!(
        "  {:<4}  {:>10}  {:>10}  {:>10}",
        "role", "entries", "attempts", "successes"
    );
    for (i, role) in roles.iter().enumerate() {
        println!(
            "  {:<4}  {:>10}  {:>10}  {:>10}",
            role,
            t[i],
            t[i + 4],
            t[i + 8]
        );
    }
    println!(
        "  {:<4}  {:>10}  {:>10}  {:>10}",
        "ALL", total_entries, total_attempts, total_successes
    );
    let success_per_match_per_team = total_successes as f64 / (n_matches as f64 * 2.0);
    println!(
        "  per-match per-team successes: {:.1}  (real football ~18)",
        success_per_match_per_team
    );

    // Save-accounting forensics: the saves vs on-target invariant must
    // hold (saves <= on_target). When it doesn't, this table tells us
    // which credit site is dropping on_target while still crediting save.
    let sa = core::save_accounting_stats::snapshot();
    println!();
    println!("--- SAVE ACCOUNTING per credit site (cumulative) ---");
    println!(
        "  {:<6}  {:>10}  {:>10}  {:>10}  {:>10}  {:>10}",
        "site", "saves", "on_target", "shots_faced", "shooter_NF", "prev_None"
    );
    let labels = core::save_accounting_stats::SITE_LABELS;
    let total_saves: u64 = sa.saves.iter().sum();
    let total_paired: u64 = sa.on_target.iter().sum();
    for (i, label) in labels.iter().enumerate() {
        println!(
            "  {:<6}  {:>10}  {:>10}  {:>10}  {:>10}  {:>10}",
            label,
            sa.saves[i],
            sa.on_target[i],
            sa.saves[i],
            sa.shooter_missing[i],
            sa.prev_owner_none[i],
        );
    }
    println!(
        "  {:<6}  {:>10}  {:>10}  {:>10}  {:>10}  {:>10}",
        "ALL",
        total_saves,
        total_paired,
        total_saves,
        sa.shooter_missing.iter().sum::<u64>(),
        sa.prev_owner_none.iter().sum::<u64>(),
    );
    println!("  on_target from goal-credit path: {}", sa.on_target_goal);
    let expected_on_target = total_paired + sa.on_target_goal;
    println!(
        "  expected memory on_target total: saves_paired ({}) + goals_paired ({}) = {}",
        total_paired, sa.on_target_goal, expected_on_target
    );
    let expected_saves_total = total_saves;
    println!(
        "  EXPECTED saves/on_target ratio = {:.1}%",
        if expected_on_target > 0 {
            expected_saves_total as f64 / expected_on_target as f64 * 100.0
        } else {
            0.0
        }
    );

    // Save-pipeline diagnostics — shows exactly where shots in flight
    // either reach the keeper for a save attempt, sail past, or fail to
    // engage at all. Helps localize whether low save% comes from few
    // attempts or low success-per-attempt.
    use std::sync::atomic::Ordering;
    let reached = core::save_accounting_stats::SAVE_TICKS_REACHED.load(Ordering::Relaxed);
    let oor = core::save_accounting_stats::SAVE_TICKS_OUT_OF_REACH.load(Ordering::Relaxed);
    let past = core::save_accounting_stats::SAVE_TICKS_PAST_GOAL_LINE.load(Ordering::Relaxed);
    let phys_fired = core::save_accounting_stats::SAVE_PHYSICS_FIRED.load(Ordering::Relaxed);
    let phys_passed = core::save_accounting_stats::SAVE_PHYSICS_PASSED.load(Ordering::Relaxed);
    println!();
    println!("--- SAVE PIPELINE ---");
    println!(
        "  ticks within reach window:  {} (out_of_reach: {}, past_line: {})",
        reached, oor, past
    );
    println!(
        "  physics save attempted:     {}  passed: {}  hit-rate: {:.1}%",
        phys_fired,
        phys_passed,
        if phys_fired > 0 {
            phys_passed as f64 / phys_fired as f64 * 100.0
        } else {
            0.0
        }
    );
}

fn run_viewer(level_a: Option<u8>, level_b: Option<u8>) {
    // Route `log::warn!` from core (notably the ball-stall snapshot) to
    // stderr. Override with `RUST_LOG=info` or `RUST_LOG=debug` for more.
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("warn"))
        .format_timestamp_millis()
        .init();

    // Enable event+state tracking for dev viewer — required so the
    // position data the HTML viewer consumes gets collected.
    MatchRuntime::set_events_mode(true);

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

    println!(
        "Completed: {}:{}, {}ms",
        home_goals,
        away_goals,
        elapsed.as_millis()
    );

    let goals_json: Vec<GoalJson> = score
        .detail()
        .iter()
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
        let gz_size = std::fs::metadata(&chunk_path)
            .map(|m| m.len() as usize)
            .unwrap_or(0);

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
    std::fs::write(
        &metadata_path,
        serde_json::to_string_pretty(&metadata).unwrap(),
    )
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
    {
        let _ = std::process::Command::new("cmd")
            .args(["/C", "start", "http://localhost:18001"])
            .spawn();
    }
    #[cfg(target_os = "macos")]
    {
        let _ = std::process::Command::new("open")
            .arg("http://localhost:18001")
            .spawn();
    }
    #[cfg(target_os = "linux")]
    {
        let _ = std::process::Command::new("xdg-open")
            .arg("http://localhost:18001")
            .spawn();
    }

    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(serve());
}

async fn serve() {
    use axum::routing::get;

    let app = axum::Router::new()
        .route("/", get(page_handler))
        .route("/api/match/{match_id}/metadata", get(metadata_handler))
        .route(
            "/api/match/{match_id}/chunk/{chunk_num}",
            get(chunk_handler),
        )
        .route("/static/images/match/field.svg", get(field_svg_handler))
        .route("/js/pixi.min.js", get(pixi_handler))
        .route("/match_data.js", get(data_handler));

    let listener = tokio::net::TcpListener::bind("0.0.0.0:18001")
        .await
        .unwrap();
    axum::serve(listener, app).await.unwrap();
}

async fn page_handler() -> axum::response::Html<String> {
    axum::response::Html(include_str!("viewer.html").to_string())
}

async fn data_handler() -> impl axum::response::IntoResponse {
    let path = PathBuf::from("match_results")
        .join(LEAGUE_SLUG)
        .join("page_data.js");
    let data = tokio::fs::read_to_string(&path).await.unwrap_or_default();
    (
        [(axum::http::header::CONTENT_TYPE, "application/javascript")],
        data,
    )
}

async fn metadata_handler(
    axum::extract::Path(match_id): axum::extract::Path<String>,
) -> impl axum::response::IntoResponse {
    let path = PathBuf::from("match_results")
        .join(LEAGUE_SLUG)
        .join(format!("{}_metadata.json", match_id));
    match tokio::fs::read_to_string(&path).await {
        Ok(data) => (
            axum::http::StatusCode::OK,
            [(axum::http::header::CONTENT_TYPE, "application/json")],
            data,
        )
            .into_response(),
        Err(_) => (axum::http::StatusCode::NOT_FOUND, "not found").into_response(),
    }
}

async fn chunk_handler(
    axum::extract::Path((match_id, chunk_num)): axum::extract::Path<(String, usize)>,
) -> impl axum::response::IntoResponse {
    let path = PathBuf::from("match_results")
        .join(LEAGUE_SLUG)
        .join(format!("{}_chunk_{}.json.gz", match_id, chunk_num));
    match tokio::fs::read(&path).await {
        Ok(data) => (
            axum::http::StatusCode::OK,
            [
                (axum::http::header::CONTENT_TYPE, "application/gzip"),
                (axum::http::header::CONTENT_ENCODING, "gzip"),
            ],
            data,
        )
            .into_response(),
        Err(_) => (axum::http::StatusCode::NOT_FOUND, "not found").into_response(),
    }
}

async fn field_svg_handler() -> impl axum::response::IntoResponse {
    let svg = include_str!("../../../src/web/assets/static/images/match/field.svg");
    ([(axum::http::header::CONTENT_TYPE, "image/svg+xml")], svg)
}

async fn pixi_handler() -> impl axum::response::IntoResponse {
    let js = include_bytes!("../../../src/web/assets/static/js/pixi.min.js");
    (
        [(axum::http::header::CONTENT_TYPE, "application/javascript")],
        js.as_slice(),
    )
}
