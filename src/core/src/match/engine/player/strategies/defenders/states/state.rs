use crate::r#match::common_states::ShapeTempo;
use crate::r#match::defenders::states::common::{DefensiveRecovery, DefensiveSlot};
use crate::r#match::defenders::states::{
    DefenderAttackingCornerState, DefenderClearingState, DefenderCoveringState,
    DefenderCrossingState, DefenderHeadingState, DefenderHoldingLineState,
    DefenderInterceptingState, DefenderJockeyingState, DefenderMarkingState,
    DefenderOverlappingState,
    DefenderPassingState, DefenderPressingState, DefenderPushingUpState,
    DefenderReturningState, DefenderRunningState, DefenderShootingState, DefenderStandingState,
    DefenderTacklingState, DefenderTakeBallState, DefenderTrackingBackState, DefenderWalkingState,
};
use crate::r#match::{StateProcessingResult, StateProcessor};
use nalgebra::Vector3;
use std::fmt::Result;
use std::fmt::{Display, Formatter};

// Explicit discriminants pin `compact_id` (see `forwarders::states::state`
// for the full rationale). New variants take the next number and append
// to `ALL`; 3 (`Resting`) and 18 (`Guarding`) are retired and never reused.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum DefenderState {
    Standing = 0,         // Standing in his place in the block
    Covering = 1,         // Covering the team-mate who is engaging the carrier
    PushingUp = 2,        // Squeezing up behind the ball while his side has it
    Passing = 4,          // Passing the ball
    Running = 5,          // On the ball, or between jobs off it
    Intercepting = 6,     // Stepping in to cut out a ball in flight
    Marking = 7,          // Marking an attacker
    Clearing = 8,         // Clearing the ball from the danger zone
    Heading = 9,          // Going up for a header
    Tackling = 10,        // Containing the carrier and challenging him
    Pressing = 11,        // Closing down the carrier
    TrackingBack = 12,    // Recovery run back to his place in the shape
    HoldingLine = 13,     // Holding his place in the line
    Returning = 14,       // Getting back into shape
    Walking = 15,         // Walking in his place while play is away
    TakeBall = 16,        // Collecting a loose ball, or a restart he takes
    Shooting = 17,        // Shooting
    AttackingCorner = 19, // Pushed up to attack an attacking corner (run into the box, head on goal)
    Crossing = 20,        // Overlapping fullback delivering from a wide advanced position
    Overlapping = 21,     // The full-back run beyond the width holder — see `overlapping`
    Jockeying = 22,       // Containing the carrier, goal-side, choosing the moment
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
        DefenderState::AttackingCorner,
        DefenderState::Crossing,
        DefenderState::Overlapping,
        DefenderState::Jockeying,
    ];

    /// Length of any table indexed by `state as usize`: one past the last
    /// discriminant, which retired ids keep above `ALL.len()`.
    pub const SLOTS: usize = Self::ALL[Self::ALL.len() - 1] as usize + 1;

    /// The states that hold the shape and steer at the slot, at the pace
    /// the play asks for (`ShapeTempo`).
    pub fn holds_the_shape(self) -> bool {
        matches!(
            self,
            DefenderState::Walking
                | DefenderState::HoldingLine
                | DefenderState::PushingUp
                | DefenderState::Returning
                | DefenderState::TrackingBack
        )
    }

    pub fn from_slot(slot: usize) -> Option<DefenderState> {
        Self::ALL.into_iter().find(|s| *s as usize == slot)
    }
}

pub struct DefenderStrategies {}

impl DefenderStrategies {
    pub fn process(state: DefenderState, state_processor: StateProcessor) -> StateProcessingResult {
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
        // A man holding the shape moves at the pace the play asks of him
        // (`ShapeTempo`): the state chooses where, the tempo how fast.
        let tempo = (state.holds_the_shape() && ShapeTempo::applies(&state_processor.ctx())).then(|| {
            let ctx = state_processor.ctx();
            let slot = DefensiveSlot::point(&ctx);
            (ShapeTempo::to(&ctx, slot), slot, ctx.player.position)
        });

        let mut result = Self::dispatch(state, state_processor);
        // An imposed movement with its own effort floor (keeper release
        // spacing) takes priority over goal-side recovery. Otherwise this
        // reverses the outlet run and pins teammates behind their keeper.
        if result.effort_floor > 0.0 {
            return result;
        }
        if let (Some((tempo, slot, position)), Some(velocity)) = (tempo, result.velocity) {
            result.velocity = Some(tempo.steer(position, slot, velocity));
            result.effort_floor = result.effort_floor.max(tempo.effort());
        }
        if let (Some((depth, weight, effort)), Some(velocity)) = (depth_override, result.velocity) {
            result.effort_floor = result.effort_floor.max(effort * weight);
            // Blend rather than replace. The recovery depth is an order of
            // magnitude larger than a state's own velocity, so overwriting
            // x outright made the rule's on/off boundary a velocity
            // INVERSION — see `DefensiveRecovery::depth_override` for the
            // per-tick dump that showed defenders alternating between
            // sprinting at the ball and sprinting at their own goal.
            let x = velocity.x * (1.0 - weight) + depth * weight;
            result.velocity = Some(Vector3::new(x, velocity.y, velocity.z));
        }
        result
    }

    fn dispatch(state: DefenderState, state_processor: StateProcessor) -> StateProcessingResult {
        match state {
            DefenderState::Standing => state_processor.process(DefenderStandingState::default()),
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
            DefenderState::AttackingCorner => {
                state_processor.process(DefenderAttackingCornerState::default())
            }
            DefenderState::Crossing => state_processor.process(DefenderCrossingState::default()),
            DefenderState::Overlapping => {
                state_processor.process(DefenderOverlappingState::default())
            }
            DefenderState::Jockeying => state_processor.process(DefenderJockeyingState::default()),
        }
    }
}

impl Display for DefenderState {
    fn fmt(&self, f: &mut Formatter) -> Result {
        match self {
            DefenderState::Standing => write!(f, "Standing"),
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
            DefenderState::AttackingCorner => write!(f, "Attacking Corner"),
            DefenderState::Crossing => write!(f, "Crossing"),
            DefenderState::Overlapping => write!(f, "Overlapping"),
            DefenderState::Jockeying => write!(f, "Jockeying"),
        }
    }
}
