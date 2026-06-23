use crate::r#match::goalkeepers::states::{
    GoalkeeperCatchingState, GoalkeeperClearingState, GoalkeeperComingOutState,
    GoalkeeperDistributingState, GoalkeeperDivingState, GoalkeeperHoldingState,
    GoalkeeperJumpingState, GoalkeeperKickingState, GoalkeeperPassingState,
    GoalkeeperPickingUpState, GoalkeeperPreparingForSaveState, GoalkeeperPunchingState,
    GoalkeeperRestingState, GoalkeeperReturningGoalState, GoalkeeperRunningState,
    GoalkeeperShootingState, GoalkeeperStandingState, GoalkeeperTacklingState,
    GoalkeeperTakeBallState, GoalkeeperThrowingState, GoalkeeperWalkingState,
};
use crate::r#match::{StateProcessingResult, StateProcessor};
use std::fmt::Result;
use std::fmt::{Display, Formatter};

// Explicit discriminants pin `compact_id` (see `forwarders::states::state`
// for the full rationale). New variants take the next number and append
// to `ALL`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum GoalkeeperState {
    Standing = 0,          // Standing
    Resting = 1,           // Resting
    Jumping = 2,           // Jumping
    Diving = 3,            // Diving to save the ball
    Catching = 4,          // Catching the ball with hands
    Punching = 5,          // Punching the ball away
    Kicking = 6,           // Kicking the ball
    Clearing = 7,          // Emergency clearance - boot the ball away
    HoldingBall = 8,       // Holding the ball in hands
    Throwing = 9,          // Throwing the ball with hands
    PickingUpBall = 10,    // Picking up the ball from the ground
    Distributing = 11,     // Distributing the ball after catching it
    ComingOut = 12,        // Coming out of the goal to intercept
    Passing = 13,          // Passing the ball
    ReturningToGoal = 14,  // Returning to the goal after coming out
    Tackling = 15,         // Tackling the ball
    Shooting = 16,         // Shoot to goal
    PreparingForSave = 17, // Preparing to make a save
    Walking = 18,          // Walking
    TakeBall = 19,         // Take the ball,
    Running = 20,          // Running
}

impl GoalkeeperState {
    /// Every variant in declared order — single source of truth for the
    /// state universe (transition-graph audit + id-stability snapshot).
    pub const ALL: [GoalkeeperState; 21] = [
        GoalkeeperState::Standing,
        GoalkeeperState::Resting,
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
        GoalkeeperState::Passing,
        GoalkeeperState::ReturningToGoal,
        GoalkeeperState::Tackling,
        GoalkeeperState::Shooting,
        GoalkeeperState::PreparingForSave,
        GoalkeeperState::Walking,
        GoalkeeperState::TakeBall,
        GoalkeeperState::Running,
    ];
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
            GoalkeeperState::Resting => state_processor.process(GoalkeeperRestingState::default()),
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
            GoalkeeperState::Tackling => {
                state_processor.process(GoalkeeperTacklingState::default())
            }
            GoalkeeperState::Shooting => {
                state_processor.process(GoalkeeperShootingState::default())
            }
            GoalkeeperState::PreparingForSave => {
                state_processor.process(GoalkeeperPreparingForSaveState::default())
            }
            GoalkeeperState::Walking => state_processor.process(GoalkeeperWalkingState::default()),
            GoalkeeperState::Passing => state_processor.process(GoalkeeperPassingState::default()),
            GoalkeeperState::TakeBall => {
                state_processor.process(GoalkeeperTakeBallState::default())
            }
            GoalkeeperState::Running => state_processor.process(GoalkeeperRunningState::default()),
        }
    }
}

impl Display for GoalkeeperState {
    fn fmt(&self, f: &mut Formatter) -> Result {
        match self {
            GoalkeeperState::Standing => write!(f, "Standing"),
            GoalkeeperState::Resting => write!(f, "Resting"),
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
            GoalkeeperState::Shooting => write!(f, "Try shoot to goal"),
            GoalkeeperState::PreparingForSave => write!(f, "Preparing for Save"),
            GoalkeeperState::Tackling => write!(f, "Tackling"),
            GoalkeeperState::Walking => write!(f, "Walking"),
            GoalkeeperState::Passing => write!(f, "Passing"),
            GoalkeeperState::TakeBall => write!(f, "Take Ball"),
            GoalkeeperState::Running => write!(f, "Running"),
        }
    }
}
