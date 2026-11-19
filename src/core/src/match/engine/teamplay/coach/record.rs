//! **What the coach did, for the result.** The plan a side kicked off with,
//! every change to it with the minute and the moment that prompted it, and
//! the half-time talk. Kept by [`MatchCoach`](super::match_coach::MatchCoach)
//! as it happens and handed back on the result.

use crate::MatchTacticType;
use crate::club::team::talks::TeamTalkTone;
use crate::r#match::engine::teamplay::coach::plan::GamePlan;
use serde::{Deserialize, Serialize};

/// What prompted a change to the plan.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ChangeMoment {
    HalfTime,
    GoalFor,
    GoalAgainst,
    SendingOff,
    ForcedChange,
    FailingRead,
}

impl ChangeMoment {
    pub const COUNT: usize = 6;
    pub const NAMES: [&'static str; Self::COUNT] = [
        "half-time",
        "goal for",
        "goal against",
        "sending-off",
        "forced change",
        "failing read",
    ];

    pub fn index(self) -> usize {
        match self {
            ChangeMoment::HalfTime => 0,
            ChangeMoment::GoalFor => 1,
            ChangeMoment::GoalAgainst => 2,
            ChangeMoment::SendingOff => 3,
            ChangeMoment::ForcedChange => 4,
            ChangeMoment::FailingRead => 5,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct PlanChange {
    pub minute: u8,
    pub moment: ChangeMoment,
    pub shape_before: MatchTacticType,
    pub shape_after: MatchTacticType,
    pub plan_before: GamePlan,
    pub plan_after: GamePlan,
}

impl PlanChange {
    pub fn changes_shape(&self) -> bool {
        self.shape_before != self.shape_after
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct CoachRecord {
    pub kickoff_plan: Option<GamePlan>,
    pub changes: Vec<PlanChange>,
    pub half_time_talk: Option<TeamTalkTone>,
    /// The commitment reading, one entry per minute played, −1 holding on
    /// to what the side has … +1 throwing everything forward.
    #[cfg(feature = "match-logs")]
    #[serde(default)]
    pub commitment: Vec<f32>,
    /// Times the opposition passed or carried the ball past the last line.
    #[cfg(feature = "match-logs")]
    #[serde(default)]
    pub line_breaks_against: u32,
}

impl CoachRecord {
    pub fn first_shape_change_minute(&self) -> Option<u8> {
        self.changes
            .iter()
            .find(|c| c.changes_shape())
            .map(|c| c.minute)
    }

    #[cfg(feature = "match-logs")]
    pub fn note_commitment(&mut self, minute: usize, value: f32) {
        while self.commitment.len() <= minute {
            self.commitment.push(value);
        }
    }
}
