use crate::r#match::goalkeepers::states::{
    GoalkeeperCatchingState, GoalkeeperClearingState, GoalkeeperComingOutState,
    GoalkeeperDistributingState, GoalkeeperDivingState, GoalkeeperGettingUpState,
    GoalkeeperHoldingState, GoalkeeperJumpingState, GoalkeeperKickingState,
    GoalkeeperOneOnOneState, GoalkeeperPickingUpState, GoalkeeperPreparingForSaveState,
    GoalkeeperPunchingState, GoalkeeperReturningGoalState, GoalkeeperStandingState,
    GoalkeeperTakeBallState, GoalkeeperThrowingState, GoalkeeperWalkingState,
};
use crate::r#match::{StateProcessingResult, StateProcessor};
use std::fmt::Result;
use std::fmt::{Display, Formatter};

// Explicit discriminants pin `compact_id` (see `forwarders::states::state`
// for the full rationale). New variants take the next FREE number and
// append to `ALL`.
//
// The gaps (1, 13, 15, 16, 20) are retired states. `Resting`, `Tackling`,
// `Shooting` and `Running` were fully implemented but had no inbound
// transition in the engine and were never observed in a real match: they
// were removed rather than wired, because `Standing` already carries the
// `Recovery` intensity `Resting` existed for, `ComingOut` covers rushing
// off the line, and a keeper shooting at the other goal is not modelled
// anywhere. `Passing` (13) went in 2026-11, its release owned by
// `Distributing`. The numbers are deliberately NOT reused. A replay stores
// a state's display NAME (`PlayerStateEntry`), so the id is an in-process
// key, but every keeper census table is indexed by it, and a reused number
// would mean two different states in two builds' tables.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum GoalkeeperState {
    Standing = 0,          // Standing
    Jumping = 2,           // Jumping
    Diving = 3,            // Diving to save the ball
    Catching = 4,          // Catching the ball with hands
    Punching = 5,          // Punching the ball away
    Kicking = 6,           // Kicking the ball long downfield
    Clearing = 7,          // Emergency clearance - boot the ball away
    HoldingBall = 8,       // Holding the ball in hands
    Throwing = 9,          // Throwing the ball with hands
    PickingUpBall = 10,    // Picking up the ball from the ground
    Distributing = 11,     // Distributing the ball after catching it
    ComingOut = 12,        // Coming out of the goal to intercept
    ReturningToGoal = 14,  // Returning to the goal after coming out
    PreparingForSave = 17, // Preparing to make a save
    Walking = 18,          // Walking
    TakeBall = 19,         // Take the ball,
    OneOnOne = 21,         // Closing down a man through on goal
    GettingUp = 22,        // On the ground after going down
}

impl GoalkeeperState {
    /// Every variant in declared order — single source of truth for the
    /// state universe (transition-graph audit + id-stability snapshot).
    pub const ALL: [GoalkeeperState; 18] = [
        GoalkeeperState::Standing,
        GoalkeeperState::Jumping,
        GoalkeeperState::Diving,
        GoalkeeperState::Catching,
        GoalkeeperState::Punching,
        GoalkeeperState::Kicking,
        GoalkeeperState::Clearing,
        GoalkeeperState::HoldingBall,
        GoalkeeperState::Throwing,
        GoalkeeperState::PickingUpBall,
        GoalkeeperState::Distributing,
        GoalkeeperState::ComingOut,
        GoalkeeperState::ReturningToGoal,
        GoalkeeperState::PreparingForSave,
        GoalkeeperState::Walking,
        GoalkeeperState::TakeBall,
        GoalkeeperState::OneOnOne,
        GoalkeeperState::GettingUp,
    ];

    /// Length of any table indexed by `state as usize`: one past the last
    /// discriminant, which retired ids keep above `ALL.len()`. `ALL` is in
    /// ascending order (`role_discriminants_are_strictly_increasing_and_unique`).
    pub const SLOTS: usize = Self::ALL[Self::ALL.len() - 1] as usize + 1;

    pub fn from_slot(slot: usize) -> Option<GoalkeeperState> {
        Self::ALL.into_iter().find(|s| *s as usize == slot)
    }

    /// The states a claim of a high ball is made in, from coming for it to
    /// carrying out the contest's decision.
    pub fn is_claim(self) -> bool {
        matches!(
            self,
            GoalkeeperState::ComingOut
                | GoalkeeperState::Catching
                | GoalkeeperState::Jumping
                | GoalkeeperState::Punching
        )
    }
}

pub struct GoalkeeperStrategies {}

impl GoalkeeperStrategies {
    pub fn process(
        state: GoalkeeperState,
        state_processor: StateProcessor,
    ) -> StateProcessingResult {
        match state {
            GoalkeeperState::Standing => {
                state_processor.process(GoalkeeperStandingState::default())
            }
            GoalkeeperState::Jumping => state_processor.process(GoalkeeperJumpingState::default()),
            GoalkeeperState::Diving => state_processor.process(GoalkeeperDivingState::default()),
            GoalkeeperState::Catching => {
                state_processor.process(GoalkeeperCatchingState::default())
            }
            GoalkeeperState::Punching => {
                state_processor.process(GoalkeeperPunchingState::default())
            }
            GoalkeeperState::Kicking => state_processor.process(GoalkeeperKickingState::default()),
            GoalkeeperState::Clearing => {
                state_processor.process(GoalkeeperClearingState::default())
            }
            GoalkeeperState::HoldingBall => {
                state_processor.process(GoalkeeperHoldingState::default())
            }
            GoalkeeperState::Throwing => {
                state_processor.process(GoalkeeperThrowingState::default())
            }
            GoalkeeperState::PickingUpBall => {
                state_processor.process(GoalkeeperPickingUpState::default())
            }
            GoalkeeperState::Distributing => {
                state_processor.process(GoalkeeperDistributingState::default())
            }
            GoalkeeperState::ComingOut => {
                state_processor.process(GoalkeeperComingOutState::default())
            }
            GoalkeeperState::ReturningToGoal => {
                state_processor.process(GoalkeeperReturningGoalState::default())
            }
            GoalkeeperState::PreparingForSave => {
                state_processor.process(GoalkeeperPreparingForSaveState::default())
            }
            GoalkeeperState::Walking => state_processor.process(GoalkeeperWalkingState::default()),
            GoalkeeperState::TakeBall => {
                state_processor.process(GoalkeeperTakeBallState::default())
            }
            GoalkeeperState::OneOnOne => {
                state_processor.process(GoalkeeperOneOnOneState::default())
            }
            GoalkeeperState::GettingUp => {
                state_processor.process(GoalkeeperGettingUpState::default())
            }
        }
    }
}

impl Display for GoalkeeperState {
    fn fmt(&self, f: &mut Formatter) -> Result {
        match self {
            GoalkeeperState::Standing => write!(f, "Standing"),
            GoalkeeperState::Jumping => write!(f, "Jumping"),
            GoalkeeperState::Diving => write!(f, "Diving"),
            GoalkeeperState::Catching => write!(f, "Catching"),
            GoalkeeperState::Punching => write!(f, "Punching"),
            GoalkeeperState::Kicking => write!(f, "Kicking"),
            GoalkeeperState::Clearing => write!(f, "Clearing"),
            GoalkeeperState::HoldingBall => write!(f, "Holding Ball"),
            GoalkeeperState::Throwing => write!(f, "Throwing"),
            GoalkeeperState::PickingUpBall => write!(f, "Picking Up Ball"),
            GoalkeeperState::Distributing => write!(f, "Distributing"),
            GoalkeeperState::ComingOut => write!(f, "Coming Out"),
            GoalkeeperState::ReturningToGoal => write!(f, "Returning to Goal"),
            GoalkeeperState::PreparingForSave => write!(f, "Preparing for Save"),
            GoalkeeperState::Walking => write!(f, "Walking"),
            GoalkeeperState::TakeBall => write!(f, "Take Ball"),
            GoalkeeperState::OneOnOne => write!(f, "One on One"),
            GoalkeeperState::GettingUp => write!(f, "Getting Up"),
        }
    }
}
