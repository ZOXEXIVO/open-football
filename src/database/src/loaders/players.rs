//! Player index built from the compiled database.
//!
//! Each [`OdbPlayer`] record corresponds to one JSON file under
//! `data/{cc}/{league}/{club}/players/` in the external data repo;
//! the compiler bundles them into `database.db` and the runtime reads
//! them back through this loader.

use chrono::NaiveDate;
use log::info;
use serde::Deserialize;
use std::collections::HashMap;

use super::compiled::compiled;

/// A single player record. Fields with `#[serde(default)]` are optional —
/// the hydrator fills sensible defaults so a minimal scraper output still
/// produces a complete `Player`.
#[derive(Debug, Clone, Deserialize)]
pub struct OdbPlayer {
    pub id: u32,
    pub first_name: String,
    pub last_name: String,
    #[serde(default)]
    pub middle_name: Option<String>,
    #[serde(default)]
    pub nickname: Option<String>,

    pub birth_date: NaiveDate,
    pub country_id: u32,

    /// The club that owns the player's primary contract (parent club).
    /// If the player is on loan, they will still appear in this club's
    /// transfer/contract records even though they physically play for `loan.to_club_id`.
    ///
    /// Defaults to 0 for free agents — players sourced from
    /// `data/{cc}/free_agents/*.json` who belong to no club at all. The
    /// hydrator routes these into `SimulatorData.free_agents` instead of
    /// any club squad.
    #[serde(default)]
    pub club_id: u32,

    /// One or more positions with skill levels (1-20).
    pub positions: Vec<OdbPosition>,

    #[serde(default)]
    pub preferred_foot: Option<String>,
    #[serde(default)]
    pub height: Option<u8>,
    #[serde(default)]
    pub weight: Option<u8>,

    pub current_ability: u8,
    pub potential_ability: u8,

    /// Market value in whole currency units (USD). When omitted the value
    /// calculator derives one from CA/age/reputation.
    #[serde(default)]
    pub value: Option<u32>,

    #[serde(default)]
    pub reputation: Option<OdbReputation>,

    /// Active contract terms. Optional so free-agent records (no club, no
    /// running deal) can omit it; clubbed players always populate it.
    #[serde(default)]
    pub contract: Option<OdbContract>,

    /// Present when the player is currently on loan to another club.
    /// `loan.to_club_id` becomes the player's CURRENT club for squad placement;
    /// `club_id` remains the parent.
    #[serde(default)]
    pub loan: Option<OdbLoan>,

    /// Prior-season career history. Each entry is one completed season at
    /// one club. Populates `Player.statistics_history.items` at hydration
    /// time so the `/players/:id/history` page shows real career data
    /// instead of starting empty.
    #[serde(default)]
    pub history: Vec<OdbHistoryItem>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct OdbPosition {
    /// Short code: GK, SW, DL, DCL, DC, DCR, DR, DM, ML, MCL, MC, MCR, MR,
    /// AML, AMC, AMR, WBL, WBR, ST, FL, FC, FR.
    pub code: String,
    pub level: u8,
}

/// Per-field reputation override. Every field is optional — a record may
/// supply all three, just one (e.g. a scraper that only captured world
/// fame), or none. Missing fields are derived from current ability via
/// the ability-curve fallback in `build_player_attributes`.
#[derive(Debug, Clone, Deserialize)]
pub struct OdbReputation {
    #[serde(default)]
    pub home: Option<i16>,
    #[serde(default)]
    pub world: Option<i16>,
    #[serde(default)]
    pub current: Option<i16>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct OdbContract {
    /// Annual salary, whole currency units.
    pub salary: u32,
    pub expiration: NaiveDate,
    #[serde(default)]
    pub started: Option<NaiveDate>,
    /// "FullTime" (default), "PartTime", "Youth", "Amateur", "NonContract".
    #[serde(default)]
    pub contract_type: Option<String>,
    #[serde(default)]
    pub shirt_number: Option<u8>,
    #[serde(default)]
    pub squad_status: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct OdbLoan {
    /// Borrowing club — the player physically plays here.
    pub to_club_id: u32,
    #[serde(default)]
    pub to_team_id: Option<u32>,
    pub expiration: NaiveDate,
    /// Loan-period salary (paid by borrower, possibly subsidised by parent).
    pub salary: u32,
    #[serde(default)]
    pub match_fee: Option<u32>,
    #[serde(default)]
    pub wage_contribution_pct: Option<u8>,
    #[serde(default)]
    pub future_fee: Option<u32>,
    #[serde(default)]
    pub future_fee_obligation: bool,
    #[serde(default)]
    pub min_appearances: Option<u16>,
}

/// One prior-season career history entry. Club identity (name/slug/league/country)
/// is resolved from the live database at hydration time via `club_id`, so
/// records stay correct even as club names/leagues change between releases.
///
/// Stat fields are minimal by design — scrapers rarely capture assists,
/// ratings, conceded goals or clean sheets per historical season, so every
/// stat is optional and defaults to 0. For goalkeepers, populate only
/// `played`; for outfield players, populate `played` and `goals`.
///
/// Keys are single-letter on the wire: history arrays repeat the same keys
/// once per season per player across ~50k players, so short names cut the
/// uncompressed JSON size meaningfully (gzip still benefits downstream).
#[derive(Debug, Clone, Deserialize)]
pub struct OdbHistoryItem {
    /// Season start year (e.g. 2017 for the 2017/18 season).
    #[serde(rename = "s")]
    pub season: u16,
    /// Club where the player played that season. Resolved to team/league/country
    /// using the loaded DB. Unknown ids produce an entry with empty club links.
    #[serde(rename = "c")]
    pub club_id: u32,
    #[serde(default, rename = "l")]
    pub is_loan: bool,

    #[serde(default, rename = "p")]
    pub played: u16,
    #[serde(default, rename = "g")]
    pub goals: u16,
    /// Average match rating 0.0–10.0. Zero renders as "-".
    #[serde(default, rename = "r")]
    pub rating: f32,
}

/// In-memory index of players, grouped by the club where they physically
/// play. Loaned-out players are indexed under the borrower so their squad
/// lists them in training / matches. Loan metadata rides on the player's
/// `contract_loan`, which the web layer reads to label the loan direction.
///
/// Players whose `club_id` is 0 (sourced from `data/{cc}/free_agents/`) live
/// in a separate `free_agents` bucket — they are not part of any club's
/// squad and will be dropped into `SimulatorData.free_agents` by the
/// generator instead.
pub struct PlayersOdb {
    by_physical_club: HashMap<u32, Vec<OdbPlayer>>,
    free_agents: Vec<OdbPlayer>,
}

impl PlayersOdb {
    /// Build the index from the embedded compiled database. Returns `None`
    /// only when the compiled doc has no players at all (fresh repo, no
    /// imports yet).
    pub fn load() -> Option<Self> {
        let source = &compiled().players;
        if source.is_empty() {
            return None;
        }
        let odb = Self::from_players(source.iter().cloned().collect());
        let total: usize = odb.by_physical_club.values().map(|v| v.len()).sum();
        info!(
            "players loaded from compiled DB: {} clubbed players across {} clubs, {} free agents",
            total,
            odb.by_physical_club.len(),
            odb.free_agents.len(),
        );
        Some(odb)
    }

    /// Index an in-memory list of players — useful for tests and ad-hoc tools.
    pub fn from_players(players: Vec<OdbPlayer>) -> Self {
        let mut by_physical_club: HashMap<u32, Vec<OdbPlayer>> = HashMap::new();
        let mut free_agents: Vec<OdbPlayer> = Vec::new();
        for p in players {
            // A free agent has no parent club and no loan placement.
            if p.club_id == 0 && p.loan.is_none() {
                free_agents.push(p);
                continue;
            }
            let physical_club = p.loan.as_ref().map(|l| l.to_club_id).unwrap_or(p.club_id);
            by_physical_club.entry(physical_club).or_default().push(p);
        }
        PlayersOdb { by_physical_club, free_agents }
    }

    pub fn for_club(&self, club_id: u32) -> Option<&[OdbPlayer]> {
        self.by_physical_club.get(&club_id).map(|v| v.as_slice())
    }

    pub fn has_club(&self, club_id: u32) -> bool {
        self.by_physical_club.contains_key(&club_id)
    }

    /// Players sourced from `data/{cc}/free_agents/` — clubless and ready
    /// to be hydrated into `SimulatorData.free_agents`.
    pub fn free_agents(&self) -> &[OdbPlayer] {
        &self.free_agents
    }

    /// Highest player id present in the index, or `None` when empty.
    /// Used to seed the procedural id sequence so generated players never
    /// collide with externally-supplied ids.
    pub fn max_player_id(&self) -> Option<u32> {
        let club_max = self
            .by_physical_club
            .values()
            .flat_map(|v| v.iter().map(|p| p.id))
            .max();
        let fa_max = self.free_agents.iter().map(|p| p.id).max();
        match (club_max, fa_max) {
            (Some(a), Some(b)) => Some(a.max(b)),
            (a, b) => a.or(b),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_player(id: u32, club_id: u32, loan_to: Option<u32>) -> OdbPlayer {
        OdbPlayer {
            id,
            first_name: "Test".into(),
            last_name: "Player".into(),
            middle_name: None,
            nickname: None,
            birth_date: NaiveDate::from_ymd_opt(1995, 5, 15).unwrap(),
            country_id: 776,
            club_id,
            positions: vec![OdbPosition { code: "MC".into(), level: 18 }],
            preferred_foot: None,
            height: None,
            weight: None,
            current_ability: 120,
            potential_ability: 130,
            value: None,
            reputation: None,
            contract: Some(OdbContract {
                salary: 100_000,
                expiration: NaiveDate::from_ymd_opt(2027, 6, 30).unwrap(),
                started: None,
                contract_type: None,
                shirt_number: None,
                squad_status: None,
            }),
            loan: loan_to.map(|to| OdbLoan {
                to_club_id: to,
                to_team_id: None,
                expiration: NaiveDate::from_ymd_opt(2026, 6, 30).unwrap(),
                salary: 100_000,
                match_fee: None,
                wage_contribution_pct: None,
                future_fee: None,
                future_fee_obligation: false,
                min_appearances: None,
            }),
            history: Vec::new(),
        }
    }

    #[test]
    fn indexes_by_physical_club() {
        let odb = PlayersOdb::from_players(vec![make_player(1, 1139, None)]);
        assert!(odb.has_club(1139));
        assert_eq!(odb.for_club(1139).unwrap().len(), 1);
    }

    #[test]
    fn free_agent_routed_out_of_club_index() {
        let mut p = make_player(99, 0, None);
        p.contract = None;
        let odb = PlayersOdb::from_players(vec![p]);
        assert!(odb.for_club(0).is_none(), "free agent must not occupy a synthetic club_id=0 bucket");
        assert_eq!(odb.free_agents().len(), 1);
        assert_eq!(odb.free_agents()[0].id, 99);
    }

    #[test]
    fn loaned_player_indexed_under_borrower() {
        let odb = PlayersOdb::from_players(vec![make_player(2, 1139, Some(866))]);
        assert!(!odb.has_club(1139), "parent club must not list the loaned player in their squad");
        assert!(odb.has_club(866), "borrower physically fields the loaned player");
    }

    /// Smoke test: the embedded compiled DB loads and contains a non-trivial
    /// number of players.
    #[test]
    fn embedded_players_load() {
        let odb = PlayersOdb::load().expect("embedded DB should contain players");
        assert!(odb.max_player_id().unwrap_or(0) > 0);
    }
}
