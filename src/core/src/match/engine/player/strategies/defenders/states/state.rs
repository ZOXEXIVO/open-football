use crate::r#match::defenders::states::{
    DefenderAttackingCornerState, DefenderClearingState, DefenderCoveringState,
    DefenderCrossingState, DefenderGuardingState, DefenderHeadingState, DefenderHoldingLineState,
    DefenderInterceptingState, DefenderMarkingState, DefenderPassingState, DefenderPressingState,
    DefenderPushingUpState, DefenderRestingState, DefenderReturningState, DefenderRunningState,
    DefenderShootingState, DefenderStandingState, DefenderTacklingState, DefenderTakeBallState,
    DefenderTrackingBackState, DefenderWalkingState,
};
use crate::r#match::defenders::states::common::DefensiveRecovery;
use crate::r#match::{StateProcessingResult, StateProcessor};
use nalgebra::Vector3;
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
    Guarding = 18, // Guarding an attacker — denying space and preventing them from getting open
    AttackingCorner = 19, // Pushed up to attack an attacking corner (run into the box, head on goal)
    Crossing = 20,        // Overlapping fullback delivering from a wide advanced position
}

impl DefenderState {
    /// States in which the defender is actively playing the ball rather
    /// than holding a position.
    ///
    /// The goal-side rule ([`DefensiveRecovery`]) is about SHAPE — where
    /// a defender stands when the ball is behind him. A defender who is
    /// mid-interception, mid-tackle, attacking a header or clearing is
    /// not out of shape; he is defending, and overriding his depth
    /// velocity drags him off the ball he was about to win. Measured:
    /// without this exemption DEF interceptions fell **1.38 → 0.40 per
    /// match against a real ~1.3** when the rule landed — the rule was
    /// pulling defenders goal-ward through the interception point.
    /// Deliberately only the three states that are a tick from contact.
    /// `Clearing` / `TakeBall` were tried and reverted: they are
    /// long-lived, so exempting them switched the rule off for most of a
    /// defender's match — goal-side presence fell 1.05 → 0.82 per shot
    /// and clearances ballooned 1.64 → 8.14 against a real ~3.5. The
    /// states where the defender already HAS the ball (`Passing`,
    /// `Shooting`, `Crossing`) need no entry here: `depth_override`
    /// exempts the ball carrier directly.
    pub fn is_playing_the_ball(self) -> bool {
        matches!(
            self,
            DefenderState::Intercepting | DefenderState::Tackling | DefenderState::Heading
        )
    }

    /// Every variant in declared order — single source of truth for the
    /// state universe (transition-graph audit + id-stability snapshot).
    pub const ALL: [DefenderState; 21] = [
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
        DefenderState::Crossing,
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

        // Read the situation before dispatching — `process` consumes the
        // processor. Every defender obeys the goal-side rule on top of
        // whatever his state wants, because five of the states run during
        // opposition possession and not one of them owns defensive depth.
        // See `DefensiveRecovery` for the measurement that made this a
        // cross-state rule rather than a fix to any single state.
        let depth_override = if state.is_playing_the_ball() {
            None
        } else {
            DefensiveRecovery::depth_override(&state_processor.ctx())
        };

        let mut result = Self::dispatch(state, state_processor);
        if let (Some(depth), Some(velocity)) = (depth_override, result.velocity) {
            result.velocity = Some(Vector3::new(depth, velocity.y, velocity.z));
        }
        result
    }

    fn dispatch(state: DefenderState, state_processor: StateProcessor) -> StateProcessingResult {
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
            DefenderState::Crossing => state_processor.process(DefenderCrossingState::default()),
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
            DefenderState::Crossing => write!(f, "Crossing"),
        }
    }
}
