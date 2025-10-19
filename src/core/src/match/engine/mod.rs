pub mod ball;
pub mod engine;
pub mod events;
pub mod field;
pub mod player;
pub mod raycast;
pub mod result;
pub mod state;
pub mod tactics;

pub use ball::*;
pub use engine::*;
pub use field::*;
pub use raycast::*;
pub use result::*;
pub use state::*;

// Re-export player items except conflicting ones
pub use player::{
    behaviours, closure, decision, objects, passing, team,
    common_states, defender_states,
    defenders, forwarders, goalkeepers, midfielders,
    BallOperationsImpl,
    MatchPlayer, PlayerSide, MatchPlayerLite,
};

// Re-export specific types from player submodules that code expects at this level
pub use player::context::GameTickContext;
pub use player::behaviours::SteeringBehavior;
pub use player::positions::{
    MatchObjectsPositions, PlayerDistanceClosure, PlayerDistanceFromStartPosition,
    closure as position_closure, objects as position_objects,
    ball as position_ball, players as position_players,
};
pub use player::strategies::players::{
    PlayerOpponentsOperationsImpl, PlayerTeammatesOperationsImpl,
};
pub use player::strategies::passing::PassEvaluator;
pub use player::strategies::processor::{
    StateProcessingContext, StateProcessingResult, StateProcessor,
    StateChangeResult, StateProcessingHandler, ConditionContext,
};
// Export modules for those who want to access them
pub use player::context as player_context;
pub use player::positions as player_positions;
pub use player::strategies::processor;
// Note: player::events conflicts with engine::events module, so we don't re-export it

// Re-export tactics items except conflicting ones
pub use tactics::field::{PositionType, POSITION_POSITIONING};
pub use tactics::field as tactics_field;
pub use tactics::positions as tactics_positions;
