use super::country_info::CountryInfo;
use super::result::WorldWorkloadCounts;
use super::seeding::{
    ClubSeedingContext, build_league_lookup, club_has_players_needing_seed,
    team_has_players_needing_seed, team_ids_for_league,
};
use crate::NationalSelectionPolicy;
use crate::NationalTeam;
use crate::PlayerSquadStatus;
use crate::club::board::manager_market::ManagerApproach;
use crate::club::player::calculators::WageCalculator;
use crate::club::staff::perception::PotentialEstimator;
use crate::competitions::GlobalCompetitions;
use crate::continent::Continent;
use crate::country::result::transfers::GlobalFreeAgentSummary;
use crate::country::result::transfers::free_agent_market_calc::FreeAgentMarketCalculator;
use crate::league::{LeagueTable, MatchStorage};
use crate::shared::SimulatorDataIndexes;
use crate::transfers::TransferPool;
use crate::transfers::pipeline::{PipelineProcessor, PlayerSummary};
use crate::utils::IntegerUtils;
use crate::utils::random::engine as rng_engine;
use crate::{Person, Player, Staff};
use chrono::{Duration, NaiveDate, NaiveDateTime};
use rayon::prelude::*;
use std::collections::HashMap;
use std::collections::HashSet;

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
        let lookup: HashMap<u32, u32> = self
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
                for player in country
                    .national_team
                    .generated_squad
                    .iter_mut()
                    .chain(country.u21_national_team.generated_squad.iter_mut())
                {
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
                for player in country
                    .national_team
                    .generated_squad
                    .iter()
                    .chain(country.u21_national_team.generated_squad.iter())
                {
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
    /// Non-senior squads (Reserve, U18..U23) seed under the main-team
    /// alias from `team_info_for` so a player who never leaves the
    /// youth setup still has a "career home" row pointing at the
    /// parent club's main team.
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
    /// Once the reason is read, the player's club-transient state is
    /// reset (`reset_on_club_change`): the upstream release pipelines
    /// only clear the contract, so without this the player would sit in
    /// the pool still flagged "Listed / Loan Listed / Wants Free
    /// Transfer / Unhappy" about a club he no longer belongs to.
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
                let league_reputations: HashMap<u32, u16> = country
                    .leagues
                    .leagues
                    .iter()
                    .map(|l| (l.id, l.reputation))
                    .collect();
                let country_id = country.id;
                let country_reputation = country.reputation;
                // Per-club main-team identity resolver — the same one the
                // history seeder uses, so a released player's spell is
                // marked departed under the exact slug it was seeded with
                // (youth / Reserve squads alias to the parent Main team).
                let league_lookup = build_league_lookup(country);
                let mut released_in_country: Vec<Player> = Vec::new();
                let mut new_history: Vec<CompletedTransfer> = Vec::new();
                for club in &mut country.clubs {
                    let club_id = club.id;
                    let club_ctx = ClubSeedingContext::resolve(club, &league_lookup);
                    for team in &mut club.teams.teams {
                        let release_team_info = club_ctx.team_info_for(team);
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
                                // A complete release fires BOTH halves: the
                                // stats-history side (snapshot in-flight match
                                // stats onto the source-club spell and mark it
                                // departed) and the market-state side below.
                                // Skipping `on_release` leaves the current-club
                                // entry `departed_date: None`, so a later
                                // same-season signing becomes a second "active"
                                // spell the History projection hides as a
                                // phantom — and the un-drained cup / friendly
                                // buckets trip `on_free_agent_signing`.
                                p.on_release(&release_team_info, date);
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
                                // The spell at the old club is over —
                                // strip transfer statuses and the
                                // unhappiness they were attached to,
                                // exactly like a completed transfer
                                // would. `released_early` consumed the
                                // `Frt` marker above, so nothing here
                                // still needs it.
                                p.reset_on_club_change();
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
            // Scrub the world of stale market state for the departed
            // players: open listings end Cancelled, team transfer lists
            // drop their rows, scouting/shortlist/monitoring interest is
            // cleared and live negotiations rejected — the same
            // chokepoint a completed transfer runs through, in its
            // release flavour. Without this a released player kept an
            // Available country-market listing pointing at a club he no
            // longer plays for.
            let released_ids: Vec<u32> = released.iter().map(|p| p.id).collect();
            PipelineProcessor::cleanup_player_release_interest_batch(self, &released_ids);
            self.free_agents.extend(released);
        }
    }

    /// Monthly retirement pass over the global free-agent pool. Anyone
    /// 12+ months without a club rolls retirement at a probability that
    /// climbs with age, low quality, and time spent unemployed; high
    /// world-rep players resist longer (they're still names, clubs come
    /// looking).
    ///
    /// On top of the probabilistic roll there is a deterministic hard
    /// bound (`deterministic_retirement_months`, 24–48 months by age /
    /// CA / observable ceiling / world rep): a player whose monthly
    /// rolls keep missing still resolves instead of haunting the pool
    /// for half a decade. Young players with visible growth room get
    /// the longest leash; old low-quality journeymen the shortest.
    ///
    /// Gated by the caller on `today.day() == 1`. The internal gate on
    /// `free_since` ≥ 12 months means a fresh database free agent
    /// (seeded `free_since = today - 30d`) is automatically skipped.
    pub fn process_free_agent_retirements(&mut self, date: NaiveDate) {
        use crate::RetirementReason;

        // Decision phase is per-player independent — run the prob roll
        // in parallel. Mutation (the `RetirementConsidering` emit plus
        // swap_remove + push into retired_players) stays serial below
        // because it requires `&mut self`. Each outcome carries
        // `(index, will_retire, months_without_club)` so the serial pass
        // can both surface late-career considering moods and retire the
        // ones whose roll came up.
        let outcomes: Vec<(usize, bool, u16)> = self
            .free_agents
            .par_iter()
            .enumerate()
            .filter_map(|(idx, player)| {
                let state = player.free_agent_state()?;
                let days_free = (date - state.free_since).num_days();
                if days_free < 365 {
                    return None;
                }
                let months_without_club = (days_free / 30).max(0) as u16;
                let age = player.age(date);
                let ca = player.player_attributes.current_ability;
                let world_rep = player.player_attributes.world_reputation;
                // Observable ceiling, not hidden biological PA — the
                // retirement judgement reads the same market-visible
                // promise every club decision does.
                let ceiling = PotentialEstimator::observable_ceiling(player, date);
                let hard_bound = FreeAgentMarketCalculator::deterministic_retirement_months(
                    age, ca, ceiling, world_rep,
                );
                if (months_without_club as u32) >= hard_bound {
                    return Some((idx, true, months_without_club));
                }
                let months_after_12 = ((days_free - 365) / 30).max(0) as u32;
                let prob = FreeAgentMarketCalculator::retirement_probability_per_month(
                    months_after_12,
                    age,
                    ca,
                    world_rep,
                );
                if prob <= 0.0 {
                    return None;
                }
                let roll = IntegerUtils::random(1, 1000) as f32 / 1000.0;
                Some((idx, roll < prob, months_without_club))
            })
            .collect();

        // Considering pass first — it only mutates happiness, never the
        // pool's structure, so indices remain valid. Players who are
        // about to retire this tick are skipped (they get the
        // announcement, not the lead-up).
        for &(idx, will_retire, months) in &outcomes {
            if will_retire {
                continue;
            }
            if let Some(player) = self.free_agents.get_mut(idx) {
                player.consider_retirement_as_free_agent(date, months);
            }
        }

        // Retirement pass. Reverse order so swap_remove against earlier
        // indexes doesn't disturb later ones.
        let mut to_retire: Vec<usize> = outcomes
            .iter()
            .filter(|(_, will_retire, _)| *will_retire)
            .map(|(idx, _, _)| *idx)
            .collect();
        to_retire.sort_unstable_by(|a, b| b.cmp(a));
        for idx in to_retire {
            let mut player = self.free_agents.swap_remove(idx);
            // A still-renowned name bows out with a planned farewell;
            // everyone else is retiring because the offers dried up.
            let reason = if player.player_attributes.world_reputation >= 7000 {
                RetirementReason::PlannedFarewell
            } else {
                RetirementReason::LongFreeAgency
            };
            player.announce_retirement(date, reason);
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
        let need_callups =
            NationalTeam::is_break_start(date) || NationalTeam::is_tournament_start(date);
        if !need_callups {
            return;
        }

        // Country IDs across the whole world — used to draw friendly
        // opponents from any nation, not just same-continent.
        let country_ids: Vec<(u32, String)> = self
            .continents
            .iter()
            .flat_map(|c| c.countries.iter())
            .map(|c| (c.id, c.name.clone()))
            .collect();

        // --- Senior selection -------------------------------------------------
        // Build a global senior candidate pool (main teams only) and run
        // the per-country call-up in parallel. Pre-distribute candidates
        // so each rayon worker owns its own slice — no shared HashMap.
        let mut candidates_by_country = NationalTeam::collect_all_candidates_by_country(
            self.continents.iter().flat_map(|c| c.countries.iter()),
            date,
        );
        let senior_work: Vec<_> = self
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
        senior_work
            .into_par_iter()
            .for_each(|(country, candidates)| {
                country.national_team.country_name = country.name.clone();
                country.national_team.reputation = country.reputation;
                let cid = country.id;
                country
                    .national_team
                    .call_up_squad(candidates, date, cid, &country_ids);
            });

        // --- U21 selection ----------------------------------------------------
        // Collect every player already taken by a senior squad in this
        // window — they're excluded from the U21 pool so the youth side
        // is a genuinely separate set of players, not a senior shadow.
        let senior_selected: HashSet<u32> = self
            .continents
            .iter()
            .flat_map(|c| c.countries.iter())
            .flat_map(|c| c.national_team.squad.iter().map(|sp| sp.player_id))
            .collect();

        let u21_policy = NationalSelectionPolicy::under21();
        let mut u21_candidates_by_country =
            NationalTeam::collect_all_candidates_by_country_with_policy(
                self.continents.iter().flat_map(|c| c.countries.iter()),
                date,
                &u21_policy,
            );
        for candidates in u21_candidates_by_country.values_mut() {
            candidates.retain(|c| !senior_selected.contains(&c.player_id));
        }
        let u21_work: Vec<_> = self
            .continents
            .iter_mut()
            .flat_map(|c| c.countries.iter_mut())
            .map(|country| {
                let candidates = u21_candidates_by_country
                    .remove(&country.id)
                    .unwrap_or_default();
                (country, candidates)
            })
            .collect();
        u21_work.into_par_iter().for_each(|(country, candidates)| {
            country.u21_national_team.country_name = country.name.clone();
            country.u21_national_team.reputation = country.reputation;
            let cid = country.id;
            country.u21_national_team.call_up_squad_with_policy(
                candidates,
                date,
                cid,
                &country_ids,
                &u21_policy,
            );
        });

        // Apply Int / IntU21 statuses across every club in every continent.
        // Senior first, then U21 — the U21 pass only toggles IntU21, so
        // the two never clash on the same player (the pools are disjoint).
        NationalTeam::apply_callup_statuses_across_world(&mut self.continents, date);
        NationalTeam::apply_u21_callup_statuses_across_world(&mut self.continents, date);
    }

    /// World-level Int release. Runs after all matches (continent
    /// matches + global tournament matches) so a tournament final
    /// landing on a release date is played with squad statuses still
    /// attached. Squad data itself is preserved for the squad UI; only
    /// the per-player Int flag is cleared.
    pub fn process_world_national_team_release(&mut self) {
        let date = self.date.date();
        let need_release =
            NationalTeam::is_break_end(date) || NationalTeam::is_tournament_end(date);
        if !need_release {
            return;
        }
        NationalTeam::release_callup_statuses_across_world(&mut self.continents);
        NationalTeam::release_u21_callup_statuses_across_world(&mut self.continents);
    }
}

#[cfg(test)]
mod free_agent_retirement_tests {
    //! Deterministic coverage for the free-agent retirement pass: the
    //! hard upper bound must resolve long sits without RNG, and young
    //! players with rep / growth room must not be swept prematurely.

    use super::*;
    use crate::club::player::builder::PlayerBuilder;
    use crate::club::player::transfer::ReleaseContext;
    use crate::competitions::global::GlobalCompetitions;
    use crate::league::LeagueCollection;
    use crate::shared::fullname::FullName;
    use crate::{
        Country, PersonAttributes, PlayerAttributes, PlayerPosition, PlayerPositionType,
        PlayerPositions, PlayerSkills,
    };

    struct RetirementFixtures;

    impl RetirementFixtures {
        fn d(y: i32, m: u32, day: u32) -> NaiveDate {
            NaiveDate::from_ymd_opt(y, m, day).unwrap()
        }

        fn pool_player(
            id: u32,
            age_years: i64,
            ca: u8,
            world_reputation: i16,
            free_since: NaiveDate,
            today: NaiveDate,
        ) -> Player {
            let mut attrs = PlayerAttributes::default();
            attrs.current_ability = ca;
            attrs.potential_ability = ca;
            attrs.world_reputation = world_reputation;
            let birth = today - Duration::days(age_years * 365 + 30);
            let mut player = PlayerBuilder::new()
                .id(id)
                .full_name(FullName::new("Pool".to_string(), format!("P{id}")))
                .birth_date(birth)
                .country_id(1)
                .attributes(PersonAttributes::default())
                .skills(PlayerSkills::default())
                .positions(PlayerPositions {
                    positions: vec![PlayerPosition {
                        position: PlayerPositionType::MidfielderCenter,
                        level: 16,
                    }],
                })
                .player_attributes(attrs)
                .build()
                .unwrap();
            player.enter_free_agent_market(ReleaseContext {
                date: free_since,
                last_club_id: Some(10),
                last_country_id: Some(1),
                last_country_reputation: 3000,
                last_league_reputation: 2500,
                last_club_reputation_score: 0.3,
                last_salary: 40_000,
                last_squad_status: PlayerSquadStatus::FirstTeamSquadRotation,
            });
            player
        }

        fn simulator(today: NaiveDate, free_agents: Vec<Player>) -> SimulatorData {
            let country = Country::builder()
                .id(1)
                .code("en".to_string())
                .slug("england".to_string())
                .name("England".to_string())
                .continent_id(1)
                .reputation(5000)
                .leagues(LeagueCollection::new(Vec::new()))
                .clubs(Vec::new())
                .build()
                .unwrap();
            let continent = Continent::new(1, "Europe".to_string(), vec![country], Vec::new());
            let mut data = SimulatorData::new(
                today.and_hms_opt(12, 0, 0).unwrap(),
                vec![continent],
                GlobalCompetitions::new(Vec::new()),
            );
            data.free_agents = free_agents;
            data
        }
    }

    #[test]
    fn old_low_quality_free_agent_retires_deterministically_after_hard_bound() {
        // 36yo CA-50 journeyman, 40 months without a club: well past
        // the 24-month deterministic bound for the old/low-CA cohort.
        // No RNG involved -- the hard bound fires before any roll.
        let today = RetirementFixtures::d(2026, 6, 1);
        let free_since = today - Duration::days(40 * 30);
        let player = RetirementFixtures::pool_player(900, 36, 50, 0, free_since, today);
        let mut data = RetirementFixtures::simulator(today, vec![player]);

        data.process_free_agent_retirements(today);

        assert!(
            data.free_agents.is_empty(),
            "hard-bound retirement must remove the player from the pool"
        );
        let country = data.country(1).unwrap();
        assert!(
            country.retired_players.iter().any(|p| p.id == 900),
            "retired player must land in his nationality country"
        );
        assert!(country.retired_players[0].retired);
    }

    #[test]
    fn young_free_agent_with_standing_is_not_prematurely_retired() {
        // 21yo, 13 months free, decent world rep: the probabilistic
        // curve nets out to zero (rep offsets the small time base) and
        // every deterministic bound is years away -- the player must
        // still be in the pool. Fully deterministic: prob <= 0 means
        // no roll happens at all.
        let today = RetirementFixtures::d(2026, 6, 1);
        let free_since = today - Duration::days(395);
        let player = RetirementFixtures::pool_player(901, 21, 70, 5000, free_since, today);
        let mut data = RetirementFixtures::simulator(today, vec![player]);

        data.process_free_agent_retirements(today);

        assert_eq!(
            data.free_agents.len(),
            1,
            "young free agent must not be swept at 13 months"
        );
        assert!(!data.free_agents[0].retired);
    }
}
