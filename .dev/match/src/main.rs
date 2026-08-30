use axum::response::IntoResponse;
use core::PlayerFieldPositionGroup;
use core::block_diag::BlockDiag;
use core::club::player::Player;
use core::club::player::PlayerPositionType;
use core::club::team::tactics::{MatchTacticType, Tactics};
use core::frame_trace::FrameTrace;
use core::heatmap_diag as heat;
use core::heatmap_diag::HeatMapCensus;
use core::r#match::FootballEngine;
use core::r#match::MatchSquad;
use core::r#match::player::MatchPlayer;
use core::r#match::player::state::PlayerState;
use core::r#match::player::strategies::players::ops::skill_composites as sc;
use core::staff_contract_mod::NaiveDate;
use core::{
    AcademyGenerationContext, MatchRuntime, PeopleNameGeneratorData, PlayerGenerator, PlayerSkills,
};
use flate2::Compression;
use flate2::write::GzEncoder;
use rand::RngExt;
use rayon::prelude::*;
use serde::Serialize;
use shared::{Appearance, Region, SkinBucket, SkinDist};
use std::env;
use std::io::Write;
use std::path::PathBuf;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicUsize, Ordering};

/// Random squad level range when no explicit level is passed. Covers the
/// realistic spread from a lower-league squad (6) to an elite top-flight
/// team (18) — gives us a mix of matchups to stress-test balance across
/// skill gaps rather than always testing 14-vs-14 homogeneous squads.
const RANDOM_LEVEL_MIN: u8 = 6;
const RANDOM_LEVEL_MAX: u8 = 18;

/// Allocation-counting global allocator — compiled in only with
/// `--features alloc-count`. Two relaxed atomics per alloc: fine for
/// counting, but it skews the timing benchmark, so the default build
/// keeps the plain system allocator. `Bench::run` prints allocs/match
/// and bytes/match when this is active.
#[cfg(feature = "alloc-count")]
mod alloc_count {
    use std::alloc::{GlobalAlloc, Layout, System};
    use std::cell::Cell;
    use std::collections::HashMap;
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicU64, Ordering};

    pub static ALLOC_CALLS: AtomicU64 = AtomicU64::new(0);
    pub static ALLOC_BYTES: AtomicU64 = AtomicU64::new(0);

    /// Sample 1 of every N allocations with a captured backtrace so we
    /// can attribute allocation volume to call sites. Backtrace capture
    /// itself allocates, so a thread-local recursion guard keeps the
    /// sampler from re-entering itself. Set OF_ALLOC_STACKS=1 to enable
    /// (needs the `profiling` cargo profile for symbolicated frames).
    const SAMPLE_EVERY: u64 = 512;
    static STACKS_ENABLED: AtomicU64 = AtomicU64::new(u64::MAX); // MAX = unresolved
    pub static SITE_COUNTS: Mutex<Option<HashMap<String, u64>>> = Mutex::new(None);

    thread_local! {
        static IN_SAMPLER: Cell<bool> = const { Cell::new(false) };
    }

    fn stacks_enabled() -> bool {
        match STACKS_ENABLED.load(Ordering::Relaxed) {
            u64::MAX => {
                let on = std::env::var_os("OF_ALLOC_STACKS").is_some() as u64;
                STACKS_ENABLED.store(on, Ordering::Relaxed);
                on == 1
            }
            v => v == 1,
        }
    }

    fn maybe_sample(calls_so_far: u64) {
        if calls_so_far % SAMPLE_EVERY != 0 || !stacks_enabled() {
            return;
        }
        IN_SAMPLER.with(|flag| {
            if flag.get() {
                return;
            }
            flag.set(true);
            let bt = std::backtrace::Backtrace::force_capture();
            let text = bt.to_string();
            // Keep only project frames — the interesting attribution is
            // "which engine call site allocated", not the alloc plumbing.
            let mut site = String::new();
            let mut skipped_plumbing = 0u32;
            for line in text.lines() {
                let t = line.trim();
                let name = t.split_once(": ").map(|(_, n)| n).unwrap_or(t);
                // Skip the sampler's own frames and the raw alloc shims;
                // keep everything else (RawVec / hashbrown growth frames
                // included — they say WHAT grew even when the engine call
                // site got inlined out of the walkable stack).
                if name.contains("alloc_count")
                    || name.contains("__rust_alloc")
                    || name.contains("__rust_realloc")
                    || name.contains("backtrace")
                    || name.contains("LocalKey")
                    || name.starts_with("at ")
                    || name.starts_with("alloc::")
                    || name.starts_with("core::iter")
                    || name.starts_with("core::slice")
                    || name.starts_with("core::ops")
                {
                    skipped_plumbing += 1;
                    let _ = skipped_plumbing;
                    continue;
                }
                if !site.is_empty() {
                    site.push_str(" <- ");
                }
                site.push_str(&name[..name.len().min(120)]);
                if site.len() > 420 {
                    break;
                }
            }
            if site.is_empty() {
                site = "<non-engine>".to_string();
            }
            let mut guard = SITE_COUNTS.lock().unwrap();
            let map = guard.get_or_insert_with(HashMap::new);
            *map.entry(site).or_insert(0) += 1;
            flag.set(false);
        });
    }

    pub struct CountingAlloc;

    unsafe impl GlobalAlloc for CountingAlloc {
        unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
            let n = ALLOC_CALLS.fetch_add(1, Ordering::Relaxed);
            ALLOC_BYTES.fetch_add(layout.size() as u64, Ordering::Relaxed);
            maybe_sample(n);
            unsafe { System.alloc(layout) }
        }
        unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
            unsafe { System.dealloc(ptr, layout) }
        }
        unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
            let n = ALLOC_CALLS.fetch_add(1, Ordering::Relaxed);
            ALLOC_BYTES.fetch_add(new_size as u64, Ordering::Relaxed);
            maybe_sample(n);
            unsafe { System.realloc(ptr, layout, new_size) }
        }
    }

    /// Print the aggregated alloc-site table (top `n`), then clear it.
    pub fn dump_sites(n: usize) {
        let mut guard = SITE_COUNTS.lock().unwrap();
        let Some(map) = guard.take() else {
            return;
        };
        let mut rows: Vec<(String, u64)> = map.into_iter().collect();
        rows.sort_by(|a, b| b.1.cmp(&a.1));
        let total: u64 = rows.iter().map(|r| r.1).sum();
        println!(
            "ALLOC SITES (sampled 1/{}, {} samples):",
            SAMPLE_EVERY, total
        );
        for (site, count) in rows.into_iter().take(n) {
            println!(
                "  {:>6.2}%  {}",
                count as f64 / total.max(1) as f64 * 100.0,
                site
            );
        }
    }

    #[global_allocator]
    static GLOBAL: CountingAlloc = CountingAlloc;
}

fn random_level() -> u8 {
    rand::rng().random_range(RANDOM_LEVEL_MIN..=RANDOM_LEVEL_MAX)
}

const MATCH_ID: &str = "dev-match-001";
const LEAGUE_SLUG: &str = "dev";
const CHUNK_DURATION_MS: u64 = 300_000;
const HOME_TEAM_NAME: &str = "Home FC";
const AWAY_TEAM_NAME: &str = "Away United";

/// The Bevy replay viewer, compiled to WebAssembly by `build.rs` and stored
/// gzipped — the same artefact the web server embeds. Empty when the machine
/// has no wasm target; the page then says so instead of hanging on a spinner.
const VIEWER_SCRIPT_GZ: &[u8] =
    include_bytes!(concat!(env!("OUT_DIR"), "/viewer/match_viewer.js.gz"));
const VIEWER_WASM_GZ: &[u8] =
    include_bytes!(concat!(env!("OUT_DIR"), "/viewer/match_viewer_bg.wasm.gz"));

/// The rendered viewer page, built once the match has been played.
static VIEWER_PAGE: std::sync::OnceLock<String> = std::sync::OnceLock::new();

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

/// Formation the synthetic squads line up in, read once from `TACTIC` (and
/// `TACTIC_B` for the away side).
///
/// `make_squad_simple` has always fielded a hardcoded 4-4-2 with TWO
/// forwards, and every calibration number in the project was measured on
/// it. The world fields whatever each club's `TacticsSelector` picked, and
/// most of those shapes carry ONE: a 4-2-3-1 is, counted in position
/// GROUPS, five defenders (the holding midfielder is one — see
/// `PlayerPositionType::position_group`), four midfielders and a lone
/// striker. That is a different quantity of attacking football, so it is
/// the first thing to vary whenever the harness and the world disagree
/// about goals.
///
/// `TACTIC=4231` — or any other name in the table — swaps BOTH the slot
/// list the players are generated for AND the `Tactics` handed to the
/// engine, so the shape a squad was built for and the shape it is asked to
/// play can never drift apart. `TACTIC_B` gives the away side a different
/// shape, which is what a real fixture looks like; unset, both sides play
/// `TACTIC`. Unset (or unrecognised) keeps 4-4-2.
///
/// `UNIFORM_PROFILE=1` additionally generates every outfielder from the
/// SAME source position, so the eleven differ only in the slot they were
/// handed. That separates the two things a shape change moves at once —
/// the attributes a slot's player is generated with, and the behaviour the
/// engine dispatches for that slot — and without it neither can be blamed.
struct HarnessTactic;

impl HarnessTactic {
    fn parse(name: &str) -> Option<MatchTacticType> {
        Some(match name.trim() {
            "442" => MatchTacticType::T442,
            "433" => MatchTacticType::T433,
            "451" => MatchTacticType::T451,
            "4231" => MatchTacticType::T4231,
            "352" => MatchTacticType::T352,
            "4141" => MatchTacticType::T4141,
            "4411" => MatchTacticType::T4411,
            "343" => MatchTacticType::T343,
            "4312" => MatchTacticType::T4312,
            "4222" => MatchTacticType::T4222,
            "442d" => MatchTacticType::T442Diamond,
            "442n" => MatchTacticType::T442Narrow,
            _ => return None,
        })
    }

    fn from_env(var: &'static str, fallback: MatchTacticType) -> MatchTacticType {
        match std::env::var(var) {
            Ok(name) => Self::parse(&name).unwrap_or_else(|| {
                eprintln!(
                    "{var}={name} not recognised — playing {}",
                    fallback.display_name()
                );
                fallback
            }),
            Err(_) => fallback,
        }
    }

    /// The home side's shape — and the default for both.
    fn selected() -> MatchTacticType {
        static PICK: std::sync::OnceLock<MatchTacticType> = std::sync::OnceLock::new();
        *PICK.get_or_init(|| Self::from_env("TACTIC", MatchTacticType::T442))
    }

    /// The away side's shape. Falls back to the home side's, so a run with
    /// only `TACTIC` set is still a like-for-like contest.
    fn selected_away() -> MatchTacticType {
        static PICK: std::sync::OnceLock<MatchTacticType> = std::sync::OnceLock::new();
        *PICK.get_or_init(|| Self::from_env("TACTIC_B", Self::selected()))
    }

    /// Team 1 is the home side in every mode that builds two squads.
    fn for_team(team_id: u32) -> MatchTacticType {
        if team_id == 1 {
            Self::selected()
        } else {
            Self::selected_away()
        }
    }

    /// The eleven slots of a side's shape, in the engine's own order.
    fn positions(team_id: u32) -> &'static [PlayerPositionType; 11] {
        static HOME: std::sync::OnceLock<[PlayerPositionType; 11]> = std::sync::OnceLock::new();
        static AWAY: std::sync::OnceLock<[PlayerPositionType; 11]> = std::sync::OnceLock::new();
        let cell = if team_id == 1 { &HOME } else { &AWAY };
        cell.get_or_init(|| *Tactics::new(Self::for_team(team_id)).positions())
    }

    /// The position a slot's player is GENERATED for. Normally the slot
    /// itself; under `UNIFORM_PROFILE` a single midfield profile for every
    /// outfielder, which holds attributes constant while the slot varies.
    fn generation_position(slot: PlayerPositionType) -> PlayerPositionType {
        static UNIFORM: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
        let uniform =
            *UNIFORM.get_or_init(|| std::env::var("UNIFORM_PROFILE").ok().as_deref() == Some("1"));
        if !uniform || slot.is_goalkeeper() {
            slot
        } else {
            PlayerPositionType::MidfielderCenter
        }
    }

    /// Printed by every mode that builds squads, so a run's numbers can
    /// never be read back without knowing which shapes produced them.
    fn label() -> String {
        let (home, away) = (Self::selected(), Self::selected_away());
        if home == away {
            home.display_name().to_string()
        } else {
            format!("{} vs {}", home.display_name(), away.display_name())
        }
    }
}

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

/// Where the twenty-two invented footballers above are from, one each and in
/// the same order — a surname and a complexion that disagree read as a bug in
/// the thing being reviewed.
///
/// Pure buckets rather than real census percentages: this is a fixture, and
/// what it wants on the pitch is one of every phenotype at once rather than a
/// demographically accurate Brazil. The mixing the game does is
/// `CountrySkin`'s job, and it needs the country table this harness has no
/// copy of.
const HOMELANDS: [SkinDist; 22] = [
    SkinDist::pure(SkinBucket::Metis, Region::LatinAmerica), // Silva
    SkinDist::pure(SkinBucket::White, Region::LatinAmerica), // Martinez
    SkinDist::pure(SkinBucket::White, Region::WestEurope),   // Müller
    SkinDist::pure(SkinBucket::White, Region::SouthEurope),  // Rossi
    SkinDist::pure(SkinBucket::White, Region::WestEurope),   // Dupont
    SkinDist::pure(SkinBucket::White, Region::BritIsles),    // Smith
    SkinDist::pure(SkinBucket::Black, Region::Caribbean),    // Johnson
    SkinDist::pure(SkinBucket::White, Region::SouthEurope),  // Garcia
    SkinDist::pure(SkinBucket::White, Region::LatinAmerica), // Fernandez
    SkinDist::pure(SkinBucket::White, Region::EastEurope),   // Novak
    SkinDist::pure(SkinBucket::White, Region::EastEurope),   // Petrov
    SkinDist::pure(SkinBucket::White, Region::NorthEurope),  // Andersson
    SkinDist::pure(SkinBucket::Metis, Region::EastAsia),     // Tanaka
    SkinDist::pure(SkinBucket::Metis, Region::EastAsia),     // Kim
    SkinDist::pure(SkinBucket::Black, Region::LatinAmerica), // Santos
    SkinDist::pure(SkinBucket::White, Region::SouthEurope),  // Costa
    SkinDist::pure(SkinBucket::White, Region::WestEurope),   // Richter
    SkinDist::pure(SkinBucket::Black, Region::SubSaharan),   // Bernard
    SkinDist::pure(SkinBucket::White, Region::SouthEurope),  // Moretti
    SkinDist::pure(SkinBucket::White, Region::EastEurope),   // Kowalski
    SkinDist::pure(SkinBucket::White, Region::EastEurope),   // Ivanov
    SkinDist::pure(SkinBucket::White, Region::NorthEurope),  // Schmidt
];

#[derive(Serialize)]
struct PlayerJson {
    id: u32,
    shirt_number: u8,
    last_name: String,
    position: String,
    is_home: bool,
    /// Whether he is in the starting eleven rather than on the bench — the
    /// eleven a side that walk out before kick-off.
    starting: bool,
    skin: u8,
    hair: u8,
    eyes: u8,
}

impl PlayerJson {
    /// `name` indexes both tables above, so the man's name and his colouring
    /// come from the same country.
    fn new(
        id: u32,
        shirt_number: u8,
        name: usize,
        position: &str,
        is_home: bool,
        starting: bool,
    ) -> PlayerJson {
        let name = name % LAST_NAMES.len();
        let look = Appearance::of(id, HOMELANDS[name]);
        PlayerJson {
            id,
            shirt_number,
            last_name: LAST_NAMES[name].to_string(),
            position: position.to_string(),
            is_home,
            starting,
            skin: look.skin as u8,
            hair: look.hair as u8,
            eyes: look.eyes as u8,
        }
    }
}

#[derive(Serialize)]
struct GoalJson {
    player_id: u32,
    time: u64,
    is_auto_goal: bool,
}

/// The near misses the engine shortlisted — see `HighlightSelector`. The
/// harness carries them for the same reason it carries the goals: they are what
/// the timeline marks, and a match watched here should have the same reel on it
/// as one watched in the game.
#[derive(Serialize)]
struct ChanceJson {
    player_id: u32,
    time: u64,
}

/// Every change either side made. Carried for the same reason the chances
/// are: the timeline marks them, and the engine stops the match to play each
/// one out, so a harness that dropped them would show a replay that pauses
/// twelve seconds for no visible reason.
#[derive(Serialize)]
struct SubstitutionJson {
    player_in_id: u32,
    player_out_id: u32,
    time: u64,
    break_ms: u64,
}

#[derive(Serialize)]
struct MetadataJson {
    chunk_count: usize,
    chunk_duration_ms: u64,
    total_duration_ms: u64,
}

/// Mirror of `match_viewer::app::config::ViewerConfig`. `debug` is always on here:
/// the state labels, the speed control and the ball-coordinate readout are the
/// entire reason to look at a match in this harness rather than in the game.
#[derive(Serialize)]
struct ViewerConfigJson<'a> {
    canvas: &'a str,
    api_base: String,
    match_time_ms: u64,
    home: ViewerColorsJson,
    away: ViewerColorsJson,
    players: &'a [PlayerJson],
    goals: &'a [GoalJson],
    chances: &'a [ChanceJson],
    substitutions: &'a [SubstitutionJson],
    venue: VenueJson,
    debug: bool,
    /// Walk the two teams out before the replay starts.
    ///
    /// On by default here as well as in the game, so what the harness shows is
    /// what a player sees. **`OF_NO_LINEUP=1` turns it off** — fifteen seconds
    /// of ceremony in front of every run of a screenshot loop is fifteen
    /// seconds of ceremony, and the same A/B switch is how the substitution
    /// walk is compared against the engine without it.
    lineup: bool,
}

#[derive(Serialize)]
struct ViewerColorsJson {
    background: &'static str,
    foreground: &'static str,
}

/// The ground the fixture is played at, which decides how much stadium the
/// viewer builds and how many people it puts in it.
///
/// A great ground by default, because that is the picture the rest of this
/// harness is compared against. **`OF_SMALL_GROUND=1` plays the same match at
/// a non-league one** — five steps of terracing, a short wrap and a thin
/// crowd — which is the A/B switch for the other end of
/// `match_viewer::scene::crowd::Stature`, the same way `OF_NO_LINEUP` is for
/// the walk-out.
#[derive(Serialize)]
struct VenueJson {
    capacity: u32,
    attendance: u32,
    reputation: u16,
    visitor: u16,
    youth: bool,
}

impl VenueJson {
    fn from_env() -> Self {
        if std::env::var("OF_SMALL_GROUND").is_ok() {
            VenueJson {
                capacity: 1_400,
                attendance: 620,
                reputation: 2_100,
                visitor: 2_300,
                youth: false,
            }
        } else {
            VenueJson {
                capacity: 62_000,
                attendance: 54_000,
                reputation: 9_400,
                visitor: 9_200,
                youth: false,
            }
        }
    }
}

/// Builds the one page the harness serves: a score header, a canvas, and the
/// fixture handed to the viewer.
struct ViewerPage;

impl ViewerPage {
    fn render(
        home_goals: u8,
        away_goals: u8,
        level_a: u8,
        level_b: u8,
        match_time_ms: u64,
        goals: &[GoalJson],
        chances: &[ChanceJson],
        substitutions: &[SubstitutionJson],
        players: &[PlayerJson],
    ) -> String {
        let config = ViewerConfigJson {
            canvas: "#match-canvas",
            api_base: format!("/api/match/{}", MATCH_ID),
            match_time_ms,
            home: ViewerColorsJson {
                background: "#00307d",
                foreground: "#ffffff",
            },
            away: ViewerColorsJson {
                background: "#b33f00",
                foreground: "#ffffff",
            },
            players,
            goals,
            chances,
            substitutions,
            venue: VenueJson::from_env(),
            debug: true,
            lineup: std::env::var("OF_NO_LINEUP").is_err(),
        };

        let placeholder = if VIEWER_WASM_GZ.is_empty() {
            "match viewer was not built — run `rustup target add wasm32-unknown-unknown`, then rebuild"
        } else {
            "Loading match data…"
        };

        // `</` is escaped so a player name can never close the script element
        // early; `\/` is a legal JSON escape, so the viewer still reads it as
        // written.
        let config_json = serde_json::to_string(&config)
            .unwrap_or_else(|_| "null".to_string())
            .replace("</", "<\\/");

        include_str!("viewer.html")
            .replace("__HOME_NAME__", HOME_TEAM_NAME)
            .replace("__AWAY_NAME__", AWAY_TEAM_NAME)
            .replace("__HOME_GOALS__", &home_goals.to_string())
            .replace("__AWAY_GOALS__", &away_goals.to_string())
            .replace(
                "__SUBTITLE__",
                &format!("level {} vs level {}", level_a, level_b),
            )
            .replace("__PLACEHOLDER__", placeholder)
            .replace("__CONFIG__", &config_json)
    }
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
        let total = s.corners
            + s.crossing
            + s.dribbling
            + s.finishing
            + s.first_touch
            + s.free_kicks
            + s.heading
            + s.long_shots
            + s.long_throws
            + s.marking
            + s.passing
            + s.penalty_taking
            + s.tackling
            + s.technique
            + m.aggression
            + m.anticipation
            + m.bravery
            + m.composure
            + m.concentration
            + m.decisions
            + m.determination
            + m.flair
            + m.leadership
            + m.off_the_ball
            + m.positioning
            + m.teamwork
            + m.vision
            + m.work_rate
            + p.acceleration
            + p.agility
            + p.balance
            + p.jumping
            + p.natural_fitness
            + p.pace
            + p.stamina
            + p.strength
            + g.aerial_reach
            + g.command_of_area
            + g.communication
            + g.eccentricity
            + g.first_touch
            + g.handling
            + g.kicking
            + g.one_on_ones
            + g.passing
            + g.punching
            + g.reflexes
            + g.rushing_out
            + g.throwing;
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

/// Optional within-squad quality spread, in skill points of standard
/// deviation, read once from `SQUAD_SPREAD`.
///
/// `make_squad_simple` retargets EVERY player to exactly
/// `LevelSkillCurve::target_mean(level)`, so a uniform squad's only
/// intra-team variation is skill SHAPE (a player with higher passing has
/// correspondingly lower everything else). That makes any rating-vs-skill
/// correlation structurally ~0 for reasons that have nothing to do with
/// the engine — there is no quality axis to correlate against.
///
/// Real squads are not uniform: a mid-table top-flight XI runs from ~14
/// (the stars) to ~9 (the role players). `SQUAD_SPREAD=2` reproduces that
/// so the RATING vs SKILL CORRELATION block measures something real.
///
/// Default 0.0 — every historical calibration number in the project was
/// measured on uniform squads, and this must not silently move them.
struct SquadSpread;

impl SquadSpread {
    fn sd() -> f32 {
        static SD: std::sync::OnceLock<f32> = std::sync::OnceLock::new();
        *SD.get_or_init(|| {
            std::env::var("SQUAD_SPREAD")
                .ok()
                .and_then(|v| v.parse::<f32>().ok())
                .unwrap_or(0.0)
                .clamp(0.0, 5.0)
        })
    }

    /// Triangular jitter (sum of two uniforms) — bell-ish without needing
    /// a normal sampler, and bounded so no player leaves the 1..20 band.
    fn jitter() -> f32 {
        let sd = Self::sd();
        if sd <= 0.0 {
            return 0.0;
        }
        let mut rng = rand::rng();
        let u: f32 = rng.random_range(-1.0..1.0);
        let v: f32 = rng.random_range(-1.0..1.0);
        (u + v) * sd
    }

    /// Apply the spread to an already-retargeted player.
    fn apply(skills: &mut PlayerSkills, level: u8) {
        if Self::sd() <= 0.0 {
            return;
        }
        let target = (LevelSkillCurve::target_mean(level) + Self::jitter()).clamp(2.0, 18.5);
        LevelSkillCurve::retarget(skills, target);
    }
}

/// **Pin ONE attribute across a whole side, and only that attribute.**
///
/// The sensitivity instrument. "Is this attribute load-bearing" is not a
/// question the level sweep can answer — moving the level moves all
/// forty-nine at once, so every KPI moves and none of them is
/// attributable. The only way to see a single attribute's channel is to
/// hold the squads identical and move that one value on one side.
///
/// ```text
///   OF_PIN=<attr>:<home>:<away>[:<unit>][,<attr>:<home>:<away>[:<unit>]…]
///   OF_PIN=flair:6:18                    home flair 6, away 18
///   OF_PIN=marking:6:18:def              defenders only (asymmetry check)
///   OF_PIN=teamwork:18:6,positioning:6:18    the SWAP test (see below)
/// ```
///
/// `<unit>` is `all` (default), `out` (outfield), `gk`, `def`, `mid` or
/// `fwd`. Applied AFTER the generator has retargeted the squad, so it
/// overwrites rather than being averaged away, and it deliberately does
/// NOT re-run `LevelSkillCurve::retarget` — the point is to move one
/// attribute with the other forty-eight held exactly where they were.
///
/// Level 14's generator mean is 11.8, so `6:18` is the symmetric pin
/// around the calibration division.
///
/// Note the pin therefore moves the squad's mean slightly, and so moves
/// `MatchStandard` by about `(pin - 11.8)/49` normalised units. That is
/// a tenth of an attribute point either way for a 6↔18 pin and it is the
/// honest reading — the pinned side really is a fraction better.
///
/// Pin the SAME value on both sides (`OF_PIN=flair:6:6`) to measure the
/// attribute's league-wide effect instead of its head-to-head one.
///
/// # The swap test
///
/// Two attributes are DEGENERATE when the engine has one channel wearing
/// two names. Uniform squads cannot show it — every attribute is equal,
/// so exchanging two of them is a no-op. Two runs do:
///
/// ```text
///   arm A: OF_PIN=teamwork:18:6,positioning:6:18
///   arm B: OF_PIN=teamwork:6:18,positioning:18:6
/// ```
///
/// Both arms give each side the same TOTAL of the two attributes and the
/// same squad mean; only which name carries which value changes. If the
/// two arms come out statistically identical, the pair is one channel.
struct SkillPin;

/// Which players on a side a pin applies to.
#[derive(Clone, Copy, PartialEq)]
enum PinUnit {
    All,
    Outfield,
    Goalkeeper,
    Defenders,
    Midfielders,
    Forwards,
}

impl PinUnit {
    fn parse(s: &str) -> PinUnit {
        match s {
            "out" | "outfield" => PinUnit::Outfield,
            "gk" | "keeper" | "goalkeeper" => PinUnit::Goalkeeper,
            "def" | "defenders" | "d" => PinUnit::Defenders,
            "mid" | "midfielders" | "m" => PinUnit::Midfielders,
            "fwd" | "forwards" | "f" => PinUnit::Forwards,
            _ => PinUnit::All,
        }
    }

    fn covers(self, pos: PlayerPositionType) -> bool {
        let group = pos.position_group();
        match self {
            PinUnit::All => true,
            PinUnit::Outfield => group != PlayerFieldPositionGroup::Goalkeeper,
            PinUnit::Goalkeeper => group == PlayerFieldPositionGroup::Goalkeeper,
            PinUnit::Defenders => group == PlayerFieldPositionGroup::Defender,
            PinUnit::Midfielders => group == PlayerFieldPositionGroup::Midfielder,
            PinUnit::Forwards => group == PlayerFieldPositionGroup::Forward,
        }
    }
}

/// A parsed `OF_PIN` directive.
#[derive(Clone, Copy)]
struct PinSpec {
    attr: &'static str,
    home: f32,
    away: f32,
    unit: PinUnit,
}

impl SkillPin {
    fn specs() -> &'static [PinSpec] {
        static SPECS: OnceLock<Vec<PinSpec>> = OnceLock::new();
        SPECS.get_or_init(|| {
            let Ok(raw) = env::var("OF_PIN") else {
                return Vec::new();
            };
            raw.split(',').filter_map(Self::parse_one).collect()
        })
    }

    fn parse_one(directive: &str) -> Option<PinSpec> {
        let directive = directive.trim();
        if directive.is_empty() {
            return None;
        }
        let mut parts = directive.split(':');
        let attr = parts.next()?.trim().to_ascii_lowercase();
        let attr = Self::canonical(&attr)?;
        let home: f32 = parts.next()?.trim().parse().ok()?;
        let away: f32 = parts
            .next()
            .and_then(|v| v.trim().parse().ok())
            .unwrap_or(home);
        let unit = parts
            .next()
            .map(|u| PinUnit::parse(u.trim()))
            .unwrap_or(PinUnit::All);
        eprintln!(
            "[OF_PIN] {attr} → home {home}, away {away}  (unit: {})",
            match unit {
                PinUnit::All => "all",
                PinUnit::Outfield => "outfield",
                PinUnit::Goalkeeper => "gk",
                PinUnit::Defenders => "def",
                PinUnit::Midfielders => "mid",
                PinUnit::Forwards => "fwd",
            }
        );
        Some(PinSpec {
            attr,
            home: home.clamp(1.0, 20.0),
            away: away.clamp(1.0, 20.0),
            unit,
        })
    }

    /// Map a user-typed name onto a `&'static str` key. Rejecting an
    /// unknown name outright (rather than silently pinning nothing) is
    /// the whole safety property: a typo in a measurement run that
    /// quietly measures the control twice is worse than no measurement.
    fn canonical(name: &str) -> Option<&'static str> {
        const NAMES: &[&str] = &[
            // Technical
            "corners",
            "crossing",
            "dribbling",
            "finishing",
            "first_touch",
            "free_kicks",
            "heading",
            "long_shots",
            "long_throws",
            "marking",
            "passing",
            "penalty_taking",
            "tackling",
            "technique",
            // Mental
            "aggression",
            "anticipation",
            "bravery",
            "composure",
            "concentration",
            "decisions",
            "determination",
            "flair",
            "leadership",
            "off_the_ball",
            "positioning",
            "teamwork",
            "vision",
            "work_rate",
            // Physical
            "acceleration",
            "agility",
            "balance",
            "jumping",
            "natural_fitness",
            "pace",
            "stamina",
            "strength",
            "match_readiness",
            // Goalkeeping (prefixed — `first_touch` and `passing` exist on
            // both scales and must not be ambiguous)
            "gk_aerial_reach",
            "gk_command_of_area",
            "gk_communication",
            "gk_eccentricity",
            "gk_first_touch",
            "gk_handling",
            "gk_kicking",
            "gk_one_on_ones",
            "gk_passing",
            "gk_punching",
            "gk_reflexes",
            "gk_rushing_out",
            "gk_throwing",
        ];
        let hit = NAMES.iter().find(|n| **n == name).copied();
        if hit.is_none() {
            eprintln!(
                "[OF_PIN] unknown attribute {name:?} — this directive is DROPPED. \
                 Nothing will be pinned for it, so the arm you are about to run is \
                 the control. GK attributes are prefixed gk_ (gk_handling)."
            );
        }
        hit
    }

    /// Apply every parsed pin to one player. `team_id` 1 is the home side.
    fn apply(skills: &mut PlayerSkills, team_id: u32, pos: PlayerPositionType) {
        for spec in Self::specs() {
            Self::apply_one(skills, team_id, pos, *spec);
        }
    }

    fn apply_one(skills: &mut PlayerSkills, team_id: u32, pos: PlayerPositionType, spec: PinSpec) {
        if !spec.unit.covers(pos) {
            return;
        }
        let v = if team_id == 1 { spec.home } else { spec.away };
        let t = &mut skills.technical;
        let m = &mut skills.mental;
        let p = &mut skills.physical;
        let g = &mut skills.goalkeeping;
        match spec.attr {
            "corners" => t.corners = v,
            "crossing" => t.crossing = v,
            "dribbling" => t.dribbling = v,
            "finishing" => t.finishing = v,
            "first_touch" => t.first_touch = v,
            "free_kicks" => t.free_kicks = v,
            "heading" => t.heading = v,
            "long_shots" => t.long_shots = v,
            "long_throws" => t.long_throws = v,
            "marking" => t.marking = v,
            "passing" => t.passing = v,
            "penalty_taking" => t.penalty_taking = v,
            "tackling" => t.tackling = v,
            "technique" => t.technique = v,
            "aggression" => m.aggression = v,
            "anticipation" => m.anticipation = v,
            "bravery" => m.bravery = v,
            "composure" => m.composure = v,
            "concentration" => m.concentration = v,
            "decisions" => m.decisions = v,
            "determination" => m.determination = v,
            "flair" => m.flair = v,
            "leadership" => m.leadership = v,
            "off_the_ball" => m.off_the_ball = v,
            "positioning" => m.positioning = v,
            "teamwork" => m.teamwork = v,
            "vision" => m.vision = v,
            "work_rate" => m.work_rate = v,
            "acceleration" => p.acceleration = v,
            "agility" => p.agility = v,
            "balance" => p.balance = v,
            "jumping" => p.jumping = v,
            "natural_fitness" => p.natural_fitness = v,
            "pace" => p.pace = v,
            "stamina" => p.stamina = v,
            "strength" => p.strength = v,
            "match_readiness" => p.match_readiness = v,
            "gk_aerial_reach" => g.aerial_reach = v,
            "gk_command_of_area" => g.command_of_area = v,
            "gk_communication" => g.communication = v,
            "gk_eccentricity" => g.eccentricity = v,
            "gk_first_touch" => g.first_touch = v,
            "gk_handling" => g.handling = v,
            "gk_kicking" => g.kicking = v,
            "gk_one_on_ones" => g.one_on_ones = v,
            "gk_passing" => g.passing = v,
            "gk_punching" => g.punching = v,
            "gk_reflexes" => g.reflexes = v,
            "gk_rushing_out" => g.rushing_out = v,
            "gk_throwing" => g.throwing = v,
            _ => {}
        }
    }
}

/// Position-relevant RAW skill composite (1..20) for the rating-vs-skill
/// diagnostics. Mirrors the weights the engine's own composites use
/// (`ops::skill_composites`) so the number means the same thing the
/// engine acts on, but reads the raw attributes: the question these
/// diagnostics ask is "does a better player produce a better stat line",
/// which must not be confounded by fatigue / match-state the way
/// `effective_skill` is.
///
/// Wrapped on a zero-sized struct rather than left as loose helpers so
/// the composite definitions live in one place — they are the x-axis of
/// every correlation the harness prints.
struct SkillComposite;

impl SkillComposite {
    /// `pos_group` uses the harness convention: 0 GK, 1 DEF, 2 MID, 3 FWD.
    fn for_group(s: &PlayerSkills, pos_group: u8) -> f32 {
        match pos_group {
            0 => Self::goalkeeper(s),
            1 => Self::defender(s),
            2 => Self::midfielder(s),
            _ => Self::forward(s),
        }
    }

    /// Shot-stopping weights from `sc::gk_shot_stopping`.
    fn goalkeeper(s: &PlayerSkills) -> f32 {
        s.goalkeeping.reflexes * 0.30
            + s.goalkeeping.handling * 0.18
            + s.physical.agility * 0.16
            + s.mental.positioning * 0.10
            + s.mental.concentration * 0.10
            + s.mental.anticipation * 0.08
            + s.goalkeeping.one_on_ones * 0.08
    }

    fn defender(s: &PlayerSkills) -> f32 {
        s.technical.marking * 0.24
            + s.technical.tackling * 0.22
            + s.mental.positioning * 0.16
            + s.mental.anticipation * 0.14
            + s.technical.heading * 0.10
            + s.physical.strength * 0.08
            + s.mental.decisions * 0.06
    }

    fn midfielder(s: &PlayerSkills) -> f32 {
        s.technical.passing * 0.24
            + s.mental.vision * 0.18
            + s.technical.technique * 0.14
            + s.mental.decisions * 0.14
            + s.technical.first_touch * 0.12
            + s.mental.work_rate * 0.10
            + s.mental.anticipation * 0.08
    }

    fn forward(s: &PlayerSkills) -> f32 {
        s.technical.finishing * 0.32
            + s.mental.off_the_ball * 0.20
            + s.technical.technique * 0.14
            + s.mental.composure * 0.14
            + s.technical.first_touch * 0.12
            + s.physical.acceleration * 0.08
    }

    /// Shift every skill so the player's position composite lands
    /// EXACTLY on `target`, preserving the generated shape.
    ///
    /// Works because every composite above is a convex combination
    /// (weights sum to 1.0): shifting all skills by δ shifts the
    /// composite by δ. This is what makes the mixed-quality spotlight
    /// reproducible across runs — `LevelSkillCurve::retarget` pins the
    /// MEAN of 49 attributes, which still leaves the seven that matter
    /// for the position swinging by ±2 between draws, and a before/after
    /// comparison can't survive that much x-axis noise.
    fn pin(skills: &mut PlayerSkills, pos_group: u8, target: f32) {
        let delta = target - Self::for_group(skills, pos_group);
        LevelSkillCurve::shift_all(skills, delta);
    }

    /// Snapshot every starter's composite so the caller can join skills
    /// onto the post-match stat rows. Taken BEFORE the squad is moved
    /// into the engine.
    fn snapshot(squad: &MatchSquad) -> Vec<(u32, f32)> {
        squad
            .main_squad
            .iter()
            .map(|p| (p.id, Self::for_group(&p.skills, pos_group_of(p.id))))
            .collect()
    }
}

/// Streaming Pearson-r accumulator. Kept as a struct (not a pass over a
/// stored sample vector) so the per-position correlations can be merged
/// across the parallel match loop the same way the volume aggregates are.
#[derive(Clone, Copy, Default)]
struct Correlation {
    n: u32,
    sx: f64,
    sy: f64,
    sxx: f64,
    syy: f64,
    sxy: f64,
}

impl Correlation {
    fn push(&mut self, x: f32, y: f32) {
        let (x, y) = (x as f64, y as f64);
        self.n += 1;
        self.sx += x;
        self.sy += y;
        self.sxx += x * x;
        self.syy += y * y;
        self.sxy += x * y;
    }

    fn r(&self) -> f32 {
        if self.n < 3 {
            return 0.0;
        }
        let n = self.n as f64;
        let cov = self.sxy - self.sx * self.sy / n;
        let vx = self.sxx - self.sx * self.sx / n;
        let vy = self.syy - self.sy * self.sy / n;
        if vx <= 0.0 || vy <= 0.0 {
            return 0.0;
        }
        (cov / (vx * vy).sqrt()) as f32
    }

    fn mean_x(&self) -> f32 {
        if self.n == 0 {
            0.0
        } else {
            (self.sx / self.n as f64) as f32
        }
    }

    fn sd_x(&self) -> f32 {
        Self::sd(self.n, self.sx, self.sxx)
    }

    fn sd_y(&self) -> f32 {
        Self::sd(self.n, self.sy, self.syy)
    }

    fn sd(n: u32, s: f64, ss: f64) -> f32 {
        if n < 2 {
            return 0.0;
        }
        let n = n as f64;
        ((ss - s * s / n) / (n - 1.0)).max(0.0).sqrt() as f32
    }
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
    let main_squad: Vec<MatchPlayer> = HarnessTactic::positions(team_id)
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
            let mut player = generate_player(
                base_id + i as u32,
                HarnessTactic::generation_position(pos),
                lvl,
            );
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
            // Opt-in within-squad quality spread (default off). Applied
            // after the playmaker overrides so an explicitly-shaped
            // player keeps his shape, just at a jittered level.
            SquadSpread::apply(&mut player.skills, lvl);
            // …and the single-attribute pin LAST of all, because it is
            // the measurement and nothing may average it away.
            SkillPin::apply(&mut player.skills, team_id, pos);
            MatchPlayer::from_player(team_id, &player, pos, false, None)
        })
        .collect();

    MatchSquad {
        team_id,
        team_name: format!("Team {}", team_id),
        tactics: Tactics::new(HarnessTactic::for_team(team_id)),
        main_squad,
        substitutes: Vec::new(),
        captain_id: None,
        vice_captain_id: None,
        penalty_taker_id: None,
        free_kick_taker_id: None,
        selection_omissions: Vec::new(),
        coach_snapshot: None,
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
            let mp = MatchPlayer::from_player(team_id, &player, pos, false, None);
            players_json.push(PlayerJson::new(
                mp.id,
                (i + 1) as u8,
                name_offset + i,
                pos.get_short_name(),
                team_id == 1,
                true,
            ));
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
            let mp = MatchPlayer::from_player(team_id, &player, pos, true, None);
            // Register the sub in PLAYERS_DATA too — that's the lookup the
            // viewer uses to build a sprite when a new id appears in
            // position chunks mid-match.
            players_json.push(PlayerJson::new(
                mp.id,
                (12 + i) as u8,
                name_offset + 11 + i,
                pos.get_short_name(),
                team_id == 1,
                false,
            ));
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
        coach_snapshot: None,
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
    /// First-touch resolver outputs (real ~8-15 miscontrols/team).
    miscontrols: u32,
    heavy_touches: u32,
    /// Discipline (real: yellows ~1.8-2.2/team, reds ~0.08/team).
    yellow_cards: u32,
    red_cards: u32,
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
    /// Goal timing: (time_ms, is_home_team_scored). Used for the
    /// draw-inflation diagnostics: first-goal time, equalizer-response
    /// rate, lead-flip rate, scoring-cascade detection. Captured from
    /// `score.detail()` filtered to real goals (excluding own-goals to
    /// avoid attributing them to the wrong team in sequence analysis;
    /// own-goals are still counted in the final score).
    goal_events: Vec<(u64, bool)>,
    /// Raw scorer rows for the ASSIST ATTRIBUTION diagnostic:
    /// `(time_ms, scorer_id, is_auto_goal)`. Unlike `goal_events` this
    /// keeps the player id so an assist can be paired with the goal it
    /// belongs to and the two teams compared.
    goal_details: Vec<(u64, u32, bool)>,
    /// `(time_ms, assister_id)` from `score.detail()`. Paired against
    /// `goal_details` by timestamp so the diagnostic can report which
    /// LINE provides assists and — the actual bug hunt — how often the
    /// credited assister plays for the CONCEDING team.
    assist_details: Vec<(u64, u32)>,
    /// Per-position sums of every counter the rating model reads as
    /// VOLUME, for the RATING VOLUME PROFILE diagnostic. Index:
    /// 0=GK 1=DEF 2=MID 3=FWD.
    pos_volumes: [RatingVolumeAgg; 4],
    /// `(player_id, raw skill composite)` for both starting XIs, taken
    /// before kickoff. Joined onto `per_player` to measure whether the
    /// engine turns player QUALITY into a better stat line — the
    /// RATING vs SKILL CORRELATION block.
    per_player_skill: Vec<(u32, f32)>,
}

/// Per-position per-match sums of the rating-relevant volume counters.
/// The RATING VOLUME PROFILE diagnostic divides these by player-samples
/// to get per-player per-match means, compared against real-football
/// per-90 references — the calibration source for the engine→real
/// volume conversion in the rating pipeline (rating/volume.rs). If the
/// engine's emission rates drift, this block is where it shows first.
#[derive(Clone, Copy, Default)]
struct RatingVolumeAgg {
    samples: u32,
    tackles: u32,
    interceptions: u32,
    blocks: u32,
    clearances: u32,
    pressures: u32,
    succ_pressures: u32,
    key_passes: u32,
    passes_into_box: u32,
    prog_passes: u32,
    prog_carries: u32,
    dribbles: u32,
    crosses_completed: u32,
    shots_on_target: u32,
    passes_attempted: u64,
    passes_completed: u64,
    /// Own-box + six-yard defensive actions (the `danger_actions` and
    /// `zone_impact` family in rating/defending.rs + calibration.rs).
    danger_zone_actions: u32,
    ft_pressures_won: u32,
    ft_tackles: u32,
    mt_interceptions: u32,
    /// Tier-ladder route counts: how many player-samples cleared the
    /// Strong bar via routine_def >= 7 / zone_impact >= 2 (see
    /// rating/calibration.rs). At real volumes these are rare monster
    /// shifts; if large shares of ordinary matches clear them, the
    /// engine's counter emission is inflating the evidence ladder.
    routine_def_ge7: u32,
    zone_impact_ge2: u32,
}

impl RatingVolumeAgg {
    fn add(&mut self, s: &core::r#match::PlayerMatchEndStats) {
        let z = &s.zone_stats;
        self.samples += 1;
        self.tackles += s.tackles as u32;
        self.interceptions += s.interceptions as u32;
        self.blocks += s.blocks as u32;
        self.clearances += s.clearances as u32;
        self.pressures += s.pressures as u32;
        self.succ_pressures += s.successful_pressures as u32;
        self.key_passes += s.key_passes as u32;
        self.passes_into_box += s.passes_into_box as u32;
        self.prog_passes += s.progressive_passes as u32;
        self.prog_carries += s.progressive_carries as u32;
        self.dribbles += s.successful_dribbles as u32;
        self.crosses_completed += s.crosses_completed as u32;
        self.shots_on_target += s.shots_on_target as u32;
        self.passes_attempted += s.passes_attempted as u64;
        self.passes_completed += s.passes_completed as u64;
        let danger = (z.tackles_own_box
            + z.interceptions_own_box
            + z.blocks_own_box
            + z.clearances_own_box
            + z.tackles_own_six_yard
            + z.interceptions_own_six_yard
            + z.blocks_own_six_yard
            + z.clearances_own_six_yard) as u32;
        self.danger_zone_actions += danger;
        self.ft_pressures_won += z.pressures_won_final_third as u32;
        self.ft_tackles += z.tackles_final_third as u32;
        self.mt_interceptions += z.interceptions_middle_third as u32;
        let routine_def =
            (s.tackles + s.interceptions + s.blocks + s.clearances + s.successful_pressures) as u32;
        if routine_def >= 7 {
            self.routine_def_ge7 += 1;
        }
        if danger + z.pressures_won_final_third as u32 >= 2 {
            self.zone_impact_ge2 += 1;
        }
    }

    fn merge(&mut self, other: &RatingVolumeAgg) {
        self.samples += other.samples;
        self.tackles += other.tackles;
        self.interceptions += other.interceptions;
        self.blocks += other.blocks;
        self.clearances += other.clearances;
        self.pressures += other.pressures;
        self.succ_pressures += other.succ_pressures;
        self.key_passes += other.key_passes;
        self.passes_into_box += other.passes_into_box;
        self.prog_passes += other.prog_passes;
        self.prog_carries += other.prog_carries;
        self.dribbles += other.dribbles;
        self.crosses_completed += other.crosses_completed;
        self.shots_on_target += other.shots_on_target;
        self.passes_attempted += other.passes_attempted;
        self.passes_completed += other.passes_completed;
        self.danger_zone_actions += other.danger_zone_actions;
        self.ft_pressures_won += other.ft_pressures_won;
        self.ft_tackles += other.ft_tackles;
        self.mt_interceptions += other.mt_interceptions;
        self.routine_def_ge7 += other.routine_def_ge7;
        self.zone_impact_ge2 += other.zone_impact_ge2;
    }
}

/// Collect the per-position rating-volume sums for one match.
fn rating_volume_profile(result: &core::r#match::MatchResultRaw) -> [RatingVolumeAgg; 4] {
    let mut agg = [RatingVolumeAgg::default(); 4];
    for (id, s) in result.player_stats.iter() {
        if s.minutes_played == 0 {
            continue;
        }
        agg[pos_group_of(*id) as usize].add(s);
    }
    agg
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
        miscontrols: 0,
        heavy_touches: 0,
        yellow_cards: 0,
        red_cards: 0,
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
            ts.miscontrols += s.miscontrols as u32;
            ts.heavy_touches += s.heavy_touches as u32;
            ts.yellow_cards += s.yellow_cards as u32;
            ts.red_cards += s.red_cards as u32;
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
            let mut player = generate_player(base_id + i as u32, pos, level);
            // Opt-in within-squad quality spread, same as `make_squad_simple`.
            // Without it every player at a level is identical in overall
            // quality and the season rating-vs-skill correlation has no
            // quality axis to correlate against.
            SquadSpread::apply(&mut player.skills, level);
            MatchPlayer::from_player(id, &player, pos, false, None)
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
        coach_snapshot: None,
    }
}

struct LeagueMatch {
    home_idx: usize,
    away_idx: usize,
    home_goals: u8,
    away_goals: u8,
    per_player: Vec<(u32, u16, u16, f32, u8, f32, u16, u16)>,
    keepers: Vec<GkRow>,
    /// `(position_group, raw performance value)` per played player —
    /// the input side of the rating model, before standardising. This
    /// is what `PerformanceScale`'s mean / sd constants are derived
    /// from, and it must come from the same expression the rating
    /// consumes, hence `RatingContext::performance_value`.
    perf: Vec<(u8, f32)>,
}

/// Per-player raw performance values for one match, normalised through
/// the same engine→real volume conversion the rating call site uses.
fn perf_rows(
    result: &core::r#match::MatchResultRaw,
    home_goals: u8,
    away_goals: u8,
) -> Vec<(u8, f32)> {
    use core::r#match::engine::rating::{EngineVolumeCalibration, RatingContext};
    let mut rows = Vec::new();
    for (id, s) in result.player_stats.iter() {
        if s.minutes_played == 0 {
            continue;
        }
        let is_left = result.left_team_players.main.contains(id);
        let (tg, og) = if is_left {
            (home_goals, away_goals)
        } else {
            (away_goals, home_goals)
        };
        let n = EngineVolumeCalibration::normalize(s);
        rows.push((
            pos_group_of(*id),
            RatingContext::new(&n, tg, og).performance_value(),
        ));
    }
    rows
}

/// One keeper's line from one match — the columns the live site's
/// goalkeeper history table shows, plus the rating inputs behind them.
/// Collected per match so the season ladder can answer the question the
/// site poses directly: does a keeper who concedes more rate lower?
#[derive(Clone, Copy)]
struct GkRow {
    id: u32,
    conceded: u8,
    saves: u16,
    shots_faced: u16,
    command: u16,
    xg_prevented: f32,
    xg_faced: f32,
    errors_to_goal: u16,
    rating: f32,
    minutes: u16,
}

fn keeper_rows(
    result: &core::r#match::MatchResultRaw,
    home_goals: u8,
    away_goals: u8,
) -> Vec<GkRow> {
    let mut rows = Vec::new();
    for (id, s) in result.player_stats.iter() {
        if pos_group_of(*id) != 0 {
            continue;
        }
        let is_left = result.left_team_players.main.contains(id);
        let conceded = if is_left { away_goals } else { home_goals };
        rows.push(GkRow {
            id: *id,
            conceded,
            saves: s.saves,
            shots_faced: s.shots_faced,
            command: s.zone_stats.gk_command_actions,
            xg_prevented: s.xg_prevented,
            xg_faced: s.xg_faced,
            errors_to_goal: s.errors_leading_to_goal,
            rating: s.match_rating,
            minutes: s.minutes_played,
        });
    }
    rows
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
            let (hg, ag) = (score.home_team.get(), score.away_team.get());
            LeagueMatch {
                home_idx: h,
                away_idx: a,
                home_goals: hg,
                away_goals: ag,
                per_player: per_player_rows(&result),
                keepers: keeper_rows(&result, hg, ag),
                perf: perf_rows(&result, hg, ag),
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

    // ── RATING vs SKILL CORRELATION, at SEASON granularity ──────────────
    //
    // The same diagnostic `stats` prints, but the number that actually
    // matters. In `stats` the squads are rebuilt every match, so the only
    // correlation available is per player-MATCH — and single-match rating
    // noise (sd 0.6-0.9) swamps the quality signal at realistic
    // within-level skill spreads, especially for keepers, whose per-match
    // rating swings hardest on one goal conceded. Here the clubs are
    // built once and every player plays the whole season, so this is the
    // correlation between a player's quality and his AV RAT — which is
    // what the site displays and what the campaign is judged on.
    {
        let mut season: std::collections::HashMap<u32, (f32, f32)> =
            std::collections::HashMap::new();
        for m in &played {
            for &(id, _g, _sh, _xg, _grp, rating, minutes, _a) in &m.per_player {
                if minutes == 0 {
                    continue;
                }
                let e = season.entry(id).or_insert((0.0, 0.0));
                e.0 += rating * minutes as f32;
                e.1 += minutes as f32;
            }
        }
        let mut corr = [Correlation::default(); 4];
        for t in &teams {
            for p in &t.players {
                if let Some((points, weight)) = season.get(&p.id) {
                    if *weight <= 0.0 {
                        continue;
                    }
                    let grp = pos_group_of(p.id);
                    corr[grp as usize]
                        .push(SkillComposite::for_group(&p.skills, grp), points / weight);
                }
            }
        }
        println!("\n--- RATING vs SKILL CORRELATION (season averages) ---");
        println!(
            "  {:<4} {:>7} {:>6} {:>10} {:>10} {:>8}    healthy r ~0.30-0.50",
            "pos", "r", "n", "skill mean", "skill sd", "season sd"
        );
        for (i, label) in ["GK", "DEF", "MID", "FWD"].iter().enumerate() {
            let c = &corr[i];
            println!(
                "  {:<4} {:>7.3} {:>6} {:>10.2} {:>10.2} {:>8.2}",
                label,
                c.r(),
                c.n,
                c.mean_x(),
                c.sd_x(),
                c.sd_y(),
            );
        }
    }
    print_performance_scale(&played);
    print_keeper_season_ladder(&played, &teams);

    println!("\n  (Gls = full-season tally; includes penalties / set-pieces the engine produced.)");
}

// ── PERFORMANCE SCALE ───────────────────────────────────────────────────
//
// The measured per-position mean and standard deviation of the rating
// model's raw performance value. These two numbers per position ARE
// `PerformanceScale` in `rating/mod.rs`: the model standardises against
// them, so if they drift the whole band drifts with them. Re-derive
// here after any change to component weights or engine emission — it is
// the only re-tuning the model needs, because the anchor and the shape
// are independent of the scale.
fn print_performance_scale(played: &[LeagueMatch]) {
    let mut vals: [Vec<f32>; 4] = Default::default();
    for m in played {
        for &(grp, v) in &m.perf {
            vals[grp as usize].push(v);
        }
    }
    println!("\n--- PERFORMANCE SCALE (raw rating input, per player-match) ---");
    println!(
        "  {:<4} {:>8} {:>8} {:>8} {:>8} {:>8} {:>7}    -> PerformanceScale {{ mean, sd }}",
        "pos", "mean", "sd", "p10", "p50", "p90", "n"
    );
    for (i, label) in ["GK", "DEF", "MID", "FWD"].iter().enumerate() {
        let v = &mut vals[i];
        if v.is_empty() {
            continue;
        }
        let n = v.len() as f32;
        let mean = v.iter().sum::<f32>() / n;
        let var = v.iter().map(|x| (x - mean) * (x - mean)).sum::<f32>() / n;
        v.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        let p = |q: f32| v[(((v.len() - 1) as f32) * q).round() as usize];
        println!(
            "  {:<4} {:>8.3} {:>8.3} {:>8.3} {:>8.3} {:>8.3} {:>7}",
            label,
            mean,
            var.sqrt(),
            p(0.10),
            p(0.50),
            p(0.90),
            v.len()
        );
    }
}

// ── KEEPER SEASON LADDER ────────────────────────────────────────────────
//
// The live site's goalkeeper history table, reproduced from engine data:
// one row per keeper with apps / conceded / clean sheets / AV RAT, sorted
// by goals conceded per game. The invariant it exists to check is the one
// a reader applies instinctively — a keeper who ships 1.5 a game must not
// out-rate one who ships 0.8 at a comparable club. Rating is the engine's
// `match_rating` (Stage 1+2+3); the site additionally applies the
// personality/morale shape, bounded to [-0.55, +0.40].
fn print_keeper_season_ladder(played: &[LeagueMatch], teams: &[LeagueTeam]) {
    #[derive(Default, Clone)]
    struct GkSeason {
        apps: u32,
        conceded: u32,
        clean_sheets: u32,
        saves: u32,
        faced: u32,
        command: u32,
        xg_prevented: f32,
        xg_faced: f32,
        errors: u32,
        rating_points: f32,
        rating_weight: f32,
        best: f32,
        worst: f32,
    }
    let mut by_gk: std::collections::HashMap<u32, GkSeason> = std::collections::HashMap::new();
    for m in played {
        for r in &m.keepers {
            if r.minutes == 0 {
                continue;
            }
            let e = by_gk.entry(r.id).or_insert(GkSeason {
                best: f32::MIN,
                worst: f32::MAX,
                ..Default::default()
            });
            e.apps += 1;
            e.conceded += r.conceded as u32;
            if r.conceded == 0 {
                e.clean_sheets += 1;
            }
            e.saves += r.saves as u32;
            e.faced += r.shots_faced.max(r.saves + r.conceded as u16) as u32;
            e.command += r.command as u32;
            e.xg_prevented += r.xg_prevented;
            e.xg_faced += r.xg_faced;
            e.errors += r.errors_to_goal as u32;
            let w = (r.minutes as f32 / 90.0).max(0.65);
            e.rating_points += r.rating * w;
            e.rating_weight += w;
            e.best = e.best.max(r.rating);
            e.worst = e.worst.min(r.rating);
        }
    }
    let mut rows: Vec<(u32, GkSeason)> = by_gk.into_iter().collect();
    rows.sort_by(|a, b| {
        let ka = a.1.conceded as f32 / a.1.apps.max(1) as f32;
        let kb = b.1.conceded as f32 / b.1.apps.max(1) as f32;
        ka.partial_cmp(&kb).unwrap_or(std::cmp::Ordering::Equal)
    });

    println!(
        "\n--- KEEPER SEASON LADDER (sorted by conceded per game — AV RAT must fall as you go down) ---"
    );
    println!(
        "  {:<12} {:>3} {:>4} {:>4} {:>4} {:>6} {:>5} {:>5} {:>6} {:>6} {:>7} {:>4} {:>7} {:>6} {:>6}",
        "club",
        "lvl",
        "Aps",
        "Con",
        "Cln",
        "con/g",
        "Sv",
        "Fcd",
        "save%",
        "xGp",
        "xG/shot",
        "Err",
        "AV RAT",
        "best",
        "worst"
    );
    // Spearman between conceded-per-game and season rating: −1.0 is a
    // perfectly ordered ladder, 0 is noise, positive means the model pays
    // keepers for being scored against.
    let mut xs: Vec<f32> = Vec::new();
    let mut ys: Vec<f32> = Vec::new();
    for (id, s) in &rows {
        if s.rating_weight <= 0.0 {
            continue;
        }
        let team_idx = (*id / 100).saturating_sub(1) as usize;
        let (club, lvl) = teams
            .get(team_idx)
            .map(|t| (t.name.as_str(), t.level))
            .unwrap_or(("?", 0));
        let av = s.rating_points / s.rating_weight;
        let cpg = s.conceded as f32 / s.apps.max(1) as f32;
        let save_pct = if s.faced > 0 {
            s.saves as f32 / s.faced as f32 * 100.0
        } else {
            0.0
        };
        let xg_per_shot = if s.faced > 0 {
            s.xg_faced / s.faced as f32
        } else {
            0.0
        };
        println!(
            "  {:<12} {:>3} {:>4} {:>4} {:>4} {:>6.2} {:>5} {:>5} {:>5.1}% {:>6.2} {:>7.3} {:>4} {:>7.2} {:>6.2} {:>6.2}",
            club,
            lvl,
            s.apps,
            s.conceded,
            s.clean_sheets,
            cpg,
            s.saves,
            s.faced,
            save_pct,
            s.xg_prevented,
            xg_per_shot,
            s.errors,
            av,
            s.best,
            s.worst
        );
        xs.push(cpg);
        ys.push(av);
    }
    if xs.len() >= 3 {
        let mut c = Correlation::default();
        for i in 0..xs.len() {
            c.push(xs[i], ys[i]);
        }
        println!(
            "  r(conceded/game, AV RAT) = {:+.3}   spread p90-p10 = {:.2}   (real football: r ≈ −0.6..−0.8)",
            c.r(),
            {
                let mut v = ys.clone();
                v.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
                let p = |q: f32| v[(((v.len() - 1) as f32) * q).round() as usize];
                p(0.9) - p(0.1)
            }
        );
    }
}

// ───────────────────────────────────────────────────────────────────────────
// Seeded benchmark — `dev_match bench [N] [level]`
//
// Runs N matches SINGLE-THREADED with fixed per-match seeds and
// fixed-skill (calibrated, condition-normalised) squads. Primary use: a
// low-variance A/B TIMING harness for engine optimizations — `per_match`
// is stable (~1%) across runs and across builds, so a real speedup shows
// up clearly.
//
// NOTE: `checksum` / `avg_goals` are only a COARSE regression signal, not
// an exact bit-for-bit oracle: the engine still carries residual
// non-determinism beyond the seeded match RNG (e.g. HashMap iteration
// order and any thread-RNG paths), so the scoreline varies run-to-run even
// with identical squads + seed. Use it to catch GROSS calibration shifts;
// prove true neutrality with a targeted unit test (see e.g.
// `effective_skill_bit_identical_to_bands`) or the project's calibration
// suite.
// ───────────────────────────────────────────────────────────────────────────
/// Seeded timing + coarse calibration benchmark. Bundled into a struct so
/// the harness exposes no loose helper functions.
/// Printable names for the `compact_id()`s the engine's censuses are
/// keyed by.
struct StateNames;

impl StateNames {
    /// Map a `PlayerState::compact_id()` back to its printable name by
    /// walking the state registry — the ids are role-banded and sparse
    /// (retired states leave holes), so a hand-written table would go
    /// stale silently.
    fn of(id: u16) -> String {
        PlayerState::all()
            .into_iter()
            .find(|s| s.compact_id() == id)
            .map(|s| s.to_string())
            .unwrap_or_else(|| format!("state#{id}"))
    }
}

struct Bench;

impl Bench {
    fn run(n: usize, level: u8) {
        let level = if level == 0 { 14 } else { level };
        let start = std::time::Instant::now();
        let mut checksum: u64 = 0;
        let mut total_goals: u64 = 0;
        // Allocation counting starts AFTER squad construction of match 0
        // would be unfair; instead snapshot before the loop and divide by
        // n — squad building is ~1k allocs/match, noise next to the
        // engine's total.
        #[cfg(feature = "alloc-count")]
        let (allocs_before, bytes_before) = (
            alloc_count::ALLOC_CALLS.load(std::sync::atomic::Ordering::Relaxed),
            alloc_count::ALLOC_BYTES.load(std::sync::atomic::Ordering::Relaxed),
        );
        for i in 0..n {
            let mut home = make_squad_calibrated(1, level);
            let mut away = make_squad_calibrated(2, level);
            Self::fix_squad_deterministic(&mut home);
            Self::fix_squad_deterministic(&mut away);
            // Distinct, deterministic seed per match (golden-ratio mix).
            let seed = 0x9E37_79B9_7F4A_7C15u64.wrapping_mul(i as u64 + 1);
            let result = FootballEngine::<840, 545>::play_seeded(
                home,
                away,
                false,
                false,
                false,
                Some(seed),
            );
            let score = result.score.as_ref().unwrap();
            let h = score.home_team.get() as u64;
            let a = score.away_team.get() as u64;
            total_goals += h + a;
            checksum = checksum
                .wrapping_mul(1_000_003)
                .wrapping_add(h.wrapping_mul(131).wrapping_add(a).wrapping_add(i as u64));
        }
        let secs = start.elapsed().as_secs_f64();
        #[cfg(feature = "alloc-count")]
        {
            let calls =
                alloc_count::ALLOC_CALLS.load(std::sync::atomic::Ordering::Relaxed) - allocs_before;
            let bytes =
                alloc_count::ALLOC_BYTES.load(std::sync::atomic::Ordering::Relaxed) - bytes_before;
            println!(
                "ALLOC calls={} bytes={} per_match_calls={:.0} per_match_bytes={:.0}",
                calls,
                bytes,
                calls as f64 / n.max(1) as f64,
                bytes as f64 / n.max(1) as f64
            );
            alloc_count::dump_sites(30);
        }
        println!(
            "BENCH n={} level={} time={:.3}s per_match={:.4}s total_goals={} avg_goals={:.2} checksum={:#018x}",
            n,
            level,
            secs,
            secs / n.max(1) as f64,
            total_goals,
            total_goals as f64 / n.max(1) as f64,
            checksum
        );
    }

    /// Normalise the RNG-derived (non-skill) fields the engine reads during
    /// a match so a calibrated squad is as deterministic as possible.
    /// `make_squad_calibrated` already pins every skill; this pins
    /// condition / jadedness / traits / birth_date / fatigue carry-ins,
    /// which `generate_player` otherwise rolls randomly.
    fn fix_squad_deterministic(squad: &mut MatchSquad) {
        for mp in squad
            .main_squad
            .iter_mut()
            .chain(squad.substitutes.iter_mut())
        {
            mp.player_attributes.condition = 10_000;
            mp.player_attributes.jadedness = 0;
            mp.traits = Vec::new();
            mp.birth_date = NaiveDate::from_ymd_opt(1995, 1, 1).unwrap();
            mp.starting_condition = 10_000;
            mp.starting_recovery_debt = 0.0;
            mp.is_force_match_selection = false;
        }
    }
}

// ── gap: mixed-quality diagnostic ──────────────────────────────────────
//
// The `stats` harness only ever plays two squads of the SAME quality, so
// the scenario the live site actually reports — a weak young player in an
// otherwise senior XI — was unmeasured. Every calibration number the
// project owns describes equal-level football, where "keeper skill does
// nothing" is invisible because both keepers are equally good.
//
// `gap N level [slot]` builds two identical level-`level` XIs and then
// replaces ONE slot: Team 1 gets a senior-quality player in it, Team 2 a
// youth-quality one. Everything else — formation, tactics, the other ten
// players' level — is the same on both sides, so the difference between
// the two spotlight rows is attributable to the quality gap alone.
//
// Squads are built ONCE and cloned per match (like the league harness),
// so the rating means printed are genuine SEASON averages of two fixed
// players, not an average over freshly-drawn ones.

/// Which slot the harness downgrades on Team 2 / upgrades on Team 1.
#[derive(Clone, Copy, PartialEq)]
enum SpotlightSlot {
    Goalkeeper,
    CentreBack,
    CentreMid,
    Forward,
}

impl SpotlightSlot {
    fn parse(s: &str) -> Option<Self> {
        match s.to_ascii_lowercase().as_str() {
            "gk" | "keeper" | "goalkeeper" => Some(Self::Goalkeeper),
            "cb" | "def" | "defender" => Some(Self::CentreBack),
            "cm" | "mid" | "midfielder" => Some(Self::CentreMid),
            "fw" | "st" | "fwd" | "forward" => Some(Self::Forward),
            _ => None,
        }
    }

    /// Index into `POSITIONS_442` — also the player's id offset.
    fn slot_index(self) -> usize {
        match self {
            Self::Goalkeeper => 0,
            Self::CentreBack => 2, // DefenderCenterLeft
            Self::CentreMid => 6,  // MidfielderCenterLeft
            Self::Forward => 9,    // ForwardLeft
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Goalkeeper => "GK",
            Self::CentreBack => "CB",
            Self::CentreMid => "CM",
            Self::Forward => "FW",
        }
    }

    fn is_goalkeeper(self) -> bool {
        self == Self::Goalkeeper
    }
}

/// Per-player season accumulator for the spotlight rows.
#[derive(Default)]
struct SpotlightAgg {
    skill: f32,
    ratings: Vec<f32>,
    minutes: u32,
    // Keeper lanes.
    saves: u32,
    shots_faced: u32,
    conceded: u32,
    command_actions: u32,
    failed_claims_shot: u32,
    failed_claims_goal: u32,
    // Shared mistake lanes.
    errors_to_shot: u32,
    errors_to_goal: u32,
    // Outfield lanes.
    passes_attempted: u32,
    passes_completed: u32,
    miscontrols: u32,
    heavy_touches: u32,
    dribbles_ok: u32,
    dribbles_try: u32,
    key_passes: u32,
    def_actions: u32,
    goals: u32,
    assists: u32,
}

impl SpotlightAgg {
    fn add(&mut self, s: &core::r#match::PlayerMatchEndStats, conceded: u32) {
        if s.minutes_played == 0 {
            return;
        }
        let z = &s.zone_stats;
        self.ratings.push(s.match_rating);
        self.minutes += s.minutes_played as u32;
        self.saves += s.saves as u32;
        self.shots_faced += s.shots_faced as u32;
        self.conceded += conceded;
        self.command_actions += z.gk_command_actions as u32;
        self.failed_claims_shot += z.gk_failed_claims_to_shot as u32;
        self.failed_claims_goal += z.gk_failed_claims_to_goal as u32;
        self.errors_to_shot += s.errors_leading_to_shot as u32;
        self.errors_to_goal += s.errors_leading_to_goal as u32;
        self.passes_attempted += s.passes_attempted as u32;
        self.passes_completed += s.passes_completed as u32;
        self.miscontrols += s.miscontrols as u32;
        self.heavy_touches += s.heavy_touches as u32;
        self.dribbles_ok += s.successful_dribbles as u32;
        self.dribbles_try += s.attempted_dribbles as u32;
        self.key_passes += s.key_passes as u32;
        self.def_actions +=
            (s.tackles + s.interceptions + s.blocks + s.clearances + s.successful_pressures) as u32;
        self.goals += s.goals as u32;
        self.assists += s.assists as u32;
    }

    fn merge(&mut self, o: SpotlightAgg) {
        self.ratings.extend(o.ratings);
        self.minutes += o.minutes;
        self.saves += o.saves;
        self.shots_faced += o.shots_faced;
        self.conceded += o.conceded;
        self.command_actions += o.command_actions;
        self.failed_claims_shot += o.failed_claims_shot;
        self.failed_claims_goal += o.failed_claims_goal;
        self.errors_to_shot += o.errors_to_shot;
        self.errors_to_goal += o.errors_to_goal;
        self.passes_attempted += o.passes_attempted;
        self.passes_completed += o.passes_completed;
        self.miscontrols += o.miscontrols;
        self.heavy_touches += o.heavy_touches;
        self.dribbles_ok += o.dribbles_ok;
        self.dribbles_try += o.dribbles_try;
        self.key_passes += o.key_passes;
        self.def_actions += o.def_actions;
        self.goals += o.goals;
        self.assists += o.assists;
    }

    fn apps(&self) -> f32 {
        self.ratings.len().max(1) as f32
    }

    /// (mean, p10, p90) of the season's per-match ratings.
    fn rating_dist(&self) -> (f32, f32, f32) {
        if self.ratings.is_empty() {
            return (0.0, 0.0, 0.0);
        }
        let mut v = self.ratings.clone();
        v.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        let mean = v.iter().sum::<f32>() / v.len() as f32;
        let p = |q: f32| -> f32 {
            let idx = ((v.len() as f32 - 1.0) * q).round() as usize;
            v[idx.min(v.len() - 1)]
        };
        (mean, p(0.10), p(0.90))
    }

    fn save_pct(&self) -> f32 {
        if self.shots_faced == 0 {
            0.0
        } else {
            self.saves as f32 / self.shots_faced as f32 * 100.0
        }
    }

    fn pass_pct(&self) -> f32 {
        if self.passes_attempted == 0 {
            0.0
        } else {
            self.passes_completed as f32 / self.passes_attempted as f32 * 100.0
        }
    }
}

struct MixedQualityHarness;

impl MixedQualityHarness {
    /// Target mean skill for the two spotlight players. Chosen to bracket
    /// the realistic senior band: a first-choice top-flight player sits
    /// ~14-16 across his relevant attributes, an academy graduate thrown
    /// in at 17-18 years old sits ~6-8. Both are retargeted with the same
    /// `LevelSkillCurve::retarget` shift the rest of the harness uses, so
    /// the position SHAPE (a keeper stays keeper-shaped) is preserved.
    const SENIOR_MEAN: f32 = 15.0;
    const YOUTH_MEAN: f32 = 7.0;

    /// How many independent squad draws the run is averaged over. The
    /// ten non-spotlight players are built once per draw and reused for
    /// that draw's matches (so the spotlight rating really is a season
    /// average in a stable team), but a SINGLE draw bakes one random
    /// supporting cast into every number — enough to swamp the effect
    /// being measured. Six draws costs nothing and makes before/after
    /// comparisons attributable.
    const DRAWS: usize = 6;

    fn run(n_matches: usize, level: u8, slot: SpotlightSlot) {
        env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("error")).init();
        let n_threads = rayon::current_num_threads();
        let per_draw = (n_matches / Self::DRAWS).max(1);
        let total = per_draw * Self::DRAWS;
        println!(
            "Mixed-quality: {} matches ({} draws x {}), both XIs level {}, spotlight slot {} \
             (Team 1 senior {:.0} vs Team 2 youth {:.0})  (parallel: {} threads)",
            total,
            Self::DRAWS,
            per_draw,
            level,
            slot.label(),
            Self::SENIOR_MEAN,
            Self::YOUTH_MEAN,
            n_threads
        );
        println!();

        core::save_accounting_stats::reset();
        core::gk_claim_diag::reset();

        let senior_slot_id = 100 + slot.slot_index() as u32;
        let youth_slot_id = 200 + slot.slot_index() as u32;

        struct Row {
            senior_gk: SpotlightAgg,
            youth_gk: SpotlightAgg,
            senior_slot: SpotlightAgg,
            youth_slot: SpotlightAgg,
            home_goals: u32,
            away_goals: u32,
        }

        let mut senior_gk = SpotlightAgg::default();
        let mut youth_gk = SpotlightAgg::default();
        let mut senior_slot = SpotlightAgg::default();
        let mut youth_slot = SpotlightAgg::default();
        let mut home_goals = 0u32;
        let mut away_goals = 0u32;
        let mut skills = (0.0f32, 0.0f32, 0.0f32, 0.0f32);

        for _ in 0..Self::DRAWS {
            // Built once per draw, cloned per match — the spotlight
            // player must be the SAME player all season for his rating
            // mean to be a season mean.
            let senior = Self::build_team(1, level, slot, Self::SENIOR_MEAN);
            let youth = Self::build_team(2, level, slot, Self::YOUTH_MEAN);
            skills.0 += Self::skill_of(&senior, 0);
            skills.1 += Self::skill_of(&youth, 0);
            skills.2 += Self::skill_of(&senior, slot.slot_index());
            skills.3 += Self::skill_of(&youth, slot.slot_index());

            let rows: Vec<Row> = (0..per_draw)
                .into_par_iter()
                .map(|_| {
                    let home = Self::squad(&senior, 1);
                    let away = Self::squad(&youth, 2);
                    let result = FootballEngine::<840, 545>::play(home, away, false, false, false);
                    let score = result.score.as_ref().unwrap();
                    let hg = score.home_team.get() as u32;
                    let ag = score.away_team.get() as u32;
                    let mut row = Row {
                        senior_gk: SpotlightAgg::default(),
                        youth_gk: SpotlightAgg::default(),
                        senior_slot: SpotlightAgg::default(),
                        youth_slot: SpotlightAgg::default(),
                        home_goals: hg,
                        away_goals: ag,
                    };
                    // Team 1 concedes the away goals and vice versa.
                    if let Some(s) = result.player_stats.get(&100) {
                        row.senior_gk.add(s, ag);
                    }
                    if let Some(s) = result.player_stats.get(&200) {
                        row.youth_gk.add(s, hg);
                    }
                    if !slot.is_goalkeeper() {
                        if let Some(s) = result.player_stats.get(&senior_slot_id) {
                            row.senior_slot.add(s, ag);
                        }
                        if let Some(s) = result.player_stats.get(&youth_slot_id) {
                            row.youth_slot.add(s, hg);
                        }
                    }
                    row
                })
                .collect();

            for r in rows {
                senior_gk.merge(r.senior_gk);
                youth_gk.merge(r.youth_gk);
                senior_slot.merge(r.senior_slot);
                youth_slot.merge(r.youth_slot);
                home_goals += r.home_goals;
                away_goals += r.away_goals;
            }
        }
        let draws = Self::DRAWS as f32;
        senior_gk.skill = skills.0 / draws;
        youth_gk.skill = skills.1 / draws;
        senior_slot.skill = skills.2 / draws;
        youth_slot.skill = skills.3 / draws;
        let n_matches = total;

        let n = n_matches.max(1) as f32;
        println!(
            "Team 1 (senior {}) scored {:.2}/m, conceded {:.2}/m   |   \
             Team 2 (youth {}) scored {:.2}/m, conceded {:.2}/m",
            slot.label(),
            home_goals as f32 / n,
            away_goals as f32 / n,
            slot.label(),
            away_goals as f32 / n,
            home_goals as f32 / n,
        );

        Self::print_keeper_table(&senior_gk, &youth_gk, slot);
        if !slot.is_goalkeeper() {
            Self::print_outfield_table(&senior_slot, &youth_slot, slot);
        }
    }

    fn print_keeper_table(senior: &SpotlightAgg, youth: &SpotlightAgg, slot: SpotlightSlot) {
        println!();
        if slot.is_goalkeeper() {
            println!("--- KEEPER SPOTLIGHT (the quality gap under test) ---");
        } else {
            println!("--- KEEPERS (both at squad level — context row) ---");
        }
        println!(
            "  {:<8} {:>6} {:>7} {:>6} {:>6} {:>7} {:>8} {:>8} {:>8} {:>8} {:>8}",
            "side",
            "skill",
            "rating",
            "p10",
            "p90",
            "save%",
            "conc/m",
            "saves/m",
            "faced/m",
            "cmd/m",
            "err→gl"
        );
        for (label, a) in [("senior", senior), ("youth", youth)] {
            let (mean, p10, p90) = a.rating_dist();
            let apps = a.apps();
            println!(
                "  {:<8} {:>6.1} {:>7.2} {:>6.2} {:>6.2} {:>6.1}% {:>8.2} {:>8.2} {:>8.2} {:>8.2} {:>8.3}",
                label,
                a.skill,
                mean,
                p10,
                p90,
                a.save_pct(),
                a.conceded as f32 / apps,
                a.saves as f32 / apps,
                a.shots_faced as f32 / apps,
                a.command_actions as f32 / apps,
                a.errors_to_goal as f32 / apps,
            );
        }
        println!(
            "  mistake lanes per match — senior: err→shot {:.3} failed-claim→shot {:.3} \
             failed-claim→goal {:.3}",
            senior.errors_to_shot as f32 / senior.apps(),
            senior.failed_claims_shot as f32 / senior.apps(),
            senior.failed_claims_goal as f32 / senior.apps(),
        );
        println!(
            "                            youth : err→shot {:.3} failed-claim→shot {:.3} \
             failed-claim→goal {:.3}",
            youth.errors_to_shot as f32 / youth.apps(),
            youth.failed_claims_shot as f32 / youth.apps(),
            youth.failed_claims_goal as f32 / youth.apps(),
        );
        let (gathers, moments, flaps) = core::gk_claim_diag::snapshot();
        let (flap_shots_seen, flap_charged, flap_dropped) = core::gk_claim_diag::resolution();
        let matches = (senior.apps() + youth.apps()).max(1.0);
        println!(
            "  claim contest (both keepers): {:.2} gathers/m, {:.2} command moments/m, \
             {:.3} flaps/m ({:.1}% of moments)   real: ~1-3 claims/m, a handful of \
             flapped crosses a SEASON",
            gathers as f32 / matches * 2.0,
            moments as f32 / matches * 2.0,
            flaps as f32 / matches * 2.0,
            if moments == 0 {
                0.0
            } else {
                flaps as f32 / moments as f32 * 100.0
            },
        );
        println!(
            "  flap resolution (totals): {} flaps, {} shots seen while pending, \
             {} charged to-shot, {} dropped (late / own side)",
            flaps, flap_shots_seen, flap_charged, flap_dropped,
        );
        println!(
            "  real reference: within-league keeper save% spread ~58% (poor) → ~78% (elite); \
             errors→goal 0-1/season elite vs 3-6 weak young"
        );
    }

    fn print_outfield_table(senior: &SpotlightAgg, youth: &SpotlightAgg, slot: SpotlightSlot) {
        println!();
        println!(
            "--- {} SPOTLIGHT (the quality gap under test) ---",
            slot.label()
        );
        println!(
            "  {:<8} {:>6} {:>7} {:>6} {:>6} {:>7} {:>7} {:>7} {:>7} {:>7} {:>7} {:>7}",
            "side",
            "skill",
            "rating",
            "p10",
            "p90",
            "pass%",
            "misc/m",
            "drib%",
            "kp/m",
            "def/m",
            "G+A/m",
            "err/m"
        );
        for (label, a) in [("senior", senior), ("youth", youth)] {
            let (mean, p10, p90) = a.rating_dist();
            let apps = a.apps();
            let drib = if a.dribbles_try == 0 {
                0.0
            } else {
                a.dribbles_ok as f32 / a.dribbles_try as f32 * 100.0
            };
            println!(
                "  {:<8} {:>6.1} {:>7.2} {:>6.2} {:>6.2} {:>6.1}% {:>7.2} {:>6.1}% {:>7.2} {:>7.2} {:>7.2} {:>7.3}",
                label,
                a.skill,
                mean,
                p10,
                p90,
                a.pass_pct(),
                a.miscontrols as f32 / apps,
                drib,
                a.key_passes as f32 / apps,
                a.def_actions as f32 / apps,
                (a.goals + a.assists) as f32 / apps,
                a.errors_to_shot as f32 / apps,
            );
        }
        println!(
            "  weak-player drag lanes to watch: pass% below ~74, miscontrols accumulating, \
             failed dribbles, engagement penalty"
        );
    }

    /// Eleven players at `level`, with `slot`'s position composite
    /// pinned exactly to `slot_skill`.
    fn build_team(
        team_id: u32,
        level: u8,
        slot: SpotlightSlot,
        slot_skill: f32,
    ) -> Vec<MatchPlayer> {
        let base_id = team_id * 100;
        POSITIONS_442
            .iter()
            .enumerate()
            .map(|(i, &pos)| {
                let id = base_id + i as u32;
                let mut player = generate_player(id, pos, level);
                if i == slot.slot_index() {
                    // Retarget the overall level first (so the whole
                    // player is youth / senior, not just his headline
                    // attributes), then pin the composite exactly.
                    LevelSkillCurve::retarget(&mut player.skills, slot_skill);
                    SkillComposite::pin(&mut player.skills, pos_group_of(id), slot_skill);
                }
                MatchPlayer::from_player(team_id, &player, pos, false, None)
            })
            .collect()
    }

    fn squad(players: &[MatchPlayer], team_id: u32) -> MatchSquad {
        MatchSquad {
            team_id,
            team_name: format!("Team {}", team_id),
            tactics: Tactics::new(MatchTacticType::T442),
            main_squad: players.to_vec(),
            substitutes: Vec::new(),
            captain_id: None,
            vice_captain_id: None,
            penalty_taker_id: None,
            free_kick_taker_id: None,
            selection_omissions: Vec::new(),
            coach_snapshot: None,
        }
    }

    fn skill_of(players: &[MatchPlayer], idx: usize) -> f32 {
        players
            .get(idx)
            .map(|p| SkillComposite::for_group(&p.skills, pos_group_of(p.id)))
            .unwrap_or(0.0)
    }
}

fn print_usage() {
    eprintln!("Usage:");
    eprintln!("  dev_match                       open browser viewer (random squad levels)");
    eprintln!("  dev_match viewer [lvlA] [lvlB]  open browser viewer — levels random unless given");
    eprintln!(
        "  dev_match stats [N] [lvlA] [lvlB]  run N matches headless; per-match random levels"
    );
    eprintln!("                                      unless BOTH lvlA and lvlB are passed");
    eprintln!(
        "  dev_match levels [N] [minLvl] [maxLvl] [step]  divisional-flatness sweep: equal squads at"
    );
    eprintln!(
        "                                      each level; goals/match MUST be flat across the pyramid"
    );
    eprintln!("  dev_match league [teams] [rounds] [minLvl] [maxLvl]  full round-robin season");
    eprintln!(
        "                                      defaults: 20 teams, 2 rounds (38 games), levels 8–18"
    );
    eprintln!(
        "  dev_match audit_levels [N]      generator diagnostic: mean outfield skills per level (default 200 squads)"
    );
    eprintln!(
        "  dev_match audit_engine_gap [N] [lvlA] [lvlB]  engine diagnostic: direct-skill matches at supplied gap"
    );
    eprintln!(
        "                                      bypasses generator; reveals engine-only response to skill gap"
    );
    eprintln!(
        "  dev_match fwdpath [min] [level] forward path trace: where each forward runs over the first N minutes,"
    );
    eprintln!(
        "                                      distance-to-goal profile, box occupancy, and an ASCII map"
    );
    eprintln!(
        "  dev_match heat [N] [level] [min]  thermal map: per-slot occupancy grids, both sides folded to"
    );
    eprintln!(
        "                                      attack right; role identity, possession phases, team shape."
    );
    eprintln!(
        "                                      [min] limits each match to its first N minutes (0 = all);"
    );
    eprintln!("                                      OF_HEAT_JSON=<path> dumps every grid");
    eprintln!(
        "  dev_match sky [N] [level]       skied-ball trace: every flight that climbs past 12 m, and what launched it"
    );
    eprintln!(
        "  dev_match trace [N] [level]     runtime per-player trace: position flicker + state looping"
    );
    eprintln!(
        "  dev_match waypoints [N] [level] tactical-route census: route geometry, take rate per state,"
    );
    eprintln!(
        "                                      where the route walk comes to rest, route vs team shape"
    );
    eprintln!(
        "  dev_match subs [N] [level]      substitution-usage diagnostic: per-team subs distribution by result"
    );
    eprintln!(
        "  dev_match reel [N] [lvlA] [lvlB]  highlight-reel census: goals / chances / substitutions"
    );
    eprintln!(
        "                                      per match, and how much of the recording each holds"
    );
    eprintln!(
        "  dev_match gap [N] [level] [slot]  mixed-quality diagnostic: identical XIs except one slot"
    );
    eprintln!(
        "                                      slot = gk (default) | cb | cm | fw; Team 1 senior vs Team 2 youth"
    );
    eprintln!();
    eprintln!("Environment:");
    eprintln!(
        "  OF_PIN=<attr>:<home>:<away>[:<unit>][,…]   hold ONE attribute across a side. The"
    );
    eprintln!(
        "                                      sensitivity instrument: the level sweep moves all fifty"
    );
    eprintln!(
        "                                      at once, so it can never attribute a KPI to one of them."
    );
    eprintln!(
        "                                      unit = all (default) | out | gk | def | mid | fwd;"
    );
    eprintln!(
        "                                      GK attributes are prefixed gk_ (gk_handling). Team 1 is HOME."
    );
    eprintln!(
        "                                      Level 14's generator mean is 11.8, so 6:18 is symmetric."
    );
    eprintln!(
        "                                      Two directives run the SWAP test for a suspected"
    );
    eprintln!(
        "                                      degenerate pair:  OF_PIN=teamwork:18:6,positioning:6:18"
    );
    eprintln!(
        "  SQUAD_SPREAD=<sd>                   within-squad quality spread in skill points (default 0)"
    );
    eprintln!();
    eprintln!(
        "Random level range: {}–{} inclusive.",
        RANDOM_LEVEL_MIN, RANDOM_LEVEL_MAX
    );
    eprintln!("Viewer serves at http://localhost:18001");
}

// ── subs: substitution-usage diagnostic ────────────────────────────────
//
// Plays N matches with production-like squads (XI at `level`, bench 3
// levels weaker, kickoff condition 82-96%) and prints how many
// substitutions each team actually made, bucketed by the team's final
// result. The production symptom this chases: teams —
// disproportionately ones holding a lead — finishing with zero subs
// and an untouched bench. Real-world reference (5-sub era): ~4.5
// subs/team, zero-sub teams essentially nonexistent.
/// **The substitution census** — `dev_match subs N [level] [bench_gap]`.
///
/// The only harness that fields a bench, and therefore the only one that can
/// see anything about substitutions at all: `stats` plays eleven men with
/// nobody behind them, so the whole timing model is a no-op there.
///
/// What it answers is *when* changes are made and *whether the match moved
/// them* — the minute histogram, the per-slot spread, and the split between
/// sides that finished behind, level and ahead. A model that reads the game
/// scatters those; a model that reads a clock stacks them on a handful of
/// values, which is exactly what the fixed `55 / 65 / 75 / 85` windows did.
///
/// It also carries the correlation budget: team-goal rho and draw share, for
/// the bench score-visibility gate that no other harness can measure.
///
/// `bench_gap` is how many levels the bench sits below the XI — 3 is a harsh
/// squad, 1 or 2 a realistic one. It matters more than it looks: subbing
/// earlier means more weak-bench minutes, and at gap 3 that alone moves
/// goals/match and rho.
struct SubstitutionCensus;

impl SubstitutionCensus {
    fn run(n_matches: usize, level: u8, bench_gap: u8) {
        println!(
            "Substitution usage: {} matches, both squads level {}",
            n_matches, level
        );

        struct SubsRow {
            home_goals: u8,
            away_goals: u8,
            home_subs: usize,
            away_subs: usize,
            /// Every change made in the match, as `(team_id, minute, index-within-team,
            /// discretionary?)`. The minute is what the census is actually for: a
            /// substitution model that reads the game should scatter these, and one
            /// that reads a clock should stack them on a handful of values.
            events: Vec<(u32, u32, usize, bool)>,
        }

        // Production-like squad: XI at `level`, bench 3 levels weaker
        // (selection puts the best players on the pitch), and kickoff
        // condition in the 82-96% band the persistence layer actually
        // hands the engine mid-season (never a pristine 100%).
        let make_squad_production_like = |team_id: u32, level: u8, seed: usize| -> MatchSquad {
            let base_id = team_id * 100;
            let bench_level = level.saturating_sub(bench_gap).max(1);
            let cond = |k: usize| 8200 + ((seed * 7 + k * 131) % 1400) as i16;

            let main_squad: Vec<MatchPlayer> = POSITIONS_442
                .iter()
                .enumerate()
                .map(|(i, &pos)| {
                    let mut player = generate_player(base_id + i as u32, pos, level);
                    player.player_attributes.condition = cond(i);
                    MatchPlayer::from_player(team_id, &player, pos, false, None)
                })
                .collect();

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
                    let mut player = generate_player(base_id + 11 + i as u32, pos, bench_level);
                    player.player_attributes.condition = cond(11 + i).min(9800);
                    MatchPlayer::from_player(team_id, &player, pos, true, None)
                })
                .collect();

            MatchSquad {
                team_id,
                team_name: format!("Team {}", team_id),
                tactics: Tactics::new(MatchTacticType::T442),
                main_squad,
                substitutes,
                captain_id: None,
                vice_captain_id: None,
                penalty_taker_id: None,
                free_kick_taker_id: None,
                selection_omissions: Vec::new(),
                coach_snapshot: None,
            }
        };

        let rows: Vec<SubsRow> = (0..n_matches)
            .into_par_iter()
            .map(|i| {
                let home = make_squad_production_like(1, level, i);
                let away = make_squad_production_like(2, level, i + 1000);
                let result = FootballEngine::<840, 545>::play(home, away, false, false, false);
                let score = result.score.as_ref().unwrap();
                let home_subs = result
                    .substitutions
                    .iter()
                    .filter(|s| s.team_id == 1)
                    .count();
                let away_subs = result
                    .substitutions
                    .iter()
                    .filter(|s| s.team_id == 2)
                    .count();
                let mut seen = std::collections::HashMap::<u32, usize>::new();
                let events = result
                    .substitutions
                    .iter()
                    .map(|s| {
                        let idx = seen.entry(s.team_id).or_insert(0);
                        let n = *idx;
                        *idx += 1;
                        (
                            s.team_id,
                            (s.match_time_ms / 60_000) as u32,
                            n,
                            matches!(s.reason, core::r#match::SubstitutionReason::Discretionary),
                        )
                    })
                    .collect();
                SubsRow {
                    home_goals: score.home_team.get(),
                    away_goals: score.away_team.get(),
                    home_subs,
                    away_subs,
                    events,
                }
            })
            .collect();

        // Distribution of subs per team-match, overall and by result.
        let mut dist = [0usize; 7]; // 0..=5, index 6 = ">5"
        let mut by_result: std::collections::HashMap<&'static str, (usize, usize, usize)> =
            std::collections::HashMap::new(); // result -> (teams, total_subs, zero_sub_teams)

        for r in &rows {
            for (subs, gf, ga) in [
                (r.home_subs, r.home_goals, r.away_goals),
                (r.away_subs, r.away_goals, r.home_goals),
            ] {
                dist[subs.min(6)] += 1;
                let key = if gf > ga {
                    "win"
                } else if gf < ga {
                    "loss"
                } else {
                    "draw"
                };
                let e = by_result.entry(key).or_insert((0, 0, 0));
                e.0 += 1;
                e.1 += subs;
                if subs == 0 {
                    e.2 += 1;
                }
            }
        }

        let total_teams = rows.len() * 2;
        let total_subs: usize = rows.iter().map(|r| r.home_subs + r.away_subs).sum();
        let total_goals: u32 = rows
            .iter()
            .map(|r| r.home_goals as u32 + r.away_goals as u32)
            .sum();
        println!(
            "\nteam-matches: {}   avg subs/team: {:.2}   (real-world ~4.5)   goals/match: {:.2}",
            total_teams,
            total_subs as f32 / total_teams.max(1) as f32,
            total_goals as f32 / rows.len().max(1) as f32
        );
        println!("subs-count distribution (per team-match):");
        for (k, v) in dist.iter().enumerate() {
            let label = if k == 6 {
                ">5".to_string()
            } else {
                k.to_string()
            };
            println!(
                "  {:>2}: {:>4}  ({:.0}%)",
                label,
                v,
                *v as f32 / total_teams.max(1) as f32 * 100.0
            );
        }
        println!("\nby final result:");
        for key in ["win", "draw", "loss"] {
            if let Some((teams, subs, zeros)) = by_result.get(key) {
                println!(
                    "  {:>4}: {:>4} teams  avg {:.2} subs  zero-sub {:>3} ({:.0}%)",
                    key,
                    teams,
                    *subs as f32 / (*teams).max(1) as f32,
                    zeros,
                    *zeros as f32 / (*teams).max(1) as f32 * 100.0
                );
            }
        }

        // WHEN the changes happen. A model that reads the game scatters these;
        // a model that reads a clock stacks them on the minute the gate opens.
        let all: Vec<(u32, u32, usize, bool)> =
            rows.iter().flat_map(|r| r.events.iter().copied()).collect();
        if all.is_empty() {
            return;
        }
        let disc: Vec<&(u32, u32, usize, bool)> = all.iter().filter(|e| e.3).collect();

        // Correlation budget. Letting the bench read the scoreline earlier than
        // the eleven do is the one part of the timing rewrite that could leak
        // into the draw-correlation regime the 62' play gate exists to bound —
        // and the calibration harness cannot see it, because its squads have no
        // bench. This is the only place the question can be asked.
        {
            let n = rows.len() as f64;
            let (mut sh, mut sa, mut shh, mut saa, mut sha) = (0.0f64, 0.0, 0.0, 0.0, 0.0);
            for r in &rows {
                let (h, a) = (r.home_goals as f64, r.away_goals as f64);
                sh += h;
                sa += a;
                shh += h * h;
                saa += a * a;
                sha += h * a;
            }
            let (mh, ma) = (sh / n, sa / n);
            let cov = sha / n - mh * ma;
            let vh = (shh / n - mh * mh).max(1e-9);
            let va = (saa / n - ma * ma).max(1e-9);
            println!(
                "\nteam-goal correlation rho: {:+.3}  (real ~0.00)   draws {:.1}% (real ~25%)",
                cov / (vh * va).sqrt(),
                rows.iter().filter(|r| r.home_goals == r.away_goals).count() as f32 / n as f32
                    * 100.0
            );
        }

        println!("\nminute histogram (all changes, 5' buckets):");
        let mut buckets = [0usize; 25];
        for e in &all {
            buckets[((e.1 / 5) as usize).min(24)] += 1;
        }
        let peak = buckets.iter().copied().max().unwrap_or(1).max(1);
        for (b, &v) in buckets.iter().enumerate() {
            if v == 0 {
                continue;
            }
            let bar = "#".repeat((v * 40 / peak).max(1));
            println!(
                "  {:>3}-{:<3} {:>4} ({:>4.1}%) {}",
                b * 5,
                b * 5 + 4,
                v,
                v as f32 / all.len() as f32 * 100.0,
                bar
            );
        }

        // Per-slot spread. The 1st change of a side, the 2nd, the 3rd... If the
        // gate is what decides, each slot collapses onto a narrow band with a
        // hard floor and the modal minute carries a large share of the mass.
        println!(
            "\nby slot (discretionary only): n / mean / sd / p10 / p50 / p90 / min / max / mode"
        );
        for slot in 0..5usize {
            let mut mins: Vec<u32> = disc.iter().filter(|e| e.2 == slot).map(|e| e.1).collect();
            if mins.is_empty() {
                continue;
            }
            mins.sort_unstable();
            let n = mins.len();
            let mean = mins.iter().map(|&m| m as f64).sum::<f64>() / n as f64;
            let sd =
                (mins.iter().map(|&m| (m as f64 - mean).powi(2)).sum::<f64>() / n as f64).sqrt();
            let q = |p: f64| mins[((n as f64 * p) as usize).min(n - 1)];
            let mut freq = std::collections::HashMap::<u32, usize>::new();
            for &m in &mins {
                *freq.entry(m).or_default() += 1;
            }
            let (mode, mode_n) = freq
                .iter()
                .max_by_key(|&(_, &c)| c)
                .map(|(&m, &c)| (m, c))
                .unwrap();
            println!(
                "  #{}: {:>4}  {:>5.1}'  sd {:>4.1}  {:>3} {:>3} {:>3}  [{:>3}..{:>3}]  mode {}' ({:.0}% of slot, {} distinct minutes)",
                slot + 1,
                n,
                mean,
                sd,
                q(0.10),
                q(0.50),
                q(0.90),
                mins[0],
                mins[n - 1],
                mode,
                mode_n as f32 / n as f32 * 100.0,
                freq.len()
            );
        }

        // Does the game state move the clock at all? Compare the first change of
        // a side when it is behind vs level vs ahead at full time. A situational
        // model chases a deficit early; a clock model does not care.
        // Windows: how many separate stoppages a side interrupts. Real sides get
        // three plus the interval, so five changes have to share.
        {
            let mut window_counts = [0usize; 7];
            let mut ht = 0usize;
            for r in &rows {
                for team in [1u32, 2] {
                    let mut minutes: Vec<u32> = r
                        .events
                        .iter()
                        .filter(|e| e.0 == team)
                        .map(|e| e.1)
                        .collect();
                    minutes.dedup();
                    let distinct = {
                        let mut m = minutes.clone();
                        m.sort_unstable();
                        m.dedup();
                        m.len()
                    };
                    window_counts[distinct.min(6)] += 1;
                }
                ht += r.events.iter().filter(|e| e.1 == 45 || e.1 == 46).count();
            }
            let total = rows.len() * 2;
            let spread: String = window_counts
                .iter()
                .enumerate()
                .filter(|&(_, &v)| v > 0)
                .map(|(k, &v)| format!("{k}:{:.0}%", v as f32 / total as f32 * 100.0))
                .collect::<Vec<_>>()
                .join("  ");
            println!("\ndistinct change moments per team-match: {spread}");
            println!(
                "half-time changes: {} ({:.1}% of all, real ~10-14%)",
                ht,
                ht as f32 / all.len() as f32 * 100.0
            );
        }

        println!("\nfirst discretionary change by final result:");
        for (label, want) in [("behind", -1i32), ("level", 0), ("ahead", 1)] {
            let mut mins: Vec<u32> = Vec::new();
            for r in &rows {
                for (team, gf, ga) in [
                    (1u32, r.home_goals, r.away_goals),
                    (2, r.away_goals, r.home_goals),
                ] {
                    let sign = (gf as i32 - ga as i32).signum();
                    if sign != want {
                        continue;
                    }
                    if let Some(e) = r.events.iter().find(|e| e.0 == team && e.3) {
                        mins.push(e.1);
                    }
                }
            }
            if mins.is_empty() {
                continue;
            }
            mins.sort_unstable();
            let mean = mins.iter().map(|&m| m as f64).sum::<f64>() / mins.len() as f64;
            println!(
                "  {:>6}: n={:<4} mean {:.1}'  median {}'  min {}'",
                label,
                mins.len(),
                mean,
                mins[mins.len() / 2],
                mins[0]
            );
        }
    }
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
        // Divisional-flatness sweep - see `LevelSweep`.
        "levels" => {
            let n: usize = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(300);
            let lo: u8 = args.get(3).and_then(|s| s.parse().ok()).unwrap_or(4);
            let hi: u8 = args.get(4).and_then(|s| s.parse().ok()).unwrap_or(18);
            let step: u8 = args.get(5).and_then(|s| s.parse().ok()).unwrap_or(2);
            LevelSweep::run(n, lo, hi, step);
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
        // Headless replay dump: writes exactly the chunk files `viewer`
        // writes, then exits — no server, no browser. Use this when the
        // question is "what does the ball actually DO", because the answer
        // is in the recorded track and the track is what the viewer draws.
        "record" => {
            let level_a: Option<u8> = args.get(2).and_then(|s| s.parse().ok());
            let level_b: Option<u8> = args.get(3).and_then(|s| s.parse().ok());
            // `record 14 14 clipped` narrows the scope to the one the GAME
            // uses, so what lands in `match_results` is the reel a player
            // actually gets rather than the whole match the harness keeps.
            // The only way to check that every goal, chance and substitution
            // has footage behind its marker.
            let clipped = args.get(4).is_some_and(|s| s == "clipped");
            run_record(level_a, level_b, clipped);
        }
        // Deterministic seeded timing + calibration-neutrality benchmark.
        // What a clipped recording actually comes out holding, over N
        // matches: goals, near misses and substitutions as three counts on
        // the same rail. The only mode that can see whether the reel is a
        // highlight package or a bench report. See `ReelCensus`.
        "reel" => {
            let n: usize = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(40);
            let level_a: Option<u8> = args.get(3).and_then(|s| s.parse().ok());
            let level_b: Option<u8> = args.get(4).and_then(|s| s.parse().ok());
            ReelCensus::run(n, level_a, level_b);
        }
        "bench" => {
            let n: usize = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(30);
            let level: u8 = args.get(3).and_then(|s| s.parse().ok()).unwrap_or(14);
            Bench::run(n, level);
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
        // Save-contest diagnostic: dumps the two composites `SaveModel`
        // differences — the keeper's `gk_shot_stopping` and the
        // shooter's `shot_threat` — per level. They must TRACK each
        // other as the level rises, because the save model scores their
        // difference; a constant offset between them biases every duel
        // in the game, and a diverging one reintroduces the
        // cross-division save% drift the contest exists to remove.
        "audit_contest" => {
            let n: usize = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(60);
            run_audit_contest(n);
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
        // Substitution-usage diagnostic: plays N matches with full benches
        // and reports the per-team subs-count distribution split by final
        // result. Reproduces "some teams never sub" reports from production.
        // Where the forwards actually RUN, over a short window, and what
        // they were doing while they were there. See `run_forward_paths`.
        "fwdpath" => {
            let minutes = args.get(2).and_then(|v| v.parse().ok()).unwrap_or(5u64);
            let lvl = args.get(3).and_then(|v| v.parse().ok()).unwrap_or(14u8);
            run_forward_paths(minutes, lvl);
        }
        "paths" => {
            let n = args.get(2).and_then(|v| v.parse().ok()).unwrap_or(2usize);
            let lvl = args.get(3).and_then(|v| v.parse().ok()).unwrap_or(14u8);
            run_paths(n, lvl);
        }
        // Woodwork trace: plays matches sequentially and dumps the ball's
        // per-tick flight around every contact with a post or the bar —
        // the run-up, the rebound, and the four seconds after it. The only
        // mode that can answer "what does the ball DO once it comes off
        // the frame", because that is a sequence and every other counter
        // in the harness is a sum. See `core::frame_trace`.
        "woodwork" => {
            let n = args.get(2).and_then(|v| v.parse().ok()).unwrap_or(40usize);
            let lvl = args.get(3).and_then(|v| v.parse().ok()).unwrap_or(14u8);
            run_woodwork(n, lvl);
        }
        // The same trace, triggered on a ball crossing the goal line ABOVE
        // the bar. Answers "where does a skied shot actually end up", which
        // is a sequence — the award, the descent, the run-out and the
        // restart — and every counter in the harness is a sum.
        "overbar" => {
            let n = args.get(2).and_then(|v| v.parse().ok()).unwrap_or(6usize);
            let lvl = args.get(3).and_then(|v| v.parse().ok()).unwrap_or(14u8);
            run_over_the_bar(n, lvl);
        }
        // The same trace, triggered on the ball climbing through
        // `FrameTrace::SKY_HEIGHT`. Answers "what launched it and what
        // brought it back down" for a ball that never goes near the frame
        // or a line. See `run_skied`.
        "sky" => {
            let n = args.get(2).and_then(|v| v.parse().ok()).unwrap_or(2usize);
            let lvl = args.get(3).and_then(|v| v.parse().ok()).unwrap_or(14u8);
            SkiedBallTrace::run(n, lvl);
        }
        // The same trace, triggered on the goalkeeper taking the ball into
        // his hands. Answers "how did it get there" — the ticks before the
        // gather carry the shot, and the keeper's own gap / height / state
        // ride alongside every row, so a catch at full stretch and a ball
        // dragged into a man lying on the floor are told apart by reading.
        "gather" => {
            let n = args.get(2).and_then(|v| v.parse().ok()).unwrap_or(2usize);
            let lvl = args.get(3).and_then(|v| v.parse().ok()).unwrap_or(14u8);
            run_gather(n, lvl);
        }
        // Runtime per-player flicker / state-churn trace. Samples every
        // simulation tick inside the engine rather than reading the
        // deduped replay track, so on-the-spot jitter and per-tick state
        // oscillation are both visible. See `run_trace`.
        "trace" => {
            let n = args.get(2).and_then(|v| v.parse().ok()).unwrap_or(1usize);
            let lvl = args.get(3).and_then(|v| v.parse().ok()).unwrap_or(14u8);
            run_trace(n, lvl);
        }
        // Tactical-route census: whether the waypoint layer is consulted,
        // where along its route each player sits, and how far the route
        // target is from the anchor the team plan gave the same man on
        // the same tick. See `WaypointCensusRun`.
        "waypoints" | "routes" => {
            let n = args.get(2).and_then(|v| v.parse().ok()).unwrap_or(2usize);
            let lvl = args.get(3).and_then(|v| v.parse().ok()).unwrap_or(14u8);
            WaypointCensusRun::run(n, lvl);
        }
        // The thermal map: where every slot actually spends the match,
        // both sides folded into one frame, split by possession phase and
        // printed beside the figure a real one gives. The only mode that
        // can answer "does this player have a position at all" — every
        // other spatial counter here is a scalar. See `HeatCensusRun`.
        "heat" | "heatmap" => {
            let n: usize = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(1);
            let level: u8 = args.get(3).and_then(|s| s.parse().ok()).unwrap_or(14);
            let minutes: u64 = args.get(4).and_then(|s| s.parse().ok()).unwrap_or(0);
            HeatCensusRun::run(n, level, minutes);
        }
        "subs" => {
            let n: usize = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(100);
            let level: u8 = args.get(3).and_then(|s| s.parse().ok()).unwrap_or(14);
            let bench_gap: u8 = args.get(4).and_then(|s| s.parse().ok()).unwrap_or(3);
            SubstitutionCensus::run(n, level, bench_gap);
        }
        // Mixed-quality diagnostic: two identical XIs except one slot,
        // where Team 1 fields a senior-quality player and Team 2 a
        // youth-quality one. The only harness mode that can see whether
        // player QUALITY reaches the stat line (and therefore the
        // rating) — `stats` only ever plays equal-quality squads.
        "gap" | "stats-gap" => {
            let n: usize = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(200);
            let level: u8 = args.get(3).and_then(|s| s.parse().ok()).unwrap_or(14);
            let slot = args
                .get(4)
                .and_then(|s| SpotlightSlot::parse(s))
                .unwrap_or(SpotlightSlot::Goalkeeper);
            MixedQualityHarness::run(n, level, slot);
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
/// Dump the two sides of the save contest per level. `gk` is the mean
/// `gk_shot_stopping` over generated keepers; the rest are mean
/// `shot_threat` over outfield players by line. `gap` is `gk − FWD`,
/// the number the save multiplier actually reads for the shots that
/// matter most — it must stay FLAT across levels for save% to be
/// level-invariant.
fn run_audit_contest(n: usize) {
    println!(
        "Generating {n} squads per level (1..20). Save-contest composites — \
         `gap` must stay flat.\n"
    );
    println!(
        "{:>3} {:>7} {:>7} {:>7} {:>7} {:>8}",
        "lvl", "gk", "FWD", "MID", "DEF", "gap"
    );
    for level in 1u8..=20 {
        let (mut gk, mut gk_n) = (0.0f32, 0u32);
        let mut thr = [0.0f32; 3];
        let mut thr_n = [0u32; 3];
        for team_id in 0..n {
            let squad = make_squad_simple((team_id + 1) as u32, level);
            for mp in &squad.main_squad {
                match mp.tactical_position.current_position.position_group() {
                    PlayerFieldPositionGroup::Goalkeeper => {
                        gk += sc::gk_shot_stopping(mp, 45);
                        gk_n += 1;
                    }
                    group => {
                        let i = match group {
                            PlayerFieldPositionGroup::Forward => 0,
                            PlayerFieldPositionGroup::Midfielder => 1,
                            _ => 2,
                        };
                        thr[i] += sc::shot_threat(mp, 45);
                        thr_n[i] += 1;
                    }
                }
            }
        }
        let gk_mean = gk / gk_n.max(1) as f32;
        let m: Vec<f32> = (0..3).map(|i| thr[i] / thr_n[i].max(1) as f32).collect();
        println!(
            "{:>3} {:>7.3} {:>7.3} {:>7.3} {:>7.3} {:>+8.3}",
            level,
            gk_mean,
            m[0],
            m[1],
            m[2],
            gk_mean - m[0]
        );
    }
}

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

    // …and the same three defending attributes SPLIT BY LINE.
    //
    // The pooled row above cannot answer the question that decides
    // whether a skill-driven defensive model can be calibrated here at
    // all: does a generated STRIKER have a centre-back's `tackling`?
    // `generate_player` takes the position shape from `PlayerGenerator`
    // and then adds a single uniform delta to every skill
    // (`LevelSkillCurve::retarget`), so the shape survives as an additive
    // offset — but the size of that offset relative to the level target
    // is what says whether the harness can see a role difference at all.
    println!();
    println!("defending attributes by line (level 14) — tck / mrk / pos");
    let mut acc = [[0.0f32; 3]; 3];
    let mut n_line = [0u32; 3];
    for team_id in 0..n {
        let squad = make_squad_simple((team_id + 1) as u32, 14);
        for mp in &squad.main_squad {
            let g = mp.tactical_position.current_position.position_group();
            let idx = match g {
                PlayerFieldPositionGroup::Defender => 0,
                PlayerFieldPositionGroup::Midfielder => 1,
                PlayerFieldPositionGroup::Forward => 2,
                PlayerFieldPositionGroup::Goalkeeper => continue,
            };
            acc[idx][0] += mp.skills.technical.tackling;
            acc[idx][1] += mp.skills.technical.marking;
            acc[idx][2] += mp.skills.mental.positioning;
            n_line[idx] += 1;
        }
    }
    for (i, label) in ["DEF", "MID", "FWD"].iter().enumerate() {
        let d = n_line[i].max(1) as f32;
        println!(
            "  {:<4} {:>5.2} {:>5.2} {:>5.2}   ({} players)",
            label,
            acc[i][0] / d,
            acc[i][1] / d,
            acc[i][2] / d,
            n_line[i]
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
            MatchPlayer::from_player(team_id, &player, pos, false, None)
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
        coach_snapshot: None,
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

    // Score-correlation fingerprint for UNIFORM squads (the generator
    // is bypassed, every player carries identical per-level skills) —
    // isolates the engine's intrinsic response correlation from the
    // squad-shape variance the random generator adds in `stats` mode.
    // If rho here is much lower than `stats N L L` shows, the surplus
    // in stats mode is squad-tilt (attack-heavy squads are
    // defense-light because the mean is pinned), a harness artifact
    // rather than engine behavior.
    {
        let n_m = outcomes.len() as f64;
        let mean_a = outcomes.iter().map(|o| o.ha as f64).sum::<f64>() / n_m;
        let mean_b = outcomes.iter().map(|o| o.aa as f64).sum::<f64>() / n_m;
        let mut cov = 0.0;
        let mut va = 0.0;
        let mut vb = 0.0;
        for o in &outcomes {
            let da = o.ha as f64 - mean_a;
            let db = o.aa as f64 - mean_b;
            cov += da * db;
            va += da * da;
            vb += db * db;
        }
        let rho = cov / (va * vb).sqrt().max(1e-9);
        println!(
            "  UNIFORM-SQUAD rho: {:+.3}  var/mean A {:.2} B {:.2}  (vs stats-mode rho — the gap is squad-tilt artifact)",
            rho,
            (va / n_m) / mean_a.max(1e-9),
            (vb / n_m) / mean_b.max(1e-9),
        );
    }

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

// ── levels: the divisional-flatness instrument ─────────────────────────
//
// Sweeps `level` with both squads EQUAL at each point and reports what
// the scoreline actually looks like there. The question it answers is
// the one no other mode can: **does this engine play the same football
// in every division?**
//
// Real football says it must. Goals per match sit near 2.5-2.8 from the
// fourth tier to the Champions League; 0-0 is ~8% of matches and four or
// more goals ~22% at every level of the pyramid. What separates the
// divisions is the quality of the football, not the number of goals in
// it. So a column here that walks with `level` is an engine bug, and
// which column walks says which channel is coupled to squad quality.
//
// It exists because that coupling is invisible to every other harness
// mode. `stats` fixes one level and reads beautifully calibrated at 14
// while the same binary produced 3.5 goals a match at level 6 and 4.5 at
// 18; `league` mixes levels inside one table and averages the two ends
// together. Only a sweep sees it, and until this mode existed nobody had
// run one — the reported symptom was "lower divisions play 3-2 and the
// top flight plays 0-0", and it had been in the engine the whole time.
//
// Run it under `SQUAD_SPREAD` as well. A uniform squad and a
// realistically-spread one at the SAME mean quality should produce the
// same match; if they don't, the shot decision is convex in quality,
// which is the fingerprint of a decision living in the tail of a
// distribution rather than in its bulk.
struct LevelSweep;

/// One level's worth of the sweep.
#[derive(Default, Clone, Copy)]
struct LevelRow {
    level: u8,
    matches: u32,
    goals: u32,
    shots: u32,
    on_target: u32,
    saves: u32,
    interceptions: u32,
    passes_attempted: u32,
    passes_completed: u32,
    nil_nil: u32,
    draws: u32,
    high_scoring: u32,
    box_shot_share: f32,
    /// Corners awarded, and how many of them a cross was actually struck
    /// from. Divisional, because the corner is where the engine's
    /// absolute constants bite hardest: the delivery deadline
    /// (`CORNER_SHAPE_MAX_TICKS`) is a fixed 2.5 s laid over a
    /// pace-driven fetch-and-carry, and a routine picked by argmax over
    /// five scores can only ever return the one with the biggest base.
    corners_awarded: u64,
    corner_crosses: u64,
    /// Mean seconds a corner shape held, and the share released by the
    /// deadline rather than by somebody touching the ball.
    corner_shape_secs: f32,
    corner_shape_deadline_pct: f32,
    /// Routine mix, `[near, spot, far, short, edge]`, as shares.
    corner_routines: [f32; 5],
    /// The accuracy chain, per division. `exec` is the mean
    /// `execution_skill` at the strike — CENTRED against the match by
    /// `MatchStandard`, so it is supposed to read the same at every
    /// level; `edge` is the total miss penalty it produces, and `cond`
    /// the fatigue half of that, which is deliberately NOT centred.
    /// Together they say which term is pricing the pyramid.
    mean_execution: f32,
    mean_edge: f32,
    mean_condition: f32,
    wide_share: f32,
    over_share: f32,
    miskick_share: f32,
    /// Where the shot was AIMED after the three forced-miss rolls, as a
    /// share of shots struck. The column to read against the table's
    /// `OT%`, which counts shots that actually reached the frame: aim is
    /// what the shooter did, `OT%` is what happened to the ball, and the
    /// gap between them is flight, blocks and deflections.
    aim_on_frame: f32,
    /// Raw count of shots that reached the aiming code, so `aim%` and
    /// the table above's `OT%` can be checked to share a denominator
    /// before their difference is read as a leak.
    aim_shots: u64,
    mean_force: f32,
    /// Traffic: mean defenders in the lane of an off-target shot, and
    /// the share of ALL shots a deflection rescued onto the frame.
    lane_defenders: f32,
    deflected_share: f32,
    /// **Where a struck shot ended up**, as shares of shots struck —
    /// `reception_diag`'s lifecycle partition, per division.
    ///
    /// This is the column that closes the gap the accuracy table opens.
    /// The aim comes out flat at every level, so a divisional slope in
    /// the on-target rate (goals + saves, i.e. shots that REACHED the
    /// keeper) has to be something eating on-frame shots in flight, and
    /// it eats far more of them in the lower divisions.
    fate_struck: u64,
    fate_reached: f32,
    fate_claimed_def: f32,
    fate_claimed_att: f32,
    fate_stopped: f32,
    fate_out: f32,
    /// Mean distance a shot was struck from, and the same for the shots
    /// that actually resolved at the goal. If the second is shorter, the
    /// leak is distance-selective.
    struck_dist_m: f32,
    reached_dist_m: f32,
}

impl LevelSweep {
    /// Goals per match real football holds at every level of the
    /// pyramid, and the band this sweep is trying to keep the engine
    /// inside. The verdict below reports the SPREAD across levels
    /// against it, because the population level is `stats`'s question
    /// and divisional flatness is this mode's.
    const REAL_GOALS: f32 = 2.65;
    /// How far apart the best and worst level are allowed to be before
    /// the sweep calls the engine division-coupled. Two runs of the same
    /// binary carry ±0.13 goals (see the harness noise floor), so
    /// anything under ~0.4 across a whole sweep is indistinguishable
    /// from sampling.
    const FLAT_TOLERANCE: f32 = 0.40;

    fn run(n_matches: usize, min_level: u8, max_level: u8, step: u8) {
        env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("error")).init();
        let step = step.max(1);
        let levels: Vec<u8> = (min_level..=max_level).step_by(step as usize).collect();
        println!(
            "Level sweep: {} matches at each of {} levels ({}-{} step {}), squads EQUAL at every point.",
            n_matches,
            levels.len(),
            min_level,
            max_level,
            step
        );
        if SquadSpread::sd() > 0.0 {
            println!(
                "SQUAD_SPREAD={:.1} - squads are spread around each level's mean.",
                SquadSpread::sd()
            );
        }
        println!();

        let rows: Vec<LevelRow> = levels
            .iter()
            .map(|&l| Self::one_level(n_matches, l))
            .collect();

        println!(
            "{:>5} {:>8} {:>9} {:>7} {:>7} {:>7} {:>7} {:>7} {:>7} {:>8}",
            "level",
            "goals/m",
            "shots/tm",
            "OT%",
            "save%",
            "0-0%",
            "draw%",
            "4+%",
            "box%",
            "int/tm"
        );
        for r in &rows {
            let n = r.matches.max(1) as f32;
            let shots = r.shots.max(1) as f32;
            println!(
                "{:>5} {:>8.2} {:>9.1} {:>6.1}% {:>6.1}% {:>6.1}% {:>6.1}% {:>6.1}% {:>6.1}% {:>8.1}",
                r.level,
                r.goals as f32 / n,
                r.shots as f32 / n / 2.0,
                r.on_target as f32 / shots * 100.0,
                if r.on_target > 0 {
                    r.saves as f32 / r.on_target as f32 * 100.0
                } else {
                    0.0
                },
                r.nil_nil as f32 / n * 100.0,
                r.draws as f32 / n * 100.0,
                r.high_scoring as f32 / n * 100.0,
                r.box_shot_share * 100.0,
                r.interceptions as f32 / n / 2.0,
            );
        }

        // ── The accuracy chain, per division ──────────────────────────
        //
        // Every other column in the table above comes out flat — shots,
        // save%, conversion of the shots that reach the frame. The whole
        // divisional spread in GOALS is the on-target rate, and this is
        // what the on-target rate is made of.
        //
        // `exec` is the load-bearing one: `ShotSkillProfile` prices every
        // band against `MatchStandard`, so a level-6 striker among
        // level-6 opposition is supposed to read the same as a level-18
        // striker among level-18 opposition. If this column slopes, the
        // centring is leaking and the fix is upstream in the profile; if
        // it is flat and `edge` still slopes, the fatigue half is what
        // prices the division.
        println!();
        println!(
            "{:>5} {:>8} {:>8} {:>8} {:>8} {:>8} {:>9} {:>7} {:>6} {:>7} {:>7} {:>8}",
            "level", "exec", "edge", "cond", "wide%", "over%", "miskick%", "aim%", "lane", "defl%", "force", "struck"
        );
        for r in &rows {
            println!(
                "{:>5} {:>8.4} {:>8.4} {:>8.4} {:>7.1}% {:>7.1}% {:>8.1}% {:>6.1}% {:>6.2} {:>6.1}% {:>7.3} {:>8}",
                r.level,
                r.mean_execution,
                r.mean_edge,
                r.mean_condition,
                r.wide_share * 100.0,
                r.over_share * 100.0,
                r.miskick_share * 100.0,
                r.aim_on_frame * 100.0,
                r.lane_defenders,
                r.deflected_share * 100.0,
                r.mean_force,
                r.aim_shots,
            );
        }

        // ── Where a struck shot ended up, per division ────────────────
        //
        // `reached` is goal + keeper: the shots that got to the end of
        // their flight and were resolved at the goal. Everything else is
        // a shot that never got to be one, and the three columns after
        // it say who took it — an outfield defender mid-flight, one of
        // the shooter's own team-mates, or nobody at all (it stopped).
        //
        // Read against `aim%` above, which is flat. Any slope here is
        // the divisional spread, and `struck` vs `reached` distance says
        // whether the leak selects on range.
        println!();
        println!(
            "{:>5} {:>9} {:>10} {:>10} {:>9} {:>7} {:>10} {:>10} {:>8}",
            "level", "reached%", "def-claim%", "att-claim%", "stopped%", "out%", "struck m", "reached m", "n"
        );
        for r in &rows {
            println!(
                "{:>5} {:>8.1}% {:>9.1}% {:>9.1}% {:>8.1}% {:>6.1}% {:>10.1} {:>10.1} {:>8}",
                r.level,
                r.fate_reached * 100.0,
                r.fate_claimed_def * 100.0,
                r.fate_claimed_att * 100.0,
                r.fate_stopped * 100.0,
                r.fate_out * 100.0,
                r.struck_dist_m,
                r.reached_dist_m,
                r.fate_struck,
            );
        }

        // ── The corner, per division ──────────────────────────────────
        //
        // Split out rather than folded into the table above because it
        // answers a different question: not "does the engine score like
        // football here" but "does the SET PIECE work here". The corner
        // carries more absolute constants than anything else in the
        // engine — a fixed delivery deadline, five fixed routine bases —
        // and an absolute constant is exactly what prices a division
        // instead of a player.
        println!();
        println!(
            "{:>5} {:>9} {:>10} {:>9} {:>9}  {:>34}",
            "level", "corners/m", "crosses/c", "shape s", "deadline%", "routine mix N/S/F/Sh/E %"
        );
        for r in &rows {
            let n = r.matches.max(1) as f32;
            println!(
                "{:>5} {:>9.2} {:>10.2} {:>9.2} {:>8.0}%  {:>7.0} {:>6.0} {:>6.0} {:>6.0} {:>6.0}",
                r.level,
                r.corners_awarded as f32 / n,
                r.corner_crosses as f32 / r.corners_awarded.max(1) as f32,
                r.corner_shape_secs,
                r.corner_shape_deadline_pct,
                r.corner_routines[0],
                r.corner_routines[1],
                r.corner_routines[2],
                r.corner_routines[3],
                r.corner_routines[4],
            );
        }

        // ── Verdict ───────────────────────────────────────────────────
        //
        // Two numbers, and they are different questions. SPREAD is this
        // mode's own: how far apart the divisions are. LEVEL is
        // `stats`'s: whether the population is scoring like football. An
        // engine can pass one and fail the other, and reading only the
        // mean is how a 1.8-to-4.5 swing hid behind a calibrated 2.4.
        let per_match: Vec<f32> = rows
            .iter()
            .map(|r| r.goals as f32 / r.matches.max(1) as f32)
            .collect();
        let lo_i = per_match
            .iter()
            .enumerate()
            .min_by(|a, b| a.1.partial_cmp(b.1).unwrap())
            .map(|(i, _)| i)
            .unwrap_or(0);
        let hi_i = per_match
            .iter()
            .enumerate()
            .max_by(|a, b| a.1.partial_cmp(b.1).unwrap())
            .map(|(i, _)| i)
            .unwrap_or(0);
        let spread = per_match[hi_i] - per_match[lo_i];
        let mean = per_match.iter().sum::<f32>() / per_match.len().max(1) as f32;
        println!();
        println!(
            "  goals/match spread across levels: {:.2}  ({:.2} at level {} -> {:.2} at level {})",
            spread, per_match[lo_i], rows[lo_i].level, per_match[hi_i], rows[hi_i].level,
        );
        println!(
            "  FLATNESS: {}  (tolerance {:.2}; real football is flat across the pyramid)",
            if spread <= Self::FLAT_TOLERANCE {
                "PASS"
            } else {
                "FAIL - the engine plays a different sport in different divisions"
            },
            Self::FLAT_TOLERANCE,
        );
        println!(
            "  LEVEL:    mean {:.2} goals/match against a real ~{:.2}  (that one is `stats`'s question)",
            mean,
            Self::REAL_GOALS,
        );
    }

    fn one_level(n_matches: usize, level: u8) -> LevelRow {
        // The distance diagnostic is a process-global accumulator, so it
        // has to be zeroed BEFORE the level runs for `box%` to be this
        // level's mix rather than the sweep's running one. Same for the
        // set-piece counters behind the corner columns.
        core::time_band_diag::reset();
        core::mid_run_diag::reset();
        // The accuracy chain is what the on-target column is made of,
        // and on-target is the only column with a real slope left in it
        // — so it has to be this level's, not the sweep's running total.
        core::shot_accuracy_diag::reset();
        core::reception_diag::reset();

        let outcomes: Vec<(u8, u8, TeamStats, TeamStats)> = (0..n_matches)
            .into_par_iter()
            .map(|_| {
                let home = make_squad_simple(1, level);
                let away = make_squad_simple(2, level);
                let result = FootballEngine::<840, 545>::play(home, away, false, false, false);
                let score = result.score.as_ref().unwrap();
                (
                    score.home_team.get(),
                    score.away_team.get(),
                    team_stats(&result, 1),
                    team_stats(&result, 2),
                )
            })
            .collect();

        // Where the football was played, read off the shared distance
        // diagnostic rather than re-derived. It is the clearest single
        // signal in the sweep: the share of shots taken from inside the
        // box moved 85% -> 31% across the level range while goals moved
        // far less, so a flat goals column with a sliding `box%` is an
        // engine that has swapped one kind of football for another.
        let [dshots, ..] = core::time_band_diag::distance_snapshot();
        let all_shots: u64 = dshots.iter().sum();
        let box_shot_share = if all_shots > 0 {
            (dshots[0] + dshots[1] + dshots[2]) as f32 / all_shots as f32
        } else {
            0.0
        };

        let sa = core::shot_accuracy_diag::snapshot();
        let fate = core::reception_diag::fate_census();
        let fate_struck = fate[0].max(1) as f32;
        let struck = sa[0].max(1) as f32;
        let mut row = LevelRow {
            level,
            matches: outcomes.len() as u32,
            box_shot_share,
            mean_execution: core::shot_accuracy_diag::mean_execution(),
            mean_edge: core::shot_accuracy_diag::mean_edge(),
            mean_condition: core::shot_accuracy_diag::mean_condition_drag(),
            wide_share: sa[1] as f32 / struck,
            over_share: sa[2] as f32 / struck,
            miskick_share: sa[3] as f32 / struck,
            aim_on_frame: sa[5] as f32 / struck,
            aim_shots: sa[0],
            mean_force: core::shot_accuracy_diag::mean_force(),
            lane_defenders: core::shot_accuracy_diag::mean_lane_defenders(),
            deflected_share: core::shot_accuracy_diag::deflected_share(),
            fate_struck: fate[0],
            fate_reached: (fate[1] + fate[2]) as f32 / fate_struck,
            fate_claimed_def: fate[4] as f32 / fate_struck,
            fate_claimed_att: fate[5] as f32 / fate_struck,
            fate_stopped: fate[6] as f32 / fate_struck,
            fate_out: fate[3] as f32 / fate_struck,
            struck_dist_m: fate[9] as f32 / 100.0 / fate_struck * 0.125,
            reached_dist_m: if fate[1] + fate[2] > 0 {
                fate[10] as f32 / 100.0 / (fate[1] + fate[2]) as f32 * 0.125
            } else {
                0.0
            },
            ..Default::default()
        };
        for (hg, ag, h, a) in &outcomes {
            let total = *hg as u32 + *ag as u32;
            row.goals += total;
            row.nil_nil += (total == 0) as u32;
            row.draws += (hg == ag) as u32;
            row.high_scoring += (total >= 4) as u32;
            for t in [h, a] {
                row.shots += t.shots as u32;
                row.on_target += t.on_target as u32;
                row.saves += t.saves as u32;
                row.interceptions += t.interceptions;
                row.passes_attempted += t.passes_attempted;
                row.passes_completed += t.passes_completed;
            }
        }
        {
            use core::mid_run_diag::SetPieceDiag;
            use std::sync::atomic::Ordering;
            let sp = SetPieceDiag::snapshot();
            row.corners_awarded = core::mid_run_diag::CORNERS_AWARDED.load(Ordering::Relaxed);
            row.corner_crosses = core::mid_run_diag::CORNER_CROSS_SENT.load(Ordering::Relaxed);
            row.corner_shape_secs = sp[16] as f32 / 100.0 / 100.0;
            row.corner_shape_deadline_pct = if sp[18] == 0 {
                0.0
            } else {
                sp[17] as f32 * 100.0 / sp[18] as f32
            };
            let picked: u64 = sp[0] + sp[1] + sp[2] + sp[3] + sp[4];
            if picked > 0 {
                for (slot, v) in row.corner_routines.iter_mut().zip(sp.iter()) {
                    *slot = *v as f32 * 100.0 / picked as f32;
                }
            }
        }
        eprintln!("  levels: level {} done", level);
        row
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
    println!("  shape: {}", HarnessTactic::label());
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
    core::key_pass_diag::reset();
    core::assist_diag::reset();
    core::reception_diag::reset();
    core::flight_diag::FlightDiag::reset();
    core::teleport::TeleportCensus::reset();
    core::teleport::PlayerTeleportCensus::reset();
    BlockDiag::reset();
    core::helper_diag::reset();
    core::mid_run_diag::reset();
    core::mid_onball_diag::reset();
    core::stuck_exit_stats::reset();
    core::chase_diag::reset();
    core::dead_ball_diag::reset();
    core::time_band_diag::reset();
    core::r#match::TransitionGraph::reset();
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
            // Skills must be read before the squads are moved into the
            // engine; the post-match result carries stats, not attributes.
            let mut per_player_skill = SkillComposite::snapshot(&home);
            per_player_skill.extend(SkillComposite::snapshot(&away));
            let result = FootballEngine::<840, 545>::play(home, away, false, false, false);
            let score = result.score.as_ref().unwrap();
            let hg = score.home_team.get();
            let ag = score.away_team.get();
            let h = team_stats(&result, 1);
            let a = team_stats(&result, 2);
            let per_player = per_player_rows(&result);
            // Goal timeline: filter to real goals (skip own-goals so the
            // scorer-team attribution from player_id/100 is correct), then
            // sort by time so cascade / equalizer analysis is well-defined
            // even if goals were emitted out of order.
            let mut goal_events: Vec<(u64, bool)> = score
                .detail()
                .iter()
                .filter(|g| {
                    g.stat_type == core::r#match::player::statistics::MatchStatisticType::Goal
                        && !g.is_auto_goal
                })
                .map(|g| (g.time, g.player_id / 100 == 1))
                .collect();
            goal_events.sort_by_key(|e| e.0);
            // Assist attribution: keep the raw (time, id) rows for both
            // goals and assists so the report can pair them and see who
            // actually got credited.
            let goal_details: Vec<(u64, u32, bool)> = score
                .detail()
                .iter()
                .filter(|g| {
                    g.stat_type == core::r#match::player::statistics::MatchStatisticType::Goal
                })
                .map(|g| (g.time, g.player_id, g.is_auto_goal))
                .collect();
            let assist_details: Vec<(u64, u32)> = score
                .detail()
                .iter()
                .filter(|g| {
                    g.stat_type == core::r#match::player::statistics::MatchStatisticType::Assist
                })
                .map(|g| (g.time, g.player_id))
                .collect();
            MatchOutcome {
                idx: i,
                level_a: match_level_a,
                level_b: match_level_b,
                home_goals: hg,
                away_goals: ag,
                home: h,
                away: a,
                per_player,
                goal_events,
                goal_details,
                assist_details,
                pos_volumes: rating_volume_profile(&result),
                per_player_skill,
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
    // Why shots end up off target. The aim band is documented as NOT
    // being the population lever — these three forced-miss rolls are —
    // so this breaks the rate down into the terms that actually set it.
    {
        let sa = core::shot_accuracy_diag::snapshot();
        let struck = sa[0].max(1) as f32;
        println!(
            "  off-target causes  : wide {:.1}%  over-bar {:.1}%  miskick {:.1}%  \
             → aim between posts {:.1}%, on frame {:.1}%",
            sa[1] as f32 / struck * 100.0,
            sa[2] as f32 / struck * 100.0,
            sa[3] as f32 / struck * 100.0,
            sa[4] as f32 / struck * 100.0,
            sa[5] as f32 / struck * 100.0,
        );
        // The anchor the two forced-miss rolls are centred on, and the
        // number `POPULATION_EXECUTION` has to equal for the centring to
        // be true — a gap between them is a miss penalty (or a gift)
        // charged to every shooter in the game.
        //
        // ⚠ It WAS printed, three lines further down and inside the
        // finishing-tier block's `if`, worded as advice for a future
        // change rather than as a check on an existing constant. Nobody
        // read it against the constant, and the two drifted 0.622
        // against 0.550. Lifted out here, unconditional, and named after
        // the thing it sets.
        println!(
            "  mean execution     : {:.4}   ← set POPULATION_EXECUTION to this",
            core::shot_accuracy_diag::mean_execution()
        );
        // …and whether the man who struck it makes any difference. If
        // these three are the same number, finishing is decorative.
        let bf = core::shot_accuracy_diag::finishing_snapshot();
        if bf[0][0] > 0 && bf[2][0] > 0 {
            let pct = |t: usize| bf[t][1] as f32 / bf[t][0].max(1) as f32 * 100.0;
            println!(
                "  aim on frame by SHOOTER finishing: poor(≤8) {:.1}% ({})   \
                 ordinary {:.1}% ({})   elite(≥14) {:.1}% ({})",
                pct(0),
                bf[0][0],
                pct(1),
                bf[1][0],
                pct(2),
                bf[2][0]
            );
        }
        // ── SHOTS BY KIND ─────────────────────────────────────────────
        //
        // Every other shot census in this file buckets by DISTANCE or by
        // the decision that produced the shot. Neither can see what the
        // shot WAS — and the action is what decides which skills strike
        // it. `heading` won the duel that produced a corner header and
        // then `finishing` hit it, for as long as nothing printed this.
        //
        // Read `mean exec` first: it is the population anchor per action,
        // so a header executing on the same number as a foot shot means
        // the type is not reaching the profile at all.
        {
            let st = core::shot_accuracy_diag::shot_type_snapshot();
            let total: u64 = st.iter().map(|r| r[0]).sum();
            if total > 0 {
                println!();
                println!(
                    "  SHOTS BY KIND (the action, not the distance) — {} struck:",
                    total
                );
                println!(
                    "    {:<18} {:>7} {:>7} {:>10} {:>10}",
                    "type", "struck", "share", "on frame", "mean exec"
                );
                for (i, name) in core::shot_accuracy_diag::SHOT_TYPE_NAMES.iter().enumerate() {
                    if st[i][0] == 0 {
                        continue;
                    }
                    let n = st[i][0] as f32;
                    println!(
                        "    {:<18} {:>7} {:>6.1}% {:>9.1}% {:>10.3}",
                        name,
                        st[i][0],
                        n / total as f32 * 100.0,
                        st[i][1] as f32 / n * 100.0,
                        st[i][2] as f32 / 1000.0 / n,
                    );
                }
            }
        }
    }
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
    let total_miscontrols: u32 = outcomes
        .iter()
        .map(|o| o.home.miscontrols + o.away.miscontrols)
        .sum();
    let total_heavy: u32 = outcomes
        .iter()
        .map(|o| o.home.heavy_touches + o.away.heavy_touches)
        .sum();
    println!(
        "miscontrols/team    : {:.1}  (real ~8-15)",
        total_miscontrols as f32 / (2.0 * n)
    );
    println!(
        "heavy touches/team  : {:.1}  (first-touch resolver, ~2x miscontrols)",
        total_heavy as f32 / (2.0 * n)
    );
    let total_yellows: u32 = outcomes
        .iter()
        .map(|o| o.home.yellow_cards + o.away.yellow_cards)
        .sum();
    let total_reds: u32 = outcomes
        .iter()
        .map(|o| o.home.red_cards + o.away.red_cards)
        .sum();
    println!(
        "yellow cards/match  : {:.2}  (real ~3.5-4.5)",
        total_yellows as f32 / n
    );
    println!(
        "red cards/match     : {:.3}  (real ~0.15-0.20)",
        total_reds as f32 / n
    );
    {
        use std::sync::atomic::Ordering;
        let pens = core::mid_run_diag::PENALTY_AWARDED.load(Ordering::Relaxed);
        let dfks = core::mid_run_diag::DIRECT_FK_AWARDED.load(Ordering::Relaxed);
        let corners = core::mid_run_diag::CORNERS_AWARDED.load(Ordering::Relaxed);
        println!(
            "penalties/match     : {:.3}  (real ~0.25-0.30)",
            pens as f32 / n
        );
        println!(
            "direct FKs/match    : {:.1}  (real ~20-24 total FKs)",
            dfks as f32 / n
        );
        // ⚠ ~10-11 IS THE PER-MATCH FIGURE, NOT THE PER-TEAM ONE. This
        // line printed a per-TEAM number against it for a long time, and
        // the same doubling propagated into the endline census ("real ~21
        // + ~13") and into the comments of the code that supplies corners
        // — so the engine has been measured against a target twice the
        // real one. A Premier League match has ~10.4 corners TOTAL (the
        // standard betting line is 9.5-11.5); a team averages ~5.2, and
        // no side in the league averages 10.
        println!(
            "corners per team    : {:.1}  (real ~5.2)      per match: {:.1}  (real ~10.4)",
            corners as f32 / (2.0 * n),
            corners as f32 / n
        );
    }
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
    let mut draws_by_total: std::collections::BTreeMap<u8, u32> = std::collections::BTreeMap::new();
    for o in &outcomes {
        // Bucket as (lower, higher) so 2-1 and 1-2 land in same row —
        // we care about scoreline shape, not which team scored.
        let key = (
            o.home_goals.min(o.away_goals),
            o.home_goals.max(o.away_goals),
        );
        *scoreline_counts.entry(key).or_default() += 1;
        if o.home_goals == o.away_goals {
            *draws_by_total.entry(o.home_goals).or_default() += 1;
        }
    }
    println!();
    println!("--- SCORELINE distribution (sorted by frequency) ---");
    let mut scoreline_sorted: Vec<((u8, u8), u32)> = scoreline_counts.into_iter().collect();
    scoreline_sorted.sort_by(|a, b| b.1.cmp(&a.1));
    let total_n = n_matches as f32;
    for ((lo, hi), count) in scoreline_sorted.iter().take(15) {
        let pct = *count as f32 / total_n * 100.0;
        let kind = if lo == hi { "DRAW" } else { "DEC " };
        let bar: String = std::iter::repeat('#')
            .take((pct.round() as usize).min(40))
            .collect();
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

    // ── GOAL TIMELINE — diagnose WHEN draws happen ───────────────────
    //
    // Hypothesis: the draw inflation comes from scoring being too
    // CORRELATED across the two teams within a single match. If team A
    // scores a goal and team B equalises within X minutes far more
    // often than real football, the engine has a "response goal"
    // dynamic baked in (kickoff momentum, possession reset, etc).
    //
    // Reference distributions (Premier League aggregate):
    //   * First-goal time: median ~32 min, mean ~36 min (geometric
    //     spread because goals can happen anywhere). 0-15 min ~25%,
    //     15-30 ~24%, 30-45 ~20%, 45-60 ~15%, 60-75 ~10%, 75-90 ~6%.
    //     The dev-engine uses 90 min of sim time.
    //   * Equalizer-within-15-min rate: after a goal that puts a team
    //     ahead, ~28% of the time the trailing team equalises within
    //     15 min. In the engine if this clears ~50% the response-goal
    //     mechanism is too strong.
    //   * Lead-flips per match: a "flip" is when the team that's
    //     trailing goes ahead (rare in real football, ~7% of matches).
    //
    // Match clock: `total_match_time` (used as `GoalDetail.time` in
    // `events/players.rs::handle_goal_event`) is in MILLISECONDS — the
    // engine increments `total_match_time += MATCH_TIME_INCREMENT_MS`
    // each tick, and MATCH_TIME_INCREMENT_MS=10. So 1 minute = 60_000.
    // 90 minutes of match time = 5_400_000.
    const TICKS_PER_MIN: u64 = 60_000;
    let mut first_goal_buckets = [0u32; 7]; // 0-15, 15-30, ... 75-90, no goal
    let mut equalizers_within_15 = 0u32;
    let mut goals_that_could_be_equalised = 0u32;
    let mut quick_response_within_5min = 0u32;
    let mut lead_flips = 0u32;
    let mut matches_with_a_lead = 0u32;
    let mut goal_gap_total: u64 = 0;
    let mut goal_gap_count: u32 = 0;
    let mut score_state_neutral_first = 0u32; // matches where score was 0-0 at HT (>=45 min in)
    let mut total_matches_with_goals = 0u32;

    for o in &outcomes {
        // First goal time bucketing.
        if let Some(first) = o.goal_events.first() {
            let min = (first.0 / TICKS_PER_MIN) as u32;
            let bucket = (min / 15).min(5) as usize;
            first_goal_buckets[bucket] += 1;
            total_matches_with_goals += 1;
            // Was the half-time score 0-0?
            if min >= 45 {
                score_state_neutral_first += 1;
            }
        } else {
            first_goal_buckets[6] += 1;
        }

        // Walk through the goal stream tracking lead state.
        let mut home_g = 0u8;
        let mut away_g = 0u8;
        let mut last_leader: Option<bool> = None; // Some(true) home, Some(false) away
        let mut ever_had_lead = false;
        for window in o.goal_events.windows(2) {
            let gap = window[1].0.saturating_sub(window[0].0);
            goal_gap_total += gap;
            goal_gap_count += 1;
        }
        for &(time, home_scored) in &o.goal_events {
            // Record state BEFORE this goal — was a leader being equalised?
            let pre_diff = home_g as i16 - away_g as i16;
            // Apply the goal.
            if home_scored {
                home_g += 1;
            } else {
                away_g += 1;
            }
            let post_diff = home_g as i16 - away_g as i16;

            // Equalizer detection: previous goal put someone ahead, and
            // this goal restored parity within X minutes.
            if pre_diff != 0 && post_diff == 0 {
                // Lookup prior goal time (could be many goals back, but
                // the most recent one is what matters). Find the most
                // recent goal before `time`.
                // We iterate again, so we find the previous goal event:
                // this is the goal that PUT the now-equalising team behind.
                if let Some(prev_time) = o
                    .goal_events
                    .iter()
                    .take_while(|(t, _)| *t < time)
                    .last()
                    .map(|(t, _)| *t)
                {
                    let gap_ticks = time.saturating_sub(prev_time);
                    let gap_min = gap_ticks / TICKS_PER_MIN;
                    goals_that_could_be_equalised += 1;
                    if gap_min <= 15 {
                        equalizers_within_15 += 1;
                    }
                    if gap_min <= 5 {
                        quick_response_within_5min += 1;
                    }
                }
            }

            // Lead-flip: someone was ahead and now the other side is.
            if let Some(prev_leader) = last_leader {
                let now_leader = if post_diff > 0 {
                    Some(true)
                } else if post_diff < 0 {
                    Some(false)
                } else {
                    None
                };
                if let Some(now) = now_leader {
                    if now != prev_leader {
                        lead_flips += 1;
                    }
                }
            }
            if post_diff > 0 {
                last_leader = Some(true);
                ever_had_lead = true;
            } else if post_diff < 0 {
                last_leader = Some(false);
                ever_had_lead = true;
            }
        }
        if ever_had_lead {
            matches_with_a_lead += 1;
        }
    }
    println!();
    println!("--- GOAL TIMELINE diagnostics (draw-correlation hunt) ---");
    let bucket_labels = [
        "0-15  min",
        "15-30 min",
        "30-45 min",
        "45-60 min",
        "60-75 min",
        "75-90 min",
    ];
    let bucket_refs = [25, 24, 20, 15, 10, 6];
    println!("  First-goal time distribution (real PL reference):");
    for (i, label) in bucket_labels.iter().enumerate() {
        let n = first_goal_buckets[i];
        let pct = n as f32 / total_matches_with_goals.max(1) as f32 * 100.0;
        println!(
            "    {} : {:>4} ({:>5.1}%)  ref ~{}%",
            label, n, pct, bucket_refs[i]
        );
    }
    println!(
        "    no goal   : {:>4} ({:>5.1}%)",
        first_goal_buckets[6],
        first_goal_buckets[6] as f32 / total_n * 100.0,
    );
    println!(
        "    0-0 at HT : {:>4} ({:>5.1}%)  — first goal happens after minute 45",
        score_state_neutral_first,
        score_state_neutral_first as f32 / total_n * 100.0,
    );
    println!();
    println!("  Response-goal mechanics (the draw-cascade signal):");
    let equ_pct = if goals_that_could_be_equalised > 0 {
        equalizers_within_15 as f32 / goals_that_could_be_equalised as f32 * 100.0
    } else {
        0.0
    };
    let quick_pct = if goals_that_could_be_equalised > 0 {
        quick_response_within_5min as f32 / goals_that_could_be_equalised as f32 * 100.0
    } else {
        0.0
    };
    println!(
        "    after a go-ahead goal, equalizer within 15min: {:>4}/{:<4} ({:>5.1}%)  ref ~28%",
        equalizers_within_15, goals_that_could_be_equalised, equ_pct
    );
    println!(
        "    after a go-ahead goal, equalizer within  5min: {:>4}/{:<4} ({:>5.1}%)  ref ~10%",
        quick_response_within_5min, goals_that_could_be_equalised, quick_pct
    );
    let flip_pct = lead_flips as f32 / total_n * 100.0;
    println!(
        "    lead-flips per match (trailer goes ahead)   : {:>4} ({:>5.1}% of matches)  ref ~7%",
        lead_flips, flip_pct,
    );
    let avg_gap_min = if goal_gap_count > 0 {
        (goal_gap_total as f32 / goal_gap_count as f32) / TICKS_PER_MIN as f32
    } else {
        0.0
    };
    println!(
        "    avg gap between consecutive goals           : {:>5.1} min  ref ~28 min",
        avg_gap_min,
    );
    println!(
        "    matches that ever had a lead                : {:>4} ({:>5.1}% of matches)",
        matches_with_a_lead,
        matches_with_a_lead as f32 / total_n * 100.0,
    );

    // ── ALL-GOALS BY MINUTE — kickoff-flood hunt ──────────────────────
    //
    // The first-goal distribution alone can't tell "early minutes are
    // hot" apart from "scoring rate is uniform but high". This block
    // buckets EVERY goal by absolute minute (15-min bands + per-minute
    // fine grain over 0-15) and, separately, by time since the most
    // recent kickoff restart (match start or the goal before it). If
    // goals cluster within 1-3 minutes of a kickoff, the restart state
    // itself is generating chances — defensive shape after the reset,
    // not steady-state play, is what's broken.
    //
    // Real reference (Opta, big-5 leagues): goals per 15-min band rise
    // monotonically — roughly 11% / 14% / 16% / 15% / 18% / 26% with
    // injury time folded in. Minute 0-1 goals are ~0.5% of all goals.
    let mut goals_by_band = [0u32; 6];
    let mut goals_by_early_minute = [0u32; 15];
    let mut since_kickoff_buckets = [0u32; 5]; // <1, 1-2, 2-5, 5-10, 10+ min
    let mut total_goal_count = 0u32;
    for o in &outcomes {
        let mut prev_kickoff_ms: u64 = 0; // match start
        for &(time, _) in &o.goal_events {
            let min = (time / TICKS_PER_MIN) as usize;
            goals_by_band[(min / 15).min(5)] += 1;
            if min < 15 {
                goals_by_early_minute[min] += 1;
            }
            let since_kickoff_min =
                (time.saturating_sub(prev_kickoff_ms)) as f32 / TICKS_PER_MIN as f32;
            let b = if since_kickoff_min < 1.0 {
                0
            } else if since_kickoff_min < 2.0 {
                1
            } else if since_kickoff_min < 5.0 {
                2
            } else if since_kickoff_min < 10.0 {
                3
            } else {
                4
            };
            since_kickoff_buckets[b] += 1;
            total_goal_count += 1;
            prev_kickoff_ms = time; // play restarts with a kickoff after each goal
        }
    }
    println!();
    println!("  ALL goals by 15-min band (real ref ~11/14/16/15/18/26%):");
    for (i, label) in bucket_labels.iter().enumerate() {
        let nb = goals_by_band[i];
        println!(
            "    {} : {:>4} ({:>5.1}%)",
            label,
            nb,
            nb as f32 / total_goal_count.max(1) as f32 * 100.0
        );
    }
    println!("  goals in minutes 0-14, per minute:");
    let early_total: u32 = goals_by_early_minute.iter().sum();
    for (m, nb) in goals_by_early_minute.iter().enumerate() {
        let bar: String = std::iter::repeat('#').take((*nb as usize) / 3).collect();
        println!(
            "    min {:>2} : {:>4} ({:>4.1}% of all goals) {}",
            m,
            nb,
            *nb as f32 / total_goal_count.max(1) as f32 * 100.0,
            bar
        );
    }
    println!(
        "    minutes 0-14 hold {:.1}% of ALL goals (uniform would be ~16.7%)",
        early_total as f32 / total_goal_count.max(1) as f32 * 100.0
    );
    println!("  time from kickoff restart (match start / previous goal) to goal:");
    let kicklabels = ["< 1 min ", "1-2 min ", "2-5 min ", "5-10 min", "10+ min "];
    for (i, label) in kicklabels.iter().enumerate() {
        let nb = since_kickoff_buckets[i];
        println!(
            "    {} : {:>4} ({:>5.1}%)",
            label,
            nb,
            nb as f32 / total_goal_count.max(1) as f32 * 100.0
        );
    }

    // ── SCORE CORRELATION — the draw machine's fingerprint ───────────
    // Decomposes draw inflation into its two distinct causes:
    //   1. Within-match correlation of the two teams' goal counts
    //      (equalizer dynamics, shared match state). Real football has
    //      near-ZERO net correlation — independence + home asymmetry
    //      lands almost exactly on the real ~25% draw share.
    //   2. Marginal under-dispersion (variance/mean < 1 — compressed
    //      team totals; Poisson = 1.0, real slightly above 1).
    // "expected draws (indep)" recombines the OBSERVED marginals as if
    // the teams were independent: the gap between observed draws and
    // that number is pure correlation — the thing to kill.
    {
        let n_m = outcomes.len() as f64;
        let mut sum_h = 0.0;
        let mut sum_a = 0.0;
        let mut sum_hh = 0.0;
        let mut sum_aa = 0.0;
        let mut sum_ha = 0.0;
        let mut h_marg = [0f64; 12];
        let mut a_marg = [0f64; 12];
        for o in &outcomes {
            let h = o.home_goals as f64;
            let a = o.away_goals as f64;
            sum_h += h;
            sum_a += a;
            sum_hh += h * h;
            sum_aa += a * a;
            sum_ha += h * a;
            h_marg[(o.home_goals as usize).min(11)] += 1.0;
            a_marg[(o.away_goals as usize).min(11)] += 1.0;
        }
        let mean_h = sum_h / n_m;
        let mean_a = sum_a / n_m;
        let var_h = sum_hh / n_m - mean_h * mean_h;
        let var_a = sum_aa / n_m - mean_a * mean_a;
        let cov = sum_ha / n_m - mean_h * mean_a;
        let rho = cov / (var_h * var_a).sqrt().max(1e-9);
        let indep_draws: f64 = (0..12).map(|k| (h_marg[k] / n_m) * (a_marg[k] / n_m)).sum();
        let observed_draws = outcomes
            .iter()
            .filter(|o| o.home_goals == o.away_goals)
            .count() as f64
            / n_m;
        println!();
        println!("--- SCORE CORRELATION (draw-machine fingerprint) ---");
        println!("  team-goal correlation rho : {:+.3}  (real ~0.00)", rho);
        println!(
            "  variance/mean  home {:.2}  away {:.2}  (Poisson = 1.00, real ~1.0-1.1)",
            var_h / mean_h.max(1e-9),
            var_a / mean_a.max(1e-9)
        );
        println!(
            "  observed draws {:>5.1}%  vs expected-if-independent {:>5.1}%  → correlation surplus {:+.1}pp",
            observed_draws * 100.0,
            indep_draws * 100.0,
            (observed_draws - indep_draws) * 100.0
        );

        // Cross-half correlation decomposition. Splits each team's
        // goals into first/second half and correlates all four pairs.
        // RESPONSE dynamics (equalizer mechanics) only couple goals
        // inside the same time window → within-half rho high, cross-
        // half rho ≈ 0. A SHARED PER-MATCH FACTOR (e.g. squad
        // attack/defense tilt, match "openness") couples everything →
        // all four rhos similar. This tells us WHERE the remaining
        // correlation surplus lives.
        let mut h1a = Vec::with_capacity(outcomes.len());
        let mut h2a = Vec::with_capacity(outcomes.len());
        let mut h1b = Vec::with_capacity(outcomes.len());
        let mut h2b = Vec::with_capacity(outcomes.len());
        const HALF_MS: u64 = 45 * 60_000;
        for o in &outcomes {
            let (mut a1, mut a2, mut b1, mut b2) = (0f64, 0f64, 0f64, 0f64);
            for &(t, home) in &o.goal_events {
                match (home, t < HALF_MS) {
                    (true, true) => a1 += 1.0,
                    (true, false) => a2 += 1.0,
                    (false, true) => b1 += 1.0,
                    (false, false) => b2 += 1.0,
                }
            }
            h1a.push(a1);
            h2a.push(a2);
            h1b.push(b1);
            h2b.push(b2);
        }
        let pearson = |x: &[f64], y: &[f64]| -> f64 {
            let n = x.len() as f64;
            let mx = x.iter().sum::<f64>() / n;
            let my = y.iter().sum::<f64>() / n;
            let mut cov = 0.0;
            let mut vx = 0.0;
            let mut vy = 0.0;
            for (a, b) in x.iter().zip(y) {
                cov += (a - mx) * (b - my);
                vx += (a - mx) * (a - mx);
                vy += (b - my) * (b - my);
            }
            cov / (vx * vy).sqrt().max(1e-9)
        };
        println!(
            "  cross-half decomposition (response → within high / cross ~0; shared factor → all similar):"
        );
        println!(
            "    within-half : rho(H1a,H1b)={:+.3}  rho(H2a,H2b)={:+.3}",
            pearson(&h1a, &h1b),
            pearson(&h2a, &h2b)
        );
        println!(
            "    cross-half  : rho(H1a,H2b)={:+.3}  rho(H2a,H1b)={:+.3}",
            pearson(&h1a, &h2b),
            pearson(&h2a, &h1b)
        );
        println!(
            "    same-team   : rho(H1a,H2a)={:+.3}  rho(H1b,H2b)={:+.3}  (persistence of a team's scoring across halves)",
            pearson(&h1a, &h2a),
            pearson(&h1b, &h2b)
        );
    }

    // ── SCORING RATE BY GAME STATE — the regime fingerprint ──────────
    // Reconstructs, for every team-minute, whether the team was
    // leading / level / trailing, and computes goals-per-90 in each
    // state. Real football: the three rates are close (leading teams
    // actually score slightly MORE per minute — counters; trailing
    // slightly more volume but worse conversion nets out). A trailing
    // rate far above the leading rate is the equalizer machine in one
    // number.
    {
        // Indexed [state][era]: state 0=leading 1=level 2=trailing,
        // era 0 = before the 62' behavioral-score gate, era 1 = after.
        // The era split shows whether a state's rate elevation comes
        // from the score-reactive regime (post-62 only) or persists
        // even while behavior is score-blind (structural).
        let mut time_in = [[0f64; 2]; 3];
        let mut goals_in = [[0u32; 2]; 3];
        const FULL_MS: u64 = 90 * 60_000;
        const GATE_MS: u64 = 62 * 60_000;
        for o in &outcomes {
            let mut h = 0i32;
            let mut a = 0i32;
            let mut prev_t = 0u64;
            let add_segment = |from: u64, to: u64, idx_home: usize, time_in: &mut [[f64; 2]; 3]| {
                // split [from, to) at the gate boundary
                let pre = to.min(GATE_MS).saturating_sub(from.min(GATE_MS)) as f64;
                let post = to.max(GATE_MS).saturating_sub(from.max(GATE_MS)) as f64;
                time_in[idx_home][0] += pre;
                time_in[2 - idx_home][0] += pre;
                time_in[idx_home][1] += post;
                time_in[2 - idx_home][1] += post;
            };
            for &(t, home_scored) in &o.goal_events {
                let idx_home = if h > a {
                    0
                } else if h == a {
                    1
                } else {
                    2
                };
                add_segment(prev_t, t, idx_home, &mut time_in);
                let era = if t < GATE_MS { 0 } else { 1 };
                if home_scored {
                    goals_in[idx_home][era] += 1;
                    h += 1;
                } else {
                    goals_in[2 - idx_home][era] += 1;
                    a += 1;
                }
                prev_t = t;
            }
            let idx_home = if h > a {
                0
            } else if h == a {
                1
            } else {
                2
            };
            add_segment(prev_t, FULL_MS, idx_home, &mut time_in);
        }
        println!();
        println!("--- SCORING RATE BY GAME STATE (goals per 90 team-minutes) ---");
        let labels = ["leading ", "level   ", "trailing"];
        for i in 0..3 {
            let total_goals: u32 = goals_in[i].iter().sum();
            let total_time: f64 = time_in[i].iter().sum();
            let per90 = total_goals as f64 / (total_time / FULL_MS as f64).max(1e-9);
            let pre90 = goals_in[i][0] as f64 / (time_in[i][0] / FULL_MS as f64).max(1e-9);
            let post90 = goals_in[i][1] as f64 / (time_in[i][1] / FULL_MS as f64).max(1e-9);
            println!(
                "  {} : {:.2} goals/90 overall  |  pre-62' {:.2}  post-62' {:.2}   (real: states ≈ equal, ~1.3-1.5)",
                labels[i], per90, pre90, post90
            );
        }
    }

    // ── NEXT-GOAL CONCEDER SHARE — locate the equalizer machine ──────
    // For each consecutive goal pair, did the team that CONCEDED goal
    // n score goal n+1? At equal strength a neutral engine should sit
    // near 50% in every gap bucket. A structural restart advantage
    // shows as conceder-share spiking in the short-gap buckets; a
    // behavioral feedback loop (game management / chasing risk) shows
    // as elevated share across ALL buckets.
    let mut pair_total = [0u32; 5];
    let mut pair_conceder_next = [0u32; 5];
    for o in &outcomes {
        for w in o.goal_events.windows(2) {
            let gap_min = (w[1].0.saturating_sub(w[0].0)) as f32 / TICKS_PER_MIN as f32;
            let b = if gap_min < 1.0 {
                0
            } else if gap_min < 2.0 {
                1
            } else if gap_min < 5.0 {
                2
            } else if gap_min < 10.0 {
                3
            } else {
                4
            };
            pair_total[b] += 1;
            // w[0].1 == home scored goal n; conceder scores next when
            // the flags differ.
            if w[0].1 != w[1].1 {
                pair_conceder_next[b] += 1;
            }
        }
    }
    println!();
    println!("  next-goal-by-conceder share per gap bucket (neutral = ~50%):");
    for (i, label) in kicklabels.iter().enumerate() {
        let nb = pair_total[i];
        println!(
            "    {} : {:>4} pairs, conceder scored next {:>5.1}%",
            label,
            nb,
            pair_conceder_next[i] as f32 / nb.max(1) as f32 * 100.0
        );
    }

    // ── PRODUCTION BY 15-MIN BAND (engine-side counters) ──────────────
    // Splits the early-goal front-load into its factors: volume
    // (roll-attempts and shots per band), chance quality (xG/shot), and
    // conversion (goals/shot). Whichever column DECAYS across bands is
    // the lever that's wrong — real-football columns are near-flat with
    // a slight late rise.
    {
        let bands = core::time_band_diag::snapshot();
        let [shots_b, on_target_b, xg_b, goals_b, rolls_b] = bands;
        println!();
        println!("--- PRODUCTION BY 15-MIN BAND (volume vs quality vs conversion) ---");
        println!("  band       rolls    shots  on-tgt%   xG/shot  goals  goals/shot  conv-on-tgt%");
        for i in 0..6 {
            let shots = shots_b[i].max(1) as f64;
            println!(
                "  {:>2}-{:<2}min {:>8} {:>8}   {:>5.1}%    {:>5.3}  {:>5}      {:>5.3}        {:>5.1}%",
                i * 15,
                (i + 1) * 15,
                rolls_b[i],
                shots_b[i],
                on_target_b[i] as f64 / shots * 100.0,
                xg_b[i] as f64 / 1000.0 / shots,
                goals_b[i],
                goals_b[i] as f64 / shots,
                goals_b[i] as f64 / on_target_b[i].max(1) as f64 * 100.0,
            );
        }
        // ── SHOT MIX BY DISTANCE ─────────────────────────────────────
        // The single most diagnostic view of "is this real football".
        // Real Opta shot distribution is ~15 / 25 / 22 / 20 / 13 / 5 %
        // across these bands — roughly 40% of all shots come from
        // OUTSIDE the 16.5m box, and population xG/shot is ~0.11. An
        // engine clustered in the first two bands is manufacturing
        // sitters: xG/shot inflates, forwards post huge ratings off
        // tap-ins, and shot VOLUME has to be suppressed artificially to
        // keep the scoreline sane.
        let [dshots, dxg, drolls, dcalls, dposs, dappr, dlost] =
            core::time_band_diag::distance_snapshot();
        let rolltotal: u64 = drolls.iter().sum();
        let calltotal: u64 = dcalls.iter().sum();
        let posstotal: u64 = dposs.iter().sum();
        let dtotal: u64 = dshots.iter().sum();
        println!();
        println!("--- SHOT MIX BY DISTANCE (where chances actually come from) ---");
        println!("  band            shots   share    xG/shot   rolls%  fire/1k   real share");
        let dlabels = [
            ("<6m      ", "~15%"),
            ("6-11m    ", "~25%"),
            ("11-16.5m ", "~22%"),
            ("16.5-22m ", "~20%"),
            ("22-30m   ", "~13%"),
            ("30m+     ", "~5%"),
        ];
        for (i, (label, real)) in dlabels.iter().enumerate() {
            let s = dshots[i].max(1) as f64;
            println!(
                "  {}  {:>8} {:>6.1}%    {:>6.3} {:>5.1}%  {:>5.1}%  {:>5.1}% {:>7} {:>5.0}% {:>7}   {}",
                label,
                dshots[i],
                dshots[i] as f64 / dtotal.max(1) as f64 * 100.0,
                dxg[i] as f64 / 1000.0 / s,
                dposs[i] as f64 / posstotal.max(1) as f64 * 100.0,
                dcalls[i] as f64 / calltotal.max(1) as f64 * 100.0,
                drolls[i] as f64 / rolltotal.max(1) as f64 * 100.0,
                dappr[i],
                dshots[i] as f64 / dappr[i].max(1) as f64 * 100.0,
                dlost[i],
                real,
            );
        }
        let pd = core::time_band_diag::pos_dist_snapshot();
        println!();
        println!("  shot distance mix BY POSITION (row = share of that line's shots):");
        println!(
            "  {:<5} {:>7} {:>7} {:>7} {:>7} {:>7} {:>7}",
            "pos", "<6m", "6-11", "11-16.5", "16.5-22", "22-30", "30m+"
        );
        for (g, label) in ["GK", "DEF", "MID", "FWD"].iter().enumerate() {
            let tot: u64 = pd[g].iter().sum();
            if tot == 0 {
                continue;
            }
            print!("  {:<5}", label);
            for b in 0..6 {
                print!(" {:>6.1}%", pd[g][b] as f64 / tot as f64 * 100.0);
            }
            println!();
        }

        let et = core::time_band_diag::emit_tag_snapshot();
        let ettotal: u64 = et.iter().sum();
        println!();
        println!("  6-11m EMITTED shots by reason (the close-range over-supply):");
        for (i, name) in core::time_band_diag::ETAG_NAMES.iter().enumerate() {
            if et[i] > 0 {
                println!(
                    "    {:<18} {:>7}  {:>5.1}%",
                    name,
                    et[i],
                    et[i] as f64 / ettotal.max(1) as f64 * 100.0
                );
            }
        }

        {
            let (held, total, gathers) = core::reception_diag::hold_snapshot();
            println!();
            println!(
                "  ball in the keepers gloves: {:.1}% of all ticks   (real ~3-6%)",
                held as f64 / total.max(1) as f64 * 100.0
            );
            println!(
                "    gathers {:.1}/match, mean hold {:.2}s   (real ~8-12/match, ~4s)",
                gathers as f64 / n_matches as f64,
                held as f64 / gathers.max(1) as f64 / 50.0
            );
            let src = core::reception_diag::gather_source_snapshot();
            println!(
                "    by state: catching {:.1}  picking-up {:.1}  diving {:.1}  other {:.1} per match",
                src[0] as f64 / n_matches as f64,
                src[1] as f64 / n_matches as f64,
                src[2] as f64 / n_matches as f64,
                src[3] as f64 / n_matches as f64
            );
        }

        {
            // Where the keeper's possession actually lives, and how it
            // ends. See `reception_diag::KEEPER_BALL`.
            let k = core::reception_diag::keeper_ball_snapshot();
            let m = n_matches as f64;
            let feet = k[0] as f64;
            let hands = k[4] as f64;
            println!();
            println!("--- KEEPER POSSESSION (where the ball is while he has it) ---");
            println!(
                "  at his FEET {:.0} ticks/match ({:.0}% of his possession)   in his GLOVES {:.0} ({:.0}%)",
                feet / m,
                feet / (feet + hands).max(1.0) * 100.0,
                hands / m,
                hands / (feet + hands).max(1.0) * 100.0
            );
            println!(
                "  at his feet: an opponent inside the claim radius on {:.0}% of ticks; he could LEGALLY have picked it up on {:.0}% (and on {:.0}% of the pressed ones)",
                k[1] as f64 / feet.max(1.0) * 100.0,
                k[2] as f64 / feet.max(1.0) * 100.0,
                k[3] as f64 / k[1].max(1) as f64 * 100.0
            );
            println!(
                "  in his gloves: {:.2} opponents inside his own area on average, at least one on {:.0}% of ticks, one within 5u on {:.0}%",
                k[7] as f64 / hands.max(1.0),
                k[5] as f64 / hands.max(1.0) * 100.0,
                k[6] as f64 / hands.max(1.0) * 100.0
            );
            println!(
                "    …over the hold: 0-1s {:.2} in the area   1-2s {:.2}   2s+ {:.2}   (they should be walking out)",
                k[15] as f64 / k[14].max(1) as f64,
                k[17] as f64 / k[16].max(1) as f64,
                k[19] as f64 / k[18].max(1) as f64
            );
            println!(
                "    steered out: {:.0} player-ticks/match at {:.3} u/tick mean",
                k[20] as f64 / m,
                k[21] as f64 / k[20].max(1) as f64 / 1000.0
            );
            println!(
                "  ROBBED: off his feet {:.2}/match   out of his gloves {:.2}/match (MUST be 0)   foot spells {:.1}/match   gloves opened under him {:.2}/match (MUST be 0)",
                k[8] as f64 / m,
                k[9] as f64 / m,
                k[10] as f64 / m,
                k[12] as f64 / m
            );
            let by_state = core::reception_diag::keeper_feet_state_snapshot();
            let mut rows: Vec<(String, u64)> = by_state
                .iter()
                .enumerate()
                .filter(|(_, n)| **n > 0)
                .map(|(i, n)| (StateNames::of(100 + i as u16), *n))
                .collect();
            rows.sort_by(|a, b| b.1.cmp(&a.1));
            let line: Vec<String> = rows
                .iter()
                .map(|(n, c)| format!("{} {:.0}", n, *c as f64 / m))
                .collect();
            println!("  ticks at his feet by state: {}", line.join("   "));
            let robbed = core::reception_diag::keeper_robbed_state_snapshot();
            let mut rrows: Vec<(String, u64)> = robbed
                .iter()
                .enumerate()
                .filter(|(_, n)| **n > 0)
                .map(|(i, n)| (StateNames::of(100 + i as u16), *n))
                .collect();
            rrows.sort_by(|a, b| b.1.cmp(&a.1));
            let rline: Vec<String> = rrows
                .iter()
                .map(|(n, c)| format!("{} {:.2}", n, *c as f64 / m))
                .collect();
            println!("  robbed off his feet in state: {}", rline.join("   "));
            let starts = core::reception_diag::keeper_feet_start_snapshot();
            let mut srows: Vec<(String, u64)> = starts
                .iter()
                .enumerate()
                .filter(|(_, n)| **n > 0)
                .map(|(i, n)| (StateNames::of(100 + i as u16), *n))
                .collect();
            srows.sort_by(|a, b| b.1.cmp(&a.1));
            let sline: Vec<String> = srows
                .iter()
                .map(|(n, c)| format!("{} {:.2}", n, *c as f64 / m))
                .collect();
            println!(
                "  handed the ball AT HIS FEET while in: {}",
                sline.join("   ")
            );
        }

        let ct = core::time_band_diag::close_tag_snapshot();
        let cttotal: u64 = ct.iter().sum();
        println!();
        println!("  <6m EMITTED shots by reason (carried in, or arriving onto it?):");
        for (i, name) in core::time_band_diag::ETAG_NAMES.iter().enumerate() {
            if ct[i] > 0 {
                println!(
                    "    {:<18} {:>7}  {:>5.1}%",
                    name,
                    ct[i],
                    ct[i] as f64 / cttotal.max(1) as f64 * 100.0
                );
            }
        }

        let et = core::time_band_diag::edge_tag_snapshot();
        let ettotal: u64 = et.iter().sum();
        println!();
        println!("  11-16.5m EMITTED shots by reason (the band the shot bar cannot reach):");
        for (i, name) in core::time_band_diag::ETAG_NAMES.iter().enumerate() {
            if et[i] > 0 {
                println!(
                    "    {:<18} {:>7}  {:>5.1}%",
                    name,
                    et[i],
                    et[i] as f64 / ettotal.max(1) as f64 * 100.0
                );
            }
        }

        let tg = core::time_band_diag::tag_snapshot();
        let tgtotal: u64 = tg.iter().sum();
        println!();
        println!("  long-range (>22m) APPROVALS by call-site tag:");
        for (i, name) in core::time_band_diag::TAG_NAMES.iter().enumerate() {
            if tg[i] > 0 {
                println!(
                    "    {:<16} {:>7}  {:>5.1}%",
                    name,
                    tg[i],
                    tg[i] as f64 / tgtotal.max(1) as f64 * 100.0
                );
            }
        }

        let rj = core::time_band_diag::reject_snapshot();
        let rnames = ["far", "min_xg", "six_xg", "no_clear", "pass_def"];
        println!();
        println!("  shot-decision REJECTIONS by distance band (% of calls in band):");
        println!(
            "  {:<10} {:>8} {:>8} {:>8} {:>8} {:>8} {:>8}",
            "reason", "<6m", "6-11", "11-16.5", "16.5-22", "22-30", "30m+"
        );
        for (r, name) in rnames.iter().enumerate() {
            print!("  {:<10}", name);
            for b in 0..6 {
                print!(
                    " {:>7.1}%",
                    rj[r][b] as f64 / dcalls[b].max(1) as f64 * 100.0
                );
            }
            println!();
        }

        let wf = core::time_band_diag::will_factor_snapshot();
        // These MUST track `record_will_factors`' slot order in
        // `forward_shot_decision.rs`. They did not: the labels still named
        // a willingness model that was replaced (xg_boost / body_ctl /
        // gk_ctx no longer exist), so the table was printing `lane` under
        // "body_ctl" and `poise` under "condition" — which is how a
        // 0.27 lane in the six-yard box read as a body-control problem.
        let wnames = [
            "urge",
            "reach",
            "angle_q",
            "lane",
            "poise",
            "boldness",
            "situatnl",
            "psych",
            "APPETITE",
            "  ├ press",
            "  └ corrid",
            "BAR",
            "popq",
            "STANDARD",
            "gk_std",
            "def_q",
        ];
        println!();
        println!("  willingness factor MEANS by distance band (roll samples):");
        println!(
            "  {:<10} {:>8} {:>8} {:>8} {:>8} {:>8} {:>8}",
            "factor", "<6m", "6-11", "11-16.5", "16.5-22", "22-30", "30m+"
        );
        for (i, name) in wnames.iter().enumerate() {
            print!("  {:<10}", name);
            for b in 0..6 {
                let n = drolls[b].max(1);
                print!(" {:>8.5}", wf[i][b] as f64 / 1_000_000.0 / n as f64);
            }
            println!();
        }

        let all_xg: u64 = dxg.iter().sum();
        println!(
            "  population xG/shot: {:.3}  (real ~0.11)   outside-box share: {:.1}%  (real ~40%)",
            all_xg as f64 / 1000.0 / dtotal.max(1) as f64,
            (dshots[3] + dshots[4] + dshots[5]) as f64 / dtotal.max(1) as f64 * 100.0,
        );

        let cond = core::time_band_diag::condition_snapshot();
        println!();
        println!("  avg condition%% by band (GK / DEF / MID / FWD):");
        for (i, row) in cond.iter().enumerate() {
            println!(
                "  {:>2}-{:<2}min   {:>5.1}  {:>5.1}  {:>5.1}  {:>5.1}",
                i * 15,
                (i + 1) * 15,
                row[0].0,
                row[1].0,
                row[2].0,
                row[3].0,
            );
        }
        let vbands = core::time_band_diag::velocity_band_snapshot();
        let vtotal: u64 = vbands.iter().sum();
        println!();
        println!("  outfield velocity-band occupancy (condition-processor ticks):");
        let vlabels = [
            "stationary (<5% max)  [recover -6.0]",
            "walking    (5-30%)    [recover -2.0]",
            "jogging    (30-60%)   [drain  +3.0]",
            "running    (60-85%)   [drain  +6.0]",
            "sprinting  (>85%)     [drain +9/10]",
        ];
        for (i, label) in vlabels.iter().enumerate() {
            println!(
                "    {} : {:>5.1}%",
                label,
                vbands[i] as f64 / vtotal.max(1) as f64 * 100.0
            );
        }
    }

    // ── XG / SHOT EFFICIENCY BY OUTCOME — who deserved the draw? ──────
    // For each match, classify by result (home win / draw / away win)
    // and average the xG totals and shot counts. If draws cluster
    // around matches where both teams had similar xG (~each team's
    // typical match), the engine is failing to convert xG differential
    // into result differential — likely keeper-save inflation or shot
    // quality compression. If draws have ABOVE-average xG, the
    // problem is conversion efficiency, not chance creation.
    let mut xg_by_result = [(0.0f32, 0.0f32, 0u32); 3]; // (home_xg, away_xg, n) for [home_win, draw, away_win]
    let mut shots_by_result = [(0u32, 0u32, 0u32); 3]; // (home_sh, away_sh, n)
    for o in &outcomes {
        let bucket = if o.home_goals > o.away_goals {
            0
        } else if o.home_goals < o.away_goals {
            2
        } else {
            1
        };
        xg_by_result[bucket].0 += o.home.xg;
        xg_by_result[bucket].1 += o.away.xg;
        xg_by_result[bucket].2 += 1;
        shots_by_result[bucket].0 += o.home.shots as u32;
        shots_by_result[bucket].1 += o.away.shots as u32;
        shots_by_result[bucket].2 += 1;
    }
    println!();
    println!("--- xG/SHOTS BY MATCH OUTCOME ---");
    let result_labels = ["home win", "draw    ", "away win"];
    for (i, label) in result_labels.iter().enumerate() {
        let n = xg_by_result[i].2;
        if n == 0 {
            continue;
        }
        let h_xg = xg_by_result[i].0 / n as f32;
        let a_xg = xg_by_result[i].1 / n as f32;
        let h_sh = shots_by_result[i].0 as f32 / n as f32;
        let a_sh = shots_by_result[i].1 as f32 / n as f32;
        let xg_diff = h_xg - a_xg;
        println!(
            "  {}  n={:>4}  xG h={:>4.1} a={:>4.1}  (diff {:+.1})  sh h={:>4.1} a={:>4.1}",
            label, n, h_xg, a_xg, xg_diff, h_sh, a_sh,
        );
    }
    println!(
        "  (if draws have similar xG-spread as decisive matches, the engine's xG→goal step is too noisy)"
    );

    // ── HOME ADVANTAGE (equal-level matches only) ─────────────────────
    // Real-football reference at equal strength: ~45% home wins / ~25%
    // draws / ~30% away wins, home goals ≈ +0.30-0.40. The engine's
    // play-quality home edge (crowd-scaled press/risk/tempo lift in
    // tactical.rs) plus the referee marginal-call bias should
    // reproduce that split; a 33/33/33-ish line means home advantage
    // is missing and equal-strength matches will over-draw relative
    // to real leagues.
    {
        let mut hw = 0u32;
        let mut dr = 0u32;
        let mut aw = 0u32;
        let mut hg = 0u32;
        let mut ag = 0u32;
        for o in outcomes.iter().filter(|o| o.level_a == o.level_b) {
            match o.home_goals.cmp(&o.away_goals) {
                std::cmp::Ordering::Greater => hw += 1,
                std::cmp::Ordering::Equal => dr += 1,
                std::cmp::Ordering::Less => aw += 1,
            }
            hg += o.home_goals as u32;
            ag += o.away_goals as u32;
        }
        let n_eq = (hw + dr + aw).max(1);
        println!();
        println!("--- HOME ADVANTAGE (equal-level matches, n={}) ---", n_eq);
        println!(
            "  home win {:>5.1}% / draw {:>5.1}% / away win {:>5.1}%   (real ~45/25/30)",
            hw as f32 / n_eq as f32 * 100.0,
            dr as f32 / n_eq as f32 * 100.0,
            aw as f32 / n_eq as f32 * 100.0
        );
        println!(
            "  home goals/match {:.2} vs away {:.2}  (real diff ~+0.35)",
            hg as f32 / n_eq as f32,
            ag as f32 / n_eq as f32
        );
    }

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

    // ── ASSISTS BY LINE + ATTRIBUTION SANITY ──────────────────────────
    //
    // Real football assist share ≈ MID 45% / FWD 30% / DEF 24% / GK ~1%
    // (a keeper assist is a long kick headed straight in — a handful per
    // league season, never a chart-topper). Two failure modes show here:
    //
    //  * `GK` share materially above ~2%: the assist is being read off a
    //    stale `recent_passers` entry — typically the goal kick that
    //    started the phase, minutes of play before the shot.
    //  * `cross-team`: the credited assister plays for the team that
    //    CONCEDED. That can only happen if the assist selection has no
    //    same-team check and the pass ring survived the turnover.
    //
    // Assists are paired to goals by timestamp — the dispatcher emits
    // `PlayerEvent::Assist` in the same tick as `PlayerEvent::Goal`, so
    // both details carry the identical `total_match_time`.
    println!();
    println!(
        "--- ASSISTS BY LINE (aggregated across {} matches) ---",
        n_matches
    );
    let mut assists_by_line = [0u32; 4];
    let mut cross_team_assists = 0u32;
    let mut cross_team_by_line = [0u32; 4];
    let mut assisted_goals = 0u32;
    let mut unmatched_assists = 0u32;
    let mut real_goals = 0u32;
    for o in &outcomes {
        real_goals += o.goal_details.iter().filter(|g| !g.2).count() as u32;
        for &(time, assister) in &o.assist_details {
            let gi = pos_group_of(assister) as usize;
            assists_by_line[gi] += 1;
            match o.goal_details.iter().find(|g| g.0 == time && !g.2) {
                Some(&(_, scorer, _)) => {
                    assisted_goals += 1;
                    if scorer / 100 != assister / 100 {
                        cross_team_assists += 1;
                        cross_team_by_line[gi] += 1;
                    }
                }
                None => unmatched_assists += 1,
            }
        }
    }
    let total_assists: u32 = assists_by_line.iter().sum::<u32>().max(1);
    for (i, label) in line_labels.iter().enumerate() {
        println!(
            "  {:<4} assists={:>4} ({:>4.1}% of all)   cross-team={:>4} ({:>4.1}% of line)",
            label,
            assists_by_line[i],
            assists_by_line[i] as f32 / total_assists as f32 * 100.0,
            cross_team_by_line[i],
            cross_team_by_line[i] as f32 / assists_by_line[i].max(1) as f32 * 100.0,
        );
    }
    println!(
        "  total assists={}  assisted goals={}/{} ({:.0}% of real goals)  unmatched={}",
        total_assists,
        assisted_goals,
        real_goals,
        assisted_goals as f32 / real_goals.max(1) as f32 * 100.0,
        unmatched_assists,
    );
    println!(
        "  CROSS-TEAM assists = {} ({:.1}% of all) — must be 0",
        cross_team_assists,
        cross_team_assists as f32 / total_assists as f32 * 100.0,
    );
    println!("  target assist share ≈ MID 45% / FWD 30% / DEF 24% / GK ~1%");
    {
        // Why a goal did or didn't carry an assist, straight from the
        // resolver. `opponent chain` is the honest reading of "the
        // scoring team won the ball and finished without passing" — it
        // used to be silently credited to whoever conceded possession.
        let (goals, empty, opponent, scorer_only, stale, credited, delay_sum) =
            core::assist_diag::snapshot();
        let pct = |x: u64| {
            if goals == 0 {
                0.0
            } else {
                x as f32 / goals as f32 * 100.0
            }
        };
        println!(
            "  resolver: {} goals — credited {:.1}%, empty chain {:.1}%, opponent chain {:.1}%, \
             scorer-only chain {:.1}%, outside window {:.1}%",
            goals,
            pct(credited),
            pct(empty),
            pct(opponent),
            pct(scorer_only),
            pct(stale),
        );
        println!(
            "  mean pass→goal delay on credited assists: {:.2}s (window {:.1}s)",
            delay_sum as f32 / credited.max(1) as f32 / 100.0,
            core::r#match::engine::ball::ball::ASSIST_WINDOW_TICKS as f32 / 100.0,
        );
        let (opp_has_teammate, opp_age) = core::assist_diag::opponent_chain_detail();
        println!(
            "  opponent-chain detail: {} of {} still had a teammate pass deeper in the ring \
             ({:.1}%); blocking opponent pass was {:.2}s old on average",
            opp_has_teammate,
            opponent,
            opp_has_teammate as f32 / opponent.max(1) as f32 * 100.0,
            opp_age as f32 / opponent.max(1) as f32 / 100.0,
        );
    }

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
    // ── RATING TAILS + MULTI-GOAL FREQUENCY ─────────────────────────────
    //
    // The "8.31 after 2 matches" class of report: a small-sample season
    // average is only as realistic as the FREQUENCY of the big matches
    // behind it. Real-football references (WhoScored-era, top leagues):
    //   * 2+ goal player-matches: FWD ~4-5%, MID ~0.5%, DEF ~0.1%
    //   * per-match rating ≥7.5: FWD ~8%, MID ~4-5%, DEF ~2%
    //   * per-match rating ≥8.0: FWD ~3%, MID ~1%, DEF ~0.5%
    // If the engine mints braces or 8.0+ matches materially more often
    // than this, small-sample season rows on the site will routinely
    // show 8+ averages and read as inflation even when each individual
    // match rating is defensible.
    {
        let mut brace = [0u32; 4];
        let mut ge75 = [0u32; 4];
        let mut ge80 = [0u32; 4];
        let mut n_pos = [0u32; 4];
        for o in &outcomes {
            for &(_id, goals, _sh, _xg, grp, rating, minutes, _assists) in &o.per_player {
                if minutes == 0 {
                    continue;
                }
                let gi = grp as usize;
                n_pos[gi] += 1;
                if goals >= 2 {
                    brace[gi] += 1;
                }
                if rating >= 7.5 {
                    ge75[gi] += 1;
                }
                if rating >= 8.0 {
                    ge80[gi] += 1;
                }
            }
        }
        println!();
        println!("--- RATING TAILS + MULTI-GOAL (per player-match) ---");
        println!(
            "  {:<4} {:>8} {:>8} {:>8}    real: braces FWD~4-5%/MID~0.5%; >=8.0 FWD~3%/MID~1%",
            "pos", "2+goals", ">=7.5", ">=8.0"
        );
        for (i, label) in ["GK", "DEF", "MID", "FWD"].iter().enumerate() {
            let n = n_pos[i].max(1) as f32;
            println!(
                "  {:<4} {:>7.2}% {:>7.2}% {:>7.2}%",
                label,
                brace[i] as f32 / n * 100.0,
                ge75[i] as f32 / n * 100.0,
                ge80[i] as f32 / n * 100.0,
            );
        }
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

    // ── RATING vs SKILL CORRELATION ─────────────────────────────────────
    //
    // The rating layer is ability-blind by contract, so player QUALITY can
    // only reach a rating through the ENGINE producing a quality-dependent
    // stat line. This block measures whether it does: Pearson r between a
    // player's raw position composite (`SkillComposite`) and the rating he
    // earned, over every player-match in the run.
    //
    // Samples are player-MATCHES, not season means: squads are regenerated
    // per match, so an id is a fresh player each time — which is exactly
    // what makes this a clean measurement of the engine channel (the same
    // id spans 400 independently drawn players at the same level). Single-
    // match outcome noise is large and real, so healthy is r ≈ 0.30-0.50,
    // not 0.9. r ≈ 0 for a position means the engine is emitting the same
    // stat line regardless of who is playing — a producer bug, never
    // something to fix in the rating.
    //
    // At fixed levels the skill spread is generator noise within one level
    // (sd ≈ 0.5-1.0), so `skill sd` is printed alongside: a near-zero
    // spread would make r meaningless, and random-level runs (no level
    // args) widen it deliberately.
    let mut skill_corr = [Correlation::default(); 4];
    {
        let mut by_id: std::collections::HashMap<u32, f32> = std::collections::HashMap::new();
        for o in &outcomes {
            by_id.clear();
            by_id.extend(o.per_player_skill.iter().copied());
            for (id, _g, _sh, _xg, grp, rating, minutes, _a) in &o.per_player {
                if *minutes == 0 {
                    continue;
                }
                if let Some(skill) = by_id.get(id) {
                    skill_corr[*grp as usize].push(*skill, *rating);
                }
            }
        }
    }
    println!();
    println!(
        "--- RATING vs SKILL CORRELATION (player-match samples, SQUAD_SPREAD={:.1}) ---",
        SquadSpread::sd()
    );
    if SquadSpread::sd() <= 0.0 {
        println!(
            "  (uniform squads: every player is retargeted to the same mean, so the only\n   \
             variation is skill SHAPE — r is structurally ~0 here regardless of the engine.\n   \
             Run with SQUAD_SPREAD=2 for a real quality axis.)"
        );
    }
    println!(
        "  {:<4} {:>7} {:>8} {:>10} {:>10} {:>7}    healthy r ~0.30-0.50",
        "pos", "r", "n", "skill mean", "skill sd", "rat sd"
    );
    for (i, label) in ["GK", "DEF", "MID", "FWD"].iter().enumerate() {
        let c = &skill_corr[i];
        println!(
            "  {:<4} {:>7.3} {:>8} {:>10.2} {:>10.2} {:>7.2}",
            label,
            c.r(),
            c.n,
            c.mean_x(),
            c.sd_x(),
            c.sd_y(),
        );
    }

    // ── RATING VOLUME PROFILE — per-player per-match counter means ───────
    //
    // The counters the rating model reads as volume, per position, next
    // to real-football per-90 references (FBref top-5-league norms).
    // This is the calibration source for the engine→real volume
    // conversion (rating/volume.rs): the rating model's saturation
    // scales and evidence-tier thresholds are set for REAL volumes, so
    // when the engine emits more raw events than a real statistician
    // would count, this table is what the divisors are derived from.
    // The Strong-route shares at the bottom are the tell: routine_def
    // >= 7 is a rare monster shift in real football; if a third of
    // ordinary player-matches clear it, ratings inflate wholesale.
    let mut vol_by_pos = [RatingVolumeAgg::default(); 4];
    for o in &outcomes {
        for (i, v) in o.pos_volumes.iter().enumerate() {
            vol_by_pos[i].merge(v);
        }
    }
    println!();
    println!("--- RATING VOLUME PROFILE (per-player per-match means) ---");
    println!(
        "  {:<22} {:>6} {:>6} {:>6}    real per-90 (DEF / MID)",
        "counter", "DEF", "MID", "FWD"
    );
    {
        let per = |v: &RatingVolumeAgg, x: u32| -> f32 {
            if v.samples == 0 {
                0.0
            } else {
                x as f32 / v.samples as f32
            }
        };
        let d = &vol_by_pos[1];
        let m = &vol_by_pos[2];
        let f = &vol_by_pos[3];
        let rows: [(&str, f32, f32, f32, &str); 14] = [
            (
                "tackles",
                per(d, d.tackles),
                per(m, m.tackles),
                per(f, f.tackles),
                "~1.6 / ~1.8",
            ),
            (
                "interceptions",
                per(d, d.interceptions),
                per(m, m.interceptions),
                per(f, f.interceptions),
                "~1.3 / ~1.0",
            ),
            (
                "blocks",
                per(d, d.blocks),
                per(m, m.blocks),
                per(f, f.blocks),
                "~0.9 / ~0.3",
            ),
            (
                "clearances",
                per(d, d.clearances),
                per(m, m.clearances),
                per(f, f.clearances),
                "~3.5 / ~1.0",
            ),
            (
                "pressures",
                per(d, d.pressures),
                per(m, m.pressures),
                per(f, f.pressures),
                "~11 / ~15",
            ),
            (
                "succ_pressures",
                per(d, d.succ_pressures),
                per(m, m.succ_pressures),
                per(f, f.succ_pressures),
                "~3.5 / ~4.5",
            ),
            (
                "key_passes",
                per(d, d.key_passes),
                per(m, m.key_passes),
                per(f, f.key_passes),
                "~0.4 / ~1.0",
            ),
            (
                "passes_into_box",
                per(d, d.passes_into_box),
                per(m, m.passes_into_box),
                per(f, f.passes_into_box),
                "~0.7 / ~1.5",
            ),
            (
                "prog_passes",
                per(d, d.prog_passes),
                per(m, m.prog_passes),
                per(f, f.prog_passes),
                "~4.0 / ~5.0",
            ),
            (
                "prog_carries",
                per(d, d.prog_carries),
                per(m, m.prog_carries),
                per(f, f.prog_carries),
                "~1.0 / ~2.0",
            ),
            (
                "succ_dribbles",
                per(d, d.dribbles),
                per(m, m.dribbles),
                per(f, f.dribbles),
                "~0.4 / ~1.0",
            ),
            (
                "crosses_completed",
                per(d, d.crosses_completed),
                per(m, m.crosses_completed),
                per(f, f.crosses_completed),
                "~0.5 / ~0.7",
            ),
            (
                "danger_zone_actions",
                per(d, d.danger_zone_actions),
                per(m, m.danger_zone_actions),
                per(f, f.danger_zone_actions),
                "~1.5 / ~0.3",
            ),
            (
                "ft_press_won+ft_tk",
                per(d, d.ft_pressures_won + d.ft_tackles),
                per(m, m.ft_pressures_won + m.ft_tackles),
                per(f, f.ft_pressures_won + f.ft_tackles),
                "~0.5 / ~1.0",
            ),
        ];
        // Why key passes under-emit: is the shot-assist TAGGING missing
        // them, or do the engine's shots genuinely not arrive from a pass
        // to the shooter? Opta's key pass is "the last pass before a
        // shot", so the second case is a possession-model property, not a
        // stat bug, and no divisor may compensate for it.
        {
            let (shots, no_link, wrong_receiver, stale, credited) = core::key_pass_diag::snapshot();
            let pct = |x: u64| {
                if shots == 0 {
                    0.0
                } else {
                    x as f32 / shots as f32 * 100.0
                }
            };
            println!(
                "  key-pass tagging: {} shots — credited {:.1}%, no completed pass on record \
                 {:.1}%, pass went to someone else {:.1}%, outside window {:.1}%   \
                 (real: ~55-60% of shots have a key pass)",
                shots,
                pct(credited),
                pct(no_link),
                pct(wrong_receiver),
                pct(stale),
            );
            // What actually feeds the engine's shots. Real football:
            // a clear majority of shots are struck by the player who
            // was just passed to; loose balls and turnovers are the
            // minority. If `pass` here is small, the possession model —
            // not the key-pass tagging — is what caps key passes and
            // assists.
            {
                let supply = core::key_pass_diag::supply_snapshot();
                let total: u64 = supply.iter().sum::<u64>().max(1);
                let names = core::r#match::engine::ball::ball::PossessionSource::NAMES;
                let mut parts: Vec<String> = Vec::new();
                for (i, n) in names.iter().enumerate() {
                    parts.push(format!(
                        "{} {:.1}%",
                        n,
                        supply[i] as f32 / total as f32 * 100.0
                    ));
                }
                println!(
                    "  shot supply (how the shooter got the ball): {}   (real: pass ~55-60%)",
                    parts.join(", "),
                );
            }
            // Where the ball actually is when a pass is booked received.
            // `move_to` drops ownership past 15u, so anything credited
            // beyond that band is a completed pass the receiver never
            // actually got — it reads as accuracy and plays as a loose ball.
            {
                let (bands, too_far) = core::reception_diag::snapshot();
                let total: u64 = bands.iter().sum::<u64>().max(1);
                let names = core::reception_diag::BAND_NAMES;
                let mut parts: Vec<String> = Vec::new();
                for (i, n) in names.iter().enumerate() {
                    parts.push(format!(
                        "{} {:.1}%",
                        n,
                        bands[i] as f32 / total as f32 * 100.0
                    ));
                }
                println!(
                    "  reception distance (receiver→ball at claim): {}   \
                     ball-tracking cutoff is 15u",
                    parts.join(", "),
                );
                println!(
                    "  ownership dropped by move_to (owner >15u away): {} ({:.2}/match)",
                    too_far,
                    too_far as f32 / n_matches as f32,
                );
                let refused = core::reception_diag::GRANT_OUT_OF_REACH
                    .load(std::sync::atomic::Ordering::Relaxed);
                println!(
                    "  grants refused before they could strand the ball: {} ({:.2}/match)   \
                     these used to become the line above, a tick later, with the \
                     velocity already zeroed",
                    refused,
                    refused as f32 / n_matches as f32,
                );
                {
                    use core::reception_diag::too_far;
                    let (bands, in_hands, during_shot, stopped) = too_far::snapshot();
                    let tot = bands.iter().sum::<u64>().max(1);
                    let mix: Vec<String> = too_far::BAND_NAMES
                        .iter()
                        .zip(bands.iter())
                        .map(|(n, &c)| format!("{} {:.0}%", n, c as f32 / tot as f32 * 100.0))
                        .collect();
                    println!(
                        "    of those drops: {}   |   in a keeper's gloves {:.0}%, \
                         shot live {:.0}%, BALL ALREADY DEAD {:.0}% (this last is the \
                         subset the viewer shows as a ball frozen in mid-pitch)",
                        mix.join(", "),
                        in_hands as f32 / tot as f32 * 100.0,
                        during_shot as f32 / tot as f32 * 100.0,
                        stopped as f32 / tot as f32 * 100.0,
                    );
                }
                let (sw, so, sc_, snt, grj, clamped) = core::reception_diag::shot_fate_snapshot();
                println!(
                    "  shot fate: wide {}, over the bar {}, claimed mid-flight {}, \
                     no projected target {}, goal REJECTED at the line {}   \
                     (vs saves+goals = the credited on-target count)",
                    sw, so, sc_, snt, grj,
                );
                {
                    // Complete partition of every struck shot. The
                    // `shot fate` line above is a set of per-site flags
                    // that between them catch ~0.5% of shots; this is
                    // the census, and it must sum to STRUCK.
                    let c = core::reception_diag::fate_census();
                    let (struck, goal, gk, out, cdef, catt, stopped, timeout) =
                        (c[0], c[1], c[2], c[3], c[4], c[5], c[6], c[7]);
                    let (live_ticks, struck_d, reached_d) = (c[8], c[9], c[10]);
                    let s = struck.max(1) as f64;
                    let reached = goal + gk;
                    println!(
                        "  SHOT LIFECYCLE CENSUS — {} struck, {:.1} live ticks each, \
                         struck from {:.0}u avg",
                        struck,
                        live_ticks as f64 / s,
                        struck_d as f64 / 100.0 / s,
                    );
                    for (label, v, note) in [
                        ("reached the goal (goal + keeper)", reached, "real ~35-40%"),
                        ("  ├ goal", goal, ""),
                        ("  └ keeper gathered/saved", gk, ""),
                        ("out of play (corner/goal kick/throw)", out, "real ~45%"),
                        (
                            "claimed mid-flight by a DEFENDER",
                            cdef,
                            "real ~15% (blocks)",
                        ),
                        ("claimed mid-flight by an ATTACKER", catt, "real ~5%"),
                        ("came to rest on the pitch", stopped, "real ~0%"),
                        ("still live at the 400-tick timeout", timeout, "real 0%"),
                    ] {
                        println!(
                            "    {:<38} {:>6} ({:>5.1}%)  {}",
                            label,
                            v,
                            v as f64 / s * 100.0,
                            note
                        );
                    }
                    if reached > 0 {
                        println!(
                            "    avg strike distance: all {:.0}u vs reached-goal {:.0}u  \
                             (a big gap = the leak is distance-selective)",
                            struck_d as f64 / 100.0 / s,
                            reached_d as f64 / 100.0 / reached as f64,
                        );
                    }
                }
                println!(
                    "  stranded in the goalmouth (endline clamp, MUST be 0): {}{}",
                    clamped,
                    if clamped == 0 {
                        ""
                    } else {
                        "   <-- an endline resolver is declining balls again"
                    },
                );
                // Who the ball dies on. A stalled ball is nearly always a
                // state with no way to act on possession — see
                // `dead_ball_diag`. Ticks here are FULL ticks (~20 ms).
                {
                    let (rows, un_ticks, un_eps, longest) = core::dead_ball_diag::snapshot();
                    let mut label = std::collections::HashMap::new();
                    for st in core::r#match::player::state::PlayerState::all() {
                        label.insert(st.compact_id(), format!("{}", st));
                    }
                    let total: u64 = rows.iter().map(|r| r.1).sum::<u64>() + un_ticks;
                    println!(
                        "  ball STUCK (inside 15u for 5s+): {:.1}s/match over {} episodes, longest {:.1}s",
                        total as f64 * 0.02 / n_matches as f64,
                        rows.iter().map(|r| r.2).sum::<u64>() + un_eps,
                        longest as f64 * 0.02,
                    );
                    for (id, ticks, eps) in rows.iter().take(8) {
                        // Dwell split per row: a state that HOLDS the ball
                        // reads high, a state everybody passes THROUGH on
                        // the tick they claim it reads 0-1. The two need
                        // opposite fixes, and the row alone cannot tell
                        // them apart.
                        let d = core::dead_ball_diag::dwell_for_state(*id);
                        let dt: u64 = d.iter().sum::<u64>().max(1);
                        println!(
                            "      {:>28}  {:>6.1}s  {:>4} episodes   dwell {:.0}/{:.0}/{:.0}/{:.0}/{:.0}%",
                            label
                                .get(id)
                                .cloned()
                                .unwrap_or_else(|| format!("state {id}")),
                            *ticks as f64 * 0.02,
                            eps,
                            d[0] as f64 / dt as f64 * 100.0,
                            d[1] as f64 / dt as f64 * 100.0,
                            d[2] as f64 / dt as f64 * 100.0,
                            d[3] as f64 / dt as f64 * 100.0,
                            d[4] as f64 / dt as f64 * 100.0,
                        );
                    }
                    println!(
                        "      (dwell buckets = owner's in-state AI ticks: {})",
                        core::dead_ball_diag::DWELL_LABELS.join(" / ")
                    );
                    if un_ticks > 0 {
                        println!(
                            "      {:>28}  {:>6.1}s  {:>4} episodes",
                            "(nobody — loose ball)",
                            un_ticks as f64 * 0.02,
                            un_eps
                        );
                    }
                    // How long the owner had been in his state. A state
                    // everybody passes THROUGH on the way into possession
                    // collects one tick per ownership grant and reads
                    // 0-1; a state that holds the ball and does nothing
                    // reads high. The two need opposite fixes.
                    let (all_dwell, tb_dwell, tb_dist) = core::dead_ball_diag::dwell_snapshot();
                    let dsum: u64 = all_dwell.iter().sum::<u64>().max(1);
                    let tsum: u64 = tb_dwell.iter().sum::<u64>().max(1);
                    let pct = |v: &[u64; 5], t: u64| {
                        v.iter()
                            .enumerate()
                            .map(|(i, n)| {
                                format!(
                                    "{} {:.0}%",
                                    core::dead_ball_diag::DWELL_LABELS[i],
                                    *n as f64 / t as f64 * 100.0
                                )
                            })
                            .collect::<Vec<_>>()
                            .join("  ")
                    };
                    println!(
                        "      owner in-state ticks when stuck: {}",
                        pct(&all_dwell, dsum)
                    );
                    println!(
                        "      …of which TakeBall ({} ticks): {}   owner sits {:.1}u ({:.2}m) off the ball",
                        tb_dwell.iter().sum::<u64>(),
                        pct(&tb_dwell, tsum),
                        tb_dist,
                        tb_dist * 0.125,
                    );
                    let (gained, lost, self_reclaim, spells) =
                        core::dead_ball_diag::churn_snapshot();
                    let ssum: u64 = spells.iter().sum::<u64>().max(1);
                    let spell_str = spells
                        .iter()
                        .enumerate()
                        .map(|(i, n)| {
                            format!(
                                "{} {:.0}%",
                                core::dead_ball_diag::SPELL_LABELS[i],
                                *n as f64 / ssum as f64 * 100.0
                            )
                        })
                        .collect::<Vec<_>>()
                        .join("  ");
                    println!(
                        "      possession churn: {:.0} claims/match, {:.0} losses/match, {:.0}% of claims re-claimed by the man who just lost it",
                        gained as f64 / n_matches as f64,
                        lost as f64 / n_matches as f64,
                        self_reclaim as f64 / gained.max(1) as f64 * 100.0,
                    );
                    println!("      spell length (full ticks): {}", spell_str);
                    {
                        let (buckets, thirds, mean, engagers) =
                            core::dead_ball_diag::carrier_pressure_snapshot();
                        let tot: u64 = buckets.iter().sum::<u64>().max(1);
                        let row = |v: &[u64], t: u64| {
                            v.iter()
                                .enumerate()
                                .map(|(i, n)| {
                                    format!(
                                        "{} {:.0}%",
                                        core::dead_ball_diag::PRESSURE_LABELS[i],
                                        *n as f64 / t.max(1) as f64 * 100.0
                                    )
                                })
                                .collect::<Vec<_>>()
                                .join("  ")
                        };
                        println!();
                        println!("--- PRESSURE ON THE MAN IN POSSESSION ---");
                        println!(
                            "  nearest opponent to the carrier: mean {:.1} m, {:.2} opponents within 10 m",
                            mean, engagers
                        );
                        println!(
                            "  all: {}   (real: a carrier is engaged inside 5 m most of the time)",
                            row(&buckets, tot)
                        );
                        for (i, label) in ["own third", "middle", "attacking"].iter().enumerate() {
                            let slice = &thirds[i * 5..i * 5 + 5];
                            let t: u64 = slice.iter().sum();
                            println!("  {:<10} {}", label, row(slice, t));
                        }
                    }
                    {
                        let (n, ccap, hcap, cspd, hspd, outpaced, tiers) =
                            core::dead_ball_diag::chase_speed_snapshot();
                        let tt: u64 = tiers.iter().sum::<u64>().max(1);
                        let trow = tiers
                            .iter()
                            .enumerate()
                            .map(|(i, v)| {
                                format!(
                                    "{} {:.0}%",
                                    core::dead_ball_diag::CHASE_TIER_LABELS[i],
                                    *v as f64 / tt as f64 * 100.0
                                )
                            })
                            .collect::<Vec<_>>()
                            .join("  ");
                        println!(
                            "  CAN HE CATCH HIM? {} samples — speed CEILING carrier {:.3} vs nearest chaser {:.3} u/tick; chaser capped LOWER on {:.0}% of ticks",
                            n,
                            ccap,
                            hcap,
                            outpaced * 100.0
                        );
                        println!(
                            "      actual speed   carrier {:.3}  chaser {:.3}   |   chaser's effort tier: {}",
                            cspd, hspd, trow
                        );
                    }
                    {
                        // **How far does a footballer actually run?** The
                        // viewer's own census read 17.6 km per ninety
                        // minutes off a recorded match against the 10-12 a
                        // real one covers, and nothing in this harness
                        // measured it. See [`MotionCensus`].
                        use core::dead_ball_diag::{CHASE_TIER_LABELS, MotionCensus};
                        let (outfield, keeper, still, tiers, capped, ceiling) =
                            MotionCensus::snapshot();
                        let trow = tiers
                            .iter()
                            .enumerate()
                            .map(|(i, v)| format!("{} {:.0}%", CHASE_TIER_LABELS[i], *v * 100.0))
                            .collect::<Vec<_>>()
                            .join("  ");
                        println!(
                            "  GROUND COVERED  outfield {outfield:.1} km/90 (real 10-12), \
                             keeper {keeper:.1} (real 4-6); below a walk on {:.0}% of ticks \
                             (real ~70%)",
                            still * 100.0
                        );
                        println!(
                            "      effort tier: {trow}\n      \
                             his speed ceiling is {ceiling:.3} u/tick and he is AT it on \
                             {:.0}% of ticks",
                            capped * 100.0
                        );
                    }
                    {
                        let (fo, yi, fl) = core::chase_diag::snapshot();
                        println!(
                            "      chase designation: {:.0} forces/match, {:.0} yields/match ({:.0}% of forces during a delivery in flight)",
                            fo as f64 / n_matches as f64,
                            yi as f64 / n_matches as f64,
                            fl as f64 / fo.max(1) as f64 * 100.0
                        );
                    }
                    {
                        let (tv, cross, zones, inflight) =
                            core::dead_ball_diag::stall_churn_snapshot();
                        let zt: u64 = zones.iter().sum::<u64>().max(1);
                        let zrow = zones
                            .iter()
                            .enumerate()
                            .map(|(i, n)| {
                                format!(
                                    "{} {:.0}%",
                                    core::dead_ball_diag::ZONE_LABELS[i],
                                    *n as f64 / zt as f64 * 100.0
                                )
                            })
                            .collect::<Vec<_>>()
                            .join("  ");
                        println!(
                            "      inside stalls: {:.1} turnovers/match, {:.0}% of them cross-team; {:.0}% of stuck ticks had a delivery in the air",
                            tv as f64 / n_matches as f64,
                            cross as f64 / tv.max(1) as f64 * 100.0,
                            inflight as f64 / zt as f64 * 100.0
                        );
                        println!("      stall zone: {}", zrow);
                    }
                    {
                        let (ex, sp) = core::stuck_exit_stats::snapshot();
                        let t: u64 = ex.iter().sum::<u64>().max(1);
                        let row = ex
                            .iter()
                            .enumerate()
                            .map(|(i, n)| {
                                format!(
                                    "{} {:.0}%",
                                    core::stuck_exit_stats::NAMES[i],
                                    *n as f64 / t as f64 * 100.0
                                )
                            })
                            .collect::<Vec<_>>()
                            .join("  ");
                        println!(
                            "      FWD Running stuck-exit ({} ticks, mean speed {:.3} u/tick): {}",
                            t, sp, row
                        );
                    }
                    let (tb_ticks, tb_spells) = core::dead_ball_diag::takeball_ownership_snapshot();
                    println!(
                        "      TakeBall held the ball {:.1}s/match over {:.0} spells — {:.1} full ticks each (1-2 = a pass-through, not a dwell)",
                        tb_ticks as f64 * 0.02 / n_matches as f64,
                        tb_spells as f64 / n_matches as f64,
                        tb_ticks as f64 / tb_spells.max(1) as f64,
                    );
                }
                // ── BALL FLIGHT CENSUS ──────────────────────────────
                //
                // Where the ball actually goes, and which pass of
                // `Ball::update` sent it there. Two failure modes live
                // here and neither shows up in any other counter: kicks
                // whose vertical velocity is in the wrong units (the
                // vertical axis is in METRES, so a hand-written `z` of
                // 4.5 is a ten-kilometre apex) and passes that relocate
                // the ball without any velocity behind the move.
                {
                    let (launches, hist, apex_max, peak_z, peak_speed) =
                        core::flight_diag::FlightDiag::launch_snapshot();
                    println!(
                        "  BALL FLIGHT — {} launches, worst apex {:.1}m, highest the ball got {:.1}m, \
                         fastest loose ball {:.2}u/tick ({:.0} m/s)",
                        launches,
                        apex_max,
                        peak_z,
                        peak_speed,
                        peak_speed * 0.125 * 100.0,
                    );
                    let ls = launches.max(1) as f32;
                    let bands: Vec<String> = core::flight_diag::APEX_LABELS
                        .iter()
                        .zip(hist.iter())
                        .map(|(l, c)| format!("{l} {:.1}%", *c as f32 / ls * 100.0))
                        .collect();
                    println!("    launch apex: {}", bands.join(" | "));
                    // Everything from 30 m up is beyond any football ever
                    // kicked, so it is a unit bug by construction.
                    let absurd: u64 = hist[5..].iter().sum();
                    if absurd > 0 {
                        println!(
                            "    ^^ {} launches ({:.2}%) above 30m — a kick site is still \
                             writing a raw z instead of solving an apex",
                            absurd,
                            absurd as f32 / ls * 100.0
                        );
                        let by_state = core::flight_diag::FlightDiag::absurd_by_state();
                        let mut label = std::collections::HashMap::new();
                        for st in core::r#match::player::state::PlayerState::all() {
                            label.insert(st.compact_id(), format!("{}", st));
                        }
                        let mut rows: Vec<(usize, u64)> = by_state
                            .iter()
                            .enumerate()
                            .filter(|(_, c)| **c > 0)
                            .map(|(i, c)| (i, *c))
                            .collect();
                        rows.sort_by(|a, b| b.1.cmp(&a.1));
                        for (id, c) in rows.iter().take(6) {
                            println!(
                                "       struck by a player in {:<28} {:>5}",
                                label
                                    .get(&(*id as u16))
                                    .cloned()
                                    .unwrap_or_else(|| format!("state {id}")),
                                c
                            );
                        }
                    }

                    let jumps = core::flight_diag::FlightDiag::jump_snapshot();
                    // Restarts move the ball on purpose (a throw-in puts
                    // it on the touchline); only the rest are unexplained.
                    let restart_total: u64 =
                        core::flight_diag::RESTART_STAGES.map(|i| jumps[i].0).sum();
                    let jump_total: u64 = jumps.iter().map(|j| j.0).sum::<u64>() - restart_total;
                    println!(
                        "    relocations: {} unexplained ({:.1}/match) + {} restart placements \
                         ({:.1}/match)",
                        jump_total,
                        jump_total as f32 / n_matches as f32,
                        restart_total,
                        restart_total as f32 / n_matches as f32,
                    );
                    for (stage, (n, mean, max, peak)) in
                        core::flight_diag::STAGES.iter().zip(jumps.iter())
                    {
                        // A stage with no jumps still matters if it left
                        // the ball travelling faster than the physics cap
                        // — that is where a runaway velocity is born.
                        if *n > 0 || *peak > 8.0 {
                            println!(
                                "      {:<16} {:>7} jumps, mean {:>6.1}u, worst {:>6.1}u ({:.1}m), peak speed {:.2}u/tick",
                                stage,
                                n,
                                mean,
                                max,
                                max * 0.125,
                                peak
                            );
                        }
                    }

                    // ── WHOLE-TICK RELOCATION CENSUS ────────────────────
                    //
                    // The table above only covers `Ball::update`. This one
                    // covers the whole tick — the aerial contests, the
                    // player layer, the restart drains — and it is where
                    // every "the ball teleported" report has actually
                    // lived. `visible` is the subset a 3D replay shows: a
                    // relocation of half a metre or more.
                    {
                        let tel = core::teleport::TeleportCensus::snapshot();
                        let ticks = core::teleport::TeleportCensus::ticks().max(1);
                        // The `∟` rows break the ball's own pass down; they
                        // are already counted in `ball_update`.
                        let is_total = |i: usize| !core::teleport::SUBROWS.contains(&i);
                        let total: u64 = tel
                            .iter()
                            .enumerate()
                            .filter(|(i, _)| is_total(*i))
                            .map(|(_, s)| s.0)
                            .sum();
                        let visible: u64 = tel
                            .iter()
                            .enumerate()
                            .filter(|(i, _)| is_total(*i))
                            .map(|(_, s)| s.3)
                            .sum();
                        let metres: f32 = tel
                            .iter()
                            .enumerate()
                            .filter(|(i, _)| is_total(*i))
                            .map(|(_, s)| s.0 as f32 * s.1 * 0.125)
                            .sum::<f32>();
                        println!(
                            "    WHOLE-TICK relocations: {} ({:.1}/match), {} viewer-visible \
                             ({:.1}/match), {:.0} m of teleport per match, over {} ticks",
                            total,
                            total as f32 / n_matches as f32,
                            visible,
                            visible as f32 / n_matches as f32,
                            metres / n_matches as f32,
                            ticks,
                        );
                        let mut rows: Vec<_> = core::teleport::STAGES
                            .iter()
                            .zip(tel.iter())
                            .filter(|(_, s)| s.0 > 0)
                            .collect();
                        // Ranked by the thing that matters — metres of
                        // visible ball movement a match, not raw counts.
                        rows.sort_by(|a, b| {
                            (b.1.0 as f32 * b.1.1)
                                .partial_cmp(&(a.1.0 as f32 * a.1.1))
                                .unwrap_or(std::cmp::Ordering::Equal)
                        });
                        for (stage, (n, mean, max, vis, dead, vert)) in rows {
                            println!(
                                "      {:<34} {:>7} ({:>5.2}/match) mean {:>6.1}u ({:>5.2}m), \
                                 worst {:>7.1}u ({:>5.1}m) — {} visible, {} dead-ball, {} of those VERTICAL",
                                stage,
                                n,
                                *n as f32 / n_matches as f32,
                                mean,
                                mean * 0.125,
                                max,
                                max * 0.125,
                                vis,
                                dead,
                                vert,
                            );
                        }
                        // The flight the aerial contests now give their ball
                        // instead of writing it onto the winner's head. The
                        // win rate is measured in the SET PIECES block; this
                        // is the question the flight introduces — does the
                        // ball actually get to him?
                        let (armed, arrived, lost, flight, gap) =
                            core::teleport::TeleportCensus::delivery_snapshot();
                        if armed > 0 {
                            println!(
                                "      aerial deliveries: {} ({:.2}/match) flown a mean {:.2} s — \
                                 {:.0}% reached the winner ({:.1}u from him), {:.0}% timed out",
                                armed,
                                armed as f32 / n_matches as f32,
                                flight / 100.0,
                                arrived as f32 * 100.0 / armed as f32,
                                gap,
                                lost as f32 * 100.0 / armed as f32,
                            );
                        }
                        // ── THE TWENTY-TWO ──────────────────────────────
                        //
                        // Read this table against the one above, in the
                        // same currency. The ball census was built for a
                        // report about the BALL; on a corner the ball
                        // moved 16 m and seventeen players moved 30 m
                        // each, so a relocation census that watches only
                        // the ball is measuring the small half.
                        {
                            let pl = core::teleport::PlayerTeleportCensus::snapshot();
                            let expected = core::teleport::PLAYER_EXPECTED;
                            let is_expected = |i: usize| expected.contains(&i);
                            let metres: f32 = pl
                                .iter()
                                .enumerate()
                                .filter(|(i, _)| !is_expected(*i))
                                .map(|(_, s)| s.0 as f32 * s.1 * 0.125)
                                .sum::<f32>();
                            let moved: u64 = pl
                                .iter()
                                .enumerate()
                                .filter(|(i, _)| !is_expected(*i))
                                .map(|(_, s)| s.0)
                                .sum();
                            println!(
                                "    PLAYER relocations: {} ({:.1}/match), {:.0} m of player \
                                 teleport per match  (substitutions and dismissals excluded — \
                                 those are supposed to move somebody)",
                                moved,
                                moved as f32 / n_matches as f32,
                                metres / n_matches as f32,
                            );
                            let mut rows: Vec<_> = core::teleport::PLAYER_SITES
                                .iter()
                                .zip(pl.iter())
                                .enumerate()
                                .filter(|(_, (_, s))| s.0 > 0 || s.3 > 0)
                                .collect();
                            rows.sort_by(|a, b| {
                                (b.1.1.0 as f32 * b.1.1.1)
                                    .partial_cmp(&(a.1.1.0 as f32 * a.1.1.1))
                                    .unwrap_or(std::cmp::Ordering::Equal)
                            });
                            for (i, (site, (n, mean, max, firings))) in rows {
                                println!(
                                    "      {:<32} {:>6.2} firings/match x {:>4.1} players, \
                                     mean {:>6.1}u ({:>5.2}m), worst {:>6.1}u ({:>5.1}m) \
                                     = {:>6.0} m/match{}",
                                    site,
                                    *firings as f32 / n_matches as f32,
                                    *n as f32 / (*firings).max(1) as f32,
                                    mean,
                                    mean * 0.125,
                                    max,
                                    max * 0.125,
                                    *n as f32 * mean * 0.125 / n_matches as f32,
                                    if is_expected(i) { "  (expected)" } else { "" },
                                );
                            }
                        }

                        let ev = core::teleport::TeleportCensus::event_snapshot();
                        let mut ev_rows: Vec<_> = core::teleport::EVENT_LABELS
                            .iter()
                            .zip(ev.iter())
                            .filter(|(_, e)| e.0 > 0)
                            .collect();
                        ev_rows.sort_by(|a, b| {
                            (b.1.0 as f32 * b.1.1)
                                .partial_cmp(&(a.1.0 as f32 * a.1.1))
                                .unwrap_or(std::cmp::Ordering::Equal)
                        });
                        if !ev_rows.is_empty() {
                            println!("      — the `dispatch` row, split by handler —");
                            for (label, (n, mean, max)) in ev_rows {
                                println!(
                                    "        {:<24} {:>7} ({:>5.2}/match) mean {:>6.1}u ({:>5.2}m), worst {:>7.1}u ({:>5.1}m)",
                                    label,
                                    n,
                                    *n as f32 / n_matches as f32,
                                    mean,
                                    mean * 0.125,
                                    max,
                                    max * 0.125,
                                );
                            }
                        }
                    }

                    let (icept, mean_z, above, no_leap, headers, headers_air) =
                        core::flight_diag::FlightDiag::aerial_snapshot();
                    println!(
                        "    interceptions {} at mean height {:.2}m — {} above standing reach \
                         ({:.1}%), of which {} taken WITHOUT leaving the ground (must be 0)",
                        icept,
                        mean_z,
                        above,
                        above as f32 / icept.max(1) as f32 * 100.0,
                        no_leap,
                    );
                    if headers > 0 {
                        println!(
                            "    headers {} — {:.1}% won in the air",
                            headers,
                            headers_air as f32 / headers as f32 * 100.0
                        );
                    }
                }

                let (emitted, superseded, dead, out_of_reach) =
                    core::reception_diag::pass_outcome_snapshot();
                println!(
                    "  pass outcomes: emitted {} — superseded while unresolved {:.1}%, \
                     died on a dead ball {:.1}%; rejected as out of reach {} ({:.2}/match)",
                    emitted,
                    superseded as f32 / emitted.max(1) as f32 * 100.0,
                    dead as f32 / emitted.max(1) as f32 * 100.0,
                    out_of_reach,
                    out_of_reach as f32 / n_matches as f32,
                );
            }
            let (seen, too_high, candidates, fired) = BlockDiag::snapshot();
            let bpct = |x: u64| {
                if seen == 0 {
                    0.0
                } else {
                    x as f32 / seen as f32 * 100.0
                }
            };
            // NB every percentage here is per BALL-TICK IN FLIGHT, not
            // per shot — `seen` counts each tick a live shot spends in
            // the check, ~15 of them per shot. Printing "blocked 1.0%"
            // beside "(real: ~18-22% of shots blocked)" invited exactly
            // the comparison it looks like, and that comparison is wrong
            // by the flight length: 1.0% of ticks is ~7 blocks a match, a
            // NORMAL number. Two rounds of defensive work were read as
            // having achieved nothing on the strength of it. The per-shot
            // rate is derived below; the per-player-per-match count in
            // the RATING VOLUME PROFILE (`blocks`, real ~0.9 for a
            // defender) is the other honest readout.
            let per_shot = if total_shots > 0 {
                fired as f32 / total_shots as f32 * 100.0
            } else {
                0.0
            };
            println!(
                "  block window: {} ball-ticks in flight reached the check — above blocking \
                 height {:.1}%, defender in the lane {:.1}%, blocked {:.1}% (all PER TICK)",
                seen,
                bpct(too_high),
                bpct(candidates),
                bpct(fired),
            );
            println!(
                "    → {} blocks over {} shots = {:.1}% of shots blocked   (real: ~18-22%)",
                fired, total_shots, per_shot,
            );
            let (opp, behind, beyond, wide, in_win, mean_perp) = BlockDiag::lane_snapshot();
            let opct = |x: u64| {
                if opp == 0 {
                    0.0
                } else {
                    x as f32 / opp as f32 * 100.0
                }
            };
            let (struck, goalside, near_line, range, depth) = BlockDiag::strike_snapshot();
            println!(
                "  at the strike: {} shots — opposition outfielders goal-side of the ball \
                 {:.2}/shot, of those within 30u of the ball's line to goal {:.2}/shot   \
                 (real: 2-4 goal-side, ~1 in the lane)",
                struck, goalside, near_line,
            );
            println!(
                "  at the strike: shot range {:.0}u ({:.1}m) from goal, defending outfielders \
                 sit {:.0}u ({:.1}m) from their own line   (defenders FURTHER out than the ball \
                 = the line never dropped)",
                range,
                range * 0.125,
                depth,
                depth * 0.125,
            );
            {
                // How much SPACE the shooter had. The shot models only
                // price a defender inside `pressure_count_10u` — 10u is
                // 1.25 m — so every band from "1-2m" rightward is a shot
                // the engine currently treats as completely unpressured.
                let bands = BlockDiag::shot_pressure_snapshot();
                let total: u64 = bands.iter().sum::<u64>().max(1);
                const EDGES: [&str; 6] = ["<1m", "1-2m", "2-3m", "3-5m", "5-8m", "8m+"];
                let row = EDGES
                    .iter()
                    .zip(bands.iter())
                    .map(|(e, c)| format!("{e} {:.0}%", *c as f64 / total as f64 * 100.0))
                    .collect::<Vec<_>>()
                    .join("  ");
                println!("  space at the strike (nearest defending outfielder to the ball): {row}");
                println!(
                    "    the shot models see pressure only inside 1.25m — {:.0}% of shots are \
                     struck with the nearest defender further out than that and are priced as \
                     if the shooter were alone",
                    (total - bands[0]) as f64 / total as f64 * 100.0,
                );
            }
            const DEF_STATE_NAMES: [&str; 21] = [
                "Standing",
                "Covering",
                "PushingUp",
                "Resting",
                "Passing",
                "Running",
                "Intercepting",
                "Marking",
                "Clearing",
                "Heading",
                "Tackling",
                "Pressing",
                "TrackingBack",
                "HoldingLine",
                "Returning",
                "Walking",
                "TakeBall",
                "Shooting",
                "Guarding",
                "AttackingCorner",
                "Crossing",
            ];
            let states = BlockDiag::defender_state_snapshot();
            let total: u64 = states.iter().sum();
            let mut ranked: Vec<(usize, u64)> = states
                .iter()
                .copied()
                .enumerate()
                .filter(|(_, c)| *c > 0)
                .collect();
            ranked.sort_by(|a, b| b.1.cmp(&a.1));
            let listed = ranked
                .iter()
                .take(10)
                .map(|(i, c)| {
                    format!(
                        "{} {:.0}%",
                        DEF_STATE_NAMES[*i],
                        *c as f32 / total.max(1) as f32 * 100.0
                    )
                })
                .collect::<Vec<_>>()
                .join(", ");
            println!("  at the strike: back-line state — {}", listed);
            println!(
                "  block lane: {} opponent-samples — behind the ball {:.1}%, \
                 beyond lookahead {:.1}%, in window {:.1}% (of those, wider than the corridor \
                 {:.1}%; mean perp {:.1}u)",
                opp,
                opct(behind),
                opct(beyond),
                opct(in_win),
                if in_win == 0 {
                    0.0
                } else {
                    wide as f32 / in_win as f32 * 100.0
                },
                mean_perp,
            );
        }
        for (label, dv, mv, fv, real) in rows {
            println!(
                "  {:<22} {:>6.2} {:>6.2} {:>6.2}    {}",
                label, dv, mv, fv, real
            );
        }
        let pct = |v: &RatingVolumeAgg, x: u32| -> f32 {
            if v.samples == 0 {
                0.0
            } else {
                x as f32 / v.samples as f32 * 100.0
            }
        };
        println!(
            "  pass%                  {:>5.1}% {:>5.1}% {:>5.1}%    (retention baseline 0.74)",
            if d.passes_attempted == 0 {
                0.0
            } else {
                d.passes_completed as f32 / d.passes_attempted as f32 * 100.0
            },
            if m.passes_attempted == 0 {
                0.0
            } else {
                m.passes_completed as f32 / m.passes_attempted as f32 * 100.0
            },
            if f.passes_attempted == 0 {
                0.0
            } else {
                f.passes_completed as f32 / f.passes_attempted as f32 * 100.0
            },
        );
        println!(
            "  Strong via routine_def>=7: DEF {:.0}% MID {:.0}% FWD {:.0}%   (real: rare, <5%)",
            pct(d, d.routine_def_ge7),
            pct(m, m.routine_def_ge7),
            pct(f, f.routine_def_ge7),
        );
        println!(
            "  Strong via zone_impact>=2: DEF {:.0}% MID {:.0}% FWD {:.0}%   (real: ~10-15% DEF)",
            pct(d, d.zone_impact_ge2),
            pct(m, m.zone_impact_ge2),
            pct(f, f.zone_impact_ge2),
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
    // ── WHERE CORNERS COME FROM ────────────────────────────────────────
    //
    // Three tagged suppliers plus a remainder. Corner SUPPLY is the one
    // number nothing else in this file can explain: it is not a rate you
    // can read off shots or crosses, it is the sum of four independent
    // mechanisms, and a shortfall in any one of them looks identical in
    // the headline. Real per-match reference ≈ 10.4 corners.
    {
        let per = |v: u64| v as f64 / n as f64;
        let total = mr[6].max(1);
        let tagged = mr[14] + mr[15] + mr[16];
        let share = |v: u64| v as f64 * 100.0 / total as f64;
        println!(
            "  corner sources /match: shot BLOCKED wide {:.2} ({:.0}%)   keeper PARRIED wide \
             {:.2} ({:.0}%)   delivery HOOKED behind {:.2} ({:.0}%)   ordinary play {:.2} ({:.0}%)",
            per(mr[14]),
            share(mr[14]),
            per(mr[15]),
            share(mr[15]),
            per(mr[16]),
            share(mr[16]),
            per(mr[6].saturating_sub(tagged)),
            share(mr[6].saturating_sub(tagged)),
        );
    }

    // ── SET PIECES ─────────────────────────────────────────────────────
    //
    // The two means printed here are what `CORNER_DELIVERY_REFERENCE` and
    // `PENALTY_EXECUTION_REFERENCE` must be set to. Both constants centre
    // a skill term on the population of *selected* takers, so if the
    // constant and the measured mean disagree the term stops being
    // redistributive and starts shifting league-wide conversion.
    {
        use core::mid_run_diag::SetPieceDiag;
        let sp = SetPieceDiag::snapshot();
        let corners: u64 = sp[0] + sp[1] + sp[2] + sp[3] + sp[4];
        let pct = |v: u64| {
            if corners == 0 {
                0.0
            } else {
                v as f64 / corners as f64 * 100.0
            }
        };
        println!("\n--- SET PIECES ---");
        println!(
            "  corner routines ({corners}): near {} ({:.0}%)  spot {} ({:.0}%)  far {} ({:.0}%)  \
             short {} ({:.0}%)  edge {} ({:.0}%)",
            sp[0],
            pct(sp[0]),
            sp[1],
            pct(sp[1]),
            sp[2],
            pct(sp[2]),
            sp[3],
            pct(sp[3]),
            sp[4],
            pct(sp[4]),
        );
        println!(
            "  corner taker delivery mean {:.3} over {} corners   \
             ← set CORNER_DELIVERY_REFERENCE to this",
            sp[5] as f64 / 1000.0,
            sp[6],
        );
        println!(
            "  penalty taker execution mean {:.3} over {} penalties  \
             ← set PENALTY_EXECUTION_REFERENCE to this",
            sp[7] as f64 / 1000.0,
            sp[8],
        );
        // ⚠ These three are TICK-scale, not event-scale: the pass
        // evaluator and the shot helper both re-run every tick the taker
        // stands over the ball, so a single long throw or free kick is
        // counted many times. Read them as "does this path fire at all,
        // and in what ratio", never as a per-match rate.
        println!(
            "  long-throw decisions: {}   direct-FK decisions: shoot {} / deliver {}  \
             (tick-scale, not per set piece)",
            sp[9], sp[10], sp[11],
        );
        // The corner BOX CENSUS: who is actually standing in the penalty
        // area at the instant the delivery is struck — OUTFIELDERS ONLY,
        // both keepers excluded, so the real-world comparison is 7-9 a
        // side rather than the 8-10 you get counting the goalkeeper.
        //
        // This is the number the corner set-up (`CornerShape`) exists to
        // move. Before it there was no set-up at all — the shape was
        // whatever open play left behind — and reading it off a recorded
        // match gave 3-6 defenders with a floor of NONE: a goalkeeper
        // alone in his box while the cross came in, which is what a
        // player reported after watching it in the 3D viewer. Read the
        // MINIMUM as hard as the mean: the mean can look respectable
        // while the tail is a deserted goalmouth.
        println!(
            "  corner box census AT THE SET-UP over {} corners: DEFENDERS {:.1} \
             (real ~7-9, worst seen {})  ATTACKERS {:.1} (real ~5-7)",
            sp[21],
            sp[19] as f64 / 100.0,
            sp[22],
            sp[20] as f64 / 100.0,
        );
        // …and the same count when the delivery is airborne, which also
        // says whether the shape SURVIVED the flight. Its worst-case is
        // noisier than the set-up's on purpose: the resolver that samples
        // it fires on the first airborne ownerless tick with a live
        // `Corner` origin, and that origin outlives the set piece, so a
        // ball hooked up two seconds later at the other end lands in this
        // sample. Compare the two means; trust the set-up's minimum.
        println!(
            "  …and AT THE DELIVERY over {}: DEFENDERS {:.1} (worst seen {})  ATTACKERS {:.1}",
            sp[14],
            sp[12] as f64 / 100.0,
            sp[15],
            sp[13] as f64 / 100.0,
        );
        // How long the stations pin anyone, and which of the two exits
        // let go. A deadline release lets go EARLY — before anybody has
        // reached the ball — so it is the safe direction; a first-contact
        // release means the football ended the set piece. Both are fine.
        // What is NOT fine is the third case this replaced: releasing
        // only on the restart origin, which needs a TOUCH, which the pin
        // itself prevents — that deadlocked at ~7 s of frozen shape.
        println!(
            "  corner shape held {:.2} s per corner, {:.0}% of them released by the \
             DEADLINE rather than by first contact   (real corner ~1-2 s)",
            sp[16] as f64 / 100.0 / 100.0,
            if sp[18] == 0 {
                0.0
            } else {
                sp[17] as f64 * 100.0 / sp[18] as f64
            },
        );
    }

    // ── WIDE PLAY ──────────────────────────────────────────────────────
    //
    // Two separate questions the aggregate cross count cannot answer, and
    // both of them are the report ("no attacks down the flanks, no
    // crosses from the touchline"): does anybody actually stand on a
    // touchline, and when the ball is delivered, is it from the byline or
    // hopefully from 35 m?
    {
        use core::mid_run_diag::WideDiag;
        let w = WideDiag::snapshot();
        let per = |v: u64| v as f64 / n_matches as f64;
        if w[0] > 0 {
            let deliveries = w[5] + w[6] + w[7];
            println!();
            println!("--- WIDE PLAY (per match unless stated) ---");
            println!(
                "  touchline-holder ticks {:.0}, of which genuinely in the outer 20%: {:.1}%   \
                 (the plan vs the shape)",
                per(w[0]),
                w[1] as f64 * 100.0 / w[0].max(1) as f64
            );
            println!(
                "  overlap-run ticks {:.0}   at the byline (<12 m from the goal line) {:.1}% of wide ticks",
                per(w[2]),
                w[3] as f64 * 100.0 / w[0].max(1) as f64
            );
            println!("  flank releases played {:.1}", per(w[4]));
            println!(
                "  deliveries {:.1}/match ({:.1} per team, real ~16-18): DEF {:.1} / MID {:.1} / FWD {:.1}",
                per(deliveries),
                per(deliveries) / 2.0,
                per(w[5]),
                per(w[6]),
                per(w[7]),
            );
            println!(
                "  …of them from inside the box's width (a byline ball) {:.1}%; mean distance from the goal line {:.1} m",
                w[8] as f64 * 100.0 / deliveries.max(1) as f64,
                (w[9] as f64 / deliveries.max(1) as f64) * 0.125,
            );
            println!(
                "  real reference: a Premier League team crosses 16-18 times a match, of which ~25% are cut back or struck from inside the width of the box"
            );
        }
    }

    // ── OVERLAPPING FULLBACK FUNNEL ────────────────────────────────────
    {
        use core::mid_run_diag::OverlapDiag;
        let f = OverlapDiag::snapshot();
        if f[0] > 0 {
            println!();
            println!("--- OVERLAPPING FULLBACK FUNNEL (survivors at each gate, /match) ---");
            for (i, name) in [
                "asked",
                "is wide",
                "we have the ball",
                "phase Attack/Progression",
                "team width > 0.45",
                "profile allows",
                "ball on same flank",
                "ball ahead of him",
                "enough rest defence",
                "COMMITTED",
            ]
            .iter()
            .enumerate()
            {
                println!(
                    "    {:<28} {:>10.0}  ({:.1}% of asked)",
                    name,
                    f[i] as f64 / n_matches as f64,
                    f[i] as f64 * 100.0 / f[0].max(1) as f64
                );
            }
        }
    }

    // ── KEEPER SWEEP FUNNEL ────────────────────────────────────────────
    {
        use core::mid_run_diag::KeeperSweepDiag;
        let s = KeeperSweepDiag::snapshot();
        if s[0] > 0 {
            println!();
            println!(
                "--- KEEPER SWEEP FUNNEL --- {:.0} come-out questions/match → carrier exists \
                 {:.0} → inside scan {:.0} → nobody covering him {:.0} → COMMITTED {:.1}",
                s[0] as f64 / n_matches as f64,
                s[1] as f64 / n_matches as f64,
                s[2] as f64 / n_matches as f64,
                s[4] as f64 / n_matches as f64,
                s[3] as f64 / n_matches as f64
            );
            let e = KeeperSweepDiag::exits();
            println!(
                "    sweep abandoned by: got-the-ball {} beyond-pursuit {} \
                 ball-crossed-halfway {}",
                e[0], e[5], e[6]
            );
            println!(
                "                        TOO-FAR-FROM-SLOT {} opponent-too-close {} \
                 carrier-going-away {} shot-in-flight {}",
                e[8], e[7], e[9], e[10]
            );
            println!(
                "    save REACTION set: diving {:.1}/match  catching {:.1}  blocked {:.1}",
                e[1] as f64 / n_matches as f64,
                e[3] as f64 / n_matches as f64,
                e[4] as f64 / n_matches as f64
            );
        }
    }

    // ── KEEPER GUARD CENSUS ────────────────────────────────────────────
    // Where the keeper actually STANDS while his goal is under threat.
    // Every other keeper block counts events; none of them can see a
    // keeper who is simply in the wrong place, because that failure emits
    // nothing at all — it shows up as a shot he never got near, which is
    // indistinguishable in `SAVE ACCOUNTING` from a shot that was too good.
    {
        use core::mid_run_diag::KeeperGuardDiag;
        let g = KeeperGuardDiag::snapshot();
        if g[0] > 0 {
            let ticks = g[0] as f64;
            println!();
            println!(
                "--- KEEPER GUARD CENSUS ({:.0} threat ticks/match — ball live within 37.5 m) ---",
                ticks / n_matches as f64
            );
            println!(
                "  off the goal→ball line {:.2} m   off his own line {:.2} m   \
                 WRONG SIDE {:.1}%   standing still {:.1}%",
                g[1] as f64 / 100.0 / ticks * 0.125,
                g[2] as f64 / 100.0 / ticks * 0.125,
                g[3] as f64 * 100.0 / ticks,
                g[4] as f64 * 100.0 / ticks
            );
            // Does reading the game buy anything? If these two rows are
            // the same number then every keeper in the game stands in the
            // same place and positioning is a decorative attribute.
            if g[13] > 0 && g[15] > 0 {
                println!(
                    "  off-angle by keeper READ: sharp (top third) {:.2} m over {:.0} ticks   \
                     vs dull (bottom third) {:.2} m over {:.0} ticks",
                    g[14] as f64 / 100.0 / g[13] as f64 * 0.125,
                    g[13] as f64 / n_matches as f64,
                    g[16] as f64 / 100.0 / g[15] as f64 * 0.125,
                    g[15] as f64 / n_matches as f64
                );
            }
            println!(
                "  population mean of the keeper positioning composite: {:.3}   \
                 (any quality term multiplying a calibrated quantity must be centred here)",
                g[21] as f64 / 1000.0 / ticks
            );
            // Was diving into the corner worth it? The dive is supposed to
            // put him NEARER the crossing point than shuffling would; if
            // this row is worse than the one beside it, it is aimed wrong.
            if g[22] > 0 && g[24] > 0 {
                println!(
                    "  lateral error AT THE SAVE, already DIVING {:.2} m ({} shots)   \
                     vs still on his feet {:.2} m ({} shots)",
                    g[23] as f64 / 100.0 / g[22] as f64 * 0.125,
                    g[22],
                    g[25] as f64 / 100.0 / g[24] as f64 * 0.125,
                    g[24]
                );
            }
            // The one that matters: how far off the shot was he WHEN IT
            // ARRIVED. If these two are equal then position selection is a
            // decorative attribute and all keeper quality lives in the
            // save roll.
            if g[17] > 0 && g[19] > 0 {
                println!(
                    "  lateral error AT THE SAVE by keeper READ: sharp {:.2} m ({} shots)   \
                     vs dull {:.2} m ({} shots)",
                    g[18] as f64 / 100.0 / g[17] as f64 * 0.125,
                    g[17],
                    g[20] as f64 / 100.0 / g[19] as f64 * 0.125,
                    g[19]
                );
            }
            if g[5] > 0 {
                let c = g[5] as f64;
                println!(
                    "  with a CARRIER inside 25 m ({:.0}/match): off-angle {:.2} m   \
                     ComingOut {:.1}%   ReturningToGoal {:.1}%   Standing/Walking {:.1}%",
                    c / n_matches as f64,
                    g[9] as f64 / 100.0 / c * 0.125,
                    g[6] as f64 * 100.0 / c,
                    g[7] as f64 * 100.0 / c,
                    g[8] as f64 * 100.0 / c
                );
            }
            let beaten = g[10];
            let rolled = g[12];
            if beaten + rolled > 0 {
                println!(
                    "  shots arriving on frame: {:.1}/match reached his save roll, \
                     {:.1}/match were BEYOND HIS REACH ({:.0}% — mean miss {:.2} m)",
                    rolled as f64 / n_matches as f64,
                    beaten as f64 / n_matches as f64,
                    beaten as f64 * 100.0 / (beaten + rolled) as f64,
                    if beaten > 0 {
                        g[11] as f64 / 100.0 / beaten as f64 * 0.125
                    } else {
                        0.0
                    }
                );
            }
        }
    }

    // ── SAVE CONTACT CENSUS ────────────────────────────────────────────
    // WHERE the ball turns when a shot is resolved, and how far that is
    // from the man credited with turning it. A save the viewer can read
    // is one where the two coincide; a save booked five metres from the
    // keeper draws as a ball bouncing off nothing.
    {
        use core::mid_run_diag::SaveContactDiag;
        let s = SaveContactDiag::snapshot();
        if s[8] > 0 {
            let row = |name: &str, n: u64, sum: u64| {
                if n > 0 {
                    println!(
                        "  {name:<22} {:>6.2}/match   gap ball→player {:>5.2} m",
                        n as f64 / n_matches as f64,
                        sum as f64 / 100.0 / n as f64 * 0.125
                    );
                }
            };
            println!();
            println!("--- SAVE CONTACT CENSUS (how far the deflection is from the deflector) ---");
            row("catch", s[0], s[1]);
            row("parry for a corner", s[2], s[3]);
            row("spilled parry", s[4], s[5]);
            row("outfield block", s[6], s[7]);
            println!(
                "  ball HEIGHT at resolution {:.2} m over {} resolutions; \
                 {:.1}% happened with nobody inside 2.5 m",
                s[9] as f64 / 100.0 / s[8] as f64,
                s[8],
                s[10] as f64 * 100.0 / s[8] as f64
            );
            // Which axis the gap lives on decides the fix — see
            // `SAVE_CONTACT`'s doc comment.
            println!(
                "  that gap splits {:.2} m ALONG the goal axis / {:.2} m ACROSS it",
                s[11] as f64 / 100.0 / s[8] as f64 * 0.125,
                s[12] as f64 / 100.0 / s[8] as f64 * 0.125
            );
        }
        // HOW THE KEEPER COMES TO BE HOLDING IT. The save-contact census
        // above only sees the physics path; most gathers arrive through the
        // state machine, which does not put the ball where the contact was
        // — it grants ownership wherever the ball is and lets `move_to`
        // drag it in. This is the census of that grant.
        {
            use core::mid_run_diag::KeeperGatherDiag;
            let g = KeeperGatherDiag::snapshot();
            if g[0] + g[15] > 0 {
                let n = g[0].max(1) as f64;
                println!();
                println!("--- KEEPER GATHER CENSUS (how the ball gets into his gloves) ---");
                println!(
                    "  state-machine grants {:>5.2}/match   physics catches {:>5.2}/match   \
                     refused out-of-reach {:>5.2}/match",
                    g[0] as f64 / n_matches as f64,
                    g[15] as f64 / n_matches as f64,
                    g[14] as f64 / n_matches as f64
                );
                println!(
                    "  gap ball→keeper at the grant: mean {:.2} m, worst {:.2} m   \
                     (physics path {:.2} m)",
                    g[1] as f64 / 100.0 / n * 0.125,
                    g[2] as f64 / 100.0 * 0.125,
                    if g[15] > 0 {
                        g[16] as f64 / 100.0 / g[15] as f64 * 0.125
                    } else {
                        0.0
                    }
                );
                println!(
                    "  over 1 m {:>5.1}%   over 1.5 m {:>5.1}%   ball still doing >12.5 m/s \
                     when it stopped dead {:>5.1}%   mean {:.1} m/s",
                    g[3] as f64 * 100.0 / n,
                    g[4] as f64 * 100.0 / n,
                    g[6] as f64 * 100.0 / n,
                    g[5] as f64 / 100.0 / n * 12.5
                );
                println!(
                    "  posture: airborne {:>5.1}%   diving {:>5.1}%   ON THE FLOOR mid-dive {:>5.1}%",
                    g[7] as f64 * 100.0 / n,
                    g[8] as f64 * 100.0 / n,
                    g[9] as f64 * 100.0 / n
                );
                println!(
                    "  live shot on the ball {:>5.1}%   of those OFF THE FRAME {:>5.1}% \
                     (wide {:.1}%, over {:.1}%)  ← a miss he collected",
                    g[10] as f64 * 100.0 / n,
                    g[11] as f64 * 100.0 / g[10].max(1) as f64,
                    g[12] as f64 * 100.0 / g[10].max(1) as f64,
                    g[13] as f64 * 100.0 / g[10].max(1) as f64
                );
                let by_state = KeeperGatherDiag::state_snapshot();
                let names: [(usize, &str); 17] = [
                    (0, "Standing"),
                    (2, "Jumping"),
                    (3, "Diving"),
                    (4, "Catching"),
                    (5, "Punching"),
                    (6, "Kicking"),
                    (7, "Clearing"),
                    (8, "HoldingBall"),
                    (9, "Throwing"),
                    (10, "PickingUpBall"),
                    (11, "Distributing"),
                    (12, "ComingOut"),
                    (13, "Passing"),
                    (14, "ReturningToGoal"),
                    (17, "PreparingForSave"),
                    (18, "Walking"),
                    (19, "TakeBall"),
                ];
                println!("  by the state he was in when the ball was handed to him:");
                for (id, name) in names {
                    let count = by_state[id * 2];
                    if count == 0 {
                        continue;
                    }
                    println!(
                        "    {name:<18} {:>6.2}/match  ({:>5.1}%)   airborne {:>5.1}%",
                        count as f64 / n_matches as f64,
                        count as f64 * 100.0 / n,
                        by_state[id * 2 + 1] as f64 * 100.0 / count as f64
                    );
                }
            }
        }

        // The woodwork. Real football strikes the frame about 0.5 times a
        // match; a count far above that means the posts are protruding into
        // the goal, and a zero means the swept test never fires.
        let f = core::mid_run_diag::FrameDiag::snapshot();
        let frame_hits = f[0] + f[1] + f[2];
        if frame_hits > 0 {
            println!(
                "  WOODWORK {:.2}/match  (posts {:.2}, bar {:.2})   real ref ~0.5/match",
                frame_hits as f64 / n_matches as f64,
                (f[0] + f[1]) as f64 / n_matches as f64,
                f[2] as f64 / n_matches as f64
            );
        }
    }

    // ── GOALKEEPER STATE CENSUS ────────────────────────────────────────
    // The general shape census cuts at 0.25% of ALL ticks, and every
    // reflex state a keeper has — the dive, the leap, the punch — is two
    // orders of magnitude below that by construction (two keepers, a
    // handful of events each, half a second apiece). So the states the
    // report "he never dives, he isn't in the game" is actually about are
    // invisible in it. Print the keeper's own breakdown, uncut, with the
    // ENTRY COUNT next to the tick share: a state can be entered often and
    // still hold no time, and those two failures need different fixes.
    {
        use core::mid_run_diag::ShapeCensus;
        let rows = ShapeCensus::snapshot();
        // `compact_id` is role-banded: goalkeepers are 100..200.
        let gk: Vec<_> = rows.iter().filter(|r| (100..200).contains(&r.0)).collect();
        let gk_ticks: u64 = gk.iter().map(|r| r.1).sum();
        if gk_ticks > 0 {
            println!();
            println!("--- GOALKEEPER STATE CENSUS (share of the two keepers' own ticks) ---");
            println!(
                "  {:<26} {:>10}  {:>7}  {:>10}  {:>7}",
                "state", "ticks", "share", "ticks/match", "still"
            );
            let mut sorted = gk.clone();
            sorted.sort_by(|a, b| b.1.cmp(&a.1));
            for (id, ticks, _lag, _axis, still) in sorted {
                println!(
                    "  {:<26} {:>10} {:>6.2}%  {:>10.1}  {:>6.0}%",
                    StateNames::of(*id),
                    ticks,
                    *ticks as f64 * 100.0 / gk_ticks as f64,
                    *ticks as f64 / n_matches as f64,
                    still * 100.0
                );
            }
            // A state with no ticks at all drops out of the census
            // entirely, which reads as "fine" when it is the loudest
            // possible signal — `Jumping` had no inbound transition in the
            // whole engine for months and nothing said so. Walk the state
            // universe and name the silent ones.
            for state in PlayerState::all() {
                let id = state.compact_id();
                if (100..200).contains(&id) && !gk.iter().any(|r| r.0 == id) {
                    println!("  {:<26} {:>10}  ** NEVER ENTERED **", state.to_string(), 0);
                }
            }
        }
    }

    // ── KEEPER MOTION CENSUS ───────────────────────────────────────────
    // What he does with the whole match rather than with the 20-35% of it
    // his goal is under threat. Every other keeper block is conditioned on
    // the threat and so cannot see the reported behaviour at all — a
    // keeper who spends the hour play is at the other end jogging after
    // the ball emits no event of any kind.
    {
        use core::mid_run_diag::KeeperMotionDiag;
        let m = KeeperMotionDiag::snapshot();
        let ticks: u64 = (0..4).map(|b| m[b * 4]).sum();
        if ticks > 0 {
            let keeper_matches = (n_matches * 2) as f64;
            // Velocity is units per ENGINE tick and the AI samples every
            // second one, so each sample stands for two ticks of travel.
            let total_m: f64 =
                (0..4).map(|b| m[b * 4 + 1] as f64).sum::<f64>() / 1000.0 * 0.125 * 2.0;
            let still: u64 = (0..4).map(|b| m[b * 4 + 2]).sum();
            println!();
            println!("--- KEEPER MOTION CENSUS (every keeper tick, not just threat ticks) ---");
            println!(
                "  distance {:.0} m per keeper per match (real ~5000)   still {:.0}% of ticks   \
                 mean depth off his line {:.1} m",
                total_m / keeper_matches,
                still as f64 * 100.0 / ticks as f64,
                (0..4).map(|b| m[b * 4 + 3] as f64).sum::<f64>() / 100.0 / ticks as f64 * 0.125
            );
            println!("  ball from his goal      time%     m/match     still%    mean depth");
            for (b, label) in ["< 12 m", "12-25 m", "25-40 m", "beyond 40 m"]
                .iter()
                .enumerate()
            {
                let t = m[b * 4];
                if t == 0 {
                    continue;
                }
                println!(
                    "  {:<20} {:>6.1}%  {:>10.0}  {:>9.0}%  {:>9.1} m",
                    label,
                    t as f64 * 100.0 / ticks as f64,
                    m[b * 4 + 1] as f64 / 1000.0 * 0.125 * 2.0 / keeper_matches,
                    m[b * 4 + 2] as f64 * 100.0 / t as f64,
                    m[b * 4 + 3] as f64 / 100.0 / t as f64 * 0.125
                );
            }
            // Churn. A transition out of a state entered under 300 ms ago
            // is two gates disagreeing, and every one of them re-aims his
            // steering — which is what the eye reads as chasing.
            println!(
                "  state transitions {:.0}/keeper/match, of which {:.0}% reversed within 300 ms   \
                 reversals over 90deg {:.0}/min",
                m[16] as f64 / keeper_matches,
                if m[16] > 0 {
                    m[17] as f64 * 100.0 / m[16] as f64
                } else {
                    0.0
                },
                m[20] as f64 / keeper_matches / 90.0
            );
        }
    }

    // ── KEEPER RETREAT CENSUS ──────────────────────────────────────────
    // "He returns to goal watching the play, then turns his back on it the
    // moment opponents arrive in the box." The viewer only ever opens his
    // heading onto his run when he is TRAVELLING — past 2.8 m/s, where his
    // side-step ends — so the drawn defect has an engine half that no
    // facing fix can reach: how much of his recovery is spent above a jog
    // with an attack arriving.
    {
        use core::mid_run_diag::KeeperRetreatDiag;
        let r = KeeperRetreatDiag::snapshot();
        if r[0] > 0 {
            let keeper_matches = (n_matches * 2) as f64;
            // Engine units a tick → metres a second: a unit is 0.125 m and
            // a tick is 10 ms.
            let mps = |milli: u64, over: u64| {
                if over == 0 {
                    0.0
                } else {
                    milli as f64 / 1000.0 / over as f64 * 12.5
                }
            };
            let share = |n: u64, of: u64| {
                if of == 0 {
                    0.0
                } else {
                    n as f64 * 100.0 / of as f64
                }
            };
            println!();
            println!(
                "--- KEEPER RETREAT CENSUS (every keeper tick; \"besieged\" = an opponent \
                 within his own area depth of his goal) ---"
            );
            println!(
                "  all ticks: mean {:.2} m/s   above 2.8 m/s {:.1}%   besieged {:.1}% of ticks \
                 (mean {:.2} m/s there)",
                mps(r[1], r[0]),
                share(r[4], r[0]),
                share(r[2], r[0]),
                mps(r[3], r[2])
            );
            println!(
                "  ABOVE 2.8 m/s WITH MEN IN HIS BOX {:.2}% of all his ticks   \
                 …and the ball also beyond 34 m of him {:.3}%  ← the frames the viewer used \
                 to draw with his back to the play",
                share(r[5], r[0]),
                share(r[6], r[0])
            );
            println!(
                "  ReturningToGoal {:.1}% of his ticks, {:.0} m/keeper/match, mean {:.2} m/s, \
                 peak {:.2} m/s",
                share(r[7], r[0]),
                r[14] as f64 / 1000.0 * 0.125 / keeper_matches,
                mps(r[8], r[7]),
                r[15] as f64 / 1000.0 * 12.5
            );
            println!(
                "  …of those: above 2.8 m/s {:.0}%   above 5 m/s {:.0}%   besieged {:.0}% \
                 (mean {:.2} m/s)   besieged AND above 2.8 m/s {:.0}%",
                share(r[11], r[7]),
                share(r[13], r[7]),
                share(r[9], r[7]),
                mps(r[10], r[9]),
                share(r[12], r[7])
            );
        }
    }

    // ── KEEPER CHASE EXITS ─────────────────────────────────────────────
    // The keeper's loose-ball chase is entered almost entirely by the
    // dispatcher's override rather than by a door of its own, and the
    // two-cycle census has repeatedly shown `Take Ball -> Returning to
    // Goal` at the top of the table with 100% of them reversing inside
    // 300 ms. This says WHICH of the state's exits does it, and whether it
    // fires on the tick he arrives — which is the difference between an
    // override and a state disagreeing, and a chase he simply lost.
    {
        use core::mid_run_diag::KeeperChaseDiag;
        let c = KeeperChaseDiag::snapshot();
        if c[0] + c[7] > 0 {
            let per = |v: u64| v as f64 / n_matches as f64;
            let share = |n: u64| {
                if c[0] == 0 {
                    0.0
                } else {
                    n as f64 * 100.0 / c[0] as f64
                }
            };
            println!();
            println!("--- KEEPER CHASE EXITS (GoalkeeperTakeBallState) ---");
            println!(
                "  {:.1}/match exited ON THE TICK HE ARRIVED, {:.1}/match after actually \
                 chasing",
                per(c[0]),
                per(c[7])
            );
            println!(
                "  of the first-tick ones: ball owned {:.0}%   his own delivery {:.0}%   \
                 outside his ground {:.0}%   a shot in flight {:.0}%   reached it {:.0}%   \
                 timed out {:.0}%",
                share(c[1]),
                share(c[2]),
                share(c[3]),
                share(c[4]),
                share(c[5]),
                share(c[6])
            );
        }
    }

    // ── KEEPER KNOCK-CHAIN CENSUS ──────────────────────────────────────
    // "Sometimes he kicks the ball around with his hands and runs after it
    // himself, and sometimes it even rolls out of bounds." Every mechanism
    // that puts a loose ball back into play off a keeper is already counted
    // somewhere; none of those counters can see the reported behaviour,
    // because it is not *a* contact, it is the SECOND one. This links them.
    {
        use core::knock_diag::KnockDiag;
        let k = KnockDiag::snapshot();
        if k[0] > 0 {
            let per = |v: u64| v as f64 / n_matches as f64;
            let share = |n: u64, of: u64| {
                if of == 0 {
                    0.0
                } else {
                    n as f64 * 100.0 / of as f64
                }
            };
            let long: u64 = (3..=7).map(|i| k[i]).sum();
            println!();
            println!("--- KEEPER KNOCK-CHAIN CENSUS (loose contacts off a keeper, linked) ---");
            println!(
                "  {:.2} chains/match over {:.2} contacts   CHAINS OF 2 OR MORE {:.3}/match \
                 ({:.0}% of chains)",
                per(k[0]),
                per(k[1]),
                per(long),
                share(long, k[0])
            );
            println!(
                "  length      1 {:.2}   2 {:.3}   3 {:.3}   4 {:.3}   5 {:.3}   6+ {:.3}",
                per(k[2]),
                per(k[3]),
                per(k[4]),
                per(k[5]),
                per(k[6]),
                per(k[7])
            );
            let ends = [
                "gloves",
                "his feet",
                "cleared",
                "to an opponent",
                "OUT OF PLAY",
                "lapsed",
            ];
            let row = |base: usize, of: u64| {
                ends.iter()
                    .enumerate()
                    .map(|(i, label)| format!("{label} {:.0}%", share(k[base + i], of)))
                    .collect::<Vec<_>>()
                    .join("  ")
            };
            println!("  ended:      {}", row(8, k[0]));
            if long > 0 {
                println!("  of the 2+:  {}", row(14, long));
            }
            let sources = ["body", "spill", "smother", "punch", "other"];
            println!(
                "  chains containing a: {}",
                sources
                    .iter()
                    .enumerate()
                    .map(|(i, label)| format!("{label} {:.2}", per(k[20 + i])))
                    .collect::<Vec<_>>()
                    .join("  ")
            );
            if long > 0 {
                println!(
                    "  …and in the 2+ ones:  {}",
                    sources
                        .iter()
                        .enumerate()
                        .map(|(i, label)| format!("{label} {:.3}", per(k[25 + i])))
                        .collect::<Vec<_>>()
                        .join("  ")
                );
            }
            println!(
                "  OUT OF PLAY OFF HIM WITH NOBODY NEAR HIM {:.3}/match (target ~0)   \
                 mean chain travel {:.1} m",
                per(k[30]),
                k[31] as f64 * 0.125 / k[0] as f64
            );
            println!(
                "  …and the out-of-play endings split {:.2}/match BEHIND (a corner or a \
                 goal kick, which is football) vs {:.3} over a TOUCHLINE (which is the report)",
                per(k[32]),
                per(k[33])
            );
            println!(
                "  mean chain duration {:.2} s",
                k[34] as f64 / 100.0 / k[0] as f64
            );
        }
    }

    // ── KEEPER TWO-CYCLE CENSUS ────────────────────────────────────────
    // WHICH two states are arguing. The scalar churn number in the motion
    // census has twice been the loudest thing in these diagnostics without
    // saying which pair produced it, and a two-cycle is always two specific
    // gates disagreeing about one condition.
    {
        use core::mid_run_diag::KeeperPairDiag;
        let p = KeeperPairDiag::snapshot();
        let n = KeeperPairDiag::STATES;
        let total: u64 = p[..n * n].iter().sum();
        if total > 0 {
            let keeper_matches = (n_matches * 2) as f64;
            let mut rows: Vec<(u64, u64, usize, usize)> = Vec::new();
            for from in 0..n {
                for to in 0..n {
                    let count = p[from * n + to];
                    if count > 0 {
                        rows.push((count, p[400 + from * n + to], from, to));
                    }
                }
            }
            rows.sort_by(|a, b| b.0.cmp(&a.0));
            let quick: u64 = p[400..400 + n * n].iter().sum();
            println!();
            println!(
                "--- KEEPER TWO-CYCLE CENSUS ({:.0} transitions/keeper/match, {:.0}% inside 300 ms) ---",
                total as f64 / keeper_matches,
                quick as f64 * 100.0 / total as f64
            );
            println!("  from -> to                                  per match   inside 300ms");
            for (count, fast, from, to) in rows.iter().take(12) {
                println!(
                    "  {:<20} -> {:<20} {:>9.0}   {:>10.0}%",
                    StateNames::of(100 + *from as u16),
                    StateNames::of(100 + *to as u16),
                    *count as f64 / keeper_matches,
                    *fast as f64 * 100.0 / (*count).max(1) as f64
                );
            }
        }
    }

    // ── KEEPER EXCURSION CENSUS ────────────────────────────────────────
    // How far from his goal he goes, on BOTH axes. Every other keeper
    // block — and every gate in the state machine that bounds him —
    // measures `|keeper.x - goal.x|`, the depth axis alone, so a keeper
    // level with his six-yard box and standing on the touchline reads
    // there as being on his goal line. "He chases the ball to the corner"
    // is invisible in all of them by construction.
    {
        use core::mid_run_diag::KeeperExcursionDiag;
        let x = KeeperExcursionDiag::snapshot();
        if x[0] > 0 {
            let ticks = x[0] as f64;
            println!();
            println!(
                "--- KEEPER EXCURSION CENSUS (every keeper tick; radial = from his goal centre) ---"
            );
            println!(
                "  mean radial {:.1} m (max {:.1})   mean lateral {:.1} m (max {:.1})",
                x[1] as f64 / 100.0 / ticks * 0.125,
                x[2] as f64 / 100.0 * 0.125,
                x[3] as f64 / 100.0 / ticks * 0.125,
                x[4] as f64 / 100.0 * 0.125
            );
            println!("  radial band          time%");
            for (i, label) in [
                "on his line <6 m",
                "his box 6-11 m",
                "edge of it 11-16.5 m",
                "16.5-25 m",
                "25-32 m",
                "beyond 32 m",
            ]
            .iter()
            .enumerate()
            {
                println!("  {:<20} {:>6.2}%", label, x[5 + i] as f64 * 100.0 / ticks);
            }
            println!(
                "  wider than his own area {:.2}%   outside it on either axis {:.2}%   \
                 CORNER COUNTRY (>25 m AND wide) {:.3}%",
                x[11] as f64 * 100.0 / ticks,
                x[12] as f64 * 100.0 / ticks,
                x[13] as f64 * 100.0 / ticks
            );
            let far: u64 = (14..=19).map(|i| x[i]).sum();
            if far > 0 {
                println!(
                    "  beyond 25 m ({:.0}/keeper/match): ComingOut {:.0}%  Standing {:.0}%  \
                     Walking {:.0}%  Returning {:.0}%  PreparingForSave {:.0}%  other {:.0}%",
                    far as f64 / (n_matches * 2) as f64,
                    x[14] as f64 * 100.0 / far as f64,
                    x[15] as f64 * 100.0 / far as f64,
                    x[16] as f64 * 100.0 / far as f64,
                    x[17] as f64 * 100.0 / far as f64,
                    x[18] as f64 * 100.0 / far as f64,
                    x[19] as f64 * 100.0 / far as f64
                );
                println!(
                    "  …and the ball then: loose {:.0}%   at an opponent's feet {:.0}%   \
                     itself wider than his area {:.0}%",
                    x[23] as f64 * 100.0 / far as f64,
                    x[24] as f64 * 100.0 / far as f64,
                    x[25] as f64 * 100.0 / far as f64
                );
                // A keeper fetching a ball that ran out for his own goal
                // kick is entitled to walk wherever it went. Read the two
                // rows above net of this one.
                println!(
                    "  …of which the ball was DEAD (his own restart to take) {:.0}%   \
                     and of the corner-country ticks, {:.0}%",
                    x[26] as f64 * 100.0 / far as f64,
                    if x[13] > 0 {
                        x[27] as f64 * 100.0 / x[13] as f64
                    } else {
                        0.0
                    }
                );
            }
            if x[21] > 0 {
                println!(
                    "  while ComingOut: mean radial {:.1} m, furthest {:.1} m",
                    x[20] as f64 / 100.0 / x[21] as f64 * 0.125,
                    x[22] as f64 / 100.0 * 0.125
                );
            }
        }
    }

    // ── GOALKEEPER ACTION CENSUS ───────────────────────────────────────
    // How often the keeper actually does each of the things a keeper does,
    // counted at the moment he commits rather than at the moment a stat is
    // ── KEEPER DISTRIBUTION CENSUS ─────────────────────────────────────
    // The state census can say he entered `Kicking`; only this can say
    // how far the ball then went. A keeper's release mix and the ground
    // each one covers is the whole of his distribution model, and both
    // were invisible — which is how `Kicking` came to mean "short pass to
    // a midfielder eighteen metres away" without anything noticing.
    //
    // Real-football reference: an open-play keeper possession is played
    // short roughly half the time; long kicks (punt + goal kick) carry
    // 50-65 m and make up most of the rest.
    {
        use core::mid_run_diag::KeeperReleaseDiag;
        let r = KeeperReleaseDiag::snapshot();
        let total: u64 = [0usize, 2, 4, 6, 8, 10].iter().map(|&i| r[i]).sum();
        if total > 0 {
            let keeper_matches = (n_matches * 2) as f64;
            println!();
            println!(
                "--- KEEPER DISTRIBUTION CENSUS ({:.1} releases per keeper per match) ---",
                total as f64 / keeper_matches
            );
            println!(
                "  {:<22} {:>10}  {:>7}  {:>14}  {:>12}",
                "release", "count", "share", "per keeper/match", "mean distance"
            );
            for (slot, label) in [
                (0usize, "short (out from back)"),
                (2, "throw"),
                (4, "punt from hands"),
                (6, "drop-kick"),
                (8, "long goal kick"),
                (10, "clearance"),
            ] {
                let n = r[slot];
                if n == 0 {
                    println!("  {:<22} {:>10}  ** NEVER **", label, 0);
                    continue;
                }
                println!(
                    "  {:<22} {:>10} {:>6.1}%  {:>14.2}  {:>9.1} m",
                    label,
                    n,
                    n as f64 * 100.0 / total as f64,
                    n as f64 / keeper_matches,
                    r[slot + 1] as f64 / n as f64 * 0.125
                );
            }
            let punts = r[4] + r[6];
            if punts > 0 {
                println!(
                    "  punts aimed at a target man {:.0}%   mean apex {:.1} m (real ~20)",
                    r[12] as f64 * 100.0 / (r[12] + r[13]).max(1) as f64,
                    r[14] as f64 / 100.0 / punts as f64
                );
            }
        }
    }

    // credited. Real-football reference in the header of each line.
    {
        use core::mid_run_diag::KeeperActionDiag;
        let a = KeeperActionDiag::snapshot();
        if a.iter().any(|&n| n > 0) {
            let per = |i: usize| a[i] as f64 / n_matches as f64 / 2.0;
            println!();
            println!("--- GOALKEEPER ACTION CENSUS (per keeper per match) ---");
            println!(
                "  dives {:.2} (real 2-4)   held it {:.2}   punches entered {:.2}  connected {:.2}",
                per(0),
                per(1),
                per(2),
                per(6)
            );
            println!(
                "  aerial claims started {:.2} (real 3-6)   left the ground {:.2}   caught {:.2}",
                per(3),
                per(4),
                per(5)
            );
            println!(
                // ×20: `in_state_time` counts AI ticks, and the AI runs on
                // every SECOND engine tick, so one is 20 ms not 10.
                "  mean dive duration {:.0} ms   mean claim range {:.1} m",
                if a[0] > 0 {
                    a[7] as f64 * 20.0 / a[0] as f64
                } else {
                    0.0
                },
                if a[3] > 0 {
                    a[8] as f64 * 0.125 / 100.0 / a[3] as f64
                } else {
                    0.0
                }
            );
            // The 1-v-1 and the anticipatory dive — the two things a
            // keeper does that the engine had no mechanism for at all
            // until Aug 2026. A smother that is never attempted and a
            // dive that only ever starts after the ball has stopped are
            // both invisible in `SAVE ACCOUNTING`, which is why they went
            // unnoticed for so long.
            println!(
                "  smothers at a carrier's feet {:.2}   gathered {:.2}  knocked away {:.2}  fouled {:.2}",
                per(11),
                per(12),
                per(13),
                per(14)
            );
            println!(
                "  dives launched IN FLIGHT {:.2} of {:.2} total ({:.0}%)",
                per(15),
                per(0),
                if a[0] > 0 {
                    a[15] as f64 * 100.0 / a[0] as f64
                } else {
                    0.0
                }
            );
            // The anchor `SaveModel::ORDINARY_PACE` is derived from — a
            // pace term centred anywhere else silently moves the whole
            // population save rate.
            if a[9] > 0 {
                println!(
                    "  mean speed of shots reaching the save roll {:.2} u/tick ({} shots)",
                    a[10] as f64 / 100.0 / a[9] as f64,
                    a[9]
                );
            }
        }
    }

    // ── KEEPER BODY ────────────────────────────────────────────────────
    // How often the ball hits the man rather than passing through him.
    // Every one of these used to be a ball travelling through a
    // goalkeeper's chest, which is what the report said it looked like.
    // Run with `OF_KEEPER_BODY=off` to get the same census with the
    // volume switched off — the count is then the number of pass-throughs
    // a match, and the save rate below is the "before".
    {
        use core::mid_run_diag::KeeperBodyDiag;
        let b = KeeperBodyDiag::snapshot();
        if b[0] > 0 {
            let per = b[0] as f64 / n_matches as f64;
            println!();
            println!("--- KEEPER BODY (contacts per match, both keepers) ---");
            println!(
                "  hit him {:.2}   off his feet {:.0}%   on his feet {:.0}%",
                per,
                b[1] as f64 * 100.0 / b[0] as f64,
                b[2] as f64 * 100.0 / b[0] as f64
            );
            println!(
                "  a shot on frame (a goal prevented) {:.2}   anything else {:.2}",
                b[3] as f64 / n_matches as f64,
                b[4] as f64 / n_matches as f64
            );
            println!(
                "  mean arrival {:.2} u/tick   mean {:.0} cm from his hips   mean height {:.2} m",
                b[5] as f64 / 100.0 / b[0] as f64,
                b[6] as f64 / b[0] as f64,
                b[7] as f64 / 100.0 / b[0] as f64
            );
            println!(
                "  mean {:.1} m out from his own line   inside the six-yard box {:.0}%",
                b[8] as f64 * 0.125 / b[0] as f64,
                b[9] as f64 * 100.0 / b[0] as f64
            );
        }
    }

    // ── KEEPER DIVE GATE ───────────────────────────────────────────────
    // `should_launch` is a conjunction and "he never dives" cannot say
    // which term killed it. Read the ratios between consecutive stages.
    {
        use core::mid_run_diag::KeeperDiveDiag;
        let d = KeeperDiveDiag::snapshot();
        if d[0] > 0 {
            let pct = |i: usize| {
                if d[0] > 0 {
                    d[i] as f64 * 100.0 / d[0] as f64
                } else {
                    0.0
                }
            };
            println!();
            println!("--- KEEPER DIVE GATE (ticks with a live shot at his goal) ---");
            println!(
                "  asked {}  →  inside the launch window {:.1}%  →  reacted {:.1}%  →  \
                 wedge {:.1}%  →  more than a step {:.1}%  →  not hopeless {:.1}%  →  \
                 LAUNCHED {:.1}%",
                d[0],
                pct(1),
                pct(2),
                pct(3),
                pct(4),
                pct(5),
                pct(6)
            );
            if d[3] > 0 {
                println!(
                    "  mean gap to cover {:.2} m   mean ground he could still walk {:.2} m",
                    d[8] as f64 / 10.0 / d[3] as f64 * 0.125,
                    d[9] as f64 / 10.0 / d[3] as f64 * 0.125
                );
            }
        }
    }

    // ── KEEPER BY SHOT RANGE ───────────────────────────────────────────
    // The aggregate "beyond his reach" row cannot tell a tap-in he had no
    // time for from a drive he never moved for. This splits it.
    {
        use core::mid_run_diag::KeeperRangeDiag;
        let r = KeeperRangeDiag::snapshot();
        if r.iter().any(|&n| n > 0) {
            println!();
            println!("--- KEEPER BY SHOT RANGE (on-frame shots that reached his plane) ---");
            println!(
                "  {:<12} {:>8} {:>10} {:>10} {:>9} {:>8} {:>9} {:>9}",
                "struck from", "arrived", "beyond", "miss", "reach", "saved", "diving", "flight"
            );
            let bands = ["< 11 m", "11-18 m", "18-28 m", "28 m +"];
            for (label, band) in bands.iter().zip(0..4usize) {
                let s = |i: usize| r[band * 24 + i];
                let n = s(0);
                if n == 0 {
                    continue;
                }
                let per = |v: u64| v as f64 * 100.0 / n as f64;
                println!(
                    "  {:<12} {:>8} {:>9.0}% {:>9.2}m {:>8.2}m {:>7.0}% {:>8.0}% {:>8.0}ms",
                    label,
                    n,
                    per(s(1)),
                    s(2) as f64 / 100.0 / n as f64 * 0.125,
                    s(3) as f64 / 100.0 / n as f64 * 0.125,
                    per(s(4)),
                    per(s(6)),
                    s(7) as f64 / 10.0 / n as f64 * 10.0,
                );
            }
            // …and why he stayed on his feet, per band. The ratio that
            // matters is `stepped to it` against `LAUNCHED`: a keeper who
            // refuses the dive because he has already walked to the ball
            // is a different animal from one who never saw it.
            println!(
                "  {:<12} {:>8} {:>10} {:>10} {:>9} {:>8} {:>9} {:>9}",
                "in-window",
                "ticks",
                "no react",
                "< a step",
                "hopeless",
                "stepped",
                "LAUNCHED",
                "gap/walk"
            );
            for (label, band) in bands.iter().zip(0..4usize) {
                let s = |i: usize| r[band * 24 + i];
                let n = s(8);
                if n == 0 {
                    continue;
                }
                let per = |v: u64| v as f64 * 100.0 / n as f64;
                println!(
                    "  {:<12} {:>8} {:>9.0}% {:>9.0}% {:>8.0}% {:>7.0}% {:>8.0}% {:>6.2}/{:.2}m",
                    label,
                    n,
                    per(s(9)),
                    per(s(10)),
                    per(s(11)),
                    per(s(12)),
                    per(s(13)),
                    s(14) as f64 / 10.0 / n as f64 * 0.125,
                    s(15) as f64 / 10.0 / n as f64 * 0.125,
                );
            }
            // …and the ball-over-his-head case on its own. A shot arriving
            // above his standing ceiling is one he has to leave his feet
            // for however straight at him it is; if this row is not
            // launching, the vertical term is not reaching the decision.
            for (label, band) in bands.iter().zip(0..4usize) {
                let s = |i: usize| r[band * 24 + i];
                let n = s(16);
                if n == 0 {
                    continue;
                }
                println!(
                    "  {:<12} {:>8} over his head, mean climb {:.1}u  →  LAUNCHED {:.0}%  \
                     still under a step {:.0}%",
                    label,
                    n,
                    s(19) as f64 / 10.0 / n as f64,
                    s(17) as f64 * 100.0 / n as f64,
                    s(18) as f64 * 100.0 / n as f64,
                );
            }
            // …and the two write-offs that happen ABOVE the launch window.
            // A shot the strike-time arc says is going over the bar is one
            // the keeper never considers at all.
            for (label, band) in bands.iter().zip(0..4usize) {
                let s = |i: usize| r[band * 24 + i];
                let n = s(20);
                if n == 0 {
                    continue;
                }
                println!(
                    "  {:<12} {:>8} live-shot ticks  →  written off OVER THE BAR {:.0}%  \
                     PAST THE POST {:.0}%   mean arrival height {:.2} m",
                    label,
                    n,
                    s(21) as f64 * 100.0 / n as f64,
                    s(22) as f64 * 100.0 / n as f64,
                    if s(8) > 0 {
                        s(23) as f64 / 100.0 / s(8) as f64
                    } else {
                        0.0
                    },
                );
            }
        }
    }

    // ── KEEPER BY HIS OWN QUALITY ──────────────────────────────────────
    // Does a better keeper GO FOR more of them, as well as stopping more?
    // Needs `SQUAD_SPREAD` set — without it every keeper in the run is
    // retargeted to the same mean and one band swallows the population.
    {
        use core::mid_run_diag::KeeperQualityDiag;
        let q = KeeperQualityDiag::snapshot();
        if q.iter().any(|&n| n > 0) {
            println!();
            println!(
                "--- KEEPER BY HIS OWN QUALITY (gk_shot_stopping; set SQUAD_SPREAD or one \
                 band holds everything) ---"
            );
            println!(
                "  {:<12} {:>8} {:>7} {:>9} {:>9} {:>8} {:>10}",
                "band", "arrived", "mean", "beyond", "diving", "saved", "dives/1k"
            );
            for (label, band) in ["< 0.40", "0.40-0.50", "0.50-0.60", "≥ 0.60"]
                .iter()
                .zip(0..4usize)
            {
                let s = |i: usize| q[band * 6 + i];
                let n = s(0);
                if n == 0 {
                    continue;
                }
                let per = |v: u64| v as f64 * 100.0 / n as f64;
                println!(
                    "  {:<12} {:>8} {:>7.3} {:>8.0}% {:>8.0}% {:>7.0}% {:>10.0}",
                    label,
                    n,
                    s(4) as f64 / 1000.0 / n as f64,
                    per(s(1)),
                    per(s(2)),
                    per(s(3)),
                    // In-flight dive launches per 1000 arrivals — the
                    // attempt rate, normalised so the bands are comparable
                    // whatever share of the population each holds.
                    s(5) as f64 * 1000.0 / n as f64,
                );
            }
        }
    }

    // ── SHOT ARRIVAL HEIGHT: PREDICTED vs REAL ─────────────────────────
    {
        use core::mid_run_diag::ShotHeightDiag;
        let h = ShotHeightDiag::snapshot();
        if h[0] > 0 {
            println!();
            println!(
                "--- SHOT ARRIVAL HEIGHT --- {} arrivals: keeper's projection {:.2} m vs the \
                 ball's own {:.2} m; predicted OVER the bar but arrived under it {:.1}%, \
                 the converse {:.1}%",
                h[0],
                h[1] as f64 / 100.0 / h[0] as f64,
                h[2] as f64 / 100.0 / h[0] as f64,
                h[3] as f64 * 100.0 / h[0] as f64,
                h[4] as f64 * 100.0 / h[0] as f64,
            );
            println!(
                "    …and of those, WROTE OFF above his own CROSSBAR_MARGIN while the ball came \
                 in UNDER the bar: {} ({:.1}% of arrivals) — a shot he decided not to move for",
                h[5],
                h[5] as f64 * 100.0 / h[0] as f64,
            );
        }
    }

    // ── KEEPER VOICE: DOES HE CALL FOR IT ──────────────────────────────
    // `KeeperBallClaim::is_favourite` had no keeper attribute in it at
    // all, so a commanding goalkeeper claimed his own six-yard box on the
    // same terms as one who says nothing.
    {
        use core::mid_run_diag::KeeperVoiceDiag;
        let v = KeeperVoiceDiag::snapshot();
        let total: u64 = (0..4).map(|b| v[b * 8]).sum();
        if total > 0 {
            println!();
            println!("--- KEEPER VOICE (loose ball in front of him — is it his?) ---");
            println!("  communication     asked     raw   composite   HIS   above head height");
            for (b, label) in [(0usize, "< 8"), (1, "8-10"), (2, "11-13"), (3, "14+")] {
                let n = v[b * 8];
                if n == 0 {
                    continue;
                }
                let air = v[b * 8 + 4];
                println!(
                    "  {:<12} {:>9}   {:>5.1}     {:>5.3}   {:>4.0}%   {:>4.0}% of {}",
                    label,
                    n,
                    v[b * 8 + 2] as f64 / 100.0 / n as f64,
                    v[b * 8 + 3] as f64 / 1000.0 / n as f64,
                    v[b * 8 + 1] as f64 * 100.0 / n as f64,
                    if air == 0 {
                        0.0
                    } else {
                        v[b * 8 + 5] as f64 * 100.0 / air as f64
                    },
                    air,
                );
            }
        }
        // …and whether it reaches the men in front of him.
        {
            use core::mid_run_diag::KeeperVoiceShapeDiag;
            let s = KeeperVoiceShapeDiag::snapshot();
            let n: u64 = (0..4).map(|b| s[b * 3]).sum();
            if n > 0 {
                println!(
                    "  defender anchor lag by the voice BEHIND him (lower is a tighter block):"
                );
                for (b, label) in [
                    (0usize, "quiet  <0.41"),
                    (1, "0.41-0.46"),
                    (2, "0.46-0.51"),
                    (3, "loud   0.51+"),
                ] {
                    let k = s[b * 4];
                    if k == 0 {
                        continue;
                    }
                    println!(
                        "    {:<14} {:>10} samples   voice {:>5.3}   lag {:>5.2} m   
                             off the line {:>5.2} m",
                        label,
                        k,
                        s[b * 4 + 2] as f64 / 1000.0 / k as f64,
                        s[b * 4 + 1] as f64 / 100.0 / k as f64 * 0.125,
                        s[b * 4 + 3] as f64 / 100.0 / k as f64 * 0.125,
                    );
                }
            }
        }
    }
    // ── KEEPER HANDLING: WHAT HE DOES WITH THE SAVE ────────────────────
    // Every other keeper block measures whether the save happens. This is
    // the only one that measures what he did with it, banded by the
    // attribute that is supposed to decide.
    {
        use core::mid_run_diag::KeeperHandlingDiag;
        let h = KeeperHandlingDiag::snapshot();
        let total: u64 = (0..4).map(|b| h[b * 8]).sum();
        if total > 0 {
            println!();
            println!("--- KEEPER HANDLING (what he does with the save, by his own Handling) ---");
            println!(
                "  handling      saves     raw   scaled    HELD    tipped   spilled   shot pace   diff"
            );
            for (b, label) in [(0usize, "< 8"), (1, "8-10"), (2, "11-13"), (3, "14+")] {
                let n = h[b * 8];
                if n == 0 {
                    continue;
                }
                let pct = |v: u64| v as f64 * 100.0 / n as f64;
                println!(
                    "  {:<10} {:>8}   {:>5.1}   {:>5.3}   {:>4.0}%     {:>4.0}%     {:>4.0}%    {:>5.2} u/t  {:>5.3}",
                    label,
                    n,
                    h[b * 8 + 4] as f64 / 100.0 / n as f64,
                    h[b * 8 + 6] as f64 / 1000.0 / n as f64,
                    pct(h[b * 8 + 1]),
                    pct(h[b * 8 + 2]),
                    pct(h[b * 8 + 3]),
                    h[b * 8 + 5] as f64 / 100.0 / n as f64,
                    h[b * 8 + 7] as f64 / 1000.0 / n as f64,
                );
            }
        }
    }
    // ── KEEPER COMMIT vs TRUTH ─────────────────────────────────────────
    // The one block that compares where the keeper thinks the shot is
    // going with where it is going. Everything else in the harness scores
    // him against his own read, so the error cancels and cannot be seen.
    {
        use core::mid_run_diag::KeeperCommitDiag;
        let c = KeeperCommitDiag::snapshot();
        let n: u64 = (0..4).map(|b| c[b * 6]).sum();
        if n > 0 || c[24] > 0 {
            let u = |v: u64, d: u64| {
                if d == 0 {
                    0.0
                } else {
                    v as f64 / 100.0 / d as f64 * 0.125
                }
            };
            println!();
            println!(
                "--- KEEPER COMMIT vs TRUTH (his read of the crossing point against the ball's) ---"
            );
            println!(
                "  at the DIVE LAUNCH        launches   read err   ball across him   he threw   WRONG WAY"
            );
            for (b, label) in [
                (0usize, "< 11 m"),
                (1, "11-18 m"),
                (2, "18-28 m"),
                (3, "28 m +"),
            ] {
                let k = c[b * 6];
                if k == 0 {
                    continue;
                }
                println!(
                    "  {:<10}          {:>8}    {:>6.2}m           {:>6.2}m    {:>6.2}m   {:>5.1}% of {}",
                    label,
                    k,
                    u(c[b * 6 + 1], k),
                    u(c[b * 6 + 2], k),
                    u(c[b * 6 + 3], k),
                    if c[b * 6 + 4] == 0 {
                        0.0
                    } else {
                        c[b * 6 + 5] as f64 * 100.0 / c[b * 6 + 4] as f64
                    },
                    c[b * 6 + 4],
                );
            }
            if c[24] > 0 {
                let a = c[24];
                println!(
                    "  at the ARRIVAL: {} on-frame arrivals, read off the true crossing by {:.2} m",
                    a,
                    u(c[25], a)
                );
                println!(
                    "    lateral error scored on his READ {:.2} m   on the TRUTH {:.2} m",
                    u(c[26], a),
                    u(c[27], a)
                );
                println!(
                    "    BEYOND HIS REACH on the read {:.1}%   on the truth {:.1}%   \
                     (rolled a save he could not reach {:.1}%, denied one he could {:.1}%)",
                    c[28] as f64 * 100.0 / a as f64,
                    c[29] as f64 * 100.0 / a as f64,
                    c[30] as f64 * 100.0 / a as f64,
                    c[31] as f64 * 100.0 / a as f64,
                );
                println!(
                    "    the ball's REAL gap across him at his own depth {:.2} m; \
                     within a metre of him {} ({:.1}% of arrivals), and of those the model \
                     called {} BEYOND HIS REACH ({:.1}%)",
                    u(c[37], a),
                    c[38],
                    c[38] as f64 * 100.0 / a as f64,
                    c[39],
                    if c[38] == 0 {
                        0.0
                    } else {
                        c[39] as f64 * 100.0 / c[38] as f64
                    },
                );
            }
            if c[36] > 0 {
                println!(
                    "  while the shot is in the air: {:.1}% of his steer ticks pull him AWAY from \
                     the ball ({} of {} ticks with it more than a metre across him); \
                     mean ball across him {:.2} m, mean pull {:.2} m",
                    if c[32] == 0 {
                        0.0
                    } else {
                        c[33] as f64 * 100.0 / c[32] as f64
                    },
                    c[33],
                    c[32],
                    u(c[34], c[36]),
                    u(c[35], c[36]),
                );
            }
        }
    }

    // ── DEFENDER SHOOTING SUPPLY ───────────────────────────────────────
    // Separates "the defender is blocked from shooting" from "he never
    // has the ball anywhere near the goal", which the shot count alone
    // cannot distinguish.
    {
        use core::mid_run_diag::DefenderShotDiag;
        let (onball, in_range, decisions) = DefenderShotDiag::snapshot();
        if onball > 0 {
            println!();
            println!(
                "--- DEFENDER SHOOTING SUPPLY --- {:.0} on-ball ticks/match, of which \
                 {:.1}% within 40m of goal; {:.1} shot decisions reached/match",
                onball as f64 / n_matches as f64,
                in_range as f64 * 100.0 / onball as f64,
                decisions as f64 / n_matches as f64
            );
        }
    }

    // ── PASS WEIGHT CENSUS ─────────────────────────────────────────────
    // How far a pass was meant to travel versus how far it actually did,
    // sampled at the first touch after it was struck. This is the only
    // measurement that answers "is the ball being struck too hard".
    {
        use core::mid_run_diag::PassWeightCensus;
        let bands = PassWeightCensus::snapshot();
        if bands.iter().any(|b| b.0 > 0) {
            println!();
            println!("--- PASS WEIGHT CENSUS (first touch while the pass was still live) ---");
            println!("  overshoot > 1.0 would mean the ball is being struck past its man");
            println!(
                "  {:<12} {:>9}  {:>9}  {:>9}  {:>10}  {:>12}",
                "band", "passes", "intended", "actual", "overshoot", "by-target"
            );
            for (label, (n, intended, actual, to_target)) in
                ["short ≤15m", "medium ≤30m", "long >30m"].iter().zip(bands)
            {
                if n == 0 {
                    continue;
                }
                println!(
                    "  {:<12} {:>9} {:>8.1}m  {:>8.1}m  {:>9.2}x  {:>11.0}%",
                    label,
                    n,
                    intended * 0.125,
                    actual * 0.125,
                    actual / intended.max(1.0),
                    to_target * 100.0
                );
            }
        }
    }

    // ── TOUCHLINE / OFFSIDE CENSUS ─────────────────────────────────────
    // The two restarts nothing measured. A throw-in leaves no statistic
    // and an offside only bumps a per-player counter nobody prints, so
    // "is it given to the right team" and "does the taker walk there"
    // were both unanswerable.
    {
        use core::mid_run_diag::RestartCensus;
        let r = RestartCensus::snapshot();
        let per = |n: u64| n as f64 / n_matches as f64;
        if r[0] > 0 {
            println!();
            println!(
                "--- TOUCHLINE CENSUS --- {:.1} throw-ins/match   (real ~40-50)",
                per(r[0])
            );
            println!(
                "  awarded on a DISPUTED last touch (toucher and owner on opposite sides): \
                 {} ({:.0}%)   must be ~0 — either answer is a coin flip",
                r[1],
                r[1] as f64 * 100.0 / r[0] as f64
            );
            println!(
                "  the ball straight back out within {} ticks: {:.1}/match ({:.0}%)",
                RestartCensus::PING_PONG_TICKS,
                per(r[2]),
                r[2] as f64 * 100.0 / r[0] as f64
            );
            println!(
                "  taker had to cover {:.1} m to the spot on average, {:.0}% of them more than \
                 a stride",
                r[3] as f64 / r[0] as f64 * 0.125,
                r[4] as f64 * 100.0 / r[0] as f64
            );
            // Denominator is slot 10 — every restart that WAITED, which is
            // throw-ins plus the offside free kick since `award_offside`
            // stopped teleporting the ball and the taker. Against slot 0
            // this row used to read over 100%.
            println!(
                "  of {:.1} awaited restarts/match ({:.1} of them offside free kicks) he WALKED \
                 to {:.0}% (teleported {:.1}/match), ball waiting {:.1} s each   →   \
                 {:.0} s/match with the ball dead on the spot",
                per(r[10]),
                per(r[6]),
                r[9] as f64 * 100.0 / r[10].max(1) as f64,
                per(r[7]),
                r[8] as f64 / r[10].max(1) as f64 / 100.0,
                r[8] as f64 / n_matches as f64 / 100.0
            );
        }
        if r[5] > 0 {
            println!(
                "  offside: {:.1} snapshots/match → {:.2} GIVEN   (real ~4-6 given)",
                per(r[5]),
                per(r[6])
            );
        }
        // The goal kick joined the same wait when "the ball instantly ends
        // up in the keeper's hands" was traced to it. The number that says
        // whether that was affordable is the TELEPORT rate: a keeper who
        // cannot reach the dead ball inside the patience bound gets it
        // placed under him, which is the behaviour this replaced.
        if r[11] > 0 {
            println!(
                "  goal kicks: {:.1}/match, keeper walks {:.1} m to the ball on average \
                 ({:.0}% of them more than a stride), never reached {:.2}/match ({:.1}%) with {:.1} m still to go",
                per(r[11]),
                r[12] as f64 / r[11] as f64 * 0.125,
                r[13] as f64 * 100.0 / r[11] as f64,
                per(r[14]),
                r[14] as f64 * 100.0 / r[11] as f64,
                r[15] as f64 / r[14].max(1) as f64 * 0.125,
            );
        }
        // What the player layer tried to do to a ball that was out of play.
        // Every one of these used to be applied: `secure_ball_for` snaps the
        // ball to the actor's feet, so each was a metre-plus jump off the
        // restart spot and a jump back on the next tick — the reported "the
        // ball bounces off the side of the field and ends up in the
        // goalkeeper's hands". See `core::DeadBall`; `OF_DEAD_BALL=off`
        // stops refusing them and this row measures the same pressure.
        if r[18] > 0 {
            println!(
                "  DEAD BALL: {:.1} s/match out of play, {:.1} ball-touching player events \
                 refused on it ({:.0}% from goalkeepers)   must be applied to NOTHING",
                r[18] as f64 / n_matches as f64 / 100.0,
                per(r[16]),
                r[17] as f64 * 100.0 / r[16].max(1) as f64,
            );
        }
        // The corner is the one restart NOT taken from where the ball
        // died, so its taker fetches the ball and then carries it to the
        // arc. The CARRY is the distance the ball used to teleport.
        if r[19] > 0 {
            let w = RestartCensus::corner_walk_snapshot();
            println!(
                "  corners: {:.1}/match, taker fetches from {:.1} m away and carries the ball \
                 {:.1} m to the arc; {:.0}% of them reach the ball on their own feet",
                per(r[19]),
                w[0] as f64 / r[19] as f64 * 0.125,
                w[1] as f64 / r[19] as f64 * 0.125,
                w[2] as f64 * 100.0 / r[19] as f64,
            );
            // A leg that timed out took the backstop teleport, which is
            // the thing the walk exists to remove. HOW FAR SHORT is what
            // says whether the bound is too tight or he never set off.
            println!(
                "    legs that timed out: {:.2}/match, a mean {:.1} m still to go",
                per(w[4]),
                w[5] as f64 / w[4].max(1) as f64 * 0.125,
            );
            // Then he stands over it and waits for the box. The ceiling
            // share is what says whether the wait is doing any work: if
            // most corners hit it, the runners are not arriving and the
            // delay is pure dead time.
            if w[6] > 0 {
                println!(
                    "    set-up wait: {:.2}/match, a mean {:.2} s on the arc waiting for {} \
                     attackers in the box ({:.0}% released by the CEILING rather than the box)",
                    per(w[6]),
                    w[7] as f64 / w[6] as f64 / 100.0,
                    core::r#match::engine::ball::ball::AwaitedRestart::CORNER_BOX_TARGET,
                    w[8] as f64 * 100.0 / w[6] as f64,
                );
            }
        }
        // ── THE RUN-OUT ────────────────────────────────────────────────
        // Reported as "the ball stops on the line behind the goal, but must
        // go beyond the goal". Every restart used to write the ball onto
        // its spot two units INSIDE the pitch on the tick it crossed the
        // line; it now keeps travelling until the hoardings stop it and the
        // taker fetches it from out there. See `core::RunOff`;
        // `OF_RUN_OUT=off` restores the snap.
        //
        // Two ways it can go wrong and neither shows in any row above. A
        // ball that never comes to rest blows the patience bound and
        // appears only as a teleport; one that stops on the line after all
        // appears as nothing whatsoever.
        {
            let o = RestartCensus::run_out_snapshot();
            if o[0] > 0 {
                println!(
                    "  RUN-OUT: {:.1}/match, ball ends {:.2} m outside the pitch in {:.2} s \
                     ({:.0}% stopped by the boards, {:.1}% still moving at the ceiling), \
                     then carried {:.2} m back to the spot",
                    per(o[0]),
                    o[1] as f64 / o[0] as f64 * 0.125,
                    o[2] as f64 / o[0] as f64 / 100.0,
                    o[3] as f64 * 100.0 / o[0] as f64,
                    o[4] as f64 * 100.0 / o[0] as f64,
                    o[5] as f64 / o[0] as f64 * 0.125,
                );
                // …and the ones nobody fetched. A ball over the metre-high
                // boards is in the crowd and one that stops behind the goal
                // is behind the netting, so both get a fresh ball on the
                // spot rather than a taker walking through a stand or a
                // net. Deliberate — see `Ball::replace_dead_ball` — but it
                // is a relocation, so it is counted where relocations are.
                println!(
                    "           replaced (nobody could fetch it): {:.1}/match into the crowd, \
                     {:.1}/match behind the goal — {:.0}% of all run-outs",
                    per(o[6]),
                    per(o[7]),
                    (o[6] + o[7]) as f64 * 100.0 / o[0] as f64,
                );
            }
        }
        // A taker who timed out in `TakeBall` was on his way; one who timed
        // out in `Standing` was never coming. Opposite fixes.
        let timeouts = RestartCensus::timeout_state_snapshot();
        if !timeouts.is_empty() {
            let total: u64 = timeouts.iter().map(|(_, n)| n).sum();
            let names: Vec<String> = timeouts
                .iter()
                .take(10)
                .map(|(id, n)| {
                    format!(
                        "{} {:.0}%",
                        StateNames::of(*id),
                        *n as f64 * 100.0 / total as f64
                    )
                })
                .collect();
            println!(
                "  TAKERS who timed out were in: {}   (of {total} timeouts overall)",
                names.join(", ")
            );
        }
    }

    // ── ENDLINE CENSUS ─────────────────────────────────────────────────
    {
        use core::mid_run_diag::EndlineCensus;
        let (corners, goal_kicks) = EndlineCensus::snapshot();
        let crossings = corners + goal_kicks;
        if crossings > 0 {
            println!();
            // Real reference, per MATCH: ~10.4 corners and ~16 goal
            // kicks, so ~26 endline crossings of which ~40% are corners.
            // (The "~21 + ~13 / 62% corners" this used to print came from
            // reading the per-match corner figure as a per-team one — see
            // the `corners per team` line in the aggregate block. Every
            // conclusion drawn from the old 62% is suspect: the engine's
            // corner SHARE has been about right all along, and it is the
            // total endline traffic that is short.)
            println!(
                "--- ENDLINE CENSUS --- {:.1} crossings/match: {:.1} corners ({:.0}%), \
                 {:.1} goal kicks   (real ~10.4 + ~16, so ~40% corners)",
                crossings as f64 / n_matches as f64,
                corners as f64 / n_matches as f64,
                corners as f64 * 100.0 / crossings as f64,
                goal_kicks as f64 / n_matches as f64
            );
            {
                use core::r#match::player::strategies::passing::CrossType;
                use core::mid_run_diag::CrossDiag;
                let by_type = CrossDiag::by_type();
                let struck: u64 = by_type.iter().sum();
                let lofted: u64 = [
                    CrossType::FloatedFarPost,
                    CrossType::WhippedNearPost,
                    CrossType::EarlyCross,
                ]
                .iter()
                .map(|t| by_type[t.diag_index()])
                .sum();
                println!(
                    "  crosses struck {:.1}/match (real ~30), of which LOFTED {:.1} ({:.0}%):",
                    struck as f64 / n_matches as f64,
                    lofted as f64 / n_matches as f64,
                    lofted as f64 * 100.0 / struck.max(1) as f64
                );
                let mut mix = String::new();
                for t in [
                    CrossType::FloatedFarPost,
                    CrossType::DrivenLowCross,
                    CrossType::Cutback,
                    CrossType::WhippedNearPost,
                    CrossType::EarlyCross,
                ] {
                    mix.push_str(&format!(
                        "  {} {:.1}",
                        t.label(),
                        by_type[t.diag_index()] as f64 / n_matches as f64
                    ));
                }
                println!("   {}", mix.trim());
                let r = CrossDiag::rejects();
                println!(
                    "  contest gate rejections (ball-ticks): above 2.9m {}, below 1.5m {}, \
                     still rising {}, >25m from goal {}",
                    r[0], r[1], r[2], r[3]
                );
                let d = CrossDiag::disarm_heights();
                println!(
                    "  lofted deliveries DISARMED before the contest, by height: \
                     on the deck {:.1}, low {:.1}, in band {:.1}, above band {:.1} (/match)",
                    d[0] as f64 / n_matches as f64,
                    d[1] as f64 / n_matches as f64,
                    d[2] as f64 / n_matches as f64,
                    d[3] as f64 / n_matches as f64
                );
                let (touched, died) = CrossDiag::lost_deliveries();
                println!(
                    "  lofted deliveries lost before the contest: touched first {:.1}/match, \
                     died armed {:.1}/match",
                    touched as f64 / n_matches as f64,
                    died as f64 / n_matches as f64
                );
                let (seen, fired, won, gk, headers) = CrossDiag::contest();
                println!(
                    "  contest funnel: seen {:.1} ball-ticks, FIRED {:.1}, GK claimed {:.1}, \
                     attacker won {:.1}, headers on goal {:.1}, headed clear {:.1}",
                    seen as f64 / n_matches as f64,
                    fired as f64 / n_matches as f64,
                    gk as f64 / n_matches as f64,
                    won as f64 / n_matches as f64,
                    headers as f64 / n_matches as f64,
                    fired.saturating_sub(gk).saturating_sub(won) as f64 / n_matches as f64
                );
            }
            println!("  corners conceded, by what the DEFENDER who put it behind was doing:");
            for (id, n) in EndlineCensus::corner_sources().iter().take(8) {
                if *n * 40 < corners {
                    continue; // below 2.5% — noise
                }
                println!(
                    "    {:<32} {:>5.1}/match",
                    StateNames::of(*id),
                    *n as f64 / n_matches as f64
                );
            }
            let from_shot = EndlineCensus::from_shot();
            println!(
                "  of those goal kicks, {:.1}/match ({:.0}%) were a MISSED SHOT, not a stray pass",
                from_shot as f64 / n_matches as f64,
                from_shot as f64 * 100.0 / goal_kicks.max(1) as f64
            );
            let (nonshot, speed, slow) = EndlineCensus::nonshot_speed();
            println!(
                "  the other {:.1}/match cross at {:.2} u/tick ({:.1} m/s); {:.0}% of them \
                 trickle out under 4.4 m/s",
                nonshot as f64 / n_matches as f64,
                speed,
                speed * 12.5,
                slow * 100.0
            );
            println!("  goal kicks by what the LAST TOUCHER was doing, and how far the");
            println!("  ball ran after that touch:");
            for (id, n, run) in EndlineCensus::run_snapshot().iter().take(10) {
                if *n * 50 < goal_kicks {
                    continue; // below 2% — noise
                }
                println!(
                    "    {:<32} {:>5.1}/match   ran {:>5.1}m after the touch",
                    StateNames::of(*id),
                    *n as f64 / n_matches as f64,
                    run * 0.125
                );
            }
        }
    }

    // ── FOUL SOURCE CENSUS ─────────────────────────────────────────────
    // Which state emitted each foul, and how many had the ball inside the
    // fouler's own box (penalty candidates). The aggregate foul/penalty
    // counts cannot say which model to tune.
    {
        use core::mid_run_diag::FoulCensus;
        let rows = FoulCensus::snapshot();
        let total: u64 = rows.iter().map(|r| r.1).sum();
        let total_box: u64 = rows.iter().map(|r| r.2).sum();
        if total > 0 {
            println!();
            println!("--- FOUL SOURCE CENSUS (contacts emitted, before the referee gate) ---");
            println!(
                "  {:<34} {:>8}  {:>7}  {:>9}  {:>7}",
                "emitting state", "fouls", "share", "ball-in-box", "in-box%"
            );
            for (id, n, in_box) in rows.iter() {
                if *n * 100 < total {
                    continue; // below 1% — noise
                }
                println!(
                    "  {:<34} {:>8} {:>6.1}%  {:>9} {:>6.1}%",
                    StateNames::of(*id),
                    n,
                    *n as f64 * 100.0 / total as f64,
                    in_box,
                    *in_box as f64 * 100.0 / *n as f64
                );
            }
            println!(
                "  TOTAL {} emitted, {} with the ball in our own box ({:.1}%)",
                total,
                total_box,
                total_box as f64 * 100.0 / total as f64
            );
        }
    }

    // ── CARD SOURCE CENSUS ─────────────────────────────────────────────
    // A red has THREE independent routes — direct off a violent foul,
    // direct off a reckless one, and a second yellow — and `red
    // cards/match` cannot tell them apart. Two rounds of tuning went into
    // the wrong one before this existed.
    {
        use core::mid_run_diag::CardDiag;
        let c = CardDiag::snapshot();
        if c[0] > 0 {
            let per = |v: u64| v as f64 / n_matches as f64;
            println!();
            println!(
                "--- CARD SOURCE CENSUS --- {:.1} fouls whistled/match: normal {:.1} \
                 reckless {:.2} violent {:.3}",
                per(c[0]),
                per(c[5]),
                per(c[6]),
                per(c[7])
            );
            println!(
                "  yellows {:.2}/match   REDS {:.3} = second-yellow {:.3} + direct-reckless \
                 {:.3} + direct-violent {:.3}   (real red ~0.15-0.20)",
                per(c[1]),
                per(c[2]) + per(c[3]) + per(c[4]),
                per(c[2]),
                per(c[3]),
                per(c[4])
            );
        }
    }

    // ── SPACING CENSUS ─────────────────────────────────────────────────
    // How much room the twenty-two give each other. The screenshot
    // complaint — "a bunch of players running around in large groups" —
    // is a claim about this and nothing else, and no other counter can
    // answer it: `nearest mate` is a per-player minimum, so a five-man
    // pile and a chain of pairs read identically. See `spacing_diag`.
    {
        use core::spacing_diag::{SpacingCensus, SpacingReport};
        let (all, final_third) = SpacingCensus::snapshot();
        if all.samples > 0 {
            println!();
            println!("--- SPACING CENSUS (sampled 4x/s while somebody is carrying the ball) ---");
            let row = |label: &str, r: &SpacingReport| {
                if r.samples == 0 {
                    return;
                }
                println!(
                    "  {label}  ({} samples)\n    \
                     biggest clump (bodies inside 3 m of each other) {:.2}   \
                     4+ on {:.0}% of ticks, 6+ on {:.0}%   {:.1} m from the ball\n    \
                     …made of  DEF {:.0}%  MID {:.0}%  FWD {:.0}%   \
                     ({:.2} with the ball, {:.2} without)\n    \
                     side in possession: nearest team-mate {:.1} m (real 15-20), \
                     under 5 m on {:.0}% of players\n    \
                     within 15 m of the ball: {:.2} of ours, {:.2} of theirs   \
                     (real ~2-3 / ~2-4)\n    \
                     FREE (no opponent inside 8 m): {:.2} of ours, {:.2} of them ahead of the ball \
                     (real 3-5 / 1-3)\n    \
                     attacking side spans {:.0} m wide x {:.0} m deep",
                    r.samples,
                    r.clump_size,
                    r.clump_ge4_share * 100.0,
                    r.clump_ge6_share * 100.0,
                    r.clump_ball_gap_m,
                    r.clump_line_share[1] * 100.0,
                    r.clump_line_share[2] * 100.0,
                    r.clump_line_share[3] * 100.0,
                    r.clump_attacking,
                    r.clump_defending,
                    r.mate_gap_m,
                    r.mate_under5_share * 100.0,
                    r.swarm_attacking,
                    r.swarm_defending,
                    r.free,
                    r.free_ahead,
                    r.att_width_m,
                    r.att_depth_m,
                );
            };
            row("ALL PLAY  ", &all);
            row("FINAL THIRD", &final_third);
        }
    }

    // ── TEAM SHAPE CENSUS ──────────────────────────────────────────────
    // Where the match time actually goes, state by state, and how far out
    // of the team's shape a player is while he is there. The `paths` mode
    // reports the block length; this says which states are responsible
    // for it, so off-ball work lands on the ticks that exist rather than
    // the ticks it feels like should.
    {
        use core::mid_run_diag::ShapeCensus;
        let rows = ShapeCensus::snapshot();
        let total: u64 = rows.iter().map(|r| r.1).sum();
        if total > 0 {
            println!();
            println!("--- TEAM SHAPE CENSUS (AI ticks by state, heaviest first) ---");
            println!("  anchor lag = distance from the place the team plan wants him (metres)");
            println!(
                "  {:<34} {:>7}  {:>7}  {:>8}  {:>8}  {:>7}",
                "state", "ticks", "share", "anchorlag", "axislag", "still"
            );
            let mut shown = 0;
            for (id, ticks, lag, axis, still) in rows.iter() {
                if *ticks * 400 < total {
                    continue; // below 0.25% — noise
                }
                println!(
                    "  {:<34} {:>7} {:>6.1}%  {:>7.1}m  {:>+7.1}m  {:>6.0}%",
                    StateNames::of(*id),
                    ticks,
                    *ticks as f64 * 100.0 / total as f64,
                    lag * 0.125,
                    axis * 0.125,
                    still * 100.0
                );
                shown += 1;
                if shown >= 40 {
                    break;
                }
            }
            // Per-role roll-up: the headline is what share of a role's
            // match is spent in states that hold shape vs states that
            // chase the ball.
            let role_of = |id: u16| -> usize { (id / 100) as usize };
            let mut role_ticks = [0u64; 5];
            let mut role_lag = [0f64; 5];
            let mut role_still = [0f64; 5];
            for (id, ticks, lag, _axis, still) in rows.iter() {
                let r = role_of(*id).min(4);
                role_ticks[r] += ticks;
                role_lag[r] += (*lag as f64) * (*ticks as f64);
                role_still[r] += (*still as f64) * (*ticks as f64);
            }
            let (anchor_span, actual_span, worst_lag) = ShapeCensus::span_snapshot();
            let axis = ShapeCensus::axis_lag_snapshot();
            println!();
            println!(
                "  mean lag ALONG the attacking axis (+ = further forward than the plan):  \
                 DEF {:+.1}m   MID {:+.1}m   FWD {:+.1}m",
                axis[1] * 0.125,
                axis[2] * 0.125,
                axis[3] * 0.125
            );
            println!();
            println!(
                "  block span along the goal axis:  planned {:.1}m   actual {:.1}m   \
                 worst single player {:.1}m out   (ALL PHASES — see the split below)",
                anchor_span * 0.125,
                actual_span * 0.125,
                worst_lag * 0.125
            );
            {
                let (def_anchor, def_actual, def_share) = ShapeCensus::span_defending_snapshot();
                println!(
                    "    while DEFENDING only ({:.0}% of refreshes):  planned {:.1}m   \
                     actual {:.1}m   (real defending block 35-45m; attacking 50-60m, so the \
                     all-phase line above answers neither on its own)",
                    def_share * 100.0,
                    def_anchor * 0.125,
                    def_actual * 0.125,
                );
            }
            println!();
            for (r, name) in [(1, "GK"), (2, "DEF"), (3, "MID"), (4, "FWD")] {
                if role_ticks[r] == 0 {
                    continue;
                }
                println!(
                    "  {:<5} mean anchor lag {:>5.1}m   still {:>4.0}%   ({} ticks)",
                    name,
                    role_lag[r] / role_ticks[r] as f64 * 0.125,
                    role_still[r] / role_ticks[r] as f64 * 100.0,
                    role_ticks[r]
                );
            }
        }
    }

    // ── MIDFIELDER ON-BALL DECISION CENSUS ─────────────────────────────
    // One row per exit of `MidfielderRunningState::process`, counted per
    // tick a midfielder had the ball at his feet. The aggregate stats can
    // only show that a pass happened; this shows WHICH of the eleven
    // pass-emitting branches produced it, and how much of the possession
    // never reaches the carry / dribble / shoot branches at all.
    {
        use core::mid_onball_diag as onball_diag;
        let (exits, ahead, skill_refused, allowed) = onball_diag::snapshot();
        let total: u64 = exits.iter().sum();
        println!();
        println!("--- MIDFIELDER ON-BALL DECISION CENSUS (ticks with the ball) ---");
        println!("  total on-ball ticks: {}", total);
        let mut rows: Vec<(usize, u64)> = exits.iter().copied().enumerate().collect();
        rows.sort_by(|a, b| b.1.cmp(&a.1));
        for (i, n) in rows {
            if n == 0 {
                continue;
            }
            println!(
                "  {:<22} {:>10}  {:>5.1}%",
                onball_diag::NAMES[i],
                n,
                n as f64 / total.max(1) as f64 * 100.0
            );
        }
        let ahead_total: u64 = ahead.iter().sum();
        println!(
            "  take-on gate reached {} ticks — opponents in cone: 0 {:.1}%  1 {:.1}%  2 {:.1}%  3+ {:.1}%",
            ahead_total,
            ahead[0] as f64 / ahead_total.max(1) as f64 * 100.0,
            ahead[1] as f64 / ahead_total.max(1) as f64 * 100.0,
            ahead[2] as f64 / ahead_total.max(1) as f64 * 100.0,
            ahead[3] as f64 / ahead_total.max(1) as f64 * 100.0,
        );
        println!(
            "  of the ticks with somebody to beat: allowed {}  refused on skill {}",
            allowed, skill_refused
        );
    }

    // ── OPEN-PLAY CROSSING CHAIN ──────────────────────────────────────
    // Delivery MIX first (has one branch of `pick_cross_type` swallowed
    // the model?), then the contest funnel: seen → fired → won → header.
    // A high `seen` with a low `fired` means the resolver's height /
    // box gates never match; a high `fired` with a low `won` means the
    // duel is too hard; a high `won` with no headers means the winner's
    // state machine isn't striking the planted ball.
    {
        use core::r#match::player::strategies::passing::CrossType;
        use core::mid_run_diag::CrossDiag;
        let by_type = CrossDiag::by_type();
        let total: u64 = by_type.iter().sum();
        println!("\n--- OPEN-PLAY CROSSING ---");
        if total == 0 {
            println!("  no crosses struck");
        } else {
            let mix = CrossType::ALL
                .iter()
                .enumerate()
                .map(|(i, ct)| {
                    format!(
                        "{} {} ({:.0}%)",
                        ct.label(),
                        by_type[i],
                        by_type[i] as f64 / total as f64 * 100.0
                    )
                })
                .collect::<Vec<_>>()
                .join("  ");
            println!("  delivery mix ({total} struck): {mix}");
        }
        use core::mid_run_diag::PlanDiag;
        let (refreshes, active, slots, slot_ticks) = PlanDiag::snapshot();
        if refreshes > 0 {
            println!(
                "  attack plan: active on {:.0}% of refreshes, {:.1} of 4 box slots filled when live, {slot_ticks} slot-driven movement ticks",
                active as f64 / refreshes as f64 * 100.0,
                if active > 0 {
                    slots as f64 / active as f64
                } else {
                    0.0
                }
            );
        }
        // What a forward given a patch of the box actually does with it.
        // `frozen` is the headline: the share of his box occupancy spent
        // at a literal zero velocity, i.e. standing in the penalty area.
        use core::mid_run_diag::BoxSlotDiag;
        let (
            slot_n,
            frozen,
            slot_speed,
            slot_gap,
            slot_marked,
            slot_opp,
            ball_progress,
            to_goal,
            camped,
            state_time,
        ) = BoxSlotDiag::snapshot();
        if slot_n > 0 {
            println!(
                "  box occupancy: {:.1}% of slot ticks FROZEN, mean speed {:.3} u/tick, {:.1}m from the slot, {:.0}% with a marker in range, nearest opponent {:.1}m ({slot_n} ticks)",
                frozen as f64 / slot_n as f64 * 100.0,
                slot_speed,
                slot_gap * 0.125,
                slot_marked as f64 / slot_n as f64 * 100.0,
                slot_opp * 0.125,
            );
            println!(
                "  …timing: ball at {:.2} of the way up the pitch, occupant {:.1}m from goal, {:.1}% of ticks CAMPED (in the box with the ball outside the final third), mean in-state {:.0} ticks",
                ball_progress,
                to_goal * 0.125,
                camped as f64 / slot_n as f64 * 100.0,
                state_time,
            );
        }
        use core::mid_run_diag::DefenceDiag;
        let (samples, depth_spread, max_gap, attackers, unmarked, nearest) =
            DefenceDiag::snapshot();
        if samples > 0 {
            println!("\n--- DEFENSIVE SHAPE (sampled while defending) ---");
            println!(
                "  back-line depth spread {:.0}u ({:.1}m)   widest lateral gap {:.0}u ({:.1}m)   (real stagger 25-65u / 3-8m)",
                depth_spread,
                depth_spread * 0.125,
                max_gap,
                max_gap * 0.125
            );
            println!(
                "  attackers in our third: {attackers}, nearest defender {:.0}u ({:.1}m) on average, {:.0}% with NOBODY within 3m",
                nearest,
                nearest * 0.125,
                unmarked as f64 / attackers.max(1) as f64 * 100.0
            );
            use core::mid_run_diag::EvasionDiag;
            let (calls, marked, tightness, edge) = EvasionDiag::snapshot();
            if calls > 0 {
                println!(
                    "  marker evasion: {:.0}% of attacker off-ball ticks had a marker, mean tightness {tightness:.2}, mean edge {edge:.2} ({marked} of {calls})",
                    marked as f64 / calls as f64 * 100.0
                );
            }
            {
                use core::mid_run_diag::{CLEAR_REASON_NAMES, ClearDiag};
                let by_reason = ClearDiag::snapshot();
                let total: u64 = by_reason.iter().sum();
                if total > 0 {
                    println!(
                        "  clearance reasons ({} total, {:.1}/team/match):",
                        total,
                        total as f32 / (2.0 * n_matches as f32)
                    );
                    let mut rows: Vec<(usize, u64)> =
                        by_reason.iter().copied().enumerate().collect();
                    rows.sort_by(|a, b| b.1.cmp(&a.1));
                    for (i, c) in rows.into_iter().filter(|(_, c)| *c > 0) {
                        println!(
                            "    {:<26} {:>8}  {:>5.1}%",
                            CLEAR_REASON_NAMES[i],
                            c,
                            c as f32 / total as f32 * 100.0
                        );
                    }
                }
            }
            let (lag_n, lag_mean, lag_max, dwell, lag_x, lag_y) = DefenceDiag::shape_lag();
            if lag_n > 0 {
                println!(
                    "  shape lag: {lag_n} samples — defender sits {lag_mean:.0}u ({:.1}m) from his shape target, worst {lag_max:.0}u ({:.1}m); depth {lag_x:.0}u width {lag_y:.0}u; mean dwell in state {dwell:.0} ticks",
                    lag_mean * 0.125,
                    lag_max * 0.125,
                );
            }
            let (duels, duel_gap, duels_lost) = DefenceDiag::duel_snapshot();
            if duels > 0 {
                println!(
                    "  marking duels: marker sits {duel_gap:.0}u ({:.1}m) from his man, attacker got away (>4m) on {:.0}% of samples",
                    duel_gap * 0.125,
                    duels_lost * 100.0
                );
                // Does the SHAPE LEASH explain the gap, or is the marker
                // just not arriving at a target that was already right?
                let (mn, want, leashed, pull, mmid, want_mid, leashed_mid) =
                    DefenceDiag::mark_leash();
                if mn > 0 {
                    println!(
                        "    leash cost: marker WANTS to stand {:.2}m from his man, the shape \
                         leaves him {:.2}m (pull {:.2}m, {} samples)",
                        want * 0.125,
                        leashed * 0.125,
                        pull * 0.125,
                        mn
                    );
                    if mmid > 0 {
                        println!(
                            "                …marking a MIDFIELDER: wants {:.2}m, gets {:.2}m \
                             ({} samples)",
                            want_mid * 0.125,
                            leashed_mid * 0.125,
                            mmid
                        );
                    }
                }
                let (on_task, on_task_gap) = DefenceDiag::duel_on_task();
                println!(
                    "    ...and only {:.0}% of those markers were in a state that ACTS on the \
                     duty (Marking/Guarding); those sit {:.1}m off",
                    on_task * 100.0,
                    on_task_gap * 0.125,
                );
                let by_state = DefenceDiag::duel_by_state();
                let bs_total = by_state.iter().sum::<u64>().max(1);
                println!(
                    "    what the OTHERS were doing: playing-the-ball {:.0}%  press/cover {:.0}%  \
                     running/recovering {:.0}%  idle {:.0}%   (the last two are duties nobody acts on)",
                    by_state[1] as f64 / bs_total as f64 * 100.0,
                    by_state[2] as f64 / bs_total as f64 * 100.0,
                    by_state[3] as f64 / bs_total as f64 * 100.0,
                    by_state[4] as f64 / bs_total as f64 * 100.0,
                );
                let by_line = DefenceDiag::duel_by_line();
                let total = by_line.iter().map(|(n, _)| *n).sum::<u64>().max(1);
                println!(
                    "    who is being marked: DEF {:.0}% ({:.1}m)  MID {:.0}% ({:.1}m)  FWD {:.0}% ({:.1}m)",
                    by_line[0].0 as f64 / total as f64 * 100.0,
                    by_line[0].1 * 0.125,
                    by_line[1].0 as f64 / total as f64 * 100.0,
                    by_line[1].1 * 0.125,
                    by_line[2].0 as f64 / total as f64 * 100.0,
                    by_line[2].1 * 0.125,
                );
            }
            let (refresh, active, individual) = DefenceDiag::plan_snapshot();
            if refresh > 0 {
                println!(
                    "  duty plan: live on {:.0}% of refreshes, {individual:.1} of the unit on an individual duty (press/cover/mark) when live",
                    active as f64 / refresh as f64 * 100.0
                );
                // WHY the rest have nothing to do. "Too few threats
                // ranked" and "too few markers in range" need opposite
                // fixes and the aggregate above cannot tell them apart.
                let s = DefenceDiag::plan_shape();
                println!(
                    "    per refresh: unit {:.1}, threats ranked {:.1} (skipped as too deep {:.1}, \
                     nobody in reach {:.1})  →  press {:.2}  cover {:.2}  marks {:.2}  \
                     holding a zone {:.1}",
                    s[0],
                    s[1],
                    s[2],
                    s[3],
                    s[4],
                    s[5],
                    s[6],
                    (s[0] - s[4] - s[5] - s[6]).max(0.0)
                );
            }
        }
        {
            use core::mid_run_diag::DuelDiag;
            let (gates, box_gates) = DuelDiag::gates();
            let (box_ticks, bodies, surrounded, contested, cooldown) = DuelDiag::box_picture();
            let total: u64 = gates.iter().sum();
            if total > 0 {
                let pct = |v: u64, d: u64| {
                    if d == 0 {
                        0.0
                    } else {
                        v as f64 / d as f64 * 100.0
                    }
                };
                println!(
                    "  CHALLENGE GATE CENSUS ({total} defender-ticks inside commit range of a carrier): \
                     challenging {:.0}%  tackle cooldown {:.0}%  not the nominated engager {:.0}%  \
                     permitted, declined {:.0}%",
                    pct(gates[0], total),
                    pct(gates[1], total),
                    pct(gates[2], total),
                    pct(gates[3], total),
                );
                let box_total: u64 = box_gates.iter().sum();
                println!(
                    "    …with the ball in our own BOX ({box_total}): challenging {:.0}%  \
                     cooldown {:.0}%  not nominated {:.0}%  declined {:.0}%",
                    pct(box_gates[0], box_total),
                    pct(box_gates[1], box_total),
                    pct(box_gates[2], box_total),
                    pct(box_gates[3], box_total),
                );
                println!(
                    "    carrier in our box: {:.1} ticks/match, {bodies:.2} defenders within 3 m, \
                     {:.0}% of those ticks had ANY body that close and only {:.0}% a live challenge",
                    box_ticks as f64 / n_matches as f64,
                    surrounded * 100.0,
                    contested * 100.0,
                );
                println!(
                    "    tackle cooldown blocks {:.0}% of all defending outfielder-ticks \
                     (see `MatchPlayer::start_tackle_cooldown`; one AI tick is 20 ms)",
                    cooldown * 100.0,
                );
                let (reach, decisions, mean_p, box_decisions, box_p) = DuelDiag::commitment();
                let reach_total: u64 = reach.iter().sum::<u64>().max(1);
                println!(
                    "    where the CHALLENGER is standing: inside contact (1.2m) {:.0}%  \
                     out to 2m {:.0}%  out to 3m {:.0}%  beyond {:.0}%   \
                     (band 1 is the block tackle; band 2 is `RecoveryChallenge`, and only for a defender the man has gone PAST — beyond that nobody rolls anything)",
                    pct(reach[0], reach_total),
                    pct(reach[1], reach_total),
                    pct(reach[2], reach_total),
                    pct(reach[3], reach_total),
                );
                println!(
                    "    commitment: {:.1} decisions/match at mean p={mean_p:.3}; \
                     in our own box {:.1}/match at p={box_p:.3}",
                    decisions as f64 / n_matches as f64,
                    box_decisions as f64 / n_matches as f64,
                );
            }
            // "They run parallel to the movement and don't try to take it."
            // The gate census above cannot see this; see `CLOSE_SAMPLES`.
            let ((n, rate, align, gap, par, gain), (dn, drate, dalign, dgap, dpar, dgain)) =
                DuelDiag::closing();
            if n > 0 {
                println!(
                    "  CLOSING CENSUS ({n} samples of the NEAREST defender to a moving carrier)\n    \
                     gap {:.2} m, closing at {rate:+.4} u/tick, heading alignment {align:+.2}  \
                     →  running PARALLEL {:.0}%, gaining on him {:.0}%",
                    gap / 8.0,
                    par * 100.0,
                    gain * 100.0,
                );
                println!(
                    "    …with the carrier in OUR OWN THIRD ({dn}): gap {:.2} m, closing {drate:+.4}, \
                     alignment {dalign:+.2}  →  PARALLEL {:.0}%, gaining {:.0}%",
                    dgap / 8.0,
                    dpar * 100.0,
                    dgain * 100.0,
                );
                let rows = DuelDiag::closing_by_state();
                let labels: Vec<String> = rows
                    .iter()
                    .map(|(l, c, p)| {
                        format!(
                            "{l} {:.0}% of samples/{:.0}% parallel",
                            *c as f64 / n as f64 * 100.0,
                            p * 100.0
                        )
                    })
                    .collect();
                println!("    what he was doing: {}", labels.join("  ·  "));
            }
            // What the beaten defender did about it — see
            // `mid_run_diag::RECOV_DECISIONS`. A challenge he could not
            // make before this existed at all.
            {
                use core::mid_run_diag::RecoveryDiag;
                let (rd, rp, ra, rwon, rfoul, rgap, rlead) = RecoveryDiag::totals();
                if rd > 0 {
                    println!(
                        "  RECOVERY CHALLENGE ({rd} decisions at mean p={rp:.3}) — {:.2} attempts/match\n    \
                         reaching {:.2} m with the man {:.2} m past him  →  won it {:.0}%, fouled {:.0}%, missed {:.0}%",
                        ra as f64 / n_matches as f64,
                        rgap / 8.0,
                        -rlead / 8.0,
                        rwon * 100.0,
                        rfoul * 100.0,
                        (1.0 - rwon - rfoul).max(0.0) * 100.0,
                    );
                }
            }
            // "A pass into the box where there are only defenders — they
            // don't go for it, they run at their own goal." Neither
            // census above can see that population; see
            // `mid_run_diag::BOXBALL_SAMPLES`.
            {
                use core::mid_run_diag::BoxBallDiag;
                let (bn, btb, btg, bgap, bgoal, batb, bours, boursg) = BoxBallDiag::picture();
                if bn > 0 {
                    println!(
                        "  BOX-DELIVERY CENSUS ({bn} ticks with a LOOSE ball in the defending side's own box)\n    \
                         nearest defender {:.2} m off the drop — heading cos: at the BALL {btb:+.2}, at his OWN GOAL {btg:+.2}\n    \
                         →  running GOALWARD rather than at the ball {:.0}%, genuinely attacking it {:.0}%",
                        bgap / 8.0,
                        bgoal * 100.0,
                        batb * 100.0,
                    );
                    println!(
                        "    …and on the {:.0}% of those where OUR man was nearest the drop (no attacker closer): goalward {:.0}%",
                        bours * 100.0,
                        boursg * 100.0,
                    );
                    let rows: Vec<String> = BoxBallDiag::by_state()
                        .iter()
                        .map(|(l, c, g)| {
                            format!(
                                "{l} {:.0}%/{:.0}% goalward",
                                *c as f64 / bn as f64 * 100.0,
                                g * 100.0
                            )
                        })
                        .collect();
                    println!("    what he was doing: {}", rows.join("  ·  "));
                }
            }
            // "Defenders with TakeBall don't intercept — they run parallel
            // with the ball." A different population from the closing
            // census above: that one only samples while somebody OWNS the
            // ball, and `TakeBall` exists only while nobody does. See
            // `mid_run_diag::CHASE_SAMPLES`.
            {
                use core::mid_run_diag::ChaseDiag;
                let (cn, crate_, clead, calign, cgap, cspeed, cpar, cahead, closing_, cgain) =
                    ChaseDiag::totals();
                if cn > 0 {
                    println!(
                        "  LOOSE-BALL CHASE CENSUS ({cn} samples of a player in TakeBall, ball loose and moving)\n    \
                         gap {:.2} m, ball {cspeed:.3} u/tick, closing at {crate_:+.4} u/tick, heading alignment {calign:+.2}",
                        cgap / 8.0,
                    );
                    println!(
                        "    aim LEAD {clead:+.3}  (0 = pointed at where the ball IS — a stern chase; \
                         >0 = cutting in front of it)  →  aimed AHEAD on {:.0}% of samples",
                        cahead * 100.0,
                    );
                    println!(
                        "    →  running PARALLEL {:.0}%, gaining {:.0}%, gap not shrinking at all {:.0}%",
                        cpar * 100.0,
                        cgain * 100.0,
                        closing_ * 100.0,
                    );
                    let lines: Vec<String> = ChaseDiag::by_line()
                        .iter()
                        .map(|(l, c, p, a)| {
                            format!(
                                "{l} {:.0}% of samples/{:.0}% parallel/{:.0}% ahead",
                                *c as f64 / cn as f64 * 100.0,
                                p * 100.0,
                                a * 100.0
                            )
                        })
                        .collect();
                    println!("    who is chasing: {}", lines.join("  ·  "));
                    let speeds: Vec<String> = ChaseDiag::by_speed()
                        .iter()
                        .map(|(l, c, p)| {
                            format!(
                                "{l} u/tick {:.0}%/{:.0}% parallel",
                                *c as f64 / cn as f64 * 100.0,
                                p * 100.0
                            )
                        })
                        .collect();
                    println!(
                        "    by ball speed (a sprint is ~0.45-0.63 u/tick): {}",
                        speeds.join("  ·  ")
                    );
                    let gaps: Vec<String> = ChaseDiag::by_gap()
                        .iter()
                        .map(|(l, c, p, a)| {
                            format!(
                                "{l} {:.0}%/{:.0}% parallel/{:.0}% ahead",
                                *c as f64 / cn as f64 * 100.0,
                                p * 100.0,
                                a * 100.0
                            )
                        })
                        .collect();
                    println!("    by gap: {}", gaps.join("  ·  "));
                    println!("      (the first band is inside CONTROL_DISTANCE — a man travelling");
                    println!("       with the ball there is COLLECTING it, not failing to close)");
                }
            }
        }
        let (seen, fired, won, gk, header) = CrossDiag::contest();
        println!(
            "  aerial contest: seen={seen}  fired={fired}  attacker-won={won}  keeper-claimed={gk}  headers on goal={header}"
        );
        if fired > 0 {
            println!(
                "  per contest: attacker wins {:.0}%, keeper claims {:.0}%   (real: cross completion ~22-25%)",
                won as f64 / fired as f64 * 100.0,
                gk as f64 / fired as f64 * 100.0
            );
        }
    }

    // Player state-transition graph — the union of every distinct
    // `from -> to` edge (tagged by source) observed across the batch.
    // Dumped as Graphviz DOT and checked against the structural
    // invariants: every non-entry state reachable, every non-terminal
    // state has an exit. This is the population-scale transition graph,
    // so it exercises the audit on real play rather than synthetic edges.
    {
        let edges = core::r#match::TransitionGraph::edges();
        let dot = core::r#match::TransitionGraph::render_dot(&edges);
        let dot_path = "player_state_transitions.dot";
        let written = std::fs::write(dot_path, &dot).is_ok();
        println!();
        println!("--- STATE TRANSITION GRAPH ---");
        if written {
            println!("  {} distinct edges → {}", edges.len(), dot_path);
        } else {
            println!("  {} distinct edges (DOT write failed)", edges.len());
        }

        // Edge-source breakdown (handler vs the out-of-band overrides).
        let mut by_source: std::collections::BTreeMap<&str, usize> =
            std::collections::BTreeMap::new();
        for e in &edges {
            *by_source.entry(e.source.as_tag()).or_insert(0) += 1;
        }
        let src_summary: Vec<String> = by_source.iter().map(|(k, v)| format!("{k}={v}")).collect();
        println!("  by source: {}", src_summary.join("  "));

        // Structural invariants over the OBSERVED graph. Entry = the four
        // kickoff defaults + reserved (Injured); terminal = reserved.
        let universe = core::r#match::player::state::PlayerState::all();
        let mut entry = core::r#match::player::state::PlayerState::entry_states().to_vec();
        entry.extend(core::r#match::player::state::PlayerState::reserved_states());
        // Terminal = reserved, plus every state somebody was still in
        // when a whistle went: a player who has not left a state gives it
        // no outbound edge, which reads as a dead end. See
        // `TransitionGraph::note_final_states`.
        let mut terminal = core::r#match::player::state::PlayerState::reserved_states().to_vec();
        terminal.extend(core::r#match::TransitionGraph::final_states());
        let violations =
            core::r#match::TransitionGraph::audit(&edges, &universe, &entry, &terminal);

        // Only flag states actually exercised this run — an unreached
        // state is "not observed", not a structural dead-end.
        let observed: std::collections::HashSet<u16> = edges
            .iter()
            .flat_map(|e| [e.from.compact_id(), e.to.compact_id()])
            .collect();
        let real: Vec<_> = violations
            .into_iter()
            .filter(|v| match v {
                core::r#match::GraphInvariantViolation::Unreachable(id)
                | core::r#match::GraphInvariantViolation::DeadEnd(id) => observed.contains(id),
            })
            .collect();
        println!("  states observed: {}/{}", observed.len(), universe.len());
        // NAME the ones that never came up. This run is the only place
        // in the project with a sample big enough to make "never entered"
        // mean anything — the unit-test version of the audit sees two
        // matches and cannot tell a rare state from a dead one — so
        // printing a bare count throws away the whole finding. A state
        // that stays on this line across a few hundred matches is either
        // unreachable or reachable only through a path nothing takes.
        let unobserved: Vec<String> = universe
            .iter()
            .filter(|s| !observed.contains(&s.compact_id()))
            .map(|s| s.to_string())
            .collect();
        if !unobserved.is_empty() {
            println!("  never entered  : {}", unobserved.join(", "));
        }
        if real.is_empty() {
            println!("  invariants: OK (no observed unreachable / dead-end states)");
        } else {
            println!(
                "  invariants: {} violation(s) among observed states:",
                real.len()
            );
            for v in &real {
                println!("    {v:?}");
            }
        }
    }

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

    // `has_clear_shot()` (index 6) is NOT in the chain. It was removed from
    // the forward gate deliberately — lane quality is priced continuously by
    // `clarity_mult` inside the helper instead — but it was still printed as
    // a cumulative stage, so the row after it showed a **negative drop** of
    // -114.7% and the table read as "more shots come out of the gate than
    // went in". It is a real measurement of how many of these shots had a
    // clear lane, which is worth knowing; it is just not a filter. Printed
    // below the chain as its own line.
    let chain_order = [0usize, 1, 2, 4, 5, 7, 8];
    let chain_labels = [
        "has_ball_in_range (dist <= 90)",
        "can_shoot (not on cooldown)",
        "has_settled (ownership >= 30)",
        "!defer_to_teammate",
        "dist <= max_shot_distance",
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
    // Informational observations, not part of the chain.
    let poss_share = s[3] as f64 / base as f64 * 100.0;
    println!(
        "  [info]   {:>5.1}% of in-range ticks had prefer_possession=false",
        poss_share
    );
    // Not a gate — see the note on `chain_order`. Shown against the shots
    // that actually FIRED, which is the comparison worth having: how many
    // of the shots we took had a clear lane.
    let fired = s[8].max(1);
    println!(
        "  [info]   {} of in-range ticks had has_clear_shot() — {:.0}% of the {} shots FIRED \
         (not a gate on this path; lane quality is priced continuously in the helper)",
        s[6],
        s[6] as f64 / fired as f64 * 100.0,
        s[8],
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
    {
        use core::save_accounting_stats as s;
        use std::sync::atomic::Ordering::Relaxed;
        let staged = s::PENDING_STAGED.load(Relaxed);
        let no_shooter = s::PENDING_NO_SHOOTER.load(Relaxed);
        let delivered = s::PENDING_DELIVERED.load(Relaxed);
        let no_player = s::PENDING_LOST_NO_PLAYER.load(Relaxed);
        let same_team = s::PENDING_LOST_SAME_TEAM.load(Relaxed);
        let vanished = staged.saturating_sub(delivered + no_player + same_team);
        println!(
            "  physics-save credit: staged {}, DELIVERED {} ({:.1}%)   lost: \
             no-shooter-to-pair {}, id not on field {}, same team {}, \
             VANISHED BEFORE DELIVERY {}",
            staged,
            delivered,
            if staged > 0 {
                delivered as f64 / staged as f64 * 100.0
            } else {
                0.0
            },
            no_shooter,
            no_player,
            same_team,
            vanished,
        );
    }
}

/// Runtime per-player trace of the two failure modes you can only see by
/// watching, never by counting events: a player who twitches on the spot,
/// and a player whose state flips back and forth every tick.
///
/// Unlike `paths`, this does NOT read the recorded 30 ms replay track —
/// that track is deduped (samples under 0.3 u are dropped), so the very
/// jitter we're hunting is partly filtered out of it. Instead the engine
/// samples EVERY simulation tick as it runs (`MatchPlayer::trace_motion`)
/// and rolls the result up per player.
///
/// Reported per player:
///
///   * **twitch%** — share of one-second windows where the player covered
///     ≥1.5 m of ground but finished <0.30 m from where they started.
///     That is the flicker, quantified: motion without displacement.
///   * **rev/s** — velocity direction reversals per second. A purposeful
///     run has ~0; a player fought over by two steering targets has many.
///   * **flips/min** — state transitions per minute, and **pong%**, the
///     share of them that bounce straight back to the state just left.
///   * **inst%** — share of transitions that left a state after ≤1 AI
///     tick, i.e. the state was entered and abandoned immediately.
/// Play `matches` matches and print the ball's flight around every contact
/// with the woodwork.
///
/// Sequential on purpose. The trace store is process-global and a capture is
/// a SEQUENCE of ticks; run two matches on two threads and the two balls
/// interleave into one unreadable table.
fn run_woodwork(matches: usize, level: u8) {
    // `net` also opens a capture on every goal. The frame is struck under
    // once a match, which is far too rare a trigger to measure the netting
    // with, and what the ball does inside the goal is most of what a viewer
    // sees after a shot off the woodwork goes in.
    unsafe { std::env::set_var("OF_FRAME_TRACE", "net") };
    core::frame_trace::FrameTrace::reset();

    for m in 0..matches {
        MatchRuntime::set_events_mode(true);
        let (home, _) = make_squad_viewer(1, HOME_TEAM_NAME, level, 0);
        let (away, _) = make_squad_viewer(2, AWAY_TEAM_NAME, level, 11);
        let _ = FootballEngine::<840, 545>::play(home, away, true, false, false);
        eprintln!("  woodwork: match {}/{} played", m + 1, matches);
    }

    let (hits, captures) = core::frame_trace::FrameTrace::report();
    let s = core::frame_trace::FrameTrace::summary();
    println!();
    println!("=== WOODWORK TRACE ({matches} matches, level {level}) ===");
    println!(
        "  {hits} frame contacts, {:.2} per match; {} captured",
        hits as f64 / matches.max(1) as f64,
        captures.len()
    );
    println!(
        "  a '*' row travelled further than its own velocity explains — somebody relocated the ball"
    );
    println!();
    println!("  --- anomalies over the captured windows ---");
    println!("  mesh jumps (netting PULLED the ball) : {}", s.mesh_jumps);
    println!("  loose jumps (open play, unclaimed)   : {}", s.loose_jumps);
    println!(
        "  worst unexplained jump               : {} cm",
        s.worst_jump_cm
    );
    println!(
        "  ground snaps (z collapsed, no fall)  : {}",
        s.ground_snaps
    );
    println!(
        "  worst unexplained one-tick drop      : {} cm",
        s.worst_drop_cm
    );
    println!(
        "  windows that ended with the ball still in the goal: {}",
        s.rested_in_net
    );
    for capture in &captures {
        println!();
        println!("{capture}");
    }
}

fn run_gather(matches: usize, level: u8) {
    // `gather,miss` arms BOTH triggers: the gather is the moment the ball
    // stops obeying its velocity, and the miss is the moment the report
    // starts. Together they cover the whole passage either way round.
    unsafe { std::env::set_var("OF_FRAME_TRACE", "gather,miss") };
    core::frame_trace::FrameTrace::reset();

    for m in 0..matches {
        MatchRuntime::set_events_mode(true);
        let (home, _) = make_squad_viewer(1, HOME_TEAM_NAME, level, 0);
        let (away, _) = make_squad_viewer(2, AWAY_TEAM_NAME, level, 11);
        let _ = FootballEngine::<840, 545>::play(home, away, true, false, false);
        eprintln!("  gather: match {}/{} played", m + 1, matches);
    }

    let (_, captures) = core::frame_trace::FrameTrace::report();
    let s = core::frame_trace::FrameTrace::summary();
    println!();
    println!("=== KEEPER GATHER TRACE ({matches} matches, level {level}) ===");
    println!("  {} captures", captures.len());
    println!(
        "  a '*' row travelled further than its own velocity explains — somebody relocated the ball"
    );
    println!(
        "  gk_gap is the nearest keeper's XY distance to the ball in game units (1u = 12.5 cm),"
    );
    println!("  gk_h his own height in metres: >0 means both feet are off the ground.");
    println!();
    println!("  loose jumps (open play, unclaimed)   : {}", s.loose_jumps);
    println!(
        "  worst unexplained jump               : {} cm",
        s.worst_jump_cm
    );
    for capture in &captures {
        println!();
        println!("{capture}");
    }
}

fn run_trace(matches: usize, level: u8) {
    use core::motion_diag;
    use std::collections::HashMap;

    motion_diag::reset();
    core::r#match::TransitionGraph::reset();

    let mut names: HashMap<u32, String> = HashMap::new();
    for m in 0..matches {
        MatchRuntime::set_events_mode(true);
        let (home, hj) = make_squad_viewer(1, HOME_TEAM_NAME, level, 0);
        let (away, aj) = make_squad_viewer(2, AWAY_TEAM_NAME, level, 11);
        for p in hj.iter().chain(aj.iter()) {
            names.insert(p.id, format!("{} {}", p.position, p.last_name));
        }
        let _ = FootballEngine::<840, 545>::play(home, away, true, false, false);
        eprintln!("  trace: match {}/{} played", m + 1, matches);
    }

    let snap = motion_diag::snapshot();
    let secs_per_window = motion_diag::WINDOW_TICKS as f64 / 100.0;

    println!();
    println!(
        "=== RUNTIME PLAYER TRACE ({} match(es), level {}) ===",
        matches, level
    );
    println!(
        "  window = {} ticks ({:.0} s);  twitch = path >= {:.1} m AND net < {:.2} m",
        motion_diag::WINDOW_TICKS,
        secs_per_window,
        motion_diag::TWITCH_PATH_M,
        motion_diag::TWITCH_NET_M
    );

    // ── per-player motion + churn ──────────────────────────────────────
    let mut rows: Vec<(u32, motion_diag::PlayerMotion)> =
        snap.players.iter().map(|(k, v)| (*k, v.clone())).collect();
    rows.sort_by(|a, b| {
        let ta = a.1.twitch_windows as f64 / a.1.windows.max(1) as f64;
        let tb = b.1.twitch_windows as f64 / b.1.windows.max(1) as f64;
        tb.partial_cmp(&ta).unwrap()
    });

    println!();
    println!(
        "  {:<5} {:<10} {:>6} {:>8} {:>8} {:>7} {:>8} {:>7} {:>9} {:>6} {:>6}  {}",
        "id",
        "player",
        "wins",
        "twitch%",
        "still%",
        "rev/s",
        "rev-in-st",
        "m/s",
        "flips/min",
        "pong%",
        "inst%",
        "worst twitch window",
    );
    for (id, p) in rows.iter().take(24) {
        let wins = p.windows.max(1) as f64;
        let mins = p.windows as f64 * secs_per_window / 60.0;
        let worst = p
            .worst
            .map(|(t, path, net, st)| {
                format!(
                    "{:>2}:{:02} path {:.1}m net {:.2}m  {}",
                    t / 60_000,
                    (t % 60_000) / 1000,
                    path,
                    net,
                    st
                )
            })
            .unwrap_or_else(|| "-".to_string());
        println!(
            "  {:<5} {:<10} {:>6} {:>7.1}% {:>7.1}% {:>7.2} {:>7.0}% {:>7.2} {:>9.1} {:>5.1}% {:>5.1}%  {}",
            id,
            names.get(id).map(|s| s.as_str()).unwrap_or("?"),
            p.windows,
            p.twitch_windows as f64 / wins * 100.0,
            p.still_ticks as f64 / p.ticks.max(1) as f64 * 100.0,
            p.reversals as f64 / (p.windows as f64 * secs_per_window).max(1.0),
            p.reversals_in_state as f64 / p.reversals.max(1) as f64 * 100.0,
            p.path_u as f64 * motion_diag::M_PER_UNIT as f64
                / (p.windows as f64 * secs_per_window).max(1.0),
            p.transitions as f64 / mins.max(0.001),
            p.ping_pongs as f64 / p.transitions.max(1) as f64 * 100.0,
            p.instant_exits as f64 / p.transitions.max(1) as f64 * 100.0,
            worst,
        );
    }

    // Squad-wide aggregate — the single number to watch across a fix.
    let tot_win: u64 = snap.players.values().map(|p| p.windows).sum();
    let tot_twitch: u64 = snap.players.values().map(|p| p.twitch_windows).sum();
    let tot_rev: u64 = snap.players.values().map(|p| p.reversals).sum();
    let tot_rev_in: u64 = snap.players.values().map(|p| p.reversals_in_state).sum();
    let tot_tr: u64 = snap.players.values().map(|p| p.transitions).sum();
    let tot_pong: u64 = snap.players.values().map(|p| p.ping_pongs).sum();
    let tot_self: u64 = snap.players.values().map(|p| p.self_transitions).sum();
    let tot_inst: u64 = snap.players.values().map(|p| p.instant_exits).sum();
    println!();
    println!(
        "  ALL: twitch {:.1}%  rev/s {:.2} (in-state {:.0}%)  flips/min {:.1}  pong {:.1}%  self {:.1}%  inst {:.1}%",
        tot_twitch as f64 / tot_win.max(1) as f64 * 100.0,
        tot_rev as f64 / (tot_win as f64 * secs_per_window).max(1.0),
        tot_rev_in as f64 / tot_rev.max(1) as f64 * 100.0,
        tot_tr as f64 / (tot_win as f64 * secs_per_window / 60.0).max(0.001),
        tot_pong as f64 / tot_tr.max(1) as f64 * 100.0,
        tot_self as f64 / tot_tr.max(1) as f64 * 100.0,
        tot_inst as f64 / tot_tr.max(1) as f64 * 100.0,
    );

    // ── which state's own steering reverses under the player ───────────
    // A high rate here means that state's `velocity()` flips direction
    // between consecutive ticks with the player never leaving it — a
    // steering bug, localised to one function.
    {
        use std::sync::atomic::Ordering;
        let mut rows: Vec<(f64, f64, u64, u64, String)> = Vec::new();
        for st in core::r#match::player::state::PlayerState::all() {
            let slot = st.compact_id() as usize;
            let ticks = motion_diag::TICKS_BY_STATE[slot].load(Ordering::Relaxed);
            let revs = motion_diag::REV_BY_STATE[slot].load(Ordering::Relaxed);
            let fast = motion_diag::REV_FAST_BY_STATE[slot].load(Ordering::Relaxed);
            if ticks < 1000 {
                continue;
            }
            // Reversals per second of occupancy (100 ticks = 1 s).
            let rate = revs as f64 / ticks as f64 * 100.0;
            let fast_rate = fast as f64 / ticks as f64 * 100.0;
            rows.push((fast_rate, rate, revs, ticks, st.to_string()));
        }
        // Sorted by the FAST rate — that is the one that reads as a twitch
        // on screen. A high total with a near-zero fast rate is a player
        // settling onto a target, not flicker.
        rows.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap());
        println!();
        println!("--- IN-STATE VELOCITY REVERSALS (state's own steering flips direction) ---");
        println!(
            "  {:<34} {:>9} {:>9} {:>12} {:>12}",
            "state", "FAST/s", "rev/s", "reversals", "ticks held"
        );
        for (fast_rate, rate, revs, ticks, name) in rows.iter().take(12) {
            println!(
                "  {:<34} {:>9.2} {:>9.2} {:>12} {:>12}",
                name, fast_rate, rate, revs, ticks
            );
        }
        // Control: the ball's OWN direction stability. Chase states aim at
        // a point derived from the ball, so a chaser cannot be steadier
        // than what he is chasing.
        let ball_rev = motion_diag::BALL_REVERSALS.load(Ordering::Relaxed);
        let ball_ticks = motion_diag::BALL_TICKS.load(Ordering::Relaxed);
        println!(
            "  {:<34} {:>9.2} {:>9.2} {:>12} {:>12}   <- the ball itself",
            "(ball direction changes)",
            0.0,
            ball_rev as f64 / ball_ticks.max(1) as f64 * 100.0,
            ball_rev,
            ball_ticks,
        );
    }

    // ── raw velocity dumps around captured reversals ───────────────────
    // Counting reversals says WHICH state; only the vectors say WHY.
    // Each row is one sampled tick: where the player was, how fast and
    // which way he was going, and where the ball was relative to him.
    {
        let eps = motion_diag::episodes();
        let states = core::r#match::player::state::PlayerState::all();
        // Only dump the worst few states — otherwise this buries the report.
        // Ranked by the COUNT of fast reversals, not the rate: a state
        // occupied for a handful of ticks can top a rate table without
        // mattering, and the dumps are for chasing what actually happens
        // most.
        let mut ranked: Vec<(u64, u16)> = eps
            .keys()
            .map(|id| {
                use std::sync::atomic::Ordering;
                let fast = motion_diag::REV_FAST_BY_STATE[*id as usize].load(Ordering::Relaxed);
                (fast, *id)
            })
            .collect();
        ranked.sort_by(|a, b| b.0.cmp(&a.0));

        println!();
        println!("--- REVERSAL DUMPS (the ticks around a fast in-state reversal) ---");
        for (_, id) in ranked.iter().take(6) {
            let name = states
                .iter()
                .find(|s| s.compact_id() == *id)
                .map(|s| s.to_string())
                .unwrap_or_default();
            let Some(list) = eps.get(id) else { continue };
            println!();
            println!("  === {} ===", name);
            for ep in list.iter().take(3) {
                println!(
                    "   t={}:{:02}  player {}   ball ({:.1},{:.1},{:.2}) vel ({:.3},{:.3},{:.3})",
                    ep.t_ms / 60_000,
                    (ep.t_ms % 60_000) / 1000,
                    ep.player_id,
                    ep.ball_pos.x,
                    ep.ball_pos.y,
                    ep.ball_pos.z,
                    ep.ball_vel.x,
                    ep.ball_vel.y,
                    ep.ball_vel.z,
                );
                println!(
                    "     {:>4} {:>17} {:>17} {:>7} {:>8} {:>9}",
                    "tick", "position", "velocity", "speed", "dist_ball", "cos(v,ball)"
                );
                for (i, (p, v)) in ep.samples.iter().enumerate() {
                    let to_ball = ep.ball_pos - p;
                    let d = to_ball.magnitude();
                    let sp = v.magnitude();
                    // Is the player moving TOWARD the ball this tick?
                    let cos = if d > 0.001 && sp > 0.001 {
                        (v.x * to_ball.x + v.y * to_ball.y) / (d * sp)
                    } else {
                        0.0
                    };
                    println!(
                        "     {:>4} ({:>7.1},{:>7.1}) ({:>7.3},{:>7.3}) {:>7.3} {:>8.1} {:>9.2}",
                        i, p.x, p.y, v.x, v.y, sp, d, cos
                    );
                }
            }
        }
    }

    // ── states that are entered and abandoned immediately ──────────────
    let mut dwell: Vec<_> = snap.dwell.values().cloned().collect();
    dwell.sort_by_key(|(d, _)| std::cmp::Reverse(d.le1));
    println!();
    println!("--- SHORTEST-DWELL STATES (AI ticks held before leaving) ---");
    println!(
        "  {:<34} {:>8} {:>7} {:>8} {:>8} {:>7}",
        "state", "exits", "mean", "<=1 tick", "<=3 tick", "max"
    );
    for (d, st) in dwell.iter().take(14) {
        println!(
            "  {:<34} {:>8} {:>7.1} {:>7.1}% {:>7.1}% {:>7}",
            st.to_string(),
            d.exits,
            d.dwell_sum as f64 / d.exits.max(1) as f64,
            d.le1 as f64 / d.exits.max(1) as f64 * 100.0,
            d.le3 as f64 / d.exits.max(1) as f64 * 100.0,
            d.max,
        );
    }

    // ── the loops themselves ───────────────────────────────────────────
    let mut pong: Vec<_> = snap
        .ping_pong
        .iter()
        .map(|((_, _, src), (n, a, b))| (*n, *a, *b, *src))
        .collect();
    pong.sort_by_key(|(n, _, _, _)| std::cmp::Reverse(*n));
    println!();
    println!("--- STATE LOOPS (A -> B -> A, by return-leg source) ---");
    println!("  {:>9}  {:<14}  {}", "count", "source", "loop");
    for (n, a, b, src) in pong.iter().take(18) {
        println!(
            "  {:>9}  {:<14}  {}  <->  {}",
            n,
            src.as_tag(),
            a.to_string(),
            b.to_string()
        );
    }

    // ── self-transitions: the timer-reset trap ─────────────────────────
    let mut selfs: Vec<_> = snap
        .self_edges
        .iter()
        .map(|((_, src), (n, st))| (*n, *st, *src))
        .collect();
    selfs.sort_by_key(|(n, _, _)| std::cmp::Reverse(*n));
    println!();
    println!("--- SELF-TRANSITIONS (A -> A: resets in_state_time, so A's timeouts never fire) ---");
    println!("  {:>9}  {:<14}  {}", "count", "source", "state");
    for (n, st, src) in selfs.iter().take(18) {
        println!("  {:>9}  {:<14}  {}", n, src.as_tag(), st.to_string());
    }
    if selfs.is_empty() {
        println!("  none");
    }
}

/// Trace where players actually GO, as opposed to what they decide.
///
/// Every other diagnostic in this harness samples events — a shot, a pass,
/// a tackle. None of them can see a player standing still in the six-yard
/// box for a minute, drifting sideways for no reason, or shadowing a
/// team-mate two metres away all match. Those are path properties, and
/// they are what "the match does not look like football" usually means.
///
/// Reads the recorded position track (30 ms cadence, the same data the
/// replay viewer renders) and reports, per line:
///
///   * **covered** — kilometres per match. Real: GK ~5, DEF ~10, MID ~11,
///     FWD ~10. The single best sanity check on movement as a whole.
///   * **to goal** — mean distance from the opposition goal, and the share
///     of the match spent inside 6 m and 12 m of it. Camping check.
///   * **mate / opp** — mean distance to the nearest team-mate and the
///     nearest opponent. Spacing and whether anyone is ever in space.
///   * **straight** — net displacement over path length in 3 s windows.
///     1.0 is a purposeful run, near 0 is jitter on the spot.
///   * **still** — share of samples under 0.5 m/s.
/// Where the forwards go, and whether they ever get anywhere worth
/// shooting from.
///
/// The shot model answers "does he hit it when he is there". This answers
/// the question underneath it — **is he ever there at all** — which no
/// other mode reports: `paths` aggregates by line and hides the
/// individual, and `stats` only counts shots that happened.
///
/// One match, the first `minutes` of it, sampled off the recorded replay
/// track. Per forward: how his time divides by distance from the goal he
/// is attacking, how much of it is spent in the box, how much of THAT is
/// while the ball is in the final third (i.e. arriving with play, not
/// standing there while it is up the other end), and a coarse map of the
/// pitch showing where he spent it.
fn run_forward_paths(minutes: u64, level: u8) {
    use std::collections::HashMap;

    const SAMPLE_MS: u64 = 30;
    const M_PER_UNIT: f64 = 0.125;
    // Bands in metres from the goal he is attacking.
    const BANDS: [f64; 6] = [6.0, 11.0, 16.5, 22.0, 30.0, 45.0];
    const BAND_LABELS: [&str; 7] = ["<6", "6-11", "11-16", "16-22", "22-30", "30-45", "45+"];
    // Map resolution. The pitch is 840 x 545 units (105 x 68 m).
    const COLS: usize = 60;
    const ROWS: usize = 17;

    let window_ms = minutes * 60 * 1000;

    println!(
        "Forward path trace: first {} min of one match, both squads level {}",
        minutes, level
    );

    MatchRuntime::set_events_mode(true);
    let (home, hj) = make_squad_viewer(1, HOME_TEAM_NAME, level, 0);
    let (away, aj) = make_squad_viewer(2, AWAY_TEAM_NAME, level, 11);
    let mut names: HashMap<u32, String> = HashMap::new();
    for pl in hj.iter().chain(aj.iter()) {
        names.insert(pl.id, format!("{} {}", pl.position, pl.last_name));
    }
    let result = FootballEngine::<840, 545>::play(home, away, true, false, false);
    let data = &result.position_data;
    let ids = data.get_player_ids();
    let last = data.max_timestamp().min(window_ms);

    // The two forward slots of the starting XI. `POSITIONS_442` puts
    // them at indices 9 and 10, so their ids are base+9 and base+10.
    //
    // NOT `id % 100 >= 9`, which is what `paths` uses to bucket by line:
    // the bench is generated after the XI, so ids ending 11 and up came
    // back as forwards too — and they are parked off-pitch, which showed
    // up as four "forwards" covering 0.00 km and standing still 100% of
    // the time. Substitutes are therefore out of scope here; over a
    // five-minute window nobody has come on yet.
    let is_forward = |id: u32| matches!(id % 100, 9 | 10);
    let forwards: Vec<u32> = ids.iter().copied().filter(|id| is_forward(*id)).collect();
    if forwards.is_empty() {
        println!("  no forwards on the team sheet");
        return;
    }

    #[derive(Default, Clone)]
    struct Fwd {
        samples: u64,
        bands: [u64; 7],
        in_box: u64,
        in_box_live: u64,
        path_units: f64,
        still: u64,
        nearest_opp: f64,
        grid: Vec<u64>,
        prev: Option<(f32, f32)>,
    }

    let mut fwd: HashMap<u32, Fwd> = forwards
        .iter()
        .map(|id| {
            (
                *id,
                Fwd {
                    grid: vec![0; COLS * ROWS],
                    ..Default::default()
                },
            )
        })
        .collect();

    let mut t = 0_u64;
    while t <= last {
        let mut snap: Vec<(u32, (f32, f32))> = Vec::with_capacity(24);
        for id in &ids {
            if let Some(pos) = data.get_player_position_at(*id, t) {
                snap.push((*id, (pos.x, pos.y)));
            }
        }
        if snap.len() < 4 {
            t += SAMPLE_MS;
            continue;
        }
        let ball = data.get_ball_position_at(t);

        // Which end each side attacks, read from where its keeper stands
        // rather than assumed — sides swap at half time.
        let keeper_x = |team_home: bool| -> Option<f32> {
            snap.iter()
                .find(|(id, _)| (*id < 200) == team_home && *id % 100 == 0)
                .map(|(_, pos)| pos.0)
        };
        let home_attacks_right = keeper_x(true).map(|x| x < 420.0).unwrap_or(true);
        let attacks_right = |id: u32| {
            if id < 200 {
                home_attacks_right
            } else {
                !home_attacks_right
            }
        };
        let goal_for = |id: u32| -> (f32, f32) {
            if attacks_right(id) {
                (840.0, 272.5)
            } else {
                (0.0, 272.5)
            }
        };

        for (id, pos) in &snap {
            if !is_forward(*id) {
                continue;
            }
            let Some(entry) = fwd.get_mut(id) else {
                continue;
            };
            let goal = goal_for(*id);
            let dx = (goal.0 - pos.0) as f64;
            let dy = (goal.1 - pos.1) as f64;
            let to_goal_m = (dx * dx + dy * dy).sqrt() * M_PER_UNIT;

            entry.samples += 1;
            let band = BANDS.iter().position(|edge| to_goal_m < *edge).unwrap_or(6);
            entry.bands[band] += 1;
            if to_goal_m < 16.5 {
                entry.in_box += 1;
                // …and the ball is in the third he is attacking, so this
                // is arriving with the play rather than loitering while
                // it is at the other end.
                if let Some(b) = ball {
                    let ball_in_third = if attacks_right(*id) {
                        b.x > 560.0
                    } else {
                        b.x < 280.0
                    };
                    if ball_in_third {
                        entry.in_box_live += 1;
                    }
                }
            }

            if let Some(prev) = entry.prev {
                let step =
                    (((pos.0 - prev.0) as f64).powi(2) + ((pos.1 - prev.1) as f64).powi(2)).sqrt();
                entry.path_units += step;
                // 0.5 m/s over a 30 ms sample is 0.015 m.
                if step * M_PER_UNIT < 0.015 {
                    entry.still += 1;
                }
            }
            entry.prev = Some(*pos);

            // Nearest opponent, for whether he is ever free.
            let mut nearest = f64::MAX;
            for (other, opos) in &snap {
                if (*other < 200) == (*id < 200) || *other % 100 == 0 {
                    continue;
                }
                let d =
                    (((opos.0 - pos.0) as f64).powi(2) + ((opos.1 - pos.1) as f64).powi(2)).sqrt();
                nearest = nearest.min(d);
            }
            if nearest < f64::MAX {
                entry.nearest_opp += nearest * M_PER_UNIT;
            }

            // Map cell, always drawn attacking to the RIGHT so the two
            // teams' maps read the same way round.
            let x = if attacks_right(*id) {
                pos.0
            } else {
                840.0 - pos.0
            };
            let y = if attacks_right(*id) {
                pos.1
            } else {
                545.0 - pos.1
            };
            let col = ((x / 840.0 * COLS as f32) as usize).min(COLS - 1);
            let row = ((y / 545.0 * ROWS as f32) as usize).min(ROWS - 1);
            entry.grid[row * COLS + col] += 1;
        }

        t += SAMPLE_MS;
    }

    // ── per-forward table ──────────────────────────────────────────────
    println!();
    println!(
        "  {:<16} {:>7} {:>7}  {}",
        "forward",
        "km",
        "still%",
        BAND_LABELS
            .iter()
            .map(|l| format!("{:>6}", l))
            .collect::<Vec<_>>()
            .join("")
    );
    let mut order: Vec<u32> = forwards.clone();
    order.sort();
    for id in &order {
        let Some(e) = fwd.get(id) else { continue };
        let n = e.samples.max(1) as f64;
        println!(
            "  {:<16} {:>7.2} {:>6.0}%  {}",
            names.get(id).map(|s| s.as_str()).unwrap_or("?"),
            e.path_units * M_PER_UNIT / 1000.0,
            e.still as f64 / n * 100.0,
            e.bands
                .iter()
                .map(|b| format!("{:>5.1}%", *b as f64 / n * 100.0))
                .collect::<Vec<_>>()
                .join(" ")
        );
    }

    println!();
    println!(
        "  {:<16} {:>10} {:>12} {:>14}",
        "forward", "in box %", "…with play", "nearest opp"
    );
    for id in &order {
        let Some(e) = fwd.get(id) else { continue };
        let n = e.samples.max(1) as f64;
        println!(
            "  {:<16} {:>9.1}% {:>11.1}% {:>12.1} m",
            names.get(id).map(|s| s.as_str()).unwrap_or("?"),
            e.in_box as f64 / n * 100.0,
            e.in_box_live as f64 / n * 100.0,
            e.nearest_opp / n,
        );
    }

    // ── maps ───────────────────────────────────────────────────────────
    // Everyone drawn attacking to the right. Density is logarithmic: the
    // point is where he goes at all, not the exact dwell.
    println!();
    println!("  Occupancy, all forwards drawn attacking RIGHT (goal at the right edge).");
    println!("  '.' rare  ':' some  '+' often  '#' most; box edge marked |");
    let box_col = ((840.0 - 132.0) / 840.0 * COLS as f32) as usize;
    for id in &order {
        let Some(e) = fwd.get(id) else { continue };
        let peak = e.grid.iter().copied().max().unwrap_or(1).max(1) as f64;
        println!();
        println!("  {}", names.get(id).map(|s| s.as_str()).unwrap_or("?"));
        for row in 0..ROWS {
            let mut line = String::with_capacity(COLS + 4);
            line.push_str("    ");
            for col in 0..COLS {
                let v = e.grid[row * COLS + col] as f64 / peak;
                let ch = if v <= 0.0 {
                    if col == box_col { '|' } else { ' ' }
                } else if v < 0.10 {
                    '.'
                } else if v < 0.30 {
                    ':'
                } else if v < 0.60 {
                    '+'
                } else {
                    '#'
                };
                line.push(ch);
            }
            println!("{}", line);
        }
    }
}

fn run_paths(matches: usize, level: u8) {
    use std::collections::HashMap;

    const SAMPLE_MS: u64 = 30;
    const WINDOW_MS: u64 = 3_000;
    const M_PER_UNIT: f64 = 0.125;

    #[derive(Default, Clone)]
    struct LineStats {
        samples: u64,
        covered_units: f64,
        to_goal_units: f64,
        inside_6m: u64,
        inside_12m: u64,
        nearest_mate: f64,
        nearest_opp: f64,
        straightness: f64,
        straight_windows: u64,
        still: u64,
        players: u64,
    }

    // 0 GK, 1 DEF, 2 MID, 3 FWD
    let line_of = |id: u32| -> usize {
        match id % 100 {
            0 => 0,
            1..=4 => 1,
            5..=8 => 2,
            _ => 3,
        }
    };

    let mut lines: [LineStats; 4] = Default::default();
    let mut team_length = 0.0_f64;
    let mut team_width = 0.0_f64;
    let mut team_samples = 0_u64;

    // ── BALL CROWD ────────────────────────────────────────────────────
    //
    // "Too many players in one group" is a claim about how many bodies
    // share the same square of grass, and no existing counter measures
    // it: `nearest mate` is a per-player minimum, so a four-man pile and
    // a pair of pairs read identically. These count the population
    // around the BALL (which is where the pile forms) and the population
    // around each PLAYER, both per sampled tick.
    let mut crowd_hist = [0_u64; 12]; // players within 5 m of the ball
    let mut crowd_samples = 0_u64;
    let mut crowd_r5 = 0.0_f64;
    let mut crowd_r10 = 0.0_f64;
    let mut stacked_mate = 0_u64; // player with a TEAM-MATE inside 2 m
    let mut stacked_any = 0_u64; // …with anybody inside 2 m
    let mut stack_samples = 0_u64;

    // ── FROZEN TABLEAU ────────────────────────────────────────────────
    //
    // The per-tick `still` column cannot see this: the replay track is
    // deduplicated at 0.3u, so anybody moving slower than ~1.2 m/s reads
    // as still on some samples and a walking player scores 60%. Net
    // displacement over a FULL SECOND cannot be faked that way — the
    // recorder emits a heartbeat every 750 ms whether the player moved
    // or not — so this is the honest count of players who genuinely went
    // nowhere, and of the moments where most of the pitch did it at once.
    // That is the thing you see in a paused viewer: a still photograph of
    // twenty-two people standing on grass.
    const FROZEN_UNITS: f32 = 2.0; // 25 cm in a second
    let mut frozen_players = 0.0_f64;
    let mut tableau_ticks = 0_u64; // ≥8 of them at once
    let mut tableau_samples = 0_u64;
    let mut tableau_runs = 0_u64;
    let mut tableau_longest = 0_u64;
    // …and the specific thing a paused viewer shows: four or more bodies
    // inside three metres of one another, none of them going anywhere.
    let mut knot_ticks = 0_u64;
    let mut knot_size = 0.0_f64;
    let mut knot_ball = 0.0_f64;

    for m in 0..matches {
        MatchRuntime::set_events_mode(true);
        let (home, _) = make_squad_viewer(1, HOME_TEAM_NAME, level, 0);
        let (away, _) = make_squad_viewer(2, AWAY_TEAM_NAME, level, 11);
        let result = FootballEngine::<840, 545>::play(home, away, true, false, false);
        let data = &result.position_data;

        let ids = data.get_player_ids();
        let last = data.max_timestamp();
        // Which end each team attacks, derived from where its KEEPER is
        // standing at this moment rather than assumed. Teams swap ends at
        // half time, and an assumed direction silently inverts every
        // goal-relative number for one half of the match.

        let mut prev: HashMap<u32, (f32, f32)> = HashMap::new();
        let mut window_start: HashMap<u32, (f32, f32)> = HashMap::new();
        let mut window_path: HashMap<u32, f64> = HashMap::new();
        // Rolling one-second history, one slot per sample.
        const FROZEN_LAG: usize = (1000 / SAMPLE_MS) as usize;
        let mut lag_ring: Vec<HashMap<u32, (f32, f32)>> = Vec::new();
        let mut tableau_run = 0_u64;

        for id in &ids {
            lines[line_of(*id)].players += 1;
        }

        // Who ever moved. Unused substitutes are in `ids` with a single
        // recorded position, and `get_player_position_at` is a
        // nearest-neighbour read, so they would otherwise read as
        // perfectly frozen for ninety minutes.
        let on_pitch: std::collections::HashSet<u32> = ids
            .iter()
            .copied()
            .filter(|id| {
                let a = data.get_player_position_at(*id, last / 4);
                let b = data.get_player_position_at(*id, last / 2);
                let c = data.get_player_position_at(*id, last * 3 / 4);
                match (a, b, c) {
                    (Some(a), Some(b), Some(c)) => {
                        (a - b).magnitude() > 1.0 || (b - c).magnitude() > 1.0
                    }
                    _ => false,
                }
            })
            .collect();

        let mut t = 0_u64;
        while t <= last {
            // Gather everyone once so the pairwise reads are cheap.
            let mut snap: Vec<(u32, (f32, f32))> = Vec::with_capacity(24);
            for id in &ids {
                if let Some(p) = data.get_player_position_at(*id, t) {
                    snap.push((*id, (p.x, p.y)));
                }
            }
            if snap.len() < 4 {
                t += SAMPLE_MS;
                continue;
            }

            // Own goal per team = the end its keeper is nearest.
            let keeper_x = |team_home: bool| -> Option<f32> {
                snap.iter()
                    .find(|(id, _)| (*id < 200) == team_home && *id % 100 == 0)
                    .map(|(_, p)| p.0)
            };
            let home_attacks_right = keeper_x(true).map(|x| x < 420.0).unwrap_or(true);
            let goal_for = |id: u32| -> (f32, f32) {
                let attacks_right = if id < 200 {
                    home_attacks_right
                } else {
                    !home_attacks_right
                };
                if attacks_right {
                    (840.0, 272.5)
                } else {
                    (0.0, 272.5)
                }
            };

            // Who has gone nowhere in the last full second, and are most
            // of them doing it at the same time?
            if lag_ring.len() >= FROZEN_LAG {
                let then = &lag_ring[lag_ring.len() - FROZEN_LAG];
                let mut frozen = 0_usize;
                // Only players who are actually PLAYING. An unused
                // substitute sits on one recorded position all match and
                // is frozen by construction, and there are as many of
                // them as there are starters.
                for (id, pos) in snap.iter().filter(|(id, _)| on_pitch.contains(id)) {
                    if let Some(p0) = then.get(id) {
                        let net = ((pos.0 - p0.0).powi(2) + (pos.1 - p0.1).powi(2)).sqrt();
                        if net < FROZEN_UNITS {
                            frozen += 1;
                        }
                    }
                }
                // The knot: the biggest set of players inside 3 m of one
                // of them, counted only when NONE of them is moving.
                let mut best = 0_usize;
                for (id, pos) in snap.iter().filter(|(id, _)| on_pitch.contains(id)) {
                    let mut n = 0_usize;
                    let mut all_frozen = true;
                    for (oid, opos) in snap.iter().filter(|(id, _)| on_pitch.contains(id)) {
                        if ((pos.0 - opos.0).powi(2) + (pos.1 - opos.1).powi(2)).sqrt() >= 24.0 {
                            continue;
                        }
                        n += 1;
                        let moved = then
                            .get(oid)
                            .map(|p0| {
                                ((opos.0 - p0.0).powi(2) + (opos.1 - p0.1).powi(2)).sqrt()
                                    >= FROZEN_UNITS
                            })
                            .unwrap_or(true);
                        if moved {
                            all_frozen = false;
                            break;
                        }
                    }
                    let _ = id;
                    if all_frozen && n > best {
                        best = n;
                    }
                }
                if best >= 4 {
                    knot_ticks += 1;
                    knot_size += best as f64;
                    if let Some(b) = data.get_ball_position_at(t) {
                        let near = snap
                            .iter()
                            .filter(|(id, _)| on_pitch.contains(id))
                            .map(|(_, p)| ((p.0 - b.x).powi(2) + (p.1 - b.y).powi(2)).sqrt())
                            .fold(f32::MAX, f32::min);
                        knot_ball += near as f64;
                    }
                }

                frozen_players += frozen as f64;
                tableau_samples += 1;
                if frozen >= 8 {
                    tableau_ticks += 1;
                    tableau_run += 1;
                } else {
                    if tableau_run > 0 {
                        tableau_runs += 1;
                        tableau_longest = tableau_longest.max(tableau_run);
                    }
                    tableau_run = 0;
                }
            }
            lag_ring.push(snap.iter().copied().collect());
            if lag_ring.len() > FROZEN_LAG + 1 {
                lag_ring.remove(0);
            }

            // How many bodies are inside 5 m / 10 m of the ball, and how
            // many players have somebody standing on top of them. 40u =
            // 5 m, 80u = 10 m, 16u = 2 m.
            if let Some(b) = data.get_ball_position_at(t) {
                let mut n5 = 0_usize;
                let mut n10 = 0_usize;
                for (_, p) in &snap {
                    let d = ((p.0 - b.x).powi(2) + (p.1 - b.y).powi(2)).sqrt();
                    if d < 40.0 {
                        n5 += 1;
                    }
                    if d < 80.0 {
                        n10 += 1;
                    }
                }
                crowd_hist[n5.min(11)] += 1;
                crowd_r5 += n5 as f64;
                crowd_r10 += n10 as f64;
                crowd_samples += 1;
            }
            for (id, pos) in &snap {
                let mut mate = false;
                let mut any = false;
                for (oid, opos) in &snap {
                    if oid == id {
                        continue;
                    }
                    let d = ((pos.0 - opos.0).powi(2) + (pos.1 - opos.1).powi(2)).sqrt();
                    if d < 16.0 {
                        any = true;
                        if (*id < 200) == (*oid < 200) {
                            mate = true;
                        }
                    }
                }
                stack_samples += 1;
                if mate {
                    stacked_mate += 1;
                }
                if any {
                    stacked_any += 1;
                }
            }

            // Team shape (home outfielders only — one team is enough).
            let outfield: Vec<&(u32, (f32, f32))> = snap
                .iter()
                .filter(|(id, _)| *id > 100 && *id < 200)
                .collect();
            if outfield.len() >= 8 {
                let xs = outfield.iter().map(|(_, p)| p.0);
                let ys = outfield.iter().map(|(_, p)| p.1);
                let (min_x, max_x) = xs.fold((f32::MAX, f32::MIN), |a, v| (a.0.min(v), a.1.max(v)));
                let (min_y, max_y) = ys.fold((f32::MAX, f32::MIN), |a, v| (a.0.min(v), a.1.max(v)));
                team_length += (max_x - min_x) as f64;
                team_width += (max_y - min_y) as f64;
                team_samples += 1;
            }

            for (id, pos) in &snap {
                let li = line_of(*id);
                let s = &mut lines[li];
                s.samples += 1;

                let (gx, gy) = goal_for(*id);
                let dg = (((pos.0 - gx).powi(2) + (pos.1 - gy).powi(2)).sqrt()) as f64;
                s.to_goal_units += dg;
                if dg * M_PER_UNIT < 6.0 {
                    s.inside_6m += 1;
                }
                if dg * M_PER_UNIT < 12.0 {
                    s.inside_12m += 1;
                }

                let same_team = |a: u32, b: u32| (a < 200) == (b < 200);
                let mut best_mate = f64::MAX;
                let mut best_opp = f64::MAX;
                for (oid, opos) in &snap {
                    if oid == id {
                        continue;
                    }
                    let d = (((pos.0 - opos.0).powi(2) + (pos.1 - opos.1).powi(2)).sqrt()) as f64;
                    if same_team(*id, *oid) {
                        best_mate = best_mate.min(d);
                    } else {
                        best_opp = best_opp.min(d);
                    }
                }
                if best_mate < f64::MAX {
                    s.nearest_mate += best_mate;
                }
                if best_opp < f64::MAX {
                    s.nearest_opp += best_opp;
                }

                if let Some(p0) = prev.get(id) {
                    let step = (((pos.0 - p0.0).powi(2) + (pos.1 - p0.1).powi(2)).sqrt()) as f64;
                    s.covered_units += step;
                    *window_path.entry(*id).or_insert(0.0) += step;
                    // 0.5 m/s over a 30 ms step = 0.015 m = 0.12 units.
                    if step < 0.12 {
                        s.still += 1;
                    }
                }
                prev.insert(*id, *pos);
                window_start.entry(*id).or_insert(*pos);
            }

            if t > 0 && t % WINDOW_MS == 0 {
                for (id, pos) in &snap {
                    if let (Some(start), Some(path)) =
                        (window_start.get(id), window_path.get(id).copied())
                    {
                        if path > 1.0 {
                            let net = (((pos.0 - start.0).powi(2) + (pos.1 - start.1).powi(2))
                                .sqrt()) as f64;
                            let s = &mut lines[line_of(*id)];
                            s.straightness += net / path;
                            s.straight_windows += 1;
                        }
                    }
                }
                window_start.clear();
                window_path.clear();
            }

            t += SAMPLE_MS;
        }
        eprintln!("  paths: match {}/{} traced", m + 1, matches);
    }

    println!();
    println!(
        "=== PLAYER PATH TRACE ({} matches, level {}) ===",
        matches, level
    );
    println!(
        "  {:<5} {:>8} {:>9} {:>7} {:>7} {:>8} {:>8} {:>9} {:>7}",
        "line", "covered", "to goal", "<6m", "<12m", "mate", "opp", "straight", "still"
    );
    let names = ["GK", "DEF", "MID", "FWD"];
    let real_km = [5.0, 10.0, 11.0, 10.0];
    for (i, name) in names.iter().enumerate() {
        let s = &lines[i];
        if s.samples == 0 {
            continue;
        }
        let per_player = (s.players.max(1) / matches.max(1) as u64).max(1) as f64;
        let km = s.covered_units * M_PER_UNIT / 1000.0 / per_player / matches as f64;
        let n = s.samples as f64;
        println!(
            "  {:<5} {:>6.1}km {:>7.1}m {:>6.1}% {:>6.1}% {:>6.1}m {:>6.1}m {:>8.2} {:>6.1}%  (real ~{:.0}km)",
            name,
            km,
            s.to_goal_units / n * M_PER_UNIT,
            s.inside_6m as f64 / n * 100.0,
            s.inside_12m as f64 / n * 100.0,
            s.nearest_mate / n * M_PER_UNIT,
            s.nearest_opp / n * M_PER_UNIT,
            s.straightness / s.straight_windows.max(1) as f64,
            s.still as f64 / n * 100.0,
            real_km[i],
        );
    }
    if team_samples > 0 {
        println!();
        println!(
            "  team shape (home outfield): length {:.1}m  width {:.1}m   (real ~35-45m x ~45-55m)",
            team_length / team_samples as f64 * M_PER_UNIT,
            team_width / team_samples as f64 * M_PER_UNIT,
        );
    }
    if crowd_samples > 0 {
        println!();
        println!("--- BALL CROWD (how many bodies share the ball's square of grass) ---");
        println!(
            "  within 5m: {:.2} players   within 10m: {:.2}   (real ~1.5-2.5 / ~4-5)",
            crowd_r5 / crowd_samples as f64,
            crowd_r10 / crowd_samples as f64,
        );
        print!("  distribution inside 5m:");
        for (n, count) in crowd_hist.iter().enumerate() {
            if *count == 0 {
                continue;
            }
            print!(
                " {}:{:.0}%",
                n,
                *count as f64 / crowd_samples as f64 * 100.0
            );
        }
        println!();
        println!(
            "  a player has a TEAM-MATE inside 2m on {:.1}% of his ticks, anybody on {:.1}%",
            stacked_mate as f64 / stack_samples.max(1) as f64 * 100.0,
            stacked_any as f64 / stack_samples.max(1) as f64 * 100.0,
        );
    }
    if tableau_samples > 0 {
        let n = tableau_samples as f64;
        println!(
            "  genuinely FROZEN (net <25cm over a full second): {:.1} of 22 players on average; \
             8+ at once on {:.1}% of ticks ({:.1}s/match over {} spells, longest {:.1}s)",
            frozen_players / n,
            tableau_ticks as f64 / n * 100.0,
            tableau_ticks as f64 * SAMPLE_MS as f64 / 1000.0 / matches as f64,
            tableau_runs,
            tableau_longest as f64 * SAMPLE_MS as f64 / 1000.0,
        );
        println!(
            "  a STANDING KNOT (4+ bodies inside 3m, none of them moving): {:.1}% of ticks, \
             {:.1}s/match, mean {:.1} players",
            knot_ticks as f64 / n * 100.0,
            knot_ticks as f64 * SAMPLE_MS as f64 / 1000.0 / matches as f64,
            knot_size / knot_ticks.max(1) as f64,
        );
        let _ = knot_ball;
    }
    println!();
    println!("  reference: nearest team-mate ~15-20m, nearest opponent ~5-12m,");
    println!("             straightness ~0.6-0.9 for purposeful movement, still ~15-25%");
}

// ── reel: what is actually ON the highlight reel ───────────────────────
//
// The game keeps a handful of clips out of a whole match (see
// `RecordingScope::Goals`), and three different rules decide which: a goal is
// never in doubt, a substitution is never in doubt, and a near miss has to
// survive `HighlightSelector`. Nothing else in this harness can say what the
// mix comes out as, and the mix IS the product — a reel that is four
// substitutions and one shot is a match report about the bench.
//
// Every number is per MATCH. The one to read first is the chance count: it is
// the only one of the three the selector controls, and it is what
// `HighlightSelector::PER_TEAM` and `MIN_XG` are tuned against.

/// One match's reel, counted at full time.
struct ReelRow {
    goals: usize,
    /// Near misses that survived the shortlist, by side.
    home_chances: usize,
    away_chances: usize,
    /// Changes made, and the number of separate STOPPAGES they were made at —
    /// a double change is one window and one marker.
    changes: usize,
    stoppages: usize,
    shots: u32,
    /// The recording itself: how many separate stretches, how long they run,
    /// and how much of that is inside a substitution's window.
    segments: usize,
    recorded_ms: u64,
    change_ms: u64,
    total_ms: u64,
}

struct ReelCensus;

impl ReelCensus {
    /// Plays `n` matches at the scope the GAME records at, and reports what
    /// each reel came out holding.
    fn run(n: usize, level_a: Option<u8>, level_b: Option<u8>) {
        // Process-global, and every match below wants the same answer out of
        // it — so it is set once here rather than per match.
        MatchRuntime::set_recording_scope(core::r#match::RecordingScope::Goals);

        println!(
            "Reel census: {} matches recorded the way the game records them \
             (RecordingScope::Goals)",
            n
        );
        println!(
            "  selector: PER_TEAM={}  MIN_XG={:.3}",
            core::r#match::HighlightSelector::PER_TEAM,
            core::r#match::HighlightSelector::MIN_XG,
        );
        println!();

        let pairs: Vec<(u8, u8)> = (0..n)
            .map(|_| {
                (
                    level_a.unwrap_or_else(random_level),
                    level_b.unwrap_or_else(random_level),
                )
            })
            .collect();

        let rows: Vec<ReelRow> = pairs
            .par_iter()
            .map(|&(a, b)| {
                let home = Self::squad(1, a);
                let away = Self::squad(2, b);
                // `true` — position recording on, which is what cuts the clips.
                let result = FootballEngine::<840, 545>::play(home, away, true, false, false);
                Self::read(&result)
            })
            .collect();

        Self::report(&rows);
    }

    /// The XI `run_stats` calibrates against, with a bench put on it.
    ///
    /// Both halves matter. The XI has to be the calibrated one or the census
    /// is counting chances in a match that takes half again as many shots as
    /// the game does â€” `make_squad_viewer`'s plain 442 measured 38.6 shots
    /// and 5.2 goals against a population of 25.5 and 2.7. And the bench has
    /// to be there at all, because a squad with nobody on it makes no changes,
    /// and the changes are half of what this census is for.
    ///
    /// Three levels weaker, as in `subs`: it is what puts the better players
    /// on the pitch to start with, which is what makes a change a decision
    /// rather than a coin toss.
    fn squad(team_id: u32, level: u8) -> MatchSquad {
        const BENCH: [PlayerPositionType; 7] = [
            PlayerPositionType::Goalkeeper,
            PlayerPositionType::DefenderCenterLeft,
            PlayerPositionType::DefenderCenterRight,
            PlayerPositionType::MidfielderCenterLeft,
            PlayerPositionType::MidfielderCenterRight,
            PlayerPositionType::ForwardLeft,
            PlayerPositionType::ForwardRight,
        ];

        let mut squad = make_squad_simple(team_id, level);
        let bench_level = level.saturating_sub(3).max(1);
        squad.substitutes = BENCH
            .iter()
            .enumerate()
            .map(|(i, &pos)| {
                let player = generate_player(team_id * 100 + 11 + i as u32, pos, bench_level);
                MatchPlayer::from_player(team_id, &player, pos, true, None)
            })
            .collect();
        squad
    }
    fn read(result: &core::r#match::MatchResultRaw) -> ReelRow {
        let score = result.score.as_ref().expect("score");
        let goals = score
            .detail()
            .iter()
            .filter(|detail| {
                detail.stat_type == core::r#match::player::statistics::MatchStatisticType::Goal
            })
            .count();

        let home_chances = result.chances.iter().filter(|c| c.team_id == 1).count();
        let away_chances = result.chances.iter().filter(|c| c.team_id == 2).count();

        // Distinct stoppages, not distinct changes: two men swapped on the
        // same whistle share a window and a marker.
        let mut marks: Vec<u64> = result
            .substitutions
            .iter()
            .map(|change| change.match_time_ms)
            .collect();
        marks.sort_unstable();
        marks.dedup();

        let shots = team_stats(result, 1).shots as u32 + team_stats(result, 2).shots as u32;

        let (segments, recorded_ms, change_ms) = match result.position_data.recorded_segments() {
            Some(segments) => {
                let recorded: u64 = segments.iter().map(|(from, to)| to - from).sum();
                // How much of what was kept sits inside a substitution's own
                // window. An intersection rather than `stoppages x clip
                // length`, because those windows merge with each other and
                // with the clip around a goal.
                let inside: u64 = segments
                    .iter()
                    .map(|(from, to)| {
                        marks
                            .iter()
                            .map(|mark| {
                                let start = mark.saturating_sub(
                                    core::r#match::result::SUBSTITUTION_CLIP_PRE_ROLL_MS,
                                );
                                let end =
                                    mark + core::r#match::result::SUBSTITUTION_CLIP_POST_ROLL_MS;
                                (*to).min(end).saturating_sub((*from).max(start))
                            })
                            .max()
                            .unwrap_or(0)
                    })
                    .sum();
                (segments.len(), recorded, inside)
            }
            None => (0, 0, 0),
        };

        ReelRow {
            goals,
            home_chances,
            away_chances,
            changes: result.substitutions.len(),
            stoppages: marks.len(),
            shots,
            segments,
            recorded_ms,
            change_ms,
            total_ms: result.match_time_ms,
        }
    }

    fn report(rows: &[ReelRow]) {
        let n = rows.len().max(1) as f64;
        let mean = |pick: fn(&ReelRow) -> f64| rows.iter().map(pick).sum::<f64>() / n;
        let chances = |row: &ReelRow| (row.home_chances + row.away_chances) as f64;

        println!("MARKERS PER MATCH (what the timeline carries)");
        println!("  goals          {:>5.2}", mean(|r| r.goals as f64));
        println!(
            "  chances        {:>5.2}   ({:.2} + {:.2} a side)",
            mean(chances),
            mean(|r| r.home_chances as f64),
            mean(|r| r.away_chances as f64),
        );
        println!(
            "  substitutions  {:>5.2}   ({:.2} changes at {:.2} stoppages)",
            mean(|r| r.stoppages as f64),
            mean(|r| r.changes as f64),
            mean(|r| r.stoppages as f64),
        );
        println!(
            "  ── total       {:>5.2}   out of {:.1} shots taken",
            mean(|r| (r.goals + r.home_chances + r.away_chances + r.stoppages) as f64),
            mean(|r| r.shots as f64),
        );
        println!();

        // The distribution is what the mean hides: a reel is watched one match
        // at a time, and "4.6 on average" is no comfort to the match that got
        // one.
        let mut histogram = [0usize; 16];
        for row in rows {
            let kept = (row.home_chances + row.away_chances).min(histogram.len() - 1);
            histogram[kept] += 1;
        }
        print!("CHANCE MARKERS PER MATCH  ");
        for (kept, count) in histogram.iter().enumerate() {
            if *count > 0 {
                print!("{}:{:.0}% ", kept, *count as f64 / n * 100.0);
            }
        }
        println!();
        let at_least = |bar: usize| {
            rows.iter()
                .filter(|row| row.home_chances + row.away_chances >= bar)
                .count() as f64
                / n
                * 100.0
        };
        println!(
            "  five or more: {:.0}%    ten or more: {:.0}%    none at all: {:.0}%",
            at_least(5),
            at_least(10),
            100.0 - at_least(1),
        );
        println!();

        println!("THE RECORDING ITSELF");
        println!(
            "  {:.1} segments, {:.0}s kept of {:.0}s played ({:.1}% of the match)",
            mean(|r| r.segments as f64),
            mean(|r| r.recorded_ms as f64) / 1000.0,
            mean(|r| r.total_ms as f64) / 1000.0,
            mean(|r| r.recorded_ms as f64) / mean(|r| r.total_ms as f64).max(1.0) * 100.0,
        );
        println!(
            "  {:.0}s of it ({:.0}%) is inside a substitution's window",
            mean(|r| r.change_ms as f64) / 1000.0,
            mean(|r| r.change_ms as f64) / mean(|r| r.recorded_ms as f64).max(1.0) * 100.0,
        );
    }
}

/// Headless sibling of `run_viewer`: play one match with position
/// recording on and write the same chunk files, then exit. No axum
/// server, no browser launch — so a replay can be captured and inspected
/// without taking over the machine.
fn run_record(level_a: Option<u8>, level_b: Option<u8>, clipped: bool) {
    MatchRuntime::set_events_mode(true);
    if clipped {
        MatchRuntime::set_recording_scope(core::r#match::RecordingScope::Goals);
    }

    let level_a = level_a.unwrap_or_else(random_level);
    let level_b = level_b.unwrap_or_else(random_level);

    let (home_squad, _) = make_squad_viewer(1, HOME_TEAM_NAME, level_a, 0);
    let (away_squad, _) = make_squad_viewer(2, AWAY_TEAM_NAME, level_b, 11);

    let result = FootballEngine::<840, 545>::play(home_squad, away_squad, true, false, false);
    let score = result.score.as_ref().unwrap();
    println!(
        "recorded: {}:{} (level {} vs {})",
        score.home_team.get(),
        score.away_team.get(),
        level_a,
        level_b
    );
    println!(
        "substitutions: {} [{}]",
        result.substitutions.len(),
        result
            .substitutions
            .iter()
            .map(|s| {
                format!(
                    "{}ms {}->{} ({}s)",
                    s.match_time_ms,
                    s.player_out_id,
                    s.player_in_id,
                    s.break_ms / 1000
                )
            })
            .collect::<Vec<_>>()
            .join(", ")
    );

    // **Does every marker have footage behind it?**
    //
    // Under the game's clipped scope the recording is a handful of segments
    // and everything outside them is grey. A goal, a chance or a substitution
    // whose window falls in a hole is a mark on the timeline that leads
    // nowhere, and nothing but this says so — the result and the recording
    // are built by different passes and neither checks the other.
    if let Some(segments) = result.position_data.recorded_segments() {
        let covers = |from: u64, to: u64| {
            segments
                .iter()
                .any(|(start, end)| from >= *start && to <= *end)
        };
        println!(
            "recorded segments: {} covering {:.0}s of {:.0}s",
            segments.len(),
            segments.iter().map(|(a, b)| (b - a) as f64).sum::<f64>() / 1000.0,
            result.match_time_ms as f64 / 1000.0,
        );
        let mut missing = 0;
        for change in &result.substitutions {
            if !covers(change.match_time_ms, change.match_time_ms + change.break_ms) {
                println!(
                    "  NOT RECORDED: substitution at {}ms ({}->{})",
                    change.match_time_ms, change.player_out_id, change.player_in_id
                );
                missing += 1;
            }
        }
        println!(
            "  {} of {} substitutions have footage behind their marker",
            result.substitutions.len() - missing,
            result.substitutions.len()
        );
    }

    let out_dir = PathBuf::from("match_results").join(LEAGUE_SLUG);
    std::fs::create_dir_all(&out_dir).expect("failed to create output dir");
    // Stale chunks from a longer previous match would be read back as part
    // of this one — the analysis concatenates every chunk in the folder.
    if let Ok(entries) = std::fs::read_dir(&out_dir) {
        for entry in entries.flatten() {
            let _ = std::fs::remove_file(entry.path());
        }
    }

    let chunks = result.position_data.split_into_chunks(CHUNK_DURATION_MS);
    for (idx, chunk) in chunks.iter().enumerate() {
        let data = serde_json::to_vec(chunk).expect("failed to serialize chunk");
        save_gzip_json(
            &out_dir.join(format!("{}_chunk_{}.json.gz", MATCH_ID, idx)),
            &data,
        );
    }
    println!("wrote {} chunks to {}", chunks.len(), out_dir.display());
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

    let (home_squad, mut players_json) = make_squad_viewer(1, HOME_TEAM_NAME, level_a, 0);
    let (away_squad, away_players) = make_squad_viewer(2, AWAY_TEAM_NAME, level_b, 11);
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

    let chances_json: Vec<ChanceJson> = result
        .chances
        .iter()
        .map(|c| ChanceJson {
            player_id: c.player_id,
            time: c.time,
        })
        .collect();

    let substitutions_json: Vec<SubstitutionJson> = result
        .substitutions
        .iter()
        .map(|s| SubstitutionJson {
            player_in_id: s.player_in_id,
            player_out_id: s.player_out_id,
            time: s.match_time_ms,
            break_ms: s.break_ms,
        })
        .collect();
    println!(
        "Substitutions: {} ({})",
        substitutions_json.len(),
        substitutions_json
            .iter()
            .map(|s| format!("{}'", s.time / 60_000))
            .collect::<Vec<_>>()
            .join(", ")
    );

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

    let _ = VIEWER_PAGE.set(ViewerPage::render(
        home_goals,
        away_goals,
        level_a,
        level_b,
        result.match_time_ms,
        &goals_json,
        &chances_json,
        &substitutions_json,
        &players_json,
    ));

    if VIEWER_WASM_GZ.is_empty() {
        println!(
            "\nWARNING: the match viewer was not built — run `rustup target add {}` and rebuild",
            "wasm32-unknown-unknown"
        );
    }

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
        // Same URLs the web server uses, so the page markup is identical on
        // both sides.
        .route("/static/viewer/match_viewer.js", get(viewer_script_handler))
        .route(
            "/static/viewer/match_viewer_bg.wasm",
            get(viewer_wasm_handler),
        );

    let listener = tokio::net::TcpListener::bind("0.0.0.0:18001")
        .await
        .unwrap();
    axum::serve(listener, app).await.unwrap();
}

async fn page_handler() -> axum::response::Html<String> {
    axum::response::Html(VIEWER_PAGE.get().cloned().unwrap_or_default())
}

async fn viewer_script_handler() -> impl axum::response::IntoResponse {
    viewer_asset(VIEWER_SCRIPT_GZ, "text/javascript")
}

async fn viewer_wasm_handler() -> impl axum::response::IntoResponse {
    viewer_asset(VIEWER_WASM_GZ, "application/wasm")
}

/// Both viewer files are stored gzipped — ~30 MB of Bevy and wgpu inflated is
/// nothing worth holding — and handed straight to the browser to inflate. The
/// wasm keeps its real content type so the browser can stream-compile it.
fn viewer_asset(body: &'static [u8], content_type: &'static str) -> axum::response::Response {
    if body.is_empty() {
        return (
            axum::http::StatusCode::NOT_FOUND,
            "match viewer was not built",
        )
            .into_response();
    }
    (
        axum::http::StatusCode::OK,
        [
            (axum::http::header::CONTENT_TYPE, content_type),
            (axum::http::header::CONTENT_ENCODING, "gzip"),
        ],
        body,
    )
        .into_response()
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

/// Trace what a ball that clears the crossbar actually does.
///
/// Report, 2026-08-21: *"the ball flies over the goal, then appears on the
/// goal line on the floor, and it runs into the goal."* Every part of that
/// is a sequence — the award, the several seconds of flight behind the
/// goal, whatever stops it, and the restart — so the only instrument that
/// can answer it is the per-tick trace. `over` opens a window on
/// `check_over_goal` and on every endline goal kick, which is the family
/// the report belongs to.
fn run_over_the_bar(matches: usize, level: u8) {
    unsafe { std::env::set_var("OF_FRAME_TRACE", "over") };
    core::frame_trace::FrameTrace::reset();

    for m in 0..matches {
        MatchRuntime::set_events_mode(true);
        let (home, _) = make_squad_viewer(1, HOME_TEAM_NAME, level, 0);
        let (away, _) = make_squad_viewer(2, AWAY_TEAM_NAME, level, 11);
        let _ = FootballEngine::<840, 545>::play(home, away, true, false, false);
        eprintln!("  overbar: match {}/{} played", m + 1, matches);
    }

    let (_, captures) = core::frame_trace::FrameTrace::report();
    let s = core::frame_trace::FrameTrace::summary();
    println!();
    println!("=== OVER-THE-BAR TRACE ({matches} matches, level {level}) ===");
    println!("  {} captures", captures.len());
    println!(
        "  a '*' row travelled further than its own velocity explains — somebody relocated the ball"
    );
    println!();
    println!("  mesh jumps (netting PULLED the ball) : {}", s.mesh_jumps);
    println!("  loose jumps (open play, unclaimed)   : {}", s.loose_jumps);
    println!(
        "  worst unexplained jump               : {} cm",
        s.worst_jump_cm
    );
    println!(
        "  ground snaps (z collapsed, no fall)  : {}",
        s.ground_snaps
    );
    println!(
        "  windows that ended with the ball still in the goal: {}",
        s.rested_in_net
    );
    for capture in &captures {
        println!();
        println!("{capture}");
    }
}

/// **Where a ball that goes UP ends up.**
///
/// The same trace as [`run_over_the_bar`], triggered on the ball climbing
/// through [`FrameTrace::SKY_HEIGHT`] instead of on a line crossing. The
/// report it answers — *"the ball flies upward, hits an invisible obstacle
/// at a height and flies back down"* — names no resolver and no restart,
/// so the height itself is the only thing that can open the window.
///
/// What it found on the run it was written for: a midfielder's knock-down
/// header leaving the forehead at **110 m/s straight up**, trimmed by the
/// ball physics' own `MAX_APEX_METRES` guard and still peaking 28 m up.
/// Nothing else could see it — the woodwork triggers need the frame, the
/// out-of-play ones need a line, and the apex census in `flight_diag`
/// counts the launch without being able to name the site.
struct SkiedBallTrace;

impl SkiedBallTrace {
    fn run(matches: usize, level: u8) {
        unsafe { std::env::set_var("OF_FRAME_TRACE", "sky") };
        FrameTrace::reset();

        for m in 0..matches {
            MatchRuntime::set_events_mode(true);
            let (home, _) = make_squad_viewer(1, HOME_TEAM_NAME, level, 0);
            let (away, _) = make_squad_viewer(2, AWAY_TEAM_NAME, level, 11);
            let _ = FootballEngine::<840, 545>::play(home, away, true, false, false);
            eprintln!("  sky: match {}/{} played", m + 1, matches);
        }

        let (_, captures) = FrameTrace::report();
        let summary = FrameTrace::summary();
        println!();
        println!("=== SKIED-BALL TRACE ({matches} matches, level {level}) ===");
        println!("  {} captures", captures.len());
        println!(
            "  a '*' row travelled further than its own velocity explains — somebody relocated the ball"
        );
        println!();
        println!(
            "  mesh jumps (netting PULLED the ball) : {}",
            summary.mesh_jumps
        );
        println!(
            "  loose jumps (open play, unclaimed)   : {}",
            summary.loose_jumps
        );
        println!(
            "  worst unexplained jump               : {} cm",
            summary.worst_jump_cm
        );
        println!(
            "  ground snaps (z collapsed, no fall)  : {}",
            summary.ground_snaps
        );
        for capture in &captures {
            println!();
            println!("{capture}");
        }
    }
}

// ── waypoints: tactical-route census ───────────────────────────────────
//
// The route layer is the oldest movement code in the engine and the only
// one nothing has ever measured. Seven states consult it — Defender
// Standing/Walking, Midfielder Running/Walking, Forward Running/Standing/
// Walking — and in each of them the route branch is the FIRST thing
// `velocity()` tries, so whatever it says overrides the anchor-, shape-
// and duty-derived movement underneath.
//
// Four blocks:
//   ROUTE GEOMETRY — the routes themselves, printed straight out of the
//     generator for both sides. Static, and enough on its own to see
//     whether a route is a football movement.
//   USAGE BY STATE — evals vs takes, so "is it used at all" is a number.
//   ROUTE WALK — how the index moves, and where it comes to rest.
//   ROUTE vs SHAPE — how far the route target is from the anchor the
//     team plan gave the same player on the same tick, and how often the
//     two point in opposite directions.
struct WaypointCensusRun;

impl WaypointCensusRun {
    fn run(matches: usize, level: u8) {
        use core::waypoint_census::WaypointCensus;

        Self::print_geometry();

        WaypointCensus::reset();
        for m in 0..matches {
            MatchRuntime::set_events_mode(false);
            let (home, _) = make_squad_viewer(1, HOME_TEAM_NAME, level, 0);
            let (away, _) = make_squad_viewer(2, AWAY_TEAM_NAME, level, 11);
            let _ = FootballEngine::<840, 545>::play(home, away, false, false, false);
            eprintln!("  waypoints: match {}/{} played", m + 1, matches);
        }

        Self::print_usage(matches, level);
        Self::print_walk();
        Self::print_vs_shape();
    }

    /// Every generated route, for both sides.
    fn print_geometry() {
        use core::r#match::PlayerSide;
        use core::r#match::engine::tactics::positions::TacticalPositions;

        println!();
        println!("=== ROUTE GEOMETRY (as generated, pitch 840x545, goals at x=0 / x=840) ===");
        println!(
            "  {:<28} {:<6} {:>7}  {}",
            "position", "side", "end-x", "route (x,y) ..."
        );
        for (position, _, _) in core::r#match::POSITION_POSITIONING {
            for (side, label) in [(PlayerSide::Left, "home"), (PlayerSide::Right, "away")] {
                let tp = TacticalPositions::new(*position, Some(side));
                let route = &tp.tactical_positions[0].waypoints;
                let end = route.last().copied().unwrap_or((0.0, 0.0));
                // How far up the pitch the route ENDS, toward the goal
                // this side attacks. 100% is the opposition goal line.
                let depth = match side {
                    PlayerSide::Left => end.0 / 840.0,
                    PlayerSide::Right => (840.0 - end.0) / 840.0,
                };
                let pts: Vec<String> = route
                    .iter()
                    .map(|(x, y)| format!("({:.0},{:.0})", x, y))
                    .collect();
                println!(
                    "  {:<28} {:<6} {:>6.0}%  {}",
                    format!("{:?}", position),
                    label,
                    depth * 100.0,
                    pts.join(" -> ")
                );
            }
        }
    }

    fn print_usage(matches: usize, level: u8) {
        use core::waypoint_census::WaypointCensus;

        println!();
        println!(
            "=== WAYPOINT USAGE BY STATE ({} match(es), level {}) ===",
            matches, level
        );
        println!(
            "  {:<34} {:>12} {:>12} {:>8}",
            "state", "asked", "followed", "take%"
        );
        let mut total_evals = 0u64;
        let mut total_takes = 0u64;
        for state in PlayerState::all() {
            let (evals, takes) = WaypointCensus::by_state(state.compact_id());
            if evals == 0 {
                continue;
            }
            total_evals += evals;
            total_takes += takes;
            println!(
                "  {:<34} {:>12} {:>12} {:>7.1}%",
                format!("{}", state),
                evals,
                takes,
                takes as f64 / evals as f64 * 100.0
            );
        }
        println!(
            "  {:<34} {:>12} {:>12} {:>7.1}%",
            "ALL",
            total_evals,
            total_takes,
            total_takes as f64 / total_evals.max(1) as f64 * 100.0
        );
    }

    fn print_walk() {
        use core::waypoint_census::WaypointCensus;

        let mgr = WaypointCensus::manager();
        println!();
        println!("=== ROUTE WALK ===");
        println!("  manager updates                        : {}", mgr.ticks);
        println!(
            "  index advances                         : {} ({:.4} per update)",
            mgr.advances,
            mgr.advances as f64 / mgr.ticks.max(1) as f64
        );
        println!(
            "    of which by the past-next projection : {} ({:.0}%)",
            mgr.advances_past_next,
            mgr.advances_past_next as f64 / mgr.advances.max(1) as f64 * 100.0
        );
        println!(
            "  routes run to their end (completions)  : {}",
            mgr.completions
        );
        println!("  routes re-armed at waypoint 0          : {}", mgr.rearms);

        println!();
        println!(
            "  {:<12} {:>10} {:>10} {:>9} {:>9}   {}",
            "group", "targeted", "followed", "at end%", "done%", "index histogram 0..7"
        );
        for group in Self::GROUPS {
            let row = WaypointCensus::by_group(group);
            if row.geom == 0 {
                continue;
            }
            let hist: Vec<String> = row
                .idx
                .iter()
                .map(|n| format!("{:.0}%", *n as f64 / row.geom as f64 * 100.0))
                .collect();
            println!(
                "  {:<12} {:>10} {:>10} {:>8.1}% {:>8.1}%   {}",
                format!("{:?}", group),
                row.geom,
                row.takes,
                row.terminus as f64 / row.geom as f64 * 100.0,
                row.completed as f64 / row.geom as f64 * 100.0,
                hist.join(" ")
            );
        }
    }

    fn print_vs_shape() {
        use core::waypoint_census::WaypointCensus;

        println!();
        println!("=== ROUTE vs SHAPE (depth: 0% = own goal line, 100% = opposition goal line) ===");
        println!(
            "  {:<12} {:>9} {:>9} {:>9} {:>10} {:>10} {:>9} {:>9} {:>8}",
            "group", "he-is", "shape", "route", "route-m", "anchor-m", "gap-m", "opposed%", "3rd%",
        );
        for group in Self::GROUPS {
            let row = WaypointCensus::by_group(group);
            if row.geom == 0 {
                continue;
            }
            const M: f64 = 0.125; // metres per field unit
            let decided = (row.agree + row.opposed).max(1);
            println!(
                "  {:<12} {:>8.1}% {:>8.1}% {:>8.1}% {:>9.1} {:>9.1} {:>8.1} {:>8.1}% {:>7.1}%",
                format!("{:?}", group),
                row.mean_player_depth * 100.0,
                row.mean_anchor_depth * 100.0,
                row.mean_target_depth * 100.0,
                row.mean_to_target_u * M,
                row.mean_to_anchor_u * M,
                row.mean_target_to_anchor_u * M,
                row.opposed as f64 / decided as f64 * 100.0,
                row.target_final_third as f64 / row.geom as f64 * 100.0,
            );
        }

        println!();
        println!(
            "  {:<12} {:>10} {:>10} {:>10} {:>10} {:>10} {:>10}",
            "group", "asked", "disarmed%", "carrier%", "chaser%", "crowded%", "empty%"
        );
        for group in Self::GROUPS {
            let row = WaypointCensus::by_group(group);
            if row.evals == 0 {
                continue;
            }
            let e = row.evals as f64;
            println!(
                "  {:<12} {:>10} {:>9.1}% {:>9.1}% {:>9.1}% {:>9.1}% {:>9.1}%",
                format!("{:?}", group),
                row.evals,
                row.disarmed as f64 / e * 100.0,
                row.skip_carrier as f64 / e * 100.0,
                row.skip_chaser as f64 / e * 100.0,
                row.crowded as f64 / e * 100.0,
                row.skip_empty as f64 / e * 100.0,
            );
        }
    }

    const GROUPS: [PlayerFieldPositionGroup; 4] = [
        PlayerFieldPositionGroup::Goalkeeper,
        PlayerFieldPositionGroup::Defender,
        PlayerFieldPositionGroup::Midfielder,
        PlayerFieldPositionGroup::Forward,
    ];
}

// ── heat: the thermal map ─────────────────────────────────────────────
//
/// **Where the twenty-two actually spend the match**, printed the way a
/// broadcaster prints it.
///
/// Reads [`core::heatmap_diag`], which samples the FIELD at 20 Hz of match
/// time rather than reading the replay track — the track is deduplicated
/// at 0.3 u with a 750 ms heartbeat, so counting its samples measures
/// movement, and a heat map is the opposite of that: it is a picture of
/// where a man STOOD.
///
/// Both sides are folded onto one another attacking RIGHT, so every map
/// below is the average of twenty-two shirts in eleven slots, and each is
/// printed beside the number a real one gives.
///
/// `OF_HEAT_JSON=<path>` additionally dumps every grid so the maps can be
/// drawn properly somewhere that has pixels.
struct HeatCensusRun;

impl HeatCensusRun {
    /// Pitch area, m² — the denominator for every area share below.
    const PITCH_AREA: f32 = heat::PITCH_LENGTH_M * heat::PITCH_WIDTH_M;
    /// Two grid rows to a printed line: a character then covers 1.25 m by
    /// 4.87 m, which a terminal's own 1:2 cell shape renders very nearly
    /// square.
    const ROW_MERGE: usize = 2;

    fn run(matches: usize, level: u8, minutes: u64) {
        HeatMapCensus::reset();
        HeatMapCensus::set_window(minutes * 60_000);
        MatchRuntime::set_events_mode(true);

        println!(
            "Thermal map: {} match(es), both squads level {}, {} — {}",
            matches,
            level,
            HarnessTactic::label(),
            if minutes == 0 {
                "whole match".to_string()
            } else {
                format!("first {} min of each", minutes)
            }
        );

        for m in 0..matches {
            let (home, _) = make_squad_viewer(1, HOME_TEAM_NAME, level, 0);
            let (away, _) = make_squad_viewer(2, AWAY_TEAM_NAME, level, 11);
            let result = FootballEngine::<840, 545>::play(home, away, true, false, false);
            if let Some(score) = result.score.as_ref() {
                eprintln!(
                    "  heat: match {}/{} played ({}:{})",
                    m + 1,
                    matches,
                    score.home_team.get(),
                    score.away_team.get()
                );
            }
        }

        let report = HeatMapCensus::snapshot();
        Self::print(&report, matches, level, minutes);
        if let Ok(path) = env::var("OF_HEAT_JSON") {
            Self::dump_json(&report, &path);
        }
    }

    /// Slots that actually took the field, in the engine's own order.
    fn live_slots(report: &core::heatmap_diag::HeatReport) -> Vec<usize> {
        report
            .positions
            .iter()
            .filter(|p| p.samples > 0)
            .map(|p| p.position)
            .collect()
    }

    fn label(index: usize) -> &'static str {
        PlayerPositionType::ALL[index].get_short_name()
    }

    /// The smallest area holding `share` of a map's time, m².
    ///
    /// This is what a heat map is actually claiming: not where a player
    /// went once, but how much grass his football is spread over. A
    /// midfielder covering 1,000 m² with half his time is holding a
    /// position; one covering 3,000 m² with the same half is following
    /// the ball.
    fn area(grid: &[u64], share: f64) -> f32 {
        let total: u64 = grid.iter().sum();
        if total == 0 {
            return 0.0;
        }
        let mut sorted: Vec<u64> = grid.iter().copied().filter(|v| *v > 0).collect();
        sorted.sort_unstable_by(|a, b| b.cmp(a));
        let target = total as f64 * share;
        let mut acc = 0.0;
        let mut cells = 0usize;
        for v in sorted {
            acc += v as f64;
            cells += 1;
            if acc >= target {
                break;
            }
        }
        cells as f32 * (Self::PITCH_AREA / heat::CELLS as f32)
    }

    /// Share of a map's time in its single hottest cell.
    fn peak(grid: &[u64]) -> f32 {
        let total: u64 = grid.iter().sum();
        if total == 0 {
            return 0.0;
        }
        grid.iter().copied().max().unwrap_or(0) as f32 / total as f32
    }

    /// Cosine similarity of two maps. 1.00 means the two men occupied the
    /// same grass in the same proportions — which for two DIFFERENT slots
    /// means the shape has no roles in it.
    fn cosine(a: &[u64], b: &[u64]) -> f32 {
        let mut dot = 0.0f64;
        let mut na = 0.0f64;
        let mut nb = 0.0f64;
        for (x, y) in a.iter().zip(b.iter()) {
            let (x, y) = (*x as f64, *y as f64);
            dot += x * y;
            na += x * x;
            nb += y * y;
        }
        if na == 0.0 || nb == 0.0 {
            return 0.0;
        }
        (dot / (na.sqrt() * nb.sqrt())) as f32
    }

    /// Pearson correlation of a map with the BALL's map, over every cell.
    ///
    /// The single most diagnostic number in this block. A real player's
    /// occupancy correlates with the ball's only loosely — he has a
    /// position and the ball visits it — so 0.3-0.5 is a healthy figure
    /// for a midfielder and lower for everybody else. A side whose every
    /// slot reads 0.8+ is not playing a shape; it is orbiting the ball.
    fn correlation(a: &[u64], b: &[u64]) -> f32 {
        let n = a.len().min(b.len()) as f64;
        if n == 0.0 {
            return 0.0;
        }
        let ma = a.iter().map(|v| *v as f64).sum::<f64>() / n;
        let mb = b.iter().map(|v| *v as f64).sum::<f64>() / n;
        let mut cov = 0.0;
        let mut va = 0.0;
        let mut vb = 0.0;
        for (x, y) in a.iter().zip(b.iter()) {
            let dx = *x as f64 - ma;
            let dy = *y as f64 - mb;
            cov += dx * dy;
            va += dx * dx;
            vb += dy * dy;
        }
        if va == 0.0 || vb == 0.0 {
            return 0.0;
        }
        (cov / (va.sqrt() * vb.sqrt())) as f32
    }

    fn print(report: &core::heatmap_diag::HeatReport, matches: usize, level: u8, minutes: u64) {
        let slots = Self::live_slots(report);
        let seconds = report.samples as f64 * 0.05;

        println!();
        println!(
            "=== THERMAL MAP ({} match(es), level {}, {}) ===",
            matches,
            level,
            HarnessTactic::label()
        );
        println!(
            "  {} instants sampled at 20 Hz = {:.1} min of football{}",
            report.samples,
            seconds / 60.0,
            if minutes == 0 {
                String::new()
            } else {
                format!(" (window: first {} min of each match)", minutes)
            }
        );
        println!(
            "  frame: x = 0 own goal line … {:.0} m the goal he attacks;  y = 0 … {:.0} m touchline to touchline",
            heat::PITCH_LENGTH_M,
            heat::PITCH_WIDTH_M
        );
        println!("  both sides folded to attack RIGHT (180° rotation, so flanks survive)");

        // ── where each slot lives ─────────────────────────────────────
        println!();
        println!("--- WHERE EACH SLOT LIVES (all play) ---");
        println!(
            "  {:<5} {:>7} {:>7} {:>7} {:>7} {:>8} {:>8} {:>7} {:>8} {:>7}",
            "slot", "mean x", "sd x", "mean y", "sd y", "A50 m²", "A95 m²", "peak%", "ball m", "<15m%"
        );
        for slot in &slots {
            let p = &report.positions[*slot];
            let grid = &p.grid[heat::PHASE_ALL];
            println!(
                "  {:<5} {:>7.1} {:>7.1} {:>7.1} {:>7.1} {:>8.0} {:>8.0} {:>6.1}% {:>8.1} {:>6.0}%",
                Self::label(*slot),
                p.mean_x,
                p.sd_x,
                p.mean_y,
                p.sd_y,
                Self::area(grid, 0.50),
                Self::area(grid, 0.95),
                Self::peak(grid) * 100.0,
                p.ball_gap,
                p.near_ball * 100.0,
            );
        }
        println!(
            "  real: a back four averages 30-40 m from its own goal and a striker 65-78; sd x 11-16 m,"
        );
        println!(
            "        sd y 7-12 m; A50 ≈ 700-1,200 m² (10-17% of the pitch), A95 ≈ 2,500-4,000 m²;"
        );
        println!(
            "        mean distance to the ball 25-35 m, and 20-30% of a match spent inside 15 m of it"
        );

        // ── the thirds ────────────────────────────────────────────────
        println!();
        println!("--- HOW THE TIME DIVIDES ---");
        println!(
            "  {:<5} {:>8} {:>9} {:>9} {:>9} {:>8} {:>9}",
            "slot", "own ½%", "f.third%", "own box%", "opp box%", "wide%", "touchl.%"
        );
        for slot in &slots {
            let p = &report.positions[*slot];
            println!(
                "  {:<5} {:>7.1}% {:>8.1}% {:>8.1}% {:>8.1}% {:>7.1}% {:>8.1}%",
                Self::label(*slot),
                p.own_half * 100.0,
                p.final_third * 100.0,
                p.own_box * 100.0,
                p.opp_box * 100.0,
                p.wide * 100.0,
                p.touchline * 100.0,
            );
        }
        println!(
            "  real: a centre-back spends 70-80% in his own half and a striker 20-30%; a winger is"
        );
        println!(
            "        outside the width of the box (wide%) 45-65% of the time and within 10 m of his"
        );
        println!("        touchline 25-40%, a centre-back under 10%");

        // ── the two phases ────────────────────────────────────────────
        println!();
        println!("--- IN POSSESSION vs OUT OF IT ---");
        println!(
            "  {:<5} {:>9} {:>9} {:>9}   {}",
            "slot", "x (ball)", "x (no b.)", "shift", "map cosine(in, out)"
        );
        for slot in &slots {
            let p = &report.positions[*slot];
            println!(
                "  {:<5} {:>9.1} {:>9.1} {:>+9.1}   {:>10.3}",
                Self::label(*slot),
                p.poss_x,
                p.oop_x,
                p.poss_x - p.oop_x,
                Self::cosine(&p.grid[heat::PHASE_IN_POSSESSION], &p.grid[heat::PHASE_OUT_OF_POSSESSION]),
            );
        }
        println!(
            "  real: every outfielder's mean position moves UP the pitch with the ball — a full-back"
        );
        println!(
            "        +10..15 m, a centre-back +6..10, a midfielder +8..14, a striker +6..12; the two"
        );
        println!("        maps are visibly different pictures, cosine ≈ 0.5-0.8");

        // ── role identity ─────────────────────────────────────────────
        println!();
        println!("--- ROLE IDENTITY: are these eleven different maps? ---");
        let outfield: Vec<usize> = slots
            .iter()
            .copied()
            .filter(|s| !PlayerPositionType::ALL[*s].is_goalkeeper())
            .collect();
        let mut pair_sum = 0.0f32;
        let mut pair_n = 0u32;
        for (i, a) in outfield.iter().enumerate() {
            for b in outfield.iter().skip(i + 1) {
                pair_sum += Self::cosine(
                    &report.positions[*a].grid[heat::PHASE_ALL],
                    &report.positions[*b].grid[heat::PHASE_ALL],
                );
                pair_n += 1;
            }
        }
        println!(
            "  mean cosine between the {} outfield maps: {:.3}   (real ≈ 0.20-0.35 — a left-back and",
            outfield.len(),
            pair_sum / pair_n.max(1) as f32
        );
        println!("  a right-back share almost no grass, a striker and a centre-back none at all)");
        println!();
        println!("  {:<5}{}", "", outfield.iter().map(|s| format!("{:>6}", Self::label(*s))).collect::<Vec<_>>().join(""));
        for a in &outfield {
            let row: String = outfield
                .iter()
                .map(|b| {
                    format!(
                        "{:>6.2}",
                        Self::cosine(
                            &report.positions[*a].grid[heat::PHASE_ALL],
                            &report.positions[*b].grid[heat::PHASE_ALL],
                        )
                    )
                })
                .collect();
            println!("  {:<5}{}", Self::label(*a), row);
        }

        println!();
        println!("  …and each slot's map against the BALL's own map (Pearson r over every cell):");
        let mut ball_row = String::new();
        for slot in &slots {
            ball_row.push_str(&format!(
                "  {} {:.2}",
                Self::label(*slot),
                Self::correlation(&report.positions[*slot].grid[heat::PHASE_ALL], &report.ball)
            ));
        }
        println!("   {}", ball_row.trim());
        println!(
            "  real: 0.2-0.5 for a central midfielder, under 0.3 for everybody else. Anything near"
        );
        println!("  1.0 says the man is not holding a position — he is a function of the ball.");

        // ── team shape ────────────────────────────────────────────────
        println!();
        println!("--- TEAM SHAPE (ten outfielders as a body) ---");
        println!(
            "  {:<14} {:>8} {:>8} {:>9} {:>9} {:>9} {:>8}",
            "phase", "length", "width", "centroid", "deepest", "highest", "swarm"
        );
        for (i, name) in ["all play", "in possession", "out of poss."].iter().enumerate() {
            let s = report.shape[i];
            println!(
                "  {:<14} {:>7.1} {:>8.1} {:>9.1} {:>9.1} {:>9.1} {:>8.2}",
                name, s.length, s.width, s.centroid_x, s.deepest, s.highest, s.swarm
            );
        }
        println!(
            "  real: length 30-40 m, width 45-58 m in possession and 28-38 m out of it; the DEEPEST"
        );
        println!(
            "        outfielder sits 18-28 m from his own goal and the HIGHEST 65-80; 4-6 of the ten"
        );
        println!("        are within 15 m of the ball");

        // ── the maps themselves ───────────────────────────────────────
        println!();
        println!("--- THE MAPS ---");
        println!(
            "  every map drawn attacking RIGHT; ' ' none  '.' rare  ':' some  '+' often  '*' heavy  '#' peak"
        );
        println!("  furniture: '|' the two box edges and the halfway line");
        Self::render(&report.ball, "THE BALL");
        let mut team: Vec<u64> = vec![0; heat::CELLS];
        for slot in &outfield {
            for (t, v) in team.iter_mut().zip(&report.positions[*slot].grid[heat::PHASE_ALL]) {
                *t += *v;
            }
        }
        Self::render(&team, "ALL TEN OUTFIELDERS");
        for slot in &slots {
            let p = &report.positions[*slot];
            Self::render(
                &p.grid[heat::PHASE_ALL],
                &format!(
                    "{}  —  mean ({:.0}, {:.0}) m, A50 {:.0} m², r(ball) {:.2}",
                    Self::label(*slot),
                    p.mean_x,
                    p.mean_y,
                    Self::area(&p.grid[heat::PHASE_ALL], 0.5),
                    Self::correlation(&p.grid[heat::PHASE_ALL], &report.ball),
                ),
            );
        }
    }

    /// One map, 84 columns by 14 printed lines.
    fn render(grid: &[u64], title: &str) {
        let cols = heat::COLS;
        let rows = heat::ROWS / Self::ROW_MERGE;
        let mut merged = vec![0u64; cols * rows];
        for (cell, v) in grid.iter().enumerate() {
            let row = (cell / cols) / Self::ROW_MERGE;
            merged[row.min(rows - 1) * cols + cell % cols] += *v;
        }
        let peak = merged.iter().copied().max().unwrap_or(1).max(1) as f64;
        // Box edges and the halfway line, in columns of this map.
        let box_near = (16.5 / heat::PITCH_LENGTH_M as f64 * cols as f64) as usize;
        let box_far = (88.5 / heat::PITCH_LENGTH_M as f64 * cols as f64) as usize;
        let half = cols / 2;

        println!();
        println!("  {}", title);
        for row in 0..rows {
            let mut line = String::with_capacity(cols + 4);
            line.push_str("   ");
            for col in 0..cols {
                let v = merged[row * cols + col] as f64 / peak;
                let ch = if v <= 0.0 {
                    if col == half || col == box_near || col == box_far {
                        '|'
                    } else {
                        ' '
                    }
                } else if v < 0.02 {
                    '.'
                } else if v < 0.06 {
                    ':'
                } else if v < 0.16 {
                    '+'
                } else if v < 0.40 {
                    '*'
                } else {
                    '#'
                };
                line.push(ch);
            }
            println!("{}", line);
        }
    }

    /// Every grid, as JSON, so the maps can be drawn where there are
    /// pixels rather than characters.
    fn dump_json(report: &core::heatmap_diag::HeatReport, path: &str) {
        let mut out = String::from("{\n");
        out.push_str(&format!(
            "  \"cols\": {}, \"rows\": {}, \"pitch\": [{}, {}], \"samples\": {},\n",
            heat::COLS,
            heat::ROWS,
            heat::PITCH_LENGTH_M,
            heat::PITCH_WIDTH_M,
            report.samples
        ));
        out.push_str(&format!(
            "  \"ball\": [{}],\n",
            report
                .ball
                .iter()
                .map(|v| v.to_string())
                .collect::<Vec<_>>()
                .join(",")
        ));
        out.push_str("  \"slots\": [\n");
        let slots = Self::live_slots(report);
        for (i, slot) in slots.iter().enumerate() {
            let p = &report.positions[*slot];
            out.push_str(&format!(
                "    {{\"name\": \"{}\", \"samples\": {}, \"mean\": [{:.2}, {:.2}], \"sd\": [{:.2}, {:.2}], \
                 \"ball_gap\": {:.2}, \"near_ball\": {:.4}, \"own_half\": {:.4}, \"final_third\": {:.4}, \
                 \"own_box\": {:.4}, \"opp_box\": {:.4}, \"wide\": {:.4}, \"touchline\": {:.4}, \
                 \"poss_x\": {:.2}, \"oop_x\": {:.2}, \"all\": [{}], \"in\": [{}], \"out\": [{}]}}{}\n",
                Self::label(*slot),
                p.samples,
                p.mean_x,
                p.mean_y,
                p.sd_x,
                p.sd_y,
                p.ball_gap,
                p.near_ball,
                p.own_half,
                p.final_third,
                p.own_box,
                p.opp_box,
                p.wide,
                p.touchline,
                p.poss_x,
                p.oop_x,
                p.grid[heat::PHASE_ALL].iter().map(|v| v.to_string()).collect::<Vec<_>>().join(","),
                p.grid[heat::PHASE_IN_POSSESSION].iter().map(|v| v.to_string()).collect::<Vec<_>>().join(","),
                p.grid[heat::PHASE_OUT_OF_POSSESSION].iter().map(|v| v.to_string()).collect::<Vec<_>>().join(","),
                if i + 1 == slots.len() { "" } else { "," }
            ));
        }
        out.push_str("  ],\n  \"shape\": [\n");
        for (i, s) in report.shape.iter().enumerate() {
            out.push_str(&format!(
                "    {{\"samples\": {}, \"length\": {:.2}, \"width\": {:.2}, \"centroid\": {:.2}, \"deepest\": {:.2}, \"highest\": {:.2}, \"swarm\": {:.3}}}{}\n",
                s.samples,
                s.length,
                s.width,
                s.centroid_x,
                s.deepest,
                s.highest,
                s.swarm,
                if i + 1 == report.shape.len() { "" } else { "," }
            ));
        }
        out.push_str("  ]\n}\n");
        match std::fs::write(path, out) {
            Ok(()) => println!("\n  grids written to {}", path),
            Err(e) => eprintln!("  could not write {}: {}", path, e),
        }
    }
}
