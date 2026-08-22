//! The scoreline: the two team tallies, the shootout, and the outcome
//! they add up to.
//!
//! [`TeamScore`] keeps its count in an `AtomicU8` because the engine
//! increments it through a shared `&Score`, which is also why the
//! (de)serialisation is hand-written.

use super::highlights::GoalDetail;
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicU8, Ordering};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Score {
    pub home_team: TeamScore,
    pub away_team: TeamScore,

    pub details: Vec<GoalDetail>,

    /// Penalty-shootout tally (0 if no shootout took place).
    pub home_shootout: u8,
    pub away_shootout: u8,
}

/// Outcome of a match from the home team's perspective, considering
/// both the regulation (+ extra time) score and the shootout tally.
/// Distinct from `club::MatchOutcome`, which is team-relative (Win/Draw/Loss).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MatchResultOutcome {
    HomeWin,
    AwayWin,
    Draw,
}

impl Score {
    /// True if the regulation + extra-time score is level.
    pub fn is_tied(&self) -> bool {
        self.home_team.get() == self.away_team.get()
    }

    /// Did a shootout take place? (Either side has shootout goals recorded.)
    pub fn had_shootout(&self) -> bool {
        self.home_shootout > 0 || self.away_shootout > 0
    }

    /// True outcome — accounts for penalty shootout tiebreak in knockouts.
    /// Regulation-only consumers (league table, points) should use
    /// `home_team.get()` / `away_team.get()` directly; those stay as-is.
    pub fn outcome(&self) -> MatchResultOutcome {
        let h = self.home_team.get();
        let a = self.away_team.get();
        if h > a {
            return MatchResultOutcome::HomeWin;
        }
        if a > h {
            return MatchResultOutcome::AwayWin;
        }
        // Regulation tied — use shootout if any kick was taken.
        if self.had_shootout() {
            if self.home_shootout > self.away_shootout {
                MatchResultOutcome::HomeWin
            } else if self.away_shootout > self.home_shootout {
                MatchResultOutcome::AwayWin
            } else {
                // Shootout technically can't end tied, but belt-and-braces.
                MatchResultOutcome::Draw
            }
        } else {
            MatchResultOutcome::Draw
        }
    }
}

#[derive(Debug)]
pub struct TeamScore {
    pub team_id: u32,
    score: AtomicU8,
}

impl Clone for TeamScore {
    fn clone(&self) -> Self {
        TeamScore {
            team_id: self.team_id,
            score: AtomicU8::new(self.score.load(Ordering::Relaxed)),
        }
    }
}

/// Custom (de)serialization because `AtomicU8` is not serde-friendly.
/// On the wire `TeamScore` is just `(team_id, score)`; the deserialised
/// value carries the snapshot in a fresh atomic.
impl serde::Serialize for TeamScore {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeStruct;
        let mut s = serializer.serialize_struct("TeamScore", 2)?;
        s.serialize_field("team_id", &self.team_id)?;
        s.serialize_field("score", &self.score.load(Ordering::Relaxed))?;
        s.end()
    }
}

impl<'de> serde::Deserialize<'de> for TeamScore {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(serde::Deserialize)]
        struct Wire {
            team_id: u32,
            score: u8,
        }
        let w = Wire::deserialize(deserializer)?;
        Ok(TeamScore {
            team_id: w.team_id,
            score: AtomicU8::new(w.score),
        })
    }
}

impl TeamScore {
    pub fn new(team_id: u32) -> Self {
        TeamScore {
            team_id,
            score: AtomicU8::new(0),
        }
    }

    pub fn new_with_score(team_id: u32, score: u8) -> Self {
        TeamScore {
            team_id,
            score: AtomicU8::new(score),
        }
    }

    pub fn get(&self) -> u8 {
        self.score.load(Ordering::Relaxed)
    }
}
impl From<&TeamScore> for TeamScore {
    fn from(team_score: &TeamScore) -> Self {
        TeamScore::new_with_score(team_score.team_id, team_score.score.load(Ordering::Relaxed))
    }
}

impl PartialEq<Self> for TeamScore {
    fn eq(&self, other: &Self) -> bool {
        self.score.load(Ordering::Relaxed) == other.score.load(Ordering::Relaxed)
    }
}

impl PartialOrd for TeamScore {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        let left_score = self.score.load(Ordering::Relaxed);
        let other_score = other.score.load(Ordering::Relaxed);

        Some(left_score.cmp(&other_score))
    }
}

impl Score {
    pub fn new(home_team_id: u32, away_team_id: u32) -> Self {
        Score {
            home_team: TeamScore::new(home_team_id),
            away_team: TeamScore::new(away_team_id),
            details: Vec::new(),
            home_shootout: 0,
            away_shootout: 0,
        }
    }

    pub fn add_goal_detail(&mut self, goal_detail: GoalDetail) {
        self.details.push(goal_detail)
    }

    pub fn detail(&self) -> &[GoalDetail] {
        &self.details
    }

    pub fn increment_home_goals(&self) {
        self.home_team.score.fetch_add(1, Ordering::Relaxed);
    }

    pub fn increment_away_goals(&self) {
        self.away_team.score.fetch_add(1, Ordering::Relaxed);
    }
}
