use crate::r#match::defenders::states::{
    DefenderClearingState, DefenderCoveringState, DefenderGuardingState, DefenderHeadingState,
    DefenderHoldingLineState, DefenderInterceptingState, DefenderMarkingState,
    DefenderPassingState, DefenderPressingState, DefenderPushingUpState, DefenderRestingState,
    DefenderReturningState, DefenderRunningState, DefenderShootingState, DefenderStandingState,
    DefenderTacklingState, DefenderTakeBallState, DefenderTrackingBackState, DefenderWalkingState,
};
use crate::r#match::{StateProcessingResult, StateProcessor};
use std::fmt::{Display, Formatter};

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum DefenderState {
    Standing,     // Standing
    Covering,     // Covering the ball
    PushingUp,    // Pushing the ball up
    Resting,      // Resting after an attack
    Passing,      // Passing the ball
    Running,      // Running in the direction of the ball
    Intercepting, // Intercepting a pass
    Marking,      // Marking an attacker
    Clearing,     // Clearing the ball from the danger zone
    Heading,      // Heading the ball, often during corners or crosses
    Tackling,     // Tackling the ball
    Pressing,     // Pressing the opponent
    TrackingBack, // Tracking back to defense after an attack
    HoldingLine,  // Holding the defensive line
    Returning,    // Returning the ball,
    Walking,      // Walking around,
    TakeBall,     // Take the ball,
    Shooting,     // Shoting the ball,
    Guarding,     // Guarding an attacker — denying space and preventing them from getting open
}

pub struct DefenderStrategies {}

impl DefenderStrategies {
    pub fn process(state: DefenderState, state_processor: StateProcessor) -> StateProcessingResult {
        // let common_state = state_processor.process(DefenderCommonState::default());
        //
        // if common_state.state.is_some() {
        //     return common_state;
        // }

        match state {
            DefenderState::Standing => state_processor.process(DefenderStandingState::default()),
            DefenderState::Resting => state_processor.process(DefenderRestingState::default()),
            DefenderState::Passing => state_processor.process(DefenderPassingState::default()),
            DefenderState::Intercepting => {
                state_processor.process(DefenderInterceptingState::default())
            }
            DefenderState::Marking => state_processor.process(DefenderMarkingState::default()),
            DefenderState::Clearing => state_processor.process(DefenderClearingState::default()),
            DefenderState::Heading => state_processor.process(DefenderHeadingState::default()),
            DefenderState::Pressing => state_processor.process(DefenderPressingState::default()),
            DefenderState::TrackingBack => {
                state_processor.process(DefenderTrackingBackState::default())
            }
            DefenderState::HoldingLine => {
                state_processor.process(DefenderHoldingLineState::default())
            }
            DefenderState::Running => state_processor.process(DefenderRunningState::default()),
            DefenderState::Returning => state_processor.process(DefenderReturningState::default()),
            DefenderState::Walking => state_processor.process(DefenderWalkingState::default()),
            DefenderState::Tackling => state_processor.process(DefenderTacklingState::default()),
            DefenderState::Covering => state_processor.process(DefenderCoveringState::default()),
            DefenderState::PushingUp => state_processor.process(DefenderPushingUpState::default()),
            DefenderState::TakeBall => state_processor.process(DefenderTakeBallState::default()),
            DefenderState::Shooting => state_processor.process(DefenderShootingState::default()),
            DefenderState::Guarding => state_processor.process(DefenderGuardingState::default()),
        }
    }
}

impl Display for DefenderState {
    fn fmt(&self, f: &mut Formatter) -> std::fmt::Result {
        match self {
            DefenderState::Standing => write!(f, "Standing"),
            DefenderState::Resting => write!(f, "Resting"),
            DefenderState::Passing => write!(f, "Passing"),
            DefenderState::Intercepting => write!(f, "Intercepting"),
            DefenderState::Marking => write!(f, "Marking"),
            DefenderState::Clearing => write!(f, "Clearing"),
            DefenderState::Heading => write!(f, "Heading"),
            DefenderState::Pressing => write!(f, "Pressing"),
            DefenderState::TrackingBack => write!(f, "Tracking Back"),
            DefenderState::HoldingLine => write!(f, "Holding Line"),
            DefenderState::Running => write!(f, "Running"),
            DefenderState::Returning => write!(f, "Returning"),
            DefenderState::Walking => write!(f, "Walking"),
            DefenderState::Tackling => write!(f, "Tackling"),
            DefenderState::Covering => write!(f, "Covering"),
            DefenderState::PushingUp => write!(f, "Pushing Up"),
            DefenderState::TakeBall => write!(f, "Take Ball"),
            DefenderState::Shooting => write!(f, "Shooting"),
            DefenderState::Guarding => write!(f, "Guarding"),
        }
    }
}
