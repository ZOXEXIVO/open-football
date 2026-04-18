//! Calendar-driven phase of the football year. Systems that care about
//! rhythms (condition recovery, match sharpness, later on transfer urgency
//! and scheduling) ask `SeasonPhase::from_date(date)` rather than
//! re-deriving their own window boundaries.
//!
//! European Aug–May calendar. Summer-calendar countries (Russia, Norway)
//! need a future `from_date_for_country` overload.

use chrono::{Datelike, NaiveDate};

/// Where in the season year a given calendar date sits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SeasonPhase {
    /// Post-season rest — competitive season is done, pre-season hasn't
    /// begun. Roughly Jun 1 – Jul 14.
    PostSeason,
    /// Players returning to training; friendlies only; sharpness rebuilds.
    /// Roughly Jul 15 – Aug 14.
    PreSeason,
    /// First two and a half months of competitive fixtures. Squads still
    /// settling, form not yet meaningful.
    EarlySeason,
    /// Mid-season run between the opening months and the winter break.
    /// Busiest congestion window.
    MidSeason,
    /// Christmas/New Year break observed in many European leagues.
    /// Roughly Dec 20 – Jan 6. English football runs through this.
    WinterBreak,
    /// Final six weeks before season end — relegation, title, and European
    /// qualification races all converge. Rotation loosens.
    RunIn,
}

impl SeasonPhase {
    /// Resolve the phase a calendar date sits in, assuming a European
    /// Aug-start season. See module docs for country-specific layering.
    pub fn from_date(date: NaiveDate) -> Self {
        let (month, day) = (date.month(), date.day());
        match (month, day) {
            (6, _) => Self::PostSeason,
            (7, d) if d <= 14 => Self::PostSeason,
            (7, _) => Self::PreSeason,
            (8, d) if d <= 14 => Self::PreSeason,
            (8, _) => Self::EarlySeason,
            (9, _) | (10, _) => Self::EarlySeason,
            (11, _) => Self::MidSeason,
            (12, d) if d < 20 => Self::MidSeason,
            (12, _) => Self::WinterBreak,
            (1, d) if d <= 6 => Self::WinterBreak,
            (1, _) | (2, _) | (3, _) => Self::MidSeason,
            (4, d) if d <= 15 => Self::MidSeason,
            (4, _) | (5, _) => Self::RunIn,
            _ => Self::MidSeason,
        }
    }

    /// Multiplier applied to rest-day condition recovery. Players in the
    /// winter break are resting at a training camp or at home — recovery
    /// is faster than a typical mid-season rest day.
    pub fn condition_recovery_multiplier(self) -> f32 {
        match self {
            Self::WinterBreak => 1.6,
            Self::PostSeason => 1.4, // summer break — pure recovery
            Self::PreSeason => 1.1,  // light conditioning sessions
            _ => 1.0,
        }
    }

    /// Match sharpness gain in non-match days. Pre-season training camps
    /// actively rebuild match readiness toward 20/20; the rest of the year
    /// is neutral-or-decaying (handled elsewhere).
    pub fn match_readiness_gain(self) -> f32 {
        match self {
            Self::PreSeason => 0.25,   // steady daily rebuild
            Self::PostSeason => 0.0,   // break; rebuilds after pre-season starts
            Self::WinterBreak => 0.10, // maintenance only
            _ => 0.0,
        }
    }

    /// True during the actual competitive run of fixtures.
    pub fn is_competitive(self) -> bool {
        matches!(
            self,
            Self::EarlySeason | Self::MidSeason | Self::RunIn
        )
    }

    /// True during phases where the club is not playing competitive matches.
    pub fn is_rest_window(self) -> bool {
        matches!(self, Self::PostSeason | Self::WinterBreak)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(y: i32, m: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, day).unwrap()
    }

    #[test]
    fn summer_is_post_season() {
        assert_eq!(SeasonPhase::from_date(d(2025, 6, 15)), SeasonPhase::PostSeason);
        assert_eq!(SeasonPhase::from_date(d(2025, 7, 5)), SeasonPhase::PostSeason);
    }

    #[test]
    fn late_july_is_pre_season() {
        assert_eq!(SeasonPhase::from_date(d(2025, 7, 20)), SeasonPhase::PreSeason);
        assert_eq!(SeasonPhase::from_date(d(2025, 8, 10)), SeasonPhase::PreSeason);
    }

    #[test]
    fn mid_august_opens_early_season() {
        assert_eq!(SeasonPhase::from_date(d(2025, 8, 20)), SeasonPhase::EarlySeason);
        assert_eq!(SeasonPhase::from_date(d(2025, 10, 31)), SeasonPhase::EarlySeason);
    }

    #[test]
    fn late_december_is_winter_break() {
        assert_eq!(SeasonPhase::from_date(d(2025, 12, 25)), SeasonPhase::WinterBreak);
        assert_eq!(SeasonPhase::from_date(d(2026, 1, 3)), SeasonPhase::WinterBreak);
    }

    #[test]
    fn january_7_onward_is_mid_season() {
        assert_eq!(SeasonPhase::from_date(d(2026, 1, 10)), SeasonPhase::MidSeason);
        assert_eq!(SeasonPhase::from_date(d(2026, 3, 15)), SeasonPhase::MidSeason);
    }

    #[test]
    fn late_april_starts_run_in() {
        assert_eq!(SeasonPhase::from_date(d(2026, 4, 20)), SeasonPhase::RunIn);
        assert_eq!(SeasonPhase::from_date(d(2026, 5, 31)), SeasonPhase::RunIn);
    }

    #[test]
    fn winter_break_has_fastest_rest_recovery() {
        let wb = SeasonPhase::WinterBreak.condition_recovery_multiplier();
        let mid = SeasonPhase::MidSeason.condition_recovery_multiplier();
        let post = SeasonPhase::PostSeason.condition_recovery_multiplier();
        assert!(wb > mid);
        assert!(wb > post); // winter > summer because winter players still train
    }

    #[test]
    fn pre_season_rebuilds_readiness() {
        assert!(SeasonPhase::PreSeason.match_readiness_gain() > 0.0);
        assert_eq!(SeasonPhase::EarlySeason.match_readiness_gain(), 0.0);
    }

    #[test]
    fn is_competitive_classification() {
        assert!(SeasonPhase::EarlySeason.is_competitive());
        assert!(SeasonPhase::RunIn.is_competitive());
        assert!(!SeasonPhase::PreSeason.is_competitive());
        assert!(!SeasonPhase::WinterBreak.is_competitive());
    }
}
