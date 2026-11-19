//! **The midfielder action census** — how far and how fast a midfielder
//! moves, the job each assignment gives him against the state he is in, how
//! his duels run and how he comes away with the ball, how each header and
//! each dropping ball is decided, which situations each state acts on, his
//! phases, the loose-ball chases his own doors start, and how long he keeps a
//! ball he receives.
//!
//! The open episodes are per match and live on
//! [`MatchContext`](crate::r#match::MatchContext); the totals are
//! process-wide counters the `dev_match` harness prints. Nothing here
//! changes what the match does.

use crate::PlayerFieldPositionGroup;
use crate::PlayerPositionType;
use crate::r#match::common_states::{ChallengeKind, TackleEngagement};
use crate::r#match::engine::ball::ball::{AerialReach, PlayerReach};
use crate::r#match::engine::teamplay::plans::attack::AttackPlan;
use crate::r#match::engine::teamplay::possession::TeamPossession;
use crate::r#match::midfielders::states::{MidfielderFamily, MidfielderState};
use crate::r#match::player::diagnostics::defender_census::{
    ChallengeOutcome, ChaseYield, DefenderCensus, DispatchRedirect, Duty, HeaderBooking,
    HeaderRule, SpellEnd,
};
use crate::r#match::player::state::PlayerState;
use crate::r#match::{
    MatchField, MatchPlayer, MatchTally, PenaltyArea, PlayerSide, StateProcessingContext,
    StateProcessingResult,
};
use nalgebra::Vector3;
use std::sync::atomic::{AtomicU64, Ordering};

/// A midfielder's place in the midfield, by his position in the formation.
pub struct MidfieldPlace;

impl MidfieldPlace {
    pub const COUNT: usize = 2;
    pub const LABELS: [&'static str; Self::COUNT] = ["central midfielder", "wide midfielder"];
    pub const CENTRAL: usize = 0;

    pub fn of(position: PlayerPositionType) -> Option<usize> {
        match position {
            PlayerPositionType::MidfielderCenterLeft
            | PlayerPositionType::MidfielderCenter
            | PlayerPositionType::MidfielderCenterRight
            | PlayerPositionType::AttackingMidfielderCenter => Some(Self::CENTRAL),
            PlayerPositionType::MidfielderLeft
            | PlayerPositionType::MidfielderRight
            | PlayerPositionType::AttackingMidfielderLeft
            | PlayerPositionType::AttackingMidfielderRight
            | PlayerPositionType::WingbackLeft
            | PlayerPositionType::WingbackRight => Some(1),
            _ => None,
        }
    }
}

/// Which side holds the ball (`TeamPossession`, so a pass in flight is the
/// passer's side's), read from the midfielder's side.
pub struct BallHeld;

impl BallHeld {
    pub const COUNT: usize = 3;
    pub const LABELS: [&'static str; Self::COUNT] = ["his side's ball", "their ball", "nobody's"];

    pub(crate) fn of(holder: Option<u32>, team: u32) -> usize {
        match holder {
            Some(t) if t == team => 0,
            Some(_) => 1,
            None => 2,
        }
    }
}

/// The attack plan's job for a midfielder off the ball, highest first.
pub struct AttackJob;

impl AttackJob {
    pub const COUNT: usize = 6;
    pub const LABELS: [&'static str; Self::COUNT] = [
        "touchline",
        "box place",
        "runner in behind",
        "short outlet",
        "kept behind the ball",
        "none",
    ];
    pub const KEPT_BEHIND: usize = 4;

    fn of(plan: &AttackPlan, id: u32) -> usize {
        if plan.wide.flank_of(id).is_some() || plan.wide.is_overlap_runner(id) {
            0
        } else if plan.slot_of(id).is_some() {
            1
        } else if plan.far_runner == Some(id) {
            2
        } else if plan.near_support == Some(id) {
            3
        } else if plan.is_rest_defence(id) {
            Self::KEPT_BEHIND
        } else {
            5
        }
    }
}

/// How a midfielder came away with the ball from `Tackling`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TackleTake {
    /// The challenge won it.
    Duel,
    /// The state's own claim on a loose ball beside the duel.
    ClaimBranch,
    /// It was already his at the start of the tick.
    AlreadyHis,
}

impl TackleTake {
    pub const COUNT: usize = 3;
    pub const LABELS: [&'static str; Self::COUNT] = [
        "the challenge",
        "Tackling's own ClaimBall",
        "already his at the start of the tick",
    ];
}

/// The rule that decided a midfielder's header.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MidHeaderRule {
    CrossContest,
    AerialDuel,
    /// `Heading`'s own decision: its roll, or a ball a contest awarded him.
    HeadingState,
    OwnershipClaim,
}

impl MidHeaderRule {
    pub const COUNT: usize = 4;
    pub const LABELS: [&'static str; Self::COUNT] = [
        "cross contest",
        "aerial duel",
        "Heading state",
        "ownership claim",
    ];

    fn of(rule: HeaderRule) -> MidHeaderRule {
        match rule {
            HeaderRule::CrossContest => MidHeaderRule::CrossContest,
            HeaderRule::AerialDuel => MidHeaderRule::AerialDuel,
            HeaderRule::HeadingShot => MidHeaderRule::HeadingState,
            HeaderRule::OwnershipClaim => MidHeaderRule::OwnershipClaim,
        }
    }
}

/// Where `Heading` let him go.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HeadingExit {
    Owned,
    AwardGone,
    Shot,
    Clearance,
    KnockDown,
    Dispatcher,
}

impl HeadingExit {
    pub const COUNT: usize = 6;
    pub const LABELS: [&'static str; Self::COUNT] = [
        "somebody owns it",
        "the award spent or gone",
        "header at goal",
        "headed clearance",
        "knock-down",
        "the dispatcher",
    ];
}

/// What became of a ball dropping with nobody named for it.
pub struct DropEnd;

impl DropEnd {
    pub const COUNT: usize = 6;
    pub const LABELS: [&'static str; Self::COUNT] = [
        "headed by the side that played it",
        "headed by the other side",
        "landed, the side that played it first",
        "landed, the other side first",
        "went out",
        "timed out",
    ];
    const HEADED_SAME: usize = 0;
    const HEADED_OTHER: usize = 1;
    const LANDED_SAME: usize = 2;
    const LANDED_OTHER: usize = 3;
    const OUT: usize = 4;
    const TIMED_OUT: usize = 5;
}

/// Who could reach a dropping ball in the air.
pub struct DropReach;

impl DropReach {
    pub const COUNT: usize = 3;
    pub const LABELS: [&'static str; Self::COUNT] =
        ["men of both sides", "one side only", "nobody"];

    fn of(reach: [u8; 2]) -> usize {
        match (reach[0] > 0, reach[1] > 0) {
            (true, true) => 0,
            (false, false) => 2,
            _ => 1,
        }
    }
}

/// The situations the door census puts to every midfielder state.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MidSituation {
    /// A carrier within `TackleEngagement::COMMIT` he is licensed to engage.
    LicensedCarrier,
    /// A carrier within 12.5 m he is not licensed to engage.
    UnlicensedCarrier,
    /// A loose ball the chase table gives him.
    LooseBall,
    /// A high ball a contest has awarded him.
    HighBall,
    /// The first ten AI ticks after his side lost the ball.
    BallLost,
    /// The first ten AI ticks after his side won it.
    BallWon,
}

impl MidSituation {
    pub const COUNT: usize = 6;
    pub const LABELS: [&'static str; Self::COUNT] = [
        "licensed carrier in reach",
        "unlicensed carrier near",
        "loose ball he is given",
        "high ball he has won",
        "his side lost it",
        "his side won it",
    ];
    const ALL: [MidSituation; Self::COUNT] = [
        MidSituation::LicensedCarrier,
        MidSituation::UnlicensedCarrier,
        MidSituation::LooseBall,
        MidSituation::HighBall,
        MidSituation::BallLost,
        MidSituation::BallWon,
    ];
    /// 12.5 m.
    const NEAR: f32 = 100.0;
    const AFTER_FLIP_AI_TICKS: u32 = 10;

    /// Whether the state he is in, or the one his handler chose, acts on
    /// the situation. For an unlicensed carrier, acting is the press he
    /// should not make.
    fn answered_by(self, state: PlayerState) -> bool {
        let PlayerState::Midfielder(s) = state else {
            return false;
        };
        let family = s.family();
        match self {
            MidSituation::LicensedCarrier | MidSituation::UnlicensedCarrier => {
                matches!(family, MidfielderFamily::Close | MidfielderFamily::Duel)
            }
            MidSituation::LooseBall => {
                matches!(
                    family,
                    MidfielderFamily::Chase | MidfielderFamily::Intercept
                )
            }
            MidSituation::HighBall => family == MidfielderFamily::Aerial,
            MidSituation::BallLost => family != MidfielderFamily::Support,
            MidSituation::BallWon => !matches!(
                family,
                MidfielderFamily::Mark
                    | MidfielderFamily::Close
                    | MidfielderFamily::Duel
                    | MidfielderFamily::Recover
            ),
        }
    }
}

/// A midfielder's phase on one tick.
pub struct MidPhase;

impl MidPhase {
    pub const COUNT: usize = 9;
    pub const LABELS: [&'static str; Self::COUNT] = [
        "relaxed",
        "alert",
        "on a job",
        "in a duel",
        "recovering",
        "supporting the attack",
        "on the ball",
        "Running without the ball",
        "other",
    ];
    const RELAXED: usize = 0;
    const ALERT: usize = 1;

    fn of(state: PlayerState, has_ball: bool) -> usize {
        if has_ball {
            return 6;
        }
        match state {
            PlayerState::Midfielder(MidfielderState::Walking) => Self::RELAXED,
            PlayerState::Midfielder(MidfielderState::Running) => 7,
            PlayerState::Midfielder(s) => match s.family() {
                MidfielderFamily::Shape => Self::ALERT,
                MidfielderFamily::Mark | MidfielderFamily::Close => 2,
                MidfielderFamily::Duel => 3,
                MidfielderFamily::Recover => 4,
                MidfielderFamily::Support => 5,
                MidfielderFamily::Start
                | MidfielderFamily::Chase
                | MidfielderFamily::Intercept
                | MidfielderFamily::Aerial
                | MidfielderFamily::Carry
                | MidfielderFamily::Play => 8,
            },
            PlayerState::Recovering => 4,
            _ => 8,
        }
    }
}

#[derive(Clone, Copy, Debug)]
struct DuelSpell {
    carrier: u32,
    place: usize,
    closing: u64,
    containing: u64,
    last: Option<ChallengeOutcome>,
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
struct ReceptionTrack {
    start: u64,
    pressed: bool,
    forced: bool,
}

#[derive(Clone, Copy, Debug)]
struct MidfielderWatch {
    id: u32,
    trail: [Vector3<f32>; MidfielderCensus::TRAIL],
    trail_len: usize,
    trail_at: usize,
    last_tick: u64,
    last_pos: Vector3<f32>,
    seen_state: Option<PlayerState>,
    seen_in_state_time: u64,
    phase: Option<PhaseTrack>,
    sprint_ticks: u32,
    spell: Option<DuelSpell>,
    duty: Option<usize>,
    elected: Option<bool>,
    chase: Option<ChaseEntry>,
    control: Option<bool>,
    since_flip: u32,
    reception: Option<ReceptionTrack>,
}

impl MidfielderWatch {
    fn new(id: u32) -> Self {
        MidfielderWatch {
            id,
            trail: [Vector3::zeros(); MidfielderCensus::TRAIL],
            trail_len: 0,
            trail_at: 0,
            last_tick: 0,
            last_pos: Vector3::zeros(),
            seen_state: None,
            seen_in_state_time: 0,
            phase: None,
            sprint_ticks: 0,
            spell: None,
            duty: None,
            elected: None,
            chase: None,
            control: None,
            since_flip: 0,
            reception: None,
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
        self.trail_at = (self.trail_at + 1) % MidfielderCensus::TRAIL;
        self.trail_len = (self.trail_len + 1).min(MidfielderCensus::TRAIL);
        resumed
    }

    /// Ground speed over the trail, in metres a second.
    fn speed(&self) -> f32 {
        if self.trail_len < 2 {
            return 0.0;
        }
        let newest = (self.trail_at + MidfielderCensus::TRAIL - 1) % MidfielderCensus::TRAIL;
        let oldest = if self.trail_len < MidfielderCensus::TRAIL {
            0
        } else {
            self.trail_at
        };
        let (a, b) = (self.trail[newest], self.trail[oldest]);
        (a.x - b.x).hypot(a.y - b.y) * MidfielderCensus::METRES_PER_UNIT
            / ((self.trail_len - 1) as f32 * MidfielderCensus::SECONDS_PER_TICK)
    }
}

#[derive(Clone, Copy, Debug)]
struct DropTrack {
    opened: u64,
    team: Option<u32>,
    reach: [u8; 2],
    landed: bool,
    decided: bool,
}

/// Everything the census has counted, for the harness.
#[derive(Clone, Debug)]
pub struct MidfielderCensusSnapshot {
    pub motion_ticks: [[[u64; 2]; BallHeld::COUNT]; MidfieldPlace::COUNT],
    pub motion_path_mm: [[[u64; 2]; BallHeld::COUNT]; MidfieldPlace::COUNT],
    pub motion_bands: [[[[u64; DefenderCensus::BANDS]; 2]; BallHeld::COUNT]; MidfieldPlace::COUNT],
    pub sprints: [[[u64; 2]; BallHeld::COUNT]; MidfieldPlace::COUNT],
    pub state_ticks: [u64; SLOTS + 1],
    pub state_ball_ticks: [u64; SLOTS + 1],
    /// Ticks in each state by whose ball it is ([`BallHeld`]), the last
    /// column a dead ball.
    pub state_held_ticks: [[u64; HELD + 1]; SLOTS + 1],
    pub state_speed_cmps: [u64; SLOTS + 1],
    pub assignment: [[u64; 4]; Duty::COUNT],
    pub assignment_state: [[u64; SLOTS]; Duty::COUNT],
    pub changes_out_of_possession: u64,
    pub changes_after_election: u64,
    pub changes_after_duty: u64,
    pub changes_after_both: u64,
    pub carrier_ticks: [u64; 2],
    pub crowded_ticks: [u64; 2],
    pub attack_job: [u64; AttackJob::COUNT],
    pub attack_job_ahead: [u64; AttackJob::COUNT],
    pub attack_job_state: [[u64; SLOTS + 1]; AttackJob::COUNT],
    pub central_pair_ticks: u64,
    pub central_pair_in_area: u64,
    pub possession_ticks: u64,
    pub free_ahead: u64,
    pub strikes: u64,
    pub strike_ahead: u64,
    pub strike_in_area: u64,
    pub spells: [u64; MidfieldPlace::COUNT],
    pub spell_closing_ticks: [u64; MidfieldPlace::COUNT],
    pub spell_containing_ticks: [u64; MidfieldPlace::COUNT],
    pub spell_ends: [[u64; SpellEnd::COUNT]; MidfieldPlace::COUNT],
    pub tackling_entries: u64,
    pub challenges: [u64; 2],
    pub challenge_outcomes: [[u64; 3]; 2],
    pub challenge_speed_cmps: u64,
    pub challenge_goal_side: u64,
    pub tackle_takes: [u64; TackleTake::COUNT],
    pub headers: [u64; MidHeaderRule::COUNT],
    pub header_state: [[u64; SLOTS + 1]; MidHeaderRule::COUNT],
    pub header_leap: [u64; MidHeaderRule::COUNT],
    pub header_booked: [[u64; HeaderBooking::COUNT]; MidHeaderRule::COUNT],
    pub heading_entries: u64,
    pub heading_exits: [u64; HeadingExit::COUNT],
    pub drops: u64,
    pub drop_decided: u64,
    pub drop_reach_men: [[u64; 3]; 2],
    pub drop_ends: [[u64; DropEnd::COUNT]; DropReach::COUNT],
    pub clearance_after: [u64; 3],
    pub doors: [[[u64; 2]; SLOTS]; MidSituation::COUNT],
    pub phase_ticks: [u64; MidPhase::COUNT],
    pub relaxed_alert_changes: u64,
    pub relaxed_alert_reversed: u64,
    pub chase_entries: [u64; SLOTS],
    pub chase_reversed: [[u64; ChaseYield::COUNT]; SLOTS],
    pub receptions: [u64; SLOTS + 1],
    pub reception_running_age: [u64; 3],
    pub reception_pressed: u64,
    pub releases: [u64; 2],
    pub release_ticks: [u64; 2],
    pub release_forced: [u64; 2],
}

const ZERO: AtomicU64 = AtomicU64::new(0);
const SLOTS: usize = MidfielderState::SLOTS;
const PLACES: usize = MidfieldPlace::COUNT;
const HELD: usize = BallHeld::COUNT;
const BANDS: usize = DefenderCensus::BANDS;

static MOTION_TICKS: [[[AtomicU64; 2]; HELD]; PLACES] =
    [const { [const { [ZERO; 2] }; HELD] }; PLACES];
static MOTION_PATH_MM: [[[AtomicU64; 2]; HELD]; PLACES] =
    [const { [const { [ZERO; 2] }; HELD] }; PLACES];
static MOTION_BANDS: [[[[AtomicU64; BANDS]; 2]; HELD]; PLACES] =
    [const { [const { [const { [ZERO; BANDS] }; 2] }; HELD] }; PLACES];
static SPRINTS: [[[AtomicU64; 2]; HELD]; PLACES] = [const { [const { [ZERO; 2] }; HELD] }; PLACES];
static STATE_TICKS: [AtomicU64; SLOTS + 1] = [ZERO; SLOTS + 1];
static STATE_BALL_TICKS: [AtomicU64; SLOTS + 1] = [ZERO; SLOTS + 1];
static STATE_HELD_TICKS: [[AtomicU64; HELD + 1]; SLOTS + 1] =
    [const { [ZERO; HELD + 1] }; SLOTS + 1];
static STATE_SPEED_CMPS: [AtomicU64; SLOTS + 1] = [ZERO; SLOTS + 1];
static ASSIGNMENT: [[AtomicU64; 4]; Duty::COUNT] = [const { [ZERO; 4] }; Duty::COUNT];
static ASSIGNMENT_STATE: [[AtomicU64; SLOTS]; Duty::COUNT] = [const { [ZERO; SLOTS] }; Duty::COUNT];
static CHANGES_OUT_OF_POSSESSION: AtomicU64 = ZERO;
static CHANGES_AFTER_ELECTION: AtomicU64 = ZERO;
static CHANGES_AFTER_DUTY: AtomicU64 = ZERO;
static CHANGES_AFTER_BOTH: AtomicU64 = ZERO;
static CARRIER_TICKS: [AtomicU64; 2] = [ZERO; 2];
static CROWDED_TICKS: [AtomicU64; 2] = [ZERO; 2];
static ATTACK_JOB: [AtomicU64; AttackJob::COUNT] = [ZERO; AttackJob::COUNT];
static ATTACK_JOB_AHEAD: [AtomicU64; AttackJob::COUNT] = [ZERO; AttackJob::COUNT];
static ATTACK_JOB_STATE: [[AtomicU64; SLOTS + 1]; AttackJob::COUNT] =
    [const { [ZERO; SLOTS + 1] }; AttackJob::COUNT];
static CENTRAL_PAIR_TICKS: AtomicU64 = ZERO;
static CENTRAL_PAIR_IN_AREA: AtomicU64 = ZERO;
static POSSESSION_TICKS: AtomicU64 = ZERO;
static FREE_AHEAD: AtomicU64 = ZERO;
static STRIKES: AtomicU64 = ZERO;
static STRIKE_AHEAD: AtomicU64 = ZERO;
static STRIKE_IN_AREA: AtomicU64 = ZERO;
static SPELLS: [AtomicU64; PLACES] = [ZERO; PLACES];
static SPELL_CLOSING_TICKS: [AtomicU64; PLACES] = [ZERO; PLACES];
static SPELL_CONTAINING_TICKS: [AtomicU64; PLACES] = [ZERO; PLACES];
static SPELL_ENDS: [[AtomicU64; SpellEnd::COUNT]; PLACES] =
    [const { [ZERO; SpellEnd::COUNT] }; PLACES];
static TACKLING_ENTRIES: AtomicU64 = ZERO;
static CHALLENGES: [AtomicU64; 2] = [ZERO; 2];
static CHALLENGE_OUTCOMES: [[AtomicU64; 3]; 2] = [const { [ZERO; 3] }; 2];
static CHALLENGE_SPEED_CMPS: AtomicU64 = ZERO;
static CHALLENGE_GOAL_SIDE: AtomicU64 = ZERO;
static TACKLE_TAKES: [AtomicU64; TackleTake::COUNT] = [ZERO; TackleTake::COUNT];
static HEADERS: [AtomicU64; MidHeaderRule::COUNT] = [ZERO; MidHeaderRule::COUNT];
static HEADER_STATE: [[AtomicU64; SLOTS + 1]; MidHeaderRule::COUNT] =
    [const { [ZERO; SLOTS + 1] }; MidHeaderRule::COUNT];
static HEADER_LEAP: [AtomicU64; MidHeaderRule::COUNT] = [ZERO; MidHeaderRule::COUNT];
static HEADER_BOOKED: [[AtomicU64; HeaderBooking::COUNT]; MidHeaderRule::COUNT] =
    [const { [ZERO; HeaderBooking::COUNT] }; MidHeaderRule::COUNT];
static HEADING_ENTRIES: AtomicU64 = ZERO;
static HEADING_EXITS: [AtomicU64; HeadingExit::COUNT] = [ZERO; HeadingExit::COUNT];
static DROPS: AtomicU64 = ZERO;
static DROP_DECIDED: AtomicU64 = ZERO;
static DROP_REACH_MEN: [[AtomicU64; 3]; 2] = [const { [ZERO; 3] }; 2];
static DROP_ENDS: [[AtomicU64; DropEnd::COUNT]; DropReach::COUNT] =
    [const { [ZERO; DropEnd::COUNT] }; DropReach::COUNT];
static DOORS: [[[AtomicU64; 2]; SLOTS]; MidSituation::COUNT] =
    [const { [const { [ZERO; 2] }; SLOTS] }; MidSituation::COUNT];
static PHASE_TICKS: [AtomicU64; MidPhase::COUNT] = [ZERO; MidPhase::COUNT];
static RELAXED_ALERT_CHANGES: AtomicU64 = ZERO;
static RELAXED_ALERT_REVERSED: AtomicU64 = ZERO;
static CHASE_ENTRIES: [AtomicU64; SLOTS] = [ZERO; SLOTS];
static CHASE_REVERSED: [[AtomicU64; ChaseYield::COUNT]; SLOTS] =
    [const { [ZERO; ChaseYield::COUNT] }; SLOTS];
static RECEPTIONS: [AtomicU64; SLOTS + 1] = [ZERO; SLOTS + 1];
static RECEPTION_RUNNING_AGE: [AtomicU64; 3] = [ZERO; 3];
static RECEPTION_PRESSED: AtomicU64 = ZERO;
static RELEASES: [AtomicU64; 2] = [ZERO; 2];
static RELEASE_TICKS: [AtomicU64; 2] = [ZERO; 2];
static RELEASE_FORCED: [AtomicU64; 2] = [ZERO; 2];
static CLEARANCE_AFTER: [AtomicU64; 3] = [ZERO; 3];

/// The open episodes of one match.
#[derive(Debug, Default)]
pub struct MidfielderCensus {
    watches: Vec<MidfielderWatch>,
    possession: TeamPossession,
    owner_last_tick: Option<u32>,
    drop: Option<DropTrack>,
    drop_armed: bool,
    /// Each side's clearances so far, home first, to see the next one.
    clearances: [u32; 2],
    /// The clearance being followed: the side that made it, and when.
    cleared: Option<(u32, u64)>,
    live_ticks: u64,
    slow_live_ticks: u64,
}

impl MidfielderCensus {
    /// A quarter second: the window a speed is read over.
    const TRAIL: usize = 26;
    const METRES_PER_UNIT: f32 = 0.125;
    const SECONDS_PER_TICK: f32 = 0.01;
    /// A sprint is a second above the top band's edge; it ends when he drops
    /// below a jog's.
    const SPRINT_TICKS: u32 = 100;
    /// 300 ms: a phase change undone inside this is a two-cycle.
    const REVERSAL_TICKS: u64 = 30;
    /// AI ticks a door's chase is watched for a yield.
    const CHASE_AI_TICKS: u8 = 10;
    /// An opponent this close at a reception has him pressed (3 m).
    const PRESSED_AT_RECEPTION: f32 = 24.0;
    /// Nobody within this of a team-mate ahead of the ball and he is free
    /// (8 m), the spacing census's radius.
    const FREE_RADIUS: f32 = 64.0;
    /// `Running`'s age at a reception, in AI ticks: 0.4 s and 1.2 s.
    const RUNNING_AGE_EDGES: [u64; 2] = [20, 60];
    /// A dropping ball nobody has touched for five seconds is no longer one.
    const DROP_TICKS: u64 = 500;
    /// Who has the ball is read three seconds after a clearance.
    const AFTER_CLEARANCE: u64 = 300;

    fn bump(counter: &AtomicU64) {
        counter.fetch_add(1, Ordering::Relaxed);
    }

    fn is_midfielder(player: &MatchPlayer) -> bool {
        player.tactical_position.current_position.position_group()
            == PlayerFieldPositionGroup::Midfielder
    }

    fn side_index(side: PlayerSide) -> usize {
        match side {
            PlayerSide::Left => 0,
            PlayerSide::Right => 1,
        }
    }

    fn state_slot(state: PlayerState) -> usize {
        match state {
            PlayerState::Midfielder(s) => s as usize,
            _ => SLOTS,
        }
    }

    pub fn slot_name(slot: usize) -> String {
        MidfielderState::from_slot(slot)
            .map_or_else(|| "not a midfielder state".to_string(), |s| s.to_string())
    }

    fn is_engaging(state: PlayerState) -> bool {
        matches!(
            state,
            PlayerState::Midfielder(s)
                if matches!(s.family(), MidfielderFamily::Close | MidfielderFamily::Duel)
        )
    }

    // ── Hooks ────────────────────────────────────────────────────────

    /// A challenge `Tackling` has just rolled.
    pub fn note_challenge(
        ctx: &StateProcessingContext,
        carrier_position: Vector3<f32>,
        kind: ChallengeKind,
        outcome: ChallengeOutcome,
    ) {
        let own_goal = ctx.ball().direction_to_own_goal();
        let goal_side = (ctx.player.position - own_goal).xy().norm()
            < (carrier_position - own_goal).xy().norm();
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
        let mut census = ctx.context.midfielder_census.borrow_mut();
        if let Some(spell) = census.watch(ctx.player.id).spell.as_mut() {
            spell.last = Some(outcome);
        }
    }

    /// A ball he came away with from `Tackling`, and the rule that gave it.
    pub fn note_tackle_take(take: TackleTake) {
        Self::bump(&TACKLE_TAKES[take as usize]);
    }

    /// One header a midfielder played: the rule that decided it, whether he
    /// left the ground, and what it was booked as.
    pub fn note_header(
        rule: HeaderRule,
        player: &MatchPlayer,
        leapt: bool,
        booking: HeaderBooking,
    ) {
        if !Self::is_midfielder(player) {
            return;
        }
        let r = MidHeaderRule::of(rule) as usize;
        Self::bump(&HEADERS[r]);
        Self::bump(&HEADER_STATE[r][Self::state_slot(player.state)]);
        if leapt || player.is_airborne() {
            Self::bump(&HEADER_LEAP[r]);
        }
        Self::bump(&HEADER_BOOKED[r][booking as usize]);
    }

    pub fn note_heading_exit(exit: HeadingExit) {
        Self::bump(&HEADING_EXITS[exit as usize]);
    }

    /// `Running`'s forced release fired for the man on the ball.
    pub fn note_forced_release(ctx: &StateProcessingContext) {
        let mut census = ctx.context.midfielder_census.borrow_mut();
        if let Some(reception) = census.watch(ctx.player.id).reception.as_mut() {
            reception.forced = true;
        }
    }

    /// A shot struck: the striker's team-mates ahead of the ball and in the
    /// area he is shooting at.
    pub fn note_strike(field: &MatchField, shooter_id: u32, area: &PenaltyArea) {
        let Some(shooter) = field.players.iter().find(|p| p.id == shooter_id) else {
            return;
        };
        let Some(side) = shooter.side else {
            return;
        };
        let forward = side.forward_dir_x();
        let ball_x = field.ball.position.x;
        let (mut ahead, mut in_area) = (0u64, 0u64);
        for p in field.players.iter().filter(|p| {
            p.team_id == shooter.team_id
                && p.id != shooter_id
                && !p.off_pitch
                && !p.tactical_position.current_position.is_goalkeeper()
        }) {
            if (p.position.x - ball_x) * forward > 0.0 {
                ahead += 1;
            }
            if area.contains(&p.position) {
                in_area += 1;
            }
        }
        Self::bump(&STRIKES);
        STRIKE_AHEAD.fetch_add(ahead, Ordering::Relaxed);
        STRIKE_IN_AREA.fetch_add(in_area, Ordering::Relaxed);
    }

    /// One midfielder's AI tick, after his state's handler has decided.
    /// `before` is his state before the dispatcher's overrides, `redirect`
    /// what they did.
    pub fn note_dispatch(
        ctx: &StateProcessingContext,
        before: PlayerState,
        redirect: DispatchRedirect,
        result: &StateProcessingResult,
    ) {
        let PlayerState::Midfielder(current) = ctx.player.state else {
            return;
        };
        let decided = result.state.unwrap_or(ctx.player.state);
        let changed = result
            .state
            .is_some_and(|s| s.compact_id() != ctx.player.state.compact_id());
        let slot = current as usize;
        let control = ctx.team().is_control_ball();

        if before == PlayerState::Midfielder(MidfielderState::Heading)
            && redirect != DispatchRedirect::None
        {
            Self::note_heading_exit(HeadingExit::Dispatcher);
        }

        let since_flip = {
            let mut census = ctx.context.midfielder_census.borrow_mut();
            let watch = census.watch(ctx.player.id);
            if watch.control != Some(control) {
                watch.since_flip = if watch.control.is_some() { 0 } else { u32::MAX };
                watch.control = Some(control);
            } else {
                watch.since_flip = watch.since_flip.saturating_add(1);
            }
            watch.since_flip
        };

        for situation in MidSituation::ALL {
            if Self::situation_holds(ctx, situation, control, since_flip) {
                let cell = &DOORS[situation as usize][slot];
                Self::bump(&cell[0]);
                if situation.answered_by(decided) || situation.answered_by(ctx.player.state) {
                    Self::bump(&cell[1]);
                }
            }
        }

        let (duty, elected) = if control {
            (None, None)
        } else {
            let plan = ctx.context.defence_plan_for_team(ctx.player.team_id);
            let duty = Duty::of(plan, ctx.player.id);
            let elected = ctx.team().is_best_player_to_chase_ball();
            let licensed = TackleEngagement::may_engage_carrier(ctx);
            Self::bump(&ASSIGNMENT[duty][usize::from(!elected) * 2 + usize::from(!licensed)]);
            Self::bump(&ASSIGNMENT_STATE[duty][slot]);
            (Some(duty), Some(elected))
        };

        let mut census = ctx.context.midfielder_census.borrow_mut();
        let watch = census.watch(ctx.player.id);
        if !control && changed {
            Self::bump(&CHANGES_OUT_OF_POSSESSION);
            let after_election = watch.elected.is_some() && watch.elected != elected;
            let after_duty = watch.duty.is_some() && watch.duty != duty;
            match (after_election, after_duty) {
                (true, true) => Self::bump(&CHANGES_AFTER_BOTH),
                (true, false) => Self::bump(&CHANGES_AFTER_ELECTION),
                (false, true) => Self::bump(&CHANGES_AFTER_DUTY),
                (false, false) => {}
            }
        }
        watch.duty = duty;
        watch.elected = elected;

        Self::follow_chase(watch, before, current, redirect, result);
    }

    /// A chase a state's own door started: confirmed when he is in
    /// `TakeBall` at his next AI tick, reversed when the dispatcher yields
    /// him within ten.
    fn follow_chase(
        watch: &mut MidfielderWatch,
        before: PlayerState,
        current: MidfielderState,
        redirect: DispatchRedirect,
        result: &StateProcessingResult,
    ) {
        let in_chase = before == PlayerState::Midfielder(MidfielderState::TakeBall);
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
        let door_entry = current != MidfielderState::TakeBall
            && redirect == DispatchRedirect::None
            && result.state == Some(PlayerState::Midfielder(MidfielderState::TakeBall));
        if door_entry {
            watch.chase = Some(ChaseEntry {
                door: current as usize,
                ai_ticks: 0,
                confirmed: false,
            });
        }
    }

    fn situation_holds(
        ctx: &StateProcessingContext,
        situation: MidSituation,
        control: bool,
        since_flip: u32,
    ) -> bool {
        match situation {
            MidSituation::LicensedCarrier | MidSituation::UnlicensedCarrier => {
                let licensed = situation == MidSituation::LicensedCarrier;
                let reach = if licensed {
                    TackleEngagement::COMMIT
                } else {
                    MidSituation::NEAR
                };
                ctx.players()
                    .opponents()
                    .with_ball()
                    .next()
                    .is_some_and(|carrier| {
                        carrier.distance(ctx) <= reach
                            && TackleEngagement::may_engage_carrier(ctx) == licensed
                    })
            }
            MidSituation::LooseBall => {
                !ctx.tick_context.ball.is_owned
                    && ctx.player.side.is_some_and(|side| {
                        ctx.tick_context.chase.cost_of(ctx.player.id).is_some()
                            && ctx.tick_context.chase.is_designated(side, ctx.player.id)
                    })
            }
            MidSituation::HighBall => {
                ctx.tick_context.ball.aerial_contest_winner == Some(ctx.player.id)
            }
            MidSituation::BallLost => !control && since_flip < MidSituation::AFTER_FLIP_AI_TICKS,
            MidSituation::BallWon => {
                control
                    && since_flip < MidSituation::AFTER_FLIP_AI_TICKS
                    && ctx.ball().owner_id() != Some(ctx.player.id)
            }
        }
    }

    // ── Every tick ───────────────────────────────────────────────────

    fn watch(&mut self, id: u32) -> &mut MidfielderWatch {
        match self.watches.iter().position(|w| w.id == id) {
            Some(at) => &mut self.watches[at],
            None => {
                self.watches.push(MidfielderWatch::new(id));
                self.watches.last_mut().expect("just pushed")
            }
        }
    }

    /// One sample a tick, after everybody has moved and every event has
    /// landed. `attacks` are each team's attack plan, `areas` the left and
    /// the right penalty area.
    pub fn sample(
        &mut self,
        field: &MatchField,
        attacks: [(u32, AttackPlan); 2],
        areas: [PenaltyArea; 2],
        tick: u64,
    ) {
        let dead = field.ball.awaiting_restart.is_some();
        let owner = field.ball.current_owner;
        self.possession.update(field);
        let holder = self.possession.holder();
        let carrier = owner.and_then(|id| field.players.iter().find(|p| p.id == id));
        let received = owner.filter(|id| self.owner_last_tick != Some(*id));
        self.owner_last_tick = owner;

        self.follow_carrier(field, carrier, &areas);
        self.follow_attack(field, holder, &attacks, &areas, dead);
        self.follow_drop(field, tick);
        self.follow_clearance(field, tick);

        let (mut live_ticks, mut slow_live_ticks) = (0, 0);
        for player in field
            .players
            .iter()
            .filter(|p| !p.off_pitch && Self::is_midfielder(p))
        {
            let place = MidfieldPlace::of(player.tactical_position.current_position);
            let has_ball = owner == Some(player.id);
            let held = BallHeld::of(holder, player.team_id);
            let carrier = carrier.filter(|c| c.team_id != player.team_id);
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
            Self::bump(&STATE_HELD_TICKS[slot][if dead { HELD } else { held }]);
            if has_ball {
                Self::bump(&STATE_BALL_TICKS[slot]);
            }
            STATE_SPEED_CMPS[slot].fetch_add((speed * 100.0) as u64, Ordering::Relaxed);

            if !dead {
                live_ticks += 1;
                if speed <= DefenderCensus::BAND_EDGES[1] {
                    slow_live_ticks += 1;
                }
            }
            if let Some(place) = place {
                let live = usize::from(!dead);
                Self::bump(&MOTION_TICKS[place][held][live]);
                MOTION_PATH_MM[place][held][live].fetch_add(
                    (step * Self::METRES_PER_UNIT * 1000.0) as u64,
                    Ordering::Relaxed,
                );
                Self::bump(&MOTION_BANDS[place][held][live][Self::band(speed)]);
                if speed > DefenderCensus::BAND_EDGES[3] {
                    watch.sprint_ticks += 1;
                    if watch.sprint_ticks == Self::SPRINT_TICKS {
                        Self::bump(&SPRINTS[place][held][live]);
                    }
                } else if speed < DefenderCensus::BAND_EDGES[2] {
                    watch.sprint_ticks = 0;
                }
            }

            if received == Some(player.id) {
                Self::open_reception(watch, field, player, tick);
            } else if !has_ball && let Some(reception) = watch.reception.take() {
                let pressed = usize::from(reception.pressed);
                Self::bump(&RELEASES[pressed]);
                RELEASE_TICKS[pressed].fetch_add(tick - reception.start, Ordering::Relaxed);
                if reception.forced {
                    Self::bump(&RELEASE_FORCED[pressed]);
                }
            }

            let entered = watch
                .seen_state
                .is_some_and(|s| s.compact_id() != state.compact_id());
            if entered && state == PlayerState::Midfielder(MidfielderState::Tackling) {
                Self::bump(&TACKLING_ENTRIES);
            }
            if entered && state == PlayerState::Midfielder(MidfielderState::Heading) {
                Self::bump(&HEADING_ENTRIES);
            }
            watch.seen_state = Some(state);
            watch.seen_in_state_time = player.in_state_time;

            Self::follow_phase(watch, MidPhase::of(state, has_ball), tick);
            Self::follow_spell(watch, place.unwrap_or(1), player, carrier, owner);
        }
        self.live_ticks += live_ticks;
        self.slow_live_ticks += slow_live_ticks;
    }

    fn band(speed_mps: f32) -> usize {
        DefenderCensus::BAND_EDGES
            .iter()
            .position(|edge| speed_mps <= *edge)
            .unwrap_or(DefenderCensus::BAND_EDGES.len())
    }

    /// The state he was in as the ball reached him, `Running`'s age if that
    /// was it, and whether an opponent was on him.
    fn open_reception(
        watch: &mut MidfielderWatch,
        field: &MatchField,
        player: &MatchPlayer,
        tick: u64,
    ) {
        let state = watch.seen_state.unwrap_or(player.state);
        Self::bump(&RECEPTIONS[Self::state_slot(state)]);
        if state == PlayerState::Midfielder(MidfielderState::Running) {
            let age = Self::RUNNING_AGE_EDGES
                .iter()
                .position(|edge| watch.seen_in_state_time <= *edge)
                .unwrap_or(Self::RUNNING_AGE_EDGES.len());
            Self::bump(&RECEPTION_RUNNING_AGE[age]);
        }
        let pressed = field.players.iter().any(|p| {
            p.team_id != player.team_id
                && !p.off_pitch
                && (p.position - player.position).xy().norm() <= Self::PRESSED_AT_RECEPTION
        });
        if pressed {
            Self::bump(&RECEPTION_PRESSED);
        }
        watch.reception = Some(ReceptionTrack {
            start: tick,
            pressed,
            forced: false,
        });
    }

    fn follow_phase(watch: &mut MidfielderWatch, phase: usize, tick: u64) {
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
                    (MidPhase::RELAXED, MidPhase::ALERT) | (MidPhase::ALERT, MidPhase::RELAXED)
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
        watch: &mut MidfielderWatch,
        place: usize,
        player: &MatchPlayer,
        carrier: Option<&MatchPlayer>,
        owner: Option<u32>,
    ) {
        let engaging = Self::is_engaging(player.state);
        let closing = player.state == PlayerState::Midfielder(MidfielderState::Pressing);
        if let Some(spell) = watch.spell.as_mut() {
            let same_carrier = carrier.is_some_and(|c| c.id == spell.carrier);
            if engaging && same_carrier && spell.last.is_none() {
                if closing {
                    spell.closing += 1;
                } else {
                    spell.containing += 1;
                }
                return;
            }
            let spell = watch.spell.take().expect("checked above");
            let end = match spell.last {
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
            Self::bump(&SPELLS[spell.place]);
            SPELL_CLOSING_TICKS[spell.place].fetch_add(spell.closing, Ordering::Relaxed);
            SPELL_CONTAINING_TICKS[spell.place].fetch_add(spell.containing, Ordering::Relaxed);
            Self::bump(&SPELL_ENDS[spell.place][end as usize]);
        }
        if engaging && let Some(c) = carrier {
            watch.spell = Some(DuelSpell {
                carrier: c.id,
                place,
                closing: u64::from(closing),
                containing: u64::from(!closing),
                last: None,
            });
        }
    }

    /// Midfielders on the carrier, outside the area of the side defending
    /// and inside it.
    fn follow_carrier(
        &mut self,
        field: &MatchField,
        carrier: Option<&MatchPlayer>,
        areas: &[PenaltyArea; 2],
    ) {
        let Some(carrier) = carrier else {
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
            .filter(|p| p.team_id != carrier.team_id && !p.off_pitch && Self::is_midfielder(p))
            .filter(|p| Self::is_engaging(p.state))
            .count();
        Self::bump(&CARRIER_TICKS[in_area]);
        if engaged >= 2 {
            Self::bump(&CROWDED_TICKS[in_area]);
        }
    }

    /// The side holding the ball: each of its midfielders' jobs against
    /// where he is, its free men ahead of the ball, and the central pair of
    /// each side in the opposition's area.
    fn follow_attack(
        &mut self,
        field: &MatchField,
        holder: Option<u32>,
        attacks: &[(u32, AttackPlan); 2],
        areas: &[PenaltyArea; 2],
        dead: bool,
    ) {
        if dead {
            return;
        }
        for (team, _) in attacks.iter() {
            let Some(side) = field
                .players
                .iter()
                .find(|p| p.team_id == *team)
                .and_then(|p| p.side)
            else {
                continue;
            };
            let their_area = &areas[1 - Self::side_index(side)];
            let mut central = 0;
            let mut central_in_area = 0;
            for p in field.players.iter().filter(|p| {
                p.team_id == *team
                    && !p.off_pitch
                    && MidfieldPlace::of(p.tactical_position.current_position)
                        == Some(MidfieldPlace::CENTRAL)
            }) {
                central += 1;
                if their_area.contains(&p.position) {
                    central_in_area += 1;
                }
            }
            if central >= 2 {
                Self::bump(&CENTRAL_PAIR_TICKS);
                if central_in_area >= 2 {
                    Self::bump(&CENTRAL_PAIR_IN_AREA);
                }
            }
        }

        let Some(team) = holder else {
            return;
        };
        let Some(side) = field
            .players
            .iter()
            .find(|p| p.team_id == team)
            .and_then(|p| p.side)
        else {
            return;
        };
        let Some((_, plan)) = attacks.iter().find(|(id, _)| *id == team) else {
            return;
        };
        let forward = side.forward_dir_x();
        let ball_x = field.ball.position.x;
        let owner = field.ball.current_owner;
        Self::bump(&POSSESSION_TICKS);
        let mut free = 0u64;
        for p in field.players.iter().filter(|p| {
            p.team_id == team
                && owner != Some(p.id)
                && !p.off_pitch
                && !p.tactical_position.current_position.is_goalkeeper()
        }) {
            let ahead = (p.position.x - ball_x) * forward > 0.0;
            if ahead
                && !field.players.iter().any(|o| {
                    o.team_id != team
                        && !o.off_pitch
                        && (o.position - p.position).xy().norm() < Self::FREE_RADIUS
                })
            {
                free += 1;
            }
            if !Self::is_midfielder(p) {
                continue;
            }
            let job = AttackJob::of(plan, p.id);
            Self::bump(&ATTACK_JOB[job]);
            if ahead {
                Self::bump(&ATTACK_JOB_AHEAD[job]);
            }
            Self::bump(&ATTACK_JOB_STATE[job][Self::state_slot(p.state)]);
        }
        FREE_AHEAD.fetch_add(free, Ordering::Relaxed);
    }

    /// Three seconds after a clearance, who has the ball: the side that
    /// cleared it, the other side, or nobody.
    fn follow_clearance(&mut self, field: &MatchField, tick: u64) {
        let teams = [field.home_team_id, field.away_team_id];
        let made = |team: u32| {
            field
                .players
                .iter()
                .filter(|p| p.team_id == team)
                .map(|p| u32::from(p.statistics.clearances))
                .sum::<u32>()
        };
        let now = [made(teams[0]), made(teams[1])];
        if let Some((team, at)) = self.cleared
            && tick.saturating_sub(at) >= Self::AFTER_CLEARANCE
        {
            let holder = field
                .ball
                .current_owner
                .and_then(|id| field.players.iter().find(|p| p.id == id))
                .map(|p| p.team_id);
            let after = match holder {
                Some(t) if t == team => 0,
                Some(_) => 1,
                None => 2,
            };
            Self::bump(&CLEARANCE_AFTER[after]);
            self.cleared = None;
        }
        if self.cleared.is_none()
            && let Some(i) = (0..2).find(|i| now[*i] > self.clearances[*i])
        {
            self.cleared = Some((teams[i], tick));
        }
        self.clearances = now;
    }

    /// A lofted ball with nobody named for it, from the moment it comes down
    /// within anybody's reach until somebody touches it or it goes dead:
    /// who of each side could have headed it, and what became of it.
    fn follow_drop(&mut self, field: &MatchField, tick: u64) {
        let ball = &field.ball;
        let unclaimed = ball.current_owner.is_none()
            && !ball.held_in_hands
            && ball.awaiting_restart.is_none()
            && ball.pass_target_player_id.is_none()
            && ball.cached_shot_target.is_none();

        if let Some(mut track) = self.drop {
            let touched = ball.last_touch_tick > track.opened || ball.current_owner.is_some();
            let end = if touched {
                let same = ball.last_touch_team_id == track.team;
                Some(match (track.landed, same) {
                    (false, true) => DropEnd::HEADED_SAME,
                    (false, false) => DropEnd::HEADED_OTHER,
                    (true, true) => DropEnd::LANDED_SAME,
                    (true, false) => DropEnd::LANDED_OTHER,
                })
            } else if ball.awaiting_restart.is_some() {
                Some(DropEnd::OUT)
            } else if tick.saturating_sub(track.opened) > Self::DROP_TICKS {
                Some(DropEnd::TIMED_OUT)
            } else {
                None
            };
            if let Some(end) = end {
                self.drop = None;
                Self::close_drop(track, end);
                return;
            }
            track.decided |= ball.aerial_decided || ball.aerial_delivery.is_some();
            if !track.landed {
                if ball.position.z <= AerialReach::VOLLEY {
                    track.landed = true;
                } else {
                    for p in field
                        .players
                        .iter()
                        .filter(|p| {
                            !p.off_pitch && !p.tactical_position.current_position.is_goalkeeper()
                        })
                        .filter(|p| PlayerReach::can_strike(ball, p, true))
                    {
                        let i = usize::from(Some(p.team_id) != track.team);
                        track.reach[i] = (track.reach[i] + 1).min(2);
                    }
                }
            }
            self.drop = Some(track);
            return;
        }

        if !unclaimed || ball.position.z <= AerialReach::VOLLEY {
            self.drop_armed = false;
            return;
        }
        if ball.position.z > AerialReach::HIGHEST {
            self.drop_armed = true;
            return;
        }
        if self.drop_armed && ball.velocity.z < 0.0 {
            self.drop_armed = false;
            self.drop = Some(DropTrack {
                opened: tick,
                team: ball.last_touch_team_id,
                reach: [0, 0],
                landed: false,
                decided: false,
            });
        }
    }

    fn close_drop(track: DropTrack, end: usize) {
        Self::bump(&DROPS);
        if track.decided {
            Self::bump(&DROP_DECIDED);
        }
        Self::bump(&DROP_REACH_MEN[0][usize::from(track.reach[0])]);
        Self::bump(&DROP_REACH_MEN[1][usize::from(track.reach[1])]);
        Self::bump(&DROP_ENDS[DropReach::of(track.reach)][end]);
    }

    /// Full time: the midfielders' live ticks, and those at or below a walk,
    /// go on the match tally, which is what the realism bands read.
    pub fn finish(&mut self, tally: &mut MatchTally) {
        tally.midfielder_motion.live_ticks += self.live_ticks;
        tally.midfielder_motion.slow_live_ticks += self.slow_live_ticks;
    }

    // ── Totals ───────────────────────────────────────────────────────

    fn read<const N: usize>(counters: &[AtomicU64; N]) -> [u64; N] {
        std::array::from_fn(|i| counters[i].load(Ordering::Relaxed))
    }

    fn read2<const N: usize, const M: usize>(counters: &[[AtomicU64; M]; N]) -> [[u64; M]; N] {
        std::array::from_fn(|i| Self::read(&counters[i]))
    }

    fn read3<const N: usize, const M: usize, const K: usize>(
        counters: &[[[AtomicU64; K]; M]; N],
    ) -> [[[u64; K]; M]; N] {
        std::array::from_fn(|i| Self::read2(&counters[i]))
    }

    pub fn snapshot() -> MidfielderCensusSnapshot {
        MidfielderCensusSnapshot {
            motion_ticks: Self::read3(&MOTION_TICKS),
            motion_path_mm: Self::read3(&MOTION_PATH_MM),
            motion_bands: std::array::from_fn(|p| Self::read3(&MOTION_BANDS[p])),
            sprints: Self::read3(&SPRINTS),
            state_ticks: Self::read(&STATE_TICKS),
            state_ball_ticks: Self::read(&STATE_BALL_TICKS),
            state_held_ticks: Self::read2(&STATE_HELD_TICKS),
            state_speed_cmps: Self::read(&STATE_SPEED_CMPS),
            assignment: Self::read2(&ASSIGNMENT),
            assignment_state: Self::read2(&ASSIGNMENT_STATE),
            changes_out_of_possession: CHANGES_OUT_OF_POSSESSION.load(Ordering::Relaxed),
            changes_after_election: CHANGES_AFTER_ELECTION.load(Ordering::Relaxed),
            changes_after_duty: CHANGES_AFTER_DUTY.load(Ordering::Relaxed),
            changes_after_both: CHANGES_AFTER_BOTH.load(Ordering::Relaxed),
            carrier_ticks: Self::read(&CARRIER_TICKS),
            crowded_ticks: Self::read(&CROWDED_TICKS),
            attack_job: Self::read(&ATTACK_JOB),
            attack_job_ahead: Self::read(&ATTACK_JOB_AHEAD),
            attack_job_state: Self::read2(&ATTACK_JOB_STATE),
            central_pair_ticks: CENTRAL_PAIR_TICKS.load(Ordering::Relaxed),
            central_pair_in_area: CENTRAL_PAIR_IN_AREA.load(Ordering::Relaxed),
            possession_ticks: POSSESSION_TICKS.load(Ordering::Relaxed),
            free_ahead: FREE_AHEAD.load(Ordering::Relaxed),
            strikes: STRIKES.load(Ordering::Relaxed),
            strike_ahead: STRIKE_AHEAD.load(Ordering::Relaxed),
            strike_in_area: STRIKE_IN_AREA.load(Ordering::Relaxed),
            spells: Self::read(&SPELLS),
            spell_closing_ticks: Self::read(&SPELL_CLOSING_TICKS),
            spell_containing_ticks: Self::read(&SPELL_CONTAINING_TICKS),
            spell_ends: Self::read2(&SPELL_ENDS),
            tackling_entries: TACKLING_ENTRIES.load(Ordering::Relaxed),
            challenges: Self::read(&CHALLENGES),
            challenge_outcomes: Self::read2(&CHALLENGE_OUTCOMES),
            challenge_speed_cmps: CHALLENGE_SPEED_CMPS.load(Ordering::Relaxed),
            challenge_goal_side: CHALLENGE_GOAL_SIDE.load(Ordering::Relaxed),
            tackle_takes: Self::read(&TACKLE_TAKES),
            headers: Self::read(&HEADERS),
            header_state: Self::read2(&HEADER_STATE),
            header_leap: Self::read(&HEADER_LEAP),
            header_booked: Self::read2(&HEADER_BOOKED),
            heading_entries: HEADING_ENTRIES.load(Ordering::Relaxed),
            heading_exits: Self::read(&HEADING_EXITS),
            drops: DROPS.load(Ordering::Relaxed),
            drop_decided: DROP_DECIDED.load(Ordering::Relaxed),
            drop_reach_men: Self::read2(&DROP_REACH_MEN),
            drop_ends: Self::read2(&DROP_ENDS),
            clearance_after: Self::read(&CLEARANCE_AFTER),
            doors: std::array::from_fn(|s| Self::read2(&DOORS[s])),
            phase_ticks: Self::read(&PHASE_TICKS),
            relaxed_alert_changes: RELAXED_ALERT_CHANGES.load(Ordering::Relaxed),
            relaxed_alert_reversed: RELAXED_ALERT_REVERSED.load(Ordering::Relaxed),
            chase_entries: Self::read(&CHASE_ENTRIES),
            chase_reversed: Self::read2(&CHASE_REVERSED),
            receptions: Self::read(&RECEPTIONS),
            reception_running_age: Self::read(&RECEPTION_RUNNING_AGE),
            reception_pressed: RECEPTION_PRESSED.load(Ordering::Relaxed),
            releases: Self::read(&RELEASES),
            release_ticks: Self::read(&RELEASE_TICKS),
            release_forced: Self::read(&RELEASE_FORCED),
        }
    }

    pub fn reset() {
        let scalars = [
            &CHANGES_OUT_OF_POSSESSION,
            &CHANGES_AFTER_ELECTION,
            &CHANGES_AFTER_DUTY,
            &CHANGES_AFTER_BOTH,
            &CENTRAL_PAIR_TICKS,
            &CENTRAL_PAIR_IN_AREA,
            &POSSESSION_TICKS,
            &FREE_AHEAD,
            &STRIKES,
            &STRIKE_AHEAD,
            &STRIKE_IN_AREA,
            &TACKLING_ENTRIES,
            &CHALLENGE_SPEED_CMPS,
            &CHALLENGE_GOAL_SIDE,
            &HEADING_ENTRIES,
            &DROPS,
            &DROP_DECIDED,
            &RELAXED_ALERT_CHANGES,
            &RELAXED_ALERT_REVERSED,
            &RECEPTION_PRESSED,
        ];
        for counter in scalars
            .into_iter()
            .chain(MOTION_TICKS.iter().flatten().flatten())
            .chain(MOTION_PATH_MM.iter().flatten().flatten())
            .chain(MOTION_BANDS.iter().flatten().flatten().flatten())
            .chain(SPRINTS.iter().flatten().flatten())
            .chain(&STATE_TICKS)
            .chain(&STATE_BALL_TICKS)
            .chain(&STATE_SPEED_CMPS)
            .chain(ASSIGNMENT.iter().flatten())
            .chain(ASSIGNMENT_STATE.iter().flatten())
            .chain(&CARRIER_TICKS)
            .chain(&CROWDED_TICKS)
            .chain(&ATTACK_JOB)
            .chain(&ATTACK_JOB_AHEAD)
            .chain(ATTACK_JOB_STATE.iter().flatten())
            .chain(&SPELLS)
            .chain(&SPELL_CLOSING_TICKS)
            .chain(&SPELL_CONTAINING_TICKS)
            .chain(SPELL_ENDS.iter().flatten())
            .chain(&CHALLENGES)
            .chain(CHALLENGE_OUTCOMES.iter().flatten())
            .chain(&TACKLE_TAKES)
            .chain(&HEADERS)
            .chain(HEADER_STATE.iter().flatten())
            .chain(&HEADER_LEAP)
            .chain(HEADER_BOOKED.iter().flatten())
            .chain(&HEADING_EXITS)
            .chain(DROP_REACH_MEN.iter().flatten())
            .chain(DROP_ENDS.iter().flatten())
            .chain(DOORS.iter().flatten().flatten())
            .chain(&PHASE_TICKS)
            .chain(&CHASE_ENTRIES)
            .chain(CHASE_REVERSED.iter().flatten())
            .chain(STATE_HELD_TICKS.iter().flatten())
            .chain(&CLEARANCE_AFTER)
            .chain(&RECEPTIONS)
            .chain(&RECEPTION_RUNNING_AGE)
            .chain(&RELEASES)
            .chain(&RELEASE_TICKS)
            .chain(&RELEASE_FORCED)
        {
            counter.store(0, Ordering::Relaxed);
        }
    }
}
