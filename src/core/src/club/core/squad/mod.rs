//! Squad management across every team the club owns.
//!
//! A player's squad is the club's decision, not the team's: the first team's
//! succession problem routinely sits in the under-eighteens, and the
//! under-eighteens' surplus keeper is routinely the reserve side's answer. So
//! the passes here all read the whole club and write back into whichever
//! roster is right — promotion and demotion, the season-start positional
//! trim, the monthly utilization audit, the loan sweep, and the goalkeeping
//! department's review of the one queue that runs through every squad.

mod decision;
pub mod departure;
mod depth;
mod goalkeeping;
mod loans;
mod parked;
mod promotion;
mod rebalance;
mod trim;
mod utilization;

pub use departure::SquadDepartures;
