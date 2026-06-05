//! Computed read of one player at one decision point.
//!
//! The [`CoachDecisionEngine`] produces a [`CoachPlayerAssessment`]
//! whenever the selection or substitution layers need a coach-aware
//! opinion. The assessment never directly drops or starts a player —
//! it returns a small adjustment and a structured list of reasons
//! the caller can fold into its existing scoring.

use super::reason::CoachDecisionReason;

/// A single coach-aware adjustment plus the reasons behind it. Used
/// both for starting-XI slot scoring and bench-role scoring; the
/// caller decides what to do with the value.
#[derive(Debug, Clone, Default)]
pub struct CoachDecisionScore {
    /// Signed adjustment to fold into the existing slot/bench score.
    /// Negative = the coach would prefer not to use the player here.
    /// Positive = the coach actively favours them.
    pub adjustment: f32,
    /// Reasons ordered by dominance — the first is the one the
    /// omissions builder / sub log should surface to the UI.
    pub reasons: Vec<CoachDecisionReason>,
}

impl CoachDecisionScore {
    pub fn neutral() -> Self {
        CoachDecisionScore::default()
    }

    pub fn dominant_reason(&self) -> Option<CoachDecisionReason> {
        self.reasons.first().copied()
    }

    pub fn push_reason(&mut self, reason: CoachDecisionReason) {
        if !self.reasons.contains(&reason) {
            self.reasons.push(reason);
        }
    }
}

/// Detailed coach-aware read of one player at one decision point.
/// Wraps the raw score signals (selection / form / risk / trust /
/// role) and a development priority bucket. Reasons are inline so the
/// caller doesn't have to re-derive them.
#[derive(Debug, Clone, Default)]
pub struct CoachPlayerAssessment {
    /// Overall selection confidence (0..1) — coach's read of "I would
    /// happily start this player today". Combines form, trust, role
    /// fit, and strategy.
    pub selection_confidence: f32,
    /// Form confidence (0..1) — short-term reading of recent ratings.
    pub form_confidence: f32,
    /// Risk confidence (0..1) — how comfortable the coach is with the
    /// player's reliability (errors, cards, hooks).
    pub risk_confidence: f32,
    /// Trust score (0..1) — relationship-and-experience composite.
    pub trust_score: f32,
    /// Role-fit score (0..1) — confidence in the player at his
    /// natural slot.
    pub role_fit_score: f32,
    /// Development priority (0..1) — coach's read of "this player
    /// needs minutes for his pathway".
    pub development_priority: f32,
    /// Drop risk (0..1) — symmetric to start_preference: how much the
    /// coach would prefer to drop the player from the XI.
    pub drop_risk: f32,
    /// Net selection-XI preference (0..1) — adjusted by strategy.
    pub start_preference: f32,
    /// Net bench preference (0..1) — does the coach want him on the
    /// matchday bench at minimum?
    pub bench_preference: f32,
    /// In-match urgency to remove the player (0..1). Live-match only.
    pub sub_off_urgency: f32,
    /// In-match preference to bring this player on (0..1). Live-match
    /// only.
    pub sub_in_preference: f32,
    pub reasons: Vec<CoachDecisionReason>,
}

impl CoachPlayerAssessment {
    /// Convert the assessment's net selection signal into a signed
    /// adjustment usable by the existing scoring engine. The caller
    /// scales this by a small constant so it doesn't dominate raw
    /// quality — coach personality nudges the result, never replaces
    /// the underlying skill judgement.
    pub fn selection_adjustment(&self) -> f32 {
        // [-1, +1] centered on neutral confidence.
        (self.start_preference - 0.5) * 2.0
    }

    /// Same for the bench. A high-trust fringe player can edge out a
    /// higher-quality but distrusted alternative on the matchday 18.
    pub fn bench_adjustment(&self) -> f32 {
        (self.bench_preference - 0.5) * 2.0
    }

    pub fn dominant_reason(&self) -> Option<CoachDecisionReason> {
        self.reasons.first().copied()
    }
}
