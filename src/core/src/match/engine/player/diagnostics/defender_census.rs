//! **The defender action census** — who takes the ball off a carrier and by
//! which rule, how a defender's duel with a carrier runs, how each header is
//! decided, the job the assignment gives him against the state he is in,
//! which situations each state acts on, how far and how fast he moves, his
//! readiness phase, the loose-ball chases his own doors start, and where a
//! ball knocked off him comes to rest.
//!
//! The open episodes are per match and live on
//! [`MatchContext`](crate::r#match::MatchContext), because matches run in
//! parallel. The totals are process-wide counters the `dev_match` harness
//! prints, summed over every match of a run. Nothing here changes what the
//! match does.

use crate::PlayerFieldPositionGroup;
use crate::PlayerPositionType;
use crate::r#match::defenders::states::DefenderState;
use crate::r#match::engine::teamplay::plans::defence::{DefensiveDuty, DefensivePlan};
use crate::r#match::player::diagnostics::forward_census::ForwardCensus;
use crate::r#match::player::diagnostics::midfielder_census::MidfielderCensus;
use crate::r#match::player::state::PlayerState;
use crate::r#match::player::strategies::players::ops::defending::defensive::DefensiveRole;
use crate::r#match::player::strategies::processor::TakeBallYield;
use crate::r#match::shared_states::{ChallengeKind, TackleEngagement, TackleOutcome};
use crate::r#match::{
    GameTickContext, MatchField, MatchPlayer, MatchTally, PenaltyArea, PlayerSide,
    StateProcessingContext, StateProcessingResult,
};
use nalgebra::Vector3;
use std::sync::atomic::{AtomicU64, Ordering};

/// The rule that put the ball with the other side.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TakeRule {
    BlockTackle,
    RecoveryChallenge,
    /// A tackle won from another role's challenge state.
    Tackle,
    /// `check_ball_ownership`'s takeover of an owned ball.
    ScanTakeover,
    /// `PlayerEvent::ClaimBall` on a ball somebody owns.
    ClaimOwned,
    Interception,
    PassBlock,
    ShotBlock,
    /// Nothing named it: a ball nobody held, claimed.
    Loose,
}

impl TakeRule {
    pub const COUNT: usize = 9;
    pub const LABELS: [&'static str; Self::COUNT] = [
        "block tackle",
        "recovery challenge",
        "tackle (another role)",
        "scan takeover",
        "ClaimBall on an owned ball",
        "interception",
        "pass block",
        "shot block",
        "loose ball",
    ];
}

/// How a challenge a defender's state rolled came out.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChallengeOutcome {
    Won,
    Missed,
    Foul,
}

impl ChallengeOutcome {
    pub fn of(outcome: &TackleOutcome) -> ChallengeOutcome {
        match outcome {
            TackleOutcome::Won => ChallengeOutcome::Won,
            TackleOutcome::Missed => ChallengeOutcome::Missed,
            TackleOutcome::Foul(_) => ChallengeOutcome::Foul,
        }
    }
}

/// What the taker's statistics gained for a turnover.
pub struct Credit;

impl Credit {
    pub const COUNT: usize = 5;
    pub const LABELS: [&'static str; Self::COUNT] =
        ["tackle", "interception", "block", "clearance", "nothing"];
}

/// How a defender's spell on a carrier ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SpellEnd {
    Won,
    Missed,
    Foul,
    Released,
    Away,
    Passed,
}

impl SpellEnd {
    pub const COUNT: usize = 6;
    pub const LABELS: [&'static str; Self::COUNT] = [
        "won",
        "missed",
        "foul",
        "carrier released",
        "carrier away",
        "job passed",
    ];
}

/// The rule that decided a header.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HeaderRule {
    CrossContest,
    AerialDuel,
    HeadingShot,
    OwnershipClaim,
}

impl HeaderRule {
    pub const COUNT: usize = 4;
    pub const LABELS: [&'static str; Self::COUNT] = [
        "cross contest",
        "aerial duel",
        "heading state's shot",
        "ownership claim",
    ];
}

/// What a header was booked as.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HeaderBooking {
    Clearance,
    Shot,
    Nothing,
    Possession,
}

impl HeaderBooking {
    pub const COUNT: usize = 4;
    pub const LABELS: [&'static str; Self::COUNT] = ["clearance", "shot", "nothing", "possession"];
}

/// The situations the door census puts to every defender state.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Situation {
    /// A carrier within `TackleEngagement::COMMIT` he is licensed to engage.
    LicensedCarrier,
    /// A loose ball the chase table gives him.
    LooseBall,
    /// A high ball a contest has awarded him.
    HighBall,
    /// His own side has the ball and he does not.
    OwnPossession,
    /// A carrier in his own area, him one of the two nearest.
    CarrierInArea,
}

impl Situation {
    pub const COUNT: usize = 5;
    pub const LABELS: [&'static str; Self::COUNT] = [
        "licensed carrier in reach",
        "loose ball he is given",
        "high ball he has won",
        "own side in possession",
        "carrier in his area",
    ];
    const ALL: [Situation; Self::COUNT] = [
        Situation::LicensedCarrier,
        Situation::LooseBall,
        Situation::HighBall,
        Situation::OwnPossession,
        Situation::CarrierInArea,
    ];

    /// Whether the state he is in, or the one his handler chose, answers
    /// the situation.
    fn answered_by(self, state: PlayerState) -> bool {
        let PlayerState::Defender(s) = state else {
            return false;
        };
        match self {
            Situation::LicensedCarrier | Situation::CarrierInArea => {
                DefenderCensus::is_duel_state(s) || s == DefenderState::Pressing
            }
            Situation::LooseBall => {
                matches!(s, DefenderState::TakeBall | DefenderState::Intercepting)
            }
            Situation::HighBall => s == DefenderState::Heading,
            Situation::OwnPossession => !matches!(
                s,
                DefenderState::Marking
                    | DefenderState::Covering
                    | DefenderState::Pressing
                    | DefenderState::Jockeying
                    | DefenderState::Tackling
            ),
        }
    }
}

/// Why the dispatcher threw a defender out of a chase his own door started.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChaseYield {
    TeammatesPass,
    Hysteresis,
    RecollectBar,
    Restart,
    AerialAward,
}

impl ChaseYield {
    pub const COUNT: usize = 5;
    pub const LABELS: [&'static str; Self::COUNT] = [
        "his side's pass to another man",
        "hysteresis margin",
        "recollect bar",
        "restart",
        "a header given to his side",
    ];
}

/// What the dispatcher did to a player before his state's handler ran.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DispatchRedirect {
    None,
    Force,
    Yield(ChaseYield),
    Dispossessed,
}

/// A ball that came off a defender without his keeping it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DeflectionKind {
    BlockKept,
    BlockBehind,
    BlockSideways,
    BlockLoose,
    BlockOn,
    TackleKnock,
}

impl DeflectionKind {
    pub const COUNT: usize = 6;
    pub const LABELS: [&'static str; Self::COUNT] = [
        "blocked, kept",
        "blocked, turned behind",
        "blocked, sideways",
        "blocked, dropped loose",
        "blocked, carried on",
        "tackle knock",
    ];
}

/// How a deflected ball's travel ended.
pub struct DeflectionEnd;

impl DeflectionEnd {
    pub const COUNT: usize = 4;
    pub const LABELS: [&'static str; Self::COUNT] =
        ["touched", "came to rest", "went out", "timed out"];
    const TOUCHED: usize = 0;
    const STOPPED: usize = 1;
    const OUT: usize = 2;
    const TIMED_OUT: usize = 3;
}

/// A defender's place in the back line, by his position in the formation.
pub struct BackLine;

impl BackLine {
    pub const COUNT: usize = 3;
    pub const LABELS: [&'static str; Self::COUNT] =
        ["centre-back", "full-back", "defensive midfielder"];

    pub fn of(position: PlayerPositionType) -> Option<usize> {
        match position {
            PlayerPositionType::DefenderCenter
            | PlayerPositionType::DefenderCenterLeft
            | PlayerPositionType::DefenderCenterRight
            | PlayerPositionType::Sweeper => Some(0),
            PlayerPositionType::DefenderLeft | PlayerPositionType::DefenderRight => Some(1),
            PlayerPositionType::DefensiveMidfielder => Some(2),
            _ => None,
        }
    }
}

/// A defender's readiness on one tick.
pub struct Phase;

impl Phase {
    pub const COUNT: usize = 7;
    pub const LABELS: [&'static str; Self::COUNT] = [
        "relaxed",
        "alert",
        "on a job",
        "in a duel",
        "recovering",
        "on the ball",
        "other",
    ];
    const RELAXED: usize = 0;
    const ALERT: usize = 1;

    fn of(state: PlayerState, has_ball: bool) -> usize {
        if has_ball {
            return 5;
        }
        match state {
            PlayerState::Defender(DefenderState::Walking) => Self::RELAXED,
            PlayerState::Defender(DefenderState::HoldingLine) => Self::ALERT,
            PlayerState::Defender(
                DefenderState::Marking | DefenderState::Covering | DefenderState::Pressing,
            ) => 2,
            PlayerState::Defender(s) if DefenderCensus::is_duel_state(s) => 3,
            PlayerState::Recovering
            | PlayerState::Defender(DefenderState::TrackingBack | DefenderState::Returning) => 4,
            PlayerState::Defender(
                DefenderState::Passing
                | DefenderState::Clearing
                | DefenderState::Shooting
                | DefenderState::Crossing,
            ) => 5,
            _ => 6,
        }
    }
}

/// The plan's duty, as the assignment census reads it.
pub struct Duty;

impl Duty {
    pub const COUNT: usize = 5;
    pub const COVER: usize = 1;
    pub const LABELS: [&'static str; Self::COUNT] =
        ["press", "cover", "mark", "hold zone", "no live plan"];

    pub fn of(plan: &DefensivePlan, id: u32) -> usize {
        if !plan.active {
            return 4;
        }
        match plan.duty_of(id) {
            DefensiveDuty::Press => 0,
            DefensiveDuty::Cover => Self::COVER,
            DefensiveDuty::Mark(_) => 2,
            DefensiveDuty::HoldZone => 3,
        }
    }
}

/// The per-tick distance ranking, as the assignment census reads it.
pub struct Rank;

impl Rank {
    pub const COUNT: usize = 4;
    pub const LABELS: [&'static str; Self::COUNT] = ["Primary", "Cover", "Help", "Hold"];

    fn of(role: DefensiveRole) -> usize {
        match role {
            DefensiveRole::Primary => 0,
            DefensiveRole::Cover => 1,
            DefensiveRole::Help => 2,
            DefensiveRole::Hold => 3,
        }
    }
}

#[derive(Clone, Copy, Debug)]
struct ChallengeNote {
    defender: u32,
    carrier: u32,
    kind: ChallengeKind,
    outcome: ChallengeOutcome,
    tick: u64,
}

#[derive(Clone, Copy, Debug)]
struct DuelSpell {
    carrier: u32,
    pressing: u64,
    tackling: u64,
    last: Option<ChallengeNote>,
}

#[derive(Clone, Copy, Debug)]
struct PhaseTrack {
    phase: usize,
    since: u64,
    previous: Option<usize>,
}

#[derive(Clone, Copy, Debug)]
struct ChaseEntry {
    door: usize,
    ai_ticks: u8,
    confirmed: bool,
}

#[derive(Clone, Copy, Debug)]
struct DefenderWatch {
    id: u32,
    trail: [Vector3<f32>; DefenderCensus::TRAIL],
    trail_len: usize,
    trail_at: usize,
    last_tick: u64,
    last_pos: Vector3<f32>,
    seen_state: Option<PlayerState>,
    phase: Option<PhaseTrack>,
    sprint_ticks: u32,
    spell: Option<DuelSpell>,
    duty: Option<usize>,
    rank: Option<usize>,
    chase: Option<ChaseEntry>,
}

impl DefenderWatch {
    fn new(id: u32) -> Self {
        DefenderWatch {
            id,
            trail: [Vector3::zeros(); DefenderCensus::TRAIL],
            trail_len: 0,
            trail_at: 0,
            last_tick: 0,
            last_pos: Vector3::zeros(),
            seen_state: None,
            phase: None,
            sprint_ticks: 0,
            spell: None,
            duty: None,
            rank: None,
            chase: None,
        }
    }

    /// Whether this sample follows a gap: dead time runs no tick body, and
    /// the restart that ends it relocates him.
    fn push(&mut self, at: Vector3<f32>, tick: u64) -> bool {
        let resumed = self.last_tick + 1 != tick;
        if resumed {
            self.trail_len = 0;
            self.trail_at = 0;
        }
        self.last_tick = tick;
        self.trail[self.trail_at] = at;
        self.trail_at = (self.trail_at + 1) % DefenderCensus::TRAIL;
        self.trail_len = (self.trail_len + 1).min(DefenderCensus::TRAIL);
        resumed
    }

    /// Ground speed over the trail, in metres a second. A velocity dithers
    /// in a slot; displacement over a quarter second is how he moves.
    fn speed(&self) -> f32 {
        if self.trail_len < 2 {
            return 0.0;
        }
        let newest = (self.trail_at + DefenderCensus::TRAIL - 1) % DefenderCensus::TRAIL;
        let oldest = if self.trail_len < DefenderCensus::TRAIL {
            0
        } else {
            self.trail_at
        };
        let (a, b) = (self.trail[newest], self.trail[oldest]);
        (a.x - b.x).hypot(a.y - b.y) * DefenderCensus::METRES_PER_UNIT
            / ((self.trail_len - 1) as f32 * DefenderCensus::SECONDS_PER_TICK)
    }
}

#[derive(Clone, Copy, Debug)]
struct Holder {
    id: u32,
    team: u32,
    since: u64,
}

#[derive(Clone, Copy, Debug)]
struct PendingTurnover {
    taker: u32,
    rule: usize,
    due: u64,
    before: [u16; 4],
}

#[derive(Clone, Copy, Debug)]
struct DeflectionTrack {
    kind: DeflectionKind,
    origin: Vector3<f32>,
    opened: u64,
}

/// Everything the census has counted, for the harness.
#[derive(Clone, Debug)]
pub struct DefenderCensusSnapshot {
    pub turnovers: [[u64; 2]; TakeRule::COUNT],
    pub take_role: [[u64; 4]; TakeRule::COUNT],
    pub take_def_state: [[u64; DefenderState::SLOTS]; TakeRule::COUNT],
    pub take_gap_mm: [u64; TakeRule::COUNT],
    pub take_tenure_ticks: [u64; TakeRule::COUNT],
    pub take_prior: [[u64; 4]; TakeRule::COUNT],
    pub take_credit: [[u64; Credit::COUNT]; TakeRule::COUNT],
    pub spells: u64,
    pub spell_pressing_ticks: u64,
    pub spell_tackling_ticks: u64,
    pub spell_ends: [u64; SpellEnd::COUNT],
    pub tackling_entries: u64,
    pub challenges: [u64; 2],
    pub challenge_outcomes: [[u64; 3]; 2],
    pub challenge_speed_cmps: u64,
    pub challenge_goal_side: u64,
    pub headers: [u64; HeaderRule::COUNT],
    pub header_state: [[u64; DefenderCensus::HEADER_STATES]; HeaderRule::COUNT],
    pub header_leap: [u64; HeaderRule::COUNT],
    pub header_booked: [[u64; HeaderBooking::COUNT]; HeaderRule::COUNT],
    pub flights_both: u64,
    pub heading_rolls: u64,
    pub heading_rerolls: u64,
    pub assignment: [[u64; Rank::COUNT]; Duty::COUNT],
    pub assignment_state: [[u64; DefenderState::SLOTS]; Duty::COUNT],
    pub changes_out_of_possession: u64,
    pub changes_after_rank: u64,
    pub changes_after_duty: u64,
    pub changes_after_both: u64,
    pub press_hands: u64,
    pub cover_hands: u64,
    pub cover_to_press: u64,
    pub plan_ticks: u64,
    pub carrier_ticks: [u64; 2],
    pub crowded_ticks: [u64; 2],
    pub doors: [[[u64; 2]; DefenderState::SLOTS]; Situation::COUNT],
    pub motion_ticks: [[u64; 2]; BackLine::COUNT],
    pub motion_path_mm: [[u64; 2]; BackLine::COUNT],
    pub motion_bands: [[[u64; DefenderCensus::BANDS]; 2]; BackLine::COUNT],
    pub sprints: [[u64; 2]; BackLine::COUNT],
    pub state_ticks: [u64; DefenderState::SLOTS + 1],
    pub state_ball_ticks: [u64; DefenderState::SLOTS + 1],
    /// Passes struck with a team-mate offside, by the state of the man
    /// whose depth set the line.
    pub offside_line: [u64; DefenderState::SLOTS + 1],
    pub state_speed_cmps: [u64; DefenderState::SLOTS + 1],
    pub phase_ticks: [u64; Phase::COUNT],
    pub relaxed_alert_changes: u64,
    pub relaxed_alert_reversed: u64,
    pub chase_entries: [u64; DefenderState::SLOTS],
    pub chase_reversed: [[u64; ChaseYield::COUNT]; DefenderState::SLOTS],
    pub deflections: [u64; DeflectionKind::COUNT],
    pub deflection_speed_x1000: [u64; DeflectionKind::COUNT],
    pub deflection_fast: [u64; DeflectionKind::COUNT],
    pub deflection_gap_mm: [u64; DeflectionKind::COUNT],
    pub deflection_beyond_lunge: [u64; DeflectionKind::COUNT],
    pub deflection_travel_mm: [u64; DeflectionKind::COUNT],
    pub deflection_ends: [[u64; DeflectionEnd::COUNT]; DeflectionKind::COUNT],
    pub deflection_travel: [[u64; DefenderCensus::TRAVEL_BINS]; DeflectionKind::COUNT],
}

const SLOTS: usize = DefenderState::SLOTS;

static TURNOVERS: [[AtomicU64; 2]; TakeRule::COUNT] =
    [const { [const { AtomicU64::new(0) }; 2] }; TakeRule::COUNT];
static TAKE_ROLE: [[AtomicU64; 4]; TakeRule::COUNT] =
    [const { [const { AtomicU64::new(0) }; 4] }; TakeRule::COUNT];
static TAKE_DEF_STATE: [[AtomicU64; SLOTS]; TakeRule::COUNT] =
    [const { [const { AtomicU64::new(0) }; SLOTS] }; TakeRule::COUNT];
static TAKE_GAP_MM: [AtomicU64; TakeRule::COUNT] = [const { AtomicU64::new(0) }; TakeRule::COUNT];
static TAKE_TENURE_TICKS: [AtomicU64; TakeRule::COUNT] =
    [const { AtomicU64::new(0) }; TakeRule::COUNT];
static TAKE_PRIOR: [[AtomicU64; 4]; TakeRule::COUNT] =
    [const { [const { AtomicU64::new(0) }; 4] }; TakeRule::COUNT];
static TAKE_CREDIT: [[AtomicU64; Credit::COUNT]; TakeRule::COUNT] =
    [const { [const { AtomicU64::new(0) }; Credit::COUNT] }; TakeRule::COUNT];
static SPELLS: AtomicU64 = AtomicU64::new(0);
static SPELL_PRESSING_TICKS: AtomicU64 = AtomicU64::new(0);
static SPELL_TACKLING_TICKS: AtomicU64 = AtomicU64::new(0);
static SPELL_ENDS: [AtomicU64; SpellEnd::COUNT] = [const { AtomicU64::new(0) }; SpellEnd::COUNT];
static TACKLING_ENTRIES: AtomicU64 = AtomicU64::new(0);
static CHALLENGES: [AtomicU64; 2] = [const { AtomicU64::new(0) }; 2];
static CHALLENGE_OUTCOMES: [[AtomicU64; 3]; 2] = [const { [const { AtomicU64::new(0) }; 3] }; 2];
static CHALLENGE_SPEED_CMPS: AtomicU64 = AtomicU64::new(0);
static CHALLENGE_GOAL_SIDE: AtomicU64 = AtomicU64::new(0);
static HEADERS: [AtomicU64; HeaderRule::COUNT] = [const { AtomicU64::new(0) }; HeaderRule::COUNT];
static HEADER_STATE: [[AtomicU64; DefenderCensus::HEADER_STATES]; HeaderRule::COUNT] =
    [const { [const { AtomicU64::new(0) }; DefenderCensus::HEADER_STATES] }; HeaderRule::COUNT];
static HEADER_LEAP: [AtomicU64; HeaderRule::COUNT] =
    [const { AtomicU64::new(0) }; HeaderRule::COUNT];
static HEADER_BOOKED: [[AtomicU64; HeaderBooking::COUNT]; HeaderRule::COUNT] =
    [const { [const { AtomicU64::new(0) }; HeaderBooking::COUNT] }; HeaderRule::COUNT];
static FLIGHTS_BOTH: AtomicU64 = AtomicU64::new(0);
static HEADING_ROLLS: AtomicU64 = AtomicU64::new(0);
static HEADING_REROLLS: AtomicU64 = AtomicU64::new(0);
static ASSIGNMENT: [[AtomicU64; Rank::COUNT]; Duty::COUNT] =
    [const { [const { AtomicU64::new(0) }; Rank::COUNT] }; Duty::COUNT];
static ASSIGNMENT_STATE: [[AtomicU64; SLOTS]; Duty::COUNT] =
    [const { [const { AtomicU64::new(0) }; SLOTS] }; Duty::COUNT];
static CHANGES_OUT_OF_POSSESSION: AtomicU64 = AtomicU64::new(0);
static CHANGES_AFTER_RANK: AtomicU64 = AtomicU64::new(0);
static CHANGES_AFTER_DUTY: AtomicU64 = AtomicU64::new(0);
static CHANGES_AFTER_BOTH: AtomicU64 = AtomicU64::new(0);
static PRESS_HANDS: AtomicU64 = AtomicU64::new(0);
static COVER_HANDS: AtomicU64 = AtomicU64::new(0);
static COVER_TO_PRESS: AtomicU64 = AtomicU64::new(0);
static PLAN_TICKS: AtomicU64 = AtomicU64::new(0);
static CARRIER_TICKS: [AtomicU64; 2] = [const { AtomicU64::new(0) }; 2];
static CROWDED_TICKS: [AtomicU64; 2] = [const { AtomicU64::new(0) }; 2];
static DOORS: [[[AtomicU64; 2]; SLOTS]; Situation::COUNT] =
    [const { [const { [const { AtomicU64::new(0) }; 2] }; SLOTS] }; Situation::COUNT];
static MOTION_TICKS: [[AtomicU64; 2]; BackLine::COUNT] =
    [const { [const { AtomicU64::new(0) }; 2] }; BackLine::COUNT];
static MOTION_PATH_MM: [[AtomicU64; 2]; BackLine::COUNT] =
    [const { [const { AtomicU64::new(0) }; 2] }; BackLine::COUNT];
static MOTION_BANDS: [[[AtomicU64; DefenderCensus::BANDS]; 2]; BackLine::COUNT] =
    [const { [const { [const { AtomicU64::new(0) }; DefenderCensus::BANDS] }; 2] };
        BackLine::COUNT];
static SPRINTS: [[AtomicU64; 2]; BackLine::COUNT] =
    [const { [const { AtomicU64::new(0) }; 2] }; BackLine::COUNT];
static STATE_TICKS: [AtomicU64; SLOTS + 1] = [const { AtomicU64::new(0) }; SLOTS + 1];
static STATE_BALL_TICKS: [AtomicU64; SLOTS + 1] = [const { AtomicU64::new(0) }; SLOTS + 1];
static OFFSIDE_LINE: [AtomicU64; SLOTS + 1] = [const { AtomicU64::new(0) }; SLOTS + 1];
static STATE_SPEED_CMPS: [AtomicU64; SLOTS + 1] = [const { AtomicU64::new(0) }; SLOTS + 1];
static PHASE_TICKS: [AtomicU64; Phase::COUNT] = [const { AtomicU64::new(0) }; Phase::COUNT];
static RELAXED_ALERT_CHANGES: AtomicU64 = AtomicU64::new(0);
static RELAXED_ALERT_REVERSED: AtomicU64 = AtomicU64::new(0);
static CHASE_ENTRIES: [AtomicU64; SLOTS] = [const { AtomicU64::new(0) }; SLOTS];
static CHASE_REVERSED: [[AtomicU64; ChaseYield::COUNT]; SLOTS] =
    [const { [const { AtomicU64::new(0) }; ChaseYield::COUNT] }; SLOTS];
static DEFLECTIONS: [AtomicU64; DeflectionKind::COUNT] =
    [const { AtomicU64::new(0) }; DeflectionKind::COUNT];
static DEFLECTION_SPEED_X1000: [AtomicU64; DeflectionKind::COUNT] =
    [const { AtomicU64::new(0) }; DeflectionKind::COUNT];
static DEFLECTION_FAST: [AtomicU64; DeflectionKind::COUNT] =
    [const { AtomicU64::new(0) }; DeflectionKind::COUNT];
static DEFLECTION_GAP_MM: [AtomicU64; DeflectionKind::COUNT] =
    [const { AtomicU64::new(0) }; DeflectionKind::COUNT];
static DEFLECTION_BEYOND_LUNGE: [AtomicU64; DeflectionKind::COUNT] =
    [const { AtomicU64::new(0) }; DeflectionKind::COUNT];
static DEFLECTION_TRAVEL_MM: [AtomicU64; DeflectionKind::COUNT] =
    [const { AtomicU64::new(0) }; DeflectionKind::COUNT];
static DEFLECTION_ENDS: [[AtomicU64; DeflectionEnd::COUNT]; DeflectionKind::COUNT] =
    [const { [const { AtomicU64::new(0) }; DeflectionEnd::COUNT] }; DeflectionKind::COUNT];
static DEFLECTION_TRAVEL: [[AtomicU64; DefenderCensus::TRAVEL_BINS]; DeflectionKind::COUNT] =
    [const { [const { AtomicU64::new(0) }; DefenderCensus::TRAVEL_BINS] }; DeflectionKind::COUNT];

/// The open episodes of one match.
#[derive(Debug, Default)]
pub struct DefenderCensus {
    watches: Vec<DefenderWatch>,
    holder: Option<Holder>,
    owner_last_tick: Option<u32>,
    take_tag: Option<(TakeRule, u32, u64)>,
    challenges: Vec<ChallengeNote>,
    pending: Vec<PendingTurnover>,
    stats: Vec<(u32, [u16; 4])>,
    plan_hands: [(Option<u32>, Option<u32>); 2],
    duel_flight: Option<u64>,
    contest_flight: Option<u64>,
    award: Option<(u32, u64)>,
    deflection: Option<DeflectionTrack>,
    live_ticks: u64,
    slow_live_ticks: u64,
}

impl DefenderCensus {
    /// Every defender state slot, then a defender in a state that is not a
    /// defender's, a midfielder and a forward.
    pub const HEADER_STATES: usize = SLOTS + 3;
    /// The furthest a man can reach to block a ball (2 m), as
    /// `Ball::BLOCK_REACH` has it.
    const LUNGE: f32 = 16.0;
    /// Speed band upper edges in m/s: standing, a walk, a jog, a run, and
    /// above that a sprint.
    pub const BAND_EDGES: [f32; 4] = [0.2, 2.0, 4.0, 5.5];
    pub const BANDS: usize = 5;
    pub const BAND_LABELS: [&'static str; Self::BANDS] =
        ["standing", "to 2 m/s", "to 4 m/s", "to 5.5 m/s", "above"];
    /// Travel bins of a deflected ball, upper edges in metres.
    pub const TRAVEL_EDGES: [f32; 5] = [2.0, 8.0, 20.0, 40.0, 80.0];
    pub const TRAVEL_BINS: usize = 6;
    pub const TRAVEL_LABELS: [&'static str; Self::TRAVEL_BINS] =
        ["<2m", "2-8m", "8-20m", "20-40m", "40-80m", "80m+"];
    /// A sprint is a second above the top band's edge; it ends when he drops
    /// below a jog's.
    const SPRINT_TICKS: u32 = 100;
    /// A quarter second: the window a speed is read over.
    const TRAIL: usize = 26;
    const METRES_PER_UNIT: f32 = 0.125;
    const SECONDS_PER_TICK: f32 = 0.01;
    /// 300 ms: a phase change undone inside this is a two-cycle.
    const REVERSAL_TICKS: u64 = 30;
    /// Half a second, in engine ticks: a challenge this recent is the one a
    /// turnover follows.
    const PRIOR_TICKS: u64 = 50;
    /// Engine ticks a turnover's credit has to land in.
    const CREDIT_TICKS: u64 = 5;
    /// AI ticks a door's chase is watched for a yield.
    const CHASE_AI_TICKS: u8 = 10;
    /// A deflection nobody touches for ten seconds is no longer one.
    const DEFLECTION_TICKS: u64 = 1000;
    /// 0.01 u/tick is 12.5 mm/s: a ball at rest.
    const AT_REST: f32 = 0.01;

    pub fn is_duel_state(state: DefenderState) -> bool {
        matches!(state, DefenderState::Jockeying | DefenderState::Tackling)
    }

    fn bump(counter: &AtomicU64) {
        counter.fetch_add(1, Ordering::Relaxed);
    }

    fn side_index(side: PlayerSide) -> usize {
        match side {
            PlayerSide::Left => 0,
            PlayerSide::Right => 1,
        }
    }

    fn role_index(player: &MatchPlayer) -> usize {
        match player.tactical_position.current_position.position_group() {
            PlayerFieldPositionGroup::Goalkeeper => 0,
            PlayerFieldPositionGroup::Defender => 1,
            PlayerFieldPositionGroup::Midfielder => 2,
            PlayerFieldPositionGroup::Forward => 3,
        }
    }

    fn is_defender(player: &MatchPlayer) -> bool {
        player.tactical_position.current_position.position_group()
            == PlayerFieldPositionGroup::Defender
    }

    pub fn note_offside_line(setter: &MatchPlayer) {
        Self::bump(&OFFSIDE_LINE[Self::state_slot(setter.state)]);
    }

    fn state_slot(state: PlayerState) -> usize {
        match state {
            PlayerState::Defender(s) => s as usize,
            _ => SLOTS,
        }
    }

    pub fn slot_name(slot: usize) -> String {
        match slot {
            s if s == SLOTS + 1 => "a midfielder".to_string(),
            s if s == SLOTS + 2 => "a forward".to_string(),
            s => DefenderState::from_slot(s)
                .map_or_else(|| "not a defender state".to_string(), |s| s.to_string()),
        }
    }

    fn band(speed_mps: f32) -> usize {
        Self::BAND_EDGES
            .iter()
            .position(|edge| speed_mps <= *edge)
            .unwrap_or(Self::BAND_EDGES.len())
    }

    // ── Hooks ────────────────────────────────────────────────────────

    /// The election's reason, with a dead ball read as its own.
    pub fn chase_yield(why: TakeBallYield, tick_context: &GameTickContext) -> ChaseYield {
        if tick_context.ball.restart_taker.is_some() {
            return ChaseYield::Restart;
        }
        match why {
            TakeBallYield::RecollectBar => ChaseYield::RecollectBar,
            TakeBallYield::TeammatesPass => ChaseYield::TeammatesPass,
            TakeBallYield::Hysteresis => ChaseYield::Hysteresis,
            TakeBallYield::AerialAward => ChaseYield::AerialAward,
        }
    }

    /// The rule about to put the ball with `taker`. A turnover to him the
    /// census sees within two ticks is booked under it.
    pub fn tag_take(&mut self, rule: TakeRule, taker: u32, tick: u64) {
        self.take_tag = Some((rule, taker, tick));
    }

    /// A challenge a defender's state has just rolled.
    pub fn note_challenge(
        ctx: &StateProcessingContext,
        carrier_id: u32,
        carrier_position: Vector3<f32>,
        kind: ChallengeKind,
        outcome: ChallengeOutcome,
    ) {
        let own_goal = ctx.ball().direction_to_own_goal();
        let goal_side = (ctx.player.position - own_goal).xy().norm()
            < (carrier_position - own_goal).xy().norm();
        let note = ChallengeNote {
            defender: ctx.player.id,
            carrier: carrier_id,
            kind,
            outcome,
            tick: ctx.context.current_tick(),
        };
        let speed_mps =
            ctx.player.velocity.xy().norm() * Self::METRES_PER_UNIT / Self::SECONDS_PER_TICK;
        let kind_index = usize::from(kind != ChallengeKind::Block);
        Self::bump(&CHALLENGES[kind_index]);
        Self::bump(
            &CHALLENGE_OUTCOMES[kind_index][match outcome {
                ChallengeOutcome::Won => 0,
                ChallengeOutcome::Missed => 1,
                ChallengeOutcome::Foul => 2,
            }],
        );
        CHALLENGE_SPEED_CMPS.fetch_add((speed_mps * 100.0) as u64, Ordering::Relaxed);
        if goal_side {
            Self::bump(&CHALLENGE_GOAL_SIDE);
        }
        let mut census = ctx.context.defender_census.borrow_mut();
        if census.challenges.len() >= 64 {
            census.challenges.remove(0);
        }
        census.challenges.push(note);
        if let Some(watch) = census.watches.iter_mut().find(|w| w.id == note.defender)
            && let Some(spell) = watch.spell.as_mut()
        {
            spell.last = Some(note);
        }
    }

    /// One header played: the rule that decided it, the man, whether he
    /// left the ground, and what it was booked as. A midfielder's goes on
    /// his own census too.
    pub fn note_header(
        rule: HeaderRule,
        player: &MatchPlayer,
        leapt: bool,
        booking: HeaderBooking,
    ) {
        if player.tactical_position.current_position.is_goalkeeper() {
            return;
        }
        MidfielderCensus::note_header(rule, player, leapt, booking);
        ForwardCensus::note_header(rule, player, leapt, booking);
        let r = rule as usize;
        Self::bump(&HEADERS[r]);
        let slot = match player.state {
            PlayerState::Defender(s) => s as usize,
            _ => match player.tactical_position.current_position.position_group() {
                PlayerFieldPositionGroup::Midfielder => SLOTS + 1,
                PlayerFieldPositionGroup::Forward => SLOTS + 2,
                _ => SLOTS,
            },
        };
        Self::bump(&HEADER_STATE[r][slot]);
        if leapt || player.is_airborne() {
            Self::bump(&HEADER_LEAP[r]);
        }
        Self::bump(&HEADER_BOOKED[r][booking as usize]);
    }

    /// A flight one of the two aerial resolvers has decided. `by_duel` is
    /// the open-play duel; otherwise the cross contest.
    pub fn note_flight_decided(&mut self, flight: u64, by_duel: bool) {
        let other = if by_duel {
            self.contest_flight
        } else {
            self.duel_flight
        };
        if other == Some(flight) {
            Self::bump(&FLIGHTS_BOTH);
        }
        if by_duel {
            self.duel_flight = Some(flight);
        } else {
            self.contest_flight = Some(flight);
        }
    }

    /// A resolver has given `player_id` the high ball.
    pub fn note_award(&mut self, player_id: u32, tick: u64) {
        self.award = Some((player_id, tick));
    }

    /// The heading state's own roll, and whether a resolver had already
    /// given him the ball.
    pub fn note_heading_roll(ctx: &StateProcessingContext) {
        Self::bump(&HEADING_ROLLS);
        let tick = ctx.context.current_tick();
        let awarded = ctx
            .context
            .defender_census
            .borrow()
            .award
            .is_some_and(|(id, at)| id == ctx.player.id && tick.saturating_sub(at) <= 300)
            || ctx.tick_context.ball.aerial_contest_winner == Some(ctx.player.id);
        if awarded {
            Self::bump(&HEADING_REROLLS);
        }
    }

    /// A ball that has come off a defender without his keeping it. The
    /// census follows it until it is touched, stops or goes out.
    pub fn note_deflection(
        &mut self,
        kind: DeflectionKind,
        ball: Vector3<f32>,
        velocity: Vector3<f32>,
        man: Vector3<f32>,
        tick: u64,
    ) {
        let k = kind as usize;
        let speed = velocity.xy().norm();
        Self::bump(&DEFLECTIONS[k]);
        DEFLECTION_SPEED_X1000[k].fetch_add((speed * 1000.0) as u64, Ordering::Relaxed);
        if speed >= 1.0 - 1e-3 {
            Self::bump(&DEFLECTION_FAST[k]);
        }
        let gap = (ball - man).xy().norm();
        DEFLECTION_GAP_MM[k].fetch_add(
            (gap * Self::METRES_PER_UNIT * 1000.0) as u64,
            Ordering::Relaxed,
        );
        if gap > Self::LUNGE {
            Self::bump(&DEFLECTION_BEYOND_LUNGE[k]);
        }
        if kind == DeflectionKind::BlockKept {
            return;
        }
        if let Some(open) = self.deflection.take() {
            Self::close_deflection(open, ball, DeflectionEnd::TOUCHED);
        }
        self.deflection = Some(DeflectionTrack {
            kind,
            origin: ball,
            opened: tick,
        });
    }

    fn close_deflection(track: DeflectionTrack, at: Vector3<f32>, end: usize) {
        let k = track.kind as usize;
        let metres = (at - track.origin).xy().norm() * Self::METRES_PER_UNIT;
        DEFLECTION_TRAVEL_MM[k].fetch_add((metres * 1000.0) as u64, Ordering::Relaxed);
        Self::bump(&DEFLECTION_ENDS[k][end]);
        let bin = Self::TRAVEL_EDGES
            .iter()
            .position(|edge| metres < *edge)
            .unwrap_or(Self::TRAVEL_EDGES.len());
        Self::bump(&DEFLECTION_TRAVEL[k][bin]);
    }

    /// One defender's AI tick, after his state's handler has decided.
    /// `before` is his state before the dispatcher's overrides, `redirect`
    /// what they did.
    pub fn note_dispatch(
        ctx: &StateProcessingContext,
        before: PlayerState,
        redirect: DispatchRedirect,
        result: &StateProcessingResult,
    ) {
        let PlayerState::Defender(current) = ctx.player.state else {
            return;
        };
        let decided = result.state.unwrap_or(ctx.player.state);
        let changed = result
            .state
            .is_some_and(|s| s.compact_id() != ctx.player.state.compact_id());
        let slot = current as usize;

        for situation in Situation::ALL {
            if Self::situation_holds(ctx, situation) {
                let cell = &DOORS[situation as usize][slot];
                Self::bump(&cell[0]);
                if situation.answered_by(decided) || situation.answered_by(ctx.player.state) {
                    Self::bump(&cell[1]);
                }
            }
        }

        let out_of_possession = !ctx.team().is_control_ball();
        let (duty, rank) = if out_of_possession {
            let plan = ctx.context.defence_plan_for_team(ctx.player.team_id);
            let duty = Duty::of(plan, ctx.player.id);
            let rank = Rank::of(ctx.player().defensive().defensive_role_for_ball_carrier());
            Self::bump(&ASSIGNMENT[duty][rank]);
            Self::bump(&ASSIGNMENT_STATE[duty][slot]);
            (Some(duty), Some(rank))
        } else {
            (None, None)
        };

        let mut census = ctx.context.defender_census.borrow_mut();
        let watch = census.watch(ctx.player.id);
        if out_of_possession && changed {
            Self::bump(&CHANGES_OUT_OF_POSSESSION);
            let after_rank = watch.rank.is_some() && watch.rank != rank;
            let after_duty = watch.duty.is_some() && watch.duty != duty;
            match (after_rank, after_duty) {
                (true, true) => Self::bump(&CHANGES_AFTER_BOTH),
                (true, false) => Self::bump(&CHANGES_AFTER_RANK),
                (false, true) => Self::bump(&CHANGES_AFTER_DUTY),
                (false, false) => {}
            }
        }
        watch.duty = duty;
        watch.rank = rank;

        Self::follow_chase(watch, before, current, redirect, result);
    }

    /// A chase a state's own door started: confirmed when he is in
    /// `TakeBall` at his next AI tick, reversed when the dispatcher yields
    /// him within ten.
    fn follow_chase(
        watch: &mut DefenderWatch,
        before: PlayerState,
        current: DefenderState,
        redirect: DispatchRedirect,
        result: &StateProcessingResult,
    ) {
        let in_chase = before == PlayerState::Defender(DefenderState::TakeBall);
        if let Some(mut chase) = watch.chase.take()
            && in_chase
        {
            if !chase.confirmed {
                chase.confirmed = true;
                Self::bump(&CHASE_ENTRIES[chase.door]);
            }
            if let DispatchRedirect::Yield(why) = redirect {
                Self::bump(&CHASE_REVERSED[chase.door][why as usize]);
            } else if chase.ai_ticks + 1 < Self::CHASE_AI_TICKS {
                chase.ai_ticks += 1;
                watch.chase = Some(chase);
            }
        }
        let door_entry = current != DefenderState::TakeBall
            && redirect == DispatchRedirect::None
            && result.state == Some(PlayerState::Defender(DefenderState::TakeBall));
        if door_entry {
            watch.chase = Some(ChaseEntry {
                door: current as usize,
                ai_ticks: 0,
                confirmed: false,
            });
        }
    }

    fn situation_holds(ctx: &StateProcessingContext, situation: Situation) -> bool {
        match situation {
            Situation::LicensedCarrier => {
                ctx.players()
                    .opponents()
                    .with_ball()
                    .next()
                    .is_some_and(|carrier| {
                        carrier.distance(ctx) <= TackleEngagement::COMMIT
                            && TackleEngagement::may_engage_carrier(ctx)
                    })
            }
            Situation::LooseBall => {
                !ctx.tick_context.ball.is_owned
                    && ctx.player.side.is_some_and(|side| {
                        ctx.tick_context.chase.cost_of(ctx.player.id).is_some()
                            && ctx.tick_context.chase.is_designated(side, ctx.player.id)
                    })
            }
            Situation::HighBall => {
                ctx.tick_context.ball.aerial_contest_winner == Some(ctx.player.id)
            }
            Situation::OwnPossession => {
                ctx.team().is_control_ball() && ctx.ball().owner_id() != Some(ctx.player.id)
            }
            Situation::CarrierInArea => Self::carrier_in_area_for(ctx),
        }
    }

    fn carrier_in_area_for(ctx: &StateProcessingContext) -> bool {
        let Some(carrier) = ctx.players().opponents().with_ball().next() else {
            return false;
        };
        let Some(side) = ctx.player.side else {
            return false;
        };
        if !ctx
            .context
            .penalty_area(side == PlayerSide::Left)
            .contains(&carrier.position)
        {
            return false;
        }
        let mine = (ctx.player.position - carrier.position).norm();
        let nearer = ctx
            .players()
            .teammates()
            .all()
            .filter(|t| t.id != ctx.player.id && !t.tactical_positions.is_goalkeeper())
            .filter(|t| (t.position - carrier.position).norm() < mine)
            .count();
        nearer < 2
    }

    // ── Every tick ───────────────────────────────────────────────────

    fn watch(&mut self, id: u32) -> &mut DefenderWatch {
        match self.watches.iter().position(|w| w.id == id) {
            Some(at) => &mut self.watches[at],
            None => {
                self.watches.push(DefenderWatch::new(id));
                self.watches.last_mut().expect("just pushed")
            }
        }
    }

    /// One sample a tick, after everybody has moved and every event has
    /// landed. `plans` are each team's defensive plan, `areas` the left and
    /// the right penalty area.
    pub fn sample(
        &mut self,
        field: &MatchField,
        plans: [(u32, DefensivePlan); 2],
        areas: [PenaltyArea; 2],
        tick: u64,
    ) {
        self.follow_turnovers(field, tick);
        self.follow_plans(field, &plans, &areas);
        self.follow_deflection(field, tick);

        let dead = field.ball.awaiting_restart.is_some();
        let owner = field.ball.current_owner;
        let (mut live_ticks, mut slow_live_ticks) = (0, 0);
        for player in field
            .players
            .iter()
            .filter(|p| !p.off_pitch && Self::is_defender(p))
        {
            let line = BackLine::of(player.tactical_position.current_position);
            let has_ball = owner == Some(player.id);
            let carrier = owner
                .and_then(|id| field.players.iter().find(|p| p.id == id))
                .filter(|c| c.team_id != player.team_id);
            let watch = self.watch(player.id);
            let resumed = watch.push(player.position, tick);
            let step = if resumed {
                0.0
            } else {
                (player.position - watch.last_pos).xy().norm()
            };
            watch.last_pos = player.position;
            let speed = watch.speed();
            let state = player.state;
            let slot = Self::state_slot(state);
            Self::bump(&STATE_TICKS[slot]);
            if has_ball {
                Self::bump(&STATE_BALL_TICKS[slot]);
            }
            STATE_SPEED_CMPS[slot].fetch_add((speed * 100.0) as u64, Ordering::Relaxed);

            if !dead {
                live_ticks += 1;
                if speed <= Self::BAND_EDGES[1] {
                    slow_live_ticks += 1;
                }
            }
            if let Some(line) = line {
                let live = usize::from(!dead);
                Self::bump(&MOTION_TICKS[line][live]);
                MOTION_PATH_MM[line][live].fetch_add(
                    (step * Self::METRES_PER_UNIT * 1000.0) as u64,
                    Ordering::Relaxed,
                );
                Self::bump(&MOTION_BANDS[line][live][Self::band(speed)]);
                if speed > Self::BAND_EDGES[3] {
                    watch.sprint_ticks += 1;
                    if watch.sprint_ticks == Self::SPRINT_TICKS {
                        Self::bump(&SPRINTS[line][live]);
                    }
                } else if speed < Self::BAND_EDGES[2] {
                    watch.sprint_ticks = 0;
                }
            }

            if watch
                .seen_state
                .is_some_and(|s| s.compact_id() != state.compact_id())
                && state == PlayerState::Defender(DefenderState::Tackling)
            {
                Self::bump(&TACKLING_ENTRIES);
            }
            watch.seen_state = Some(state);

            Self::follow_phase(watch, Phase::of(state, has_ball), tick);
            Self::follow_spell(watch, player, carrier, owner);
        }
        self.live_ticks += live_ticks;
        self.slow_live_ticks += slow_live_ticks;
        self.snapshot_stats(field);
    }

    fn follow_phase(watch: &mut DefenderWatch, phase: usize, tick: u64) {
        Self::bump(&PHASE_TICKS[phase]);
        match watch.phase {
            None => {
                watch.phase = Some(PhaseTrack {
                    phase,
                    since: tick,
                    previous: None,
                })
            }
            Some(track) if track.phase != phase => {
                let relaxed_alert = matches!(
                    (track.phase, phase),
                    (Phase::RELAXED, Phase::ALERT) | (Phase::ALERT, Phase::RELAXED)
                );
                if relaxed_alert {
                    Self::bump(&RELAXED_ALERT_CHANGES);
                    if track.previous == Some(phase)
                        && tick.saturating_sub(track.since) <= Self::REVERSAL_TICKS
                    {
                        Self::bump(&RELAXED_ALERT_REVERSED);
                    }
                }
                watch.phase = Some(PhaseTrack {
                    phase,
                    since: tick,
                    previous: Some(track.phase),
                });
            }
            Some(_) => {}
        }
    }

    /// A spell runs while he is closing or containing a carrier, and ends
    /// on a challenge's outcome or on what took the carrier from him.
    fn follow_spell(
        watch: &mut DefenderWatch,
        player: &MatchPlayer,
        carrier: Option<&MatchPlayer>,
        owner: Option<u32>,
    ) {
        let engaging = match player.state {
            PlayerState::Defender(s) => s == DefenderState::Pressing || Self::is_duel_state(s),
            _ => false,
        };
        if let Some(spell) = watch.spell.as_mut() {
            let same_carrier = carrier.is_some_and(|c| c.id == spell.carrier);
            if engaging && same_carrier {
                if player.state == PlayerState::Defender(DefenderState::Pressing) {
                    spell.pressing += 1;
                } else {
                    spell.tackling += 1;
                }
                return;
            }
            let spell = watch.spell.take().expect("checked above");
            let end = match spell.last.map(|c| c.outcome) {
                Some(ChallengeOutcome::Won) => SpellEnd::Won,
                Some(ChallengeOutcome::Missed) => SpellEnd::Missed,
                Some(ChallengeOutcome::Foul) => SpellEnd::Foul,
                None if owner != Some(spell.carrier) => SpellEnd::Released,
                None => match carrier {
                    Some(c)
                        if (c.position - player.position).norm() > TackleEngagement::DISENGAGE =>
                    {
                        SpellEnd::Away
                    }
                    _ => SpellEnd::Passed,
                },
            };
            Self::bump(&SPELLS);
            SPELL_PRESSING_TICKS.fetch_add(spell.pressing, Ordering::Relaxed);
            SPELL_TACKLING_TICKS.fetch_add(spell.tackling, Ordering::Relaxed);
            Self::bump(&SPELL_ENDS[end as usize]);
        }
        if engaging && let Some(c) = carrier {
            let pressing = player.state == PlayerState::Defender(DefenderState::Pressing);
            watch.spell = Some(DuelSpell {
                carrier: c.id,
                pressing: u64::from(pressing),
                tackling: u64::from(!pressing),
                last: None,
            });
        }
    }

    /// Who has the ball now, against who had it: a new owner on the other
    /// side is a turnover, booked under the rule that tagged it.
    fn follow_turnovers(&mut self, field: &MatchField, tick: u64) {
        let owner = field.ball.current_owner;
        let owned_last_tick = self.owner_last_tick;
        self.owner_last_tick = owner;

        let due: Vec<PendingTurnover> = self
            .pending
            .iter()
            .copied()
            .filter(|p| p.due <= tick)
            .collect();
        self.pending.retain(|p| p.due > tick);
        for p in due {
            let after = field
                .players
                .iter()
                .find(|q| q.id == p.taker)
                .map_or(p.before, Self::stat_line);
            let credit = (0..4)
                .find(|i| after[*i] > p.before[*i])
                .unwrap_or(Credit::COUNT - 1);
            Self::bump(&TAKE_CREDIT[p.rule][credit]);
        }

        let Some(owner_id) = owner else {
            return;
        };
        let Some(taker) = field.players.iter().find(|p| p.id == owner_id) else {
            return;
        };
        let previous = self.holder;
        if previous.is_none_or(|h| h.id != owner_id) {
            self.holder = Some(Holder {
                id: owner_id,
                team: taker.team_id,
                since: tick,
            });
        }
        let Some(loser) = previous.filter(|h| h.team != taker.team_id) else {
            return;
        };

        let tagged = self
            .take_tag
            .filter(|(_, taker, at)| *taker == owner_id && tick.saturating_sub(*at) <= 2)
            .map(|(rule, _, _)| rule);
        let rule = match tagged {
            Some(TakeRule::Tackle) => self
                .challenges
                .iter()
                .rev()
                .find(|c| c.defender == owner_id && tick.saturating_sub(c.tick) <= 4)
                .map_or(TakeRule::Tackle, |c| {
                    if c.kind == ChallengeKind::Block {
                        TakeRule::BlockTackle
                    } else {
                        TakeRule::RecoveryChallenge
                    }
                }),
            Some(rule) => rule,
            None => TakeRule::Loose,
        };
        let r = rule as usize;
        let carried = usize::from(owned_last_tick == Some(loser.id));
        Self::bump(&TURNOVERS[r][carried]);
        Self::bump(&TAKE_ROLE[r][Self::role_index(taker)]);
        if let PlayerState::Defender(s) = taker.state {
            Self::bump(&TAKE_DEF_STATE[r][s as usize]);
        }
        if let Some(loser_player) = field.players.iter().find(|p| p.id == loser.id) {
            TAKE_GAP_MM[r].fetch_add(
                ((taker.position - loser_player.position).xy().norm()
                    * Self::METRES_PER_UNIT
                    * 1000.0) as u64,
                Ordering::Relaxed,
            );
        }
        TAKE_TENURE_TICKS[r].fetch_add(tick.saturating_sub(loser.since), Ordering::Relaxed);
        let prior = self
            .challenges
            .iter()
            .rev()
            .find(|c| c.carrier == loser.id && tick.saturating_sub(c.tick) <= Self::PRIOR_TICKS)
            .map_or(0, |c| match c.outcome {
                ChallengeOutcome::Won => 1,
                ChallengeOutcome::Missed => 2,
                ChallengeOutcome::Foul => 3,
            });
        Self::bump(&TAKE_PRIOR[r][prior]);
        let before = self
            .stats
            .iter()
            .find(|(id, _)| *id == owner_id)
            .map_or([0; 4], |(_, line)| *line);
        self.pending.push(PendingTurnover {
            taker: owner_id,
            rule: r,
            due: tick + Self::CREDIT_TICKS,
            before,
        });
    }

    fn stat_line(player: &MatchPlayer) -> [u16; 4] {
        let s = &player.statistics;
        [s.tackles, s.interceptions, s.blocks, s.clearances]
    }

    fn snapshot_stats(&mut self, field: &MatchField) {
        self.stats.clear();
        self.stats
            .extend(field.players.iter().map(|p| (p.id, Self::stat_line(p))));
    }

    fn follow_plans(
        &mut self,
        field: &MatchField,
        plans: &[(u32, DefensivePlan); 2],
        areas: &[PenaltyArea; 2],
    ) {
        for (i, (_, plan)) in plans.iter().enumerate() {
            if !plan.active {
                self.plan_hands[i] = (None, None);
                continue;
            }
            Self::bump(&PLAN_TICKS);
            let now = (plan.presser(), plan.cover());
            let (press, cover) = self.plan_hands[i];
            if press.is_some() && now.0.is_some() && press != now.0 {
                Self::bump(&PRESS_HANDS);
            }
            if cover.is_some() && now.1.is_some() && cover != now.1 {
                Self::bump(&COVER_HANDS);
                if now.0 == cover {
                    Self::bump(&COVER_TO_PRESS);
                }
            }
            self.plan_hands[i] = now;
        }

        let Some(carrier) = field
            .ball
            .current_owner
            .and_then(|id| field.players.iter().find(|p| p.id == id))
        else {
            return;
        };
        let Some(defending_side) = field
            .players
            .iter()
            .find(|p| p.team_id != carrier.team_id)
            .and_then(|p| p.side)
        else {
            return;
        };
        let in_area =
            usize::from(areas[Self::side_index(defending_side)].contains(&carrier.position));
        let engaged = field
            .players
            .iter()
            .filter(|p| p.team_id != carrier.team_id && !p.off_pitch)
            .filter(|p| match p.state {
                PlayerState::Defender(s) => s == DefenderState::Pressing || Self::is_duel_state(s),
                _ => false,
            })
            .count();
        Self::bump(&CARRIER_TICKS[in_area]);
        if engaged >= 2 {
            Self::bump(&CROWDED_TICKS[in_area]);
        }
    }

    fn follow_deflection(&mut self, field: &MatchField, tick: u64) {
        let Some(track) = self.deflection else {
            return;
        };
        let ball = &field.ball;
        let end = if ball.last_touch_tick > track.opened || ball.current_owner.is_some() {
            Some(DeflectionEnd::TOUCHED)
        } else if ball.awaiting_restart.is_some() {
            Some(DeflectionEnd::OUT)
        } else if ball.velocity.xy().norm() < Self::AT_REST && ball.position.z <= 0.05 {
            Some(DeflectionEnd::STOPPED)
        } else if tick.saturating_sub(track.opened) > Self::DEFLECTION_TICKS {
            Some(DeflectionEnd::TIMED_OUT)
        } else {
            None
        };
        if let Some(end) = end {
            self.deflection = None;
            Self::close_deflection(track, ball.position, end);
        }
    }

    /// Full time: the defenders' live ticks, and those at or below a walk,
    /// go on the match tally, which is what the realism bands read.
    pub fn finish(&mut self, tally: &mut MatchTally) {
        tally.defender_motion.live_ticks += self.live_ticks;
        tally.defender_motion.slow_live_ticks += self.slow_live_ticks;
        if let Some(track) = self.deflection.take() {
            Self::bump(&DEFLECTION_ENDS[track.kind as usize][DeflectionEnd::TIMED_OUT]);
        }
    }

    // ── Totals ───────────────────────────────────────────────────────

    fn read<const N: usize>(counters: &[AtomicU64; N]) -> [u64; N] {
        std::array::from_fn(|i| counters[i].load(Ordering::Relaxed))
    }

    fn read2<const N: usize, const M: usize>(counters: &[[AtomicU64; M]; N]) -> [[u64; M]; N] {
        std::array::from_fn(|i| Self::read(&counters[i]))
    }

    pub fn snapshot() -> DefenderCensusSnapshot {
        DefenderCensusSnapshot {
            turnovers: Self::read2(&TURNOVERS),
            take_role: Self::read2(&TAKE_ROLE),
            take_def_state: Self::read2(&TAKE_DEF_STATE),
            take_gap_mm: Self::read(&TAKE_GAP_MM),
            take_tenure_ticks: Self::read(&TAKE_TENURE_TICKS),
            take_prior: Self::read2(&TAKE_PRIOR),
            take_credit: Self::read2(&TAKE_CREDIT),
            spells: SPELLS.load(Ordering::Relaxed),
            spell_pressing_ticks: SPELL_PRESSING_TICKS.load(Ordering::Relaxed),
            spell_tackling_ticks: SPELL_TACKLING_TICKS.load(Ordering::Relaxed),
            spell_ends: Self::read(&SPELL_ENDS),
            tackling_entries: TACKLING_ENTRIES.load(Ordering::Relaxed),
            challenges: Self::read(&CHALLENGES),
            challenge_outcomes: Self::read2(&CHALLENGE_OUTCOMES),
            challenge_speed_cmps: CHALLENGE_SPEED_CMPS.load(Ordering::Relaxed),
            challenge_goal_side: CHALLENGE_GOAL_SIDE.load(Ordering::Relaxed),
            headers: Self::read(&HEADERS),
            header_state: Self::read2(&HEADER_STATE),
            header_leap: Self::read(&HEADER_LEAP),
            header_booked: Self::read2(&HEADER_BOOKED),
            flights_both: FLIGHTS_BOTH.load(Ordering::Relaxed),
            heading_rolls: HEADING_ROLLS.load(Ordering::Relaxed),
            heading_rerolls: HEADING_REROLLS.load(Ordering::Relaxed),
            assignment: Self::read2(&ASSIGNMENT),
            assignment_state: Self::read2(&ASSIGNMENT_STATE),
            changes_out_of_possession: CHANGES_OUT_OF_POSSESSION.load(Ordering::Relaxed),
            changes_after_rank: CHANGES_AFTER_RANK.load(Ordering::Relaxed),
            changes_after_duty: CHANGES_AFTER_DUTY.load(Ordering::Relaxed),
            changes_after_both: CHANGES_AFTER_BOTH.load(Ordering::Relaxed),
            press_hands: PRESS_HANDS.load(Ordering::Relaxed),
            cover_hands: COVER_HANDS.load(Ordering::Relaxed),
            cover_to_press: COVER_TO_PRESS.load(Ordering::Relaxed),
            plan_ticks: PLAN_TICKS.load(Ordering::Relaxed),
            carrier_ticks: Self::read(&CARRIER_TICKS),
            crowded_ticks: Self::read(&CROWDED_TICKS),
            doors: std::array::from_fn(|s| Self::read2(&DOORS[s])),
            motion_ticks: Self::read2(&MOTION_TICKS),
            motion_path_mm: Self::read2(&MOTION_PATH_MM),
            motion_bands: std::array::from_fn(|l| Self::read2(&MOTION_BANDS[l])),
            sprints: Self::read2(&SPRINTS),
            state_ticks: Self::read(&STATE_TICKS),
            state_ball_ticks: Self::read(&STATE_BALL_TICKS),
            offside_line: Self::read(&OFFSIDE_LINE),
            state_speed_cmps: Self::read(&STATE_SPEED_CMPS),
            phase_ticks: Self::read(&PHASE_TICKS),
            relaxed_alert_changes: RELAXED_ALERT_CHANGES.load(Ordering::Relaxed),
            relaxed_alert_reversed: RELAXED_ALERT_REVERSED.load(Ordering::Relaxed),
            chase_entries: Self::read(&CHASE_ENTRIES),
            chase_reversed: Self::read2(&CHASE_REVERSED),
            deflections: Self::read(&DEFLECTIONS),
            deflection_speed_x1000: Self::read(&DEFLECTION_SPEED_X1000),
            deflection_fast: Self::read(&DEFLECTION_FAST),
            deflection_gap_mm: Self::read(&DEFLECTION_GAP_MM),
            deflection_beyond_lunge: Self::read(&DEFLECTION_BEYOND_LUNGE),
            deflection_travel_mm: Self::read(&DEFLECTION_TRAVEL_MM),
            deflection_ends: Self::read2(&DEFLECTION_ENDS),
            deflection_travel: Self::read2(&DEFLECTION_TRAVEL),
        }
    }

    pub fn reset() {
        let scalars = [
            &SPELLS,
            &SPELL_PRESSING_TICKS,
            &SPELL_TACKLING_TICKS,
            &TACKLING_ENTRIES,
            &CHALLENGE_SPEED_CMPS,
            &CHALLENGE_GOAL_SIDE,
            &FLIGHTS_BOTH,
            &HEADING_ROLLS,
            &HEADING_REROLLS,
            &CHANGES_OUT_OF_POSSESSION,
            &CHANGES_AFTER_RANK,
            &CHANGES_AFTER_DUTY,
            &CHANGES_AFTER_BOTH,
            &PRESS_HANDS,
            &COVER_HANDS,
            &COVER_TO_PRESS,
            &PLAN_TICKS,
            &RELAXED_ALERT_CHANGES,
            &RELAXED_ALERT_REVERSED,
        ];
        for counter in scalars
            .into_iter()
            .chain(TURNOVERS.iter().flatten())
            .chain(TAKE_ROLE.iter().flatten())
            .chain(TAKE_DEF_STATE.iter().flatten())
            .chain(&TAKE_GAP_MM)
            .chain(&TAKE_TENURE_TICKS)
            .chain(TAKE_PRIOR.iter().flatten())
            .chain(TAKE_CREDIT.iter().flatten())
            .chain(&SPELL_ENDS)
            .chain(&CHALLENGES)
            .chain(CHALLENGE_OUTCOMES.iter().flatten())
            .chain(&HEADERS)
            .chain(HEADER_STATE.iter().flatten())
            .chain(&HEADER_LEAP)
            .chain(HEADER_BOOKED.iter().flatten())
            .chain(ASSIGNMENT.iter().flatten())
            .chain(ASSIGNMENT_STATE.iter().flatten())
            .chain(&CARRIER_TICKS)
            .chain(&CROWDED_TICKS)
            .chain(DOORS.iter().flatten().flatten())
            .chain(MOTION_TICKS.iter().flatten())
            .chain(MOTION_PATH_MM.iter().flatten())
            .chain(MOTION_BANDS.iter().flatten().flatten())
            .chain(SPRINTS.iter().flatten())
            .chain(&STATE_TICKS)
            .chain(&STATE_BALL_TICKS)
            .chain(&OFFSIDE_LINE)
            .chain(&STATE_SPEED_CMPS)
            .chain(&PHASE_TICKS)
            .chain(&CHASE_ENTRIES)
            .chain(CHASE_REVERSED.iter().flatten())
            .chain(&DEFLECTIONS)
            .chain(&DEFLECTION_SPEED_X1000)
            .chain(&DEFLECTION_FAST)
            .chain(&DEFLECTION_GAP_MM)
            .chain(&DEFLECTION_BEYOND_LUNGE)
            .chain(&DEFLECTION_TRAVEL_MM)
            .chain(DEFLECTION_ENDS.iter().flatten())
            .chain(DEFLECTION_TRAVEL.iter().flatten())
        {
            counter.store(0, Ordering::Relaxed);
        }
    }
}
