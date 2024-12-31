use crate::r#match::forwarders::states::{
    ForwardAssistingState, ForwardCreatingSpaceState, ForwardCrossReceivingState,
    ForwardDribblingState, ForwardFinishingState, ForwardHeadingState, ForwardHeadingUpPlayState,
    ForwardInterceptingState, ForwardOffsideTrapBreakingState, ForwardPassingState,
    ForwardPressingState, ForwardReturningState, ForwardRunningInBehindState, ForwardRunningState,
    ForwardShootingState, ForwardStandingState, ForwardTacklingState, ForwardTakeBallState,
};
use crate::r#match::{StateProcessingResult, StateProcessor};
use std::fmt::{Display, Formatter};

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ForwardState {
    Standing,            // Standing still
    Passing,             // Passing the ball
    Dribbling,           // Dribbling the ball past opponents
    Shooting,            // Taking a shot on goal
    Heading,             // Heading the ball, often during crosses or set pieces
    HoldingUpPlay,       // Holding up the ball to allow teammates to join the attack
    RunningInBehind,     // Making a run behind the defense to receive a pass
    Running,             // Running in the direction of the ball
    Pressing,            // Pressing defenders to force a mistake or regain possession
    Finishing,           // Attempting to score from a close range
    CreatingSpace,       // Creating space for teammates by pulling defenders away
    CrossReceiving,      // Positioning to receive a cross
    OffsideTrapBreaking, // Trying to beat the offside trap by timing runs
    Tackling,            // Tackling the ball
    Assisting,           // Providing an assist by passing or crossing to a teammate
    TakeBall,            // Take the ball,
    Intercepting,        // Intercepting the ball,
    Returning,           // Returning the ball
}

pub struct ForwardStrategies {}

impl ForwardStrategies {
    pub fn process(state: ForwardState, state_processor: StateProcessor) -> StateProcessingResult {
        match state {
            ForwardState::Standing => state_processor.process(ForwardStandingState::default()),
            ForwardState::Passing => state_processor.process(ForwardPassingState::default()),
            ForwardState::Dribbling => state_processor.process(ForwardDribblingState::default()),
            ForwardState::Shooting => state_processor.process(ForwardShootingState::default()),
            ForwardState::Heading => state_processor.process(ForwardHeadingState::default()),
            ForwardState::HoldingUpPlay => {
                state_processor.process(ForwardHeadingUpPlayState::default())
            }
            ForwardState::RunningInBehind => {
                state_processor.process(ForwardRunningInBehindState::default())
            }
            ForwardState::Pressing => state_processor.process(ForwardPressingState::default()),
            ForwardState::Finishing => state_processor.process(ForwardFinishingState::default()),
            ForwardState::CreatingSpace => {
                state_processor.process(ForwardCreatingSpaceState::default())
            }
            ForwardState::CrossReceiving => {
                state_processor.process(ForwardCrossReceivingState::default())
            }
            ForwardState::OffsideTrapBreaking => {
                state_processor.process(ForwardOffsideTrapBreakingState::default())
            }
            ForwardState::Tackling => state_processor.process(ForwardTacklingState::default()),
            ForwardState::Assisting => state_processor.process(ForwardAssistingState::default()),
            ForwardState::Running => state_processor.process(ForwardRunningState::default()),
            ForwardState::TakeBall => state_processor.process(ForwardTakeBallState::default()),
            ForwardState::Intercepting => {
                state_processor.process(ForwardInterceptingState::default())
            }
            ForwardState::Returning => state_processor.process(ForwardReturningState::default()),
        }
    }
}

impl Display for ForwardState {
    fn fmt(&self, f: &mut Formatter) -> std::fmt::Result {
        match self {
            ForwardState::Standing => write!(f, "Standing"),
            ForwardState::Dribbling => write!(f, "Dribbling"),
            ForwardState::Shooting => write!(f, "Shooting"),
            ForwardState::Heading => write!(f, "Heading"),
            ForwardState::HoldingUpPlay => write!(f, "Holding Up Play"),
            ForwardState::RunningInBehind => write!(f, "Running In Behind"),
            ForwardState::Pressing => write!(f, "Pressing"),
            ForwardState::Finishing => write!(f, "Finishing"),
            ForwardState::CreatingSpace => write!(f, "Creating Space"),
            ForwardState::CrossReceiving => write!(f, "Cross Receiving"),
            ForwardState::OffsideTrapBreaking => write!(f, "Offside Trap Breaking"),
            ForwardState::Assisting => write!(f, "Assisting"),
            ForwardState::Passing => write!(f, "Passing"),
            ForwardState::Tackling => write!(f, "Tackling"),
            ForwardState::Running => write!(f, "Running"),
            ForwardState::TakeBall => write!(f, "Take Ball"),
            ForwardState::Intercepting => write!(f, "Intercepting"),
            ForwardState::Returning => write!(f, "Returning"),
        }
    }
}
