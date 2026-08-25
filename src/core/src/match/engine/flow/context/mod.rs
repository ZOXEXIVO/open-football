//! **The world one match carries around with it.**
//!
//! [`MatchContext`] is the single mutable value threaded through every
//! engine pass — clock, score, squads, plans, shapes, coaches,
//! psychology, referee, and the RNG. It is big because a match IS big;
//! what is split here is not the state but the passes over it, so each
//! file holds one question the context is asked:
//!
//! * [`match_context`] — the struct itself, its construction from
//!   [`MatchEngineConfig`], and the per-side lookups
//!   (`attack_plan_for_team`, `coach_for_team`, …) that pick home or away.
//! * [`config`] — [`MatchEngineConfig`]: seed, date, weather, referee,
//!   knockout/friendly. Everything a caller needs to get a SPECIFIC
//!   match rather than the neutral default one.
//! * [`clock`] — the 10 ms tick, period ends, stoppage time, and the
//!   goal/concede timestamps "recently" is measured against.
//! * [`score_reaction`] — when and how hard teams are allowed to react to
//!   the scoreline, plus the four `OF_*` A/B switches. Read once per
//!   process, not per match.
//! * [`substitution_record`] — the live swap ledger and its budget.
//! * [`penalty_area`] — the box as a rectangle, scaled to the pitch.
//! * [`rng`] — [`MatchRng`](rng::MatchRng), the match-owned seeded
//!   stream. It belongs here because the context OWNS it: one match, one
//!   stream, and determinism is a property of this struct's lifetime.
//!
//! Every item is re-exported below, so `flow::context::Item` resolves as
//! it did when this was one file — which matters because the engine root
//! globs this module.

pub mod clock;
pub mod config;
pub mod match_context;
pub mod penalty_area;
pub mod rng;
pub mod score_reaction;
pub mod substitution_record;

pub use clock::MATCH_TIME_INCREMENT_MS;
pub use config::MatchEngineConfig;
pub use match_context::{MatchContext, PendingAdvantage};
pub use penalty_area::PenaltyArea;
pub use substitution_record::SubstitutionRecord;
