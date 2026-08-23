//! Mind telemetry harness.
//!
//! `.dev/simulate` answers "how fast is the world tick" and
//! `.dev/transfers` answers "is the market alive". This answers the two
//! questions both mind plans stop at:
//!
//!   * **Is the parallel run agreeing with the layer it shadows?**
//!     `docs/player_mind.md` phase 3b asks whether `GoalStack::is_pressing`
//!     matches the legacy `Req` status; phase 3b's sibling 4b asks whether
//!     `MoodProfile` agrees with `PlayerHappiness`. `docs/staff_mind.md`
//!     S4b asks the same of `StaffMoodProfile` against `job_satisfaction`.
//!     None of them can be answered by a unit test — each asks what a
//!     *population* does over a season.
//!   * **What does a mind actually hold after a season, and what does it
//!     cost?** Episode / conviction / account / judgement counts, the
//!     goal-status ladder, the footprint, and the per-day tick budget.
//!
//! It also carries the manager-market census `docs/staff_mind.md` §10
//! gates S5's remaining conversions on: appointment rate, tenure, and the
//! sack-versus-resign split.
//!
//! Every parity line prints the **raw distribution of both sides** next
//! to it, and that is not decoration. A bias of +48 reads identically
//! whether the parallel run is high or the layer it shadows is low, and
//! the two want opposite fixes — the first census here mis-attributed
//! exactly that and cost a rebuild to find out.
//!
//! Everything here reads public world state after a real tick, so it
//! measures the shipping minds rather than a parallel model of them.
//!
//! Usage:
//!   cd .dev/mind && cargo build --release
//!   ./target/release/dev_mind [days] [--every N]
//!
//! Defaults to 300 days — long enough for a full season of episodes to
//! consolidate into convictions (the pass is monthly) and for the goal
//! ladder to have climbed somewhere, which is the shortest horizon on
//! which either parity question means anything.
//!
//! Budget for it: with the real engine that is roughly an hour. Pass
//! `--every 50` for interim reports, and redirect stdout to a file
//! directly rather than piping through `tee`, which block-buffers the
//! whole run into silence.

use core::club::player::mind::{GoalStatus, MemoryCensus};
use core::utils::DateUtils;
use core::{
    FootballSimulator, PlayerStatusType, SimulationResult, SimulatorData, Staff, StaffPosition,
    TeamType,
};
use database::{DatabaseGenerator, DatabaseLoader};
use mimalloc::MiMalloc;

/// Windows' system heap serialises concurrent alloc/free behind a global
/// lock; under the world sim's rayon fan-out that lock dominates. Same
/// rationale as `.dev/simulate`.
#[global_allocator]
static GLOBAL: MiMalloc = MiMalloc;

use chrono::NaiveDate;
use env_logger::Env;
use std::collections::HashMap;
use std::future::Future;
use std::pin::pin;
use std::task::{Context, Poll, Waker};
use std::time::Instant;

/// One season of episodes, consolidations and goal reviews.
const DEFAULT_DAYS: u32 = 300;

/// Youngest age counted as a senior. Below it a player's mind is mostly
/// empty by construction and would dilute every distribution.
const SENIOR_AGE: u8 = 20;

// ---------------------------------------------------------------------
// Distributions
// ---------------------------------------------------------------------

/// A bucketed integer distribution, reported as a histogram plus the
/// numbers an inspection actually reads: mean, median, p90, max.
#[derive(Debug, Default)]
struct Spread {
    values: Vec<u32>,
}

impl Spread {
    fn push(&mut self, value: u32) {
        self.values.push(value);
    }

    fn len(&self) -> usize {
        self.values.len()
    }

    fn mean(&self) -> f64 {
        if self.values.is_empty() {
            return 0.0;
        }
        self.values.iter().map(|v| *v as f64).sum::<f64>() / self.values.len() as f64
    }

    fn percentile(&mut self, p: f64) -> u32 {
        if self.values.is_empty() {
            return 0;
        }
        self.values.sort_unstable();
        let index = ((self.values.len() - 1) as f64 * p).round() as usize;
        self.values[index]
    }

    fn max(&self) -> u32 {
        self.values.iter().copied().max().unwrap_or(0)
    }

    fn line(&mut self, label: &str) -> String {
        format!(
            "  {label:<28} n={:<7} mean={:<7.2} p50={:<5} p90={:<5} max={}",
            self.len(),
            self.mean(),
            self.percentile(0.50),
            self.percentile(0.90),
            self.max(),
        )
    }
}

/// Signed error between a parallel-run reading and the layer it shadows,
/// on the same scale. What a parity check is actually made of.
#[derive(Debug, Default)]
struct Agreement {
    errors: Vec<f32>,
}

impl Agreement {
    fn push(&mut self, mine: f32, theirs: f32) {
        self.errors.push(mine - theirs);
    }

    fn len(&self) -> usize {
        self.errors.len()
    }

    fn bias(&self) -> f32 {
        if self.errors.is_empty() {
            return 0.0;
        }
        self.errors.iter().sum::<f32>() / self.errors.len() as f32
    }

    fn mean_abs(&self) -> f32 {
        if self.errors.is_empty() {
            return 0.0;
        }
        self.errors.iter().map(|e| e.abs()).sum::<f32>() / self.errors.len() as f32
    }

    /// Share of readings within `band` of the layer being shadowed. The
    /// number a swap-over decision is actually made on.
    fn within(&self, band: f32) -> f32 {
        if self.errors.is_empty() {
            return 0.0;
        }
        let inside = self.errors.iter().filter(|e| e.abs() <= band).count();
        inside as f32 / self.errors.len() as f32
    }

    fn line(&self, label: &str) -> String {
        format!(
            "  {label:<28} n={:<7} bias={:<+8.2} mean|err|={:<7.2} within±10={:.1}%  within±20={:.1}%",
            self.len(),
            self.bias(),
            self.mean_abs(),
            self.within(10.0) * 100.0,
            self.within(20.0) * 100.0,
        )
    }
}

/// The four-way agreement between a boolean the sim acts on and the
/// boolean the parallel run would act on. A raw agreement percentage
/// hides the only thing that matters — *which way* they disagree.
#[derive(Debug, Default)]
struct Confusion {
    both: usize,
    legacy_only: usize,
    mind_only: usize,
    neither: usize,
}

impl Confusion {
    fn push(&mut self, legacy: bool, mind: bool) {
        match (legacy, mind) {
            (true, true) => self.both += 1,
            (true, false) => self.legacy_only += 1,
            (false, true) => self.mind_only += 1,
            (false, false) => self.neither += 1,
        }
    }

    fn total(&self) -> usize {
        self.both + self.legacy_only + self.mind_only + self.neither
    }

    fn agreement(&self) -> f32 {
        if self.total() == 0 {
            return 0.0;
        }
        (self.both + self.neither) as f32 / self.total() as f32
    }
}

// ---------------------------------------------------------------------
// Census
// ---------------------------------------------------------------------

#[derive(Debug, Default)]
struct MindCensus {
    // ── Population ────────────────────────────────────────────────────
    players: usize,
    seniors: usize,
    staff: usize,
    managers: usize,

    // ── What a player's mind holds ────────────────────────────────────
    episodes: Spread,
    flashbulbs: Spread,
    convictions: Spread,
    accounts: Spread,
    goals_held: Spread,
    /// Players holding nothing at all after a season — the number that
    /// says whether the emit sites are actually wired to anything.
    empty_minds: usize,

    /// Where every live player goal sits on the ladder.
    goal_ladder: HashMap<&'static str, usize>,

    // ── Parallel-run agreement, player side ───────────────────────────
    /// Phase 3b: the legacy `Req` status against `GoalStack::is_pressing`.
    pressing: Confusion,
    /// Phase 4b: `PlayerHappiness::morale` against `MoodProfile`.
    morale: Agreement,
    /// How much of a player the five faculties can actually read.
    coverage: Vec<f32>,
    /// The two raw readings behind the parity lines, so a disagreement
    /// can be attributed. "The mind is high" and "the legacy layer is
    /// low" produce the same bias and want opposite fixes.
    raw_morale: Spread,
    raw_mind_morale: Spread,

    // ── What a manager's mind holds ───────────────────────────────────
    mgr_episodes: Spread,
    mgr_convictions: Spread,
    mgr_judgements: Spread,
    mgr_firm_judgements: Spread,
    mgr_settled_judgements: Spread,
    mgr_wrong_judgements: Spread,
    mgr_goals_held: Spread,
    mgr_goal_ladder: HashMap<&'static str, usize>,
    /// S4b: `job_satisfaction` against `StaffMoodProfile`.
    satisfaction: Agreement,
    mgr_coverage: Vec<f32>,
    raw_job_satisfaction: Spread,
    raw_mind_satisfaction: Spread,

    // ── Manager market (§10) ──────────────────────────────────────────
    /// Tenure in months, from the manager's own record of taking the job.
    tenure_months: Spread,
    /// Sackings carried by the men currently in work — a career count,
    /// not a rate, and the thing `AmbitionMind::cynicism` reads.
    career_sackings: Spread,
    /// Clubs whose main-team head-coach seat is vacant right now.
    vacant_seats: usize,
    /// Clubs whose seat is filled by a caretaker rather than a manager.
    caretaker_seats: usize,

    // ── Footprint ─────────────────────────────────────────────────────
    player_mind_bytes: usize,
    staff_mind_bytes: usize,
}

impl MindCensus {
    fn collect(data: &SimulatorData) -> Self {
        let mut census = MindCensus {
            player_mind_bytes: size_of::<core::club::player::mind::PlayerMind>(),
            staff_mind_bytes: size_of::<core::club::staff::mind::StaffMind>(),
            ..Default::default()
        };
        let today = data.date.date();

        for continent in &data.continents {
            for country in &continent.countries {
                for club in &country.clubs {
                    census.collect_club(club, today);
                }
            }
        }
        census
    }

    fn collect_club(&mut self, club: &core::Club, today: NaiveDate) {
        let mut seat_filled = false;
        let mut seat_is_caretaker = false;

        for team in club.teams.teams.iter() {
            for player in team.players.iter() {
                self.collect_player(player, today);
            }
            if team.team_type != TeamType::Main {
                continue;
            }
            for staff in team.staffs.iter() {
                self.collect_staff(staff, today);
                let Some(contract) = &staff.contract else {
                    continue;
                };
                match contract.position {
                    StaffPosition::Manager => seat_filled = true,
                    StaffPosition::CaretakerManager => {
                        seat_filled = true;
                        seat_is_caretaker = true;
                    }
                    _ => {}
                }
            }
        }

        if !seat_filled {
            self.vacant_seats += 1;
        } else if seat_is_caretaker {
            self.caretaker_seats += 1;
        }
    }

    fn collect_player(&mut self, player: &core::Player, today: NaiveDate) {
        self.players += 1;
        if DateUtils::age(player.birth_date, today) < SENIOR_AGE {
            return;
        }
        self.seniors += 1;

        let memory: MemoryCensus = player.mind.census();
        let goals = player.mind.goal_census();

        self.episodes.push(memory.episodes as u32);
        self.flashbulbs.push(memory.flashbulbs as u32);
        self.convictions.push(memory.facts as u32);
        self.accounts.push(memory.accounts as u32);
        self.goals_held.push(goals.live() as u32);

        if memory.episodes == 0 && memory.facts == 0 && goals.live() == 0 {
            self.empty_minds += 1;
        }

        for goal in player.mind.goals().live() {
            *self.goal_ladder.entry(status_label(goal.status)).or_default() += 1;
        }

        // Phase 3b. `Req` is the status the transfer path acts on today;
        // `is_pressing` is what the goal stack would act on instead.
        self.pressing.push(
            player.statuses.has(PlayerStatusType::Req),
            player.mind.is_pressing(),
        );

        // Phase 4b. Both on the 0..100 morale scale.
        let profile = player.mind.appraise();
        self.coverage.push(profile.coverage());
        self.morale
            .push(profile.as_morale(), player.happiness.morale);
        self.raw_morale.push(player.happiness.morale.round() as u32);
        self.raw_mind_morale.push(profile.as_morale().round() as u32);
    }

    fn collect_staff(&mut self, staff: &Staff, today: NaiveDate) {
        self.staff += 1;
        let is_manager = staff
            .contract
            .as_ref()
            .map(|c| {
                matches!(
                    c.position,
                    StaffPosition::Manager | StaffPosition::CaretakerManager
                )
            })
            .unwrap_or(false);
        if !is_manager {
            return;
        }
        self.managers += 1;

        let memory = staff.mind.census();
        let goals = staff.mind.goal_census();
        self.mgr_episodes.push(memory.episodes as u32);
        self.mgr_convictions.push(memory.facts as u32);
        self.mgr_goals_held.push(goals.live() as u32);

        let ctx = staff.mind_context(today, 0);
        let judgements = staff.mind.judgement_census(&ctx);
        self.mgr_judgements.push(judgements.held as u32);
        self.mgr_firm_judgements.push(judgements.firm as u32);
        self.mgr_settled_judgements.push(judgements.settled as u32);
        self.mgr_wrong_judgements.push(judgements.wrong as u32);

        for goal in staff.mind.goals().live() {
            *self
                .mgr_goal_ladder
                .entry(status_label(goal.status))
                .or_default() += 1;
        }

        let profile = staff.mind.appraise();
        self.mgr_coverage.push(profile.coverage());
        self.satisfaction
            .push(profile.as_satisfaction(), staff.job_satisfaction);
        self.raw_job_satisfaction
            .push(staff.job_satisfaction.round() as u32);
        self.raw_mind_satisfaction
            .push(profile.as_satisfaction().round() as u32);

        self.tenure_months
            .push(staff.mind.ambition.months_in_the_job(ctx.day()) as u32);
        self.career_sackings.push(staff.mind.ambition.sackings as u32);
    }
}

fn status_label(status: GoalStatus) -> &'static str {
    match status {
        GoalStatus::Latent => "Latent",
        GoalStatus::Active => "Active",
        GoalStatus::Voiced => "Voiced",
        GoalStatus::Pressing => "Pressing",
        GoalStatus::Satisfied => "Satisfied",
        GoalStatus::Frustrated => "Frustrated",
        GoalStatus::Abandoned => "Abandoned",
    }
}

/// The ladder in its own order, not the hash map's.
const LADDER: [&str; 7] = [
    "Latent",
    "Active",
    "Voiced",
    "Pressing",
    "Satisfied",
    "Frustrated",
    "Abandoned",
];

// ---------------------------------------------------------------------
// Report
// ---------------------------------------------------------------------

struct ReportPrinter;

impl ReportPrinter {
    fn print(census: &mut MindCensus, data: &SimulatorData, day: u32, tick_ms: f64) {
        let date = data.date.date();
        println!("\n{}", "=".repeat(96));
        println!("MIND CENSUS — day {day}  ({date})");
        println!("{}", "=".repeat(96));

        println!(
            "\npopulation: {} players ({} senior), {} staff ({} managers)",
            census.players, census.seniors, census.staff, census.managers
        );
        println!(
            "footprint:  PlayerMind {} B · StaffMind {} B  ⇒ {:.1} MB across the world",
            census.player_mind_bytes,
            census.staff_mind_bytes,
            (census.players * census.player_mind_bytes + census.staff * census.staff_mind_bytes)
                as f64
                / 1_048_576.0,
        );

        Self::player_section(census);
        Self::staff_section(census);
        Self::market_section(census);
        Self::budget_section(census, tick_ms);
    }

    fn player_section(census: &mut MindCensus) {
        println!("\n── what a senior player's mind holds ──");
        println!("{}", census.episodes.line("episodes (cap 32)"));
        println!("{}", census.flashbulbs.line("flashbulbs (cap 6)"));
        println!("{}", census.convictions.line("convictions (cap 24)"));
        println!("{}", census.accounts.line("ledger accounts (cap 32)"));
        println!("{}", census.goals_held.line("live goals (cap 12)"));
        println!(
            "  {:<28} {} ({:.1}% of seniors)  ← an emit-site wiring check",
            "empty minds",
            census.empty_minds,
            pct(census.empty_minds, census.seniors),
        );

        println!("\n── the goal ladder, players ──");
        Self::ladder(&census.goal_ladder);

        println!("\n── parallel run: player ──");
        println!(
            "  {:<28} agreement={:.1}%   both={} legacy-only={} mind-only={} neither={}",
            "Req vs is_pressing (3b)",
            census.pressing.agreement() * 100.0,
            census.pressing.both,
            census.pressing.legacy_only,
            census.pressing.mind_only,
            census.pressing.neither,
        );
        println!("{}", census.morale.line("MoodProfile vs morale (4b)"));
        println!("{}", census.raw_morale.line("  …morale, raw"));
        println!("{}", census.raw_mind_morale.line("  …MoodProfile, raw"));
        println!(
            "  {:<28} mean={:.2}  ← how much of a player the faculties can read",
            "faculty coverage",
            mean(&census.coverage),
        );
    }

    fn staff_section(census: &mut MindCensus) {
        println!("\n── what a manager's mind holds ──");
        println!("{}", census.mgr_episodes.line("episodes (cap 32)"));
        println!("{}", census.mgr_convictions.line("convictions (cap 24)"));
        println!("{}", census.mgr_goals_held.line("live goals (cap 12)"));
        println!("{}", census.mgr_judgements.line("judgements (cap 48)"));
        println!("{}", census.mgr_firm_judgements.line("  …firm (conf ≥ .5)"));
        println!("{}", census.mgr_settled_judgements.line("  …settled"));
        println!(
            "{}",
            census.mgr_wrong_judgements.line("  …he got wrong")
        );

        println!("\n── the goal ladder, managers ──");
        Self::ladder(&census.mgr_goal_ladder);

        println!("\n── parallel run: staff ──");
        println!(
            "{}",
            census.satisfaction.line("StaffMood vs job_satisfaction")
        );
        println!(
            "{}",
            census.raw_job_satisfaction.line("  …job_satisfaction, raw")
        );
        println!(
            "{}",
            census.raw_mind_satisfaction.line("  …StaffMood, raw")
        );
        println!(
            "  {:<28} mean={:.2}",
            "faculty coverage",
            mean(&census.mgr_coverage),
        );
    }

    fn market_section(census: &mut MindCensus) {
        println!("\n── manager market ──");
        println!("{}", census.tenure_months.line("tenure (months)"));
        println!("{}", census.career_sackings.line("career sackings"));
        println!(
            "  {:<28} {} vacant · {} caretaker",
            "head-coach seats", census.vacant_seats, census.caretaker_seats,
        );
    }

    fn budget_section(census: &MindCensus, tick_ms: f64) {
        println!("\n── budget ──");
        println!("  {:<28} {tick_ms:.1} ms", "whole world tick");
        let minds = census.players + census.staff;
        println!(
            "  {:<28} {} minds ⇒ {:.3} ms/mind",
            "…across",
            minds,
            if minds == 0 {
                0.0
            } else {
                tick_ms / minds as f64
            },
        );
        println!(
            "  note: the ≤2 ms/day staff budget in docs/staff_mind.md §10 needs an\n  \
             A/B against a build with the mind compiled out; this is the whole tick."
        );
    }

    fn ladder(counts: &HashMap<&'static str, usize>) {
        let total: usize = counts.values().sum();
        for rung in LADDER {
            let n = counts.get(rung).copied().unwrap_or(0);
            let bar = "█".repeat((pct(n, total) / 2.0).round() as usize);
            println!("  {rung:<12} {n:>7}  {:>5.1}%  {bar}", pct(n, total));
        }
        println!("  {:<12} {total:>7}", "total");
    }
}

fn pct(part: usize, whole: usize) -> f64 {
    if whole == 0 {
        return 0.0;
    }
    part as f64 / whole as f64 * 100.0
}

fn mean(values: &[f32]) -> f32 {
    if values.is_empty() {
        return 0.0;
    }
    values.iter().sum::<f32>() / values.len() as f32
}

// ---------------------------------------------------------------------
// Harness
// ---------------------------------------------------------------------

struct SimHarness {
    data: SimulatorData,
    /// Rolling mean of the whole-world tick, so the budget line reads a
    /// steady state rather than whichever day the report landed on.
    tick_ms: f64,
    ticks: u32,
}

impl SimHarness {
    fn generate() -> Self {
        let database = DatabaseLoader::load();
        let data = DatabaseGenerator::generate(&database);
        SimHarness {
            data,
            tick_ms: 0.0,
            ticks: 0,
        }
    }

    fn tick(&mut self) -> SimulationResult {
        let started = Instant::now();
        let result = Self::block_on(FootballSimulator::simulate(&mut self.data));
        let elapsed = started.elapsed().as_secs_f64() * 1_000.0;
        self.ticks += 1;
        self.tick_ms += (elapsed - self.tick_ms) / self.ticks as f64;
        result
    }

    /// See `.dev/simulate`: the simulate future never awaits an I/O
    /// point, so a no-op waker completes it on the first poll.
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

    fn run(&mut self, days: u32, every: Option<u32>) {
        let start = Instant::now();
        for day in 1..=days {
            self.tick();
            if day % 25 == 0 {
                eprintln!(
                    "  … day {day}/{days}  {}  ({:.0}s elapsed)",
                    self.data.date.date(),
                    start.elapsed().as_secs_f64(),
                );
            }
            if let Some(n) = every
                && n > 0
                && day % n == 0
                && day != days
            {
                let mut report = MindCensus::collect(&self.data);
                ReportPrinter::print(&mut report, &self.data, day, self.tick_ms);
            }
        }
        let mut report = MindCensus::collect(&self.data);
        ReportPrinter::print(&mut report, &self.data, days, self.tick_ms);
        eprintln!(
            "simulated {days} days in {:.1}s",
            start.elapsed().as_secs_f64()
        );
    }
}

fn main() {
    // Quiet by default: the world generator and the national-callup pass
    // emit a warn line per synthetic squad, which would bury the report.
    env_logger::Builder::from_env(Env::default().default_filter_or("error")).init();

    let args: Vec<String> = std::env::args().skip(1).collect();
    let days = args
        .iter()
        .find_map(|a| a.parse::<u32>().ok())
        .unwrap_or(DEFAULT_DAYS);
    let every = args
        .iter()
        .position(|a| a == "--every")
        .and_then(|i| args.get(i + 1))
        .and_then(|v| v.parse::<u32>().ok());

    eprintln!("generating world…");
    let gen_start = Instant::now();
    let mut harness = SimHarness::generate();
    eprintln!(
        "world generated in {:.1}s — simulating {days} days",
        gen_start.elapsed().as_secs_f64(),
    );

    // The world as generated, so every later number reads against its
    // starting point rather than against zero. Every mind is empty here
    // by construction; if they are not, something seeded them.
    let mut initial = MindCensus::collect(&harness.data);
    ReportPrinter::print(&mut initial, &harness.data, 0, 0.0);

    harness.run(days, every);
}
