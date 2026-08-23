pub mod ability;
pub mod bias;

pub mod potential;
pub mod profile;
pub mod utils;

// Re-export key types at module level
pub use ability::{AbilityEstimator, DevelopmentFormEvidence};
pub use bias::{PlayerBias, PlayerImpression, RecentMove, RecentMoveType};
pub use potential::{EstimationContext, PotentialEstimate, PotentialEstimator};
pub use profile::{CoachProfile, PerceptionLens};
pub use utils::{date_to_week, seeded_decision, sigmoid_probability};

// CoachDecisionState is defined here (the "state" module) because it orchestrates
// evaluation + bias + profile. Evaluation methods are in evaluation.rs as `impl CoachDecisionState`.

// `CoachDecisionState` — the coach's running impressions of the squad —
// moved under the judgements organ, beside `CoachMemory`. Re-exported so
// every `staff::perception::CoachDecisionState` path (and the `state`
// alias `evaluation.rs` reaches through) resolves unchanged.
pub use crate::club::staff::mind::organs::judgements::impressions::CoachDecisionState;

pub(crate) use utils::perception_noise_raw;
