//! Embedded compiled database (`database.db`).
//!
//! The whole game database — continents, countries, national competitions,
//! leagues, clubs, names, players — is built by `open-football-database/compiler`
//! into a single gzip-compressed JSON document and embedded at compile time.
//!
//! Parsing happens exactly once per process via [`OnceLock`]; every `*Loader`
//! reads from the cached [`CompiledDatabase`].

use std::io::Read;
use std::sync::OnceLock;

use serde::Deserialize;

use super::club::ClubEntity;
use super::continent::ContinentEntity;
use super::country::CountryEntity;
use super::league::LeagueEntity;
use super::names::NamesByCountryEntity;
use super::national_competition::NationalCompetitionEntity;
use super::players::OdbPlayer;

pub const SUPPORTED_VERSION: &str = "0.01";

static DATABASE_BYTES: &[u8] = include_bytes!("../data/database.db");

#[derive(Deserialize)]
pub struct CompiledDatabase {
    pub version: String,
    pub continents: Vec<ContinentEntity>,
    pub countries: Vec<CountryEntity>,
    pub national_competitions: Vec<NationalCompetitionEntity>,
    pub leagues: Vec<LeagueEntity>,
    pub clubs: Vec<ClubEntity>,
    pub names: Vec<NamesByCountryEntity>,
    pub players: Vec<OdbPlayer>,
}

static DB: OnceLock<CompiledDatabase> = OnceLock::new();

/// Shared reference to the decompressed, parsed database. First call parses;
/// subsequent calls return the cached result. Panics if the embedded file is
/// missing, malformed, or carries an unsupported version — all compile-time
/// correctness issues that should never reach a release binary.
pub fn compiled() -> &'static CompiledDatabase {
    DB.get_or_init(|| match decode(DATABASE_BYTES) {
        Ok(db) => db,
        Err(e) => panic!("failed to load embedded database.db: {e}"),
    })
}

fn decode(compressed: &[u8]) -> Result<CompiledDatabase, String> {
    let mut dec = flate2::read::GzDecoder::new(compressed);
    let mut json = String::new();
    dec.read_to_string(&mut json)
        .map_err(|e| format!("gunzip: {e}"))?;
    let parsed: CompiledDatabase =
        serde_json::from_str(&json).map_err(|e| format!("parse: {e}"))?;
    if parsed.version != SUPPORTED_VERSION {
        return Err(format!(
            "unsupported database.db version '{}' (expected '{}')",
            parsed.version, SUPPORTED_VERSION
        ));
    }
    Ok(parsed)
}

/// Resolve a country code like "mt" → its numeric id. Returns 0 when the code
/// is empty or unknown (matching the old loader's silent-fallback behaviour).
pub fn country_id_for_code(code: &str) -> u32 {
    if code.is_empty() {
        return 0;
    }
    compiled()
        .countries
        .iter()
        .find(|c| c.code == code)
        .map(|c| c.id)
        .unwrap_or(0)
}
