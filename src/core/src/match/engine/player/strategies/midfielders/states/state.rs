use crate::r#match::common_states::ShapeTempo;
use crate::r#match::midfielders::states::{
    MidfielderAttackSupportingState, MidfielderCreatingSpaceState, MidfielderCrossingState,
    MidfielderDistanceShootingState, MidfielderDribblingState, MidfielderGuardingState,
    MidfielderHeadingState, MidfielderHoldingShapeState, MidfielderHoldingWidthState,
    MidfielderInterceptingState, MidfielderJockeyingState, MidfielderPassingState,
    MidfielderPressingState, MidfielderReturningState, MidfielderRunningState,
    MidfielderShootingState, MidfielderStandingState, MidfielderSwitchingPlayState,
    MidfielderTacklingState, MidfielderTakeBallState, MidfielderTrackingBackState,
    MidfielderWalkingState,
};
use crate::r#match::{StateProcessingResult, StateProcessor};
use std::fmt::Result;
use std::fmt::{Display, Formatter};

// Explicit discriminants pin `compact_id` (see `forwarders::states::state`
// for the full rationale). New variants take the next number and append
// to `ALL`; 1 (`Distributing`) and 12 (`Resting`) are retired and never
// reused.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum MidfielderState {
    Standing = 0,         // Where he starts: kick-off, coming on, back on his feet
    Dribbling = 2,        // Taking a man on
    AttackSupporting = 3, // Running beyond the ball in his side's attack
    SwitchingPlay = 4,    // Switching the play to the other side of the field
    Crossing = 5,         // Delivering a cross into the box
    Passing = 6,          // Picking and playing a pass
    Running = 7,          // On the ball
    DistanceShooting = 8, // Taking a shot from a long distance
    Pressing = 9,         // Closing down the carrier
    Tackling = 10,        // The challenge itself
    Returning = 11,       // Getting back to his place in the shape
    Walking = 13,         // Relaxed in his place while play is away
    TakeBall = 14,        // Collecting a loose ball, or a restart he takes
    Shooting = 15,        // Shooting
    Intercepting = 16,    // Stepping in to cut out a ball in flight
    CreatingSpace = 17,   // Moving away from the ball to offer a pass
    Guarding = 18,        // Marking a man
    Heading = 19,         // Contesting an aerial ball in midfield — knock-down, clearance or flick
    HoldingWidth = 20,    // Holding a touchline to stretch the defence — see `holding_width`
    Jockeying = 21,       // Containing the carrier, goal-side, choosing the moment
    HoldingShape = 22,    // Holding his place in the shape, on his toes
    TrackingBack = 23,    // Recovery run back to his place with the opposition on the ball
}

impl MidfielderState {
    /// Every variant in declared order — single source of truth for the
    /// state universe (transition-graph audit + id-stability snapshot).
    pub const ALL: [MidfielderState; 22] = [
        MidfielderState::Standing,
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
        MidfielderState::Walking,
        MidfielderState::TakeBall,
        MidfielderState::Shooting,
        MidfielderState::Intercepting,
        MidfielderState::CreatingSpace,
        MidfielderState::Guarding,
        MidfielderState::Heading,
        MidfielderState::HoldingWidth,
        MidfielderState::Jockeying,
        MidfielderState::HoldingShape,
        MidfielderState::TrackingBack,
    ];

    /// Length of any table indexed by `state as usize`: one past the last
    /// discriminant, which retired ids keep above `ALL.len()`.
    pub const SLOTS: usize = Self::ALL[Self::ALL.len() - 1] as usize + 1;

    pub fn from_slot(slot: usize) -> Option<MidfielderState> {
        Self::ALL.into_iter().find(|s| *s as usize == slot)
    }

    /// What he is doing in this state. The censuses group states by it, so
    /// a new state is classified here once and every table that groups
    /// states counts it.
    pub fn family(self) -> MidfielderFamily {
        match self {
            MidfielderState::Standing => MidfielderFamily::Start,
            MidfielderState::Walking | MidfielderState::HoldingShape => MidfielderFamily::Shape,
            MidfielderState::Guarding => MidfielderFamily::Mark,
            MidfielderState::Pressing => MidfielderFamily::Close,
            MidfielderState::Jockeying | MidfielderState::Tackling => MidfielderFamily::Duel,
            MidfielderState::Returning | MidfielderState::TrackingBack => MidfielderFamily::Recover,
            MidfielderState::AttackSupporting
            | MidfielderState::CreatingSpace
            | MidfielderState::HoldingWidth => MidfielderFamily::Support,
            MidfielderState::TakeBall => MidfielderFamily::Chase,
            MidfielderState::Intercepting => MidfielderFamily::Intercept,
            MidfielderState::Heading => MidfielderFamily::Aerial,
            MidfielderState::Running | MidfielderState::Dribbling => MidfielderFamily::Carry,
            MidfielderState::Passing
            | MidfielderState::SwitchingPlay
            | MidfielderState::Crossing
            | MidfielderState::Shooting
            | MidfielderState::DistanceShooting => MidfielderFamily::Play,
        }
    }
}

/// The groups the midfielder states fall into.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MidfielderFamily {
    /// Where he starts: kick-off, coming on.
    Start,
    /// Holding his place in the shape, relaxed or alert.
    Shape,
    Mark,
    /// The run to the carrier.
    Close,
    /// On the carrier: containing him or challenging.
    Duel,
    /// Getting back to his place.
    Recover,
    /// A job in his side's attack.
    Support,
    /// Collecting a loose ball.
    Chase,
    /// Stepping in to cut out a ball in flight.
    Intercept,
    Aerial,
    /// The ball at his feet.
    Carry,
    /// Playing the ball away.
    Play,
}

pub struct MidfielderStrategies {}

impl MidfielderStrategies {
    pub fn process(
        state: MidfielderState,
        state_processor: StateProcessor,
    ) -> StateProcessingResult {
        // A man holding his place, or getting back to it, moves at the pace
        // the play asks for (`ShapeTempo`): the state chooses where, the
        // tempo how fast.
        let tempo = (matches!(
            state.family(),
            MidfielderFamily::Shape | MidfielderFamily::Recover
        ) && !ShapeTempo::midfielders_off())
        .then(|| {
            let ctx = state_processor.ctx();
            let place = ctx.team().my_anchor();
            (
                ShapeTempo::midfielder(&ctx, place),
                place,
                ctx.player.position,
            )
        });
        let mut result = Self::dispatch(state, state_processor);
        // An imposed movement with its own effort floor (the keeper's release
        // spacing) keeps its pace.
        if result.effort_floor > 0.0 {
            return result;
        }
        if let (Some((tempo, place, position)), Some(velocity)) = (tempo, result.velocity) {
            result.velocity = Some(tempo.steer(position, place, velocity));
            result.effort_floor = tempo.effort();
        }
        result
    }

    fn dispatch(state: MidfielderState, state_processor: StateProcessor) -> StateProcessingResult {
        match state {
            MidfielderState::Standing => {
                state_processor.process(MidfielderStandingState::default())
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
            MidfielderState::Heading => state_processor.process(MidfielderHeadingState::default()),
            MidfielderState::HoldingWidth => {
                state_processor.process(MidfielderHoldingWidthState::default())
            }
            MidfielderState::Jockeying => {
                state_processor.process(MidfielderJockeyingState::default())
            }
            MidfielderState::HoldingShape => {
                state_processor.process(MidfielderHoldingShapeState::default())
            }
            MidfielderState::TrackingBack => {
                state_processor.process(MidfielderTrackingBackState::default())
            }
        }
    }
}

impl Display for MidfielderState {
    fn fmt(&self, f: &mut Formatter) -> Result {
        match self {
            MidfielderState::Standing => write!(f, "Standing"),
            MidfielderState::AttackSupporting => write!(f, "Supporting Attack"),
            MidfielderState::SwitchingPlay => write!(f, "Switching Play"),
            MidfielderState::Crossing => write!(f, "Crossing"),
            MidfielderState::Passing => write!(f, "Passing"),
            MidfielderState::Pressing => write!(f, "Pressing"),
            MidfielderState::Tackling => write!(f, "Tackling"),
            MidfielderState::DistanceShooting => write!(f, "DistanceShooting"),
            MidfielderState::Returning => write!(f, "Returning"),
            MidfielderState::Walking => write!(f, "Walking"),
            MidfielderState::Running => write!(f, "Running"),
            MidfielderState::TakeBall => write!(f, "Take Ball"),
            MidfielderState::Dribbling => write!(f, "Dribbling"),
            MidfielderState::Shooting => write!(f, "Shooting"),
            MidfielderState::Intercepting => write!(f, "Intercepting"),
            MidfielderState::CreatingSpace => write!(f, "Creating Space"),
            MidfielderState::Guarding => write!(f, "Guarding"),
            MidfielderState::Heading => write!(f, "Heading"),
            MidfielderState::HoldingWidth => write!(f, "Holding Width"),
            MidfielderState::Jockeying => write!(f, "Jockeying"),
            MidfielderState::HoldingShape => write!(f, "Holding Shape"),
            MidfielderState::TrackingBack => write!(f, "Tracking Back"),
        }
    }
}
