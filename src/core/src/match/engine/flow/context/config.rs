//! Full match-construction inputs — the one struct a caller fills in to
//! get a specific match rather than the engine's neutral defaults.

use crate::r#match::engine::environment::MatchEnvironment;
use crate::r#match::engine::referee::RefereeProfile;
use chrono::{NaiveDate, Utc};

/// Full match-construction inputs. Replaces the loose
/// `play_seeded(.., seed)` signature for callers that need to inject
/// weather, referee profile, or fixture date alongside the seed —
/// notably the calibration harness and any replay/test path that wants
/// a real rainy / strict-ref / cup-final match instead of the
/// engine's neutral defaults.
///
/// `play` and `play_seeded` are kept as compatibility wrappers around
/// `play_with_config` so existing call sites don't move.
#[derive(Debug, Clone)]
pub struct MatchEngineConfig {
    pub seed: Option<u64>,
    pub today: NaiveDate,
    pub environment: MatchEnvironment,
    pub referee: RefereeProfile,
    pub is_friendly: bool,
    pub is_knockout: bool,
    pub match_recordings: bool,
}

impl Default for MatchEngineConfig {
    fn default() -> Self {
        MatchEngineConfig {
            seed: None,
            today: Utc::now().naive_utc().date(),
            environment: MatchEnvironment::default(),
            referee: RefereeProfile::default(),
            is_friendly: false,
            is_knockout: false,
            match_recordings: false,
        }
    }
}

impl MatchEngineConfig {
    /// Convenience: build a seeded config with everything else default.
    pub fn seeded(seed: u64) -> Self {
        MatchEngineConfig {
            seed: Some(seed),
            ..Default::default()
        }
    }
}
