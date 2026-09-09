//! The transfer system.
//!
//! Eleven folders, each owning one thing:
//!
//! * [`market`]  — the market's own state, its geography and its calendar.
//! * [`deal`]    — one transaction: the offer, the negotiation, the record.
//! * [`value`]   — what a player, a signing and a wage are worth.
//! * [`gate`]    — may this move happen at all.
//! * [`squad`]   — what a club needs, and what it will part with.
//! * [`scouting`]— who the club knows about, and how it came to know.
//! * [`loan`]    — the loan market, end to end.
//! * [`pool`]    — the free-agent pool, as the market sees it.
//! * [`view`]    — read-only projections of the world.
//! * [`pipeline`]— the passes that run each tick.
//! * `tests`     — fixtures and the layering guard (test-only).
//!
//! Direction of dependency: `pipeline` drives everything; the subsystem
//! folders read the world and answer questions; nothing here reaches back
//! into `country::result`. That last rule is asserted, not remembered —
//! see `tests::layering`.

pub mod deal;
pub mod gate;
pub mod loan;
pub mod market;
pub mod pipeline;
pub mod pool;
pub mod scouting;
pub mod squad;
pub mod value;
pub(crate) mod view;

#[cfg(test)]
pub(crate) mod tests;

pub use deal::*;
pub use gate::*;
pub use loan::*;
pub use market::*;
pub use pipeline::*;
pub use pool::*;
pub use scouting::*;
pub use squad::*;
pub use value::*;
