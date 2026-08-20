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

use core::{FootballSimulator, SimulationResult, SimulatorData};
use database::{DatabaseGenerator, DatabaseLoader};
use mimalloc::MiMalloc;

/// Windows' system heap serialises concurrent alloc/free behind a global
/// lock; under the world sim's 32-thread rayon fan-out that lock — not CPU
/// or parallelism — is the dominant cost. mimalloc's per-thread heaps
/// remove the contention so the parallel phases actually scale.
#[global_allocator]
static GLOBAL: MiMalloc = MiMalloc;
use env_logger::Env;
use std::future::Future;
use std::pin::pin;
use std::collections::HashMap;
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
            let (h, a) = (m.score.home_team.get() as u32, m.score.away_team.get() as u32);
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

        for day in 1..=days {
            let tick_start = Instant::now();
            let result = self.tick();
            let ms = tick_start.elapsed().as_secs_f64() * 1000.0;

            let matches = result.match_results.len();
            census.record(&result);
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
        census.print(MIN_CENSUS_MATCHES);
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

    harness.bench(days);
}
