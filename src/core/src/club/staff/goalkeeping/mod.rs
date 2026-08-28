//! The goalkeeping department.
//!
//! One position at a football club is shared by the whole club: there is a
//! single shirt, and every keeper from the first team to the
//! under-eighteens is queuing for it. That is why real clubs employ
//! somebody whose entire job is the queue — who trains all of them, ranks
//! all of them, and tells the manager which one to play and when to give
//! the boy his debut.
//!
//! The sim had the role in its staff enum and nothing behind it. Keepers
//! were picked by an argmax over ability that re-ran every Saturday, the
//! academy keepers were invisible to the first team except as emergency
//! cover, and no part of the club ever asked who would be in goal in three
//! years' time.
//!
//! * [`room`] — the census: every keeper at the club, read from what a
//!   coach can see, on a keeper's own age curve rather than an outfield one.
//! * [`advice`] — the declared hierarchy, and the closed catalogue of
//!   things a goalkeeping coach says.
//! * [`plan`] — the standing plan, held on the man whose opinion it is.
//! * [`department`] — his standing with the manager, and the monthly review.
//! * [`brief`] — the four names and one number the team sheet needs.

pub mod advice;
pub mod brief;
pub mod department;
pub mod plan;
pub mod room;

#[cfg(test)]
mod tests;

pub use advice::{KeeperAdvice, KeeperRecommendation, KeeperSuccession, KeeperTier, KeeperUrgency};
pub use brief::KeeperSelectionBrief;
pub use department::{GoalkeepingDepartment, GoalkeepingStaff, KeeperCoachAuthority};
pub use plan::{KeeperAssignment, KeeperNomination, KeeperReviewOutcome, KeeperRoomPlan};
pub use room::{KeeperAgeCurve, KeeperRoom, RoomKeeper};
