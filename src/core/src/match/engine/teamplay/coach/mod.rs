//! Coach in-match instructions that control team tempo and behavior.
//!
//! The coach evaluates score, time, and fatigue every few seconds and issues
//! instructions that all players consult when making decisions.
//!
//! **Every clock threshold that gates a SCORE-dependent branch in here goes
//! through `MatchContext::score_reaction_threshold`**, so the coach's
//! escalation ladder moves with the rest of the score-reactive regime
//! instead of being tuned against it — see
//! `MatchContext::SCORE_REACTION_GAIN` for why a partial application is
//! worse than none. Thresholds that read only the clock or the fatigue
//! (half-time management, tired legs) are NOT score-reactive and stay put.
//!
//! Split by what each part owns:
//!
//! | Module | Concern |
//! |---|---|
//! | [`instruction`] | What an instruction MEANS: [`CoachInstruction`] and its [`InstructionCoefficients`] |
//! | [`metrics`] | What the coach watches: [`RollingTeamMetrics`] and the [`MetricSnapshot`] it is rotated from |
//! | [`match_coach`] | The live [`MatchCoach`] and its evaluate ladder |
//! | [`needs`] | [`TacticalNeed`] — the substitution scorer's read of the same state |
//!
//! Every item the flat `coach` module exported is re-exported below, so
//! `coach::Item` paths (and the engine root's `pub use coach::*`) are
//! unchanged.

pub mod instruction;
pub mod match_coach;
pub mod metrics;
pub mod needs;

pub use instruction::{CoachInstruction, InstructionCoefficients};
pub use match_coach::MatchCoach;
pub use metrics::{MetricSnapshot, RollingTeamMetrics};
pub use needs::TacticalNeed;
