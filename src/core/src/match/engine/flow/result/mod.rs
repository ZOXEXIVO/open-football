//! **What comes out of a match**, split by the thing each part
//! describes. Everything here is data plus the small amount of logic
//! that decides what data survives — nothing in this group simulates
//! anything, and every type is on the wire between the engine, the
//! worker and the league pipeline.
//!
//! | Module            | Concern                                                        |
//! |-------------------|----------------------------------------------------------------|
//! | [`score`]         | The two tallies, the shootout, and the outcome they add up to   |
//! | [`highlights`]    | The goals, the near misses, and which near misses reach the sheet |
//! | [`player_stats`]  | One player's stat line and physical state when he left the pitch |
//! | [`substitution`]  | The record of a swap, and why it fired                          |
//! | [`raw`]           | The whole engine payload and the league-facing wrapper around it |
//!
//! Every item is re-exported below, so `flow::result::Item` resolves
//! exactly as it did when this was one file — which the engine root
//! globs and the league pipeline names directly.

pub mod highlights;
pub mod player_stats;
pub mod raw;
pub mod score;
pub mod substitution;

pub use highlights::{ChanceDetail, GoalDetail, HighlightSelector};
pub use player_stats::{PlayerMatchEndStats, PlayerMatchPhysicalSnapshot};
pub use raw::{FieldSquad, MatchResult, MatchResultRaw, PenaltyShootoutKick};
pub use score::{MatchResultOutcome, Score, TeamScore};
pub use substitution::{SubstitutionInfo, SubstitutionReason};
