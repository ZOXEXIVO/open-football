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

pub use coach::*;
pub use model::*;
pub use perception::*;
pub use recruitment::*;
