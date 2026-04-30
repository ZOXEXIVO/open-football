use crate::r#match::{MatchObjectsPositions, StateProcessingContext};

pub mod behaviours;
pub mod context;
pub mod events;
pub mod memory;
pub mod player;
pub mod positions;
pub mod state;
pub mod statistics;
pub mod strategies;
mod waypoints;

pub use behaviours::*;
pub use context::*;
use itertools::Itertools;
pub use player::*;

// Re-export positions items except conflicting ones
pub use positions::ball as position_ball;
pub use positions::players as position_players;
pub use positions::{closure, objects};

// Re-export strategies items except conflicting ones
pub use strategies::{
    BallOperationsImpl, common_states, decision, defender_states, defenders, forwarders,
    goalkeepers, midfielders, passing, processor, team,
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
            .as_slice()
            .iter()
            .sorted_by_key(|m| m.player_id)
            .flat_map(|p| p.position.as_slice().to_vec())
            .map(|m| m as f64)
            .collect();

        players_positions
    }
}
