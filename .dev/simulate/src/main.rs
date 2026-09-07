//! Headless driver for `FootballSimulator::simulate` — the whole-world
//! daily tick.
//!
//! This is the "simple simulation" loop that used to only exist inside the
//! web crate's `POST /api/game/process` handler (`ProcessingRun::execute`).
//! Lifting it into a standalone binary lets a sampling profiler see the
//! simulator graph directly, with no HTTP, no tokio worker pool, and no
//! shared-state locking in the way.
//!
//! Build it with the `match-stub` feature (on by default) so the match
//! engine collapses to a 0-0 result: the trace then shows the graph that
//! WRAPS the engine — squad/roster maintenance, transfers and the
//! free-agent market, awards, index rebuilds, career-history snapshots,
//! and the national/global competition passes — instead of the AI hot
//! path (which `.dev/match` already covers).
//!
//! Usage:
//!   cargo build --profile profiling
//!   ./target/profiling/dev_simulate [days]          # default 60
//!   ./target/profiling/dev_simulate bench [days]    # same; muscle memory
//!
//! Profile it:
//!   samply record --save-only -o prof.json.gz -r 4000 \
//!       ./target/profiling/dev_simulate 60

use core::PlayerFieldPositionGroup;
use core::club::staff::perception::{
    AbilityEstimator, CoachEye, EstimationContext, PotentialEstimator,
};
use core::r#match::FieldSquad;
use core::utils::DateUtils;
use core::{
    FootballSimulator, PerformanceProfiler, SimulationResult, SimulatorData,
    TeamType,
};
use database::{DatabaseGenerator, DatabaseLoader};
use mimalloc::MiMalloc;

/// Windows' system heap serialises concurrent alloc/free behind a global
/// lock; under the world sim's 32-thread rayon fan-out that lock — not CPU
/// or parallelism — is the dominant cost. mimalloc's per-thread heaps
/// remove the contention so the parallel phases actually scale.
#[global_allocator]
static GLOBAL: MiMalloc = MiMalloc;
use env_logger::Env;
use std::collections::HashMap;
use std::future::Future;
use std::pin::pin;
use std::task::{Context, Poll, Waker};
use std::time::Instant;

/// Default number of simulated days when none is passed. Long enough that
/// the daily tick dominates the one-off world generation in a CPU trace,
/// and that at least a couple of weekly/monthly periodic sub-passes fire
/// (Monday awards, month-start rankings) so their cost shows up too.
const DEFAULT_DAYS: u32 = 60;

/// Fewest matches a competition needs before the goal census ranks it.
/// Below this the goals-per-match column is one weekend's variance.
const MIN_CENSUS_MATCHES: u32 = 30;

/// Per-competition goal census — the instrument for "does the match
/// engine play the same football at every level of the pyramid".
///
/// The `.dev/match` harness can only ever answer that question about its
/// OWN squads: `make_squad_simple(level)` builds eleven clones of one
/// synthetic skill level, so a level sweep there measures the engine's
/// response to a uniform skill dial and not to a real squad. This one
/// reads the world's actual clubs — real players, real spread, real
/// keepers, real tactics — and buckets every match the day tick produced
/// by the competition it was played in. Divisions differ by squad quality
/// and by nothing else the harness controls, so a goals-per-match column
/// that walks with the league's strength is the engine responding to
/// PLAYER QUALITY, which is exactly the reported symptom.
///
/// Only meaningful with the real engine: under the default `match-stub`
/// feature every result is 0-0 and the table prints zeros.
#[derive(Default)]
struct LeagueGoalCensus {
    rows: HashMap<String, CensusRow>,
}

#[derive(Default)]
struct CensusRow {
    matches: u32,
    goals: u32,
    home_goals: u32,
    draws: u32,
    nil_nil: u32,
    /// Matches with four or more goals in them — the "3-2 every week"
    /// end of the complaint.
    high_scoring: u32,
    /// Sum of squares, so the table can print variance/mean: an engine
    /// stuck on 1-0 and one that spreads properly can share a mean.
    goals_sq: u32,
}

impl LeagueGoalCensus {
    fn record(&mut self, result: &SimulationResult) {
        for m in &result.match_results {
            if m.friendly {
                continue;
            }
            let row = self.rows.entry(m.league_slug.clone()).or_default();
            let (h, a) = (
                m.score.home_team.get() as u32,
                m.score.away_team.get() as u32,
            );
            let total = h + a;
            row.matches += 1;
            row.goals += total;
            row.goals_sq += total * total;
            row.home_goals += h;
            row.draws += (h == a) as u32;
            row.nil_nil += (total == 0) as u32;
            row.high_scoring += (total >= 4) as u32;
        }
    }

    /// `min_matches` keeps single-fixture cup rounds out of the ranking —
    /// one match is not a rate.
    fn print(&self, min_matches: u32) {
        let mut rows: Vec<(&String, &CensusRow)> = self
            .rows
            .iter()
            .filter(|(_, r)| r.matches >= min_matches)
            .collect();
        if rows.is_empty() {
            println!("\nno competition reached {min_matches} matches — nothing to rank");
            return;
        }
        rows.sort_by(|a, b| {
            let ga = a.1.goals as f64 / a.1.matches as f64;
            let gb = b.1.goals as f64 / b.1.matches as f64;
            gb.partial_cmp(&ga).unwrap()
        });
        println!(
            "\n--- GOALS BY COMPETITION ({} with >= {min_matches} matches) ---",
            rows.len()
        );
        println!(
            "{:<38} {:>7} {:>8} {:>8} {:>7} {:>7} {:>7} {:>7}",
            "competition", "matches", "goals/m", "var/mean", "draws", "0-0", "4+", "home%"
        );
        for (slug, r) in rows {
            let n = r.matches as f64;
            let mean = r.goals as f64 / n;
            let var = r.goals_sq as f64 / n - mean * mean;
            println!(
                "{:<38} {:>7} {:>8.2} {:>8.2} {:>6.1}% {:>6.1}% {:>6.1}% {:>6.1}%",
                slug,
                r.matches,
                mean,
                if mean > 0.0 { var / mean } else { 0.0 },
                r.draws as f64 / n * 100.0,
                r.nil_nil as f64 / n * 100.0,
                r.high_scoring as f64 / n * 100.0,
                if r.goals > 0 {
                    r.home_goals as f64 / r.goals as f64 * 100.0
                } else {
                    0.0
                },
            );
        }
    }
}

/// Owns a generated world and ticks it one simulated day at a time. All
/// the harness plumbing (generation, the async driver, per-day timing)
/// lives here so `main` is just argument parsing.
struct SimHarness {
    data: SimulatorData,
}

impl SimHarness {
    /// Load the embedded database and generate a full world — the exact
    /// two steps the app runs at startup (`src/main.rs`: load → generate).
    /// The database is baked into the binary via `include_bytes!`, so this
    /// needs no working directory or data files.
    fn generate() -> Self {
        let database = DatabaseLoader::load();
        let data = DatabaseGenerator::generate(&database);
        SimHarness { data }
    }

    /// Tick one simulated day and return that day's result.
    ///
    /// `FootballSimulator::simulate` is declared `async` but never awaits
    /// an I/O point — it drives rayon internally and the future is ready on
    /// the first poll. So a no-op-waker `block_on` completes it in a single
    /// step; no tokio runtime, no dispatcher registration. With no
    /// `MatchDispatcherRegistry` installed, the engine pool falls back to
    /// the local rayon path, which under `match-stub` returns 0-0 stubs.
    fn tick(&mut self) -> SimulationResult {
        Self::block_on(FootballSimulator::simulate(&mut self.data))
    }

    /// Simulate `days` ticks, printing a per-day timing line and a final
    /// summary. Timing is wall-clock per tick; a single-threaded stall
    /// shows up here as a heavy day and, in the CPU trace, as self-time
    /// pinned to the main thread while the rayon workers sit idle.
    fn bench(&mut self, days: u32) {
        let start_date = self.data.date.date();
        let overall = Instant::now();
        let mut total_matches: u64 = 0;
        let mut slowest_day = 0u32;
        let mut slowest_ms = 0.0f64;
        let mut census = LeagueGoalCensus::default();
        let mut world = WorldMatchCensus::default();
        // Taken before the first tick: the world exactly as the database
        // hydrated it, which is the state the youth pathway has to heal.
        let youth_before = YouthSquadCensus::take(&self.data);

        for day in 1..=days {
            let tick_start = Instant::now();
            let result = self.tick();
            let ms = tick_start.elapsed().as_secs_f64() * 1000.0;

            let matches = result.match_results.len();
            census.record(&result);
            world.record(&result);
            total_matches += matches as u64;
            if ms > slowest_ms {
                slowest_ms = ms;
                slowest_day = day;
            }

            println!(
                "day {day:>4}  {date}  {ms:>9.2} ms  matches={matches}",
                date = self.data.date.date(),
            );
        }

        let total_ms = overall.elapsed().as_secs_f64() * 1000.0;
        println!(
            "\n{days} days  {start} → {end}\n\
             total {total_ms:.1} ms  mean {mean:.2} ms/day  \
             slowest day {slowest_day} ({slowest_ms:.2} ms)  \
             matches {total_matches}",
            start = start_date,
            end = self.data.date.date(),
            mean = total_ms / days as f64,
        );

        // At least a full round of fixtures before a competition earns a
        // row: one match is a scoreline, not a rate.
        // Per-phase wall/CPU breakdown of the tick — only when OF_SIM_PROF is
        // set. `cores` is the column that matters: it is CPU/wall, i.e. how
        // wide each phase actually ran.
        PerformanceProfiler::report(&format!("{days}d"));

        census.print(MIN_CENSUS_MATCHES);
        world.print();
        youth_before.print(&YouthSquadCensus::take(&self.data));
    }

    /// Minimal executor for a future guaranteed ready on its first poll
    /// (see `tick`). A no-op waker is sound precisely because the future
    /// never registers interest in being woken; the loop guards against a
    /// future that yields anyway rather than spinning the CPU forever
    /// unintentionally.
    fn block_on<F: Future>(future: F) -> F::Output {
        let mut future = pin!(future);
        let waker = Waker::noop();
        let mut cx = Context::from_waker(waker);
        loop {
            if let Poll::Ready(output) = future.as_mut().poll(&mut cx) {
                return output;
            }
            std::hint::spin_loop();
        }
    }
}

fn main() {
    // Quiet by default (the database loader and simulator emit `info!`
    // lines that would swamp the per-day timing); raise with RUST_LOG.
    env_logger::Builder::from_env(Env::default().default_filter_or("warn")).init();

    // Accept `dev_simulate [days]` or `dev_simulate bench [days]`: take the
    // first argument that parses as a day count so both spellings work.
    let days = std::env::args()
        .skip(1)
        .find_map(|arg| arg.parse::<u32>().ok())
        .unwrap_or(DEFAULT_DAYS);

    eprintln!("generating world…");
    let gen_start = Instant::now();
    let mut harness = SimHarness::generate();
    eprintln!(
        "world generated in {:.2} s — simulating {days} days",
        gen_start.elapsed().as_secs_f64(),
    );

    // `dev_simulate stars [team-slug] [days]` — the potential-star census:
    // how the coach's displayed potential stars relate to the hidden PA
    // across the day-0 world (and after `days` ticks when given).
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().map(|a| a == "stars").unwrap_or(false) {
        let slug = args.iter().skip(1).find(|a| a.parse::<u32>().is_err()).cloned();
        let ticks = args.iter().skip(1).find_map(|a| a.parse::<u32>().ok()).unwrap_or(0);
        for _ in 0..ticks {
            harness.tick();
        }
        StarCensus::take(&harness.data).print();
        if let Some(slug) = slug {
            StarCensus::print_team(&harness.data, &slug);
        }
        return;
    }

    harness.bench(days);
}

/// Every counter the census keeps, for one bucket of team-matches.
///
/// A bucket is a set of team-matches — the whole world, one competition,
/// or every side that lined up in one shape. Keeping the counters in their
/// own struct is what lets the same numbers be sliced three ways without
/// three copies of the accumulation code.
#[derive(Default, Clone)]
struct MatchStatTotals {
    /// Team-matches counted. Every rate below divides by this.
    team_matches: u32,
    goals: u64,
    shots: u64,
    on_target: u64,
    saves: u64,
    shots_faced: u64,
    xg: f64,
    passes_attempted: u64,
    passes_completed: u64,
    tackles: u64,
    interceptions: u64,
    fouls: u64,
    key_passes: u64,
    crosses_attempted: u64,
    crosses_completed: u64,
    dribbles_attempted: u64,
    dribbles_completed: u64,
    miscontrols: u64,
    blocks: u64,
    clearances: u64,
    /// Minutes summed over the starting eleven — 990 a team-match when
    /// every starter plays the full 90. Falls below that with
    /// substitutions and red cards, and is the check that the world's
    /// matches are the same LENGTH as the harness's.
    starter_minutes: u64,
    subs_used: u64,
    /// Starting-eleven shape, counted off `starter_slots` so it reads the
    /// slot the coach picked rather than the player's natural position.
    /// The harness fields a hardcoded 4-4-2; the world fields whatever
    /// each club's tactics say, and a one-striker shape is a different
    /// quantity of attacking football.
    start_def: u64,
    start_mid: u64,
    start_fwd: u64,
    /// Goals and shots by the scorer's position group, so the world can be
    /// compared against the harness's GOALS BY LINE.
    goals_by_group: [u64; 4],
    shots_by_group: [u64; 4],
}

impl MatchStatTotals {
    /// Fold one side of one match in. `squad` is that side's `FieldSquad`;
    /// the stats come out of the shared `player_stats` map.
    fn add_team_match(&mut self, details: &core::r#match::MatchResultRaw, squad: &FieldSquad) {
        self.team_matches += 1;
        self.subs_used += squad.substitutes_used.len() as u64;
        for (_, slot) in &squad.starter_slots {
            match slot.position_group() {
                PlayerFieldPositionGroup::Defender => self.start_def += 1,
                PlayerFieldPositionGroup::Midfielder => self.start_mid += 1,
                PlayerFieldPositionGroup::Forward => self.start_fwd += 1,
                PlayerFieldPositionGroup::Goalkeeper => {}
            }
        }
        for id in &squad.main {
            if let Some(s) = details.player_stats.get(id) {
                self.starter_minutes += s.minutes_played as u64;
            }
        }
        for id in squad.main.iter().chain(&squad.substitutes) {
            let Some(s) = details.player_stats.get(id) else {
                continue;
            };
            self.goals += s.goals as u64;
            self.shots += s.shots_total as u64;
            self.on_target += s.shots_on_target as u64;
            self.saves += s.saves as u64;
            self.shots_faced += s.shots_faced as u64;
            self.xg += s.xg as f64;
            self.passes_attempted += s.passes_attempted as u64;
            self.passes_completed += s.passes_completed as u64;
            self.tackles += s.tackles as u64;
            self.interceptions += s.interceptions as u64;
            self.fouls += s.fouls as u64;
            self.key_passes += s.key_passes as u64;
            self.crosses_attempted += s.crosses_attempted as u64;
            self.crosses_completed += s.crosses_completed as u64;
            self.dribbles_attempted += s.attempted_dribbles as u64;
            self.dribbles_completed += s.successful_dribbles as u64;
            self.miscontrols += s.miscontrols as u64;
            self.blocks += s.blocks as u64;
            self.clearances += s.clearances as u64;
            let g = s.position_group.index();
            self.goals_by_group[g] += s.goals as u64;
            self.shots_by_group[g] += s.shots_total as u64;
        }
    }

    fn per(&self, total: u64) -> f64 {
        if self.team_matches == 0 {
            0.0
        } else {
            total as f64 / self.team_matches as f64
        }
    }

    fn pct(num: u64, den: u64) -> f64 {
        if den == 0 {
            0.0
        } else {
            num as f64 / den as f64 * 100.0
        }
    }

    /// The full block, printed once for the world as a whole. Deliberately
    /// mirrors `dev_match stats`'s AGGREGATE section line for line so the
    /// two can be read side by side without re-deriving anything.
    fn print_full(&self) {
        println!("  goals               {:>8.2}", self.per(self.goals));
        println!("  shots               {:>8.2}", self.per(self.shots));
        println!(
            "  on target           {:>8.2}   {:>5.1}% of shots",
            self.per(self.on_target),
            Self::pct(self.on_target, self.shots)
        );
        println!(
            "  saves               {:>8.2}   {:>5.1}% of shots faced",
            self.per(self.saves),
            Self::pct(self.saves, self.shots_faced)
        );
        println!(
            "  xg                  {:>8.2}   {:>5.1} shots per xG",
            self.xg / self.team_matches.max(1) as f64,
            if self.xg > 0.0 {
                self.shots as f64 / self.xg
            } else {
                0.0
            }
        );
        println!(
            "  passes              {:>8.1}   {:>5.1}% completed",
            self.per(self.passes_attempted),
            Self::pct(self.passes_completed, self.passes_attempted)
        );
        println!("  tackles             {:>8.2}", self.per(self.tackles));
        println!(
            "  interceptions       {:>8.2}",
            self.per(self.interceptions)
        );
        println!("  fouls               {:>8.2}", self.per(self.fouls));
        println!("  key passes          {:>8.2}", self.per(self.key_passes));
        println!(
            "  crosses             {:>8.2}   {:>5.1}% completed",
            self.per(self.crosses_attempted),
            Self::pct(self.crosses_completed, self.crosses_attempted)
        );
        println!(
            "  dribbles            {:>8.2}   {:>5.1}% completed",
            self.per(self.dribbles_attempted),
            Self::pct(self.dribbles_completed, self.dribbles_attempted)
        );
        println!("  miscontrols         {:>8.2}", self.per(self.miscontrols));
        println!("  blocks              {:>8.2}", self.per(self.blocks));
        println!("  clearances          {:>8.2}", self.per(self.clearances));
        println!(
            "  starter minutes     {:>8.1}   (990 = eleven starters, full 90)",
            self.per(self.starter_minutes)
        );
        println!("  subs used           {:>8.2}", self.per(self.subs_used));
        println!(
            "  starting shape       {:>4.1} DEF / {:.1} MID / {:.1} FWD",
            self.per(self.start_def),
            self.per(self.start_mid),
            self.per(self.start_fwd)
        );
        let outfield_goals =
            self.goals_by_group[1] + self.goals_by_group[2] + self.goals_by_group[3];
        let outfield_shots =
            self.shots_by_group[1] + self.shots_by_group[2] + self.shots_by_group[3];
        println!(
            "  goals by line        DEF {:.1}% / MID {:.1}% / FWD {:.1}%   (real ~10 / 32 / 58)",
            Self::pct(self.goals_by_group[1], outfield_goals),
            Self::pct(self.goals_by_group[2], outfield_goals),
            Self::pct(self.goals_by_group[3], outfield_goals)
        );
        println!(
            "  shots by line        DEF {:.1}% / MID {:.1}% / FWD {:.1}%",
            Self::pct(self.shots_by_group[1], outfield_shots),
            Self::pct(self.shots_by_group[2], outfield_shots),
            Self::pct(self.shots_by_group[3], outfield_shots)
        );
    }

    /// One row of a breakdown table — the five numbers that say WHICH
    /// stage of the chance chain a bucket differs at.
    fn print_row(&self, label: &str) {
        println!(
            "{:<24} {:>7} {:>8.2} {:>8.2} {:>7.1}% {:>7.1}% {:>7.2} {:>6.1}",
            label,
            self.team_matches,
            self.per(self.goals),
            self.per(self.shots),
            Self::pct(self.on_target, self.shots),
            Self::pct(self.saves, self.shots_faced),
            self.xg / self.team_matches.max(1) as f64,
            self.per(self.start_fwd),
        );
    }

    fn row_header(title: &str) {
        println!("\n{title}");
        println!(
            "{:<24} {:>7} {:>8} {:>8} {:>8} {:>8} {:>7} {:>6}",
            "", "team-m", "goals", "shots", "on-tgt", "saved", "xg", "FWD"
        );
    }
}

/// Whole-match statistics census over the world's real fixtures — the
/// counterpart to `dev_match stats`, printing the SAME ratios off real
/// squads so the two instruments can be laid side by side.
///
/// [`LeagueGoalCensus`] above answers "how many goals, per competition".
/// It cannot answer "why", because a scoreline is the product of four
/// rates (chances created, shots taken, shots on target, shots saved) and
/// a single number cannot say which of them moved. This prints all four,
/// then slices them by the shape each side started in and by competition —
/// the two ways the world differs from the harness that a scoreline alone
/// cannot separate.
#[derive(Default)]
struct WorldMatchCensus {
    /// Fixtures counted (one per match).
    matches: u32,
    overall: MatchStatTotals,
    by_tactic: HashMap<&'static str, MatchStatTotals>,
    by_competition: HashMap<String, MatchStatTotals>,
}

impl WorldMatchCensus {
    /// Fewest team-matches a bucket needs before its row is printed. Below
    /// this the on-target and save columns are one afternoon's variance.
    const MIN_BUCKET: u32 = 60;

    fn record(&mut self, result: &SimulationResult) {
        for m in &result.match_results {
            if m.friendly {
                continue;
            }
            let Some(details) = m.details.as_ref() else {
                continue;
            };
            self.matches += 1;
            for squad in [&details.left_team_players, &details.right_team_players] {
                self.overall.add_team_match(details, squad);
                self.by_competition
                    .entry(m.league_slug.clone())
                    .or_default()
                    .add_team_match(details, squad);
                // The starting shape is recorded home/away; map it back to
                // this side by team id rather than by list order, because
                // the engine swaps left/right at half-time.
                let tactic = if squad.team_id == m.home_team_id {
                    details.starting_home_tactic
                } else {
                    details.starting_away_tactic
                };
                if let Some(t) = tactic {
                    self.by_tactic
                        .entry(t.display_name())
                        .or_default()
                        .add_team_match(details, squad);
                }
            }
        }
    }

    fn print(&self) {
        if self.overall.team_matches == 0 {
            println!("\nno non-friendly match carried details — nothing to census");
            return;
        }
        println!(
            "\n--- WORLD MATCH CENSUS ({} matches, per team-match) ---",
            self.matches
        );
        self.overall.print_full();

        MatchStatTotals::row_header("--- BY STARTING SHAPE ---");
        let mut shapes: Vec<(&&str, &MatchStatTotals)> = self
            .by_tactic
            .iter()
            .filter(|(_, t)| t.team_matches >= Self::MIN_BUCKET)
            .collect();
        shapes.sort_by(|a, b| b.1.team_matches.cmp(&a.1.team_matches));
        for (name, totals) in shapes {
            totals.print_row(name);
        }

        MatchStatTotals::row_header("--- BY COMPETITION (12 highest and 12 lowest scoring) ---");
        let mut comps: Vec<(&String, &MatchStatTotals)> = self
            .by_competition
            .iter()
            .filter(|(_, t)| t.team_matches >= Self::MIN_BUCKET)
            .collect();
        comps.sort_by(|a, b| {
            b.1.per(b.1.goals)
                .partial_cmp(&a.1.per(a.1.goals))
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        for (slug, totals) in comps.iter().take(12) {
            totals.print_row(slug);
        }
        if comps.len() > 24 {
            println!("{:<24} {:>7}", "  …", comps.len() - 24);
        }
        for (slug, totals) in comps.iter().skip(comps.len().saturating_sub(12)) {
            totals.print_row(slug);
        }
    }
}

/// Youth-squad census — the instrument for "can the world's age-restricted
/// teams actually field a side, and how long does it take them to get
/// there".
///
/// The shipped database carries real senior squads and almost no youth
/// records: a third of the U18 teams start a new world with nobody in
/// them at all. Whether that heals is not visible in any match statistic
/// (an eleven-a-side fixture is played whatever the roster looks like) so
/// it needs its own reading, taken over the same world the day tick is
/// already running. Take one before the first tick and one after the
/// last: the pair is the whole measurement.
#[derive(Default)]
struct YouthSquadCensus {
    teams: usize,
    empty: usize,
    below_eleven: usize,
    below_fourteen: usize,
    players: usize,
    /// Academy rosters behind those teams — the pool the youth squads are
    /// fed from, and the thing a runaway backfill would inflate.
    academies: usize,
    academy_players: usize,
    /// Every footballer in the world: senior squads, youth squads,
    /// academies, free agents. The guard rail. Nothing in the youth
    /// pathway is allowed to be a *source* — an academy that hands a boy
    /// up and invents a replacement the same morning reads here as a
    /// population that climbs and never settles, which is how the last
    /// two throughput regressions were caught.
    world_players: usize,
}

impl YouthSquadCensus {
    /// A football team is eleven players; eleven plus three substitutes
    /// is a squad that can play a fixture properly. Both numbers mirror
    /// `ClubAcademy::EMERGENCY_YOUTH_SIZE` / `_TARGET`.
    const FIELDING: usize = 11;
    const WORKING: usize = 14;

    fn take(data: &SimulatorData) -> Self {
        let mut c = YouthSquadCensus::default();
        c.world_players = data.free_agents.len();
        for continent in &data.continents {
            for country in &continent.countries {
                for club in &country.clubs {
                    c.academies += 1;
                    c.academy_players += club.academy.players.players.len();
                    c.world_players += club.academy.players.players.len();
                    for team in &club.teams.teams {
                        c.world_players += team.players.len();
                        if !team.team_type.is_youth() {
                            continue;
                        }
                        let n = team.players.len();
                        c.teams += 1;
                        c.players += n;
                        if n == 0 {
                            c.empty += 1;
                        }
                        if n < Self::FIELDING {
                            c.below_eleven += 1;
                        }
                        if n < Self::WORKING {
                            c.below_fourteen += 1;
                        }
                    }
                }
            }
        }
        c
    }

    fn pct(part: usize, whole: usize) -> f64 {
        if whole == 0 {
            0.0
        } else {
            part as f64 * 100.0 / whole as f64
        }
    }

    /// Print `self` (day 0) against `after` (last day) as one table, so
    /// the reading is the change rather than two absolute numbers the
    /// reader has to difference by hand.
    fn print(&self, after: &YouthSquadCensus) {
        println!("\nyouth squads (U18…U23)          day 0      end     change");
        let row = |label: &str, a: usize, b: usize| {
            println!("  {label:<26} {a:>7} {b:>8} {:>+10}", b as i64 - a as i64);
        };
        row("teams", self.teams, after.teams);
        row("  empty", self.empty, after.empty);
        row(
            "  cannot field XI (<11)",
            self.below_eleven,
            after.below_eleven,
        );
        row(
            "  under a working 14",
            self.below_fourteen,
            after.below_fourteen,
        );
        row("players on youth rosters", self.players, after.players);
        println!(
            "  {:<26} {:>6.1}% {:>7.1}%",
            "cannot field XI",
            Self::pct(self.below_eleven, self.teams),
            Self::pct(after.below_eleven, after.teams),
        );
        println!(
            "  {:<26} {:>7.1} {:>8.1}",
            "mean squad size",
            self.players as f64 / self.teams.max(1) as f64,
            after.players as f64 / after.teams.max(1) as f64,
        );
        row(
            "academy players (world)",
            self.academy_players,
            after.academy_players,
        );
        println!(
            "  {:<26} {:>7.1} {:>8.1}",
            "mean academy size",
            self.academy_players as f64 / self.academies.max(1) as f64,
            after.academy_players as f64 / after.academies.max(1) as f64,
        );
        row(
            "WORLD PLAYERS (all)",
            self.world_players,
            after.world_players,
        );
        println!(
            "  {:<26} {:>16.1}%",
            "world growth",
            Self::pct(after.world_players, self.world_players.max(1)) - 100.0,
        );
    }
}

/// Accuracy counters for one read model against the hidden PA.
#[derive(Default, Clone)]
struct ReadAccuracy {
    n: u32,
    exact: u32,
    within_half: u32,
    off_full: u32,
    under_full: u32,
    over_full: u32,
    shown: [u32; 6],
    sx: f64,
    sy: f64,
    sxx: f64,
    syy: f64,
    sxy: f64,
}

impl ReadAccuracy {
    fn add(&mut self, shown_halves: u8, truth_halves: u8, read: u8, pa: u8) {
        self.n += 1;
        let d = shown_halves as i16 - truth_halves as i16;
        if d == 0 {
            self.exact += 1;
        }
        if d.abs() <= 1 {
            self.within_half += 1;
        }
        if d.abs() >= 2 {
            self.off_full += 1;
        }
        if d <= -2 {
            self.under_full += 1;
        }
        if d >= 2 {
            self.over_full += 1;
        }
        self.shown[(shown_halves / 2) as usize] += 1;
        let (x, y) = (read as f64, pa as f64);
        self.sx += x;
        self.sy += y;
        self.sxx += x * x;
        self.syy += y * y;
        self.sxy += x * y;
    }

    fn corr(&self) -> f64 {
        let n = self.n as f64;
        if n < 2.0 {
            return 0.0;
        }
        let cov = self.sxy / n - (self.sx / n) * (self.sy / n);
        let vx = self.sxx / n - (self.sx / n).powi(2);
        let vy = self.syy / n - (self.sy / n).powi(2);
        if vx <= 0.0 || vy <= 0.0 {
            0.0
        } else {
            cov / (vx * vy).sqrt()
        }
    }

    fn pct(&self, c: u32) -> f64 {
        if self.n == 0 {
            0.0
        } else {
            c as f64 / self.n as f64 * 100.0
        }
    }

    fn line(&self, label: &str) -> String {
        format!(
            "{label:<22} {:>6} {:>5.2} {:>6.1}% {:>6.1}% {:>6.1}% {:>6.1}% {:>6.1}%",
            self.n,
            self.corr(),
            self.pct(self.exact),
            self.pct(self.within_half),
            self.pct(self.off_full),
            self.pct(self.under_full),
            self.pct(self.over_full),
        )
    }

    fn header() -> String {
        format!(
            "{:<22} {:>6} {:>5} {:>7} {:>7} {:>7} {:>7} {:>7}",
            "", "n", "corr", "exact", "<=half", ">=1*", "under", "over"
        )
    }
}

/// One age band of the potential-star census.
#[derive(Default, Clone)]
struct StarBand {
    n: u32,
    /// Players judged by the stub coach (vacant bench) -> observer-free read.
    stub_coach: u32,
    /// Hidden PA by whole stars, for comparison.
    truth: [u32; 6],
    /// Displayed CURRENT ability, whole stars.
    current: [u32; 6],
    /// Confusion of the LIVE read: truth star (row) x shown star (column).
    confusion: [[u32; 6]; 6],
    live: ReadAccuracy,
    proposed: ReadAccuracy,
    /// Proposed read split by the judging coach: JPP <=7 / 8..13 / >=14.
    proposed_by_jpp: [ReadAccuracy; 3],
    sum_pa: f64,
    sum_ca: f64,
    sum_visible: f64,
    sum_credible: f64,
    sum_ceiling: f64,
    sum_estimated: f64,
    sum_proposed: f64,
}

/// Potential-star census: the web's `PotentialStarsView` recomputed here
/// against every player in the world, next to the hidden PA it is never
/// allowed to read. Answers "does a 5-star boy show 5 stars to a coach who
/// can judge him?" with numbers instead of one roster page.
struct StarCensus {
    bands: [StarBand; 5],
}

impl StarCensus {
    const BAND_LABELS: [&'static str; 5] = ["<=17", "18-20", "21-23", "24-28", "29+"];
    const JPP_LABELS: [&'static str; 3] = ["JPP <=7", "JPP 8-13", "JPP >=14"];

    fn band(age: u8) -> usize {
        match age {
            0..=17 => 0,
            18..=20 => 1,
            21..=23 => 2,
            24..=28 => 3,
            _ => 4,
        }
    }

    fn jpp_bucket(jpp: u8) -> usize {
        match jpp {
            0..=7 => 0,
            8..=13 => 1,
            _ => 2,
        }
    }

    /// The web's `StarRating::from_ability_scale`: 1..200 -> 0..=10 halves.
    fn halves(value: u8) -> u8 {
        (((value as f32 / 200.0) * 10.0).round().clamp(0.0, 10.0) as u8).min(10)
    }

    /// `StarRating` orders by (full, half); the web floors potential at the
    /// current-ability stars with `Ord::max` on that pair, which is the same
    /// as max on halves.
    fn shown_halves(current: u8, credible: u8) -> u8 {
        Self::halves(credible).max(Self::halves(current))
    }

    fn take(data: &SimulatorData) -> Self {
        let now = data.date.date();
        let mut bands: [StarBand; 5] = Default::default();
        for continent in &data.continents {
            for country in &continent.countries {
                for club in &country.clubs {
                    for team in &club.teams.teams {
                        let coach = team.staffs.head_coach();
                        let is_main = team.team_type == TeamType::Main;
                        let jpp = coach.staff_attributes.knowledge.judging_player_potential;
                        for p in team.players.iter() {
                            let age = DateUtils::age(p.birth_date, now);
                            let b = &mut bands[Self::band(age)];
                            let pa = p.player_attributes.potential_ability;
                            let ca = p.player_attributes.current_ability;
                            let visible = PotentialEstimator::visible_ability(p);
                            let level = AbilityEstimator::observable_level(p);
                            let ceiling = PotentialEstimator::observable_ceiling(p, now);
                            let (credible, estimated, proposed) = if coach.id == 0 {
                                b.stub_coach += 1;
                                (ceiling, ceiling, ceiling)
                            } else {
                                let ctx = EstimationContext {
                                    observation_count: 20,
                                    is_main_team: is_main,
                                    ..EstimationContext::default()
                                };
                                let e = PotentialEstimator::estimate_for_staff(p, coach, &ctx, now);
                                let eye = CoachEye::read(p, coach, &EstimationContext { observation_count: 20, is_main_team: is_main, ..EstimationContext::default() }, now);
                                (e.credible_potential, e.estimated_potential, eye)
                            };
                            let shown = Self::shown_halves(level, credible);
                            let shown_new = Self::shown_halves(level, proposed);
                            let truth = Self::halves(pa);
                            let cur = Self::halves(level);
                            b.n += 1;
                            b.truth[(truth / 2) as usize] += 1;
                            b.current[(cur / 2) as usize] += 1;
                            b.confusion[(truth / 2) as usize][(shown / 2) as usize] += 1;
                            b.live.add(shown, truth, credible, pa);
                            b.proposed.add(shown_new, truth, proposed, pa);
                            if coach.id != 0 {
                                b.proposed_by_jpp[Self::jpp_bucket(jpp)]
                                    .add(shown_new, truth, proposed, pa);
                            }
                            b.sum_pa += pa as f64;
                            b.sum_ca += ca as f64;
                            b.sum_visible += visible as f64;
                            b.sum_credible += credible as f64;
                            b.sum_ceiling += ceiling as f64;
                            b.sum_estimated += estimated as f64;
                            b.sum_proposed += proposed as f64;
                        }
                    }
                }
            }
        }
        StarCensus { bands }
    }

    fn row(label: &str, hist: &[u32; 6], n: u32) -> String {
        let pct = |c: u32| if n == 0 { 0.0 } else { c as f64 / n as f64 * 100.0 };
        format!(
            "{label:<14} {:>6.1}% {:>6.1}% {:>6.1}% {:>6.1}% {:>6.1}% {:>6.1}%",
            pct(hist[0]),
            pct(hist[1]),
            pct(hist[2]),
            pct(hist[3]),
            pct(hist[4]),
            pct(hist[5]),
        )
    }

    fn print(&self) {
        println!("\n--- POTENTIAL STARS vs HIDDEN PA (coach read with 20 observations) ---");
        println!(
            "{:<7} {:>7} {:>6} {:>7} {:>7} {:>7} {:>7} {:>7} {:>7} {:>7}",
            "age", "players", "stub%", "PA", "CA", "visible", "ceiling", "estim", "credib", "eye"
        );
        for (i, b) in self.bands.iter().enumerate() {
            let n = b.n.max(1) as f64;
            println!(
                "{:<7} {:>7} {:>5.1}% {:>7.1} {:>7.1} {:>7.1} {:>7.1} {:>7.1} {:>7.1} {:>7.1}",
                Self::BAND_LABELS[i],
                b.n,
                b.stub_coach as f64 / n * 100.0,
                b.sum_pa / n,
                b.sum_ca / n,
                b.sum_visible / n,
                b.sum_ceiling / n,
                b.sum_estimated / n,
                b.sum_credible / n,
                b.sum_proposed / n,
            );
        }
        for (i, b) in self.bands.iter().enumerate() {
            println!(
                "\n[{}] whole-star distribution (columns 0..5 stars), n={}",
                Self::BAND_LABELS[i],
                b.n
            );
            println!(
                "{:<14} {:>7} {:>7} {:>7} {:>7} {:>7} {:>7}",
                "", "0", "1", "2", "3", "4", "5"
            );
            println!("{}", Self::row("hidden PA", &b.truth, b.n));
            println!("{}", Self::row("shown cur", &b.current, b.n));
            println!("{}", Self::row("live pot", &b.live.shown, b.n));
            println!("{}", Self::row("eye pot", &b.proposed.shown, b.n));
            println!("{}", ReadAccuracy::header());
            println!("{}", b.live.line("live (credible)"));
            println!("{}", b.proposed.line("coach eye (proposed)"));
            for (j, acc) in b.proposed_by_jpp.iter().enumerate() {
                println!("{}", acc.line(&format!("  eye, {}", Self::JPP_LABELS[j])));
            }
            if i <= 1 {
                println!("live confusion: rows = hidden-PA stars, columns = shown potential stars (row %)");
                for (t, row) in b.confusion.iter().enumerate() {
                    let rn: u32 = row.iter().sum();
                    println!("{}", Self::row(&format!("PA {t}* n={rn}"), row, rn));
                }
            }
        }
    }

    fn print_team(data: &SimulatorData, slug: &str) {
        let now = data.date.date();
        for continent in &data.continents {
            for country in &continent.countries {
                for club in &country.clubs {
                    for team in &club.teams.teams {
                        if team.slug != slug {
                            continue;
                        }
                        let coach = team.staffs.head_coach();
                        let is_main = team.team_type == TeamType::Main;
                        // The same man with his judging_player_potential
                        // forced, so the sweep isolates the judge from the roster.
                        let mut weak = coach.clone();
                        weak.staff_attributes.knowledge.judging_player_potential = 3;
                        let mut elite = coach.clone();
                        elite.staff_attributes.knowledge.judging_player_potential = 18;
                        println!(
                            "\n--- {} ({:?}) coach id={} JPP={} JPA={} WwY={} ---",
                            team.name,
                            team.team_type,
                            coach.id,
                            coach.staff_attributes.knowledge.judging_player_potential,
                            coach.staff_attributes.knowledge.judging_player_ability,
                            coach.staff_attributes.coaching.working_with_youngsters,
                        );
                        for t in &club.teams.teams {
                            let room: Vec<String> = t
                                .staffs
                                .staffs
                                .iter()
                                .map(|s| {
                                    format!(
                                        "{:?}:{}",
                                        s.contract.as_ref().map(|c| c.position.clone()),
                                        s.staff_attributes.knowledge.judging_player_potential
                                    )
                                })
                                .collect();
                            println!("staff room {:?} (JPP): {}", t.team_type, room.join(", "));
                        }
                        println!(
                            "{:<22} {:>3} {:>4} {:>4} {:>4} {:>5} {:>5} {:>5} {:>5} {:>4} | {:>5} {:>5} {:>5} | {:>6} {:>6} {:>6}",
                            "name", "age", "PA", "CA", "vis", "level", "ceil", "estim", "cred",
                            "unc", "eye", "eye3", "eye18", "cur*", "live*", "eye*"
                        );
                        for p in team.players.iter() {
                            let age = DateUtils::age(p.birth_date, now);
                            let visible = PotentialEstimator::visible_ability(p);
                            let level = AbilityEstimator::observable_level(p);
                            let ceiling = PotentialEstimator::observable_ceiling(p, now);
                            let ctx = EstimationContext {
                                observation_count: 20,
                                is_main_team: is_main,
                                ..EstimationContext::default()
                            };
                            let e = PotentialEstimator::estimate_for_staff(p, coach, &ctx, now);
                            let credible = if coach.id == 0 {
                                ceiling
                            } else {
                                e.credible_potential
                            };
                            let eye = CoachEye::read(p, coach, &EstimationContext { observation_count: 20, is_main_team: is_main, ..EstimationContext::default() }, now);
                            let eye3 = CoachEye::read(p, &weak, &ctx, now);
                            let eye18 = CoachEye::read(p, &elite, &ctx, now);
                            let shown = Self::shown_halves(level, credible);
                            let shown_eye = Self::shown_halves(level, eye);
                            println!(
                                "{:<22} {:>3} {:>4} {:>4} {:>4} {:>5} {:>5} {:>5} {:>5} {:>4} | {:>5} {:>5} {:>5} | {:>6.1} {:>6.1} {:>6.1}",
                                p.full_name.display_last_name(),
                                age,
                                p.player_attributes.potential_ability,
                                p.player_attributes.current_ability,
                                visible,
                                level,
                                ceiling,
                                e.estimated_potential,
                                credible,
                                e.uncertainty,
                                eye,
                                eye3,
                                eye18,
                                Self::halves(level) as f32 / 2.0,
                                shown as f32 / 2.0,
                                shown_eye as f32 / 2.0,
                            );
                        }
                        return;
                    }
                }
            }
        }
        println!("\nno team with slug {slug}");
    }
}
