use axum::response::IntoResponse;
use core::club::player::Player;
use core::club::player::PlayerPositionType;
use core::club::team::tactics::{MatchTacticType, Tactics};
use core::r#match::FootballEngine;
use core::r#match::MatchSquad;
use core::r#match::player::MatchPlayer;
use core::staff_contract_mod::NaiveDate;
use core::{
    AcademyGenerationContext, MatchRuntime, PeopleNameGeneratorData, PlayerGenerator, PlayerSkills,
};
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

/// Maps the user-facing `level` parameter (1..20) onto a target mean
/// outfield skill the rest of the test rig calibrates around. Wraps the
/// constants and the retargeting routine into one struct so the level→
/// skill contract lives in a single place rather than scattered free
/// functions.
///
/// Anchor points (linear so consecutive levels stay distinguishable):
///   level  1 →  4.2  (Sunday League)
///   level  6 →  7.4  (lower English Football League)
///   level 10 →  9.6  (Championship-mid)
///   level 14 → 11.8  (PL mid-table)
///   level 18 → 14.0  (PL top six)
///   level 20 → 15.1  (Champions League elite)
///
/// Real-team skill distributions are narrower than 1..20 — peak adult
/// pros sit in the 12..17 band — so the curve keeps every level inside
/// the realistic envelope while preserving a meaningful step.
struct LevelSkillCurve;

impl LevelSkillCurve {
    const BASE: f32 = 3.6;
    const STEP: f32 = 0.575;
    /// `match_readiness` pinned here so fatigue doesn't distort the
    /// strength signal — players entering a friendly test should start
    /// fully match-ready.
    const MATCH_READINESS: f32 = 14.0;

    fn target_mean(level: u8) -> f32 {
        Self::BASE + level as f32 * Self::STEP
    }

    /// Additively shift every individually-set skill so the player's
    /// mean matches `target_mean`. The same delta lands on every skill,
    /// which preserves the natural intra-player shape (a forward stays
    /// finishing-heavy, a defender stays marking/tackling-heavy) while
    /// retargeting the absolute strength.
    fn retarget(skills: &mut PlayerSkills, target_mean: f32) {
        let cur_mean = Self::current_mean(skills);
        let delta = target_mean - cur_mean;
        skills.physical.match_readiness = Self::MATCH_READINESS;
        Self::shift_all(skills, delta);
    }

    fn current_mean(skills: &PlayerSkills) -> f32 {
        let s = &skills.technical;
        let m = &skills.mental;
        let p = &skills.physical;
        let g = &skills.goalkeeping;
        let total = s.corners + s.crossing + s.dribbling + s.finishing + s.first_touch
            + s.free_kicks + s.heading + s.long_shots + s.long_throws + s.marking
            + s.passing + s.penalty_taking + s.tackling + s.technique
            + m.aggression + m.anticipation + m.bravery + m.composure + m.concentration
            + m.decisions + m.determination + m.flair + m.leadership + m.off_the_ball
            + m.positioning + m.teamwork + m.vision + m.work_rate
            + p.acceleration + p.agility + p.balance + p.jumping + p.natural_fitness
            + p.pace + p.stamina + p.strength
            + g.aerial_reach + g.command_of_area + g.communication + g.eccentricity
            + g.first_touch + g.handling + g.kicking + g.one_on_ones + g.passing
            + g.punching + g.reflexes + g.rushing_out + g.throwing;
        // 14 technical + 14 mental + 8 physical (excluding match_readiness)
        // + 13 goalkeeping.
        total / (14 + 14 + 8 + 13) as f32
    }

    fn shift_all(skills: &mut PlayerSkills, delta: f32) {
        let bump = |x: &mut f32| *x = (*x + delta).clamp(1.0, 20.0);
        let s = &mut skills.technical;
        bump(&mut s.corners);
        bump(&mut s.crossing);
        bump(&mut s.dribbling);
        bump(&mut s.finishing);
        bump(&mut s.first_touch);
        bump(&mut s.free_kicks);
        bump(&mut s.heading);
        bump(&mut s.long_shots);
        bump(&mut s.long_throws);
        bump(&mut s.marking);
        bump(&mut s.passing);
        bump(&mut s.penalty_taking);
        bump(&mut s.tackling);
        bump(&mut s.technique);
        let m = &mut skills.mental;
        bump(&mut m.aggression);
        bump(&mut m.anticipation);
        bump(&mut m.bravery);
        bump(&mut m.composure);
        bump(&mut m.concentration);
        bump(&mut m.decisions);
        bump(&mut m.determination);
        bump(&mut m.flair);
        bump(&mut m.leadership);
        bump(&mut m.off_the_ball);
        bump(&mut m.positioning);
        bump(&mut m.teamwork);
        bump(&mut m.vision);
        bump(&mut m.work_rate);
        let p = &mut skills.physical;
        bump(&mut p.acceleration);
        bump(&mut p.agility);
        bump(&mut p.balance);
        bump(&mut p.jumping);
        bump(&mut p.natural_fitness);
        bump(&mut p.pace);
        bump(&mut p.stamina);
        bump(&mut p.strength);
        let g = &mut skills.goalkeeping;
        bump(&mut g.aerial_reach);
        bump(&mut g.command_of_area);
        bump(&mut g.communication);
        bump(&mut g.eccentricity);
        bump(&mut g.first_touch);
        bump(&mut g.handling);
        bump(&mut g.kicking);
        bump(&mut g.one_on_ones);
        bump(&mut g.passing);
        bump(&mut g.punching);
        bump(&mut g.reflexes);
        bump(&mut g.rushing_out);
        bump(&mut g.throwing);
    }
}

/// Generate an adult first-team player whose mean skill matches the
/// requested `level`. Two-step pipeline:
///
///   1. `PlayerGenerator::generate_with_context` with adult age (25-28)
///      so the position-specific skill SHAPE (forwards score higher on
///      finishing, defenders on marking/tackling, etc.) and trait roll
///      come out naturally. The academy context is left at the
///      `average()` defaults — its absolute level doesn't matter because
///      step 2 retargets the mean directly.
///   2. `LevelSkillCurve::retarget` adds a single delta to every skill so
///      the player's mean lands on the level-target curve.
///
/// Necessary because `PlayerGenerator::generate(level)` (used previously
/// here) routes `level` only into `AcademyGenerationContext.academy_level`,
/// which contributes a 15% weight to `ca_floor_score()` and zero to the
/// PA-cap-driving `ecosystem_score()`. Empirically that collapsed every
/// level into the same ~5-7 skill band — see `audit_levels` output —
/// which made `run_stats`' strength-curve alarm meaningless.
fn generate_player(id: u32, position: PlayerPositionType, level: u8) -> Player {
    let empty_names = PeopleNameGeneratorData {
        first_names: Vec::new(),
        last_names: Vec::new(),
        nicknames: Vec::new(),
    };
    // Anchor `now` on the 2026 season we're simulating; min/max ages 25-28
    // place every player on the adult plateau of the age curves
    // (`generator.rs:1268`) where tech ≥0.95, mental ≥0.85, physical ≥0.95.
    // The youth path's `min_age=max_age=14` damped every skill by 25-45%.
    let now = NaiveDate::from_ymd_opt(2026, 6, 1).unwrap();
    let mut player = PlayerGenerator::generate_with_context(
        1,
        now,
        position,
        &empty_names,
        &AcademyGenerationContext::average(),
        25,
        28,
        None,
    );

    LevelSkillCurve::retarget(&mut player.skills, LevelSkillCurve::target_mean(level));

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
    /// Times a teammate carried the ball INTO the opponent's final third
    /// on a single carry. Together with `prog_passes_into_final_third`,
    /// this is the canonical "did the team reach a dangerous area?"
    /// signal — distinguishes "weak team never gets into the final third"
    /// from "weak team gets there but can't shoot".
    prog_carries_into_final_third: u32,
    /// Completed passes ending in the opponent's final third from outside.
    prog_passes_into_final_third: u32,
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
    /// Per-player rows for this match:
    /// (player_id, goals, shots, xg, pos_group, rating, minutes, assists).
    /// pos_group: 0=GK 1=DEF 2=MID 3=FWD (derived from the 442 id slot).
    /// Used to measure per-player concentration, per-line goal share,
    /// and rating distribution by position / goal-count tier.
    per_player: Vec<(u32, u16, u16, f32, u8, f32, u16, u16)>,
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

/// Collect per-player (id, goals, shots, xg, pos_group, rating, minutes, assists) rows.
fn per_player_rows(
    result: &core::r#match::MatchResultRaw,
) -> Vec<(u32, u16, u16, f32, u8, f32, u16, u16)> {
    let mut rows = Vec::new();
    for (id, s) in result.player_stats.iter() {
        rows.push((
            *id,
            s.goals,
            s.shots_total,
            s.xg,
            pos_group_of(*id),
            s.match_rating,
            s.minutes_played,
            s.assists,
        ));
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
        prog_carries_into_final_third: 0,
        prog_passes_into_final_third: 0,
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
            ts.prog_carries_into_final_third +=
                s.zone_stats.progressive_carries_into_final_third as u32;
            ts.prog_passes_into_final_third +=
                s.zone_stats.progressive_passes_into_final_third as u32;
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
    per_player: Vec<(u32, u16, u16, f32, u8, f32, u16, u16)>,
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
        for &(id, g, sh, xg, grp, _rating, _minutes, _assists) in &m.per_player {
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
    eprintln!("  dev_match audit_levels [N]      generator diagnostic: mean outfield skills per level (default 200 squads)");
    eprintln!("  dev_match audit_engine_gap [N] [lvlA] [lvlB]  engine diagnostic: direct-skill matches at supplied gap");
    eprintln!("                                      bypasses generator; reveals engine-only response to skill gap");
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
        // Generator diagnostic: dumps mean outfield skills per level so
        // we can see whether `make_squad_simple(level)` actually responds
        // to `level`. If lvl 1 and lvl 20 print nearly identical numbers,
        // the strength-curve alarm in `stats` is measuring noise — fix
        // the generator path before tuning the engine. See
        // `run_audit_levels` for the rationale.
        "audit_levels" => {
            let n: usize = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(200);
            run_audit_levels(n);
        }
        // Engine diagnostic: directly assigns per-level skills (bypassing
        // the generator) and runs N matches at the supplied gap. Lets us
        // tell engine response apart from generator behaviour. See
        // `run_audit_engine_gap` / `make_squad_calibrated`.
        "audit_engine_gap" => {
            let n: usize = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(50);
            let a: u8 = args.get(3).and_then(|s| s.parse().ok()).unwrap_or(6);
            let b: u8 = args.get(4).and_then(|s| s.parse().ok()).unwrap_or(18);
            run_audit_engine_gap(n, a, b);
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

// ── audit_levels: dump avg outfield skills by level ────────────────────
//
// Generates `n` squads at every level 1..20 via `make_squad_simple` and
// prints the per-level mean of selected outfield attributes. The headline
// signal: if level 1 and level 20 produce nearly the same numbers, the
// generator path used by `.dev/match` is not actually translating its
// `level` argument into team strength — and any "strength curve" check
// in `run_stats` is then measuring squad noise, not engine behaviour.
//
// Background: `PlayerGenerator::generate(level)` routes its `level` only
// into `AcademyGenerationContext.academy_level`, which contributes 15% of
// `ca_floor_score()` and nothing to the PA-ceiling-driving `ecosystem_score()`.
// All other reputation / facility / coaching inputs default to "average".
// Empirically this collapses lvl 1 vs lvl 20 finishing to ~0.1 points apart.
fn run_audit_levels(n: usize) {
    println!(
        "Generating {} squads at each level (1..20), dumping avg outfield skill bands.\n",
        n
    );
    println!(
        "{:>3} {:>5} {:>5} {:>5} {:>5} {:>5} {:>5} {:>5} {:>5} {:>5} {:>5}",
        "lvl", "fin", "ls", "tch", "psg", "tck", "mrk", "anti", "dec", "pos", "agi"
    );
    for level in 1u8..=20 {
        let mut sum_fin = 0.0f32;
        let mut sum_ls = 0.0f32;
        let mut sum_tch = 0.0f32;
        let mut sum_psg = 0.0f32;
        let mut sum_tck = 0.0f32;
        let mut sum_mrk = 0.0f32;
        let mut sum_anti = 0.0f32;
        let mut sum_dec = 0.0f32;
        let mut sum_pos = 0.0f32;
        let mut sum_agi = 0.0f32;
        let mut count = 0u32;
        for team_id in 0..n {
            let squad = make_squad_simple((team_id + 1) as u32, level);
            for mp in &squad.main_squad {
                let s = &mp.skills;
                sum_fin += s.technical.finishing;
                sum_ls += s.technical.long_shots;
                sum_tch += s.technical.technique;
                sum_psg += s.technical.passing;
                sum_tck += s.technical.tackling;
                sum_mrk += s.technical.marking;
                sum_anti += s.mental.anticipation;
                sum_dec += s.mental.decisions;
                sum_pos += s.mental.positioning;
                sum_agi += s.physical.agility;
                count += 1;
            }
        }
        let d = count as f32;
        println!(
            "{:>3} {:>5.2} {:>5.2} {:>5.2} {:>5.2} {:>5.2} {:>5.2} {:>5.2} {:>5.2} {:>5.2} {:>5.2}",
            level,
            sum_fin / d,
            sum_ls / d,
            sum_tch / d,
            sum_psg / d,
            sum_tck / d,
            sum_mrk / d,
            sum_anti / d,
            sum_dec / d,
            sum_pos / d,
            sum_agi / d,
        );
    }
}

// ── audit_engine_gap: measure engine response to a real skill gap ──────
//
// Bypasses `PlayerGenerator` entirely and directly assigns every player
// the same per-level skill value (`3.0 + level/20 * 14.0`, so lvl 1 ≈ 3.7
// and lvl 20 ≈ 17.0). Then runs `n` matches at the supplied level pair
// and reports favourite / draw / upset frequency.
//
// Purpose: separate engine behaviour from squad-generation behaviour. If
// `run_stats` and this diagnostic disagree about whether the strength
// curve is biting, the generator path is the bottleneck (see
// `run_audit_levels`). If both show flat results, the engine itself
// fails to translate skill into outcomes.
//
// Stamina, natural_fitness, and match_readiness are pinned at 14 so
// fatigue dynamics don't confound the skill-curve measurement.
fn make_squad_calibrated(team_id: u32, level: u8) -> MatchSquad {
    let base_id = team_id * 100;
    let target = 3.0 + (level as f32 / 20.0) * 14.0;
    let main_squad: Vec<MatchPlayer> = POSITIONS_442
        .iter()
        .enumerate()
        .map(|(i, &pos)| {
            let mut player = generate_player(base_id + i as u32, pos, level);
            let s = &mut player.skills;
            // Technical
            s.technical.corners = target;
            s.technical.crossing = target;
            s.technical.dribbling = target;
            s.technical.finishing = target;
            s.technical.first_touch = target;
            s.technical.free_kicks = target;
            s.technical.heading = target;
            s.technical.long_shots = target;
            s.technical.long_throws = target;
            s.technical.marking = target;
            s.technical.passing = target;
            s.technical.penalty_taking = target;
            s.technical.tackling = target;
            s.technical.technique = target;
            // Mental
            s.mental.aggression = target;
            s.mental.anticipation = target;
            s.mental.bravery = target;
            s.mental.composure = target;
            s.mental.concentration = target;
            s.mental.decisions = target;
            s.mental.determination = target;
            s.mental.flair = target;
            s.mental.leadership = target;
            s.mental.off_the_ball = target;
            s.mental.positioning = target;
            s.mental.teamwork = target;
            s.mental.vision = target;
            s.mental.work_rate = target;
            // Physical — pin stamina/natural_fitness/match_readiness so
            // fatigue doesn't distort the skill-gap measurement.
            s.physical.acceleration = target;
            s.physical.agility = target;
            s.physical.balance = target;
            s.physical.jumping = target;
            s.physical.natural_fitness = 14.0;
            s.physical.pace = target;
            s.physical.stamina = 14.0;
            s.physical.strength = target;
            s.physical.match_readiness = 14.0;
            // Goalkeeping
            s.goalkeeping.aerial_reach = target;
            s.goalkeeping.command_of_area = target;
            s.goalkeeping.communication = target;
            s.goalkeeping.eccentricity = target;
            s.goalkeeping.first_touch = target;
            s.goalkeeping.handling = target;
            s.goalkeeping.kicking = target;
            s.goalkeeping.one_on_ones = target;
            s.goalkeeping.passing = target;
            s.goalkeeping.punching = target;
            s.goalkeeping.reflexes = target;
            s.goalkeeping.rushing_out = target;
            s.goalkeeping.throwing = target;
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

fn run_audit_engine_gap(n: usize, level_a: u8, level_b: u8) {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("error")).init();
    let target_a = 3.0 + (level_a as f32 / 20.0) * 14.0;
    let target_b = 3.0 + (level_b as f32 / 20.0) * 14.0;
    println!(
        "Engine gap test: {} matches, lvl {} (skills={:.1}) vs lvl {} (skills={:.1})",
        n, level_a, target_a, level_b, target_b
    );
    println!();

    struct GapOutcome {
        ha: u8,
        aa: u8,
        sh_a: u32,
        sh_b: u32,
        ot_a: u32,
        ot_b: u32,
        sv_a: u32,
        sv_b: u32,
        pa_a: u32,
        pa_b: u32,
        pc_a: u32,
        pc_b: u32,
        tk_a: u32,
        tk_b: u32,
        int_a: u32,
        int_b: u32,
        xg_a: f32,
        xg_b: f32,
        ft_carry_a: u32,
        ft_carry_b: u32,
        ft_pass_a: u32,
        ft_pass_b: u32,
    }

    let outcomes: Vec<GapOutcome> = (0..n)
        .into_par_iter()
        .map(|_| {
            let home = make_squad_calibrated(1, level_a);
            let away = make_squad_calibrated(2, level_b);
            let result = FootballEngine::<840, 545>::play(home, away, false, false, false);
            let score = result.score.as_ref().unwrap();
            let h = team_stats(&result, 1);
            let a = team_stats(&result, 2);
            GapOutcome {
                ha: score.home_team.get(),
                aa: score.away_team.get(),
                sh_a: h.shots as u32,
                sh_b: a.shots as u32,
                ot_a: h.on_target as u32,
                ot_b: a.on_target as u32,
                sv_a: h.saves as u32,
                sv_b: a.saves as u32,
                pa_a: h.passes_attempted as u32,
                pa_b: a.passes_attempted as u32,
                pc_a: h.passes_completed as u32,
                pc_b: a.passes_completed as u32,
                tk_a: h.tackles as u32,
                tk_b: a.tackles as u32,
                int_a: h.interceptions,
                int_b: a.interceptions,
                xg_a: h.xg,
                xg_b: a.xg,
                ft_carry_a: h.prog_carries_into_final_third,
                ft_carry_b: a.prog_carries_into_final_third,
                ft_pass_a: h.prog_passes_into_final_third,
                ft_pass_b: a.prog_passes_into_final_third,
            }
        })
        .collect();

    let mut a_wins = 0u32;
    let mut draws = 0u32;
    let mut b_wins = 0u32;
    let mut a_goals = 0u32;
    let mut b_goals = 0u32;
    let mut a_sh = 0u32;
    let mut b_sh = 0u32;
    let mut a_ot = 0u32;
    let mut b_ot = 0u32;
    let mut a_sv = 0u32;
    let mut b_sv = 0u32;
    let mut a_pa = 0u32;
    let mut b_pa = 0u32;
    let mut a_pc = 0u32;
    let mut b_pc = 0u32;
    let mut a_tk = 0u32;
    let mut b_tk = 0u32;
    let mut a_int = 0u32;
    let mut b_int = 0u32;
    let mut a_xg = 0.0f32;
    let mut b_xg = 0.0f32;
    let mut a_ftc = 0u32;
    let mut b_ftc = 0u32;
    let mut a_ftp = 0u32;
    let mut b_ftp = 0u32;
    for o in &outcomes {
        a_goals += o.ha as u32;
        b_goals += o.aa as u32;
        a_sh += o.sh_a;
        b_sh += o.sh_b;
        a_ot += o.ot_a;
        b_ot += o.ot_b;
        a_sv += o.sv_a;
        b_sv += o.sv_b;
        a_pa += o.pa_a;
        b_pa += o.pa_b;
        a_pc += o.pc_a;
        b_pc += o.pc_b;
        a_tk += o.tk_a;
        b_tk += o.tk_b;
        a_int += o.int_a;
        b_int += o.int_b;
        a_xg += o.xg_a;
        b_xg += o.xg_b;
        a_ftc += o.ft_carry_a;
        b_ftc += o.ft_carry_b;
        a_ftp += o.ft_pass_a;
        b_ftp += o.ft_pass_b;
        if o.ha > o.aa {
            a_wins += 1;
        } else if o.ha < o.aa {
            b_wins += 1;
        } else {
            draws += 1;
        }
    }
    let total = outcomes.len() as f32;
    let (fav_label, fav_w, dog_w) = if target_a >= target_b {
        ("A (home)", a_wins, b_wins)
    } else {
        ("B (away)", b_wins, a_wins)
    };
    println!(
        "  fav {} wins: {}/{} ({:.1}%)   draws: {}/{} ({:.1}%)   upsets: {}/{} ({:.1}%)",
        fav_label,
        fav_w,
        n,
        fav_w as f32 / total * 100.0,
        draws,
        n,
        draws as f32 / total * 100.0,
        dog_w,
        n,
        dog_w as f32 / total * 100.0,
    );
    println!(
        "  goals  A: {} (avg {:.2}/match)   B: {} (avg {:.2}/match)",
        a_goals,
        a_goals as f32 / total,
        b_goals,
        b_goals as f32 / total,
    );
    // Per-team funnel: shots → on-target → goals. Lets us tell apart
    // "weak team takes no shots" from "weak team takes shots but every
    // one is saved" from "weak team takes shots but they all miss".
    let pct = |num: u32, den: u32| {
        if den == 0 {
            0.0
        } else {
            num as f32 * 100.0 / den as f32
        }
    };
    println!(
        "  shots  A: {} (avg {:.1})   ot {} ({:.1}%)   sv {} ({:.1}% saved)   conv {:.1}% goals/ot",
        a_sh,
        a_sh as f32 / total,
        a_ot,
        pct(a_ot, a_sh),
        b_sv, // saves by GK B against shots from A
        pct(b_sv, a_ot),
        pct(a_goals, a_ot),
    );
    println!(
        "  shots  B: {} (avg {:.1})   ot {} ({:.1}%)   sv {} ({:.1}% saved)   conv {:.1}% goals/ot",
        b_sh,
        b_sh as f32 / total,
        b_ot,
        pct(b_ot, b_sh),
        a_sv,
        pct(a_sv, b_ot),
        pct(b_goals, b_ot),
    );
    println!(
        "  passes A: {} ({:.1}% acc)   B: {} ({:.1}% acc)",
        a_pa,
        pct(a_pc, a_pa),
        b_pa,
        pct(b_pc, b_pa),
    );
    // Possession proxy via pass volume. A team that holds the ball longer
    // attempts more passes per match — this is the metric Opta uses
    // internally for "possession %" (their lines aren't from clock time,
    // they're from event count). Useful here because the engine doesn't
    // expose a possession-time field directly.
    let pass_total = (a_pa + b_pa).max(1);
    let a_poss = a_pa as f32 / pass_total as f32 * 100.0;
    let b_poss = b_pa as f32 / pass_total as f32 * 100.0;
    println!(
        "  possession (pass-share)  A: {:.1}%   B: {:.1}%",
        a_poss, b_poss
    );
    // Shots-per-possession: how efficiently a team converts ball
    // ownership into goal attempts. Real PL: ~3.5% across both teams.
    // A 5× gap here (vs ~1.6× possession gap) means the bottleneck
    // is NOT possession — it's converting possession into chances.
    println!(
        "  shots / 100 passes attempted  A: {:.2}   B: {:.2}",
        a_sh as f32 / a_pa.max(1) as f32 * 100.0,
        b_sh as f32 / b_pa.max(1) as f32 * 100.0,
    );
    // Defensive turnovers TAKEN by each team (tackles + interceptions
    // they made themselves). Compare against the volume of pass attempts
    // by the OPPOSING team — a team that wins back 30% of opponent
    // pass attempts is a high-pressing side.
    let a_steals = a_tk + a_int;
    let b_steals = b_tk + b_int;
    println!(
        "  tackles+ints  A: {} ({} tk + {} int)   B: {} ({} tk + {} int)",
        a_steals, a_tk, a_int, b_steals, b_tk, b_int,
    );
    println!(
        "  steals / 100 opp-passes  A: {:.2} (vs B's {} passes)   B: {:.2} (vs A's {} passes)",
        a_steals as f32 / b_pa.max(1) as f32 * 100.0,
        b_pa,
        b_steals as f32 / a_pa.max(1) as f32 * 100.0,
        a_pa,
    );
    // xG totals: did the weak team even GENERATE chances worth taking?
    // If team-A xG is ~0 the issue is "no shots created", not "shots
    // not converted".
    println!(
        "  xG total  A: {:.1} ({:.2}/match, {:.3}/shot)   B: {:.1} ({:.2}/match, {:.3}/shot)",
        a_xg,
        a_xg / total,
        a_xg / a_sh.max(1) as f32,
        b_xg,
        b_xg / total,
        b_xg / b_sh.max(1) as f32,
    );
    // Final-third entries: how many times did each team reach the
    // opponent's attacking third (carries that crossed in + completed
    // passes that ended there from outside). Bridges the gap between
    // possession share and shot volume — if A has 38% possession but
    // only 5% of final-third entries, the funnel collapse is in midfield
    // not in the box.
    println!(
        "  final-third entries  A: {} ({} carries + {} passes, {:.1}/match)   B: {} ({} carries + {} passes, {:.1}/match)",
        a_ftc + a_ftp,
        a_ftc,
        a_ftp,
        (a_ftc + a_ftp) as f32 / total,
        b_ftc + b_ftp,
        b_ftc,
        b_ftp,
        (b_ftc + b_ftp) as f32 / total,
    );
    // Shots per final-third entry — "did the team SHOOT from the
    // dangerous areas they reached?". Real PL bottom vs top: ~0.5 shots
    // per FT entry on both sides — when you get into the final third,
    // you usually get a shot away. If the engine shows weak teams
    // entering the final third but not shooting, the bottleneck is in
    // the final-third shot decision (a defender always close enough to
    // suppress the shot); if FT entries are themselves rare, the
    // bottleneck is midfield progression.
    let a_ft_entries = (a_ftc + a_ftp).max(1);
    let b_ft_entries = (b_ftc + b_ftp).max(1);
    println!(
        "  shots / final-third entry  A: {:.2}   B: {:.2}",
        a_sh as f32 / a_ft_entries as f32,
        b_sh as f32 / b_ft_entries as f32,
    );
    println!();
    // Bucket-aligned reference rows. Use the actual `level` gap as the
    // bucket key (same as the upset-frequency table in `run_stats`).
    let gap = (level_a as i32 - level_b as i32).unsigned_abs() as u32;
    let (ref_fav, ref_draw, ref_up, ref_label) = match gap {
        0..=2 => (45, 25, 30, "gap 0-2 close"),
        3..=5 => (58, 22, 20, "gap 3-5 clear edge"),
        6..=8 => (70, 17, 13, "gap 6-8 heavy fav."),
        _ => (78, 13, 9, "gap 9+ extreme"),
    };
    println!(
        "  reference for {} (gap {}): fav {}%, draw {}%, upset {}%",
        ref_label, gap, ref_fav, ref_draw, ref_up,
    );
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

    // ── Scoreline distribution — diagnose draw inflation ──────────────
    //
    // Real PL scoreline distribution (approximate, last 5 seasons):
    //   1-1: 11% | 1-0: 10% | 2-1: 12% | 0-0: 8% | 2-0: 9% | 2-2: 5%
    //   3-1: 7% | 3-0: 5% | 3-2: 4% | other: 29%
    //   Total draws ≈ 25%, decisive ≈ 75%
    //
    // The engine sits at ~52-55% draws at equal skill. This breakdown
    // identifies WHICH draws are over-represented. Hypotheses:
    //   - 0-0 inflation → not enough scoring opportunities (low total goals)
    //   - 1-1 inflation → equalizer dynamic (team B scores soon after A)
    //   - 2-2 inflation → back-and-forth correlation (both keep responding)
    let mut scoreline_counts: std::collections::BTreeMap<(u8, u8), u32> =
        std::collections::BTreeMap::new();
    let mut draws_by_total: std::collections::BTreeMap<u8, u32> =
        std::collections::BTreeMap::new();
    for o in &outcomes {
        // Bucket as (lower, higher) so 2-1 and 1-2 land in same row —
        // we care about scoreline shape, not which team scored.
        let key = (o.home_goals.min(o.away_goals), o.home_goals.max(o.away_goals));
        *scoreline_counts.entry(key).or_default() += 1;
        if o.home_goals == o.away_goals {
            *draws_by_total.entry(o.home_goals).or_default() += 1;
        }
    }
    println!();
    println!("--- SCORELINE distribution (sorted by frequency) ---");
    let mut scoreline_sorted: Vec<((u8, u8), u32)> =
        scoreline_counts.into_iter().collect();
    scoreline_sorted.sort_by(|a, b| b.1.cmp(&a.1));
    let total_n = n_matches as f32;
    for ((lo, hi), count) in scoreline_sorted.iter().take(15) {
        let pct = *count as f32 / total_n * 100.0;
        let kind = if lo == hi { "DRAW" } else { "DEC " };
        let bar: String = std::iter::repeat('#').take((pct.round() as usize).min(40)).collect();
        println!(
            "  {}-{}  {}  {:>4} ({:>5.1}%) {}",
            lo, hi, kind, count, pct, bar
        );
    }
    println!();
    println!("--- DRAWS breakdown (each n-n) ---");
    let total_draws: u32 = draws_by_total.values().sum();
    let real_draw_breakdown = [
        (0u8, "0-0 (real ~8%)"),
        (1u8, "1-1 (real ~11%)"),
        (2u8, "2-2 (real ~5%)"),
        (3u8, "3-3 (real ~1%)"),
    ];
    for (n, label) in &real_draw_breakdown {
        let count = draws_by_total.get(n).copied().unwrap_or(0);
        let pct = count as f32 / total_n * 100.0;
        println!("  {} : {:>4} ({:>5.1}% of all matches)", label, count, pct);
    }
    let other_draws: u32 = draws_by_total
        .iter()
        .filter(|(n, _)| **n >= 4)
        .map(|(_, c)| *c)
        .sum();
    println!(
        "  4-4+         : {:>4} ({:>5.1}% of all matches)",
        other_draws,
        other_draws as f32 / total_n * 100.0,
    );
    println!(
        "  total draws  : {:>4} ({:>5.1}% of all matches, real ~25%)",
        total_draws,
        total_draws as f32 / total_n * 100.0,
    );

    // ── UPSET FREQUENCY by level gap ──────────────────────────────────
    //
    // Does the stronger team actually win more often when the gap is
    // big? Real-football reference (Premier League / La Liga seasons):
    //
    //   gap 0-2 (close):       favorite ~45%, draw ~25%, underdog ~30%
    //   gap 3-5 (clear edge):  favorite ~58%, draw ~22%, underdog ~20%
    //   gap 6-8 (heavy fav.):  favorite ~70%, draw ~17%, underdog ~13%
    //   gap 9+  (extreme):     favorite ~78%, draw ~13%, underdog ~9%
    //
    // The "underdog" column is the upset frequency — should drop as
    // the gap widens but never reach zero (real football has the rare
    // 1-0 dogged shock). A flat underdog rate across all gaps means
    // team strength isn't biting; a zero underdog rate at large gaps
    // means the strength multiplier is too steep.
    //
    // Drawn matches between equal-level teams are excluded from the
    // bucket totals (no favorite/underdog to assign).
    let mut gap_buckets: [(u32, u32, u32); 4] = [(0, 0, 0); 4]; // (fav_w, draw, upset)
    let bucket_labels = [
        "gap 0-2 (close)     ",
        "gap 3-5 (clear edge)",
        "gap 6-8 (heavy fav.)",
        "gap 9+  (extreme)   ",
    ];
    let mut total_in_buckets = 0u32;
    for o in &outcomes {
        if o.level_a == o.level_b {
            continue; // can't measure upsets when levels match
        }
        let gap = o.level_a.abs_diff(o.level_b);
        let bucket = match gap {
            0..=2 => 0,
            3..=5 => 1,
            6..=8 => 2,
            _ => 3,
        };
        let stronger_is_home = o.level_a > o.level_b;
        let (fav_goals, dog_goals) = if stronger_is_home {
            (o.home_goals, o.away_goals)
        } else {
            (o.away_goals, o.home_goals)
        };
        if fav_goals > dog_goals {
            gap_buckets[bucket].0 += 1;
        } else if fav_goals < dog_goals {
            gap_buckets[bucket].2 += 1;
        } else {
            gap_buckets[bucket].1 += 1;
        }
        total_in_buckets += 1;
    }
    println!();
    println!("--- UPSET FREQUENCY by level gap (mismatched levels only) ---");
    println!(
        "  {:<22} {:>6}  {:>6}  {:>6}  {:>6}    reference",
        "bucket", "fav%", "draw%", "upset%", "n"
    );
    let refs = [
        "fav 45%, draw 25%, upset 30%",
        "fav 58%, draw 22%, upset 20%",
        "fav 70%, draw 17%, upset 13%",
        "fav 78%, draw 13%, upset  9%",
    ];
    for (i, label) in bucket_labels.iter().enumerate() {
        let (fw, dr, up) = gap_buckets[i];
        let total = (fw + dr + up).max(1);
        let pct = |x: u32| x as f32 / total as f32 * 100.0;
        println!(
            "  {:<22} {:>5.1}%  {:>5.1}%  {:>5.1}%  {:>6}    {}",
            label,
            pct(fw),
            pct(dr),
            pct(up),
            fw + dr + up,
            refs[i],
        );
    }
    println!(
        "  ({} matches with non-equal levels; {} equal-level matches excluded)",
        total_in_buckets,
        outcomes.len() as u32 - total_in_buckets,
    );

    // Headline upset alarm: if ANY mismatched bucket shows ≥40% upset
    // or 0% upset, the strength curve is wrong. Print a one-liner
    // verdict so it's obvious without reading the table.
    let mut alarms: Vec<String> = Vec::new();
    for (i, label) in bucket_labels.iter().enumerate() {
        let (fw, dr, up) = gap_buckets[i];
        let total = (fw + dr + up).max(1) as f32;
        if total < 8.0 {
            continue; // sample too small to read
        }
        let up_pct = up as f32 / total * 100.0;
        // Refs: 30/20/13/9. Tolerance ±10 for the close-gap bucket,
        // tightening to ±6 for the extreme bucket where upsets are rare.
        let (ref_pct, tol) = match i {
            0 => (30.0, 10.0),
            1 => (20.0, 9.0),
            2 => (13.0, 8.0),
            _ => (9.0, 7.0),
        };
        let diff = up_pct - ref_pct;
        if diff.abs() > tol {
            let direction = if diff > 0.0 {
                "too many upsets"
            } else {
                "too few upsets"
            };
            alarms.push(format!(
                "  ⚠ {} — upset% {:.1} vs ref {:.1} ({})",
                label.trim_end(),
                up_pct,
                ref_pct,
                direction,
            ));
        }
    }
    if !alarms.is_empty() {
        println!();
        println!("  Strength-curve alarms:");
        for a in &alarms {
            println!("{}", a);
        }
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
        for &(id, goals, shots, xg, grp, _rating, _minutes, _assists) in &o.per_player {
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

    // ── RATINGS DISTRIBUTION — per-position mean/median/p10/p90 ──────────
    //
    // Compares the engine's match-rating output against real-football
    // reference bands (WhoScored season averages):
    //   GK   ≈ 6.65-7.10    (varies with team strength)
    //   DEF  ≈ 6.55-6.95
    //   MID  ≈ 6.60-7.00
    //   FWD  ≈ 6.55-7.15    (most volatile — goal output drives it)
    //
    // For each position, also splits the rating distribution by goal
    // count (0g, 1g, 2g+) so the "11g/13ap scorer at 6.53" symptom
    // surfaces directly: if the 1g+ band fails to clear the 0g band by
    // enough, goal-event credit is under-weighted; if both bands sit
    // below the reference, ARE / shot-spam / context damping is too
    // aggressive overall.
    //
    // Per-line aggregation: every (player, match) sample is one row.
    // Apps with minutes==0 are skipped (they didn't really play).
    let mut ratings_by_pos: [Vec<f32>; 4] = Default::default();
    let mut ratings_by_pos_goalless: [Vec<f32>; 4] = Default::default();
    let mut ratings_by_pos_one_goal: [Vec<f32>; 4] = Default::default();
    let mut ratings_by_pos_two_plus: [Vec<f32>; 4] = Default::default();
    let mut ratings_by_pos_with_assist_only: [Vec<f32>; 4] = Default::default();
    // Per-PLAYER weighted season-average rating, sliced by line. This is
    // the apples-to-apples comparison against the website's "AV RAT"
    // column the user reports against.
    let mut player_rating_sum: std::collections::HashMap<u32, (f32, f32, u8)> =
        std::collections::HashMap::new(); // id -> (rating_points, rating_weight, group)
    for o in &outcomes {
        for &(id, goals, _sh, _xg, grp, rating, minutes, assists) in &o.per_player {
            if minutes == 0 {
                continue;
            }
            let gi = grp as usize;
            ratings_by_pos[gi].push(rating);
            match goals {
                0 if assists == 0 => ratings_by_pos_goalless[gi].push(rating),
                0 => ratings_by_pos_with_assist_only[gi].push(rating),
                1 => ratings_by_pos_one_goal[gi].push(rating),
                _ => ratings_by_pos_two_plus[gi].push(rating),
            }
            // Minute-weighted (mirror PlayerStatistics::record_match_rating
            // clamps: starter floor 0.65, sub floor 0.20). The 442 sim has
            // no subs, but the floor logic still matters when subs land.
            let is_starter = minutes as u32 >= 45; // crude proxy: full-game sample
            let raw = minutes as f32 / 90.0;
            let min_weight = if is_starter { 0.65 } else { 0.20 };
            let w = raw.max(min_weight);
            let e = player_rating_sum.entry(id).or_insert((0.0, 0.0, grp));
            e.0 += rating * w;
            e.1 += w;
        }
    }
    fn dist_summary(vals: &mut Vec<f32>) -> (f32, f32, f32, f32, usize) {
        let n = vals.len();
        if n == 0 {
            return (0.0, 0.0, 0.0, 0.0, 0);
        }
        let mean = vals.iter().sum::<f32>() / n as f32;
        vals.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        let p = |q: f32| -> f32 {
            let idx = ((n as f32 - 1.0) * q).round() as usize;
            vals[idx.min(n - 1)]
        };
        (mean, p(0.50), p(0.10), p(0.90), n)
    }
    println!();
    println!(
        "--- RATINGS DISTRIBUTION (per-match samples, {} matches) ---",
        n_matches
    );
    println!(
        "  {:<4} {:>6} {:>6} {:>6} {:>6} {:>6}    reference",
        "pos", "mean", "p50", "p10", "p90", "n"
    );
    let refs = [
        ("GK", "6.65-7.10"),
        ("DEF", "6.55-6.95"),
        ("MID", "6.60-7.00"),
        ("FWD", "6.55-7.15"),
    ];
    for (i, (label, refband)) in refs.iter().enumerate() {
        let (m, p50, p10, p90, n) = dist_summary(&mut ratings_by_pos[i]);
        println!(
            "  {:<4} {:>6.2} {:>6.2} {:>6.2} {:>6.2} {:>6}    {}",
            label, m, p50, p10, p90, n, refband
        );
    }
    println!();
    println!("--- RATINGS BY GOAL COUNT (FWD slice, the canonical \"goal scorer\" diagnostic) ---");
    println!(
        "  {:<14} {:>6} {:>6} {:>6} {:>6} {:>6}",
        "tier", "mean", "p50", "p10", "p90", "n"
    );
    let fwd_tiers = [
        ("FWD 0g/0a", &mut ratings_by_pos_goalless[3]),
        ("FWD 0g+1a", &mut ratings_by_pos_with_assist_only[3]),
        ("FWD 1g", &mut ratings_by_pos_one_goal[3]),
        ("FWD 2g+", &mut ratings_by_pos_two_plus[3]),
    ];
    for (label, vals) in fwd_tiers {
        let (m, p50, p10, p90, n) = dist_summary(vals);
        println!(
            "  {:<14} {:>6.2} {:>6.2} {:>6.2} {:>6.2} {:>6}",
            label, m, p50, p10, p90, n
        );
    }
    println!();
    println!("--- PER-PLAYER SEASON AVG (minute-weighted, like website's AV RAT) ---");
    let mut player_avgs_by_pos: [Vec<f32>; 4] = Default::default();
    for (_id, (pts, w, grp)) in &player_rating_sum {
        if *w <= 0.0 {
            continue;
        }
        player_avgs_by_pos[*grp as usize].push(pts / w);
    }
    println!(
        "  {:<4} {:>6} {:>6} {:>6} {:>6} {:>6}",
        "pos", "mean", "p50", "p10", "p90", "n"
    );
    for (i, label) in ["GK", "DEF", "MID", "FWD"].iter().enumerate() {
        let (m, p50, p10, p90, n) = dist_summary(&mut player_avgs_by_pos[i]);
        println!(
            "  {:<4} {:>6.2} {:>6.2} {:>6.2} {:>6.2} {:>6}",
            label, m, p50, p10, p90, n
        );
    }

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
