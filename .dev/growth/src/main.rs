//! Development census. Builds the shipped world, runs it on the match stub,
//! and follows every player in a first team, reserve or youth side, academy
//! or the free-agent pool: a snapshot of each on the opening day and on every
//! anniversary of it, and his football once a week. The report reads ability
//! and attributes by position, age, football played and place, beside the
//! opening world, which is the database as loaded.
//!
//! Usage:
//!   cargo build --release
//!   ./target/release/dev_growth [seasons] [--days N] [--seed S]
//!   ./target/release/dev_growth --compare <before> <after>
//!   ./target/release/dev_growth --check <report>
//!
//!   seasons     seasons to simulate (default 4)
//!   --days N    simulate N days instead, for a quick look
//!   --seed S    pin the simulation seed before the world is generated (default 7)
//!   --compare   list every row of two reports that moved beyond the noise
//!   --check     pass or fail for each bullet of the world-shape requirement;
//!               exits non-zero unless every bullet passes

mod check;
mod compare;
mod memory;
mod report;
mod snapshot;
mod stats;
mod tracker;
mod world;

use check::ShapeCheck;
use compare::ReportComparison;
use core::utils::random::engine::RandomEngine;
use core::{FootballSimulator, SimulationResult, SimulatorData};
use database::{DatabaseGenerator, DatabaseLoader};
use memory::MemoryFloor;
use mimalloc::MiMalloc;
use report::Report;
use snapshot::YearlySnapshots;
use std::future::Future;
use std::pin::pin;
use std::task::{Context, Poll, Waker};
use std::time::Instant;
use tracker::WeeklyTracker;

#[global_allocator]
static GLOBAL: MiMalloc = MiMalloc;

/// A match result is kept this long. The census reads no match, and four
/// seasons of results cost tens of GB.
const MATCH_RETENTION_DAYS: i64 = 40;
const YEAR: u32 = 365;

struct Sim;

impl Sim {
    fn tick(data: &mut SimulatorData) -> SimulationResult {
        let mut future = pin!(FootballSimulator::simulate(data));
        let waker = Waker::noop();
        let mut cx = Context::from_waker(waker);
        loop {
            if let Poll::Ready(result) = future.as_mut().poll(&mut cx) {
                return result;
            }
            std::hint::spin_loop();
        }
    }
}

enum Options {
    Run { days: u32, seed: u64 },
    Compare { before: String, after: String },
    Check { report: String },
}

impl Options {
    fn parse(args: Vec<String>) -> Self {
        if args.first().map(String::as_str) == Some("--compare") && args.len() >= 3 {
            return Options::Compare {
                before: args[1].clone(),
                after: args[2].clone(),
            };
        }
        if args.first().map(String::as_str) == Some("--check") && args.len() >= 2 {
            return Options::Check {
                report: args[1].clone(),
            };
        }
        let (mut days, mut seed) = (4 * YEAR, 7);
        let mut args = args.into_iter();
        while let Some(arg) = args.next() {
            let mut value = || args.next().and_then(|v| v.parse::<u64>().ok());
            match arg.as_str() {
                "--days" => days = value().unwrap_or(60) as u32,
                "--seed" => seed = value().unwrap_or(7),
                other => {
                    if let Ok(seasons) = other.parse::<u32>() {
                        days = seasons * YEAR;
                    }
                }
            }
        }
        Options::Run { days, seed }
    }
}

#[derive(Default)]
struct GrowthCensus {
    tracker: WeeklyTracker,
    snapshots: YearlySnapshots,
}

impl GrowthCensus {
    fn run(days: u32, seed: u64) {
        RandomEngine::set_seed(seed);
        eprintln!("generating world...");
        let database = DatabaseLoader::load();
        let mut data = DatabaseGenerator::generate(&database);
        data.match_store =
            std::mem::take(&mut data.match_store).with_retention_days(MATCH_RETENTION_DAYS);

        let mut census = GrowthCensus::default();
        census.tracker.observe(&data);
        census.snapshots.take(&data, &mut census.tracker);
        eprintln!(
            "world starts {} with {} players",
            data.date.date(),
            census.snapshots.years[0].len()
        );

        let started = Instant::now();
        let mut simulated = 0;
        for day in 1..=days {
            Sim::tick(&mut data);
            simulated = day;
            if day % 7 == 0 {
                census.tracker.observe(&data);
            }
            if day % YEAR == 0 {
                census.snapshots.take(&data, &mut census.tracker);
            }
            if day % 30 == 0 {
                eprintln!(
                    "day {day} ({}) {:.0}s resident {:.1} GB free {:.1} GB",
                    data.date.date(),
                    started.elapsed().as_secs_f64(),
                    MemoryFloor::resident_gigabytes(),
                    MemoryFloor::free_gigabytes()
                );
                if MemoryFloor::reached() {
                    eprintln!("memory floor reached: reporting what was measured");
                    break;
                }
            }
        }
        let elapsed = started.elapsed().as_secs_f64();
        drop(data);
        println!(
            "# run   {simulated} days, mean day {:.0} ms, peak resident {:.1} GB",
            elapsed * 1000.0 / simulated.max(1) as f64,
            MemoryFloor::resident_gigabytes()
        );
        Report::new(&census.snapshots).print();
    }
}

fn main() {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("error")).init();
    let read = |path: &str| {
        std::fs::read_to_string(path).unwrap_or_else(|e| panic!("cannot read {path}: {e}"))
    };
    match Options::parse(std::env::args().skip(1).collect()) {
        Options::Run { days, seed } => GrowthCensus::run(days, seed),
        Options::Compare { before, after } => {
            ReportComparison::run(&read(&before), &read(&after));
        }
        Options::Check { report } => {
            if !ShapeCheck::run(&read(&report)) {
                std::process::exit(1);
            }
        }
    }
}
