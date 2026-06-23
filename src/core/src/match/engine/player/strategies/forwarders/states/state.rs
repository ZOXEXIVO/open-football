use crate::r#match::forwarders::states::{
    ForwardAssistingState, ForwardCreatingSpaceState, ForwardCrossReceivingState,
    ForwardCrossingState, ForwardDribblingState, ForwardFinishingState, ForwardHeadingState,
    ForwardInterceptingState, ForwardPassingState, ForwardPressingState, ForwardRestingState,
    ForwardReturningState, ForwardRunningInBehindState, ForwardRunningState, ForwardShootingState,
    ForwardStandingState, ForwardTacklingState, ForwardTakeBallState, ForwardWalkingState,
};
use crate::r#match::{StateProcessingResult, StateProcessor};
use std::fmt::Result;
use std::fmt::{Display, Formatter};

// Explicit discriminants: `PlayerState::compact_id` casts these via
// `as u16`, and they're embedded in replay/position records. Pinning each
// value to its variant means inserting or reordering a state can never
// silently renumber the others. New variants MUST take the next free
// number and be appended to `ALL`; `compact_id_snapshot` in
// `player/state.rs` fails loudly if any value moves.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ForwardState {
    Standing = 0,         // Standing still
    Walking = 1,          // Walking at low intensity to reposition or conserve energy
    Passing = 2,          // Passing the ball
    Dribbling = 3,        // Dribbling the ball past opponents
    Shooting = 4,         // Taking a shot on goal
    Heading = 5,          // Heading the ball, often during crosses or set pieces
    RunningInBehind = 6,  // Making a run behind the defense to receive a pass
    Running = 7,          // Running in the direction of the ball
    Pressing = 8,         // Pressing defenders to force a mistake or regain possession
    Finishing = 9,        // Attempting to score from a close range
    CreatingSpace = 10,   // Creating space for teammates by pulling defenders away
    CrossReceiving = 11,  // Positioning to receive a cross
    Crossing = 12,        // Delivering a cross from a wide position
    Tackling = 13,        // Tackling the ball
    Assisting = 14,       // Providing an assist by passing or crossing to a teammate
    TakeBall = 15,        // Take the ball,
    Intercepting = 16,    // Intercepting the ball,
    Returning = 17,       // Returning the ball
    Resting = 18,         // Recovering stamina when fatigued
}

impl ForwardState {
    /// Every variant in declared order — the single source of truth for
    /// the state universe (transition-graph audit + id-stability snapshot).
    pub const ALL: [ForwardState; 19] = [
        ForwardState::Standing,
        ForwardState::Walking,
        ForwardState::Passing,
        ForwardState::Dribbling,
        ForwardState::Shooting,
        ForwardState::Heading,
        ForwardState::RunningInBehind,
        ForwardState::Running,
        ForwardState::Pressing,
        ForwardState::Finishing,
        ForwardState::CreatingSpace,
        ForwardState::CrossReceiving,
        ForwardState::Crossing,
        ForwardState::Tackling,
        ForwardState::Assisting,
        ForwardState::TakeBall,
        ForwardState::Intercepting,
        ForwardState::Returning,
        ForwardState::Resting,
    ];
}

pub struct ForwardStrategies {}

impl ForwardStrategies {
    pub fn process(state: ForwardState, state_processor: StateProcessor) -> StateProcessingResult {
        match state {
            ForwardState::Standing => state_processor.process(ForwardStandingState::default()),
            ForwardState::Walking => state_processor.process(ForwardWalkingState::default()),
            ForwardState::Passing => state_processor.process(ForwardPassingState::default()),
            ForwardState::Dribbling => state_processor.process(ForwardDribblingState::default()),
            ForwardState::Shooting => state_processor.process(ForwardShootingState::default()),
            ForwardState::Heading => state_processor.process(ForwardHeadingState::default()),
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
            ForwardState::Crossing => state_processor.process(ForwardCrossingState::default()),
            ForwardState::Tackling => state_processor.process(ForwardTacklingState::default()),
            ForwardState::Assisting => state_processor.process(ForwardAssistingState::default()),
            ForwardState::Running => state_processor.process(ForwardRunningState::default()),
            ForwardState::TakeBall => state_processor.process(ForwardTakeBallState::default()),
            ForwardState::Intercepting => {
                state_processor.process(ForwardInterceptingState::default())
            }
            ForwardState::Returning => state_processor.process(ForwardReturningState::default()),
            ForwardState::Resting => state_processor.process(ForwardRestingState::default()),
        }
    }
}

impl Display for ForwardState {
    fn fmt(&self, f: &mut Formatter) -> Result {
        match self {
            ForwardState::Standing => write!(f, "Standing"),
            ForwardState::Walking => write!(f, "Walking"),
            ForwardState::Dribbling => write!(f, "Dribbling"),
            ForwardState::Shooting => write!(f, "Shooting"),
            ForwardState::Heading => write!(f, "Heading"),
            ForwardState::RunningInBehind => write!(f, "Running In Behind"),
            ForwardState::Pressing => write!(f, "Pressing"),
            ForwardState::Finishing => write!(f, "Finishing"),
            ForwardState::CreatingSpace => write!(f, "Creating Space"),
            ForwardState::CrossReceiving => write!(f, "Cross Receiving"),
            ForwardState::Crossing => write!(f, "Crossing"),
            ForwardState::Assisting => write!(f, "Assisting"),
            ForwardState::Passing => write!(f, "Passing"),
            ForwardState::Tackling => write!(f, "Tackling"),
            ForwardState::Running => write!(f, "Running"),
            ForwardState::TakeBall => write!(f, "Take Ball"),
            ForwardState::Intercepting => write!(f, "Intercepting"),
            ForwardState::Returning => write!(f, "Returning"),
            ForwardState::Resting => write!(f, "Resting"),
        }
    }
}
