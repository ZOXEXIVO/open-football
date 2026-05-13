use super::country_info::CountryInfo;
use super::result::WorldWorkloadCounts;
use super::seeding::{
    ClubSeedingContext, build_league_lookup, club_has_players_needing_seed,
    team_has_players_needing_seed, team_ids_for_league,
};
use crate::PlayerSquadStatus;
use crate::club::board::manager_market::ManagerApproach;
use crate::club::player::calculators::WageCalculator;
use crate::competitions::GlobalCompetitions;
use crate::continent::Continent;
use crate::country::result::transfers::GlobalFreeAgentSummary;
use crate::country::result::transfers::free_agent_market_calc::FreeAgentMarketCalculator;
use crate::league::{LeagueTable, MatchStorage};
use crate::shared::SimulatorDataIndexes;
use crate::transfers::TransferPool;
use crate::transfers::pipeline::PlayerSummary;
use crate::utils::IntegerUtils;
use crate::utils::random::engine as rng_engine;
use crate::{Person, Player, Staff};
use chrono::{Duration, NaiveDate, NaiveDateTime};
use rayon::prelude::*;
use std::collections::HashMap;

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
    pub pending_manager_approaches: Vec<ManagerApproach>,

    pub watchlist: Vec<u32>,

    pub global_competitions: GlobalCompetitions,

    /// All countries by id (for nationality lookups — includes countries without active leagues)
    pub country_info: HashMap<u32, CountryInfo>,

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
            daily_world_player_pool: None,
            daily_global_free_agents: None,
        };

        let mut indexes = SimulatorDataIndexes::new();

        indexes.refresh(&data);

        data.indexes = Some(indexes);

        data.init_league_tables();
        data.seed_player_histories();
        data.seed_player_nationality_continents();

        data
    }

    /// Populate `Player.nationality_continent_id` from `country_info` for
    /// every player on every roster + retired + national-team + free-agent
    /// pool. Called once at construction time after `country_info` is
    /// populated. Cheap parallel pass.
    pub fn seed_player_nationality_continents(&mut self) {
        let lookup: std::collections::HashMap<u32, u32> = self
            .country_info
            .iter()
            .map(|(k, v)| (*k, v.continent_id))
            .collect();
        if lookup.is_empty() {
            return;
        }
        self.continents
            .par_iter_mut()
            .flat_map(|continent| continent.countries.par_iter_mut())
            .for_each(|country| {
                for club in &mut country.clubs {
                    for team in club.teams.iter_mut() {
                        for player in &mut team.players.players {
                            if player.nationality_continent_id == 0 {
                                if let Some(cid) = lookup.get(&player.country_id) {
                                    player.nationality_continent_id = *cid;
                                }
                            }
                        }
                    }
                }
                for player in &mut country.retired_players {
                    if player.nationality_continent_id == 0 {
                        if let Some(cid) = lookup.get(&player.country_id) {
                            player.nationality_continent_id = *cid;
                        }
                    }
                }
                for player in &mut country.national_team.generated_squad {
                    if player.nationality_continent_id == 0 {
                        if let Some(cid) = lookup.get(&player.country_id) {
                            player.nationality_continent_id = *cid;
                        }
                    }
                }
            });
        for player in &mut self.free_agents {
            if player.nationality_continent_id == 0 {
                if let Some(cid) = lookup.get(&player.country_id) {
                    player.nationality_continent_id = *cid;
                }
            }
        }
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
        self.continents
            .par_iter_mut()
            .flat_map(|continent| continent.countries.par_iter_mut())
            .for_each(|country| {
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
            });
    }

    /// Seed statistics history for every player. Called once at
    /// construction time — touches every player unconditionally.
    fn seed_player_histories(&mut self) {
        let date = self.date.date();
        self.continents
            .par_iter_mut()
            .flat_map(|continent| continent.countries.par_iter_mut())
            .for_each(|country| {
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
            });
    }

    /// Seed any players whose history is still empty — catches youth intake,
    /// regens, and newly-generated clubs within one simulated tick.
    /// Skip-fast at club AND team level so the steady-state cost is close
    /// to zero when nothing needs seeding.
    pub fn seed_missing_player_histories(&mut self) {
        let date = self.date.date();
        self.continents
            .par_iter_mut()
            .flat_map(|continent| continent.countries.par_iter_mut())
            .for_each(|country| {
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
            });
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
        use crate::club::player::transfer::ReleaseContext;
        use crate::shared::{Currency, CurrencyValue};
        use crate::transfers::{CompletedTransfer, TransferType};

        let date = self.date.date();
        let released: Vec<Player> = self
            .continents
            .par_iter_mut()
            .flat_map(|continent| continent.countries.par_iter_mut())
            .flat_map_iter(|country| {
                // League reputation is needed by `on_release` so the
                // player carries an accurate market-state snapshot into
                // the free-agent pool. Pre-collect once per country —
                // immutable read before the mutable club iteration
                // takes the borrow.
                let league_reputations: std::collections::HashMap<u32, u16> = country
                    .leagues
                    .leagues
                    .iter()
                    .map(|l| (l.id, l.reputation))
                    .collect();
                let country_id = country.id;
                let country_reputation = country.reputation;
                let mut released_in_country: Vec<Player> = Vec::new();
                let mut new_history: Vec<CompletedTransfer> = Vec::new();
                for club in &mut country.clubs {
                    let club_id = club.id;
                    for team in &mut club.teams.teams {
                        let team_id = team.id;
                        let team_name = team.name.clone();
                        let team_reputation_world = team.reputation.world;
                        let team_league_reputation = team
                            .league_id
                            .and_then(|lid| league_reputations.get(&lid).copied())
                            .unwrap_or(country_reputation);
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
                            if let Some(mut p) = team.players.take_player(&id) {
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
                                // Stamp the player's market-state
                                // snapshot at the moment they enter the
                                // pool. `last_salary` is unrecoverable
                                // here (the contract was already cleared
                                // upstream), so seed from the wage
                                // calculator using the team / league
                                // tiers as a faithful replacement.
                                let last_squad_status = PlayerSquadStatus::FirstTeamSquadRotation;
                                let club_score =
                                    (team_reputation_world as f32 / 10_000.0).clamp(0.0, 1.0);
                                let last_salary = WageCalculator::expected_annual_wage(
                                    &p,
                                    p.age(date),
                                    club_score,
                                    team_league_reputation,
                                );
                                if p.free_agent_state().is_none() {
                                    p.enter_free_agent_market(ReleaseContext {
                                        date,
                                        last_club_id: Some(club_id),
                                        last_country_id: Some(country_id),
                                        last_country_reputation: country_reputation,
                                        last_league_reputation: team_league_reputation,
                                        last_club_reputation_score: club_score,
                                        last_salary,
                                        last_squad_status,
                                    });
                                }
                                released_in_country.push(p);
                            }
                        }
                    }
                }
                country.transfer_market.transfer_history.extend(new_history);
                released_in_country
            })
            .collect();
        if !released.is_empty() {
            self.dirty_player_index = true;
            self.free_agents.extend(released);
        }
    }

    /// Monthly retirement pass over the global free-agent pool. Anyone
    /// 12+ months without a club rolls retirement at a probability that
    /// climbs with age, low quality, and time spent unemployed; high
    /// world-rep players resist longer (they're still names, clubs come
    /// looking).
    ///
    /// Gated by the caller on `today.day() == 1`. The internal gate on
    /// `free_since` ≥ 12 months means a fresh database free agent
    /// (seeded `free_since = today - 30d`) is automatically skipped.
    pub fn process_free_agent_retirements(&mut self, date: NaiveDate) {
        use crate::PlayerStatusType;

        let mut to_retire: Vec<usize> = Vec::new();
        for (idx, player) in self.free_agents.iter().enumerate() {
            let Some(state) = player.free_agent_state() else {
                continue;
            };
            let days_free = (date - state.free_since).num_days();
            if days_free < 365 {
                continue;
            }
            let months_after_12 = ((days_free - 365) / 30).max(0) as u32;
            let prob = FreeAgentMarketCalculator::retirement_probability_per_month(
                months_after_12,
                player.age(date),
                player.player_attributes.current_ability,
                player.player_attributes.world_reputation,
            );
            if prob <= 0.0 {
                continue;
            }
            let roll = IntegerUtils::random(1, 1000) as f32 / 1000.0;
            if roll < prob {
                to_retire.push(idx);
            }
        }

        // Reverse order so swap_remove against earlier indexes doesn't
        // disturb later ones.
        to_retire.sort_unstable_by(|a, b| b.cmp(a));
        for idx in to_retire {
            let mut player = self.free_agents.swap_remove(idx);
            player.statuses.add(date, PlayerStatusType::Ret);
            player.contract = None;
            player.retired = true;
            let country_id = player.country_id;
            if let Some(country) = self.country_mut(country_id) {
                country.retired_players.push(player);
            }
            // Else: nationality country isn't loaded — drop silently.
            // The player is gone from the pool either way.
        }
    }

    pub fn next_date(&mut self) {
        self.date += Duration::days(1);
    }

    /// Walk the world once to count countries, leagues, clubs and
    /// players. Used by the perf dashboard at end-of-tick — single
    /// linear pass, no allocation.
    pub fn workload_counts(&self) -> WorldWorkloadCounts {
        let mut counts = WorldWorkloadCounts {
            countries: 0,
            leagues: 0,
            clubs: 0,
            players: 0,
        };
        for continent in &self.continents {
            for country in &continent.countries {
                counts.countries += 1;
                counts.leagues += country.leagues.leagues.len() as u64;
                counts.clubs += country.clubs.len() as u64;
                for club in &country.clubs {
                    for team in &club.teams.teams {
                        counts.players += team.players.players.len() as u64;
                    }
                }
            }
        }
        counts
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

        // Pre-distribute candidates per country so each rayon worker owns
        // its own slice — no shared HashMap, no lock. The serial drain
        // here is O(countries) and trivial next to the parallel
        // `call_up_squad` body.
        let work_items: Vec<_> = self
            .continents
            .iter_mut()
            .flat_map(|c| c.countries.iter_mut())
            .map(|country| {
                let candidates = candidates_by_country
                    .remove(&country.id)
                    .unwrap_or_default();
                (country, candidates)
            })
            .collect();

        work_items
            .into_par_iter()
            .for_each(|(country, candidates)| {
                country.national_team.country_name = country.name.clone();
                country.national_team.reputation = country.reputation;
                let cid = country.id;
                country
                    .national_team
                    .call_up_squad(candidates, date, cid, &country_ids);
            });

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
