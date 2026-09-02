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

use core::club::player::core::player::TransferRequestReason;
use core::club::player::transfer::{BigStagePull, BigStagePullContext};
use core::club::team::squad::{SquadAssetClass, SquadAssetContext};
use core::country::result::transfers::free_agent_audit::FreeAgentMarketAuditor;
use core::club::player::statistics::StuckCareerScan;
use core::transfers::ScoutingRegion;
use core::transfers::pipeline::appraisal::TermsRefusalCause;
use core::transfers::pipeline::planning::BriefTier;
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
use std::cmp::Ordering;
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

    // ── Big-stage ambition ────────────────────────────────────────────
    /// Seniors whose big-stage pull has reached each of the three tiers.
    /// The silent tier should be much the largest: most good players in a
    /// sub-elite league would listen, few agitate, fewer still demand.
    stage_inclined: usize,
    stage_mood: usize,
    stage_requesting: usize,
    /// Inclined players by the reputation band of the league they are in,
    /// so an inspection can tell "the mid-tier leagues are restless" from
    /// "everybody everywhere wants out".
    stage_inclined_by_league_band: HashMap<&'static str, usize>,

    // ── Parked primes ─────────────────────────────────────────────────
    /// Players at/over `PARKED_PRIME_AGE` sitting on a B / Second /
    /// Reserve squad. This is the population the reserve trap used to
    /// hold indefinitely.
    parked_prime: usize,
    /// Of those, how many carry any market signal at all — the share the
    /// club or the player is actually trying to resolve.
    parked_prime_with_availability: usize,
}

/// Age from which a senior reserve squad has stopped being development.
/// Mirrors the club-side resolution pass.
const PARKED_PRIME_AGE: u8 = 24;

/// Tier bars, mirrored from [`core::BigStagePullConfig`] defaults so the
/// harness reports the same three tiers the simulation acts on.
const STAGE_INCLINED_BAR: f32 = 0.22;
const STAGE_MOOD_BAR: f32 = 0.40;

/// The exporter bands — the leagues a standout is supposed to be bought
/// OUT of. Turkey, Portugal, the Netherlands, Belgium, Russia, Brazil and
/// Argentina all sit inside this window.
const STANDOUT_LEAGUE_MIN: u16 = 5_500;
const STANDOUT_LEAGUE_MAX: u16 = 8_499;
/// Age window of the classic step-up move, measured off the real market
/// (Ünder 20, Yazıcı 22, Kadıoğlu 24, Aktürkoğlu 25, Tosun 26).
const STANDOUT_AGE_MIN: u8 = 21;
const STANDOUT_AGE_MAX: u8 = 27;
/// `BigStagePull::standing` at or above which a player reads as a genuine
/// standout for his league rather than merely a starter in it.
const STANDOUT_STANDING_BAR: f32 = 0.6;

/// Coarse league-strength label for the ambition breakdown.
fn league_band(reputation: u16) -> &'static str {
    match reputation {
        r if r >= 8_500 => "elite 8500+",
        r if r >= 7_000 => "strong 7000-8499",
        r if r >= 5_500 => "mid 5500-6999",
        r if r >= 4_000 => "modest 4000-5499",
        0 => "unknown",
        _ => "weak <4000",
    }
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

/// Who lends to whom, and how repetitively.
///
/// The market can post healthy loan *volume* and still be a fixed
/// permutation underneath — the same parent sending its prospects to the
/// same borrower every window, because the pipeline chose destinations with
/// an argmax over a number that barely moves. Volume alone cannot see that;
/// these counters can.
#[derive(Debug, Default)]
struct LoanFlowCensus {
    /// Loans out per (parent, borrower) pair.
    pairs: HashMap<(u32, u32), usize>,
    /// Total loans out per parent club.
    per_parent: HashMap<u32, usize>,
    /// Total loans in per borrowing club.
    per_borrower: HashMap<u32, usize>,
}

impl LoanFlowCensus {
    /// Parents that lent at least this often are the ones whose habits are
    /// worth reading — below it, "all my loans went to one club" is small
    /// numbers rather than a pattern.
    const MIN_LOANS_FOR_HABIT: usize = 3;

    fn record(&mut self, from_club_id: u32, to_club_id: u32) {
        *self.pairs.entry((from_club_id, to_club_id)).or_insert(0) += 1;
        *self.per_parent.entry(from_club_id).or_insert(0) += 1;
        *self.per_borrower.entry(to_club_id).or_insert(0) += 1;
    }

    /// Mean share of a lending club's loans that landed at its single
    /// most-used destination, over parents past [`Self::MIN_LOANS_FOR_HABIT`].
    /// 1.0 means every such parent used exactly one borrower; the lower the
    /// better, floored by how many plausible destinations actually exist.
    fn mean_top_destination_share(&self) -> f64 {
        let mut shares = Vec::new();
        for (parent, total) in &self.per_parent {
            if *total < Self::MIN_LOANS_FOR_HABIT {
                continue;
            }
            let top = self
                .pairs
                .iter()
                .filter(|((from, _), _)| from == parent)
                .map(|(_, n)| *n)
                .max()
                .unwrap_or(0);
            shares.push(top as f64 / *total as f64);
        }
        if shares.is_empty() {
            return 0.0;
        }
        shares.iter().sum::<f64>() / shares.len() as f64
    }

    /// Mean number of distinct destinations used by those same parents.
    fn mean_distinct_destinations(&self) -> f64 {
        let mut counts = Vec::new();
        for (parent, total) in &self.per_parent {
            if *total < Self::MIN_LOANS_FOR_HABIT {
                continue;
            }
            counts.push(self.pairs.keys().filter(|(from, _)| from == parent).count() as f64);
        }
        if counts.is_empty() {
            return 0.0;
        }
        counts.iter().sum::<f64>() / counts.len() as f64
    }

    /// Pairs that repeated at least `n` times — the literal "this club keeps
    /// taking that club's players" reading.
    fn pairs_repeating(&self, n: usize) -> usize {
        self.pairs.values().filter(|c| **c >= n).count()
    }

    /// Share of all loans absorbed by the single busiest borrower in the
    /// world. A market with one universal destination shows up here.
    fn busiest_borrower_share(&self) -> f64 {
        let total: usize = self.per_borrower.values().sum();
        if total == 0 {
            return 0.0;
        }
        self.per_borrower.values().copied().max().unwrap_or(0) as f64 / total as f64
    }
}

// ---------------------------------------------------------------------
// Springboard census
// ---------------------------------------------------------------------

/// What the world knows about one club, resolved once so the move walk and
/// the cohort walk can both ask questions about clubs that live in a
/// different country from the row they are reading.
#[derive(Debug, Clone)]
struct ClubFacts {
    name: String,
    country_id: u32,
    /// Main-team league reputation, 0..10000. Zero when the club has no
    /// registered senior league.
    league_reputation: u16,
    league_id: Option<u32>,
    league_name: String,
    /// Annualised trailing income — never the raw trailing sum (memory
    /// `club_finance_fm_rebuild`).
    annual_income: i64,
    balance: i64,
    transfer_budget: f64,
}

impl ClubFacts {
    fn band(&self) -> &'static str {
        league_band(self.league_reputation)
    }
}

/// Ordered strength of the league bands, so "moved up a band" is a
/// comparison rather than a table of special cases.
struct BandLadder;

impl BandLadder {
    /// Rungs, weakest first. `league_band` returns one of these labels.
    const RUNGS: [&'static str; 6] = [
        "unknown",
        "weak <4000",
        "modest 4000-5499",
        "mid 5500-6999",
        "strong 7000-8499",
        "elite 8500+",
    ];

    fn rung(label: &str) -> usize {
        Self::RUNGS.iter().position(|r| *r == label).unwrap_or(0)
    }

    /// True when `to` sits strictly above `from`. "unknown" never counts as
    /// a step in either direction — it is missing data, not a weak league.
    fn is_step_up(from: &str, to: &str) -> bool {
        if from == "unknown" || to == "unknown" {
            return false;
        }
        Self::rung(to) > Self::rung(from)
    }
}

/// One permanent move, reduced to the facts the flow matrix needs.
#[derive(Debug, Clone)]
struct FlowMove {
    season: u16,
    from_band: &'static str,
    to_band: &'static str,
    fee: f64,
    age: Option<u8>,
    /// Fee ÷ the SELLING club's annual income — the number that actually
    /// decides these deals in real life.
    fee_over_seller_income: Option<f64>,
    /// The selling club's league, for the exporter-drain table.
    from_league: Option<(u32, String)>,
    /// True when the two clubs sit in different countries. The springboard
    /// is a CROSS-BORDER phenomenon, and a domestic promotion between
    /// bands is a different market; counting them together would let a
    /// healthy domestic ladder hide a border that nothing crosses.
    cross_border: bool,
}

/// Permanent moves bucketed by `(from band → to band)`, the two-cell
/// question this whole campaign turns on: `strong → elite` and
/// `mid → elite/strong`.
#[derive(Debug, Default)]
struct FlowMatrix {
    moves: Vec<FlowMove>,
    /// Distinct seasons seen, so per-season rates are honest.
    seasons: HashSet<u16>,
}

impl FlowMatrix {
    /// Age band of the classic step-up move — the population the calibration
    /// targets are written against.
    const STEP_UP_AGE_MIN: u8 = 21;
    const STEP_UP_AGE_MAX: u8 = 27;

    fn record(&mut self, m: FlowMove) {
        self.seasons.insert(m.season);
        self.moves.push(m);
    }

    fn season_count(&self) -> usize {
        self.seasons.len().max(1)
    }

    /// Every move from a named band to a named band.
    fn cell(&self, from: &str, to: &str) -> Vec<&FlowMove> {
        self.moves
            .iter()
            .filter(|m| m.from_band == from && m.to_band == to)
            .collect()
    }

    /// Step-up moves by a player inside the classic age window, out of the
    /// exporter bands. The cohort the targets are about.
    fn step_ups(&self) -> Vec<&FlowMove> {
        self.moves
            .iter()
            .filter(|m| {
                BandLadder::is_step_up(m.from_band, m.to_band)
                    && m.age.is_none_or(|a| {
                        (Self::STEP_UP_AGE_MIN..=Self::STEP_UP_AGE_MAX).contains(&a)
                    })
            })
            .collect()
    }
}

/// The population the springboard is supposed to serve: a standout in a
/// sub-elite league, young enough to be bought, playing senior football.
#[derive(Debug, Default)]
struct StandoutCohort {
    /// Cohort size.
    total: usize,
    /// Of those, how many at least one club in ANOTHER country is actively
    /// carrying on its books — a monitoring row, a standing recommendation,
    /// a shortlist place or a live negotiation. This is the funnel's first
    /// stage made visible: before this work, a contented standout abroad
    /// generated none of the four.
    with_foreign_file: usize,
    /// …and how many have a live cross-border negotiation right now.
    with_live_foreign_bid: usize,
    /// Ages, for the distribution.
    ages: Vec<u8>,
    /// Cohort members per exporter league, so the drain table can report a
    /// rate rather than a raw count.
    per_league: HashMap<u32, (String, usize)>,
}

/// Balance-sheet hoarding — money that is never spendable is a finance leak
/// of its own, and the budget ceiling is the thing that decides whether it
/// ever reaches the market.
#[derive(Debug, Clone)]
struct HoardRow {
    name: String,
    balance: i64,
    annual_income: i64,
    transfer_budget: f64,
    /// Fees paid for players signed in the most recent recorded season.
    gross_spend: f64,
}

/// The live-market instruments: what the seven planning / knowledge /
/// pricing layers actually produced, as distinct from how many players
/// moved.
///
/// Move totals alone cannot tell a market from a procurement queue — a
/// queue that processes the same requests faster posts the same totals.
/// These read the state the new layers write: whether clubs are planning at
/// all, whether their boards diverge, whether anything was quietly for sale,
/// and whether the price of money moved.
#[derive(Debug, Default)]
struct LiveMarketCensus {
    /// Clubs carrying a written recruitment brief, and the tier mix of the
    /// slots on them.
    clubs_with_brief: usize,
    briefed_slots: usize,
    tier_a: usize,
    tier_b: usize,
    tier_c: usize,
    /// Money slack recorded on each brief — the axis the appetite runs on.
    money_slack: Vec<f32>,
    /// Watchlist depth per club, and the names the biggest clubs carry, so
    /// the herd failure mode is measurable.
    watchlist_sizes: Vec<usize>,
    /// (club reputation, set of watched player ids) for the top clubs.
    top_club_boards: Vec<(u16, HashSet<u32>)>,
    /// Sell-list depth per club, and how many entries are marketed.
    sell_list_sizes: Vec<usize>,
    marketed_players: usize,
    /// Senior squad sizes against the board's registered cap.
    squad_sizes: Vec<usize>,
    squads_over_cap: usize,
    /// Registered-foreigner violations against the country's own rules —
    /// the P5 acceptance metric, which must be zero.
    regulation_violations: usize,
    /// Price level per country, keyed by name.
    price_levels: Vec<(String, f32)>,
    /// Cumulative story counters, summed across countries.
    contested_agreements: u32,
    deadline_agreements: u32,
    sell_list_conversions: u32,
    agent_led_approaches: u32,
    /// Gross fee spend per league in the most recent recorded season.
    gross_spend_by_league: HashMap<u32, (String, f64)>,
    /// Ages at fee moves of at least ten million.
    big_move_ages: Vec<u8>,
}

impl LiveMarketCensus {
    /// How many of the biggest clubs' boards to compare for overlap. Twenty
    /// is the P2 acceptance cohort.
    const OVERLAP_CLUBS: usize = 20;
    /// Fee at which a move is a headline one — the band whose median age
    /// the real market reports at about 24.
    const BIG_MOVE_FEE: f64 = 10_000_000.0;

    /// Mean pairwise overlap between the top clubs' watchlists, as a share
    /// of the smaller board. The herd failure mode is this number going up:
    /// if every rich club watches the same eight names the market is one
    /// auction and the losers buy nothing.
    fn board_overlap(&self) -> Option<f64> {
        let mut boards: Vec<&HashSet<u32>> = self
            .top_club_boards
            .iter()
            .filter(|(_, b)| !b.is_empty())
            .map(|(_, b)| b)
            .collect();
        if boards.len() < 2 {
            return None;
        }
        boards.truncate(Self::OVERLAP_CLUBS);
        let mut total = 0.0;
        let mut pairs = 0usize;
        for i in 0..boards.len() {
            for j in (i + 1)..boards.len() {
                let shared = boards[i].intersection(boards[j]).count() as f64;
                let smaller = boards[i].len().min(boards[j].len()) as f64;
                if smaller > 0.0 {
                    total += shared / smaller;
                    pairs += 1;
                }
            }
        }
        (pairs > 0).then(|| total / pairs as f64)
    }
}

#[derive(Debug, Default)]
struct MarketReport {
    players: PlayerCensus,
    moves: MoveCensus,
    listings: ListingCensus,
    loan_flow: LoanFlowCensus,
    flow: FlowMatrix,
    cohort: StandoutCohort,
    hoard: Vec<HoardRow>,
    live_negotiations: usize,
    clubs: usize,
    free_agents: usize,
    live: LiveMarketCensus,
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

    /// Resolve every club in the world once, plus the two cross-country
    /// indexes the springboard census needs.
    ///
    /// A transfer-history row names two club ids and nothing else, and the
    /// clubs it names routinely live in different countries from the row.
    /// Without a world-wide index, "did this move go up a band?" and "what
    /// was the fee worth to the seller?" are unanswerable — which is why
    /// the report could count moves and say nothing about direction.
    fn world_index(
        data: &SimulatorData,
        date: NaiveDate,
    ) -> (
        HashMap<u32, ClubFacts>,
        HashMap<u32, NaiveDate>,
        HashMap<u32, HashSet<u32>>,
    ) {
        let mut clubs: HashMap<u32, ClubFacts> = HashMap::new();
        let mut birth_dates: HashMap<u32, NaiveDate> = HashMap::new();
        // player_id → the country ids of clubs carrying him on their books.
        let mut watchers: HashMap<u32, HashSet<u32>> = HashMap::new();

        for continent in &data.continents {
            for country in &continent.countries {
                for club in &country.clubs {
                    let league = club
                        .teams
                        .main()
                        .and_then(|t| t.league_id)
                        .and_then(|lid| country.leagues.leagues.iter().find(|l| l.id == lid));
                    clubs.insert(
                        club.id,
                        ClubFacts {
                            name: club.name.clone(),
                            country_id: country.id,
                            league_reputation: league.map(|l| l.reputation).unwrap_or(0),
                            league_id: league.map(|l| l.id),
                            league_name: league
                                .map(|l| l.name.clone())
                                .unwrap_or_else(|| "—".to_string()),
                            annual_income: club.finance.estimated_annual_income(date),
                            balance: club.finance.balance.balance,
                            transfer_budget: club
                                .finance
                                .transfer_budget
                                .as_ref()
                                .map(|b| b.amount)
                                .unwrap_or(0.0),
                        },
                    );

                    // Everything this club is carrying on another player.
                    let plan = &club.transfer_plan;
                    let watched = plan
                        .scout_monitoring
                        .iter()
                        .map(|m| m.player_id)
                        .chain(plan.staff_recommendations.iter().map(|r| r.player_id))
                        .chain(
                            plan.shortlists
                                .iter()
                                .flat_map(|s| s.candidates.iter().map(|c| c.player_id)),
                        );
                    for player_id in watched {
                        watchers.entry(player_id).or_default().insert(country.id);
                    }

                    for team in &club.teams.teams {
                        for player in team.players.iter() {
                            birth_dates.insert(player.id, player.birth_date);
                        }
                    }
                }
                // A live negotiation is the strongest form of "carrying him".
                for negotiation in country.transfer_market.negotiations.values() {
                    watchers
                        .entry(negotiation.player_id)
                        .or_default()
                        .insert(country.id);
                }
            }
        }
        (clubs, birth_dates, watchers)
    }

    /// Walk the whole world and build the report.
    fn collect(data: &SimulatorData) -> MarketReport {
        let date = data.date.date();
        let mut report = MarketReport::default();
        report.free_agents = data.free_agents.len();
        let (club_facts, birth_dates, watchers) = Self::world_index(data, date);
        // Gross spend per buying club in the most recent recorded season —
        // the "does the money actually leave the building?" half of the
        // hoard line.
        let mut gross_spend: HashMap<u32, f64> = HashMap::new();
        let mut live_negotiation_countries: HashMap<u32, HashSet<u32>> = HashMap::new();
        for continent in &data.continents {
            for country in &continent.countries {
                for negotiation in country.transfer_market.negotiations.values() {
                    live_negotiation_countries
                        .entry(negotiation.player_id)
                        .or_default()
                        .insert(country.id);
                }
            }
        }

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
                            // Direction of travel — the whole springboard
                            // question. Both clubs are resolved through the
                            // world index because either can live in a
                            // different country from this history row.
                            let from = club_facts.get(&t.from_club_id);
                            let to = club_facts.get(&t.to_club_id);
                            *gross_spend.entry(t.to_club_id).or_insert(0.0) += t.fee.amount;
                            // Gross spend belongs to the BUYER's league —
                            // that is the market whose price level and
                            // budgets it reflects.
                            if let Some(buyer) = to {
                                if let Some(league_id) = buyer.league_id {
                                    let row = report
                                        .live
                                        .gross_spend_by_league
                                        .entry(league_id)
                                        .or_insert_with(|| (buyer.league_name.clone(), 0.0));
                                    row.1 += t.fee.amount;
                                }
                            }
                            // The headline band: real markets put its
                            // median age at about 24, with 90 % between 19
                            // and 29. A drift upward means clubs are buying
                            // the finished article instead of an asset.
                            if t.fee.amount >= LiveMarketCensus::BIG_MOVE_FEE {
                                if let Some(age) = birth_dates
                                    .get(&t.player_id)
                                    .map(|b| DateUtils::age(*b, t.transfer_date))
                                {
                                    report.live.big_move_ages.push(age);
                                }
                            }
                            report.flow.record(FlowMove {
                                season: t.season_year,
                                from_band: from.map(|c| c.band()).unwrap_or("unknown"),
                                to_band: to.map(|c| c.band()).unwrap_or("unknown"),
                                fee: t.fee.amount,
                                age: birth_dates
                                    .get(&t.player_id)
                                    .map(|b| DateUtils::age(*b, t.transfer_date)),
                                fee_over_seller_income: from
                                    .filter(|c| c.annual_income > 0)
                                    .map(|c| t.fee.amount / c.annual_income as f64),
                                from_league: from.and_then(|c| {
                                    c.league_id.map(|id| (id, c.league_name.clone()))
                                }),
                                cross_border: match (from, to) {
                                    (Some(f), Some(t)) => f.country_id != t.country_id,
                                    _ => false,
                                },
                            });
                        }
                        TransferType::Loan(_) => {
                            report.moves.loan += 1;
                            entry.1 += 1;
                            report.loan_flow.record(t.from_club_id, t.to_club_id);
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
                // ---- the live-market layers --------------------------
                // Everything the planner, the watchlist, the ledger and
                // the pricing model wrote this tick. Read before the
                // player walk so a club that produced nothing still shows
                // up as a zero rather than as an absence.
                let story = &market.story;
                report.live.contested_agreements += story.contested_agreements;
                report.live.deadline_agreements += story.deadline_agreements;
                report.live.sell_list_conversions += story.sell_list_conversions;
                report.live.agent_led_approaches += story.agent_led_approaches;
                report
                    .live
                    .price_levels
                    .push((country.name.clone(), country.settings.pricing.price_level));

                for club in &country.clubs {
                    let plan = &club.transfer_plan;
                    if let Some(brief) = plan.brief.as_ref() {
                        report.live.clubs_with_brief += 1;
                        report.live.briefed_slots += brief.slots.len();
                        report.live.money_slack.push(brief.money_slack);
                        for slot in brief.slots.iter() {
                            match slot.tier {
                                BriefTier::A => report.live.tier_a += 1,
                                BriefTier::B => report.live.tier_b += 1,
                                BriefTier::C => report.live.tier_c += 1,
                            }
                        }
                    }
                    report.live.watchlist_sizes.push(plan.watchlist.len());
                    report.live.sell_list_sizes.push(plan.sell_list.len());
                    report.live.marketed_players +=
                        plan.sell_list.iter().filter(|e| e.is_marketed()).count();

                    let Some(main) = club.teams.main() else {
                        continue;
                    };
                    report.live.top_club_boards.push((
                        main.reputation.world,
                        plan.watchlist.iter().map(|e| e.player_id).collect(),
                    ));

                    // Squad size against the board's own registered cap,
                    // and the country's foreigner rule. Both are P5
                    // acceptance metrics and both must stay clean.
                    let senior: Vec<&core::Player> =
                        main.players.iter().filter(|p| !p.is_on_loan()).collect();
                    report.live.squad_sizes.push(senior.len());
                    if let Some(cap) = club.board.season_targets.as_ref().map(|t| t.max_squad_size)
                    {
                        if cap > 0 && senior.len() > cap as usize {
                            report.live.squads_over_cap += 1;
                        }
                    }
                    if !country
                        .regulations
                        .omitted_for_foreign_limit(&senior, country.id)
                        .is_empty()
                    {
                        report.live.regulation_violations += 1;
                    }
                }

                // ---- players -----------------------------------------
                for club in &country.clubs {
                    report.clubs += 1;
                    // Strength of the stage this club competes on — the
                    // yardstick the big-stage pull is measured against.
                    let club_league_reputation = club
                        .teams
                        .main()
                        .and_then(|t| t.league_id)
                        .and_then(|lid| country.leagues.leagues.iter().find(|l| l.id == lid))
                        .map(|l| l.reputation)
                        .unwrap_or(0);
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
                                    *report.players.status_counts.entry(label).or_insert(0) += 1;
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

                            let months_left = (contract.expiration - date).num_days() as f32 / 30.0;
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

                            // ── Springboard standout cohort ──
                            // A first-team player in one of the exporter
                            // bands who is plainly above his league's own
                            // starter level, young enough to be bought.
                            // This is the population every calibration
                            // target in the campaign is written against.
                            if band == SquadBand::Main
                                && (STANDOUT_LEAGUE_MIN..=STANDOUT_LEAGUE_MAX)
                                    .contains(&club_league_reputation)
                                && (STANDOUT_AGE_MIN..=STANDOUT_AGE_MAX).contains(&age)
                            {
                                // Neutral context: the harness measures
                                // STANDING (how far above his league he is),
                                // which the boyhood-club and isolation terms
                                // do not touch. Reusing the shipped model
                                // keeps the census and the simulation from
                                // disagreeing about who a standout is.
                                let assessed = BigStagePull::assess(
                                    player,
                                    date,
                                    &BigStagePullContext {
                                        league_reputation: club_league_reputation,
                                        continentally_isolated: false,
                                        squad_tier: TeamType::Main,
                                        at_favourite_club: false,
                                    },
                                );
                                if assessed.standing >= STANDOUT_STANDING_BAR {
                                    report.cohort.total += 1;
                                    report.cohort.ages.push(age);
                                    let foreign_watchers = watchers
                                        .get(&player.id)
                                        .map(|set| set.iter().any(|c| *c != country.id))
                                        .unwrap_or(false);
                                    if foreign_watchers {
                                        report.cohort.with_foreign_file += 1;
                                    }
                                    let foreign_bid = live_negotiation_countries
                                        .get(&player.id)
                                        .map(|set| set.iter().any(|c| *c != country.id))
                                        .unwrap_or(false);
                                    if foreign_bid {
                                        report.cohort.with_live_foreign_bid += 1;
                                    }
                                    if let Some(league_id) =
                                        club.teams.main().and_then(|t| t.league_id)
                                    {
                                        let league_name = country
                                            .leagues
                                            .leagues
                                            .iter()
                                            .find(|l| l.id == league_id)
                                            .map(|l| l.name.clone())
                                            .unwrap_or_else(|| "—".to_string());
                                        let row = report
                                            .cohort
                                            .per_league
                                            .entry(league_id)
                                            .or_insert((league_name, 0));
                                        row.1 += 1;
                                    }
                                }
                            }

                            // Big-stage ambition, by tier.
                            let pull = player.big_stage_inclination;
                            if pull >= STAGE_INCLINED_BAR {
                                report.players.stage_inclined += 1;
                                *report
                                    .players
                                    .stage_inclined_by_league_band
                                    .entry(league_band(club_league_reputation))
                                    .or_insert(0) += 1;
                            }
                            if pull >= STAGE_MOOD_BAR {
                                report.players.stage_mood += 1;
                            }
                            if player
                                .transfer_request_reasons
                                .iter()
                                .any(|r| matches!(r, TransferRequestReason::WantsStrongerLeague))
                            {
                                report.players.stage_requesting += 1;
                            }

                            // Prime-age professionals parked below the
                            // first team, and whether anything is being
                            // done about them.
                            if band == SquadBand::SeniorReserve && age >= PARKED_PRIME_AGE {
                                report.players.parked_prime += 1;
                                let moving = player.statuses.has(PlayerStatusType::Lst)
                                    || player.statuses.has(PlayerStatusType::Loa)
                                    || player.statuses.has(PlayerStatusType::Req)
                                    || listed_for_transfer.contains(&player.id)
                                    || listed_for_loan.contains(&player.id);
                                if moving {
                                    report.players.parked_prime_with_availability += 1;
                                }
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

        // ---- hoard ------------------------------------------------
        // Top clubs by cash, and whether that cash is reachable. A club
        // whose balance is several years of income while its budget is a
        // fraction of one is not rich — it is a leak.
        let mut by_balance: Vec<(&u32, &ClubFacts)> = club_facts.iter().collect();
        by_balance.sort_by(|a, b| b.1.balance.cmp(&a.1.balance));
        report.hoard = by_balance
            .into_iter()
            .take(HOARD_ROWS)
            .map(|(club_id, c)| HoardRow {
                name: c.name.clone(),
                balance: c.balance,
                annual_income: c.annual_income,
                transfer_budget: c.transfer_budget,
                gross_spend: gross_spend.get(club_id).copied().unwrap_or(0.0),
            })
            .collect();

        report
    }
}

/// How many clubs the hoard table reports.
const HOARD_ROWS: usize = 20;

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

    /// The springboard block: does a standout in a sub-elite league get
    /// bought by a bigger one, at the ages and fees the real market does
    /// it at — and is anybody's money actually moving?
    ///
    /// Every line here answers a question the rest of the report cannot.
    /// Move VOLUME was always healthy; direction was never measured, so a
    /// world in which the exporter leagues were terminal posted exactly
    /// The live-market section: did the seven layers actually run, and did
    /// they produce a market rather than a faster queue?
    fn print_live_market(report: &MarketReport) {
        let live = &report.live;
        println!("\n-- LIVE MARKET (planner / watchlist / ledger / pricing) --");

        let brief_share = Self::pct(live.clubs_with_brief, report.clubs);
        println!(
            "briefs: {} of {} clubs ({brief_share:.1}%), {} slots  |  tiers A {} / B {} / C {}",
            live.clubs_with_brief,
            report.clubs,
            live.briefed_slots,
            live.tier_a,
            live.tier_b,
            live.tier_c,
        );
        if !live.money_slack.is_empty() {
            let mut slack: Vec<f64> = live.money_slack.iter().map(|s| *s as f64).collect();
            let mean = slack.iter().sum::<f64>() / slack.len() as f64;
            println!(
                "money slack: mean {mean:.2}, median {:.2}, clubs above 0.30 {:.1}%",
                Self::median(&mut slack),
                Self::pct(
                    live.money_slack.iter().filter(|s| **s > 0.30).count(),
                    live.money_slack.len()
                ),
            );
        }

        if !live.watchlist_sizes.is_empty() {
            let carried = live.watchlist_sizes.iter().filter(|n| **n > 0).count();
            let total: usize = live.watchlist_sizes.iter().sum();
            println!(
                "watchlists: {} of {} clubs carry one, mean depth {:.1}",
                carried,
                live.watchlist_sizes.len(),
                total as f64 / live.watchlist_sizes.len().max(1) as f64,
            );
            match live.board_overlap() {
                // The herd guard. Above ~35 % the biggest clubs are all
                // chasing the same names, the market becomes one auction,
                // and everybody who loses it signs nobody.
                Some(overlap) => println!(
                    "top-{} board overlap: {:.1}% of the smaller board (target <= 35%)",
                    LiveMarketCensus::OVERLAP_CLUBS,
                    overlap * 100.0
                ),
                None => println!("top-club board overlap: not enough boards to compare"),
            }
        }

        let sell_total: usize = live.sell_list_sizes.iter().sum();
        println!(
            "sell lists: {} entries, {} marketed, on {} clubs",
            sell_total,
            live.marketed_players,
            live.sell_list_sizes.iter().filter(|n| **n > 0).count(),
        );

        if !live.squad_sizes.is_empty() {
            let mut sizes: Vec<f64> = live.squad_sizes.iter().map(|n| *n as f64).collect();
            // "Trimmed" rather than "in violation": omitting the weakest
            // foreigners at registration is how a club MEETS the quota, not
            // how it breaks it. The number to watch is whether it grows —
            // a club buying past its own country's rule is wasting money.
            println!(
                "senior squads: median {:.0}, max {:.0}, over the board cap {} ({:.1}%)  |  \
                 squads the foreigner quota trims {} ({:.1}%)",
                Self::median(&mut sizes),
                sizes.iter().copied().fold(0.0f64, f64::max),
                live.squads_over_cap,
                Self::pct(live.squads_over_cap, live.squad_sizes.len()),
                live.regulation_violations,
                Self::pct(live.regulation_violations, live.squad_sizes.len()),
            );
        }

        if !live.big_move_ages.is_empty() {
            let mut ages: Vec<f64> = live.big_move_ages.iter().map(|a| *a as f64).collect();
            let in_band = live
                .big_move_ages
                .iter()
                .filter(|a| (19..=29).contains(*a))
                .count();
            println!(
                "fee moves >= {:.0}M: {}, median age {:.1} (real ~24), {:.1}% aged 19-29 (real ~90%)",
                LiveMarketCensus::BIG_MOVE_FEE / 1_000_000.0,
                ages.len(),
                Self::median(&mut ages),
                Self::pct(in_band, live.big_move_ages.len()),
            );
        }

        println!(
            "stories: contested agreements {}, deadline-week {}, sell-list conversions {}, \
             agent-led approaches {}",
            live.contested_agreements,
            live.deadline_agreements,
            live.sell_list_conversions,
            live.agent_led_approaches,
        );

        let mut leagues: Vec<(&u32, &(String, f64))> = live.gross_spend_by_league.iter().collect();
        leagues.sort_by(|a, b| b.1.1.partial_cmp(&a.1.1).unwrap_or(Ordering::Equal));
        if !leagues.is_empty() {
            println!("gross fee spend by buying league (top 8):");
            for (_, (name, spend)) in leagues.iter().take(8) {
                println!("  {name:<28} {:.1}M", spend / 1_000_000.0);
            }
        }

        // Price level is the slowest-moving instrument here. It is SEEDED
        // per country by the database (England 1.5, Spain 1.2, …), so the
        // number to read across two runs is the drift, not the level: a
        // country that has moved more than a tenth in a year is the
        // inflation spiral this model exists to bound.
        let mut levels: Vec<&(String, f32)> = live.price_levels.iter().collect();
        levels.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(Ordering::Equal));
        let spread: Vec<f32> = levels.iter().map(|(_, l)| *l).collect();
        println!(
            "price level across {} countries: min {:.3}, max {:.3}  (compare run-to-run for drift)",
            spread.len(),
            spread.iter().copied().fold(f32::INFINITY, f32::min),
            spread.iter().copied().fold(f32::NEG_INFINITY, f32::max),
        );
        for (name, level) in levels.iter().take(6) {
            println!("  {name:<28} {level:.3}");
        }
    }

    /// the same headline numbers as one in which they were springboards.
    fn print_springboard(report: &MarketReport) {
        let flow = &report.flow;
        let seasons = flow.season_count() as f64;

        println!("\n-- FLOW MATRIX (permanent moves, from band → to band) --");
        println!(
            "{:<20} {:<20} {:>7} {:>12} {:>12} {:>7}",
            "from", "to", "moves", "median fee", "max fee", "med age"
        );
        for from in BandLadder::RUNGS.iter().rev() {
            for to in BandLadder::RUNGS.iter().rev() {
                let cell = flow.cell(from, to);
                if cell.is_empty() {
                    continue;
                }
                let mut fees: Vec<f64> = cell.iter().map(|m| m.fee).collect();
                let mut ages: Vec<f64> = cell
                    .iter()
                    .filter_map(|m| m.age)
                    .map(|a| a as f64)
                    .collect();
                println!(
                    "{:<20} {:<20} {:>7} {:>12.0} {:>12.0} {:>7.0}",
                    from,
                    to,
                    cell.len(),
                    Self::median(&mut fees),
                    fees.iter().copied().fold(0.0f64, f64::max),
                    Self::median(&mut ages),
                );
            }
        }
        // The two cells this campaign is about, plus the reverse flow that
        // must survive it.
        for (from, to) in [
            ("strong 7000-8499", "elite 8500+"),
            ("mid 5500-6999", "elite 8500+"),
            ("mid 5500-6999", "strong 7000-8499"),
            ("elite 8500+", "strong 7000-8499"),
        ] {
            let n = flow.cell(from, to).len();
            println!(
                "  {from:<20} → {to:<20} {n:>5} total, {:.1}/season",
                n as f64 / seasons
            );
        }

        let step_ups = flow.step_ups();
        let mut step_ages: Vec<f64> = step_ups
            .iter()
            .filter_map(|m| m.age)
            .map(|a| a as f64)
            .collect();
        let mut step_income: Vec<f64> = step_ups
            .iter()
            .filter_map(|m| m.fee_over_seller_income)
            .collect();
        let mut step_fees: Vec<f64> = step_ups.iter().map(|m| m.fee).collect();
        let cross_border = step_ups.iter().filter(|m| m.cross_border).count();
        println!(
            "step-ups (age {}-{}, band strictly up): {} total, {:.1}/season, \
             {} cross-border ({:.1}%)  |  median age {:.0}, median fee {:.0}, \
             median fee/seller income {:.2}",
            FlowMatrix::STEP_UP_AGE_MIN,
            FlowMatrix::STEP_UP_AGE_MAX,
            step_ups.len(),
            step_ups.len() as f64 / seasons,
            cross_border,
            Self::pct(cross_border, step_ups.len()),
            Self::median(&mut step_ages),
            Self::median(&mut step_fees),
            Self::median(&mut step_income),
        );

        let c = &report.cohort;
        println!(
            "\n-- STANDOUT COHORT (Main squad, league {}-{}, age {}-{}, standing >= {:.1}) --",
            STANDOUT_LEAGUE_MIN,
            STANDOUT_LEAGUE_MAX,
            STANDOUT_AGE_MIN,
            STANDOUT_AGE_MAX,
            STANDOUT_STANDING_BAR,
        );
        let mut cohort_ages: Vec<f64> = c.ages.iter().map(|a| *a as f64).collect();
        println!(
            "size {}  |  median age {:.0}  |  carried by a FOREIGN club {} ({:.1}%)  \
             |  live foreign bid {} ({:.1}%)",
            c.total,
            Self::median(&mut cohort_ages),
            c.with_foreign_file,
            Self::pct(c.with_foreign_file, c.total),
            c.with_live_foreign_bid,
            Self::pct(c.with_live_foreign_bid, c.total),
        );

        // Exporter drain: standouts sold UP per league per season, next to
        // how many that league is holding. An over-correction that empties
        // the Super Lig is as wrong as one that never sells.
        println!("\n-- EXPORTER DRAIN (standouts sold up, per league per season) --");
        let mut sold_up_per_league: HashMap<u32, (String, usize)> = HashMap::new();
        for m in &step_ups {
            if let Some((id, name)) = &m.from_league {
                let row = sold_up_per_league.entry(*id).or_insert((name.clone(), 0));
                row.1 += 1;
            }
        }
        let mut drains: Vec<(&u32, &(String, usize))> = sold_up_per_league.iter().collect();
        drains.sort_by(|a, b| b.1.1.cmp(&a.1.1));
        if drains.is_empty() {
            println!("  none");
        }
        for (league_id, (name, sold)) in drains.into_iter().take(12) {
            let holding = c.per_league.get(league_id).map(|(_, n)| *n).unwrap_or(0);
            println!(
                "  {name:<34} sold up {:>5.1}/season   (currently holding {holding} standouts)",
                *sold as f64 / seasons,
            );
        }

        println!("\n-- HOARD (top {HOARD_ROWS} clubs by balance) --");
        println!(
            "{:<30} {:>14} {:>10} {:>10} {:>10}",
            "club", "balance", "bal/inc", "budget/inc", "spend/bud"
        );
        for row in &report.hoard {
            let inc = row.annual_income.max(1) as f64;
            println!(
                "{:<30} {:>14.0} {:>10.2} {:>10.2} {:>10.2}",
                row.name,
                row.balance as f64,
                row.balance as f64 / inc,
                row.transfer_budget / inc,
                if row.transfer_budget > 0.0 {
                    row.gross_spend / row.transfer_budget
                } else {
                    0.0
                },
            );
        }
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

        // Is the loan market a market, or a fixed permutation? Volume above
        // says nothing about this: a pipeline that picks destinations by
        // argmax over a near-constant reputation posts the same throughput
        // while sending the same players to the same clubs every window.
        let lf = &report.loan_flow;
        if !lf.pairs.is_empty() {
            println!("\n-- loan flow (who lends to whom) --");
            println!(
                "lending clubs {}  |  borrowing clubs {}  |  distinct pairs {}",
                lf.per_parent.len(),
                lf.per_borrower.len(),
                lf.pairs.len(),
            );
            println!(
                "parents with {}+ loans: mean {:.2} distinct destinations, \
                 {:.0}% of their loans to their top one",
                LoanFlowCensus::MIN_LOANS_FOR_HABIT,
                lf.mean_distinct_destinations(),
                lf.mean_top_destination_share() * 100.0,
            );
            println!(
                "repeat pairs: {} used 3+ times, {} used 5+  |  busiest borrower takes {:.1}% of all loans",
                lf.pairs_repeating(3),
                lf.pairs_repeating(5),
                lf.busiest_borrower_share() * 100.0,
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
        println!(
            "  by asset class:  {}",
            Self::top(&p.zero_by_asset_class, 8)
        );
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

        println!("\n-- big-stage ambition (senior, servable) --");
        println!(
            "would listen {} ({:.1}%)  |  publicly restless {} ({:.1}%)  |  formally asking {} ({:.1}%)",
            p.stage_inclined,
            Self::pct(p.stage_inclined, p.senior_servable),
            p.stage_mood,
            Self::pct(p.stage_mood, p.senior_servable),
            p.stage_requesting,
            Self::pct(p.stage_requesting, p.senior_servable),
        );
        println!(
            "  would listen, by league: {}",
            Self::top(&p.stage_inclined_by_league_band, 6)
        );

        Self::print_springboard(report);
        Self::print_live_market(report);

        println!("\n-- parked primes (24+ on a B / Second / Reserve squad) --");
        println!(
            "total {}  |  {} being moved ({:.1}%)",
            p.parked_prime,
            p.parked_prime_with_availability,
            Self::pct(p.parked_prime_with_availability, p.parked_prime),
        );

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
// The player's side of the market — P0 instrumentation
// ---------------------------------------------------------------------
//
// Everything above measures the market as a flow of moves. None of it can
// answer the three questions the player-side model exists to settle:
//
//   * does anybody take a PAY CUT to move, and who?
//   * does money in a weak league ever buy a good player out of a strong
//     one, and at what multiple?
//   * does a stuck foreign prospect go home on loan, and does it convert?
//
// The transfer record carries no wage, so the wage question has to be
// answered by watching: a snapshot of every player each day, and a
// comparison when his club changes. That is the only way to see a cut at
// all, and it is why this census is a running tracker rather than a
// report-time walk.

/// One player, as he was yesterday. The comparison that turns "he moved"
/// into "he moved and took 60 % less".
#[derive(Debug, Clone, Copy)]
struct PlayerSnapshot {
    club_id: u32,
    wage: u32,
    /// League reputation of the club he was at, so a move's direction is a
    /// comparison rather than a lookup.
    league_reputation: u16,
    country_id: u32,
    age: u8,
    /// The availability signal he carried BEFORE the move — the cell the
    /// pay-cut share is bucketed by.
    availability: Availability,
    starter_ratio: f32,
    tenure_days: u16,
    nationality_country_id: u32,
    nationality_region: Option<u8>,
}

/// What the player's own state said about him when the move began.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Availability {
    None,
    Unhappy,
    Requested,
    Listed,
}

impl Availability {
    fn label(self) -> &'static str {
        match self {
            Availability::None => "no signal",
            Availability::Unhappy => "Unh",
            Availability::Requested => "Req",
            Availability::Listed => "listed",
        }
    }

    const ALL: [Availability; 4] = [
        Availability::None,
        Availability::Unhappy,
        Availability::Requested,
        Availability::Listed,
    ];
}

/// One completed permanent move, with both wages.
#[derive(Debug, Clone, Copy)]
struct WageMove {
    ratio: f64,
    age: u8,
    availability: Availability,
    /// −1 down a band, 0 the same, +1 up.
    direction: i8,
    /// The buyer's owner-funding read, 0..1.
    buyer_benefactor: f32,
    /// Buyer league reputation minus seller's.
    league_gap: i32,
}

impl WageMove {
    fn age_band(&self) -> &'static str {
        match self.age {
            0..=23 => "≤23",
            24..=27 => "24–27",
            28..=30 => "28–30",
            _ => "31+",
        }
    }

    fn direction_label(&self) -> &'static str {
        match self.direction {
            i8::MIN..=-1 => "down",
            0 => "same",
            _ => "up",
        }
    }

    /// A money move: the buyer's cash cannot be explained by its revenue,
    /// and its league is clearly weaker than the one it is buying out of.
    /// No country list, no flag — two numbers.
    fn is_money_move(&self) -> bool {
        self.buyer_benefactor >= PlayerSideCensus::BENEFACTOR_BAR
            && self.league_gap <= -PlayerSideCensus::MONEY_MOVE_LEAGUE_GAP
    }
}

/// One loan out of a stronger league by a stuck young foreigner.
#[derive(Debug, Clone, Copy)]
struct LoanHomeMove {
    /// Where he went, relative to where he is from.
    destination: HomeDestination,
    /// Converted to a permanent at the same club inside two years.
    converted: bool,
    /// The loan date, so conversions can be dated against it.
    day: u32,
    player_id: u32,
    to_club_id: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum HomeDestination {
    HomeCountry,
    HomeRegion,
    Elsewhere,
}

/// How personal terms actually ended, and why.
#[derive(Debug, Default)]
struct TermsOutcomes {
    agreed: usize,
    /// Refusals by cause label.
    refused: HashMap<&'static str, usize>,
    /// `reservation ÷ offered` on every refusal that named a number.
    demand_ratios: Vec<f64>,
}

/// The running player-side census. Fed once a day; printed with the rest.
#[derive(Debug, Default)]
struct PlayerSideCensus {
    snapshots: HashMap<u32, PlayerSnapshot>,
    wage_moves: Vec<WageMove>,
    /// Foreign U23s with a start share under the bar after six months —
    /// the population Part I.3 is written against, sampled the day they
    /// first qualify so the "share loaned next window" denominator is a
    /// cohort rather than a moving target.
    stuck_cohort: HashSet<u32>,
    stuck_cohort_loaned: HashSet<u32>,
    loans_home: Vec<LoanHomeMove>,
    terms: TermsOutcomes,
    /// Negotiations already banked, so a multi-day rejection is counted
    /// once.
    seen_negotiations: HashSet<(u32, u32)>,
    day: u32,
}

impl PlayerSideCensus {
    /// Owner funding at which a club reads as bankrolled rather than
    /// merely solvent — [`ClubBenefactor::STATE_BACKED_BAR`].
    const BENEFACTOR_BAR: f32 = 0.5;
    /// League-reputation gap that makes a purchase a MONEY move rather
    /// than an ordinary one: the buyer is buying out of a clearly stronger
    /// competition than its own.
    const MONEY_MOVE_LEAGUE_GAP: i32 = 1_500;
    /// Below this share of starts a young foreigner is not playing.
    const STUCK_STARTER_BAR: f32 = 0.20;
    /// …and this long is enough for that to be a verdict.
    const STUCK_TENURE_DAYS: u16 = 180;
    const STUCK_MAX_AGE: u8 = 23;
    /// A cut, as the calibration table defines one.
    const CUT_RATIO: f64 = 0.8;
    /// Conversion window for a home loan.
    const CONVERSION_DAYS: u32 = 730;

    /// Walk the world once and fold today into the running census.
    fn observe(&mut self, data: &SimulatorData, day: u32) {
        self.day = day;
        let date = data.date.date();

        // Club-level facts the move classification needs, resolved once
        // per day rather than per player.
        let mut club_league: HashMap<u32, u16> = HashMap::new();
        let mut club_country: HashMap<u32, u32> = HashMap::new();
        let mut club_benefactor: HashMap<u32, f32> = HashMap::new();
        let mut club_region: HashMap<u32, u8> = HashMap::new();
        for continent in &data.continents {
            for country in &continent.countries {
                let region = region_code(country.continent_id, &country.code);
                for club in &country.clubs {
                    let league_rep = club
                        .teams
                        .main()
                        .or_else(|| club.teams.teams.first())
                        .and_then(|t| t.league_id)
                        .and_then(|lid| country.leagues.leagues.iter().find(|l| l.id == lid))
                        .map(|l| l.reputation)
                        .unwrap_or(0);
                    club_league.insert(club.id, league_rep);
                    club_country.insert(club.id, country.id);
                    club_benefactor.insert(club.id, club.board.ownership.benefactor);
                    club_region.insert(club.id, region);
                }
            }
        }

        // ---- personal-terms outcomes -------------------------------
        for continent in &data.continents {
            for country in &continent.countries {
                for negotiation in country.transfer_market.negotiations.values() {
                    let key = (country.id, negotiation.id);
                    if let Some(cause) = negotiation.terms_refusal_cause {
                        if self.seen_negotiations.insert(key) {
                            *self.terms.refused.entry(refusal_label(cause)).or_insert(0) += 1;
                            if let (Some(reservation), Some(offered)) = (
                                negotiation.terms_reservation_wage,
                                negotiation.offered_salary,
                            ) {
                                if offered > 0 {
                                    self.terms
                                        .demand_ratios
                                        .push(reservation as f64 / offered as f64);
                                }
                            }
                        }
                    } else if matches!(
                        negotiation.phase,
                        core::transfers::NegotiationPhase::MedicalAndFinalization { .. }
                    ) && self.seen_negotiations.insert(key)
                    {
                        self.terms.agreed += 1;
                    }
                }
            }
        }

        // ---- the daily player walk ---------------------------------
        let mut today: HashMap<u32, PlayerSnapshot> = HashMap::with_capacity(self.snapshots.len());
        for continent in &data.continents {
            for country in &continent.countries {
                for club in &country.clubs {
                    let league_reputation = club_league.get(&club.id).copied().unwrap_or(0);
                    for team in &club.teams.teams {
                        for player in &team.players.players {
                            let Some(contract) = player.contract.as_ref() else {
                                continue;
                            };
                            let snap = PlayerSnapshot {
                                club_id: club.id,
                                wage: contract.salary,
                                league_reputation,
                                country_id: country.id,
                                age: DateUtils::age(player.birth_date, date),
                                availability: availability_of(player),
                                starter_ratio: player.happiness.starter_ratio,
                                tenure_days: tenure_days(player, date),
                                nationality_country_id: player.country_id,
                                nationality_region: player.home_region().map(region_of),
                            };
                            self.fold_move(player.id, &snap, &club_benefactor, &club_region);
                            today.insert(player.id, snap);
                        }
                    }
                }
            }
        }
        self.snapshots = today;
    }

    /// Compare a player against yesterday, and bank whatever the
    /// comparison says.
    fn fold_move(
        &mut self,
        player_id: u32,
        now: &PlayerSnapshot,
        club_benefactor: &HashMap<u32, f32>,
        club_region: &HashMap<u32, u8>,
    ) {
        // The stuck cohort is sampled continuously: once a man qualifies
        // he stays in the denominator, so "share loaned next window" is a
        // rate over a fixed population rather than a moving one.
        if now.age <= Self::STUCK_MAX_AGE
            && now.nationality_country_id != 0
            && now.nationality_country_id != now.country_id
            && now.starter_ratio < Self::STUCK_STARTER_BAR
            && now.tenure_days >= Self::STUCK_TENURE_DAYS
        {
            self.stuck_cohort.insert(player_id);
        }

        let Some(before) = self.snapshots.get(&player_id).copied() else {
            return;
        };
        if before.club_id == now.club_id {
            return;
        }

        // A home loan that converts: he was lent here, and now the same
        // club owns him.
        for loan in self.loans_home.iter_mut() {
            if loan.player_id == player_id
                && loan.to_club_id == now.club_id
                && self.day.saturating_sub(loan.day) <= Self::CONVERSION_DAYS
            {
                loan.converted = true;
            }
        }

        if self.stuck_cohort.contains(&player_id) {
            self.stuck_cohort_loaned.insert(player_id);
            let destination = if now.nationality_country_id != 0
                && now.nationality_country_id == now.country_id
            {
                HomeDestination::HomeCountry
            } else if now.nationality_region.is_some()
                && now.nationality_region == club_region.get(&now.club_id).copied()
            {
                HomeDestination::HomeRegion
            } else {
                HomeDestination::Elsewhere
            };
            self.loans_home.push(LoanHomeMove {
                destination,
                converted: false,
                day: self.day,
                player_id,
                to_club_id: now.club_id,
            });
        }

        if before.wage == 0 || now.wage == 0 {
            return;
        }
        let league_gap = now.league_reputation as i32 - before.league_reputation as i32;
        self.wage_moves.push(WageMove {
            ratio: now.wage as f64 / before.wage as f64,
            age: before.age,
            availability: before.availability,
            direction: match league_gap {
                g if g >= 750 => 1,
                g if g <= -750 => -1,
                _ => 0,
            },
            buyer_benefactor: club_benefactor.get(&now.club_id).copied().unwrap_or(0.0),
            league_gap,
        });
    }
}
/// Small helpers the player-side census needs. Free of the harness's own
/// state so they read the way a football fact reads.
fn availability_of(player: &core::Player) -> Availability {
    if player.statuses.has(PlayerStatusType::Req) {
        Availability::Requested
    } else if player.statuses.has(PlayerStatusType::Unh) {
        Availability::Unhappy
    } else if player.statuses.has(PlayerStatusType::Lst)
        || player.statuses.has(PlayerStatusType::Loa)
    {
        Availability::Listed
    } else {
        Availability::None
    }
}

/// The honest anchor for a stay — a loan return re-stamps
/// `last_transfer_date` (memory `loan_pipeline`).
fn tenure_days(player: &core::Player, date: NaiveDate) -> u16 {
    StuckCareerScan::club_tenure_days(player, date)
        .unwrap_or(i64::from(u16::MAX))
        .clamp(0, i64::from(u16::MAX)) as u16
}

fn region_of(region: ScoutingRegion) -> u8 {
    ScoutingRegion::all()
        .iter()
        .position(|r| *r == region)
        .unwrap_or(usize::MAX) as u8
}

fn region_code(continent_id: u32, country_code: &str) -> u8 {
    region_of(ScoutingRegion::from_country(continent_id, country_code))
}

fn refusal_label(cause: TermsRefusalCause) -> &'static str {
    match cause {
        TermsRefusalCause::WageDemand => "wage demand",
        TermsRefusalCause::SportingStepDown => "sporting step down",
        TermsRefusalCause::Role => "role",
        TermsRefusalCause::Place => "place",
        TermsRefusalCause::Attachment => "attachment",
    }
}

fn median(values: &mut [f64]) -> f64 {
    if values.is_empty() {
        return 0.0;
    }
    values.sort_by(|a, b| a.partial_cmp(b).unwrap_or(Ordering::Equal));
    values[values.len() / 2]
}

/// Prints the three questions the player-side model exists to settle.
struct PlayerSidePrinter;

impl PlayerSidePrinter {
    fn print(census: &PlayerSideCensus) {
        println!();
        println!("── The player's side ─────────────────────────────────");

        Self::wage_census(census);
        Self::money_moves(census);
        Self::loan_home(census);
        Self::terms(census);
    }

    /// Does anybody take a cut, and who? The share of permanent moves at
    /// under 80 % of the previous wage, cell by cell.
    fn wage_census(census: &PlayerSideCensus) {
        let moves = &census.wage_moves;
        if moves.is_empty() {
            println!("  wage census: no wage-bearing moves observed yet");
            return;
        }
        let cuts = moves
            .iter()
            .filter(|m| m.ratio < PlayerSideCensus::CUT_RATIO)
            .count();
        let mut all: Vec<f64> = moves.iter().map(|m| m.ratio).collect();
        println!(
            "  wage census: {} moves, {:.0}% at a cut (<0.8x), median ratio {:.2}x",
            moves.len(),
            100.0 * cuts as f64 / moves.len() as f64,
            median(&mut all),
        );
        println!("    {:<8} {:<10} {:<6} {:>6} {:>8} {:>8}", "age", "signal", "dir", "n", "cut %", "median");
        for band in ["≤23", "24–27", "28–30", "31+"] {
            for signal in Availability::ALL {
                for dir in ["up", "same", "down"] {
                    let cell: Vec<&WageMove> = moves
                        .iter()
                        .filter(|m| {
                            m.age_band() == band
                                && m.availability == signal
                                && m.direction_label() == dir
                        })
                        .collect();
                    if cell.len() < 5 {
                        continue;
                    }
                    let cut = cell
                        .iter()
                        .filter(|m| m.ratio < PlayerSideCensus::CUT_RATIO)
                        .count();
                    let mut ratios: Vec<f64> = cell.iter().map(|m| m.ratio).collect();
                    println!(
                        "    {:<8} {:<10} {:<6} {:>6} {:>7.0}% {:>7.2}x",
                        band,
                        signal.label(),
                        dir,
                        cell.len(),
                        100.0 * cut as f64 / cell.len() as f64,
                        median(&mut ratios),
                    );
                }
            }
        }
    }

    /// Money out of a weak league into a strong one's players. Two ratios
    /// decide membership — owner funding and the league gap — so the Gulf,
    /// Russia and MLS appear or do not appear on their own.
    fn money_moves(census: &PlayerSideCensus) {
        let cell: Vec<&WageMove> = census
            .wage_moves
            .iter()
            .filter(|m| m.is_money_move())
            .collect();
        if cell.is_empty() {
            println!("  money moves: none — no benefactor club bought out of a stronger league");
            return;
        }
        let mut multiples: Vec<f64> = cell.iter().map(|m| m.ratio).collect();
        let mut ages: Vec<f64> = cell.iter().map(|m| m.age as f64).collect();
        let under_26 = cell.iter().filter(|m| m.age < 26).count();
        println!(
            "  money moves: {} — median age {:.0}, {:.0}% under 26, median wage multiple {:.1}x",
            cell.len(),
            median(&mut ages),
            100.0 * under_26 as f64 / cell.len() as f64,
            median(&mut multiples),
        );
    }

    /// The stuck foreign prospect: is he lent, and does he go home?
    fn loan_home(census: &PlayerSideCensus) {
        if census.stuck_cohort.is_empty() {
            println!("  loan home: no stuck foreign U23 cohort yet");
            return;
        }
        let loaned = census.stuck_cohort_loaned.len();
        let home = census
            .loans_home
            .iter()
            .filter(|l| l.destination == HomeDestination::HomeCountry)
            .count();
        let region = census
            .loans_home
            .iter()
            .filter(|l| l.destination == HomeDestination::HomeRegion)
            .count();
        let converted = census.loans_home.iter().filter(|l| l.converted).count();
        let moved = census.loans_home.len().max(1);
        println!(
            "  loan home: cohort {} stuck foreign U23, {} moved ({:.0}%)",
            census.stuck_cohort.len(),
            loaned,
            100.0 * loaned as f64 / census.stuck_cohort.len() as f64,
        );
        println!(
            "    destinations: {:.0}% home country, {:.0}% home region, {:.0}% elsewhere; {:.0}% converted",
            100.0 * home as f64 / moved as f64,
            100.0 * region as f64 / moved as f64,
            100.0 * (moved - home - region) as f64 / moved as f64,
            100.0 * converted as f64 / moved as f64,
        );
    }

    /// Whose decision killed the deal, and by how much.
    fn terms(census: &PlayerSideCensus) {
        let refused: usize = census.terms.refused.values().sum();
        let total = census.terms.agreed + refused;
        if total == 0 {
            println!("  personal terms: nothing resolved yet");
            return;
        }
        println!(
            "  personal terms: {} agreed, {} refused ({:.0}% agreed)",
            census.terms.agreed,
            refused,
            100.0 * census.terms.agreed as f64 / total as f64,
        );
        let mut causes: Vec<(&&str, &usize)> = census.terms.refused.iter().collect();
        causes.sort_by(|a, b| b.1.cmp(a.1));
        for (cause, count) in causes {
            println!(
                "    {:<20} {:>5} ({:.0}%)",
                cause,
                count,
                100.0 * *count as f64 / refused.max(1) as f64
            );
        }
        let mut ratios = census.terms.demand_ratios.clone();
        if !ratios.is_empty() {
            println!(
                "    median demand ÷ offered on a refusal: {:.2}x",
                median(&mut ratios)
            );
        }
    }
}
// ---------------------------------------------------------------------
// Harness
// ---------------------------------------------------------------------

struct SimHarness {
    data: SimulatorData,
    /// The player-side census is a running tracker, not a report-time
    /// walk: the transfer record carries no wage, so the only way to see a
    /// pay cut at all is to hold yesterday's number and compare.
    player_side: PlayerSideCensus,
}

impl SimHarness {
    fn generate() -> Self {
        let database = DatabaseLoader::load();
        let data = DatabaseGenerator::generate(&database);
        let mut harness = SimHarness {
            data,
            player_side: PlayerSideCensus::default(),
        };
        // Day zero: every player's starting wage, so the very first
        // window's moves already have a "before" to compare against.
        harness.player_side.observe(&harness.data, 0);
        harness
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
            self.player_side.observe(&self.data, day);
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
                PlayerSidePrinter::print(&self.player_side);
            }
        }
        let mut report = MarketCensus::collect(&self.data);
        ReportPrinter::print(&mut report, &self.data, days);
        PlayerSidePrinter::print(&self.player_side);
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
