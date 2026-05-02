pub mod ball;
pub mod chemistry;
pub mod coach;
pub mod context;
pub mod engine;
pub mod environment;
pub mod events;
pub mod game_management;
pub mod field;
pub mod goal;
pub mod player;
pub mod rating;
pub mod psychology;
pub mod referee;
pub mod set_pieces;
pub mod raycast;
pub mod result;
pub mod state;
pub mod sub_scoring;
pub mod substitutions;

#[cfg(test)]
mod intelligence_tests;
#[cfg(test)]
mod match_realism_tests;
pub mod tactical;
pub mod tactics;

pub use ball::*;
pub use coach::*;
pub use context::*;
pub use engine::*;
pub use chemistry::{
    ChemistryInputs, ChemistryMap, ChemistryModifiers, Lane, Role, TacticalFamiliarity,
    chemistry_modifiers, initial_chemistry,
};
pub use environment::{EnvModifiers, MatchEnvironment, Pitch, Weather};
pub use game_management::{
    CounterAttackThreat, HomeAdvantageDeltas, ProfessionalFoulCard, StoppageEvent,
    TimeWastingRestart, home_advantage_deltas, professional_foul_prob, professional_foul_red_prob,
    stoppage_for, time_wasting_delay_ms, time_wasting_yellow_prob,
};
pub use field::*;
pub use goal::*;
pub use rating::*;
pub use psychology::{
    NegativeEvent, PositiveEvent, PsychState, PsychologyState, SkillModifiers, TeamMomentum,
    confidence_delta_negative, confidence_delta_positive, initial_confidence, initial_nervousness,
    keeper_communication_score, leadership_damped_momentum, pressure_load, skill_modifiers,
    team_leadership_score,
};
pub use referee::{ContactLocation, FoulCallContext, RefereeProfile};
pub use set_pieces::{
    CornerRoutine, CornerScores, FreeKickBand, FreeKickChoice, FreeKickChoiceScores,
    ROUTINE_REPEAT_XG_THRESHOLD, SetPieceHistory, TakerScore, ThrowRoutine,
    penalty_conversion_prob, pick_corner_routine, pick_taker, pick_throw_routine,
    score_corner_routines, score_corner_taker, score_free_kick_choices, score_free_kick_taker,
    score_keeper_save, score_penalty_taker, wall_block_prob, wall_size_for,
};
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
