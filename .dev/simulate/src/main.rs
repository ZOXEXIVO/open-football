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
use core::r#match::FieldSquad;
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
        let mut world = WorldMatchCensus::default();

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
        census.print(MIN_CENSUS_MATCHES);
        world.print();
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
        println!("  interceptions       {:>8.2}", self.per(self.interceptions));
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
