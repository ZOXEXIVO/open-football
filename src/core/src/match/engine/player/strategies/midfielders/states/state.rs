use crate::r#match::midfielders::states::{
    MidfielderAttackSupportingState, MidfielderCreatingSpaceState, MidfielderCrossingState,
    MidfielderDistanceShootingState, MidfielderDistributingState, MidfielderDribblingState,
    MidfielderGuardingState, MidfielderInterceptingState, MidfielderPassingState,
    MidfielderPressingState, MidfielderRestingState, MidfielderReturningState,
    MidfielderRunningState, MidfielderShootingState, MidfielderStandingState,
    MidfielderSwitchingPlayState, MidfielderTacklingState, MidfielderTakeBallState,
    MidfielderWalkingState,
};
use crate::r#match::{StateProcessingResult, StateProcessor};
use std::fmt::Result;
use std::fmt::{Display, Formatter};

// Explicit discriminants pin `compact_id` (see `forwarders::states::state`
// for the full rationale). New variants take the next number and append
// to `ALL`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum MidfielderState {
    Standing = 0,         // Standing still
    Distributing = 1,     // Distributing the ball to teammates
    Dribbling = 2,        // Dribbling the ball
    AttackSupporting = 3, // Supporting the attack, moving forward
    SwitchingPlay = 4,    // Switching the play to the other side of the field
    Crossing = 5,         // Delivering a cross into the box
    Passing = 6,          // Executing a  pass
    Running = 7,          // Running in the direction of the ball
    DistanceShooting = 8, // Taking a shot from a long distance
    Pressing = 9,         // Pressing the opponent to regain possession
    Tackling = 10,        // Tackling to win the ball
    Returning = 11,       // Returning the ball,
    Resting = 12,         // Resting
    Walking = 13,         // Walking
    TakeBall = 14,        // Take the ball,
    Shooting = 15,        // Shooting,
    Intercepting = 16,    // Intercepting the ball,
    CreatingSpace = 17,   // Creating space for teammates
    Guarding = 18,        // Guarding an attacker — denying space and preventing them from getting open
}

impl MidfielderState {
    /// Every variant in declared order — single source of truth for the
    /// state universe (transition-graph audit + id-stability snapshot).
    pub const ALL: [MidfielderState; 19] = [
        MidfielderState::Standing,
        MidfielderState::Distributing,
        MidfielderState::Dribbling,
        MidfielderState::AttackSupporting,
        MidfielderState::SwitchingPlay,
        MidfielderState::Crossing,
        MidfielderState::Passing,
        MidfielderState::Running,
        MidfielderState::DistanceShooting,
        MidfielderState::Pressing,
        MidfielderState::Tackling,
        MidfielderState::Returning,
        MidfielderState::Resting,
        MidfielderState::Walking,
        MidfielderState::TakeBall,
        MidfielderState::Shooting,
        MidfielderState::Intercepting,
        MidfielderState::CreatingSpace,
        MidfielderState::Guarding,
    ];
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
            MidfielderState::Tackling => {
                state_processor.process(MidfielderTacklingState::default())
            }
            MidfielderState::Returning => {
                state_processor.process(MidfielderReturningState::default())
            }
            MidfielderState::Resting => state_processor.process(MidfielderRestingState::default()),
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
            }
            MidfielderState::CreatingSpace => {
                state_processor.process(MidfielderCreatingSpaceState::default())
            }
            MidfielderState::Guarding => {
                state_processor.process(MidfielderGuardingState::default())
            }
        }
    }
}

impl Display for MidfielderState {
    fn fmt(&self, f: &mut Formatter) -> Result {
        match self {
            MidfielderState::Standing => write!(f, "Standing"),
            MidfielderState::Distributing => write!(f, "Distributing"),
            MidfielderState::AttackSupporting => write!(f, "Supporting Attack"),
            MidfielderState::SwitchingPlay => write!(f, "Switching Play"),
            MidfielderState::Crossing => write!(f, "Crossing"),
            MidfielderState::Passing => write!(f, "Passing"),
            MidfielderState::Pressing => write!(f, "Pressing"),
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
            MidfielderState::Guarding => write!(f, "Guarding"),
        }
    }
}
