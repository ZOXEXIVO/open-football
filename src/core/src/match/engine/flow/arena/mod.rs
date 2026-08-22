//! **The stage the match is played on**, and the two things that put the
//! ball back on it.
//!
//! One group because they are the physical facts a tick is resolved
//! against — none of them decides anything, they only say where things
//! are and what the surface does to them:
//!
//! * [`environment`] — the conditions: weather, pitch, crowd,
//!   importance, and the deterministic [`EnvModifiers`](environment::EnvModifiers)
//!   they hand the passing / handling / injury paths.
//! * [`field`] — [`MatchField`](field::MatchField): the twenty-two, the
//!   bench, the ball, the two dugouts, and the formation reset every
//!   restart writes ([`ResetReason`](field::ResetReason)).
//! * [`goal`] — the frame at each end. The goal-line test, and the
//!   kickoff/restart bookkeeping that follows a ball crossing it.
//!
//! `goal` reaches into `field` and the celebration reaches back into
//! `goal`, which is the reason the three sit together rather than under
//! whatever consumes them.
//!
//! Each module is re-exported from [`flow`](super)
//! under the name it had before the grouping, so `flow::environment`,
//! `flow::field` and `flow::goal` keep resolving.

pub mod environment;
pub mod field;
pub mod goal;
