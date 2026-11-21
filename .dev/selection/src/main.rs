//! Matchday-selection census.
//!
//! Ticks a stubbed world for a season and reads every first-team sheet of the
//! top two divisions: how much an XI changes and why, how football is spread
//! through a squad, who keeps goal, what the bench holds, and who starts. Each
//! headline figure stands beside a real-football reference band.
//!
//! Usage:
//!   dev_selection [days]                              run a census (default 330 days)
//!   dev_selection --compare <a> <b> [--noise <c> <d>] rows that moved between two reports
//!   dev_selection --noise <c> <d>                     what two runs of one build differ by
//!   dev_selection --bands                             the reference bands and their sources

mod census;
mod reference;
mod report;
mod world;

use census::Census;
use core::{FootballSimulator, SimulationResult, SimulatorData};
use database::{DatabaseGenerator, DatabaseLoader};
use env_logger::Env;
use mimalloc::MiMalloc;
use reference::RealFootball;
use report::{Comparison, Report};
use std::future::Future;
use std::pin::pin;
use std::process::ExitCode;
use std::task::{Context, Poll, Waker};
use std::time::Instant;
use world::{DayFacts, WorldIndex};

#[global_allocator]
static GLOBAL: MiMalloc = MiMalloc;

struct Harness {
    data: SimulatorData,
}

impl Harness {
    const DEFAULT_DAYS: u32 = 330;
    /// Promotions and relegations move teams between tiers.
    const REINDEX_DAYS: u32 = 30;

    fn generate() -> Self {
        let database = DatabaseLoader::load();
        Harness {
            data: DatabaseGenerator::generate(&database),
        }
    }

    fn tick(&mut self) -> SimulationResult {
        Self::block_on(FootballSimulator::simulate(&mut self.data))
    }

    fn block_on<F: Future>(future: F) -> F::Output {
        let mut future = pin!(future);
        let mut cx = Context::from_waker(Waker::noop());
        loop {
            if let Poll::Ready(output) = future.as_mut().poll(&mut cx) {
                return output;
            }
            std::hint::spin_loop();
        }
    }

    fn run(&mut self, days: u32) {
        println!("# bin  dev_selection");
        println!(
            "# env  engine {}, build {}",
            if cfg!(feature = "match-stub") { "match-stub" } else { "real" },
            std::env::var("OF_BUILD").unwrap_or_else(|_| "unnamed".to_string())
        );
        println!("# args {days}");
        println!("start date {}", self.data.date.date());
        let mut census = Census::new(&self.data);
        let started = Instant::now();
        for day in 1..=days {
            let facts = DayFacts::read(&self.data, &census.index);
            let result = self.tick();
            census.record_day(&self.data, &facts, &result);
            if day % Self::REINDEX_DAYS == 0 {
                census.index = WorldIndex::build(&self.data);
                eprintln!(
                    "day {day} ({}) {:.0} s",
                    self.data.date.date(),
                    started.elapsed().as_secs_f64()
                );
            }
        }
        println!("end date {}", self.data.date.date());
        println!(
            "# time mean day {:.0} ms",
            1000.0 * started.elapsed().as_secs_f64() / f64::from(days.max(1))
        );
        let report = census.report();
        print!("{}", report.render());
        print!("{}", report.render_verdicts());
    }
}

enum Command {
    Census(u32),
    Compare {
        before: String,
        after: String,
        control: Option<(String, String)>,
    },
    Noise(String, String),
    Bands,
}

impl Command {
    fn parse(args: &[String]) -> Result<Command, String> {
        let pair = |flag: &str| -> Result<Option<(String, String)>, String> {
            match args.iter().position(|a| a == flag) {
                None => Ok(None),
                Some(at) => match (args.get(at + 1), args.get(at + 2)) {
                    (Some(a), Some(b)) => Ok(Some((a.clone(), b.clone()))),
                    _ => Err(format!("{flag} takes two report files")),
                },
            }
        };
        if args.iter().any(|a| a == "--bands") {
            return Ok(Command::Bands);
        }
        if let Some((before, after)) = pair("--compare")? {
            return Ok(Command::Compare {
                before,
                after,
                control: pair("--noise")?,
            });
        }
        if let Some((a, b)) = pair("--noise")? {
            return Ok(Command::Noise(a, b));
        }
        match args.first() {
            None => Ok(Command::Census(Harness::DEFAULT_DAYS)),
            Some(days) => days
                .parse()
                .map(Command::Census)
                .map_err(|_| format!("not a number of days: {days}")),
        }
    }

    fn run(self) -> Result<(), String> {
        match self {
            Command::Census(days) => Harness::generate().run(days),
            Command::Bands => print!("{}", RealFootball::render()),
            Command::Noise(a, b) => {
                print!("{}", Comparison::render_noise(&Report::load(&a)?, &Report::load(&b)?))
            }
            Command::Compare {
                before,
                after,
                control,
            } => {
                let control = match control {
                    Some((a, b)) => Some((Report::load(&a)?, Report::load(&b)?)),
                    None => None,
                };
                let comparison =
                    Comparison::new(Report::load(&before)?, Report::load(&after)?, control);
                print!("{}", comparison.render());
            }
        }
        Ok(())
    }
}

fn main() -> ExitCode {
    env_logger::Builder::from_env(Env::default().default_filter_or("error")).init();
    let args: Vec<String> = std::env::args().skip(1).collect();
    match Command::parse(&args).and_then(Command::run) {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("{message}");
            ExitCode::FAILURE
        }
    }
}
