use crate::r#match::defenders::states::{
    DefenderAttackingCornerState, DefenderClearingState, DefenderCoveringState,
    DefenderGuardingState, DefenderHeadingState, DefenderHoldingLineState,
    DefenderInterceptingState, DefenderMarkingState, DefenderPassingState, DefenderPressingState,
    DefenderPushingUpState, DefenderRestingState, DefenderReturningState, DefenderRunningState,
    DefenderShootingState, DefenderStandingState, DefenderTacklingState, DefenderTakeBallState,
    DefenderTrackingBackState, DefenderWalkingState,
};
use crate::r#match::{StateProcessingResult, StateProcessor};
use std::fmt::Result;
use std::fmt::{Display, Formatter};

// Explicit discriminants pin `compact_id` (see `forwarders::states::state`
// for the full rationale). New variants take the next number and append
// to `ALL`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum DefenderState {
    Standing = 0,         // Standing
    Covering = 1,         // Covering the ball
    PushingUp = 2,        // Pushing the ball up
    Resting = 3,          // Resting after an attack
    Passing = 4,          // Passing the ball
    Running = 5,          // Running in the direction of the ball
    Intercepting = 6,     // Intercepting a pass
    Marking = 7,          // Marking an attacker
    Clearing = 8,         // Clearing the ball from the danger zone
    Heading = 9,          // Heading the ball, often during corners or crosses
    Tackling = 10,        // Tackling the ball
    Pressing = 11,        // Pressing the opponent
    TrackingBack = 12,    // Tracking back to defense after an attack
    HoldingLine = 13,     // Holding the defensive line
    Returning = 14,       // Returning the ball,
    Walking = 15,         // Walking around,
    TakeBall = 16,        // Take the ball,
    Shooting = 17,        // Shoting the ball,
    Guarding = 18,        // Guarding an attacker — denying space and preventing them from getting open
    AttackingCorner = 19, // Pushed up to attack an attacking corner (run into the box, head on goal)
}

impl DefenderState {
    /// Every variant in declared order — single source of truth for the
    /// state universe (transition-graph audit + id-stability snapshot).
    pub const ALL: [DefenderState; 20] = [
        DefenderState::Standing,
        DefenderState::Covering,
        DefenderState::PushingUp,
        DefenderState::Resting,
        DefenderState::Passing,
        DefenderState::Running,
        DefenderState::Intercepting,
        DefenderState::Marking,
        DefenderState::Clearing,
        DefenderState::Heading,
        DefenderState::Tackling,
        DefenderState::Pressing,
        DefenderState::TrackingBack,
        DefenderState::HoldingLine,
        DefenderState::Returning,
        DefenderState::Walking,
        DefenderState::TakeBall,
        DefenderState::Shooting,
        DefenderState::Guarding,
        DefenderState::AttackingCorner,
    ];
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
            DefenderState::AttackingCorner => {
                state_processor.process(DefenderAttackingCornerState::default())
            }
        }
    }
}

impl Display for DefenderState {
    fn fmt(&self, f: &mut Formatter) -> Result {
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
            DefenderState::AttackingCorner => write!(f, "Attacking Corner"),
        }
    }
}
