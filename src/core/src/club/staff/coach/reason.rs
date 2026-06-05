//! Structured reasons the coach attaches to a decision.
//!
//! Closed enum, reused by squad selection, substitutions, and the
//! omission events feed. The point is to keep the *why* of a decision
//! out of free text — a downstream consumer (UI, manager talk, morale
//! event) can branch on the variant without parsing prose.
//!
//! Reasons are evidence-pointing labels, not actions. "PoorRecentForm"
//! means the coach saw recent ratings drop below the baseline; it does
//! not mean the coach has decided to drop the player — selection and
//! substitution layers turn the reasons into adjustments.

/// One reason the coach attached to a decision. Multiple can be
/// present on the same assessment — the top one becomes the dominant
/// explanation, the rest enrich diagnostics.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CoachDecisionReason {
    /// Recent EMA has fallen below the long-form baseline.
    PoorRecentForm,
    /// Coach has seen K poor matches in a row — read as a settled drop.
    SustainedPoorForm,
    /// Recent EMA has risen above the long-form baseline.
    StrongRecentForm,
    /// Coach's tactical_trust reading is low.
    LowTacticalTrust,
    /// Coach's tactical_trust reading is high.
    HighTacticalTrust,
    /// Heavy workload / acute:chronic spike — sit him.
    FatigueRisk,
    /// Returning from injury / fragile to a heavy minute.
    InjuryRisk,
    /// Development call — the player needs minutes for his pathway.
    DevelopmentPathway,
    /// Succession-planning slot — coach is bedding in the heir.
    SuccessionPlanning,
    /// Player has shown up in cup ties / derbies — start him.
    BigMatchReliability,
    /// Player has failed in big matches — prefer the alternative.
    BigMatchFailure,
    /// Coach has formed a positive read of his training contributions.
    TrainingLevel,
    /// Coach has a strong personal relationship with the player.
    CoachRelationship,
    /// Coach's role_fit_confidence is low — wrong slot for him.
    RoleMismatch,
    /// Tactical shape demanded a different profile.
    TacticalNeed,
    /// Live in-match rating below threshold.
    LiveMatchUnderperformance,
    /// In-match error contributed to a goal — strong sub-off case.
    CostlyError,
    /// Yellow card + aggression + defensive role — pull him.
    CardRisk,
    /// Star contribution (goal / assist / high rating) protects him.
    ProtectingStar,
    /// Sticky doubt flag has not yet been cleared.
    StickyDoubt,
}

impl CoachDecisionReason {
    /// Stable i18n token. Used by future UI / morale-event renderers
    /// to look up a localised sentence. Mirrors the
    /// `SelectionOmissionReason::as_i18n_key` pattern so callers can
    /// route by key without depending on the enum's display name.
    pub fn as_i18n_key(&self) -> &'static str {
        match self {
            CoachDecisionReason::PoorRecentForm => "coach_reason_poor_recent_form",
            CoachDecisionReason::SustainedPoorForm => "coach_reason_sustained_poor_form",
            CoachDecisionReason::StrongRecentForm => "coach_reason_strong_recent_form",
            CoachDecisionReason::LowTacticalTrust => "coach_reason_low_tactical_trust",
            CoachDecisionReason::HighTacticalTrust => "coach_reason_high_tactical_trust",
            CoachDecisionReason::FatigueRisk => "coach_reason_fatigue_risk",
            CoachDecisionReason::InjuryRisk => "coach_reason_injury_risk",
            CoachDecisionReason::DevelopmentPathway => "coach_reason_development_pathway",
            CoachDecisionReason::SuccessionPlanning => "coach_reason_succession_planning",
            CoachDecisionReason::BigMatchReliability => "coach_reason_big_match_reliability",
            CoachDecisionReason::BigMatchFailure => "coach_reason_big_match_failure",
            CoachDecisionReason::TrainingLevel => "coach_reason_training_level",
            CoachDecisionReason::CoachRelationship => "coach_reason_coach_relationship",
            CoachDecisionReason::RoleMismatch => "coach_reason_role_mismatch",
            CoachDecisionReason::TacticalNeed => "coach_reason_tactical_need",
            CoachDecisionReason::LiveMatchUnderperformance => {
                "coach_reason_live_match_underperformance"
            }
            CoachDecisionReason::CostlyError => "coach_reason_costly_error",
            CoachDecisionReason::CardRisk => "coach_reason_card_risk",
            CoachDecisionReason::ProtectingStar => "coach_reason_protecting_star",
            CoachDecisionReason::StickyDoubt => "coach_reason_sticky_doubt",
        }
    }

    /// True when the reason argues *against* selecting / keeping the
    /// player on. Lets the selection layer decide whether to apply the
    /// reason's adjustment as a negative or positive nudge.
    pub fn is_negative(&self) -> bool {
        matches!(
            self,
            CoachDecisionReason::PoorRecentForm
                | CoachDecisionReason::SustainedPoorForm
                | CoachDecisionReason::LowTacticalTrust
                | CoachDecisionReason::FatigueRisk
                | CoachDecisionReason::InjuryRisk
                | CoachDecisionReason::BigMatchFailure
                | CoachDecisionReason::RoleMismatch
                | CoachDecisionReason::LiveMatchUnderperformance
                | CoachDecisionReason::CostlyError
                | CoachDecisionReason::CardRisk
                | CoachDecisionReason::StickyDoubt
        )
    }
}
