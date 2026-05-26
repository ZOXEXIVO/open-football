// Core team data model: the Team struct, its builder/collection, the
// per-team context, the simulation result, and the team-type taxonomy.
pub mod model;
// Squad-life helpers: captaincy, mentorship, squad-status, and the
// derived social/chemistry views.
pub mod squad_life;
// Fitness & scheduling: fixture windows and preventive-rest passes.
pub mod fitness;
// Reputation domain: scores, achievements, competition types.
pub mod reputation;
// Team / dressing-room talks.
pub mod talks;

// Established domain folders.
pub mod behaviour;
pub mod matches;
pub mod squad;
pub mod tactics;
pub mod training;
pub mod transfers;

pub use fitness::*;
pub use model::*;
pub use reputation::*;
pub use squad_life::*;
pub use talks::*;

pub use behaviour::*;
pub use matches::*;
pub use squad::*;
pub use tactics::*;
pub use training::*;
pub use transfers::*;
