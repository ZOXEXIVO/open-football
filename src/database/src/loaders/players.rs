//! External player database loader (`players.odb`).
//!
//! The file is a gzip-compressed JSON document of the form:
//! ```json
//! { "version": "0.01", "players": [ ... ] }
//! ```
//! embedded at compile time from `src/data/players.odb`. Every club referenced
//! by at least one ODB player is populated from this file instead of via the
//! procedural generator. Academy/youth (U18/U19) generation is left untouched.

use chrono::NaiveDate;
use log::{info, warn};
use serde::Deserialize;
use std::collections::HashMap;
use std::io::Read;

pub const ODB_SUPPORTED_VERSION: &str = "0.01";

/// Embedded gzip-compressed ODB payload.
static ODB_BYTES: &[u8] = include_bytes!("../data/players.odb");

#[derive(Debug, Deserialize)]
pub struct OdbFile {
    pub version: String,
    pub players: Vec<OdbPlayer>,
}

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

#[derive(Debug, Clone, Deserialize)]
pub struct OdbReputation {
    pub home: i16,
    pub world: i16,
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

/// In-memory index of ODB players, grouped by the club where they currently play
/// (loan destination if loaned, otherwise the parent club).
pub struct PlayersOdb {
    by_current_club: HashMap<u32, Vec<OdbPlayer>>,
}

impl PlayersOdb {
    /// Decode the embedded `players.odb` payload. Returns `None` if the
    /// bundled file is empty, malformed, or carries an unsupported version.
    pub fn load() -> Option<Self> {
        if ODB_BYTES.is_empty() {
            return None;
        }
        match Self::load_from_bytes(ODB_BYTES) {
            Ok(odb) => {
                let total: usize = odb.by_current_club.values().map(|v| v.len()).sum();
                info!(
                    "players.odb loaded: {} players across {} clubs (embedded, {} bytes)",
                    total,
                    odb.by_current_club.len(),
                    ODB_BYTES.len()
                );
                Some(odb)
            }
            Err(e) => {
                warn!("embedded players.odb ignored: {}", e);
                None
            }
        }
    }

    pub fn load_from_bytes(compressed: &[u8]) -> Result<Self, String> {
        let mut decoder = flate2::read::GzDecoder::new(compressed);
        let mut json = String::new();
        decoder
            .read_to_string(&mut json)
            .map_err(|e| format!("gunzip: {e}"))?;

        let parsed: OdbFile =
            serde_json::from_str(&json).map_err(|e| format!("parse: {e}"))?;

        if parsed.version != ODB_SUPPORTED_VERSION {
            return Err(format!(
                "unsupported version '{}' (expected '{}')",
                parsed.version, ODB_SUPPORTED_VERSION
            ));
        }

        let mut by_current_club: HashMap<u32, Vec<OdbPlayer>> = HashMap::new();
        for p in parsed.players {
            let current_club = p
                .loan
                .as_ref()
                .map(|l| l.to_club_id)
                .unwrap_or(p.club_id);
            by_current_club.entry(current_club).or_default().push(p);
        }

        Ok(PlayersOdb { by_current_club })
    }

    pub fn for_club(&self, club_id: u32) -> Option<&[OdbPlayer]> {
        self.by_current_club.get(&club_id).map(|v| v.as_slice())
    }

    pub fn has_club(&self, club_id: u32) -> bool {
        self.by_current_club.contains_key(&club_id)
    }

    /// Highest player id present in the index, or `None` when empty.
    /// Used to seed the procedural id sequence so generated players never
    /// collide with ODB-supplied ids.
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
    use flate2::write::GzEncoder;
    use flate2::Compression;
    use std::io::Write;

    fn gz(json: &str) -> Vec<u8> {
        let mut enc = GzEncoder::new(Vec::new(), Compression::default());
        enc.write_all(json.as_bytes()).unwrap();
        enc.finish().unwrap()
    }

    #[test]
    fn loads_minimal_record() {
        let bytes = gz(r#"{
            "version": "0.01",
            "players": [{
                "id": 1,
                "first_name": "Test",
                "last_name": "Player",
                "birth_date": "1995-05-15",
                "country_id": 776,
                "club_id": 1139,
                "positions": [{"code": "MC", "level": 18}],
                "current_ability": 120,
                "potential_ability": 130,
                "contract": {"salary": 100000, "expiration": "2027-06-30"}
            }]
        }"#);
        let odb = PlayersOdb::load_from_bytes(&bytes).unwrap();
        assert!(odb.has_club(1139));
        assert_eq!(odb.for_club(1139).unwrap().len(), 1);
    }

    #[test]
    fn loaned_player_indexed_under_borrower() {
        let bytes = gz(r#"{
            "version": "0.01",
            "players": [{
                "id": 2,
                "first_name": "On",
                "last_name": "Loan",
                "birth_date": "2000-01-01",
                "country_id": 390,
                "club_id": 1139,
                "positions": [{"code": "DR", "level": 18}],
                "current_ability": 130,
                "potential_ability": 140,
                "contract": {"salary": 200000, "expiration": "2028-06-30"},
                "loan": {
                    "to_club_id": 866,
                    "expiration": "2026-06-30",
                    "salary": 200000
                }
            }]
        }"#);
        let odb = PlayersOdb::load_from_bytes(&bytes).unwrap();
        assert!(!odb.has_club(1139), "parent club must NOT have the loaned player");
        assert!(odb.has_club(866), "borrower must own the loaned player");
    }

    #[test]
    fn rejects_unknown_version() {
        let bytes = gz(r#"{"version": "9.99", "players": []}"#);
        assert!(PlayersOdb::load_from_bytes(&bytes).is_err());
    }

    /// Sanity check that the embedded sample is a valid ODB and contains
    /// the expected Juventus (1139) and OM (866) buckets.
    #[test]
    fn bundled_sample_loads() {
        let odb = PlayersOdb::load().expect("embedded players.odb is malformed");
        assert!(odb.has_club(1139), "sample missing Juventus bucket");
        assert!(odb.has_club(866), "sample missing OM bucket");
    }
}
