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

    pub contract: OdbContract,

    /// Present when the player is currently on loan to another club.
    /// `loan.to_club_id` becomes the player's CURRENT club for squad placement;
    /// `club_id` remains the parent.
    #[serde(default)]
    pub loan: Option<OdbLoan>,
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

/// In-memory index of players, grouped by the club where they currently play
/// (loan destination if loaned, otherwise the parent club).
pub struct PlayersOdb {
    by_current_club: HashMap<u32, Vec<OdbPlayer>>,
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
        let total: usize = odb.by_current_club.values().map(|v| v.len()).sum();
        info!(
            "players loaded from compiled DB: {} players across {} clubs",
            total,
            odb.by_current_club.len()
        );
        Some(odb)
    }

    /// Index an in-memory list of players — useful for tests and ad-hoc tools.
    pub fn from_players(players: Vec<OdbPlayer>) -> Self {
        let mut by_current_club: HashMap<u32, Vec<OdbPlayer>> = HashMap::new();
        for p in players {
            let current_club = p.loan.as_ref().map(|l| l.to_club_id).unwrap_or(p.club_id);
            by_current_club.entry(current_club).or_default().push(p);
        }
        PlayersOdb { by_current_club }
    }

    pub fn for_club(&self, club_id: u32) -> Option<&[OdbPlayer]> {
        self.by_current_club.get(&club_id).map(|v| v.as_slice())
    }

    pub fn has_club(&self, club_id: u32) -> bool {
        self.by_current_club.contains_key(&club_id)
    }

    /// Highest player id present in the index, or `None` when empty.
    /// Used to seed the procedural id sequence so generated players never
    /// collide with externally-supplied ids.
    pub fn max_player_id(&self) -> Option<u32> {
        self.by_current_club
            .values()
            .flat_map(|v| v.iter().map(|p| p.id))
            .max()
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
            contract: OdbContract {
                salary: 100_000,
                expiration: NaiveDate::from_ymd_opt(2027, 6, 30).unwrap(),
                started: None,
                contract_type: None,
                shirt_number: None,
                squad_status: None,
            },
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
        }
    }

    #[test]
    fn indexes_by_current_club() {
        let odb = PlayersOdb::from_players(vec![make_player(1, 1139, None)]);
        assert!(odb.has_club(1139));
        assert_eq!(odb.for_club(1139).unwrap().len(), 1);
    }

    #[test]
    fn loaned_player_indexed_under_borrower() {
        let odb = PlayersOdb::from_players(vec![make_player(2, 1139, Some(866))]);
        assert!(!odb.has_club(1139), "parent club must NOT have the loaned player");
        assert!(odb.has_club(866), "borrower must own the loaned player");
    }

    /// Smoke test: the embedded compiled DB loads and contains a non-trivial
    /// number of players.
    #[test]
    fn embedded_players_load() {
        let odb = PlayersOdb::load().expect("embedded DB should contain players");
        assert!(odb.max_player_id().unwrap_or(0) > 0);
    }
}
