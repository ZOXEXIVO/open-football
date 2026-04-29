//! Cross-cutting Player event handlers — the canonical home for the
//! `Player::on_*` / `complete_*` methods that mutate the player in
//! response to something happening *to* them. Carved into per-domain
//! submodules so each concern is small and self-explanatory:
//!
//! | Submodule          | Concern                                                   |
//! |--------------------|-----------------------------------------------------------|
//! | [`types`]          | Free types: `MatchOutcome`, `MatchParticipation`, transfer/loan completions |
//! | [`scaling`]        | Personality-aware multipliers (reputation, ambition, …)    |
//! | [`match_play`]     | Stats / morale / reputation effects of a finished match    |
//! | [`match_exertion`] | Physical exertion: load, jadedness, post-match injury roll |
//! | [`role`]           | Starter-share tracking + season-event scaling factors      |
//! | [`career`]         | Youth breakthrough + team-level season events              |
//! | [`transfer`]       | Permanent / loan / free-agent contract installation        |
//! | [`transfer_social`]| Social fallout: bid rejection, dream-move, friend sold     |
//!
//! ## Public surface
//!
//! Existing call sites import the four free types
//! (`MatchOutcome`, `MatchParticipation`, `TransferCompletion`,
//! `LoanCompletion`) directly via `events::*` and the magnitude
//! helpers via `events::scaling::*`. Both paths are preserved by
//! re-exports below — no caller changes needed.

pub mod career;
pub mod match_exertion;
pub mod match_play;
pub mod role;
pub mod scaling;
pub mod transfer;
pub mod transfer_social;
pub mod types;

pub use types::{LoanCompletion, MatchOutcome, MatchParticipation, TransferCompletion};

#[cfg(test)]
mod tests;
