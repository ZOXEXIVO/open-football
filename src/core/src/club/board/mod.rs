//! The boardroom.
//!
//! One actor per club, running on the club's daily tick. It reads a
//! [`context::BoardContext`] snapshot of the club's money, results, squad
//! and facilities, and answers with a [`result::BoardResult`] the club
//! applies afterwards — the board never mutates the club itself, which is
//! what keeps the daily pass parallel-safe and makes every boardroom effect
//! auditable as a [`decision::BoardDecision`].
//!
//! One submodule per concern:
//!
//! * [`core`] — the board itself and the passes it runs
//! * [`vision`] — the brief the manager is judged against
//! * [`chairman`] — the temperament in the chair
//! * [`ownership`] — who owns the club and how they exercise power
//! * [`targets`] — the season mandate and the budget arithmetic behind it
//! * [`governance`] — the transfer hearing
//! * [`manager`] — the seat, the search, and the market for head coaches
//! * [`scoring`] — the four component scores of a monthly review
//! * [`pressure`] — supporter, media, dressing-room and regulatory heat
//! * [`relationship`] — the five facets of board-manager trust
//! * [`promise`] — what the board pledged and whether it delivered
//! * [`infrastructure`] — the yearly facility review
//! * [`sale`] — the board telling the manager somebody has to go
//! * [`severance`] — what it costs to end a manager's deal early
//! * [`takeover`] — rare ownership change
//! * [`strategy`] — how the board wants the club run, beyond the vision
//! * [`decision`] — the explainable decisions the board emits
//! * [`roll`] — reproducible chance, for a board with no RNG

pub mod chairman;
pub mod context;
pub mod core;
pub mod decision;
pub mod governance;
pub mod infrastructure;
pub mod manager;
mod mood;
pub mod ownership;
pub mod pressure;
pub mod promise;
pub mod relationship;
mod result;
pub mod roll;
pub mod sale;
pub mod scoring;
pub mod severance;
pub mod strategy;
pub mod takeover;
pub mod targets;
pub mod vision;

pub use chairman::*;
pub use context::*;
pub use core::*;
pub use decision::*;
pub use governance::*;
pub use infrastructure::*;
pub use manager::*;
pub use mood::*;
pub use ownership::*;
pub use pressure::*;
pub use promise::*;
pub use relationship::*;
pub use result::*;
pub use roll::*;
pub use sale::*;
pub use scoring::*;
pub use severance::*;
pub use strategy::*;
pub use takeover::*;
pub use targets::*;
pub use vision::*;
