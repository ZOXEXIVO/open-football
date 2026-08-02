//! Transfer-market telemetry harness.
//!
//! `.dev/simulate` answers "how fast is the world tick"; this answers "is
//! the market alive". It drives the same whole-world daily tick and then
//! walks the resulting world state, reporting the numbers a football
//! analyst would ask for:
//!
//!   * moves per season, split permanent / loan / free, with fee spread
//!   * squad turnover — how much of a squad changes in a year
//!   * the **stuck census**: senior players with zero official
//!     appearances, bucketed by the squad-asset class and squad status
//!     that decide whether any disposal path can touch them
//!   * listing lifecycle, including the `is_transfer_listed`-flag-without
//!     -a-listing-row limbo that blocks renewal *and* the market
//!   * free-agent pool health (reuses the in-core auditor)
//!
//! None of this exists in-sim: `TransferActivitySummary` is built and
//! dropped every country every tick, and the free-agent auditor only
//! emits behind `debug!`. Everything here reads public world state, so it
//! measures the shipping pipeline rather than a parallel model of it.
//!
//! Usage:
//!   cargo build --release
//!   ./target/release/dev_transfers [days] [--every N]
//!
//! Defaults to 400 days — one full season plus both windows, which is the
//! shortest horizon that answers the mobility questions, because the stuck
//! census reads the latest COMPLETED season and a shorter run may not have
//! crossed a season boundary.
//!
//! Budget for it: with the real engine that is roughly an hour and ~10 GB
//! of resident memory. Pass `--every 100` to get interim reports along the
//! way so a run can be cut short and still be worth something — and do NOT
//! pipe stdout through `grep`/`tee` into a file, which block-buffers the
//! whole run into silence; redirect directly instead.

use core::club::team::squad::{SquadAssetClass, SquadAssetContext};
use core::country::result::transfers::free_agent_audit::FreeAgentMarketAuditor;
use core::transfers::{
    TransferListingOrigin, TransferListingStatus, TransferListingType, TransferType,
};
use core::utils::DateUtils;
use core::{
    FootballSimulator, PlayerSquadStatus, PlayerStatusType, SimulationResult, SimulatorData,
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
use std::collections::{HashMap, HashSet};
use std::fmt::Write as _;
use std::future::Future;
use std::pin::pin;
use std::task::{Context, Poll, Waker};
use std::time::Instant;

/// One full season plus both transfer windows, so a report covers a
/// complete market cycle rather than a single window's activity.
const DEFAULT_DAYS: u32 = 400;

/// Youngest age counted as a senior for the stuck census. Below this a
/// zero-appearance season is normal academy progression, not a symptom.
const SENIOR_AGE: u8 = 20;

// ---------------------------------------------------------------------
// Census structures
// ---------------------------------------------------------------------

/// Where a player sits in the club hierarchy. Splits the squads that owe
/// a player a career (main, senior reserve) from the youth pyramid, since
/// only the former makes a zero-appearance year a market failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
enum SquadBand {
    Main,
    SeniorReserve,
    Youth,
}

impl SquadBand {
    fn of(team_type: TeamType) -> Self {
        if team_type == TeamType::Main {
            SquadBand::Main
        } else if team_type.is_youth() {
            SquadBand::Youth
        } else {
            SquadBand::SeniorReserve
        }
    }

    fn label(self) -> &'static str {
        match self {
            SquadBand::Main => "main",
            SquadBand::SeniorReserve => "reserve",
            SquadBand::Youth => "youth",
        }
    }
}

/// Counters over every contracted player in the world.
#[derive(Debug, Default)]
struct PlayerCensus {
    total: usize,
    /// Seniors (>= SENIOR_AGE) on a main or senior-reserve squad, not out
    /// on loan — the population the market is supposed to serve.
    senior_servable: usize,
    /// Of `senior_servable`, those with at least one completed season on
    /// record. Only these can be judged for a wasted year.
    senior_judgeable: usize,
    /// Of `senior_judgeable`, those with zero appearances in their most
    /// recent completed season.
    senior_zero_apps: usize,
    /// Appearances in the latest completed season, for the distribution.
    last_season_apps: Vec<u16>,
    /// Live (in-progress season) appearances, as a cross-check that
    /// matches are actually being recorded at all.
    live_apps_nonzero: usize,
    /// Zero-appearance seniors split by the class that gates disposal.
    zero_by_asset_class: HashMap<&'static str, usize>,
    /// …and by their formal squad status.
    zero_by_squad_status: HashMap<&'static str, usize>,
    /// …and by which squad they sit on.
    zero_by_band: HashMap<SquadBand, usize>,
    /// Zero-appearance seniors carrying no availability signal at all —
    /// not listed, not loan-listed, not requested, not unhappy. Nobody is
    /// trying to move these players and they are not asking to move.
    zero_apps_no_availability: usize,
    /// Players whose contract carries `is_transfer_listed` but who have no
    /// live listing row. The latch blocks renewal and coach termination
    /// while the market cannot see them.
    flagged_without_listing: usize,
    /// Players carrying the `Loa` badge with no loan listing row.
    loan_badge_without_listing: usize,
    /// Availability statuses across the whole contracted population.
    status_counts: HashMap<&'static str, usize>,
    /// Contracts inside their final 12 months, by whether the club still
    /// has any market action live on them.
    expiring_12m: usize,
    expiring_12m_unlisted: usize,
}

/// Moves recorded in `transfer_market.transfer_history`, deduplicated
/// across countries (a cross-border move is written on both sides).
#[derive(Debug, Default)]
struct MoveCensus {
    permanent: usize,
    loan: usize,
    free: usize,
    /// Permanent-move fees, for the spread.
    fees: Vec<f64>,
    /// Distinct (season, club) pairs seen, for turnover.
    per_club_in: HashMap<(u16, u32), usize>,
    per_club_out: HashMap<(u16, u32), usize>,
    by_season: HashMap<u16, (usize, usize, usize)>,
}

/// Live listing rows, by type and status.
#[derive(Debug, Default)]
struct ListingCensus {
    available_transfer: usize,
    available_loan: usize,
    in_negotiation: usize,
    synthetic: usize,
    /// Age in days of `Available` seller-advertised rows.
    ages: Vec<i64>,
}

#[derive(Debug, Default)]
struct MarketReport {
    players: PlayerCensus,
    moves: MoveCensus,
    listings: ListingCensus,
    live_negotiations: usize,
    clubs: usize,
    free_agents: usize,
}

// ---------------------------------------------------------------------
// Collection
// ---------------------------------------------------------------------

struct MarketCensus;

impl MarketCensus {
    /// Official appearances in the *current* (in-progress) season, read
    /// off the live counters.
    fn live_apps(player: &core::Player) -> u16 {
        player.statistics.played
            + player.statistics.played_subs
            + player.cup_statistics.played
            + player.cup_statistics.played_subs
    }

    /// Official appearances in the player's most recent **completed**
    /// season, read off the canonical ledger.
    ///
    /// The live counters are drained into the ledger at season rollover,
    /// so a census sampled just after a rollover would score every player
    /// in the world at zero. Reading the ledger's newest season instead
    /// asks the question the stalled-career logic itself asks — "did he
    /// play last year" — and is stable wherever in the calendar the
    /// sample lands. `None` for a player with no completed season yet
    /// (a first-year academy graduate), who is not a market failure.
    fn last_season_apps(player: &core::Player) -> Option<(u16, u16)> {
        let ledger = &player.statistics_history.season_ledger;
        let newest = ledger
            .iter()
            .filter(|e| e.competition_kind.counts_toward_career_history())
            .map(|e| e.season_start_year)
            .max()?;
        let apps: u16 = ledger
            .iter()
            .filter(|e| {
                e.season_start_year == newest && e.competition_kind.counts_toward_career_history()
            })
            .map(|e| e.statistics.played + e.statistics.played_subs)
            .sum();
        Some((newest, apps))
    }

    fn squad_status_label(status: &PlayerSquadStatus) -> &'static str {
        match status {
            PlayerSquadStatus::Invalid => "invalid",
            PlayerSquadStatus::NotYetSet => "not_yet_set",
            PlayerSquadStatus::KeyPlayer => "key_player",
            PlayerSquadStatus::FirstTeamRegular => "first_team_regular",
            PlayerSquadStatus::FirstTeamSquadRotation => "rotation",
            PlayerSquadStatus::MainBackupPlayer => "backup",
            PlayerSquadStatus::HotProspectForTheFuture => "hot_prospect",
            PlayerSquadStatus::DecentYoungster => "decent_youngster",
            PlayerSquadStatus::NotNeeded => "not_needed",
            PlayerSquadStatus::SquadStatusCount => "count",
        }
    }

    /// Walk the whole world and build the report.
    fn collect(data: &SimulatorData) -> MarketReport {
        let date = data.date.date();
        let mut report = MarketReport::default();
        report.free_agents = data.free_agents.len();

        // Deduplicate cross-border history rows: the same move is written
        // into both the selling and the buying country's market.
        let mut seen_moves: HashSet<(u32, NaiveDate, u32)> = HashSet::new();

        for continent in &data.continents {
            for country in &continent.countries {
                let market = &country.transfer_market;

                // ---- moves -------------------------------------------
                for t in &market.transfer_history {
                    let key = (t.player_id, t.transfer_date, t.to_club_id);
                    if !seen_moves.insert(key) {
                        continue;
                    }
                    let entry = report
                        .moves
                        .by_season
                        .entry(t.season_year)
                        .or_insert((0, 0, 0));
                    match t.transfer_type {
                        TransferType::Permanent => {
                            report.moves.permanent += 1;
                            entry.0 += 1;
                            report.moves.fees.push(t.fee.amount);
                        }
                        TransferType::Loan(_) => {
                            report.moves.loan += 1;
                            entry.1 += 1;
                        }
                        TransferType::Free => {
                            report.moves.free += 1;
                            entry.2 += 1;
                        }
                    }
                    *report
                        .moves
                        .per_club_in
                        .entry((t.season_year, t.to_club_id))
                        .or_insert(0) += 1;
                    *report
                        .moves
                        .per_club_out
                        .entry((t.season_year, t.from_club_id))
                        .or_insert(0) += 1;
                }

                // ---- listings ----------------------------------------
                // Index live rows so the player walk can tell a flagged
                // player apart from an actually-advertised one.
                let mut listed_for_transfer: HashSet<u32> = HashSet::new();
                let mut listed_for_loan: HashSet<u32> = HashSet::new();
                for l in &market.listings {
                    let live = matches!(
                        l.status,
                        TransferListingStatus::Available | TransferListingStatus::InNegotiation
                    );
                    if !live {
                        continue;
                    }
                    if l.origin == TransferListingOrigin::SyntheticUnsolicited {
                        report.listings.synthetic += 1;
                        continue;
                    }
                    match l.listing_type {
                        TransferListingType::Transfer => {
                            listed_for_transfer.insert(l.player_id);
                        }
                        TransferListingType::Loan => {
                            listed_for_loan.insert(l.player_id);
                        }
                        TransferListingType::EndOfContract => {}
                    }
                    if l.status == TransferListingStatus::InNegotiation {
                        report.listings.in_negotiation += 1;
                        continue;
                    }
                    match l.listing_type {
                        TransferListingType::Transfer => report.listings.available_transfer += 1,
                        TransferListingType::Loan => report.listings.available_loan += 1,
                        TransferListingType::EndOfContract => {}
                    }
                    report
                        .listings
                        .ages
                        .push((date - l.listed_date).num_days().max(0));
                }

                report.live_negotiations += market.negotiations.len();

                // ---- players -----------------------------------------
                for club in &country.clubs {
                    report.clubs += 1;
                    for team in club.teams.teams.iter() {
                        let band = SquadBand::of(team.team_type);
                        // Classify against the player's own squad: that is
                        // the roster whose depth and level decide whether
                        // any sweep may act on him.
                        let ctx = SquadAssetContext::for_squad(&team.players);
                        for player in team.players.iter() {
                            let Some(contract) = player.contract.as_ref() else {
                                continue;
                            };
                            report.players.total += 1;

                            for status in player.statuses.statuses.iter() {
                                let label = match status.status {
                                    PlayerStatusType::Lst => Some("Lst"),
                                    PlayerStatusType::Loa => Some("Loa"),
                                    PlayerStatusType::Req => Some("Req"),
                                    PlayerStatusType::Unh => Some("Unh"),
                                    PlayerStatusType::Wnt => Some("Wnt"),
                                    PlayerStatusType::Bid => Some("Bid"),
                                    _ => None,
                                };
                                if let Some(label) = label {
                                    *report
                                        .players
                                        .status_counts
                                        .entry(label)
                                        .or_insert(0) += 1;
                                }
                            }

                            let has_transfer_row = listed_for_transfer.contains(&player.id);
                            let has_loan_row = listed_for_loan.contains(&player.id);
                            if contract.is_transfer_listed && !has_transfer_row {
                                report.players.flagged_without_listing += 1;
                            }
                            if player.statuses.has(PlayerStatusType::Loa) && !has_loan_row {
                                report.players.loan_badge_without_listing += 1;
                            }

                            let months_left =
                                (contract.expiration - date).num_days() as f32 / 30.0;
                            if months_left > 0.0 && months_left <= 12.0 {
                                report.players.expiring_12m += 1;
                                if !has_transfer_row && !has_loan_row {
                                    report.players.expiring_12m_unlisted += 1;
                                }
                            }

                            let age = DateUtils::age(player.birth_date, date);
                            let servable = age >= SENIOR_AGE
                                && band != SquadBand::Youth
                                && !player.is_on_loan();
                            if !servable {
                                continue;
                            }
                            report.players.senior_servable += 1;
                            if Self::live_apps(player) > 0 {
                                report.players.live_apps_nonzero += 1;
                            }

                            let Some((_, apps)) = Self::last_season_apps(player) else {
                                // No completed season on record yet — a
                                // first-year graduate is not a stuck career.
                                continue;
                            };
                            report.players.senior_judgeable += 1;
                            report.players.last_season_apps.push(apps);
                            if apps > 0 {
                                continue;
                            }
                            report.players.senior_zero_apps += 1;

                            let class = ctx.classify(player, date);
                            *report
                                .players
                                .zero_by_asset_class
                                .entry(class.label())
                                .or_insert(0) += 1;
                            *report
                                .players
                                .zero_by_squad_status
                                .entry(Self::squad_status_label(&contract.squad_status))
                                .or_insert(0) += 1;
                            *report.players.zero_by_band.entry(band).or_insert(0) += 1;

                            let any_availability = player.statuses.has(PlayerStatusType::Lst)
                                || player.statuses.has(PlayerStatusType::Loa)
                                || player.statuses.has(PlayerStatusType::Req)
                                || player.statuses.has(PlayerStatusType::Unh)
                                || has_transfer_row
                                || has_loan_row;
                            if !any_availability {
                                report.players.zero_apps_no_availability += 1;
                            }
                        }
                    }
                }
            }
        }

        report
    }
}

// ---------------------------------------------------------------------
// Rendering
// ---------------------------------------------------------------------

struct ReportPrinter;

impl ReportPrinter {
    fn pct(part: usize, whole: usize) -> f64 {
        if whole == 0 {
            0.0
        } else {
            part as f64 * 100.0 / whole as f64
        }
    }

    fn top(map: &HashMap<&'static str, usize>, take: usize) -> String {
        let mut rows: Vec<(&&str, &usize)> = map.iter().collect();
        rows.sort_by(|a, b| b.1.cmp(a.1).then_with(|| a.0.cmp(b.0)));
        let mut out = String::new();
        for (k, v) in rows.into_iter().take(take) {
            if !out.is_empty() {
                out.push_str(", ");
            }
            let _ = write!(out, "{k}={v}");
        }
        if out.is_empty() {
            out.push_str("none");
        }
        out
    }

    fn median(values: &mut Vec<f64>) -> f64 {
        if values.is_empty() {
            return 0.0;
        }
        values.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        values[values.len() / 2]
    }

    fn median_i64(values: &mut Vec<i64>) -> i64 {
        if values.is_empty() {
            return 0;
        }
        values.sort_unstable();
        values[values.len() / 2]
    }

    fn print(report: &mut MarketReport, data: &SimulatorData, day: u32) {
        let date = data.date.date();
        let m = &report.moves;
        let total_moves = m.permanent + m.loan + m.free;

        println!("\n================ TRANSFER MARKET REPORT ================");
        println!("day {day}  {date}   clubs={}", report.clubs);

        println!("\n-- moves (cumulative, deduped) --");
        println!(
            "total {total_moves}  |  permanent {} ({:.1}%)  loan {} ({:.1}%)  free {} ({:.1}%)",
            m.permanent,
            Self::pct(m.permanent, total_moves),
            m.loan,
            Self::pct(m.loan, total_moves),
            m.free,
            Self::pct(m.free, total_moves),
        );
        let mut fees = m.fees.clone();
        let paid: Vec<f64> = fees.iter().copied().filter(|f| *f > 0.0).collect();
        println!(
            "permanent fees: {} with a fee, median {:.0}, max {:.0}",
            paid.len(),
            Self::median(&mut fees),
            fees.iter().copied().fold(0.0f64, f64::max),
        );

        let mut seasons: Vec<(&u16, &(usize, usize, usize))> = m.by_season.iter().collect();
        seasons.sort_by_key(|(y, _)| **y);
        for (year, (p, l, f)) in seasons {
            println!("  season {year}: permanent {p}, loan {l}, free {f}");
        }

        // Turnover: mean incoming moves per club per season, as a share
        // of a nominal 25-man senior squad.
        if !m.per_club_in.is_empty() {
            let season_count = m.by_season.len().max(1);
            let total_in: usize = m.per_club_in.values().sum();
            let mean_in = total_in as f64 / (report.clubs.max(1) * season_count) as f64;
            let total_out: usize = m.per_club_out.values().sum();
            let mean_out = total_out as f64 / (report.clubs.max(1) * season_count) as f64;
            println!(
                "turnover: {mean_in:.1} in / {mean_out:.1} out per club per season  \
                 (~{:.0}% of a 25-man squad)",
                mean_in * 100.0 / 25.0,
            );
        }

        println!("\n-- listings --");
        println!(
            "available: {} transfer, {} loan  |  in negotiation {}  |  synthetic {}",
            report.listings.available_transfer,
            report.listings.available_loan,
            report.listings.in_negotiation,
            report.listings.synthetic,
        );
        println!(
            "live negotiations {}  |  median listing age {}d",
            report.live_negotiations,
            Self::median_i64(&mut report.listings.ages),
        );

        let p = &report.players;
        println!("\n-- population --");
        println!(
            "contracted {}  |  senior servable {}  |  free agents {}",
            p.total, p.senior_servable, report.free_agents
        );
        println!("statuses: {}", Self::top(&p.status_counts, 8));

        println!("\n-- STUCK CENSUS (senior, non-loan, latest COMPLETED season) --");
        let mut apps_dist = p.last_season_apps.clone();
        apps_dist.sort_unstable();
        let median_apps = if apps_dist.is_empty() {
            0
        } else {
            apps_dist[apps_dist.len() / 2]
        };
        println!(
            "judgeable {} of {} servable  |  median apps last season {median_apps}  \
             |  playing right now {} ({:.1}%)",
            p.senior_judgeable,
            p.senior_servable,
            p.live_apps_nonzero,
            Self::pct(p.live_apps_nonzero, p.senior_servable),
        );
        println!(
            "zero-app seniors: {} ({:.1}% of judgeable)",
            p.senior_zero_apps,
            Self::pct(p.senior_zero_apps, p.senior_judgeable),
        );
        println!(
            "  of which NO availability signal at all: {} ({:.1}% of zero-app)",
            p.zero_apps_no_availability,
            Self::pct(p.zero_apps_no_availability, p.senior_zero_apps),
        );
        println!("  by asset class:  {}", Self::top(&p.zero_by_asset_class, 8));
        println!(
            "  by squad status: {}",
            Self::top(&p.zero_by_squad_status, 8)
        );
        let mut bands: Vec<(&SquadBand, &usize)> = p.zero_by_band.iter().collect();
        bands.sort_by_key(|(b, _)| **b);
        let band_line: Vec<String> = bands
            .iter()
            .map(|(b, n)| format!("{}={}", b.label(), n))
            .collect();
        println!("  by squad band:   {}", band_line.join(", "));

        println!("\n-- limbo --");
        println!(
            "is_transfer_listed flag with NO listing row: {}",
            p.flagged_without_listing
        );
        println!(
            "Loa badge with NO loan listing row:          {}",
            p.loan_badge_without_listing
        );
        println!(
            "expiring within 12m: {} ({} with no market action = {:.1}%)",
            p.expiring_12m,
            p.expiring_12m_unlisted,
            Self::pct(p.expiring_12m_unlisted, p.expiring_12m),
        );

        let fa = FreeAgentMarketAuditor::aggregate(data, date);
        println!("\n-- free-agent pool --");
        let buckets: Vec<String> = fa
            .buckets
            .iter()
            .map(|b| format!("{}:{} (cp {:.2})", b.label, b.count, b.avg_career_pressure))
            .collect();
        println!("total {}  |  {}", fa.total, buckets.join("  "));
        let reasons: Vec<String> = fa
            .block_reason_counts
            .iter()
            .take(6)
            .map(|(r, n)| format!("{}={}", r.label(), n))
            .collect();
        println!(
            "top block reasons: {}",
            if reasons.is_empty() {
                "none".to_string()
            } else {
                reasons.join(", ")
            }
        );
        println!(
            "flow this month: {} global + {} domestic-expiry + {} pre-contract signed; \
             {} released, {} retired",
            fa.flow.signed_from_global_pool,
            fa.flow.signed_same_country_expired,
            fa.flow.signed_pre_contract,
            fa.flow.released_to_pool,
            fa.flow.retired_from_pool,
        );
        println!("=======================================================\n");
    }
}

// ---------------------------------------------------------------------
// Harness
// ---------------------------------------------------------------------

struct SimHarness {
    data: SimulatorData,
}

impl SimHarness {
    fn generate() -> Self {
        let database = DatabaseLoader::load();
        let data = DatabaseGenerator::generate(&database);
        SimHarness { data }
    }

    fn tick(&mut self) -> SimulationResult {
        Self::block_on(FootballSimulator::simulate(&mut self.data))
    }

    /// See `.dev/simulate`: the simulate future never awaits an I/O point,
    /// so a no-op waker completes it on the first poll.
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
                let mut report = MarketCensus::collect(&self.data);
                ReportPrinter::print(&mut report, &self.data, day);
            }
        }
        let mut report = MarketCensus::collect(&self.data);
        ReportPrinter::print(&mut report, &self.data, days);
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

    // Report the world as generated, so every later number reads against
    // its starting point rather than against zero.
    let mut initial = MarketCensus::collect(&harness.data);
    ReportPrinter::print(&mut initial, &harness.data, 0);

    harness.run(days, every);
}

/// Silences the unused-import warning for `NaiveDate` on builds where the
/// census helpers are inlined away; the type is part of the public
/// signatures above.
#[allow(dead_code)]
fn _type_anchor(_: NaiveDate, _: SquadAssetClass) {}
