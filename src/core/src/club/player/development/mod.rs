//! Player skill development system.
//!
//! Key principles:
//! 1. Position-aware: skills relevant to the player's position develop faster
//!    and have a higher ceiling. Irrelevant skills stay low.
//! 2. Age curve: physical skills peak 24-28, decline from ~30; mental skills
//!    can grow into the 30s; technical skills plateau in the late 20s.
//! 3. Personality: professionalism, ambition, determination drive growth rate.
//! 4. Match experience: playing competitive matches accelerates development.
//! 5. Potential ceiling: PA gates maximum achievable level; per-skill ceilings
//!    based on PA x position weight create realistic skill profiles.
//! 6. Workload: tired, jaded or injured players don't grow normally.
//!
//! ## Testing seam
//!
//! The public entry point [`Player::process_development`] uses the global
//! thread-local RNG, which makes results irreproducible. The internal
//! [`Player::process_development_with`] variant accepts any [`RollSource`]
//! so tests can drive a deterministic stream of rolls and assert on stable
//! outputs.
//!
//! ## Submodule layout
//!
//! - [`skills_array`]: `SkillKey`, flat-array indexes, `SkillCategory`,
//!   and the round-trip helpers between `Player.skills` and `[f32; 50]`.
//! - [`position_weights`]: position grouping and per-position skill weights
//!   that drive both ceilings and growth rates.
//! - [`age_curve`]: age-band growth/decline rates and per-skill peak offsets.
//! - [`modifiers`]: independent multipliers (personality, match experience,
//!   workload, etc.) and the [`FitnessState`] gate.
//! - [`rolls`]: deterministic [`RollSource`] seam used by the tick.
//! - [`coaching`]: [`CoachingEffect`] — per-category coach effectiveness.
//! - [`tick`]: weekly development tick; wires everything together into the
//!   `Player::process_development[_with]` impls.
//!
//! [`Player::process_development`]: crate::club::player::Player::process_development
//! [`Player::process_development_with`]: crate::club::player::Player::process_development_with

mod age_curve;
mod coaching;
mod modifiers;
mod position_weights;
mod rolls;
mod skills_array;
mod tick;

pub use coaching::CoachingEffect;
pub use modifiers::FitnessState;
pub use rolls::{FixedRolls, RollSource, ThreadRolls};
pub use skills_array::{SkillCategory, SkillKey};

#[cfg(test)]
mod tests;
