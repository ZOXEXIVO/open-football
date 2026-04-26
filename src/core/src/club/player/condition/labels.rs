//! Derived condition label for UI / scout / coach displays.
//!
//! UI surfaces should not need to know about the four scales (condition,
//! fitness, jadedness, match_readiness) plus PlayerLoad and recovery
//! debt. Pick the most informative label and let the user click through
//! for raw bars if they want.
//!
//! Labels are checked in priority order — the first match wins, so
//! "Returning From Injury" beats "Heavy Legs" beats "Fresh".

use crate::club::player::load::{PHYSICAL_LOAD_DANGER, RECOVERY_DEBT_HEAVY};
use crate::club::player::player::Player;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConditionLabel {
    Injured,
    ReturningFromInjury,
    /// Out long enough that a manager would want to manage minutes.
    LimitedMinutesRecommended,
    /// Workload spike or post-injury — a manager should think twice
    /// before starting them.
    ElevatedInjuryRisk,
    /// Recovery debt is high — body's tired beyond what condition shows.
    HeavyLegs,
    /// Jadedness up; the standard FM "needs a rest" call.
    NeedsRest,
    /// Lots of training but not many matches — sharpness is fading.
    LackingMatchSharpness,
    /// Healthy and on a moderate workload.
    Fit,
    /// Top-end: high condition, low load, sharp.
    Fresh,
}

impl ConditionLabel {
    pub fn as_str(self) -> &'static str {
        match self {
            ConditionLabel::Injured => "Injured",
            ConditionLabel::ReturningFromInjury => "Returning From Injury",
            ConditionLabel::LimitedMinutesRecommended => "Limited Minutes Recommended",
            ConditionLabel::ElevatedInjuryRisk => "Elevated Injury Risk",
            ConditionLabel::HeavyLegs => "Heavy Legs",
            ConditionLabel::NeedsRest => "Needs Rest",
            ConditionLabel::LackingMatchSharpness => "Lacking Match Sharpness",
            ConditionLabel::Fit => "Fit",
            ConditionLabel::Fresh => "Fresh",
        }
    }
}

impl Player {
    /// Single, human-readable summary of how the player is feeling.
    /// Reads `is_injured`, `is_in_recovery`, condition, jadedness,
    /// PlayerLoad (physical & spike), recovery_debt, and
    /// match_readiness. Order matters — see priority list above.
    pub fn condition_label(&self) -> ConditionLabel {
        if self.player_attributes.is_injured {
            return ConditionLabel::Injured;
        }
        if self.player_attributes.is_in_recovery() {
            // Long recoveries → recommend limited minutes for a while
            // beyond just "returning". 14d is the rough threshold where
            // a coach would phase a player back in via cameos.
            if self.player_attributes.recovery_days_remaining >= 14 {
                return ConditionLabel::LimitedMinutesRecommended;
            }
            return ConditionLabel::ReturningFromInjury;
        }

        let condition_pct = self.player_attributes.condition_percentage();
        let jadedness = self.player_attributes.jadedness;
        let load_7 = self.load.physical_load_7;
        let debt = self.load.recovery_debt;
        let mr = self.skills.physical.match_readiness;
        let days_idle = self.player_attributes.days_since_last_match;

        // Workload spike (only meaningful with chronic baseline) or
        // outright overload — coach should consider rotation.
        if self.load.is_workload_spike() || load_7 >= PHYSICAL_LOAD_DANGER {
            return ConditionLabel::ElevatedInjuryRisk;
        }

        if debt >= RECOVERY_DEBT_HEAVY {
            return ConditionLabel::HeavyLegs;
        }

        if jadedness >= 7000 {
            return ConditionLabel::NeedsRest;
        }

        if mr < 8.0 && days_idle >= 14 {
            return ConditionLabel::LackingMatchSharpness;
        }

        // Fresh: high condition, low recent load, sharp readiness.
        if condition_pct >= 90 && load_7 < 200.0 && mr >= 14.0 && jadedness < 3000 {
            return ConditionLabel::Fresh;
        }

        ConditionLabel::Fit
    }
}
