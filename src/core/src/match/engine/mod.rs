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

// ── Flat engine surface ────────────────────────────────────────────────────
//
// The engine root publishes exactly the names the rest of `core` and the
// development harnesses reach through it. The grouped modules stay reachable
// by their own paths; nothing is re-exported by accident.

// ball/ — the ball model's root.
pub(crate) use ball::{Ball, MAX_OWNER_TRACK_DISTANCE, OffsideLine, OffsideSnapshot, ShotTarget};
pub use ball::{GoalOrigin, PassOriginRestart};

// engine/ — the simulation entry point and its standalone value types.
pub use engine::FootballEngine;
pub(crate) use engine::{
    BallSide, MATCH_EXTRA_TIME_MS, MATCH_HALF_TIME_MS, MATCH_TIME_MS, MatchFieldSize,
    MatchPlayerCollection, MatchTime, TeamsTactics,
};
pub(crate) use raycast::Space;

// state/ — the match clock's state machine.
pub(crate) use state::{MatchState, StateManager};

// teamplay/ — re-export the modules (preserve `engine::<module>::` paths)
// and the items previously surfaced at the engine root.
pub use teamplay::adherence::Adherence;
pub use teamplay::attack::{AttackPlan, AttackRefreshInputs, BoxSlot};
pub use teamplay::chemistry::{
    ChemistryInputs, ChemistryMap, ChemistryModifiers, Lane, Role, TacticalFamiliarity,
    chemistry_modifiers, initial_chemistry,
};
pub use teamplay::coach::{
    ChangeMoment, CoachCard, CoachRead, CoachRecord, GamePlan, MatchCoach, Occasion, Outlook,
    PlanChange, ReadCounters, RollingTeamMetrics, ScoringRates, ShapeChoice, Situation, SquadFit,
    Stance, TacticalNeed, TeamPins, Temperament,
};
pub use teamplay::defence::{DefenceRefreshInputs, DefensiveDuty, DefensivePlan};
pub use teamplay::shape::{ShapeRefreshInputs, TeamShape};
pub use teamplay::standard::MatchStandard;
pub use teamplay::tactical::{
    BallSideZone, BallZone, GamePhase, TacticalRefreshInputs, TeamSkillAggregates,
    TeamTacticalState,
};
pub use teamplay::wide::{Flank, WidePlan};
pub use teamplay::zones::{LateralLane, MatchZone, ZoneCoeffs, ZoneStats};
pub use teamplay::{
    adherence, attack, chemistry, coach, defence, history, shape, standard, tactical, wide, zones,
};

// flow/
pub use flow::celebration::GoalCelebration;
pub use flow::context::MatchEngineConfig;
#[cfg(feature = "match-logs")]
pub(crate) use flow::context::PenaltyArea;
#[cfg(test)]
pub(crate) use flow::context::SubstitutionWindows;
pub(crate) use flow::context::{MATCH_TIME_INCREMENT_MS, MatchContext};
pub use flow::environment::{EnvModifiers, MatchEnvironment, Pitch, Weather};
pub(crate) use flow::field::MatchField;
pub(crate) use flow::goal::GoalPosition;
pub use flow::result::{
    ChanceDetail, DeadTime, FieldSquad, GoalDetail, MatchResult, MatchResultOutcome,
    MatchResultRaw, PeriodKind, PlayerMatchEndStats, Score, SubstitutionInfo, TeamScore,
};
pub(crate) use flow::result::{
    HighlightSelector, MatchTally, PenaltyShootoutKick, SubstitutionReason,
};
#[cfg(feature = "match-logs")]
pub use flow::result::{LedgerState, TeamCensusTally};
pub use flow::rng::MatchRng;
pub use flow::touchline::changeover::{advance_substitution_break, finish_substitution_break};
pub use flow::touchline::{Bench, SubstitutionBreak, TouchlineStand};
pub use flow::{celebration, context, environment, field, goal, result, rng, touchline};

// officiating/
pub use officiating::corner_shape::{CornerRole, CornerShape, CornerStation};
pub use officiating::kickoff_shape::{KickoffPlan, KickoffShape, KickoffStation};
pub use officiating::management::{
    CounterAttackThreat, Dissent, ProfessionalFoul, ProfessionalFoulCard, TimeWasting,
    TimeWastingRestart,
};
pub use officiating::referee::{ContactLocation, FoulCallContext, RefereeProfile};
pub use officiating::set_pieces::{
    CornerRoutine, CornerScores, FreeKickBand, FreeKickChoice, FreeKickChoiceScores,
    ROUTINE_REPEAT_XG_THRESHOLD, SetPieceHistory, TakerScore, ThrowRoutine,
    penalty_conversion_prob, pick_corner_routine, pick_taker, pick_throw_routine,
    score_corner_routines, score_corner_taker, score_free_kick_choices, score_free_kick_taker,
    score_keeper_save, score_penalty_taker, wall_block_prob, wall_size_for,
};
pub use officiating::{corner_shape, kickoff_shape, management, referee, set_pieces};

// substitution/ — full-path access only (no glob was exported originally).
pub use substitution::{sub_scoring, substitutions, urgency};

// rating + psychology — direct children of the engine module.
pub use psychology::{
    ActionLapse, LapseFamily, NegativeEvent, PositiveEvent, PsychState, Psychology,
    PsychologyState, SkillModifiers, TeamMomentum,
};
#[cfg(test)]
pub(crate) use rating::RatingContext;

// Re-export player items except conflicting ones
pub use player::{
    BallOperationsImpl, ClaimIntent, ClaimOutcome, GraphEdge, GraphInvariantViolation, MatchPlayer,
    MatchPlayerLite, PlayerSide, SaveOutcome, TransitionGraph, TransitionSource, behaviours,
    closure, defender_states, defenders, forwarders, goalkeepers, midfielders, objects, passing,
    shared_states, team, transition,
};

// Re-export specific types from player submodules that code expects at this level
pub use player::behaviours::SteeringBehavior;
pub use player::context::GameTickContext;
pub use player::positions::{
    GridPlayer, MatchObjectsPositions, PlayerDistanceClosure, PlayerDistanceFromPosition,
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
pub use player::strategies::shared_states::{ActivityIntensity, MovementEffort};
// Export modules for those who want to access them
pub use player::context as player_context;
pub use player::positions as player_positions;
pub use player::strategies::processor;
// Note: player::events conflicts with engine::events module, so we don't re-export it

// Re-export tactics items except conflicting ones
pub use tactics::field as tactics_field;
pub use tactics::field::{POSITION_POSITIONING, PositionType};
pub use tactics::positions as tactics_positions;
