use crate::r#match::forwarders::states::{
    ForwardCreatingSpaceState, ForwardCrossReceivingState, ForwardCrossingState,
    ForwardDribblingState, ForwardFinishingState, ForwardHeadingState, ForwardHoldingShapeState,
    ForwardHoldingWidthState, ForwardInterceptingState, ForwardJockeyingState, ForwardPassingState,
    ForwardPressingState, ForwardReturningState, ForwardRunningInBehindState,
    ForwardRunningState, ForwardShootingState, ForwardStandingState, ForwardTacklingState,
    ForwardTakeBallState, ForwardWalkingState,
};
use crate::r#match::common_states::ShapeTempo;
use crate::r#match::{StateProcessingResult, StateProcessor};
use std::fmt::Result;
use std::fmt::{Display, Formatter};

// Explicit discriminants: `PlayerState::compact_id` casts these via
// `as u16`, and the census tables index on them (a replay stores the
// state's name instead). Pinning each value to its variant means inserting
// or reordering a state can never silently renumber the others. New variants MUST take the next free
// number and be appended to `ALL`; `compact_id_snapshot` in
// `player/state.rs` fails loudly if any value moves.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ForwardState {
    Standing = 0,        // Standing in his place; hands him on after 0.2 s
    Walking = 1,         // Walking to his place while nobody owns the ball
    Passing = 2,         // Picking and playing a pass
    Dribbling = 3,       // Taking a man on
    Shooting = 4,        // Taking a shot on goal
    Heading = 5,         // Going up for a header
    RunningInBehind = 6, // Making a run behind the last line
    Running = 7,         // On the ball, or between jobs off it
    Pressing = 8,        // Closing down the carrier
    Finishing = 9,       // A finish from inside six metres
    CreatingSpace = 10,  // Showing for the ball, or a box place, or a gap between opponents
    CrossReceiving = 11, // Positioning to receive a cross
    Crossing = 12,       // Delivering a cross from a wide position
    Tackling = 13,       // Containing the carrier and challenging
    TakeBall = 15,       // Collecting a loose ball, or a restart he takes
    Intercepting = 16,   // Stepping in to cut out a ball in flight
    Returning = 17,      // Getting back to his place
    HoldingWidth = 19,   // Holding a touchline to stretch the defence — see `holding_width`
    Jockeying = 20,      // Containing the carrier, goal-side, waiting for the moment
    HoldingShape = 21,   // Alert in his place: the play is in reach of it
}

impl ForwardState {
    /// Every variant in declared order — the single source of truth for
    /// the state universe (transition-graph audit + id-stability snapshot).
    pub const ALL: [ForwardState; 20] = [
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
        ForwardState::TakeBall,
        ForwardState::Intercepting,
        ForwardState::Returning,
        ForwardState::HoldingWidth,
        ForwardState::Jockeying,
        ForwardState::HoldingShape,
    ];

    /// Length of any table indexed by `state as usize`: one past the last
    /// discriminant, which retired ids keep above `ALL.len()`.
    pub const SLOTS: usize = Self::ALL[Self::ALL.len() - 1] as usize + 1;

    pub fn from_slot(slot: usize) -> Option<ForwardState> {
        Self::ALL.into_iter().find(|s| *s as usize == slot)
    }

    /// What he is doing in this state. The censuses group states by it, so
    /// a new state is classified here once and every table that groups
    /// states counts it.
    pub fn family(self) -> ForwardFamily {
        match self {
            ForwardState::Standing => ForwardFamily::Start,
            ForwardState::Walking | ForwardState::HoldingShape => ForwardFamily::Shape,
            ForwardState::Pressing => ForwardFamily::Close,
            ForwardState::Tackling | ForwardState::Jockeying => ForwardFamily::Duel,
            ForwardState::Returning => ForwardFamily::Recover,
            ForwardState::RunningInBehind
            | ForwardState::CreatingSpace
            | ForwardState::CrossReceiving
            | ForwardState::HoldingWidth => ForwardFamily::Job,
            ForwardState::TakeBall => ForwardFamily::Chase,
            ForwardState::Intercepting => ForwardFamily::Intercept,
            ForwardState::Heading => ForwardFamily::Aerial,
            ForwardState::Running | ForwardState::Dribbling => ForwardFamily::Carry,
            ForwardState::Passing
            | ForwardState::Shooting
            | ForwardState::Finishing
            | ForwardState::Crossing => ForwardFamily::Play,
        }
    }
}

/// The groups the forward states fall into.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ForwardFamily {
    /// Where he starts: kick-off, coming on.
    Start,
    /// Holding his place in the shape.
    Shape,
    /// The run to the carrier.
    Close,
    /// On the carrier: containing him or challenging.
    Duel,
    /// Getting back to his place.
    Recover,
    /// A job in his side's attack: the run in behind, a place in the box,
    /// the short outlet, a touchline.
    Job,
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

pub struct ForwardStrategies {}

impl ForwardStrategies {
    pub fn process(state: ForwardState, state_processor: StateProcessor) -> StateProcessingResult {
        // A man holding his place, or getting back to it, moves at the pace
        // the play asks for (`ShapeTempo`): the state chooses where, the
        // tempo how fast.
        let tempo = (matches!(state.family(), ForwardFamily::Shape | ForwardFamily::Recover)
            && !ShapeTempo::forwards_off())
        .then(|| {
            let ctx = state_processor.ctx();
            let place = ctx.team().my_anchor();
            (ShapeTempo::forward(&ctx, place), place, ctx.player.position)
        });
        let mut result = Self::dispatch(state, state_processor);
        // An imposed movement with its own effort floor keeps its pace.
        if result.effort_floor > 0.0 {
            return result;
        }
        if let (Some((tempo, place, position)), Some(velocity)) = (tempo, result.velocity) {
            result.velocity = Some(tempo.steer(position, place, velocity));
            result.effort_floor = tempo.effort();
        }
        result
    }

    fn dispatch(state: ForwardState, state_processor: StateProcessor) -> StateProcessingResult {
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
            ForwardState::Running => state_processor.process(ForwardRunningState::default()),
            ForwardState::TakeBall => state_processor.process(ForwardTakeBallState::default()),
            ForwardState::Intercepting => {
                state_processor.process(ForwardInterceptingState::default())
            }
            ForwardState::Returning => state_processor.process(ForwardReturningState::default()),
            ForwardState::HoldingWidth => {
                state_processor.process(ForwardHoldingWidthState::default())
            }
            ForwardState::Jockeying => state_processor.process(ForwardJockeyingState::default()),
            ForwardState::HoldingShape => {
                state_processor.process(ForwardHoldingShapeState::default())
            }
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
            ForwardState::Passing => write!(f, "Passing"),
            ForwardState::Tackling => write!(f, "Tackling"),
            ForwardState::Running => write!(f, "Running"),
            ForwardState::TakeBall => write!(f, "Take Ball"),
            ForwardState::Intercepting => write!(f, "Intercepting"),
            ForwardState::Returning => write!(f, "Returning"),
            ForwardState::HoldingWidth => write!(f, "Holding Width"),
            ForwardState::Jockeying => write!(f, "Jockeying"),
            ForwardState::HoldingShape => write!(f, "Holding Shape"),
        }
    }
}
