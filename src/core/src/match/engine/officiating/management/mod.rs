//! Game-management helpers: professional fouls, time-wasting, dissent.
//!
//! Each concern lives in its own submodule and exposes a logical struct
//! whose associated functions replace the old free functions. Pure
//! helpers; no engine state mutation. Returned values are probabilities
//! or millisecond deltas the caller folds into the existing dispatcher /
//! referee logic.

pub mod dissent;
pub mod professional_foul;
pub mod time_wasting;

pub use dissent::Dissent;
pub use professional_foul::{CounterAttackThreat, ProfessionalFoul, ProfessionalFoulCard};
pub use time_wasting::{TimeWasting, TimeWastingLedger, TimeWastingRestart};
