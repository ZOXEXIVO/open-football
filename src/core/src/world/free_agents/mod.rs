//! The unattached pool and the money that follows a player out of a
//! squad.
//!
//! Three passes, all world-level because a club and the player who just
//! left it can sit in different countries:
//!
//! * [`intake`] — sweeping contractless players off rosters into
//!   `data.free_agents`.
//! * [`retirement`] — the monthly roll that resolves a long sit in the
//!   pool.
//! * [`loan_wages`] — the parent club's residual share of a loanee's
//!   primary contract.

mod intake;
mod loan_wages;
mod retirement;

pub use loan_wages::LoanWageSettlement;
