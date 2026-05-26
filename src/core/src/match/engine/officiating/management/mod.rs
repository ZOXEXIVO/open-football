//! Game-management helpers: professional fouls, time-wasting,
//! stoppage-time accounting, home-advantage modulation.
//!
//! Each concern lives in its own submodule and exposes a logical struct
//! whose associated functions replace the old free functions. Pure
//! helpers; no engine state mutation. Returned values are probabilities
//! or millisecond deltas the caller folds into the existing dispatcher /
//! referee logic.

pub mod home_advantage;
pub mod professional_foul;
pub mod stoppage;
pub mod time_wasting;

pub use home_advantage::{HomeAdvantage, HomeAdvantageDeltas};
pub use professional_foul::{CounterAttackThreat, ProfessionalFoul, ProfessionalFoulCard};
pub use stoppage::{StoppageEvent, StoppageTime};
pub use time_wasting::{TimeWasting, TimeWastingRestart};
