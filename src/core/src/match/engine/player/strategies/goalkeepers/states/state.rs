use crate::r#match::goalkeepers::states::{GoalkeeperAttentiveState, GoalkeeperCatchingState, GoalkeeperComingOutState, GoalkeeperDistributingState, GoalkeeperDivingState, GoalkeeperHoldingState, GoalkeeperJumpingState, GoalkeeperKickingState, GoalkeeperPassingState, GoalkeeperPenaltyState, GoalkeeperPickingUpState, GoalkeeperPreparingForSaveState, GoalkeeperPressureState, GoalkeeperPunchingState, GoalkeeperRestingState, GoalkeeperReturningGoalState, GoalkeeperRunningState, GoalkeeperShootingState, GoalkeeperStandingState, GoalkeeperSweepingState, GoalkeeperTacklingState, GoalkeeperTakeBallState, GoalkeeperThrowingState, GoalkeeperWalkingState};
use crate::r#match::{StateProcessingResult, StateProcessor};
use std::fmt::{Display, Formatter};

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum GoalkeeperState {
    Standing,         // Standing
    Resting,          // Resting
    Jumping,          // Jumping
    Diving,           // Diving to save the ball
    Catching,         // Catching the ball with hands
    Punching,         // Punching the ball away
    Kicking,          // Kicking the ball
    HoldingBall,      // Holding the ball in hands
    Throwing,         // Throwing the ball with hands
    PickingUpBall,    // Picking up the ball from the ground
    Distributing,     // Distributing the ball after catching it
    ComingOut,        // Coming out of the goal to intercept
    Passing,          // Passing the ball
    ReturningToGoal,  // Returning to the goal after coming out
    Tackling,         // Tackling the ball
    Sweeping,         // Acting as a "sweeper", clearing the ball in front of the defense line
    UnderPressure,    // Under pressure from opponents
    Shooting,         // Shoot to goal
    PreparingForSave, // Preparing to make a save
    PenaltySave,      // Saving a penalty,
    Walking,          // Walking
    TakeBall,         // Take the ball,
    Attentive,        // Attentive,
    Running,          // Running
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
            GoalkeeperState::Sweeping => {
                state_processor.process(GoalkeeperSweepingState::default())
            }
            GoalkeeperState::UnderPressure => {
                state_processor.process(GoalkeeperPressureState::default())
            }
            GoalkeeperState::Shooting => {
                state_processor.process(GoalkeeperShootingState::default())
            }
            GoalkeeperState::PreparingForSave => {
                state_processor.process(GoalkeeperPreparingForSaveState::default())
            }
            GoalkeeperState::PenaltySave => {
                state_processor.process(GoalkeeperPenaltyState::default())
            }
            GoalkeeperState::Walking => state_processor.process(GoalkeeperWalkingState::default()),
            GoalkeeperState::Passing => state_processor.process(GoalkeeperPassingState::default()),
            GoalkeeperState::TakeBall => {
                state_processor.process(GoalkeeperTakeBallState::default())
            }
            GoalkeeperState::Attentive => {
                state_processor.process(GoalkeeperAttentiveState::default())
            }
            GoalkeeperState::Running => { state_processor.process(GoalkeeperRunningState::default())}
        }
    }
}

impl Display for GoalkeeperState {
    fn fmt(&self, f: &mut Formatter) -> std::fmt::Result {
        match self {
            GoalkeeperState::Standing => write!(f, "Standing"),
            GoalkeeperState::Resting => write!(f, "Resting"),
            GoalkeeperState::Jumping => write!(f, "Jumping"),
            GoalkeeperState::Diving => write!(f, "Diving"),
            GoalkeeperState::Catching => write!(f, "Catching"),
            GoalkeeperState::Punching => write!(f, "Punching"),
            GoalkeeperState::Kicking => write!(f, "Kicking"),
            GoalkeeperState::HoldingBall => write!(f, "Holding Ball"),
            GoalkeeperState::Throwing => write!(f, "Throwing"),
            GoalkeeperState::PickingUpBall => write!(f, "Picking Up Ball"),
            GoalkeeperState::Distributing => write!(f, "Distributing"),
            GoalkeeperState::ComingOut => write!(f, "Coming Out"),
            GoalkeeperState::ReturningToGoal => write!(f, "Returning to Goal"),
            GoalkeeperState::Sweeping => write!(f, "Sweeping"),
            GoalkeeperState::UnderPressure => write!(f, "Under Pressure"),
            GoalkeeperState::Shooting => write!(f, "Try shoot to goal"),
            GoalkeeperState::PreparingForSave => write!(f, "Preparing for Save"),
            GoalkeeperState::PenaltySave => write!(f, "Penalty Save"),
            GoalkeeperState::Tackling => write!(f, "Tackling"),
            GoalkeeperState::Walking => write!(f, "Walking"),
            GoalkeeperState::Passing => write!(f, "Passing"),
            GoalkeeperState::TakeBall => write!(f, "Take Ball"),
            GoalkeeperState::Attentive => write!(f, "Attentive"),
            GoalkeeperState::Running => write!(f, "Running"),
        }
    }
}
