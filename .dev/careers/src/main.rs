//! Career census. Follows cohorts of different kinds of player through whole
//! careers in a stubbed world: a biography per man, and counts of the moves a
//! reader would question, each read against what they should be read against.
//!
//! Usage:
//!   cargo build --release
//!   ./target/release/dev_careers [days] [--every N] [--bios N] [--seed S]
//!
//!   days        simulated days after cohorts are chosen (default 730)
//!   --every N   report every N days as well as at the end
//!   --bios N    biographies printed per cohort (default 6)
//!   --seed S    pin the simulation seed before the world is generated
//!
//! Set `OF_CAREER_ANCHORS_OFF=1` to read the same rows from a world whose
//! decisions ignore the career anchors.

mod biography;
mod census;
mod cohort;
mod judgement;
mod listing_watch;
mod memory;
mod reading;

use biography::Biographies;
use census::Census;
use chrono::Duration;
use cohort::Cohort;
use core::utils::random::engine::RandomEngine;
use core::{FootballSimulator, SimulationResult, SimulatorData};
use database::{DatabaseGenerator, DatabaseLoader};
use judgement::ClubJudgement;
use listing_watch::ListingWatch;
use memory::ResidentMemory;
use mimalloc::MiMalloc;
use reading::{ClubRanks, MarketWatch, MoveKind, RetirementWatch, Snap, WorldWalk};
use std::collections::HashSet;
use std::future::Future;
use std::pin::pin;
use std::task::{Context, Poll, Waker};
use std::time::Instant;

#[global_allocator]
static GLOBAL: MiMalloc = MiMalloc;

/// Clubs first assign squad status on a monthly pass, so a man reads as
/// `NotYetSet` on day 0 and a regulars' cohort would be empty.
const SELECT_AFTER_DAYS: u32 = 40;
/// A match result is kept this long. A career census reads no match after
/// the day it was played, and three seasons of results cost tens of GB.
const MATCH_RETENTION_DAYS: i64 = 40;
const YEAR: u32 = 365;
/// Seasons fold their history at their start: mid-August every statistic a
/// club reads about the season before is in place in most of the world.
const FIRST_SEASON_READING: u32 = 330;

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

struct Options {
    days: u32,
    every: u32,
    bios: usize,
    seed: Option<u64>,
}

impl Options {
    fn parse(args: Vec<String>) -> Self {
        let mut options = Options {
            days: 730,
            every: 0,
            bios: 6,
            seed: None,
        };
        let mut args = args.into_iter();
        while let Some(arg) = args.next() {
            let mut value = || args.next().and_then(|v| v.parse::<u64>().ok());
            match arg.as_str() {
                "--every" => options.every = value().unwrap_or(0) as u32,
                "--bios" => options.bios = value().unwrap_or(6) as usize,
                "--seed" => options.seed = value(),
                other => {
                    if let Ok(days) = other.parse() {
                        options.days = days;
                    }
                }
            }
        }
        options
    }
}

#[derive(Default)]
struct CareerCensus {
    ranks: ClubRanks,
    market: MarketWatch,
    retirements: RetirementWatch,
    census: Census,
    judgement: ClubJudgement,
    biographies: Biographies,
    listing_watch: ListingWatch,
}

impl CareerCensus {
    fn observe(&mut self, data: &SimulatorData, day: u32) {
        // A tick ends by advancing the calendar. What it did is dated the
        // day it just simulated, and a reader compares against that.
        let date = data.date.date() - Duration::days(1);
        let yearly = day % YEAR == 1;
        if yearly {
            self.ranks = ClubRanks::read(data);
        }
        let rows = self.market.new_rows(data);
        self.census.on_market_rows(day, &rows, &self.ranks);

        let CareerCensus {
            ranks,
            census,
            judgement,
            biographies,
            listing_watch,
            ..
        } = self;
        WorldWalk::players(data, ranks, |p, place| {
            let now = Snap::read(p, date, place);
            let today = rows.get(&p.id);
            let before = census.snaps.get(&p.id).copied();
            let sold_today = today.is_some_and(|r| {
                r.iter().any(|x| x.kind != MoveKind::Loan && x.fee > 0.0)
            }) && before.is_some_and(|b| b.club_id != now.club_id);
            judgement.on_player(p, before.as_ref(), &now, sold_today);
            if let Some(before) = before.as_ref() {
                listing_watch.on_player(p, before, &now, day, date, today, place.main_team);
            }
            census.on_player(p, &now, place, day, date, today, data);
            biographies.on_day(p, &now, place, date, yearly, today);
        });

        for p in self.retirements.new_since(data) {
            self.biographies.on_retired(p, date);
            self.census.on_retired(p, date);
        }
        self.listing_watch.on_day(data, &self.ranks, date);
        if day % 7 == 0 {
            self.listing_watch.on_week(data, &self.ranks, date);
            self.census.observe_wants(data, day);
        }
        if yearly {
            self.census.observe_year(data, &self.ranks, day);
        }
        if day >= FIRST_SEASON_READING && (day - FIRST_SEASON_READING) % YEAR == 0 {
            self.judgement.on_season(data, &self.census.snaps);
        }
    }

    fn report(&self, data: &SimulatorData, day: u32, bios: usize, started: &Instant) {
        println!(
            "\n================ CAREER CENSUS day {day} ({}) after {:.0}s, resident {:.1} GB ================",
            data.date.date(),
            started.elapsed().as_secs_f64(),
            ResidentMemory::gigabytes(),
        );
        self.census.report(data, day);
        self.listing_watch.report(data, day);
        self.judgement.report();
        self.biographies.report(bios);
        println!(
            "\nend of report: resident {:.1} GB",
            ResidentMemory::gigabytes()
        );
    }
}

fn main() {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("error")).init();
    let options = Options::parse(std::env::args().skip(1).collect());
    if let Some(seed) = options.seed {
        RandomEngine::set_seed(seed);
    }

    eprintln!("generating world...");
    let database = DatabaseLoader::load();
    let mut data = DatabaseGenerator::generate(&database);
    data.match_store = std::mem::take(&mut data.match_store).with_retention_days(MATCH_RETENTION_DAYS);

    let started = Instant::now();
    for _ in 0..SELECT_AFTER_DAYS {
        Sim::tick(&mut data);
    }

    let mut lab = CareerCensus {
        ranks: ClubRanks::read(&data),
        ..Default::default()
    };
    let chosen = Cohort::select(&data, &lab.ranks);
    lab.biographies = Biographies::follow(&data, &lab.ranks, &chosen);
    let unique: HashSet<u32> = chosen.keys().copied().collect();
    println!(
        "following {} men in {} cohorts; found {} of them",
        unique.len(),
        Cohort::ALL.len(),
        lab.biographies.men()
    );
    lab.observe(&data, 0);

    for day in 1..=options.days {
        let result = Sim::tick(&mut data);
        lab.census.on_results(&result);
        lab.observe(&data, day);
        if day % 30 == 0 {
            eprintln!(
                "day {day} {:.0}s, resident {:.1} GB",
                started.elapsed().as_secs_f64(),
                ResidentMemory::gigabytes()
            );
        }
        if options.every > 0 && day % options.every == 0 && day != options.days {
            lab.report(&data, day, options.bios, &started);
        }
    }
    lab.report(&data, options.days, options.bios, &started);
}
