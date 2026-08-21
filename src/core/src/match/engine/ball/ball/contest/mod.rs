//! **Players intervening on a ball that is already moving** — the two
//! halves of a contested ball, and the ledger of who ended up with it.
//!
//! * [`interactions`] — somebody gets in the way: interception duels,
//!   shot blocks, and the keeper's save model.
//! * [`ownership`] — somebody claims it: pass-target claims, the
//!   reception and first-touch flow, deadlock resolution and the
//!   unowned-too-long safety nets.
//! * [`possession`] — the bookkeeping that outlives the contest: how the
//!   carrier came by the ball ([`PossessionSource`]), the pass chain the
//!   assist resolver walks, and the giveaway / shot / carry metadata.

pub mod interactions;
pub mod ownership;
pub mod possession;

pub use possession::{PassChainEntry, PossessionSource};
