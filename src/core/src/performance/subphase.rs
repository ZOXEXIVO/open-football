//! Fine-grained timing for inner sub-phases of
//! `FootballSimulator::simulate_with`'s `ResultProcessing` and `Cleanup`
//! phases — the serial tail where the current profile shows ~85% of
//! wall time concentrated on one thread.
//!
//! The high-level `PerfPhase` covers the wall time of each top-level
//! phase, but it can't tell us whether the time is in
//! `league_result.process`, `club_result.process`, transfer market,
//! loan returns, or one of the per-country leaf functions. That makes
//! the parallelization target ambiguous. The breakdown here is a
//! diagnostic-only addition: it accumulates per-subphase nanoseconds
//! into atomics during the tick and the simulator logs the totals at
//! `end_tick`. Call sites use `ResultProcSubphase::scope(...)` for RAII
//! timing — identical pattern to `PerfCounters::scope`.

use std::sync::OnceLock;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

/// Subphases of `CountryResult::process` (one row per `data.*` call site
/// the simulator visits, batched by intent). The order matches the
/// invocation order inside `CountryResult::process` so the log output
/// reads top-to-bottom like the source.
#[derive(Clone, Copy, Debug)]
pub enum ResultProcSubphase {
    /// Per-tick build of the shared world-wide foreign player pool and
    /// global free-agent snapshot. Sits at the top of Phase C in
    /// `simulator/mod.rs` and gates every country's transfer market.
    WorldSnapshots,
    /// `simulate_media_coverage` per country.
    MediaCoverage,
    /// `process_end_of_period` per country — retirements, renewals,
    /// monthly team ticks. Excludes loan returns (see `LoanReturns`).
    EndOfPeriod,
    /// `update_country_reputation` per country.
    CountryReputation,
    /// `league_result.process` per league per country — the per-match
    /// stat / history fan-out that processes today's matches.
    LeagueProcess,
    /// `snapshot_player_season_statistics` per country — only on
    /// season-end ticks.
    SnapshotSeasonStats,
    /// `process_loan_returns` per country. Cross-country.
    LoanReturns,
    /// `enforce_squad_registration` per country — only on season-start.
    SquadRegistration,
    /// `club_result.process` per club per country — finance, board,
    /// academy, contract interactions.
    ClubProcess,
    /// `simulate_preseason_activities` per country in off-season.
    Preseason,
    /// `simulate_transfer_market` per country. Cross-country.
    TransferMarket,
    /// `simulate_international_competitions` per country.
    IntlCompetitions,
    /// `update_economic_factors` per country — monthly only.
    EconomicFactors,
}

const SLOT_COUNT: usize = 13;

fn slot(s: ResultProcSubphase) -> usize {
    match s {
        ResultProcSubphase::WorldSnapshots => 0,
        ResultProcSubphase::MediaCoverage => 1,
        ResultProcSubphase::EndOfPeriod => 2,
        ResultProcSubphase::CountryReputation => 3,
        ResultProcSubphase::LeagueProcess => 4,
        ResultProcSubphase::SnapshotSeasonStats => 5,
        ResultProcSubphase::LoanReturns => 6,
        ResultProcSubphase::SquadRegistration => 7,
        ResultProcSubphase::ClubProcess => 8,
        ResultProcSubphase::Preseason => 9,
        ResultProcSubphase::TransferMarket => 10,
        ResultProcSubphase::IntlCompetitions => 11,
        ResultProcSubphase::EconomicFactors => 12,
    }
}

fn label(s: ResultProcSubphase) -> &'static str {
    match s {
        ResultProcSubphase::WorldSnapshots => "world_snapshots",
        ResultProcSubphase::MediaCoverage => "media_coverage",
        ResultProcSubphase::EndOfPeriod => "end_of_period",
        ResultProcSubphase::CountryReputation => "country_reputation",
        ResultProcSubphase::LeagueProcess => "league_process",
        ResultProcSubphase::SnapshotSeasonStats => "snapshot_season_stats",
        ResultProcSubphase::LoanReturns => "loan_returns",
        ResultProcSubphase::SquadRegistration => "squad_registration",
        ResultProcSubphase::ClubProcess => "club_process",
        ResultProcSubphase::Preseason => "preseason",
        ResultProcSubphase::TransferMarket => "transfer_market",
        ResultProcSubphase::IntlCompetitions => "intl_competitions",
        ResultProcSubphase::EconomicFactors => "economic_factors",
    }
}

pub struct ResultProcBreakdown {
    slots: [AtomicU64; SLOT_COUNT],
}

impl ResultProcBreakdown {
    pub fn instance() -> &'static ResultProcBreakdown {
        static INSTANCE: OnceLock<ResultProcBreakdown> = OnceLock::new();
        INSTANCE.get_or_init(|| ResultProcBreakdown {
            slots: std::array::from_fn(|_| AtomicU64::new(0)),
        })
    }

    pub fn reset(&self) {
        for slot in &self.slots {
            slot.store(0, Ordering::Relaxed);
        }
    }

    pub fn add(&self, sub: ResultProcSubphase, elapsed: Duration) {
        self.slots[slot(sub)].fetch_add(elapsed.as_nanos() as u64, Ordering::Relaxed);
    }

    pub fn scope(&'static self, sub: ResultProcSubphase) -> SubphaseScope {
        SubphaseScope {
            breakdown: self,
            sub,
            start: Instant::now(),
        }
    }

    /// Total wall time captured this tick across every subphase. Sum
    /// across countries so the magnitude is comparable to the parent
    /// `ResultProcessing` PerfPhase (the parent measures wall time of
    /// the serial loop; the children measure CPU time spent inside it
    /// — they should be close on a single-threaded driver).
    pub fn total_ns(&self) -> u64 {
        self.slots
            .iter()
            .map(|s| s.load(Ordering::Relaxed))
            .sum()
    }

    /// Snapshot every subphase for logging. Allocates a small Vec; only
    /// called once per tick at `end_tick`, so the cost is negligible.
    pub fn snapshot(&self) -> Vec<(&'static str, u64)> {
        (0..SLOT_COUNT)
            .map(|i| {
                let sub = match i {
                    0 => ResultProcSubphase::WorldSnapshots,
                    1 => ResultProcSubphase::MediaCoverage,
                    2 => ResultProcSubphase::EndOfPeriod,
                    3 => ResultProcSubphase::CountryReputation,
                    4 => ResultProcSubphase::LeagueProcess,
                    5 => ResultProcSubphase::SnapshotSeasonStats,
                    6 => ResultProcSubphase::LoanReturns,
                    7 => ResultProcSubphase::SquadRegistration,
                    8 => ResultProcSubphase::ClubProcess,
                    9 => ResultProcSubphase::Preseason,
                    10 => ResultProcSubphase::TransferMarket,
                    11 => ResultProcSubphase::IntlCompetitions,
                    _ => ResultProcSubphase::EconomicFactors,
                };
                (label(sub), self.slots[i].load(Ordering::Relaxed))
            })
            .collect()
    }
}

pub struct SubphaseScope {
    breakdown: &'static ResultProcBreakdown,
    sub: ResultProcSubphase,
    start: Instant,
}

impl Drop for SubphaseScope {
    fn drop(&mut self) {
        self.breakdown.add(self.sub, self.start.elapsed());
    }
}
