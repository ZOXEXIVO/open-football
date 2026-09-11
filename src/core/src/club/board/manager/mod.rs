//! The manager market — the seat, the search, the shortlist, and the
//! poaching of managers already in work.
//!
//! When a club's seat opens up (the board sacks the incumbent, the
//! incumbent walks, or a contract lapses), `ClubBoard.manager_search_since`
//! is set to today. From that tick onward the world-level manager-market
//! phase refreshes the club's `manager_shortlist` once a week, ranking the
//! candidates the board is willing to consider.
//!
//! One submodule per concern, and every helper is a method on the type that
//! owns it:
//!
//! * [`candidate`] — what a candidate is and where it came from
//! * [`seat`] — head-coach-seat operations on a main team (caretakers,
//!   dismissals, the one path by which a seat is vacated)
//! * [`repair`] — the daily invariant pass that guarantees every active
//!   team has somebody picking it
//! * [`search`] — the board's own search state
//! * [`scorer`] — pure scoring and acceptance predicates
//! * [`shortlist`] — building the free-agent and employed shortlists
//! * [`market`] — the daily orchestrator and the appointment phase
//! * [`approach`] — the poaching state machine

pub mod approach;
pub mod candidate;
pub mod market;
pub mod repair;
pub mod scorer;
pub mod search;
pub mod seat;
pub mod shortlist;

#[cfg(test)]
mod tests;

pub use approach::{ApproachState, ManagerApproach};
pub use candidate::{CandidateSource, ManagerCandidate};
pub use market::ManagerMarketTick;
pub use repair::ManagerSeatRepair;
pub use scorer::ManagerCandidateScorer;
pub use search::ManagerSearch;
pub use seat::ManagerSeat;
pub use shortlist::ManagerShortlist;
