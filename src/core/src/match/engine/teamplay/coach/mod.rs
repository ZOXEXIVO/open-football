//! **The coach who manages each side.** One [`MatchCoach`] per side holds
//! the man in the dugout ([`CoachCard`]), the plan he sets at kickoff
//! ([`GamePlan`]), how much the side should commit for the score
//! ([`Stance`]), his moving read of the run of play ([`CoachRead`]) and what
//! he did ([`CoachRecord`]).
//!
//! The stance moves the side through the plan's levers at every minute,
//! from the kickoff stance. At the moments coaches act — half-time, a goal,
//! a sending-off, a forced change, a read that says the plan is failing — he
//! changes the plan to answer how the side is playing against what he
//! expected of it, and may change the formation; at the interval he also
//! gives his talk.
//!
//! | Module | Concern |
//! |---|---|
//! | [`card`] | [`CoachCard`] — his temperament and the football he prefers |
//! | [`plan`] | [`GamePlan`] — the seven dials a side plays to, fitted to what the eleven can do |
//! | [`stance`] | [`Stance`] — how much the side should commit, from what the next goal is worth |
//! | [`metrics`] | What he watches: the moving [`CoachRead`] and the [`RollingTeamMetrics`] it gives |
//! | [`match_coach`] | The live [`MatchCoach`]: his reading, his moments and his decisions |
//! | [`needs`] | [`TacticalNeed`] — the bench's read of the same match |
//! | [`pins`] | [`TeamPins`] — the harness's stated coach and plan |
//! | [`record`] | [`CoachRecord`] — what he did, handed back on the result |

pub mod card;
pub mod match_coach;
pub mod metrics;
pub mod needs;
pub mod pins;
pub mod plan;
pub mod record;
pub mod stance;

pub use card::{CoachCard, Temperament};
pub use match_coach::{MatchCoach, ShapeChoice};
pub use metrics::{CoachRead, ReadCounters, RollingTeamMetrics};
pub use needs::TacticalNeed;
pub use pins::TeamPins;
pub use plan::{GamePlan, SquadFit};
pub use record::{ChangeMoment, CoachRecord, PlanChange};
pub use stance::{Occasion, Outlook, ScoringRates, Situation, Stance};
