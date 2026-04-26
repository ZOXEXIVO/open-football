use crate::ai::{Ai, AiBatchProcessor};
use crate::club::ai::apply_ai_responses;
use crate::club::board::manager_market;
use crate::competitions::simulation::GlobalCompetitionSimulator;
use crate::competitions::GlobalCompetitions;
use crate::context::{GlobalContext, SimulationContext};
use crate::continent::{Continent, ContinentResult};
use crate::league::{LeagueTable, MatchStorage};
use crate::r#match::MatchResult;
use crate::shared::SimulatorDataIndexes;
use crate::simulator_config::SimulatorConfig;
use crate::transfers::TransferPool;
use crate::utils::random::engine as rng_engine;
use crate::{Player, Staff, TeamInfo, TeamType};
use chrono::{Datelike, Duration, NaiveDateTime};
use rayon::prelude::*;
use std::collections::HashMap;
use std::panic::{self, AssertUnwindSafe};
use std::sync::atomic::{AtomicU64, Ordering};

/// Lightweight country info for nationality lookups.
/// Covers ALL countries (not just simulation participants).
#[derive(Clone, Debug)]
pub struct CountryInfo {
    pub id: u32,
    pub code: String,
    pub slug: String,
    pub name: String,
}

fn panic_message(payload: &(dyn std::any::Any + Send)) -> &'static str {
    if let Some(s) = payload.downcast_ref::<&'static str>() {
        s
    } else if payload.downcast_ref::<String>().is_some() {
        "<String panic>"
    } else {
        "<non-string panic>"
    }
}

/// Cumulative count of continent panics swallowed by the simulator. The
/// `simulate` loop catches a panicking continent and substitutes an empty
/// result so the rest of the world keeps ticking — this counter exposes
/// that silent failure to operators and tests. Read from anywhere via
/// `panicked_continents_total()`.
static PANICKED_CONTINENTS: AtomicU64 = AtomicU64::new(0);

/// Total continent panics swallowed since process start.
pub fn panicked_continents_total() -> u64 {
    PANICKED_CONTINENTS.load(Ordering::Relaxed)
}

pub struct FootballSimulator;

impl FootballSimulator {
    /// Tick the simulator one day with default tunables. Use `simulate_with`
    /// to plumb a `SimulatorConfig` (per-save overrides, faster timeouts in
    /// tests, etc.).
    pub async fn simulate(data: &mut SimulatorData) -> SimulationResult {
        Self::simulate_with(data, &SimulatorConfig::default()).await
    }

    pub async fn simulate_with(
        data: &mut SimulatorData,
        config: &SimulatorConfig,
    ) -> SimulationResult {
        let mut result = SimulationResult::new();

        let current_date = data.date;

        let ctx = GlobalContext::new(SimulationContext::new(data.date), Ai::new());

        // Phase ordering note:
        // A simulates continents and emits AI requests as FnOnce closures that
        // capture stable IDs (club_id, player_id, …). B executes those requests
        // against the freshly-mutated data. C then drains the ContinentResults
        // collected in A. The results reference the same stable IDs, so Phase B
        // mutations (contracts, morale, etc.) are safely visible to Phase C.

        // Phase A: simulate all continents in parallel. Each call mutates its
        // own continent and pushes AI requests into the shared (Arc<Mutex>) Ai
        // collector. Continents are independent during this phase.
        //
        // A panic inside one continent must not kill the whole tick — a
        // single buggy state machine or malformed save row would otherwise
        // unwind the Rayon pool and dump the player's save. `AssertUnwindSafe`
        // is sound here because the closure mutates only its own continent
        // (no shared `&mut` state) and doesn't hold any locks; the Rayon
        // worker doesn't carry poisoned state across iterations. Panic is
        // surfaced via the `PANICKED_CONTINENTS` counter and a structured
        // log line; surviving continents still advance. Per-tick count
        // is recovered as the delta on the atomic since map closures
        // running in parallel can't share a `&mut u32`.
        let panicks_before = PANICKED_CONTINENTS.load(Ordering::Relaxed);
        let results: Vec<ContinentResult> = data
            .continents
            .par_iter_mut()
            .map(|continent| {
                let cid = continent.id;
                let name = continent.name.clone();
                let ctx_ref = &ctx;
                panic::catch_unwind(AssertUnwindSafe(|| {
                    continent.simulate(ctx_ref.with_continent(cid))
                }))
                .unwrap_or_else(|payload| {
                    PANICKED_CONTINENTS.fetch_add(1, Ordering::Relaxed);
                    let msg = panic_message(&payload);
                    log::error!(
                        "event=continent_panic continent_id={} continent_name={:?} message={:?} tick_action=continue_with_empty_result",
                        cid, name, msg
                    );
                    ContinentResult::new(cid, Vec::new(), Vec::new())
                })
            })
            .collect();
        result.panicked_continents =
            (PANICKED_CONTINENTS.load(Ordering::Relaxed) - panicks_before) as u32;

        // Phase B: collect and batch-execute all AI requests. Guarded by
        // a timeout so a hung upstream provider can't stall the whole
        // simulation forever — on timeout, responses are dropped and the
        // tick advances without applying AI decisions.
        let all_requests = ctx.ai.drain();
        let ai_count = all_requests.len();
        if !all_requests.is_empty() {
            let fut = AiBatchProcessor::execute(all_requests);
            match tokio::time::timeout(config.ai_batch_timeout, fut).await {
                Ok(completed) => apply_ai_responses(completed, data),
                Err(_) => log::error!(
                    "AI batch timed out after {:?} ({} requests dropped), tick continues",
                    config.ai_batch_timeout, ai_count
                ),
            }
        }

        // Phase C: process the collected results against post-AI data
        for continent_result in results {
            continent_result.process(data, &mut result);
        }

        // Phase D: world-level manager market. Order is load-bearing —
        // see `manager_market::tick_daily` for the dependency rationale.
        let today = data.date.date();
        manager_market::tick_daily(data, today);

        // Phase D2: parent-side loan wage settlement. Per-club monthly
        // finance runs inside Phase A and bills the borrower for the
        // loan contract; the parent club still owes the residual share
        // of its primary contract for the duration of the loan. Done
        // here at the world level because parent and borrower may live
        // in different countries — a per-country pass can't see them
        // both.
        if today.day() == 1 {
            settle_parent_residual_loan_wages(data);
        }

        // Global competitions (Champions League, World Cup, etc.)
        GlobalCompetitionSimulator::simulate(data);

        // Refresh player indexes only if a transfer actually moved a player
        // between clubs today. Walking the world every day is wasteful.
        data.rebuild_indexes_if_dirty();

        // Seed history for any players created today that haven't been seeded
        // (youth intake, regens, new clubs) — catches them within one tick.
        data.seed_missing_player_histories();

        // Periodic prune of the global match store. Cadence lives on the
        // config (default: first of every month). Cheap — BTreeMap range
        // walk over evicted dates only.
        if config.is_trim_day(current_date.date()) {
            data.match_store.trim(current_date.date());
        }

        data.next_date();

        result
    }
}

#[derive(Clone)]
pub struct SimulatorData {
    pub continents: Vec<Continent>,

    pub date: NaiveDateTime,

    pub transfer_pool: TransferPool<Player>,

    pub indexes: Option<SimulatorDataIndexes>,

    /// Set to true whenever a transfer moves a player between clubs. Checked
    /// by the simulator to decide whether to rebuild player location indexes.
    pub dirty_player_index: bool,

    pub free_agents: Vec<Player>,

    /// Coaches/managers/staff between jobs. Populated on sacking and on
    /// natural contract expiry; drained when the manager market signs
    /// a candidate. Globally scoped so a Premier League club can hire
    /// a sacked Bundesliga manager without per-country plumbing.
    pub free_agent_staff: Vec<Staff>,

    /// In-flight approaches by clubs pursuing employed managers at
    /// other clubs (slice C — poaching). Each entry is one
    /// requesting-club ↔ candidate ↔ source-club triplet that
    /// progresses through `ApproachState` over ~5 daily ticks before
    /// either resolving in a signing (with cascade) or being rejected.
    pub pending_manager_approaches:
        Vec<crate::club::board::manager_market::ManagerApproach>,

    pub watchlist: Vec<u32>,

    pub global_competitions: GlobalCompetitions,

    /// All countries by id (for nationality lookups — includes countries without active leagues)
    pub country_info: HashMap<u32, CountryInfo>,

    /// Global match result storage — all match types (league, cup, national team) write here
    pub match_store: MatchStorage,
}

impl SimulatorData {
    /// Build a SimulatorData with the deterministic sim RNG pinned to `seed`.
    /// Passing a non-zero seed makes the util-layer RNG stream reproducible
    /// per worker thread; Rayon scheduling still reorders draws across
    /// threads, so this is a debugging aid, not a replay tool.
    ///
    /// **Note: the seed is process-global state.** `set_seed` writes to
    /// the RNG engine's static; building two `SimulatorData` back-to-back
    /// means the second silently inherits whatever seed the first left
    /// behind unless this function (or `set_seed`) is called again.
    /// Don't rely on this constructor to fully isolate two simulators
    /// running in the same process.
    pub fn new_seeded(
        date: NaiveDateTime,
        continents: Vec<Continent>,
        global_competitions: GlobalCompetitions,
        seed: u64,
    ) -> Self {
        rng_engine::set_seed(seed);
        Self::new(date, continents, global_competitions)
    }

    /// Build a SimulatorData populated from `continents`.
    ///
    /// **`country_info` lifecycle:** the constructor seeds the nationality
    /// lookup map only with countries that participate in the simulation
    /// (i.e. countries whose continents are passed in). Some nationalities
    /// belong to countries that have no active leagues — those need to be
    /// added explicitly via [`add_country_info`] by the database loader
    /// before the first `simulate()` call. A nationality lookup that misses
    /// returns `None` silently, so a forgotten generator step manifests as
    /// blank flags / empty country names in the UI rather than a panic.
    pub fn new(date: NaiveDateTime, continents: Vec<Continent>, global_competitions: GlobalCompetitions) -> Self {
        // Build country_info from simulation participants
        let country_info: HashMap<u32, CountryInfo> = continents.iter()
            .flat_map(|cont| &cont.countries)
            .map(|c| (c.id, CountryInfo {
                id: c.id,
                code: c.code.clone(),
                slug: c.slug.clone(),
                name: c.name.clone(),
            }))
            .collect();

        let mut data = SimulatorData {
            continents,
            date,
            transfer_pool: TransferPool::new(),
            indexes: None,
            dirty_player_index: false,
            free_agents: Vec::new(),
            free_agent_staff: Vec::new(),
            pending_manager_approaches: Vec::new(),
            watchlist: Vec::new(),
            global_competitions,
            country_info,
            match_store: MatchStorage::new(),
        };

        let mut indexes = SimulatorDataIndexes::new();

        indexes.refresh(&data);

        data.indexes = Some(indexes);

        data.init_league_tables();
        data.seed_player_histories();

        data
    }

    /// Register country info for countries that may not have active leagues in the simulation.
    /// Called by the database generator to ensure nationality lookups always succeed.
    pub fn add_country_info(&mut self, id: u32, code: String, slug: String, name: String) {
        self.country_info.entry(id).or_insert(CountryInfo { id, code, slug, name });
    }

    /// Remove a country from the nationality lookup map.
    pub fn remove_country_info(&mut self, id: u32) {
        self.country_info.remove(&id);
    }

    /// Initial population of league tables at construction time.
    /// Per-season rebuilds happen inside `League::simulate` when a new
    /// schedule is generated. The skip-if-non-empty guard below is
    /// therefore intentional: it only prevents the initial seed from
    /// clobbering an already-populated table.
    fn init_league_tables(&mut self) {
        for continent in &mut self.continents {
            for country in &mut continent.countries {
                let clubs = &country.clubs;
                for league in &mut country.leagues.leagues {
                    if !league.table.rows.is_empty() {
                        continue;
                    }
                    let team_ids = team_ids_for_league(clubs, league.id);
                    if !team_ids.is_empty() {
                        league.table = LeagueTable::new(&team_ids);
                    }
                }
            }
        }
    }

    /// Seed statistics history for every player. Called once at
    /// construction time — touches every player unconditionally.
    fn seed_player_histories(&mut self) {
        let date = self.date.date();
        for continent in &mut self.continents {
            for country in &mut continent.countries {
                let league_lookup = build_league_lookup(country);
                for club in &mut country.clubs {
                    let club_ctx = ClubSeedingContext::resolve(club, &league_lookup);
                    for team in club.teams.iter_mut() {
                        let team_info = club_ctx.team_info_for(team);
                        for player in &mut team.players.players {
                            let is_loan = player.is_on_loan();
                            player
                                .statistics_history
                                .seed_initial_team(&team_info, date, is_loan);
                        }
                    }
                }
            }
        }
    }

    /// Seed any players whose history is still empty — catches youth intake,
    /// regens, and newly-generated clubs within one simulated tick.
    /// Skip-fast at club AND team level so the steady-state cost is close
    /// to zero when nothing needs seeding.
    pub fn seed_missing_player_histories(&mut self) {
        let date = self.date.date();
        for continent in &mut self.continents {
            for country in &mut continent.countries {
                let league_lookup = build_league_lookup(country);
                for club in &mut country.clubs {
                    if !club_has_players_needing_seed(club) {
                        continue;
                    }
                    let club_ctx = ClubSeedingContext::resolve(club, &league_lookup);
                    for team in club.teams.iter_mut() {
                        if !team_has_players_needing_seed(team) {
                            continue;
                        }
                        let team_info = club_ctx.team_info_for(team);
                        for player in &mut team.players.players {
                            if !player.statistics_history.needs_current_season_seed() {
                                continue;
                            }
                            let is_loan = player.is_on_loan();
                            player
                                .statistics_history
                                .seed_initial_team(&team_info, date, is_loan);
                        }
                    }
                }
            }
        }
    }

    pub fn next_date(&mut self) {
        self.date += Duration::days(1);
    }
}

pub struct SimulationResult {
    pub match_results: Vec<MatchResult>,
    /// Number of continents whose `simulate` call panicked during this
    /// tick. Surfaces silent failures the orchestrator catches and
    /// substitutes empty results for. Sum across ticks via the
    /// process-global `panicked_continents_total()`.
    pub panicked_continents: u32,
}

impl Default for SimulationResult {
    fn default() -> Self {
        Self::new()
    }
}

impl SimulationResult {
    pub fn new() -> Self {
        SimulationResult {
            match_results: Vec::new(),
            panicked_continents: 0,
        }
    }

    pub fn has_match_results(&self) -> bool {
        !self.match_results.is_empty()
    }
}

/// Walk every player in the world, find loaned-in players, and bill
/// the parent club for its residual share of the primary contract that
/// the borrower's loan contract didn't cover. Runs once per calendar
/// month from `simulate_with`.
///
/// Residual = `(parent_salary - loan_salary).max(0) / 12`. When
/// `loan_wage_contribution_pct` is recorded it implies the loan salary
/// is already a percentage of the parent salary, so the residual
/// arithmetic is correct without a separate pct path. Negative
/// residuals (borrower paying more than the parent contract — should
/// not happen in practice) are clamped to zero so we never accidentally
/// credit the parent.
fn settle_parent_residual_loan_wages(data: &mut SimulatorData) {
    // Pass 1 (read): collect (parent_club_id, monthly_residual) entries
    // so we don't hold borrows across the credit pass.
    let mut owed_by_parent: HashMap<u32, i64> = HashMap::new();
    for continent in &data.continents {
        for country in &continent.countries {
            for club in &country.clubs {
                for team in club.teams.teams.iter() {
                    for player in team.players.players.iter() {
                        let Some(loan) = player.contract_loan.as_ref() else {
                            continue;
                        };
                        let Some(parent_id) = loan.loan_from_club_id else {
                            continue;
                        };
                        let Some(parent_contract) = player.contract.as_ref() else {
                            continue;
                        };
                        let parent_annual = parent_contract.salary;
                        let borrower_annual = loan.salary;
                        let residual_annual =
                            parent_annual.saturating_sub(borrower_annual);
                        if residual_annual == 0 {
                            continue;
                        }
                        let monthly = (residual_annual / 12) as i64;
                        if monthly > 0 {
                            *owed_by_parent.entry(parent_id).or_insert(0) += monthly;
                        }
                    }
                }
            }
        }
    }

    // Pass 2 (write): charge each parent club once.
    for (parent_id, amount) in owed_by_parent {
        if let Some(club) = data.club_mut(parent_id) {
            club.finance.balance.push_expense_player_wages(amount);
        }
    }
}

// ============================================================
// Internal: league-seeding helpers
// ============================================================

/// Collect every team id that participates in `league_id` across the
/// given clubs. Extracted so `init_league_tables`'s outer loop reads as
/// "for each league, install a table from the team ids" without an inline
/// `flat_map` chain.
fn team_ids_for_league(clubs: &[crate::Club], league_id: u32) -> Vec<u32> {
    clubs
        .iter()
        .flat_map(|c| c.teams.with_league(league_id))
        .collect()
}

// ============================================================
// Internal: history-seeding helpers
// ============================================================

/// Per-country `league_id -> (name, slug)` cache. Built once at the start
/// of a country's seeding sweep so the per-club main-team lookup is O(1).
fn build_league_lookup(country: &crate::Country) -> HashMap<u32, (String, String)> {
    country
        .leagues
        .leagues
        .iter()
        .map(|l| (l.id, (l.name.clone(), l.slug.clone())))
        .collect()
}

/// True if any team in the club has at least one player needing a current-
/// season seed entry. Cheap traversal — exits as soon as one is found.
fn club_has_players_needing_seed(club: &crate::Club) -> bool {
    club.teams
        .iter()
        .any(|t| team_has_players_needing_seed(t))
}

fn team_has_players_needing_seed(team: &crate::club::Team) -> bool {
    team.players
        .iter()
        .any(|p| p.statistics_history.needs_current_season_seed())
}

/// Snapshot of the club's main-team identity for stats-seeding purposes.
/// Resolved once per club so non-main teams (reserve, U21) inherit the
/// main brand consistently across all their players.
struct ClubSeedingContext {
    main_name: Option<String>,
    main_slug: Option<String>,
    main_reputation: u16,
    main_league_name: String,
    main_league_slug: String,
}

impl ClubSeedingContext {
    fn resolve(
        club: &crate::Club,
        league_lookup: &HashMap<u32, (String, String)>,
    ) -> Self {
        let main_team = club.teams.main();
        let main_name = main_team.map(|t| t.name.clone());
        let main_slug = main_team.map(|t| t.slug.clone());
        let main_reputation = main_team.map(|t| t.reputation.world).unwrap_or(0);
        let (main_league_name, main_league_slug) = main_team
            .and_then(|t| t.league_id)
            .and_then(|lid| league_lookup.get(&lid))
            .map(|(n, s)| (n.clone(), s.clone()))
            .unwrap_or_default();
        ClubSeedingContext {
            main_name,
            main_slug,
            main_reputation,
            main_league_name,
            main_league_slug,
        }
    }

    /// Build the `TeamInfo` that the seeder writes onto the player's
    /// history. Main teams use their own identity; non-main teams inherit
    /// the main brand so stats aggregate correctly under one club name.
    fn team_info_for(&self, team: &crate::club::Team) -> TeamInfo {
        let inherit = team.team_type != TeamType::Main && self.main_name.is_some();
        if inherit {
            TeamInfo {
                name: self.main_name.clone().unwrap_or_default(),
                slug: self.main_slug.clone().unwrap_or_default(),
                reputation: self.main_reputation,
                league_name: self.main_league_name.clone(),
                league_slug: self.main_league_slug.clone(),
            }
        } else {
            TeamInfo {
                name: team.name.clone(),
                slug: team.slug.clone(),
                reputation: team.reputation.world,
                league_name: self.main_league_name.clone(),
                league_slug: self.main_league_slug.clone(),
            }
        }
    }
}
