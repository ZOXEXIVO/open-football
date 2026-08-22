//! **What the side needs next.** [`TacticalNeed`] — the single answer to
//! "what does this team want right now" that the substitution scorer
//! reads to pick which position group to bring on.
//!
//! Lives with the coach because the coach state is the source of truth
//! for that question: the need is derived from the same score / clock /
//! fatigue reading and the same [`RollingTeamMetrics`] window.

use crate::r#match::MatchContext;
use crate::r#match::engine::teamplay::coach::instruction::CoachInstruction;
use crate::r#match::engine::teamplay::coach::metrics::RollingTeamMetrics;

/// Substitution candidate scoring (Section 6). Lives here so the coach
/// state is the single source of truth for "what does this team want
/// right now". The substitutions module reads `tactical_need_for` to
/// pick which position group to bring on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TacticalNeed {
    /// Trailing late — bring on attackers for goals.
    Chasing,
    /// Leading and absorbing — bring on defenders / DM.
    ProtectingLead,
    /// Outpassed in midfield — bring on a CM/DM with passing/vision.
    LosingMidfield,
    /// Being pressed off the ball — composure / first touch / passing.
    BeingPressed,
    /// Need crosses / wing service.
    NeedingCrosses,
    /// No urgent need — fatigue rotation only.
    Fatigue,
}

impl TacticalNeed {
    /// Decide the most pressing tactical need for a team given match
    /// state and rolling metrics. Order matters — the first match wins.
    pub fn from_state(
        score_diff: i8,
        match_progress: f32,
        avg_team_condition: f32,
        metrics: RollingTeamMetrics,
    ) -> Self {
        let rung = MatchContext::score_reaction_threshold;
        let late = match_progress > rung(0.66);
        if late && score_diff < 0 {
            return TacticalNeed::Chasing;
        }
        if late && score_diff > 0 && metrics.field_tilt_last_10 > 0.55 {
            return TacticalNeed::ProtectingLead;
        }
        if metrics.possession_last_10 < 0.42 && metrics.dangerous_turnovers_last_10 >= 3 {
            return TacticalNeed::LosingMidfield;
        }
        if metrics.dangerous_turnovers_last_10 >= 4 || avg_team_condition < 0.40 {
            return TacticalNeed::BeingPressed;
        }
        if score_diff <= 0 && match_progress > rung(0.55) && metrics.shots_for_last_15 < 2 {
            return TacticalNeed::NeedingCrosses;
        }
        TacticalNeed::Fatigue
    }

    /// Same read, seeded with the mentality engine's current
    /// instruction so the two coach brains agree: a bench told
    /// "all-out attack" sends on an attacker and a side killing the
    /// game sends on a defender, regardless of what the rolling
    /// metrics alone would have inferred. Neutral instructions fall
    /// through to the metric read.
    pub fn from_state_with_instruction(
        instruction: CoachInstruction,
        score_diff: i8,
        match_progress: f32,
        avg_team_condition: f32,
        metrics: RollingTeamMetrics,
    ) -> Self {
        match instruction {
            CoachInstruction::AllOutAttack => return TacticalNeed::Chasing,
            CoachInstruction::PushForward if score_diff <= 0 => {
                return TacticalNeed::Chasing;
            }
            CoachInstruction::ParkTheBus | CoachInstruction::WasteTime => {
                return TacticalNeed::ProtectingLead;
            }
            CoachInstruction::SlowDown
                if score_diff > 0
                    && match_progress > MatchContext::score_reaction_threshold(0.66) =>
            {
                return TacticalNeed::ProtectingLead;
            }
            _ => {}
        }
        Self::from_state(score_diff, match_progress, avg_team_condition, metrics)
    }
}
