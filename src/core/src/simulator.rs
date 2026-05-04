use crate::ai::{Ai, AiBatchProcessor};
use crate::club::ai::apply_ai_responses;
use crate::club::board::manager_market;
use crate::competitions::GlobalCompetitions;
use crate::competitions::simulation::GlobalCompetitionSimulator;
use crate::config::SimulatorConfig;
use crate::context::{GlobalContext, SimulationContext};
use crate::continent::national::world as national_world;
use crate::continent::{Continent, ContinentResult};
use crate::league::awards::{
    AwardAggregator, MonthlyAwardSelector, MonthlyPlayerAward, SeasonAwardsSnapshot,
    TeamOfTheWeekAward, TeamOfTheWeekSelector, TeamOfTheWeekSlot,
};
use crate::league::player_of_week::{PlayerOfTheWeekAward, PlayerOfTheWeekSelector};
use crate::league::{LeagueTable, MatchStorage};
use crate::r#match::MatchResult;
use crate::shared::SimulatorDataIndexes;
use crate::transfers::TransferPool;
use crate::utils::DateUtils;
use crate::utils::random::engine as rng_engine;
use crate::{HappinessEventType, Player, Staff, TeamInfo};
use chrono::{Datelike, Duration, NaiveDateTime, Weekday};
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
    /// Continent the country sits on. Carried here so the region-prestige
    /// gate used by the loan market, scouting, and personal-terms
    /// negotiation can resolve a `ScoutingRegion` even for nationalities
    /// whose country has no active leagues in this save.
    pub continent_id: u32,
    /// Football reputation (0..10000). Mirrors the same field on `Country`
    /// so the country-reputation realism gate keeps working when the
    /// nationality's leagues aren't loaded — without this it falls back to
    /// `0` and an Argentinian free agent slips through to a Mali buyer.
    pub reputation: u16,
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

        // National-team call-ups run at the world level so a player's
        // nationality and their club's continent can differ. Must
        // happen BEFORE the world-level national-competition phase —
        // those matches need a populated squad with world visibility.
        data.process_world_national_team_callups();

        // National-team competition matches simulate at the world level
        // so squads can include foreign-based players and post-match
        // stats updates fan out across every continent. Lifted out of
        // the parallel continent phase because squad construction needs
        // read access to clubs in *every* continent.
        let national_match_results = national_world::simulate_world_national_competitions(
            &mut data.continents,
            current_date.date(),
        );
        for match_result in &national_match_results {
            data.match_store
                .push(match_result.clone(), current_date.date());
        }
        result.match_results.extend(national_match_results);

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

        // Phase B: collect and batch-execute all AI requests. The tick
        // waits for the batch to finish — no timeout, no dropped responses.
        let all_requests = ctx.ai.drain();
        if !all_requests.is_empty() {
            let completed = AiBatchProcessor::execute(all_requests).await;
            apply_ai_responses(completed, data);
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

        // Release Int statuses AFTER all matches (continent + global) —
        // a tournament final on the release date should be played
        // before the squad's flags are cleared.
        data.process_world_national_team_release();

        // Move any player whose contract was cleared this tick (positional
        // surplus, free-transfer release, contract expiry) off their old
        // team's roster and into the global free-agent pool, so the player
        // page header and contract panel agree.
        data.sweep_released_to_free_agents();

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

        // Pick each league's Player of the Week. Runs every Monday after
        // the matchday pipeline has flushed last week's results into each
        // league's MatchStorage.
        WeeklyAwardsTick::run(data);
        // Team of the Week — one XI per league, every Monday.
        TeamOfTheWeekTick::run(data);
        // Monthly awards — first day of each month, awarding the previous
        // calendar month.
        MonthlyAwardsTick::run(data);
        // Drain any league-side pending season-awards snapshots and emit
        // the player events while stats are still meaningful.
        SeasonAwardsTick::run(data);
        // World player of the year — runs once per year. Builds a global
        // ranking from per-continent rankings so a top performer in any
        // league can win.
        WorldPlayerOfYearTick::run(data);

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
    pub pending_manager_approaches: Vec<crate::club::board::manager_market::ManagerApproach>,

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
    pub fn new(
        date: NaiveDateTime,
        continents: Vec<Continent>,
        global_competitions: GlobalCompetitions,
    ) -> Self {
        // Build country_info from simulation participants
        let country_info: HashMap<u32, CountryInfo> = continents
            .iter()
            .flat_map(|cont| &cont.countries)
            .map(|c| {
                (
                    c.id,
                    CountryInfo {
                        id: c.id,
                        code: c.code.clone(),
                        slug: c.slug.clone(),
                        name: c.name.clone(),
                        continent_id: c.continent_id,
                        reputation: c.reputation,
                    },
                )
            })
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
    pub fn add_country_info(
        &mut self,
        id: u32,
        code: String,
        slug: String,
        name: String,
        continent_id: u32,
        reputation: u16,
    ) {
        self.country_info.entry(id).or_insert(CountryInfo {
            id,
            code,
            slug,
            name,
            continent_id,
            reputation,
        });
    }

    /// Walk every player slot in the simulator and bump the procedural id
    /// sequence past the highest id seen. The single source of truth for
    /// future id allocation — call this after world generation (and after
    /// any future save-load path) so runtime academy intake / U18 fallback
    /// can never collide with an id that already exists in the world.
    /// Cheap: a single pass over all rosters; only runs at startup.
    pub fn seed_player_id_sequence(&self) {
        let mut max_id: u32 = 0;
        for continent in &self.continents {
            for country in &continent.countries {
                for club in &country.clubs {
                    for team in &club.teams.teams {
                        for player in &team.players.players {
                            if player.id > max_id {
                                max_id = player.id;
                            }
                        }
                    }
                }
                for player in &country.retired_players {
                    if player.id > max_id {
                        max_id = player.id;
                    }
                }
                for player in &country.national_team.generated_squad {
                    if player.id > max_id {
                        max_id = player.id;
                    }
                }
            }
        }
        for player in &self.free_agents {
            if player.id > max_id {
                max_id = player.id;
            }
        }
        crate::seed_core_player_id_sequence(max_id);
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

    /// Move every team-attached player whose main-club contract is `None`
    /// onto the global `free_agents` pool. Several pipelines (positional
    /// surplus, unresolved-salary "free transfer", contract expiry) clear
    /// the contract in place; without this sweep the player lingers on the
    /// roster as a "free agent on a team," which the player page renders
    /// inconsistently — the header reads the team name while the contract
    /// panel reads "Free Agent."
    ///
    /// Each move is logged as a `CompletedTransfer` (zero fee, `Free`
    /// type) on the losing club's country, so the transfer history page
    /// reflects the departure. Reason is derived from the player's
    /// status: `Frt` set means the club explicitly released early
    /// (mutual / surplus / unresolved-salary path); otherwise the
    /// contract simply expired.
    ///
    /// Loanees are skipped (their `contract` is the parent-club contract
    /// and stays `Some` during the loan), as are retired players (already
    /// removed from team rosters by the retirement pipeline). Sets
    /// `dirty_player_index` so the next index rebuild picks up the moves.
    pub fn sweep_released_to_free_agents(&mut self) {
        use crate::PlayerStatusType;
        use crate::shared::{Currency, CurrencyValue};
        use crate::transfers::{CompletedTransfer, TransferType};

        let date = self.date.date();
        let mut released: Vec<Player> = Vec::new();
        for continent in &mut self.continents {
            for country in &mut continent.countries {
                let mut new_history: Vec<CompletedTransfer> = Vec::new();
                for club in &mut country.clubs {
                    let club_id = club.id;
                    for team in &mut club.teams.teams {
                        let team_id = team.id;
                        let team_name = team.name.clone();
                        let candidates: Vec<(u32, String, bool)> = team
                            .players
                            .players
                            .iter()
                            .filter(|p| p.contract.is_none() && !p.is_on_loan() && !p.retired)
                            .map(|p| {
                                let was_released_early =
                                    p.statuses.get().contains(&PlayerStatusType::Frt);
                                (p.id, p.full_name.to_string(), was_released_early)
                            })
                            .collect();
                        for (id, player_name, released_early) in candidates {
                            if let Some(p) = team.players.take_player(&id) {
                                let reason = if released_early {
                                    "dec_reason_released_free".to_string()
                                } else {
                                    "dec_reason_contract_expired".to_string()
                                };
                                new_history.push(
                                    CompletedTransfer::new(
                                        id,
                                        player_name,
                                        club_id,
                                        team_id,
                                        team_name.clone(),
                                        0,
                                        "Free Agent".to_string(),
                                        date,
                                        CurrencyValue::new(0.0, Currency::Usd),
                                        TransferType::Free,
                                    )
                                    .with_reason(reason),
                                );
                                released.push(p);
                            }
                        }
                    }
                }
                country.transfer_market.transfer_history.extend(new_history);
            }
        }
        if !released.is_empty() {
            self.dirty_player_index = true;
            self.free_agents.extend(released);
        }
    }

    pub fn next_date(&mut self) {
        self.date += Duration::days(1);
    }

    /// World-level national-team call-ups. Runs at the start of each
    /// break/tournament window, before any continent simulates, so
    /// candidate visibility spans the entire world — a Brazilian
    /// playing at a Spanish club is reachable from Brazil's selection
    /// pool without per-continent plumbing.
    pub fn process_world_national_team_callups(&mut self) {
        let date = self.date.date();
        let need_callups = crate::NationalTeam::is_break_start(date)
            || crate::NationalTeam::is_tournament_start(date);
        if !need_callups {
            return;
        }

        // Build a global candidate pool from every club in every country.
        let mut candidates_by_country = crate::NationalTeam::collect_all_candidates_by_country(
            self.continents.iter().flat_map(|c| c.countries.iter()),
            date,
        );

        // Country IDs across the whole world — used to draw friendly
        // opponents from any nation, not just same-continent.
        let country_ids: Vec<(u32, String)> = self
            .continents
            .iter()
            .flat_map(|c| c.countries.iter())
            .map(|c| (c.id, c.name.clone()))
            .collect();

        for continent in &mut self.continents {
            for country in &mut continent.countries {
                country.national_team.country_name = country.name.clone();
                country.national_team.reputation = country.reputation;
                let candidates = candidates_by_country
                    .remove(&country.id)
                    .unwrap_or_default();
                let cid = country.id;
                country
                    .national_team
                    .call_up_squad(candidates, date, cid, &country_ids);
            }
        }

        // Apply Int status across every club in every continent.
        crate::NationalTeam::apply_callup_statuses_across_world(&mut self.continents, date);
    }

    /// World-level Int release. Runs after all matches (continent
    /// matches + global tournament matches) so a tournament final
    /// landing on a release date is played with squad statuses still
    /// attached. Squad data itself is preserved for the squad UI; only
    /// the per-player Int flag is cleared.
    pub fn process_world_national_team_release(&mut self) {
        let date = self.date.date();
        let need_release =
            crate::NationalTeam::is_break_end(date) || crate::NationalTeam::is_tournament_end(date);
        if !need_release {
            return;
        }
        crate::NationalTeam::release_callup_statuses_across_world(&mut self.continents);
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
                        let residual_annual = parent_annual.saturating_sub(borrower_annual);
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

/// Monday-only orchestration that walks every non-friendly league, picks
/// its Player of the Week from last calendar week's matches, and applies
/// the side effects (player happiness event + league archive). Two-pass
/// design avoids overlapping `&` and `&mut` borrows of `SimulatorData`.
struct WeeklyAwardsTick;

impl WeeklyAwardsTick {
    fn run(data: &mut SimulatorData) {
        let today = data.date.date();
        if today.weekday() != Weekday::Mon {
            return;
        }
        let week_end = today;
        let week_start = today - Duration::days(7);

        let pending = Self::collect_pending(data, week_start, week_end);
        Self::apply_pending(data, pending);
    }

    fn collect_pending(
        data: &SimulatorData,
        week_start: chrono::NaiveDate,
        week_end: chrono::NaiveDate,
    ) -> Vec<PendingWeeklyAward> {
        let mut pending: Vec<PendingWeeklyAward> = Vec::new();
        for continent in &data.continents {
            for country in &continent.countries {
                for league in &country.leagues.leagues {
                    if league.friendly {
                        continue;
                    }
                    if league.player_of_week.has_award_for_week(week_end) {
                        continue;
                    }

                    let scores = PlayerOfTheWeekSelector::aggregate(
                        league.matches.iter_in_range(week_start, week_end),
                    );
                    let Some((winner_id, agg)) = PlayerOfTheWeekSelector::pick_winner(&scores)
                    else {
                        continue;
                    };

                    let Some(player) = data.player(winner_id) else {
                        continue;
                    };
                    let player_name = format!(
                        "{} {}",
                        player.full_name.display_first_name(),
                        player.full_name.display_last_name()
                    );
                    let player_slug = player.slug();
                    let (club_id, club_name, club_slug) =
                        Self::resolve_club_card(data, winner_id);
                    let average_rating = if agg.matches_played > 0 {
                        agg.rating_sum / agg.matches_played as f32
                    } else {
                        0.0
                    };

                    pending.push(PendingWeeklyAward {
                        league_id: league.id,
                        winner_id,
                        award: PlayerOfTheWeekAward {
                            week_end_date: week_end,
                            player_id: winner_id,
                            player_name,
                            player_slug,
                            club_id,
                            club_name,
                            club_slug,
                            score: agg.score,
                            goals: agg.goals,
                            assists: agg.assists,
                            matches_played: agg.matches_played,
                            average_rating,
                        },
                    });
                }
            }
        }
        pending
    }

    fn apply_pending(data: &mut SimulatorData, pending: Vec<PendingWeeklyAward>) {
        for entry in pending {
            if let Some(player) = data.player_mut(entry.winner_id) {
                player.on_player_of_the_week();
            }
            if let Some(league) = data.league_mut(entry.league_id) {
                league.player_of_week.record(entry.award);
            }
        }
    }

    /// Resolve the active-club card (id, display name, slug) for a
    /// winning player. Falls back to empty values if the player isn't on
    /// a roster (free agent at award time — extremely unlikely
    /// Monday-morning, but guarded against).
    fn resolve_club_card(data: &SimulatorData, player_id: u32) -> (u32, String, String) {
        let location = data
            .indexes
            .as_ref()
            .and_then(|i| i.get_player_location(player_id));
        let Some((_, _, club_id, _)) = location else {
            return (0, String::new(), String::new());
        };
        let Some(club) = data.club(club_id) else {
            return (club_id, String::new(), String::new());
        };
        let main_team = club.teams.main();
        let club_name = main_team
            .map(|t| t.name.clone())
            .unwrap_or_else(|| club.name.clone());
        let club_slug = main_team
            .map(|t| t.slug.clone())
            .unwrap_or_else(String::new);
        (club_id, club_name, club_slug)
    }
}

struct PendingWeeklyAward {
    league_id: u32,
    winner_id: u32,
    award: PlayerOfTheWeekAward,
}

/// Monday-only Team of the Week selection. Builds an XI per league with
/// the canonical 1-4-3-3 quotas and emits `TeamOfTheWeekSelection` to each
/// selected player.
struct TeamOfTheWeekTick;

struct PendingTeamOfWeek {
    league_id: u32,
    award: TeamOfTheWeekAward,
}

impl TeamOfTheWeekTick {
    fn run(data: &mut SimulatorData) {
        let today = data.date.date();
        if today.weekday() != Weekday::Mon {
            return;
        }
        let week_end = today;
        let week_start = today - Duration::days(7);
        let pending = Self::collect(data, week_start, week_end);
        Self::apply(data, pending);
    }

    fn collect(
        data: &SimulatorData,
        week_start: chrono::NaiveDate,
        week_end: chrono::NaiveDate,
    ) -> Vec<PendingTeamOfWeek> {
        let mut pending: Vec<PendingTeamOfWeek> = Vec::new();
        for continent in &data.continents {
            for country in &continent.countries {
                for league in &country.leagues.leagues {
                    if league.friendly {
                        continue;
                    }
                    if league.awards.has_team_of_week_for(week_end) {
                        continue;
                    }
                    let scores =
                        AwardAggregator::aggregate(league.matches.iter_in_range(week_start, week_end));
                    let team = TeamOfTheWeekSelector::pick(&scores);
                    if team.is_empty() {
                        continue;
                    }

                    let mut slots: Vec<TeamOfTheWeekSlot> = Vec::with_capacity(team.len());
                    for (pid, pos, score, agg) in team {
                        let Some(player) = data.player(pid) else {
                            continue;
                        };
                        let player_name = format!(
                            "{} {}",
                            player.full_name.display_first_name(),
                            player.full_name.display_last_name()
                        );
                        let player_slug = player.slug();
                        let (club_id, club_name, club_slug) = WeeklyAwardsTick::resolve_club_card(data, pid);
                        slots.push(TeamOfTheWeekSlot {
                            player_id: pid,
                            player_name,
                            player_slug,
                            club_id,
                            club_name,
                            club_slug,
                            position_group: pos,
                            score,
                            matches_played: agg.matches_played,
                            goals: agg.goals,
                            assists: agg.assists,
                            average_rating: agg.average_rating(),
                        });
                    }
                    pending.push(PendingTeamOfWeek {
                        league_id: league.id,
                        award: TeamOfTheWeekAward {
                            week_end_date: week_end,
                            slots,
                        },
                    });
                }
            }
        }
        pending
    }

    fn apply(data: &mut SimulatorData, pending: Vec<PendingTeamOfWeek>) {
        for entry in pending {
            for slot in &entry.award.slots {
                if let Some(player) = data.player_mut(slot.player_id) {
                    player.happiness.add_event_default_with_cooldown(
                        HappinessEventType::TeamOfTheWeekSelection,
                        6,
                    );
                }
            }
            if let Some(league) = data.league_mut(entry.league_id) {
                league.awards.record_team_of_week(entry.award);
            }
        }
    }
}

/// Monthly awards — POM and Young POM per league. Runs on the 1st of
/// each calendar month, awarding the *previous* calendar month.
struct MonthlyAwardsTick;

struct PendingMonthlyAward {
    league_id: u32,
    pom: Option<MonthlyPlayerAward>,
    young: Option<MonthlyPlayerAward>,
}

impl MonthlyAwardsTick {
    fn run(data: &mut SimulatorData) {
        let today = data.date.date();
        if today.day() != 1 {
            return;
        }
        let (start, end) = match Self::previous_month_window(today) {
            Some(w) => w,
            None => return,
        };

        let pending = Self::collect(data, today, start, end);
        Self::apply(data, pending);
    }

    /// First-of-month → start = first of previous month, end = first of
    /// this month (exclusive in `iter_in_range`).
    fn previous_month_window(today: chrono::NaiveDate) -> Option<(chrono::NaiveDate, chrono::NaiveDate)> {
        let first_this_month = chrono::NaiveDate::from_ymd_opt(today.year(), today.month(), 1)?;
        let prev_month = if today.month() == 1 {
            chrono::NaiveDate::from_ymd_opt(today.year() - 1, 12, 1)?
        } else {
            chrono::NaiveDate::from_ymd_opt(today.year(), today.month() - 1, 1)?
        };
        Some((prev_month, first_this_month))
    }

    fn collect(
        data: &SimulatorData,
        today: chrono::NaiveDate,
        start: chrono::NaiveDate,
        end: chrono::NaiveDate,
    ) -> Vec<PendingMonthlyAward> {
        let month_end = end - Duration::days(1);
        let mut pending: Vec<PendingMonthlyAward> = Vec::new();

        for continent in &data.continents {
            for country in &continent.countries {
                for league in &country.leagues.leagues {
                    if league.friendly {
                        continue;
                    }
                    if league.awards.has_monthly_award_for(month_end) {
                        continue;
                    }
                    let scores =
                        AwardAggregator::aggregate(league.matches.iter_in_range(start, end));

                    let pom = MonthlyAwardSelector::pick_best(&scores, league.reputation, 3, |_| {
                        true
                    })
                    .and_then(|(id, agg, score)| {
                        let player = data.player(id)?;
                        let (club_id, club_name, club_slug) =
                            WeeklyAwardsTick::resolve_club_card(data, id);
                        Some(MonthlyPlayerAward {
                            month_end_date: month_end,
                            player_id: id,
                            player_name: format!(
                                "{} {}",
                                player.full_name.display_first_name(),
                                player.full_name.display_last_name()
                            ),
                            player_slug: player.slug(),
                            club_id,
                            club_name,
                            club_slug,
                            matches_played: agg.matches_played,
                            goals: agg.goals,
                            assists: agg.assists,
                            average_rating: agg.average_rating(),
                            score,
                        })
                    });

                    let data_ref = data;
                    let young = MonthlyAwardSelector::pick_best(
                        &scores,
                        league.reputation,
                        2,
                        |id| {
                            data_ref
                                .player(id)
                                .map(|p| DateUtils::age(p.birth_date, today) <= 21)
                                .unwrap_or(false)
                        },
                    )
                    .and_then(|(id, agg, score)| {
                        let player = data.player(id)?;
                        let (club_id, club_name, club_slug) =
                            WeeklyAwardsTick::resolve_club_card(data, id);
                        Some(MonthlyPlayerAward {
                            month_end_date: month_end,
                            player_id: id,
                            player_name: format!(
                                "{} {}",
                                player.full_name.display_first_name(),
                                player.full_name.display_last_name()
                            ),
                            player_slug: player.slug(),
                            club_id,
                            club_name,
                            club_slug,
                            matches_played: agg.matches_played,
                            goals: agg.goals,
                            assists: agg.assists,
                            average_rating: agg.average_rating(),
                            score,
                        })
                    });

                    if pom.is_some() || young.is_some() {
                        pending.push(PendingMonthlyAward {
                            league_id: league.id,
                            pom,
                            young,
                        });
                    }
                }
            }
        }
        pending
    }

    fn apply(data: &mut SimulatorData, pending: Vec<PendingMonthlyAward>) {
        for entry in pending {
            if let Some(award) = &entry.pom {
                if let Some(player) = data.player_mut(award.player_id) {
                    player.happiness.add_event_default_with_cooldown(
                        HappinessEventType::PlayerOfTheMonth,
                        28,
                    );
                }
            }
            if let Some(award) = &entry.young {
                if let Some(player) = data.player_mut(award.player_id) {
                    player.happiness.add_event_default_with_cooldown(
                        HappinessEventType::YoungPlayerOfTheMonth,
                        28,
                    );
                }
            }
            if let Some(league) = data.league_mut(entry.league_id) {
                if let Some(a) = entry.pom {
                    league.awards.record_player_of_month(a);
                }
                if let Some(a) = entry.young {
                    league.awards.record_young_player_of_month(a);
                }
            }
        }
    }
}

/// Season awards — drains each league's pending snapshot (built inside
/// `process_season_end` before stats archive) and fires player events.
struct SeasonAwardsTick;

impl SeasonAwardsTick {
    fn run(data: &mut SimulatorData) {
        let today = data.date.date();
        let pending: Vec<(u32, SeasonAwardsSnapshot)> = data
            .continents
            .iter_mut()
            .flat_map(|c| c.countries.iter_mut())
            .flat_map(|c| c.leagues.leagues.iter_mut())
            .filter_map(|l| {
                l.awards
                    .pending_season_awards
                    .take()
                    .map(|s| (l.id, s))
            })
            .collect();

        for (league_id, snapshot) in pending {
            if let Some(player) = snapshot
                .player_of_season
                .and_then(|id| data.player_mut(id))
            {
                player.happiness.add_event_default_with_cooldown(
                    HappinessEventType::PlayerOfTheSeason,
                    330,
                );
            }
            if let Some(player) = snapshot
                .young_player_of_season
                .and_then(|id| data.player_mut(id))
            {
                player.happiness.add_event_default_with_cooldown(
                    HappinessEventType::YoungPlayerOfTheSeason,
                    330,
                );
            }
            for pid in &snapshot.team_of_season {
                if let Some(player) = data.player_mut(*pid) {
                    player.happiness.add_event_default_with_cooldown(
                        HappinessEventType::TeamOfTheSeasonSelection,
                        330,
                    );
                }
            }
            if let Some(player) = snapshot.top_scorer.and_then(|id| data.player_mut(id)) {
                player.happiness.add_event_default_with_cooldown(
                    HappinessEventType::LeagueTopScorer,
                    330,
                );
            }
            if let Some(player) = snapshot.top_assists.and_then(|id| data.player_mut(id)) {
                player.happiness.add_event_default_with_cooldown(
                    HappinessEventType::LeagueTopAssists,
                    330,
                );
            }
            if let Some(player) = snapshot.golden_glove.and_then(|id| data.player_mut(id)) {
                player.happiness.add_event_default_with_cooldown(
                    HappinessEventType::LeagueGoldenGlove,
                    330,
                );
            }
            // Archive the snapshot once events have been applied.
            let mut snapshot = snapshot;
            snapshot.season_end_date = today;
            if let Some(league) = data.league_mut(league_id) {
                league.awards.record_season(snapshot);
            }
        }
    }
}

/// World player-of-year. Runs once on year-end. Pools each continent's
/// ranking, picks the global top 3 (nominees) and the global #1
/// (winner). Reuses `ContinentResult::rank_continent` so the scoring
/// formula has a single source of truth.
struct WorldPlayerOfYearTick;

impl WorldPlayerOfYearTick {
    fn run(data: &mut SimulatorData) {
        let today = data.date.date();
        if !DateUtils::is_year_end(today) {
            return;
        }

        let mut combined: Vec<(u32, f32)> = data
            .continents
            .iter()
            .flat_map(|c| {
                crate::continent::ContinentResult::rank_continent(c, today)
            })
            .collect();
        combined.sort_by(|a, b| {
            b.1.partial_cmp(&a.1)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then(a.0.cmp(&b.0))
        });

        let top_three: Vec<u32> = combined.iter().take(3).map(|(id, _)| *id).collect();
        let winner = combined.first().map(|(id, _)| *id);

        for pid in top_three {
            if let Some(player) = data.player_mut(pid) {
                player.happiness.add_event_default_with_cooldown(
                    HappinessEventType::WorldPlayerOfYearNomination,
                    330,
                );
            }
        }
        if let Some(id) = winner {
            if let Some(player) = data.player_mut(id) {
                player.happiness.add_event_default_with_cooldown(
                    HappinessEventType::WorldPlayerOfYear,
                    330,
                );
                let cur = player.player_attributes.current_reputation;
                let home = player.player_attributes.home_reputation;
                let world = player.player_attributes.world_reputation;
                player.player_attributes.update_reputation(
                    ((cur as i32 + 900).min(10000) - cur as i32) as i16,
                    ((home as i32 + 900).min(10000) - home as i32) as i16,
                    ((world as i32 + 500).min(10000) - world as i32) as i16,
                );
            }
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
    club.teams.iter().any(|t| team_has_players_needing_seed(t))
}

fn team_has_players_needing_seed(team: &crate::club::Team) -> bool {
    team.players
        .iter()
        .any(|p| p.statistics_history.needs_current_season_seed())
}

/// Snapshot of the club's main-team identity for stats-seeding purposes.
/// Resolved once per club so youth teams (U18-U23) and Reserve inherit the
/// main brand consistently across all their players. Senior reserves
/// (B, Second) keep their own identity because they compete in real
/// lower divisions and players' histories should show that.
struct ClubSeedingContext {
    main_name: Option<String>,
    main_slug: Option<String>,
    main_reputation: u16,
    main_league_name: String,
    main_league_slug: String,
    league_lookup: HashMap<u32, (String, String)>,
}

impl ClubSeedingContext {
    fn resolve(club: &crate::Club, league_lookup: &HashMap<u32, (String, String)>) -> Self {
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
            league_lookup: league_lookup.clone(),
        }
    }

    /// Build the `TeamInfo` that the seeder writes onto the player's
    /// history. Main, B and Second teams keep their own identity (each
    /// competes in a real league); youth and Reserve squads inherit the
    /// main brand so synthetic sub-league stats aggregate under the club.
    fn team_info_for(&self, team: &crate::club::Team) -> TeamInfo {
        let keeps_own_identity = team.team_type.is_own_team();
        if keeps_own_identity {
            let (league_name, league_slug) = team
                .league_id
                .and_then(|lid| self.league_lookup.get(&lid))
                .cloned()
                .unwrap_or_else(|| (self.main_league_name.clone(), self.main_league_slug.clone()));
            TeamInfo {
                name: team.name.clone(),
                slug: team.slug.clone(),
                reputation: team.reputation.world,
                league_name,
                league_slug,
            }
        } else if self.main_name.is_some() {
            TeamInfo {
                name: self.main_name.clone().unwrap_or_default(),
                slug: self.main_slug.clone().unwrap_or_default(),
                reputation: self.main_reputation,
                league_name: self.main_league_name.clone(),
                league_slug: self.main_league_slug.clone(),
            }
        } else {
            // Club has no main team at all — fall back to the team's own info.
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
