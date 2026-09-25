//! Squad management across every team the club owns.
//!
//! A player's squad is the club's decision, not the team's: the first team's
//! succession problem routinely sits in the under-eighteens, and the
//! under-eighteens' surplus keeper is routinely the reserve side's answer. So
//! the passes here all read the whole club and write back into whichever
//! roster is right — promotion and demotion, the season-start positional
//! trim, the monthly utilization audit, the loan sweep, the goalkeeping
//! department's review of the one queue that runs through every squad, and
//! what the club does with a listed man a whole window would not buy.

mod decision;
pub mod departure;
mod depth;
mod goalkeeping;
mod loans;
mod parked;
mod pathway;
mod promotion;
mod rebalance;
pub mod stranded;
mod trim;
mod utilization;

pub use departure::SquadDepartures;
pub use stranded::{StrandedEffect, StrandedListing, StrandedMarket};
