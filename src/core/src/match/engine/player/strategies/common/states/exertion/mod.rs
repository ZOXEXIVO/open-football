//! **How hard a player is working, what it costs him, and what it lets
//! him do.**
//!
//! One loop with two directions, which is why these four live together:
//!
//! * [`activity_intensity`] — [`ActivityIntensity`] is the tier a state
//!   declares each tick (`VeryHigh` … `Recovery`), plus the per-role
//!   [`ActivityIntensityConfig`] that prices each tier.
//! * [`constants`] — the condition scale and the rates: the 15% match
//!   floor, `FATIGUE_RATE_MULTIPLIER`, recovery, jadedness.
//! * [`condition`] — the down direction. [`ConditionProcessor`] spends
//!   condition at the declared tier and recovers it.
//! * [`movement_effort`] — the up direction. [`MovementEffort`] turns
//!   the same tier into a **hard speed ceiling**, shaded down by
//!   tiredness for self-pacing.
//!
//! ⚠ Because the tier is a speed cap and not a cost alone, a state that
//! declares the wrong tier does not merely mis-bill fatigue — it loses
//! every race it enters. See [`ActivityIntensity::chase`].

pub mod activity_intensity;
pub mod condition;
pub mod constants;
pub mod movement_effort;

pub use activity_intensity::*;
pub use condition::*;
pub use constants::*;
pub use movement_effort::*;
