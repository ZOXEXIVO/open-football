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
pub use strategies::*;
pub use positions::*;

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
