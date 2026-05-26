pub mod ball;
pub mod engine;
pub mod events;
pub mod flow;
pub mod officiating;
pub mod player;
pub mod psychology;
pub mod rating;
pub mod raycast;
pub mod state;
pub mod substitution;
pub mod tactics;
pub mod teamplay;

#[cfg(test)]
mod tests;

pub use ball::*;
pub use engine::*;
pub use raycast::*;
pub use state::*;

// teamplay/ — re-export the modules (preserve `engine::<module>::` paths)
// and the items previously surfaced at the engine root.
pub use teamplay::chemistry::{
    ChemistryInputs, ChemistryMap, ChemistryModifiers, Lane, Role, TacticalFamiliarity,
    chemistry_modifiers, initial_chemistry,
};
pub use teamplay::coach::*;
pub use teamplay::tactical::*;
pub use teamplay::zones::{LateralLane, MatchZone, ZoneCoeffs, ZoneStats};
pub use teamplay::{chemistry, coach, tactical, zones};

// flow/
pub use flow::context::*;
pub use flow::environment::{EnvModifiers, MatchEnvironment, Pitch, Weather};
pub use flow::field::*;
pub use flow::goal::*;
pub use flow::result::*;
pub use flow::{context, environment, field, goal, result};

// officiating/
pub use officiating::management::{
    CounterAttackThreat, HomeAdvantage, HomeAdvantageDeltas, ProfessionalFoul,
    ProfessionalFoulCard, StoppageEvent, StoppageTime, TimeWasting, TimeWastingRestart,
};
pub use officiating::referee::{ContactLocation, FoulCallContext, RefereeProfile};
pub use officiating::set_pieces::{
    CornerRoutine, CornerScores, FreeKickBand, FreeKickChoice, FreeKickChoiceScores,
    ROUTINE_REPEAT_XG_THRESHOLD, SetPieceHistory, TakerScore, ThrowRoutine,
    penalty_conversion_prob, pick_corner_routine, pick_taker, pick_throw_routine,
    score_corner_routines, score_corner_taker, score_free_kick_choices, score_free_kick_taker,
    score_keeper_save, score_penalty_taker, wall_block_prob, wall_size_for,
};
pub use officiating::{management, referee, set_pieces};

// substitution/ — full-path access only (no glob was exported originally).
pub use substitution::{sub_scoring, substitutions};

// rating + psychology — direct children of the engine module.
pub use psychology::{
    NegativeEvent, PositiveEvent, PsychState, Psychology, PsychologyState, SkillModifiers,
    TeamMomentum,
};
pub use rating::*;

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
