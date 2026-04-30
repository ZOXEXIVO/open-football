//! Tunables for the top-level simulator orchestrator.
//!
//! Mirrors the `TransferConfig`, `ScoutingConfig`, and `PlayerBehaviourConfig`
//! pattern: per-knob fields with named documentation, a `Default` impl that
//! captures the published values, and a clear hook for per-save overrides
//! later (just plumb a `&SimulatorConfig` through the public entry points
//! instead of calling `default()` internally).

use chrono::Datelike;

#[derive(Debug, Clone)]
pub struct SimulatorConfig {
    /// Day-of-month on which `MatchStorage::trim` runs. Once-a-month is a
    /// cheap BTreeMap range walk over evicted dates only — no need to do
    /// it daily, but doing it less often risks unbounded growth.
    pub match_store_trim_day_of_month: u32,
}

impl Default for SimulatorConfig {
    fn default() -> Self {
        SimulatorConfig {
            match_store_trim_day_of_month: 1,
        }
    }
}

impl SimulatorConfig {
    /// Whether `now` falls on the configured trim day (compared by day-of-month).
    pub fn is_trim_day(&self, now: chrono::NaiveDate) -> bool {
        now.day() == self.match_store_trim_day_of_month
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;

    #[test]
    fn default_values_match_documented_constants() {
        let c = SimulatorConfig::default();
        assert_eq!(c.match_store_trim_day_of_month, 1);
    }

    #[test]
    fn trim_day_check() {
        let c = SimulatorConfig::default();
        assert!(c.is_trim_day(NaiveDate::from_ymd_opt(2026, 4, 1).unwrap()));
        assert!(!c.is_trim_day(NaiveDate::from_ymd_opt(2026, 4, 15).unwrap()));
    }

    #[test]
    fn override_trim_day() {
        let mut c = SimulatorConfig::default();
        c.match_store_trim_day_of_month = 15;
        assert!(c.is_trim_day(NaiveDate::from_ymd_opt(2026, 4, 15).unwrap()));
        assert!(!c.is_trim_day(NaiveDate::from_ymd_opt(2026, 4, 1).unwrap()));
    }
}
