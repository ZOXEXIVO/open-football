pub mod ball;
pub mod coach;
pub mod context;
pub mod engine;
pub mod events;
pub mod field;
pub mod goal;
pub mod player;
pub mod rating;
pub mod raycast;
pub mod result;
pub mod state;
pub mod substitutions;
pub mod tactical;
pub mod tactics;

pub use ball::*;
pub use coach::*;
pub use context::*;
pub use engine::*;
pub use field::*;
pub use goal::*;
pub use rating::*;
pub use raycast::*;
pub use result::*;
pub use state::*;
pub use tactical::*;

// Re-export player items except conflicting ones
pub use player::{
    BallOperationsImpl, MatchPlayer, MatchPlayerLite, PlayerSide, behaviours, closure,
    common_states, decision, defender_states, defenders, forwarders, goalkeepers, midfielders,
    objects, passing, team,
};

// Re-export specific types from player submodules that code expects at this level
pub use player::behaviours::SteeringBehavior;
pub use player::context::GameTickContext;
pub use player::positions::{
    GridPlayer, MatchObjectsPositions, PlayerDistanceClosure, PlayerDistanceFromStartPosition,
    SpatialGrid, ball as position_ball, closure as position_closure, objects as position_objects,
    players as position_players,
};
pub use player::strategies::passing::PassEvaluator;
pub use player::strategies::players::{
    PlayerOpponentsOperationsImpl, PlayerTeammatesOperationsImpl,
};
pub use player::strategies::processor::{
    ConditionContext, StateChangeResult, StateProcessingContext, StateProcessingHandler,
    StateProcessingResult, StateProcessor,
};
// Export modules for those who want to access them
pub use player::context as player_context;
pub use player::positions as player_positions;
pub use player::strategies::processor;
// Note: player::events conflicts with engine::events module, so we don't re-export it

// Re-export tactics items except conflicting ones
pub use tactics::field as tactics_field;
pub use tactics::field::{POSITION_POSITIONING, PositionType};
pub use tactics::positions as tactics_positions;
