//! **Properties of the two squads that turned up** — read at kickoff and
//! then held, as opposed to the ball-driven state in
//! [`plans`](super::plans) and [`tactical`](super::tactical) that is
//! recomputed every tick.
//!
//! * [`chemistry`] — how well these particular players know each other:
//!   pair chemistry seeded from the roster, plus the team-level
//!   [`TacticalFamiliarity`](chemistry::TacticalFamiliarity). Consulted
//!   by the pass evaluator and the pressing / offside sites.
//! * [`standard`] — the standard of football in this fixture, and its
//!   distance from the division the engine's constants were fitted in.
//!   Latched at kickoff for the same reason: it is a property of the two
//!   squads, not of how tired they are in the 80th minute.
//!
//! Both are cheap lookups the rest of the engine prices decisions
//! against — neither is refreshed from the ball, and neither belongs to
//! a single side's plan.

pub mod chemistry;
pub mod standard;
