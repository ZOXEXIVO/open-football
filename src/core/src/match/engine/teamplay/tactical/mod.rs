//! **The team-level tactical state** every player of a side reads: the phase
//! of play, the line, the press, the width, the tempo, the risk on the ball
//! and the men kept back. Refreshed periodically from the ball and
//! possession, the side's [`GamePlan`](super::coach::GamePlan) and the
//! coach's stance, and consulted by the player states when they branch.
//!
//! | Module | Concern |
//! |---|---|
//! | [`phase`] | How the game is read: [`GamePhase`], [`BallZone`], [`BallSideZone`], and the phase / transition-window math |
//! | [`inputs`] | What `refresh` is handed: [`TacticalRefreshInputs`] and [`TeamSkillAggregates`] |
//! | [`team_state`] | [`TeamTacticalState`] itself and the `refresh` pass that fills it |
//! | [`signals`] | The plan and stance math behind every scalar on it |
//! | [`quality`] | The team-quality corrections `refresh` lays on top of those signals |

pub mod inputs;
pub mod phase;
pub mod quality;
pub mod signals;
pub mod team_state;

pub use inputs::{TacticalRefreshInputs, TeamSkillAggregates};
pub use phase::{BallSideZone, BallZone, GamePhase};
pub use team_state::TeamTacticalState;
