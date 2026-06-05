//! Live-match snapshot of the coach's persistent state.
//!
//! The live match runner doesn't carry the head coach's `Staff`
//! across the match boundary — the match engine only needs the
//! pieces that drive its in-flight decisions. [`CoachMatchSnapshot`]
//! is the minimal bundle (memory store + perception profile +
//! match-day strategy) the substitution layer needs to build a
//! [`CoachDecisionEngine`] on the live side of the match.
//!
//! Captured at squad-construction time from the team's head coach
//! and carried through [`MatchSquad`] → [`MatchField`] →
//! [`MatchContext`] so the substitution wrapper can build the engine
//! without reaching back to the league pipeline.

use super::memory::CoachMemoryStore;
use super::strategy::CoachStrategy;
use crate::club::staff::CoachProfile;

/// Live-match coach snapshot. Cloned from the head coach at
/// squad-construction time. The memory store is the only field with
/// non-trivial size — and even that is bounded by the squad size.
#[derive(Debug, Clone)]
pub struct CoachMatchSnapshot {
    pub memory: CoachMemoryStore,
    pub profile: CoachProfile,
    pub strategy: CoachStrategy,
}

impl CoachMatchSnapshot {
    pub fn new(memory: CoachMemoryStore, profile: CoachProfile, strategy: CoachStrategy) -> Self {
        CoachMatchSnapshot {
            memory,
            profile,
            strategy,
        }
    }
}
