//! Match simulation domain.
//!
//! # Export policy
//!
//! The modules below are implementation detail. Only the types listed at the
//! bottom of this file are part of the domain's public API; everything else is
//! reached through `pub(crate)` imports and never escapes the crate.
//!
//! Three sub-domains keep their own public module because they are contracts
//! of their own:
//!
//! * [`engine`] — the simulation itself. Dev harnesses drive `FootballEngine`
//!   and read its diagnostics directly, and the engine root already publishes a
//!   curated surface of its own.
//! * [`result`] — the recording/replay value types. The web crate bakes these
//!   to disk and ships them over the worker wire, so the module (and its clip
//!   constants) is deliberately public.
//! * [`squad`] — squad construction and the selector API the club domain and
//!   the selection harnesses call directly.
//!
//! The explicit `pub(crate) use` lists below keep the historical
//! `crate::r#match::Name` paths working inside the crate without re-exporting
//! those names to other crates. `pub use` is reserved for what a caller
//! outside `core` actually needs.

#[macro_use]
mod logs;

pub mod engine;
pub mod result;

mod calibration;
mod dispatch;
mod fixture;
mod game;
mod pool;
mod rules;
pub mod squad;
mod state;

// Crate-internal prelude. These names preserve the internal paths the league,
// club and national-team domains already use, but they do not widen the public
// API. The public surface is the explicit `pub use` block below.
#[cfg(all(test, feature = "match-logs"))]
pub(crate) use engine::DeadTime;
pub(crate) use engine::{
    ActivityIntensity,
    AttackPlan,
    AttackRefreshInputs,
    Ball,
    BallOperationsImpl,
    BallSide,
    BallSideZone,
    BallZone,
    Bench,
    BoxSlot,
    ClaimIntent,
    ClaimOutcome,
    ConditionContext,
    DefenceRefreshInputs,
    DefensiveDuty,
    DefensivePlan,
    Flank,
    GamePhase,
    GameTickContext,
    GoalCelebration,
    GoalOrigin,
    GoalPosition,
    HighlightSelector,
    MATCH_EXTRA_TIME_MS,
    MATCH_HALF_TIME_MS,
    MATCH_TIME_INCREMENT_MS,
    MATCH_TIME_MS,
    MAX_OWNER_TRACK_DISTANCE,
    MatchCoach,
    MatchContext,
    MatchField,
    MatchFieldSize,
    MatchObjectsPositions,
    MatchPlayerCollection,
    MatchPlayerLite,
    MatchRng,
    MatchState,
    MatchTally,
    MatchTime,
    MovementEffort,
    OffsideLine,
    OffsideSnapshot,
    Outlook,
    POSITION_POSITIONING,
    PassEvaluator,
    PassOriginRestart,
    PenaltyShootoutKick,
    PeriodKind,
    PlayerDistanceFromPosition,
    PlayerOpponentsOperationsImpl,
    PlayerTeammatesOperationsImpl,
    PositionType,
    ReadCounters,
    SaveOutcome,
    ShapeChoice,
    ShapeRefreshInputs,
    ShotTarget,
    Situation,
    Space,
    SpatialGrid,
    SquadFit,
    StateChangeResult,
    StateManager,
    StateProcessingContext,
    StateProcessingHandler,
    StateProcessingResult,
    StateProcessor,
    SteeringBehavior,
    // Kept for the league pipeline's match-event classification.
    SubstitutionReason,
    TacticalRefreshInputs,
    TeamPins,
    TeamShape,
    TeamSkillAggregates,
    TeamTacticalState,
    TeamsTactics,
    TransitionSource,
    WidePlan,
};
#[cfg(test)]
pub(crate) use engine::{
    ChemistryMap, CornerShape, GamePlan, PlanChange, RatingContext, RollingTeamMetrics,
    SubstitutionBreak, SubstitutionWindows, TacticalFamiliarity, TacticalNeed, officiating,
};
#[cfg(feature = "match-logs")]
pub(crate) use engine::{PenaltyArea, TransitionGraph};
// Module aliases the engine root used to publish. They stay available by the
// historical `crate::r#match::<module>::` paths, but the individual items
// inside them are not flattened into this root.
pub(crate) use engine::{
    ball, defenders, events, field, flow, forwarders, goalkeepers, midfielders, passing, player,
    player_positions, position_ball, position_players, shared_states, team,
};
pub(crate) use result::VectorExtensions;
pub(crate) use squad::{FixtureRoom, SelectionCompetition, SelectionContext, SquadSelector};

// ── Public surface ─────────────────────────────────────────────────────────
//
// What a match *is*, how it is played, and what comes out of it. Everything
// here is named by `web`, the binary, or the replay/worker wire.

// What a match is, and the seam a distributed dispatcher plugs into.
pub use dispatch::{MatchDispatcher, MatchDispatcherRegistry, SquadFixture};
pub use fixture::{CompetitionKind, FixtureContext};
pub use game::Match;
pub use pool::MatchPlayEnginePool;
pub use rules::MatchRules;

// Squad construction and selection.
pub use squad::{MatchSquad, OmittedPlayer, SelectionReading};

// The result payload and the stat lines the league pipeline consumes.
pub use engine::player::statistics::MatchStatisticType;
pub use engine::{
    ChanceDetail, FieldSquad, GoalDetail, MatchResult, MatchResultOutcome, MatchResultRaw,
    PlayerMatchEndStats, Score, SubstitutionInfo, TeamScore,
};
pub(crate) use state::GameState;

// A match in progress, for harnesses and the worker wire.
pub use engine::{CoachCard, FootballEngine, MatchEngineConfig, MatchPlayer, PlayerSide};

// Recording/replay value types.
pub use result::{
    MatchEventData, PassEventData, RecordingArtifacts, RecordingScope, ResultMatchPositionData,
    ResultPositionDataItem,
};

// Acceptance bands used by the development harness.
pub use calibration::{CalibrationLine, GoalTimeline, IndependentScoring, MatchCalibrationStats};
