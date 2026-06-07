//! Coach decision / coach memory system.
//!
//! Encapsulates the persistent coach lens previously scattered across
//! squad selection, substitutions, player happiness, morale, and
//! training. Splits cleanly into four concepts:
//!
//! * [`memory`] — persistent per-coach record of player observations.
//! * [`strategy`] — high-level approach the coach takes to a fixture.
//! * [`reason`] — closed catalog of structured decision reasons.
//! * [`assessment`] — computed read of one player at one decision point.
//! * [`engine`] — the [`CoachDecisionEngine`] service the selection /
//!   substitution layers consume.
//!
//! The engine never replaces the existing scoring engine — it returns
//! small signed adjustments and structured reasons the caller folds
//! into its own decision. This makes the system additive and removable:
//! turning the coach engine off would collapse the assessment to a
//! neutral 0.0 adjustment and leave the existing scoring intact.

pub mod assessment;
pub mod bond;
pub mod engine;
pub mod memory;
pub mod reason;
pub mod snapshot;
pub mod strategy;

#[cfg(test)]
mod tests;

pub use assessment::{CoachDecisionScore, CoachPlayerAssessment};
pub use bond::{CoachPlayerBond, CoachPlayerBondBreakdown};
pub use engine::{CoachDecisionEngine, CoachLiveMatchContext, CoachSelectionContext};
pub use memory::{
    CoachMatchObservation, CoachMemory, CoachMemoryFlags, CoachMemoryStore, MemoryEngine,
};
pub use reason::CoachDecisionReason;
pub use snapshot::CoachMatchSnapshot;
pub use strategy::{CoachStrategy, StrategyDeriver, StrategyInputs};
