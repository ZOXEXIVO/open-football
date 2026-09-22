//! Squad-balance harness.
//!
//! Generates the world, ticks it day by day, and watches how clubs spread
//! their players across their squads (Main / Second / B / Reserve /
//! U23..U18 / academy):
//!
//! * a roster table per squad of every watched club — size, lines, age
//!   bands, mean CA, and how many men are registered in the wrong squad for
//!   their age (overage, or young enough for a younger squad the club runs);
//! * every intra-club move of a watched club's players as the day it
//!   happens (academy → U19, U19 → U21, Main → 2, …), plus joins/leaves;
//! * appearances per squad over each window — who is actually playing;
//! * a world census at the end: youth squads that cannot field a matchday
//!   squad, youth-eligible boys parked in older squads, idle senior squads.
//!
//! Usage:
//!   cargo build --release
//!   ./target/release/dev_squads [days] [--every N] [club name ...]
//!
//!   days      simulated days (default 120)
//!   --every   roster/appearance snapshot interval in days (default 30)
//!   club      one or more club names; exact (case-insensitive) match wins,
//!             else substring. Default: "Spartak Moscow".
//!
//! The rebalance pass logs every move with its reason at debug level:
//!   RUST_LOG=core::club::core::squad=debug ./target/release/dev_squads

use core::{Club, Person, PlayerFieldPositionGroup, SimulationResult, SimulatorData, TeamType};
use core::{FootballSimulator, Player};
use database::{DatabaseGenerator, DatabaseLoader};
use env_logger::Env;
use mimalloc::MiMalloc;
use std::collections::{BTreeMap, HashMap};
use std::future::Future;
use std::pin::pin;
use std::task::{Context, Poll, Waker};
use std::time::Instant;

#[global_allocator]
static GLOBAL: MiMalloc = MiMalloc;

const DEFAULT_DAYS: u32 = 120;
const DEFAULT_EVERY: u32 = 30;
const DEFAULT_CLUB: &str = "Spartak Moscow";

/// Eleven to start, five on the bench: fewer than this and a squad cannot
/// name a proper matchday sheet.
const MATCHDAY_SQUAD: usize = 16;
const FIELDING: usize = 11;

struct Args {
    days: u32,
    every: u32,
    clubs: Vec<String>,
}

impl Args {
    fn parse() -> Self {
        let mut days = DEFAULT_DAYS;
        let mut every = DEFAULT_EVERY;
        let mut clubs = Vec::new();
        let mut it = std::env::args().skip(1);
        let mut days_set = false;
        while let Some(a) = it.next() {
            if a == "--every" {
                every = it
                    .next()
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(DEFAULT_EVERY);
            } else if !days_set && a.parse::<u32>().is_ok() {
                days = a.parse().unwrap();
                days_set = true;
            } else {
                clubs.push(a);
            }
        }
        if clubs.is_empty() {
            clubs.push(DEFAULT_CLUB.to_string());
        }
        Args {
            days,
            every: every.max(1),
            clubs,
        }
    }
}

/// Where a player sits inside his club.
#[derive(Clone, PartialEq, Eq)]
enum Slot {
    Team { team_id: u32, label: String },
    Academy,
}

impl Slot {
    fn label(&self) -> &str {
        match self {
            Slot::Team { label, .. } => label,
            Slot::Academy => "Academy",
        }
    }
}

#[derive(Clone)]
struct Whereabouts {
    club_id: u32,
    slot: Slot,
    name: String,
    age: u8,
    ca: u8,
    pos: &'static str,
}

struct World;

impl World {
    fn clubs(data: &SimulatorData) -> impl Iterator<Item = &Club> {
        data.continents
            .iter()
            .flat_map(|c| c.countries.iter())
            .flat_map(|c| c.clubs.iter())
    }

    fn team_label(club: &Club, team_type: TeamType, name: &str) -> String {
        match team_type {
            TeamType::Main => "Main".to_string(),
            TeamType::Second | TeamType::B => {
                let short = name.strip_prefix(&club.name).unwrap_or(name).trim();
                if short.is_empty() {
                    format!("{team_type:?}")
                } else {
                    format!("{team_type:?}({short})")
                }
            }
            other => format!("{other:?}"),
        }
    }

    fn line(p: &Player) -> &'static str {
        match p.position().position_group() {
            PlayerFieldPositionGroup::Goalkeeper => "GK",
            PlayerFieldPositionGroup::Defender => "DEF",
            PlayerFieldPositionGroup::Midfielder => "MID",
            PlayerFieldPositionGroup::Forward => "FWD",
        }
    }
}

/// The club's youngest-first ladder of age-capped squads.
struct YouthLadder {
    caps: Vec<(TeamType, u8)>,
}

impl YouthLadder {
    fn of(club: &Club) -> Self {
        let mut caps: Vec<(TeamType, u8)> = club
            .teams
            .teams
            .iter()
            .filter_map(|t| t.team_type.development_age_cap().map(|c| (t.team_type, c)))
            .collect();
        caps.sort_by_key(|(_, c)| *c);
        YouthLadder { caps }
    }

    /// The youngest squad this club runs that a player of `age` may play in.
    fn home_for(&self, age: u8) -> Option<TeamType> {
        self.caps
            .iter()
            .find(|(_, cap)| age <= *cap)
            .map(|(t, _)| *t)
    }

    /// He is on `team_type` but the club has a younger squad he still fits.
    fn fits_younger(&self, team_type: TeamType, age: u8) -> bool {
        let Some(home) = self.home_for(age) else {
            return false;
        };
        if home == team_type {
            return false;
        }
        match team_type.development_age_cap() {
            Some(cap) => self
                .caps
                .iter()
                .find(|(t, _)| *t == home)
                .is_some_and(|(_, home_cap)| *home_cap < cap),
            None => true,
        }
    }
}

/// Starts and substitute appearances per player, and fixtures per team,
/// read straight off the match results' field squads — so it counts under
/// the stub too.
#[derive(Default)]
struct Appearances {
    per_player: HashMap<u32, (u16, u16)>,
    per_team: HashMap<u32, u16>,
}

impl Appearances {
    fn record(&mut self, result: &SimulationResult) {
        for m in &result.match_results {
            *self.per_team.entry(m.home_team_id).or_default() += 1;
            *self.per_team.entry(m.away_team_id).or_default() += 1;
            let Some(details) = &m.details else { continue };
            for squad in [&details.left_team_players, &details.right_team_players] {
                for id in &squad.main {
                    self.per_player.entry(*id).or_default().0 += 1;
                }
                for id in &squad.substitutes_used {
                    self.per_player.entry(*id).or_default().1 += 1;
                }
            }
        }
    }

    fn apps(&self, id: u32) -> u16 {
        self.per_player.get(&id).map_or(0, |(s, u)| s + u)
    }

    fn fixtures(&self, team_id: u32) -> u16 {
        self.per_team.get(&team_id).copied().unwrap_or(0)
    }
}

/// Age bands the roster table prints.
struct AgeBands;

impl AgeBands {
    const LABELS: [&'static str; 8] = ["<=16", "17", "18", "19", "20", "21", "22-23", "24+"];

    fn index(age: u8) -> usize {
        match age {
            0..=16 => 0,
            17 => 1,
            18 => 2,
            19 => 3,
            20 => 4,
            21 => 5,
            22..=23 => 6,
            _ => 7,
        }
    }
}

struct RosterReport;

impl RosterReport {
    fn print(club: &Club, date: chrono::NaiveDate, apps: Option<(&Appearances, u32)>) {
        let ladder = YouthLadder::of(club);
        let caps: Vec<String> = ladder
            .caps
            .iter()
            .map(|(t, c)| format!("{t:?}<={c}"))
            .collect();
        println!(
            "\n=== {} [{}] {}  (youth ladder: {})",
            club.name,
            club.id,
            date,
            if caps.is_empty() {
                "none".to_string()
            } else {
                caps.join(", ")
            }
        );
        print!(
            "{:<16} {:>4} {:>4} {:>3} {:>3} {:>3} {:>3} |",
            "squad", "lg", "n", "GK", "DEF", "MID", "FWD"
        );
        for l in AgeBands::LABELS {
            print!(" {l:>5}");
        }
        print!(" | {:>5} {:>5} {:>6}", "meanCA", "over", "young");
        if apps.is_some() {
            print!(" | {:>4} {:>6} {:>6}", "fix", "0-apps", "apps/p");
        }
        println!();

        let mut teams: Vec<_> = club.teams.teams.iter().collect();
        teams.sort_by_key(|t| t.team_type.menu_order());
        for t in teams {
            let players = &t.players.players;
            let mut lines = [0usize; 4];
            let mut bands = [0usize; 8];
            let mut ca_sum = 0u32;
            let mut over = 0;
            let mut young = 0;
            let mut idle = 0;
            let mut app_sum = 0u32;
            for p in players {
                let age = p.age(date);
                lines[p.position().position_group().index()] += 1;
                bands[AgeBands::index(age)] += 1;
                ca_sum += p.player_attributes.current_ability as u32;
                if t.team_type
                    .development_age_cap()
                    .is_some_and(|cap| age > cap)
                {
                    over += 1;
                }
                if ladder.fits_younger(t.team_type, age) {
                    young += 1;
                }
                if let Some((a, _)) = apps {
                    let n = a.apps(p.id);
                    app_sum += n as u32;
                    if n == 0 {
                        idle += 1;
                    }
                }
            }
            let n = players.len();
            print!(
                "{:<16} {:>4} {:>4} {:>3} {:>3} {:>3} {:>3} |",
                World::team_label(club, t.team_type, &t.name),
                if t.league_id.is_some() { "yes" } else { "-" },
                n,
                lines[0],
                lines[1],
                lines[2],
                lines[3],
            );
            for b in bands {
                print!(" {b:>5}");
            }
            print!(
                " | {:>6.1} {:>5} {:>6}",
                if n == 0 {
                    0.0
                } else {
                    ca_sum as f64 / n as f64
                },
                over,
                young
            );
            if let Some((a, _)) = apps {
                print!(
                    " | {:>4} {:>6} {:>6.1}",
                    a.fixtures(t.id),
                    idle,
                    if n == 0 {
                        0.0
                    } else {
                        app_sum as f64 / n as f64
                    }
                );
            }
            let flag = if t.team_type.is_youth() && n < FIELDING {
                "  << cannot field XI"
            } else if n < MATCHDAY_SQUAD {
                "  << below matchday 16"
            } else {
                ""
            };
            println!("{flag}");
        }
        println!(
            "{:<16} {:>4} {:>4}",
            "Academy",
            "-",
            club.academy.players.players.len()
        );
        if let Some((_, window)) = apps {
            println!("(fix / 0-apps / apps per player over the last {window} days)");
        }
    }
}

/// Every intra-club move of the watched clubs' players.
#[derive(Default)]
struct MoveLog {
    last: HashMap<u32, Whereabouts>,
    flows: BTreeMap<(String, String, String), u32>,
}

impl MoveLog {
    fn snapshot(data: &SimulatorData, watched: &[u32]) -> HashMap<u32, Whereabouts> {
        let date = data.date.date();
        let mut out = HashMap::new();
        for club in World::clubs(data).filter(|c| watched.contains(&c.id)) {
            let mut put = |p: &Player, slot: Slot| {
                out.insert(
                    p.id,
                    Whereabouts {
                        club_id: club.id,
                        slot,
                        name: p.full_name.to_string(),
                        age: p.age(date),
                        ca: p.player_attributes.current_ability,
                        pos: World::line(p),
                    },
                );
            };
            for t in &club.teams.teams {
                let slot = Slot::Team {
                    team_id: t.id,
                    label: World::team_label(club, t.team_type, &t.name),
                };
                for p in &t.players.players {
                    put(p, slot.clone());
                }
            }
            for p in &club.academy.players.players {
                put(p, Slot::Academy);
            }
        }
        out
    }

    fn start(data: &SimulatorData, watched: &[u32]) -> Self {
        MoveLog {
            last: Self::snapshot(data, watched),
            flows: BTreeMap::new(),
        }
    }

    fn club_name(data: &SimulatorData, id: u32) -> String {
        World::clubs(data)
            .find(|c| c.id == id)
            .map(|c| c.name.clone())
            .unwrap_or_default()
    }

    fn step(&mut self, data: &SimulatorData, watched: &[u32], day: u32) {
        let now = Self::snapshot(data, watched);
        let date = data.date.date();
        let mut lines: Vec<String> = Vec::new();
        for (id, w) in &now {
            let (from, kind) = match self.last.get(id) {
                Some(prev) if prev.club_id == w.club_id && prev.slot == w.slot => continue,
                Some(prev) if prev.club_id == w.club_id => (prev.slot.label().to_string(), "MOVE"),
                _ => ("(outside)".to_string(), "JOIN"),
            };
            lines.push(format!(
                "d{day:<4} {date} {kind:<5} {:<24} {:>2}y CA{:<3} {:<3} {from:>12} -> {}",
                w.name,
                w.age,
                w.ca,
                w.pos,
                w.slot.label()
            ));
            *self
                .flows
                .entry((
                    Self::club_name(data, w.club_id),
                    from,
                    w.slot.label().to_string(),
                ))
                .or_default() += 1;
        }
        for (id, prev) in &self.last {
            if now.contains_key(id) {
                continue;
            }
            lines.push(format!(
                "d{day:<4} {date} LEAVE {:<24} {:>2}y CA{:<3} {:<3} {:>12} -> (outside)",
                prev.name,
                prev.age,
                prev.ca,
                prev.pos,
                prev.slot.label()
            ));
            *self
                .flows
                .entry((
                    Self::club_name(data, prev.club_id),
                    prev.slot.label().to_string(),
                    "(outside)".to_string(),
                ))
                .or_default() += 1;
        }
        lines.sort();
        for l in lines {
            println!("{l}");
        }
        self.last = now;
    }

    fn print_flows(&self) {
        println!("\n--- SQUAD FLOWS (whole run) ---");
        for ((club, from, to), n) in &self.flows {
            println!("{club:<24} {from:>14} -> {to:<14} {n:>4}");
        }
    }
}

/// Whole-world squad-balance census.
#[derive(Default)]
struct WorldCensus {
    /// Per youth team type: (teams, empty, <11, <16, players).
    youth: BTreeMap<String, (usize, usize, usize, usize, usize)>,
    /// Boys whose youngest squad is short while they sit in an older one.
    stranded_young: usize,
    /// Clubs where that happens.
    stranded_clubs: usize,
    /// Per team type: (players, 0-apps players, teams with no fixture).
    idle: BTreeMap<String, (usize, usize, usize)>,
    /// Per team type: squads above 30.
    bloated: BTreeMap<String, usize>,
}

impl WorldCensus {
    fn take(data: &SimulatorData, apps: Option<&Appearances>) -> Self {
        let date = data.date.date();
        let mut c = WorldCensus::default();
        for club in World::clubs(data) {
            let ladder = YouthLadder::of(club);
            let mut club_stranded = 0;
            for t in &club.teams.teams {
                let n = t.players.len();
                let key = format!("{:?}", t.team_type);
                if t.team_type.is_youth() {
                    let row = c.youth.entry(key.clone()).or_default();
                    row.0 += 1;
                    row.1 += (n == 0) as usize;
                    row.2 += (n < FIELDING) as usize;
                    row.3 += (n < MATCHDAY_SQUAD) as usize;
                    row.4 += n;
                }
                if n > 30 {
                    *c.bloated.entry(key.clone()).or_default() += 1;
                }
                for p in &t.players.players {
                    let age = p.age(date);
                    if !ladder.fits_younger(t.team_type, age) {
                        continue;
                    }
                    let home = ladder.home_for(age).unwrap();
                    let home_size = club
                        .teams
                        .teams
                        .iter()
                        .find(|x| x.team_type == home)
                        .map_or(0, |x| x.players.len());
                    if home_size < MATCHDAY_SQUAD {
                        club_stranded += 1;
                    }
                }
                if let Some(a) = apps {
                    let row = c.idle.entry(key).or_default();
                    row.0 += n;
                    row.1 += t
                        .players
                        .players
                        .iter()
                        .filter(|p| a.apps(p.id) == 0)
                        .count();
                    row.2 += (a.fixtures(t.id) == 0) as usize;
                }
            }
            if club_stranded > 0 {
                c.stranded_young += club_stranded;
                c.stranded_clubs += 1;
            }
        }
        c
    }

    fn print(&self, label: &str) {
        println!("\n--- WORLD SQUAD CENSUS: {label} ---");
        println!(
            "{:<8} {:>6} {:>6} {:>6} {:>6} {:>8}",
            "youth", "teams", "empty", "<11", "<16", "mean n"
        );
        for (k, (teams, empty, xi, md, n)) in &self.youth {
            println!(
                "{k:<8} {teams:>6} {empty:>6} {xi:>6} {md:>6} {:>8.1}",
                *n as f64 / (*teams).max(1) as f64
            );
        }
        println!(
            "boys in an older squad while their own is short (<16): {} across {} clubs",
            self.stranded_young, self.stranded_clubs
        );
        if !self.bloated.is_empty() {
            let b: Vec<String> = self
                .bloated
                .iter()
                .map(|(k, v)| format!("{k}={v}"))
                .collect();
            println!("squads above 30 players: {}", b.join(", "));
        }
        if !self.idle.is_empty() {
            println!(
                "{:<8} {:>8} {:>8} {:>7} {:>12}",
                "squad", "players", "0-apps", "idle%", "no fixtures"
            );
            for (k, (n, idle, nofix)) in &self.idle {
                println!(
                    "{k:<8} {n:>8} {idle:>8} {:>6.1}% {nofix:>12}",
                    *idle as f64 * 100.0 / (*n).max(1) as f64
                );
            }
        }
    }
}

struct Harness {
    data: SimulatorData,
}

impl Harness {
    fn generate() -> Self {
        let database = DatabaseLoader::load();
        Harness {
            data: DatabaseGenerator::generate(&database),
        }
    }

    fn tick(&mut self) -> SimulationResult {
        Self::block_on(FootballSimulator::simulate(&mut self.data))
    }

    /// `simulate` never awaits I/O, so it is ready on the first poll.
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

    fn resolve(&self, names: &[String]) -> Vec<u32> {
        let mut ids = Vec::new();
        for name in names {
            let lower = name.to_lowercase();
            let exact: Vec<&Club> = World::clubs(&self.data)
                .filter(|c| c.name.to_lowercase() == lower)
                .collect();
            let found: Vec<&Club> = if exact.is_empty() {
                World::clubs(&self.data)
                    .filter(|c| c.name.to_lowercase().contains(&lower))
                    .collect()
            } else {
                exact
            };
            if found.is_empty() {
                eprintln!("no club matches '{name}'");
            }
            ids.extend(found.iter().map(|c| c.id));
        }
        ids
    }

    fn print_watched(&self, watched: &[u32], apps: Option<(&Appearances, u32)>) {
        let date = self.data.date.date();
        for club in World::clubs(&self.data).filter(|c| watched.contains(&c.id)) {
            RosterReport::print(club, date, apps);
        }
    }

    fn run(&mut self, args: &Args) {
        let watched = self.resolve(&args.clubs);
        println!("start date {}", self.data.date.date());
        self.print_watched(&watched, None);
        WorldCensus::take(&self.data, None).print("day 0");

        let mut log = MoveLog::start(&self.data, &watched);
        let mut window = Appearances::default();
        let mut total = Appearances::default();
        let mut window_start = 1;
        let started = Instant::now();

        println!("\n--- MOVES ---");
        for day in 1..=args.days {
            let result = self.tick();
            window.record(&result);
            total.record(&result);
            log.step(&self.data, &watched, day);
            if day % args.every == 0 || day == args.days {
                self.print_watched(&watched, Some((&window, day - window_start + 1)));
                window = Appearances::default();
                window_start = day + 1;
                println!("\n--- MOVES ---");
            }
        }

        log.print_flows();
        WorldCensus::take(&self.data, Some(&total)).print(&format!(
            "after {} days ({})",
            args.days,
            self.data.date.date()
        ));
        eprintln!(
            "simulated {} days in {:.1} s",
            args.days,
            started.elapsed().as_secs_f64()
        );
    }
}

fn main() {
    env_logger::Builder::from_env(Env::default().default_filter_or("warn")).init();
    let args = Args::parse();
    eprintln!("generating world…");
    let t = Instant::now();
    let mut harness = Harness::generate();
    eprintln!("world generated in {:.1} s", t.elapsed().as_secs_f64());
    harness.run(&args);
}
