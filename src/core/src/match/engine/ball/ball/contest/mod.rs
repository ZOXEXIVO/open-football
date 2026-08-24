//! **Players intervening on a ball that is already moving** — the three
//! ways a flight is ended by somebody other than its intended receiver,
//! and the ledger of who ended up with it.
//!
//! Each resolver runs only on unowned balls with `in_flight_state > 0`,
//! so routine possession play is never disturbed. They differ in reach
//! and in what they are aimed at:
//!
//! * [`interception`] — ≤ 2.5u, pass-targeted, a tiny per-tick chance.
//! * [`block`] — ≤ 4u, shot-targeted, a higher per-event chance.
//! * [`save`] — the keeper, and [`SaveModel`](save::SaveModel), the
//!   shot-stopping curve the live path and the spread regression share.
//!
//! Alongside them:
//!
//! * [`body`] — the keeper's own volume: not a contest at all, but the
//!   statement that a ball arriving where he already is comes off him.
//! * [`contact`] — the arm that decides whether a resolved contact
//!   writes the ball onto the man or leaves it where it was.
//! * [`ownership`] — somebody claims it instead: pass-target claims, the
//!   reception and first-touch flow, deadlock resolution and the
//!   unowned-too-long safety nets.
//! * [`possession`] — the bookkeeping that outlives the contest: how the
//!   carrier came by the ball ([`PossessionSource`]), the pass chain the
//!   assist resolver walks, and the giveaway / shot / carry metadata.

pub mod block;
pub mod body;
pub mod contact;
pub mod interception;
pub mod ownership;
pub mod possession;
pub mod save;

pub use contact::ContactInPlace;
pub use possession::{PassChainEntry, PossessionSource};
