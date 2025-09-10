use crate::r#match::midfielders::states::{MidfielderAttackSupportingState, MidfielderCreatingSpaceState, MidfielderCrossingState, MidfielderDistanceShootingState, MidfielderDistributingState, MidfielderDribblingState, MidfielderHoldingPossessionState, MidfielderInterceptingState, MidfielderPassingState, MidfielderPressingState, MidfielderReturningState, MidfielderRunningState, MidfielderShootingState, MidfielderStandingState, MidfielderSwitchingPlayState, MidfielderTacklingState, MidfielderTakeBallState, MidfielderTrackingRunnerState, MidfielderWalkingState};
use crate::r#match::{StateProcessingResult, StateProcessor};
use std::fmt::{Display, Formatter};

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum MidfielderState {
    Standing,          // Standing still
    Distributing,      // Distributing the ball to teammates
    Dribbling,         // Dribbling the ball
    AttackSupporting,  // Supporting the attack, moving forward
    HoldingPossession, // Holding possession of the ball, maintaining control
    SwitchingPlay,     // Switching the play to the other side of the field
    Crossing,          // Delivering a cross into the box
    Passing,           // Executing a  pass
    Running,           // Running in the direction of the ball
    DistanceShooting,  // Taking a shot from a long distance
    Pressing,          // Pressing the opponent to regain possession
    TrackingRunner,    // Tracking a runner to prevent a break
    Tackling,          // Tackling to win the ball
    Returning,         // Returning the ball,
    Resting,           // Resting
    Walking,           // Walking
    TakeBall,          // Take the ball,
    Shooting,          // Shooting,
    Intercepting,      // Intercepting the ball,
    CreatingSpace,     // Creating space for teammates
}

pub struct MidfielderStrategies {}

impl MidfielderStrategies {
    pub fn process(
        state: MidfielderState,
        state_processor: StateProcessor,
    ) -> StateProcessingResult {
        match state {
            MidfielderState::Standing => {
                state_processor.process(MidfielderStandingState::default())
            }
            MidfielderState::Distributing => {
                state_processor.process(MidfielderDistributingState::default())
            }
            MidfielderState::AttackSupporting => {
                state_processor.process(MidfielderAttackSupportingState::default())
            }
            MidfielderState::HoldingPossession => {
                state_processor.process(MidfielderHoldingPossessionState::default())
            }
            MidfielderState::SwitchingPlay => {
                state_processor.process(MidfielderSwitchingPlayState::default())
            }
            MidfielderState::Crossing => {
                state_processor.process(MidfielderCrossingState::default())
            }
            MidfielderState::Passing => state_processor.process(MidfielderPassingState::default()),
            MidfielderState::DistanceShooting => {
                state_processor.process(MidfielderDistanceShootingState::default())
            }
            MidfielderState::Pressing => {
                state_processor.process(MidfielderPressingState::default())
            }
            MidfielderState::TrackingRunner => {
                state_processor.process(MidfielderTrackingRunnerState::default())
            }
            MidfielderState::Tackling => {
                state_processor.process(MidfielderTacklingState::default())
            }
            MidfielderState::Returning => {
                state_processor.process(MidfielderReturningState::default())
            }
            MidfielderState::Resting => {
                state_processor.process(MidfielderDistributingState::default())
            }
            MidfielderState::Walking => state_processor.process(MidfielderWalkingState::default()),
            MidfielderState::Running => state_processor.process(MidfielderRunningState::default()),
            MidfielderState::TakeBall => {
                state_processor.process(MidfielderTakeBallState::default())
            }
            MidfielderState::Dribbling => {
                state_processor.process(MidfielderDribblingState::default())
            }
            MidfielderState::Shooting => {
                state_processor.process(MidfielderShootingState::default())
            }
            MidfielderState::Intercepting => {
                state_processor.process(MidfielderInterceptingState::default())
            },
            MidfielderState::CreatingSpace => {
                state_processor.process(MidfielderCreatingSpaceState::default())
            },
        }
    }
}

impl Display for MidfielderState {
    fn fmt(&self, f: &mut Formatter) -> std::fmt::Result {
        match self {
            MidfielderState::Standing => write!(f, "Standing"),
            MidfielderState::Distributing => write!(f, "Distributing"),
            MidfielderState::AttackSupporting => write!(f, "Supporting Attack"),
            MidfielderState::HoldingPossession => write!(f, "Holding Possession"),
            MidfielderState::SwitchingPlay => write!(f, "Switching Play"),
            MidfielderState::Crossing => write!(f, "Crossing"),
            MidfielderState::Passing => write!(f, "Passing"),
            MidfielderState::Pressing => write!(f, "Pressing"),
            MidfielderState::TrackingRunner => write!(f, "Tracking Runner"),
            MidfielderState::Tackling => write!(f, "Tackling"),
            MidfielderState::DistanceShooting => write!(f, "DistanceShooting"),
            MidfielderState::Returning => write!(f, "Returning"),
            MidfielderState::Resting => write!(f, "Resting"),
            MidfielderState::Walking => write!(f, "Walking"),
            MidfielderState::Running => write!(f, "Running"),
            MidfielderState::TakeBall => write!(f, "Take Ball"),
            MidfielderState::Dribbling => write!(f, "Dribbling"),
            MidfielderState::Shooting => write!(f, "Shooting"),
            MidfielderState::Intercepting => write!(f, "Intercepting"),
            MidfielderState::CreatingSpace => write!(f, "Creating Space"),
        }
    }
}
