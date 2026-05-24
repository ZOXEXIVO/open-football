//! Data-access abstraction for `LeagueResult::process` and its
//! per-match-event fan-out.
//!
//! Two callers need to drive the same per-match stat / history / morale
//! pipeline:
//!
//! * **Domestic league matches** (`LeagueResult::process_local`). Each
//!   country processes its own matches inside `Country::simulate` —
//!   parallel across countries via `countries.par_iter_mut()` — so the
//!   caller only has `&mut Country` and a deferred-ops queue. All
//!   ID-based lookups (team_id, player_id, …) resolve to entities in
//!   this country.
//! * **Continental cup matches** (`LeagueResult::process_cup_match`).
//!   Champions League and friends pit teams from different countries,
//!   so the caller still has `&mut SimulatorData` and lookups can hit
//!   any country.
//!
//! Rather than maintain two divergent copies of the 1.7k-line
//! `process_match_events`, we generic-ize it over a `LeagueProcessAccess`
//! trait. Both `SimulatorData` and a new `CountryProcessCtx<'a>`
//! implement the trait, so the same function body services both
//! callers — `SimulatorData` keeps the global index-backed accessors,
//! and `CountryProcessCtx` scans within its single country (cheap at
//! the country sizes we have) and routes the rare cross-country writes
//! into a `DeferredGlobalOps` queue that the simulator drains serially
//! after the parallel pass.

use crate::league::League;
use crate::shared::indexes::SimulatorDataIndexes;
use crate::simulator::CountryInfo;
use crate::simulator::SimulatorData;
use crate::{Club, Country, Player, Team};
use chrono::NaiveDateTime;
use std::collections::HashMap;

/// Read/mutate surface needed by `process_match_results` and friends.
///
/// The method set matches the `SimulatorData` accessors the existing
/// code already calls. `CountryProcessCtx` implements the same surface
/// but scoped to a single country, with cross-country sites pushed
/// onto a deferred-ops queue.
///
/// Beyond the bare accessors there are a few "global ops" — write
/// paths into truly process-wide state (free-agent staff pool, manager
/// appointment market). For `SimulatorData` these execute inline;
/// `CountryProcessCtx` captures them in `DeferredGlobalOps` for the
/// serial drain.
pub trait LeagueProcessAccess {
    fn date(&self) -> NaiveDateTime;
    fn indexes(&self) -> Option<&SimulatorDataIndexes>;
    fn country_info(&self) -> &HashMap<u32, CountryInfo>;
    fn country(&self, id: u32) -> Option<&Country>;
    fn country_mut(&mut self, id: u32) -> Option<&mut Country>;
    fn country_by_club(&self, club_id: u32) -> Option<&Country>;
    fn league(&self, id: u32) -> Option<&League>;
    fn league_mut(&mut self, id: u32) -> Option<&mut League>;
    fn club(&self, id: u32) -> Option<&Club>;
    fn club_mut(&mut self, id: u32) -> Option<&mut Club>;
    fn team(&self, id: u32) -> Option<&Team>;
    fn team_mut(&mut self, id: u32) -> Option<&mut Team>;
    fn player(&self, id: u32) -> Option<&Player>;
    fn player_mut(&mut self, id: u32) -> Option<&mut Player>;

    /// Admit a freshly sacked staff member to the global free-agent
    /// pool. SimulatorData applies inline; CountryProcessCtx defers.
    fn admit_free_agent_staff(&mut self, staff: crate::Staff);
    /// Queue a permanent manager appointment for a club. The actual
    /// `manager_market::execute_appointment` reads cross-club state
    /// and the global free-agent pool, so the parallel path can only
    /// record the intent; SimulatorData runs it inline.
    fn queue_manager_appointment(&mut self, club_id: u32);
    /// Pick a random player from the data view's scope. SimulatorData
    /// picks from every team in the world; CountryProcessCtx picks
    /// from the current country only. Used by staff relationship
    /// events whose "random teammate" semantics don't actually need
    /// to span continents.
    fn random_player_mut(&mut self) -> Option<&mut Player>;
}

impl LeagueProcessAccess for SimulatorData {
    fn date(&self) -> NaiveDateTime {
        self.date
    }
    fn indexes(&self) -> Option<&SimulatorDataIndexes> {
        self.indexes.as_ref()
    }
    fn country_info(&self) -> &HashMap<u32, CountryInfo> {
        &self.country_info
    }
    fn country(&self, id: u32) -> Option<&Country> {
        SimulatorData::country(self, id)
    }
    fn country_mut(&mut self, id: u32) -> Option<&mut Country> {
        SimulatorData::country_mut(self, id)
    }
    fn country_by_club(&self, club_id: u32) -> Option<&Country> {
        SimulatorData::country_by_club(self, club_id)
    }
    fn league(&self, id: u32) -> Option<&League> {
        SimulatorData::league(self, id)
    }
    fn league_mut(&mut self, id: u32) -> Option<&mut League> {
        SimulatorData::league_mut(self, id)
    }
    fn club(&self, id: u32) -> Option<&Club> {
        SimulatorData::club(self, id)
    }
    fn club_mut(&mut self, id: u32) -> Option<&mut Club> {
        SimulatorData::club_mut(self, id)
    }
    fn team(&self, id: u32) -> Option<&Team> {
        SimulatorData::team(self, id)
    }
    fn team_mut(&mut self, id: u32) -> Option<&mut Team> {
        SimulatorData::team_mut(self, id)
    }
    fn player(&self, id: u32) -> Option<&Player> {
        SimulatorData::player(self, id)
    }
    fn player_mut(&mut self, id: u32) -> Option<&mut Player> {
        SimulatorData::player_mut(self, id)
    }
    fn admit_free_agent_staff(&mut self, staff: crate::Staff) {
        crate::club::staff::free_pool::admit_to_pool(
            &mut self.free_agent_staff,
            staff,
            self.date.date(),
        );
    }
    fn queue_manager_appointment(&mut self, club_id: u32) {
        crate::club::board::manager_market::ManagerMarketTick::execute_appointment(
            self,
            club_id,
            self.date.date(),
        );
    }
    fn random_player_mut(&mut self) -> Option<&mut Player> {
        let player_count: usize = self
            .continents
            .iter()
            .flat_map(|c| &c.countries)
            .flat_map(|c| &c.clubs)
            .flat_map(|c| &c.teams.teams)
            .map(|t| t.players.players.len())
            .sum();
        if player_count == 0 {
            return None;
        }
        let target = (rand::random::<f32>() * player_count as f32) as usize;
        let mut current = 0;
        for continent in &mut self.continents {
            for country in &mut continent.countries {
                for club in &mut country.clubs {
                    for team in &mut club.teams.teams {
                        for player in &mut team.players.players {
                            if current == target {
                                return Some(player);
                            }
                            current += 1;
                        }
                    }
                }
            }
        }
        None
    }
}

/// Per-country positional index for O(1) accessor lookups during the
/// result-processing fan-out. Maps `player_id`/`team_id`/`club_id` to
/// the `(club_idx, team_idx, player_idx)` triple inside `Country::clubs`
/// so `LeagueProcessAccess::player(id)` etc. don't linear-scan
/// thousands of players per call.
///
/// Built once before `LeagueResult::process_local` / `ClubResult::process`
/// and dropped before any path that mutates rosters (transfer market,
/// retirements). The accessors validate the slot's `player.id` after
/// indexing so a stale entry still returns `None` rather than the wrong
/// player — keeps the index advisory.
pub struct CountryLookupIndex {
    pub players: HashMap<u32, (u16, u8, u16)>,
    pub teams: HashMap<u32, (u16, u8)>,
    pub clubs: HashMap<u32, u16>,
}

impl CountryLookupIndex {
    pub fn build(country: &Country) -> Self {
        let player_cap: usize = country
            .clubs
            .iter()
            .flat_map(|c| &c.teams.teams)
            .map(|t| t.players.players.len())
            .sum();
        let team_cap: usize = country.clubs.iter().map(|c| c.teams.teams.len()).sum();
        let mut players = HashMap::with_capacity(player_cap);
        let mut teams = HashMap::with_capacity(team_cap);
        let mut clubs = HashMap::with_capacity(country.clubs.len());
        for (ci, club) in country.clubs.iter().enumerate() {
            let ci16 = ci.min(u16::MAX as usize) as u16;
            clubs.insert(club.id, ci16);
            for (ti, team) in club.teams.teams.iter().enumerate() {
                let ti8 = ti.min(u8::MAX as usize) as u8;
                teams.insert(team.id, (ci16, ti8));
                for (pi, player) in team.players.players.iter().enumerate() {
                    let pi16 = pi.min(u16::MAX as usize) as u16;
                    players.insert(player.id, (ci16, ti8, pi16));
                }
            }
        }
        CountryLookupIndex {
            players,
            teams,
            clubs,
        }
    }
}

/// Country-local data-access wrapper for the parallel Phase-A pass.
///
/// Owns `&mut Country` (the one country whose results are being
/// processed) plus read-only references to global tables the engine
/// needs (`country_info`, `indexes`). Any mutation that would have
/// touched another country (foreign-loan parent club, etc.) gets
/// captured in `deferred` and applied serially after the parallel pass
/// joins — see `DeferredGlobalOps`.
pub struct CountryProcessCtx<'a> {
    pub country: &'a mut Country,
    pub date: NaiveDateTime,
    pub country_info_ref: &'a HashMap<u32, CountryInfo>,
    pub indexes_ref: Option<&'a SimulatorDataIndexes>,
    pub deferred: &'a mut DeferredGlobalOps,
    /// O(1) lookup index for this country's players/teams/clubs.
    /// `None` falls back to linear scans via `Country::player(id)` etc.
    pub lookup: Option<&'a CountryLookupIndex>,
}

impl<'a> LeagueProcessAccess for CountryProcessCtx<'a> {
    fn date(&self) -> NaiveDateTime {
        self.date
    }
    fn indexes(&self) -> Option<&SimulatorDataIndexes> {
        self.indexes_ref
    }
    fn country_info(&self) -> &HashMap<u32, CountryInfo> {
        self.country_info_ref
    }
    fn country(&self, id: u32) -> Option<&Country> {
        if id == self.country.id {
            Some(self.country)
        } else {
            // No read access to other countries in this ctx. Callers
            // that hit this path (rare — only the world-pool sponsor
            // reputation lookup) lose precision; everything else stays
            // correct because `id` is overwhelmingly self.
            None
        }
    }
    fn country_mut(&mut self, id: u32) -> Option<&mut Country> {
        if id == self.country.id {
            Some(self.country)
        } else {
            None
        }
    }
    fn country_by_club(&self, club_id: u32) -> Option<&Country> {
        if self.country.owns_club(club_id) {
            Some(self.country)
        } else {
            // No read access to OTHER countries in this ctx. Callers
            // here are doing "is this club's country different from
            // ours" checks (foreign-loan scouting memory in
            // `match_events.rs`). Answering `None` for foreign clubs
            // means those callers treat the player as if no foreign
            // loan applied — a small loss in scouting-memory fidelity
            // for cross-country loaned players. Acceptable for the
            // first-pass parallelisation.
            None
        }
    }
    fn league(&self, id: u32) -> Option<&League> {
        self.country.league(id)
    }
    fn league_mut(&mut self, id: u32) -> Option<&mut League> {
        self.country.league_mut(id)
    }
    fn club(&self, id: u32) -> Option<&Club> {
        if let Some(idx) = self.lookup {
            if let Some(&ci) = idx.clubs.get(&id) {
                return self.country.clubs.get(ci as usize).filter(|c| c.id == id);
            }
            return None;
        }
        self.country.club(id)
    }
    fn club_mut(&mut self, id: u32) -> Option<&mut Club> {
        if let Some(idx) = self.lookup {
            if let Some(&ci) = idx.clubs.get(&id) {
                return self
                    .country
                    .clubs
                    .get_mut(ci as usize)
                    .filter(|c| c.id == id);
            }
            return None;
        }
        self.country.club_mut(id)
    }
    fn team(&self, id: u32) -> Option<&Team> {
        if let Some(idx) = self.lookup {
            if let Some(&(ci, ti)) = idx.teams.get(&id) {
                return self
                    .country
                    .clubs
                    .get(ci as usize)
                    .and_then(|c| c.teams.teams.get(ti as usize))
                    .filter(|t| t.id == id);
            }
            return None;
        }
        self.country.team(id)
    }
    fn team_mut(&mut self, id: u32) -> Option<&mut Team> {
        if let Some(idx) = self.lookup {
            if let Some(&(ci, ti)) = idx.teams.get(&id) {
                return self
                    .country
                    .clubs
                    .get_mut(ci as usize)
                    .and_then(|c| c.teams.teams.get_mut(ti as usize))
                    .filter(|t| t.id == id);
            }
            return None;
        }
        self.country.team_mut(id)
    }
    fn player(&self, id: u32) -> Option<&Player> {
        if let Some(idx) = self.lookup {
            if let Some(&(ci, ti, pi)) = idx.players.get(&id) {
                return self
                    .country
                    .clubs
                    .get(ci as usize)
                    .and_then(|c| c.teams.teams.get(ti as usize))
                    .and_then(|t| t.players.players.get(pi as usize))
                    .filter(|p| p.id == id);
            }
            return None;
        }
        self.country.player(id)
    }
    fn player_mut(&mut self, id: u32) -> Option<&mut Player> {
        if let Some(idx) = self.lookup {
            if let Some(&(ci, ti, pi)) = idx.players.get(&id) {
                return self
                    .country
                    .clubs
                    .get_mut(ci as usize)
                    .and_then(|c| c.teams.teams.get_mut(ti as usize))
                    .and_then(|t| t.players.players.get_mut(pi as usize))
                    .filter(|p| p.id == id);
            }
            return None;
        }
        self.country.player_mut(id)
    }
    fn admit_free_agent_staff(&mut self, staff: crate::Staff) {
        self.deferred.free_agent_staff.push(staff);
    }
    fn queue_manager_appointment(&mut self, club_id: u32) {
        self.deferred.pending_appointments.push(club_id);
    }
    fn random_player_mut(&mut self) -> Option<&mut Player> {
        // Country-local fallback: pick from this country's roster.
        let player_count: usize = self
            .country
            .clubs
            .iter()
            .flat_map(|c| &c.teams.teams)
            .map(|t| t.players.players.len())
            .sum();
        if player_count == 0 {
            return None;
        }
        let target = (rand::random::<f32>() * player_count as f32) as usize;
        let mut current = 0;
        for club in &mut self.country.clubs {
            for team in &mut club.teams.teams {
                for player in &mut team.players.players {
                    if current == target {
                        return Some(player);
                    }
                    current += 1;
                }
            }
        }
        None
    }
}

/// Read-only world state needed by `process_local`. Built once in
/// Phase A's prelude (single-threaded) and handed to every worker by
/// shared reference — the fields are immutable for the duration of
/// the parallel pass, so multiple workers reading them concurrently
/// is safe.
#[derive(Clone, Copy)]
pub struct WorldSnapshot<'a> {
    pub date: NaiveDateTime,
    pub country_info: &'a HashMap<u32, CountryInfo>,
    pub indexes: Option<&'a SimulatorDataIndexes>,
    /// World-wide foreign-player pool — every country's transferable
    /// players, walked once before the parallel pass. Each country's
    /// `simulate_transfer_market_local` filters out own-country entries.
    pub world_pool: &'a [crate::transfers::pipeline::PlayerSummary],
    /// Snapshot of the global "Move on Free" pool. Read-only during
    /// Phase A; `apply_deferred_transfer_ops` mutates `data.free_agents`
    /// in Phase C.
    pub global_free_agents: &'a [crate::country::result::transfers::GlobalFreeAgentSummary],
}

/// Cross-country / global mutations that the parallel Phase-A pass
/// can't apply in place because they reach outside the worker's owned
/// `&mut Country`. The simulator drains this serially after Phase A
/// joins.
///
/// Fields are public so the serial drain in `simulator/mod.rs` can
/// fold them straight into `data`. Each variant is a precise record
/// of the original mutation — no need for a `Box<dyn FnOnce>` since
/// the inputs are small and Copy/Clone.
#[derive(Default)]
pub struct DeferredGlobalOps {
    /// Sacked staff that should be admitted to `data.free_agent_staff`
    /// after the parallel pass. Produced by board sacking logic.
    pub free_agent_staff: Vec<crate::Staff>,
    /// Club ids that need `manager_market::execute_appointment` to
    /// finalise a permanent hire from the global free-agent staff
    /// pool. Produced when the board's search window has elapsed.
    pub pending_appointments: Vec<u32>,
    /// Academy prospects who aged out at the U18 graduation tick.
    /// `contract = None`, `Frt` status already stamped — Phase C
    /// extends them onto `data.free_agents` so they remain
    /// discoverable through the senior market.
    pub free_agent_players: Vec<crate::Player>,
}

impl DeferredGlobalOps {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn is_empty(&self) -> bool {
        self.free_agent_staff.is_empty()
            && self.pending_appointments.is_empty()
            && self.free_agent_players.is_empty()
    }
}
