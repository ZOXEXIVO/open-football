//! **What the side needs next.** [`TacticalNeed`] — the single answer to
//! "what does this team want right now" that the substitution scorer
//! reads to pick which position group to bring on.
//!
//! Lives with the coach because the coach is the source of truth for that
//! question: the need is his stance and his read of the run of play.

use crate::r#match::engine::teamplay::coach::metrics::RollingTeamMetrics;

/// Substitution candidate scoring (Section 6). Lives here so the coach
/// state is the single source of truth for "what does this team want
/// right now". The substitutions module reads `tactical_need_for` to
/// pick which position group to bring on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TacticalNeed {
    /// Chasing the match — bring on attackers for goals.
    Chasing,
    /// Protecting a result and absorbing — bring on defenders / DM.
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
    /// A stance this far either way asks the bench for an attacker, or for
    /// a defender while the side is being pushed back.
    const DECIDED: f32 = 0.4;

    /// What the bench is asked for: the coach's stance first, then what his
    /// read of the run of play says the side is short of.
    pub fn from_coach(stance: f32, read: RollingTeamMetrics, avg_team_condition: f32) -> Self {
        if stance >= Self::DECIDED {
            return TacticalNeed::Chasing;
        }
        if stance <= -Self::DECIDED && read.field_tilt_last_10 > 0.5 {
            return TacticalNeed::ProtectingLead;
        }
        if read.possession_last_10 < 0.42 && read.dangerous_turnovers_last_10 >= 3 {
            return TacticalNeed::LosingMidfield;
        }
        if read.dangerous_turnovers_last_10 >= 4 || avg_team_condition < 0.40 {
            return TacticalNeed::BeingPressed;
        }
        if stance > 0.0 && read.shots_for_last_15 < 2 {
            return TacticalNeed::NeedingCrosses;
        }
        TacticalNeed::Fatigue
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_chasing_coach_asks_for_an_attacker_and_a_protecting_one_for_a_defender() {
        let read = RollingTeamMetrics {
            field_tilt_last_10: 0.6,
            possession_last_10: 0.5,
            ..Default::default()
        };
        assert_eq!(
            TacticalNeed::from_coach(0.7, read, 0.8),
            TacticalNeed::Chasing
        );
        assert_eq!(
            TacticalNeed::from_coach(-0.7, read, 0.8),
            TacticalNeed::ProtectingLead
        );
        assert_eq!(
            TacticalNeed::from_coach(0.0, read, 0.8),
            TacticalNeed::Fatigue
        );
    }
}
