// Core staff entity: the Staff struct + collection, its attribute /
// focus / responsibility / position descriptors, context, stub, display,
// and the simulation result.
pub mod model;
// Recruitment / market: the free-agent staff pool and transfer-pipeline
// staff resolution.
pub mod recruitment;
// Staff perception / scouting evaluation.
pub mod perception;
// Persistent coach decision / coach memory system. The lens that
// translates a player's body of work into a coach-aware assessment
// the selection / substitution layers can fold into their scoring.
pub mod coach;
// The goalkeeping department: the one position the whole club shares,
// managed as a group across the first team, the reserves and the
// academy — the keeper room, its declared pecking order, and the
// specialist coach whose advice the manager weighs.
pub mod goalkeeping;
// The manager's mind: shared memory / goal organs, a judgements organ
// of its own, and five faculties over them.
pub mod mind;

pub use coach::*;
// Named rather than globbed: `goalkeeping` carries a `plan` module of its
// own and a glob would make `staff::plan` ambiguous against the coach's.
pub use goalkeeping::{
    GoalkeepingDepartment, GoalkeepingStaff, KeeperAdvice, KeeperAgeCurve, KeeperAssignment,
    KeeperCoachAuthority, KeeperNomination, KeeperRecommendation, KeeperReviewOutcome, KeeperRoom,
    KeeperRoomPlan, KeeperSelectionBrief, KeeperSuccession, KeeperTier, KeeperUrgency, RoomKeeper,
};
pub use model::*;
pub use perception::*;
pub use recruitment::*;
