use super::CountryInfo;
use super::FreeAgentFlowCounters;
use crate::club::board::manager::ManagerApproach;
use crate::competitions::GlobalCompetitions;
use crate::continent::Continent;
use crate::country::result::transfers::GlobalFreeAgentSummary;
use crate::league::MatchStorage;
use crate::shared::SimulatorDataIndexes;
use crate::transfers::market::map::{CountryTransferProfile, MarketMap};
use crate::transfers::pipeline::PlayerSummary;
use crate::utils::random::engine as rng_engine;
use crate::{Player, Staff};
use chrono::{Duration, NaiveDateTime};
use std::collections::HashMap;
use std::sync::Arc;

#[derive(Clone)]
pub struct SimulatorData {
    pub continents: Vec<Continent>,

    pub date: NaiveDateTime,

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
    pub pending_manager_approaches: Vec<ManagerApproach>,

    pub watchlist: Vec<u32>,

    pub global_competitions: GlobalCompetitions,

    /// All countries by id (for nationality lookups — includes countries without active leagues)
    pub country_info: HashMap<u32, CountryInfo>,

    /// The world's transfer geography: every country's corridor card plus
    /// the facts (region, reputation, wage level) the derived fallback and
    /// `import_capacity` are computed from.
    ///
    /// Built once at construction from `country_info`, refreshed at each
    /// transfer-window boundary so the money axis tracks a world whose wages
    /// have moved. Never per candidate: 224 × 224 pairs answered from sparse
    /// lists is nothing, and answering them inside a scouting loop is not.
    ///
    /// Behind an `Arc` because `initiate_foreign_negotiations` needs an
    /// OWNED handle: the geography gate reads the map for every candidate
    /// and the resolve pass beside it needs `&mut data`, so a shared borrow
    /// cannot span the two. Cloning the whole map to bridge that — a few
    /// hundred kilobytes of sparse lists, once per country per tick — was
    /// the largest single cost the geography added to the world sim.
    pub market_map: Arc<MarketMap>,

    /// Global match result storage — all match types (league, cup, national team) write here
    pub match_store: MatchStorage,

    /// Per-tick scratch cache: every non-loaned player in the world,
    /// summarised once at the top of Phase C so per-country transfer
    /// markets reuse the snapshot instead of rebuilding it per call.
    /// Reset (`= None`) at the end of each `simulate_with` tick;
    /// readers fall back to a local rebuild when the cache is `None`
    /// so test paths and one-off callers still work.
    pub daily_world_player_pool: Option<Vec<PlayerSummary>>,

    /// Per-tick scratch cache: snapshot of every globally-pooled free
    /// agent. Same lifecycle as `daily_world_player_pool` —
    /// `simulate_transfer_market` would otherwise call
    /// `snapshot_global_free_agents` per country, which mutates each
    /// player's `free_agent_state` (idempotent on repeat with the same
    /// date) and walks every free agent. Crate-private because the
    /// snapshot type is internal to the country/result module.
    pub(crate) daily_global_free_agents: Option<Vec<GlobalFreeAgentSummary>>,

    /// Monthly free-agent market flow counters (signed from the global
    /// pool, signed off a domestic expiry, signed on a pre-contract,
    /// released into the pool, retired out of it). A point-in-time scan of
    /// `free_agents` can't recover these flows — the signed / released /
    /// retired players have already moved — so the execution, sweep, and
    /// retirement passes bump them as events land, and
    /// `FreeAgentMarketAuditor::log_pool_stats` reads them on the first of
    /// each month before the caller `reset`s them.
    pub free_agent_flow: FreeAgentFlowCounters,
}

impl SimulatorData {
    /// Build a SimulatorData with the deterministic sim RNG pinned to `seed`.
    /// Passing a non-zero seed makes the util-layer RNG stream reproducible
    /// per worker thread; Rayon scheduling still reorders draws across
    /// threads, so this is a debugging aid, not a replay tool.
    ///
    /// **Note: the seed is process-global state.** `RandomEngine::set_seed`
    /// writes to the RNG engine's static; building two `SimulatorData`
    /// back-to-back means the second silently inherits whatever seed the
    /// first left behind unless this function (or `RandomEngine::set_seed`)
    /// is called again.
    /// Don't rely on this constructor to fully isolate two simulators
    /// running in the same process.
    pub fn new_seeded(
        date: NaiveDateTime,
        continents: Vec<Continent>,
        global_competitions: GlobalCompetitions,
        seed: u64,
    ) -> Self {
        rng_engine::RandomEngine::set_seed(seed);
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
                        top_flight_reputation: c
                            .leagues
                            .leagues
                            .iter()
                            .map(|l| l.reputation)
                            .max()
                            .unwrap_or(0),
                        transfer_profile: c.transfer_profile.clone(),
                    },
                )
            })
            .collect();

        let mut data = SimulatorData {
            continents,
            date,
            indexes: None,
            dirty_player_index: false,
            free_agents: Vec::new(),
            free_agent_staff: Vec::new(),
            pending_manager_approaches: Vec::new(),
            watchlist: Vec::new(),
            global_competitions,
            country_info,
            market_map: Arc::new(MarketMap::default()),
            match_store: MatchStorage::new(),
            daily_world_player_pool: None,
            daily_global_free_agents: None,
            free_agent_flow: FreeAgentFlowCounters::default(),
        };

        let mut indexes = SimulatorDataIndexes::new();

        indexes.refresh(&data);

        data.indexes = Some(indexes);

        data.init_league_tables();
        data.seed_player_histories();
        data.seed_player_nationality_continents();
        data.rebuild_market_map();
        data.bootstrap_market_ledgers();

        data
    }

    /// Register country info for countries that may not have active leagues in the simulation.
    /// Called by the database generator to ensure nationality lookups always succeed.
    ///
    /// `transfer_profile` is the nationality's half of the corridor map —
    /// where its nationals go and where its diaspora lives. A leagueless
    /// country still has both, and both are read constantly (a Senegalese at
    /// a French club is a corridor whether or not Senegal runs a division in
    /// this save), so the profile travels with the registration rather than
    /// being filled in later.
    pub fn add_country_info(
        &mut self,
        id: u32,
        code: String,
        slug: String,
        name: String,
        continent_id: u32,
        reputation: u16,
        transfer_profile: CountryTransferProfile,
    ) {
        self.country_info.entry(id).or_insert(CountryInfo {
            id,
            code,
            slug,
            name,
            continent_id,
            reputation,
            // A country registered through this path has no leagues in
            // the save, so there is no top flight to go home to.
            top_flight_reputation: 0,
            transfer_profile,
        });
    }

    /// Remove a country from the nationality lookup map.
    pub fn remove_country_info(&mut self, id: u32) {
        self.country_info.remove(&id);
    }

    pub fn next_date(&mut self) {
        self.date += Duration::days(1);
    }
}
