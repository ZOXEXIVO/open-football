use crate::r#match::{MatchObjectsPositions, StateProcessingContext};

pub mod behaviours;
pub mod context;
pub mod player;
pub mod state;
pub mod statistics;
pub mod strategies;
pub mod positions;
pub mod events;
mod waypoints;

pub use behaviours::*;
pub use context::*;
use itertools::Itertools;
pub use player::*;

// Re-export positions items except conflicting ones
pub use positions::{closure, objects};
pub use positions::ball as position_ball;
pub use positions::players as position_players;

// Re-export strategies items except conflicting ones
pub use strategies::{
    BallOperationsImpl, decision, passing, team,
    common_states, defender_states, processor,
    defenders, forwarders, goalkeepers, midfielders,
};
// Note: strategies re-exports players and ball from common, which conflicts with positions
// We'll use the position_ prefixed versions as the primary ones

pub struct GameFieldContextInput<'p> {
    object_positions: &'p MatchObjectsPositions,
}

impl<'p> GameFieldContextInput<'p> {
    pub fn from_contexts(ctx: &StateProcessingContext<'p>) -> Self {
        GameFieldContextInput {
            object_positions: &ctx.tick_context.positions,
        }
    }

    pub fn to_input(&self) -> Vec<f64> {
        let players_positions: Vec<f64> = self
            .object_positions
            .players
            .items
            .iter()
            .sorted_by_key(|m| m.player_id)
            .flat_map(|p| p.position.as_slice().to_vec())
            .map(|m| m as f64)
            .collect();

        players_positions
    }
}
