//! Read-only projections of the world, for the market to read.
//!
//! A view answers a question about the world as it stands — how full is
//! this squad, what is this club worth, where does this player rank. It
//! never decides anything: the thresholds and the gates live in [`gate`],
//! [`value`] and [`squad`].
//!
//! [`gate`]: crate::transfers::gate
//! [`value`]: crate::transfers::value
//! [`squad`]: crate::transfers::squad

pub mod club;
pub mod player;
pub mod world;
