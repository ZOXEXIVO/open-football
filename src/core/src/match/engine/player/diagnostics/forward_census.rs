//! **The forward action census** — how far and how fast a forward moves, the
//! job each assignment gives him against the state he is in, every run in
//! behind from start to end, his duels, his headers, the shots and passes his
//! states ask for, which situations each state acts on, his phases, the
//! loose-ball chases his own doors start, and how long he keeps a ball he
//! receives.
//!
//! The open episodes are per match and live on
//! [`MatchContext`](crate::r#match::MatchContext); the totals are
//! process-wide counters the `dev_match` harness prints. Nothing here
//! changes what the match does.

use crate::PlayerFieldPositionGroup;
use crate::r#match::engine::ball::ball::OffsideLine;
use crate::r#match::engine::teamplay::plans::attack::AttackPlan;
use crate::r#match::engine::teamplay::possession::TeamPossession;
use crate::r#match::forwarders::states::shared::ForwardRead;
use crate::r#match::forwarders::states::{ForwardFamily, ForwardState};
use crate::r#match::player::diagnostics::defender_census::{
    ChallengeOutcome, ChaseYield, DefenderCensus, DispatchRedirect, HeaderBooking, HeaderRule,
    SpellEnd,
};
use crate::r#match::player::diagnostics::midfielder_census::BallHeld;
use crate::r#match::player::state::PlayerState;
use crate::r#match::player::strategies::players::ops::box_movement::BoxMovement;
use crate::r#match::shared_states::{ChallengeKind, TackleEngagement};
use crate::r#match::{
    MatchField, MatchPlayer, MatchTally, PlayerSide, StateProcessingContext, StateProcessingResult,
};
use nalgebra::Vector3;
use std::sync::atomic::{AtomicU64, Ordering};

/// A forward's place in the front line, by his slot in the formation, on the
/// wide plan's own test (`WideBuilder::holds_a_wide_slot`): a slot in the
/// outer quarter of his side's shape, or the furthest out on his flank of a
/// front line of three or more. A 4-4-2's two strikers are central, a
/// 4-3-3's flank forwards wide.
pub struct FrontPlace;

impl FrontPlace {
    pub const COUNT: usize = 2;
    pub const LABELS: [&'static str; Self::COUNT] = ["central forward", "wide forward"];
    pub const CENTRAL: usize = 0;
    const WIDE: usize = 1;
    const WIDE_SLOT: f32 = 0.5;
    const FRONT_LINE_DEPTH: f32 = 0.9;

    fn of(field: &MatchField, player: &MatchPlayer) -> usize {
        let Some(side) = player.side else {
            return Self::CENTRAL;
        };
        let field_width = field.size.width as f32;
        let mates: Vec<&MatchPlayer> = field
            .players
            .iter()
            .filter(|p| {
                p.team_id == player.team_id && !p.tactical_position.current_position.is_goalkeeper()
            })
            .collect();
        let depth_of = |p: &MatchPlayer| side.attacking_progress_x(p.start_position.x, field_width);
        let (mut min_lat, mut max_lat) = (f32::MAX, f32::MIN);
        let (mut min_depth, mut max_depth) = (f32::MAX, f32::MIN);
        for p in &mates {
            min_lat = min_lat.min(p.start_position.y);
            max_lat = max_lat.max(p.start_position.y);
            min_depth = min_depth.min(depth_of(p));
            max_depth = max_depth.max(depth_of(p));
        }
        let lat_span = (max_lat - min_lat).max(f32::EPSILON);
        let depth_span = (max_depth - min_depth).max(f32::EPSILON);
        let lat = |p: &MatchPlayer| (p.start_position.y - min_lat) / lat_span;
        let depth = |p: &MatchPlayer| (depth_of(p) - min_depth) / depth_span;
        let out = |p: &MatchPlayer| (lat(p) - 0.5).abs();
        if out(player) * 2.0 >= Self::WIDE_SLOT {
            return Self::WIDE;
        }
        if depth(player) < Self::FRONT_LINE_DEPTH {
            return Self::CENTRAL;
        }
        let centre = field.size.height as f32 * 0.5;
        let flank = player.start_position.y < centre;
        let line_mates: Vec<&&MatchPlayer> = mates
            .iter()
            .filter(|p| p.id != player.id && depth(p) >= Self::FRONT_LINE_DEPTH)
            .collect();
        let outflanked = line_mates
            .iter()
            .any(|p| (p.start_position.y < centre) == flank && out(p) > out(player));
        if line_mates.len() >= 2 && !outflanked {
            Self::WIDE
        } else {
            Self::CENTRAL
        }
    }
}

/// The attack plan's job for a forward off the ball, highest first.
pub struct FwdJob;

impl FwdJob {
    pub const COUNT: usize = 7;
    pub const LABELS: [&'static str; Self::COUNT] = [
        "touchline",
        "box place, staged",
        "box place, not staged",
        "runner in behind",
        "short outlet",
        "kept behind the ball",
        "none",
    ];
    pub const BOX_STAGED: usize = 1;
    pub const BOX_UNSTAGED: usize = 2;

    fn of(plan: &AttackPlan, id: u32, staged: bool) -> usize {
        if plan.wide.flank_of(id).is_some() || plan.wide.is_overlap_runner(id) {
            0
        } else if plan.slot_of(id).is_some() {
            if staged {
                Self::BOX_STAGED
            } else {
                Self::BOX_UNSTAGED
            }
        } else if plan.far_runner == Some(id) {
            3
        } else if plan.near_support == Some(id) {
            4
        } else if plan.is_rest_defence(id) {
            5
        } else {
            6
        }
    }
}

/// The door that started a run in behind.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RunDoor {
    /// `CreatingSpace`'s own run door.
    CreatingSpace,
    /// `Running`'s `should_make_run_in_behind`.
    Running,
    /// `Running`'s phase dispatch on an attacking transition.
    PhaseDispatch,
    Other,
}

impl RunDoor {
    pub const COUNT: usize = 4;
    pub const LABELS: [&'static str; Self::COUNT] = [
        "CreatingSpace's run door",
        "Running's run door",
        "Running's phase dispatch",
        "other",
    ];
}

/// How a run in behind ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RunEnd {
    GotBall,
    Flagged,
    /// `RunInBehind::stays_viable` released him.
    StaysViable,
    /// The swarmed-passer door.
    Swarmed,
    /// The dispatcher took him.
    Dispatcher,
    /// The dispatcher sent him to collect a ball played to him.
    Collecting,
    /// His side lost the ball.
    BallLost,
    /// The read ended it: the run is no longer his.
    Read,
    Other,
}

impl RunEnd {
    pub const COUNT: usize = 9;
    pub const LABELS: [&'static str; Self::COUNT] = [
        "he got the ball",
        "he was flagged",
        "RunInBehind::stays_viable",
        "the swarmed-passer door",
        "the dispatcher",
        "sent to collect a ball played to him",
        "his side lost the ball",
        "the run is no longer his",
        "other",
    ];
}

/// Where a run in behind ended, against the referee's line.
pub struct LineGap;

impl LineGap {
    pub const COUNT: usize = 5;
    pub const LABELS: [&'static str; Self::COUNT] = [
        "more than 2 m inside",
        "0-2 m inside",
        "0-2 m beyond",
        "more than 2 m beyond",
        "no line",
    ];
    /// 2 m.
    const NEAR: f32 = 16.0;

    fn of(beyond: Option<f32>) -> usize {
        match beyond {
            None => 4,
            Some(d) if d < -Self::NEAR => 0,
            Some(d) if d < 0.0 => 1,
            Some(d) if d <= Self::NEAR => 2,
            Some(_) => 3,
        }
    }
}

/// How a forward came away with the ball from `Tackling`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FwdTackleTake {
    Duel,
    ClaimBranch,
}

impl FwdTackleTake {
    pub const COUNT: usize = 2;
    pub const LABELS: [&'static str; Self::COUNT] = ["the challenge", "Tackling's own ClaimBall"];
}

/// The rule that decided a forward's header.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FwdHeaderRule {
    CrossContest,
    /// The open-play aerial duel, the contest for a dropping ball among them.
    AerialDuel,
    /// `Heading`'s own duel against the nearest opponent.
    HeadingDuel,
    /// `Heading`'s corner path.
    HeadingCorner,
    OwnershipClaim,
}

impl FwdHeaderRule {
    pub const COUNT: usize = 5;
    pub const LABELS: [&'static str; Self::COUNT] = [
        "cross contest",
        "aerial duel",
        "Heading's own duel",
        "Heading's corner path",
        "ownership claim",
    ];

    fn of(rule: HeaderRule) -> FwdHeaderRule {
        match rule {
            HeaderRule::CrossContest | HeaderRule::HeadingShot => FwdHeaderRule::CrossContest,
            HeaderRule::AerialDuel => FwdHeaderRule::AerialDuel,
            HeaderRule::OwnershipClaim => FwdHeaderRule::OwnershipClaim,
        }
    }
}

/// Where `Heading` let him go.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FwdHeadingExit {
    /// The ball was given to another man.
    OtherAward,
    /// His award dropped under the band unstruck.
    AwardDropped,
    AwardShot,
    AwardGlance,
    /// Nothing awarded, and the ball beyond the state's reach.
    TooFar,
    TooLow,
    /// Out of headed range, or a shot gate.
    NoShot,
    CornerShot,
    CornerGlance,
    DuelLost,
    DuelShot,
    DuelMissed,
    Dispatcher,
    Other,
}

impl FwdHeadingExit {
    pub const COUNT: usize = 14;
    pub const LABELS: [&'static str; Self::COUNT] = [
        "the ball given to another man",
        "his award dropped",
        "award: header at goal",
        "award: glance",
        "ball too far (no award)",
        "ball too low (no award)",
        "out of range / shot gate",
        "corner: header at goal",
        "corner: glance",
        "own duel lost",
        "own duel: header at goal",
        "own duel: missed",
        "the dispatcher",
        "other",
    ];
}

/// A shot or a pass a forward's state asks for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FwdRequest {
    Snapshot,
    PointBlank,
    Prio05,
    /// `Running` proposes `Passing` after the helper held under pressure.
    PassAfterHold,
    /// `Running`'s `should_pass`.
    PassShouldPass,
    /// `Running`'s stall release.
    PassAfterStall,
    /// Every other proposal of `Passing` from `Running`.
    PassOther,
    /// `Passing` plays the pass.
    PassingPass,
    /// `Passing` hands him back without one.
    PassingBounce,
    PatientPossession,
    Cutback,
    OneTwo,
    HoldUpLayoff,
    FlankRelease,
}

impl FwdRequest {
    pub const COUNT: usize = 14;
    pub const LABELS: [&'static str; Self::COUNT] = [
        "shot FWD_SNAPSHOT_PRESSED",
        "shot FWD_RUN_POINT_BLANK",
        "shot FWD_RUN_PRIO05_CLEAR",
        "Running -> Passing, held under pressure",
        "Running -> Passing, should_pass",
        "Running -> Passing, stall release",
        "Running -> Passing, other branches",
        "Passing: the pass",
        "Passing: the bounce",
        "pass FWD_PATIENT_POSSESSION",
        "pass FWD_RUNNING_CUTBACK_TO_RUNNER",
        "pass FWD_RUNNING_ONE_TWO_RETURN",
        "pass FWD_RUNNING_HOLD_UP_LAYOFF",
        "pass FWD_FLANK_RELEASE",
    ];

    fn is_shot(self) -> bool {
        (self as usize) < 3
    }

    /// A proposal of `Passing`, followed until a pass is played or it lapses.
    fn is_pass_proposal(self) -> bool {
        matches!(
            self,
            FwdRequest::PassAfterHold
                | FwdRequest::PassShouldPass
                | FwdRequest::PassAfterStall
                | FwdRequest::PassOther
        )
    }
}

/// What a shot site's helper answered.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FwdVerdict {
    Approved,
    HelperPass,
    HelperHold,
}

/// Why a request did not end in the shot or the pass it asked for.
pub struct FwdRefusal;

impl FwdRefusal {
    pub const COUNT: usize = 6;
    pub const LABELS: [&'static str; Self::COUNT] = [
        "the helper said pass",
        "the helper said hold",
        "asked again before it was played",
        "never left the state that asked",
        "Passing bounced him",
        "lapsed",
    ];
    const HELPER_PASS: usize = 0;
    const HELPER_HOLD: usize = 1;
    const ASKED_AGAIN: usize = 2;
    const HELD: usize = 3;
    const BOUNCED: usize = 4;
    const LAPSED: usize = 5;
}

/// The four limits on a forward's time on the ball in `Running`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FwdLimit {
    /// The first-time snapshot's window.
    Snapshot,
    /// The point-blank hold's patience.
    PointBlankPatience,
    /// The box hold's patience.
    BoxHoldPatience,
    /// The stall release.
    Stall,
}

impl FwdLimit {
    pub const COUNT: usize = 4;
    pub const LABELS: [&'static str; Self::COUNT] = [
        "snapshot window (0.16 s)",
        "point-blank patience (3 s)",
        "box hold patience (6 s)",
        "stall release (2.4 s)",
    ];
}

/// The press doors the assignment census follows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PressDoor {
    /// `Pressing` gives him up to `Returning`.
    CalledOff,
    /// `Returning` sends him to a carrier inside its band.
    ReturningPress,
}

impl PressDoor {
    pub const COUNT: usize = 2;
    pub const LABELS: [&'static str; Self::COUNT] = [
        "Pressing -> Returning (called off)",
        "Returning -> Pressing (carrier near)",
    ];
}

/// The situations the door census puts to every forward state.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FwdSituation {
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
    /// Beyond the opposition's last line with his side not on the ball.
    BeyondLine,
}

impl FwdSituation {
    pub const COUNT: usize = 7;
    pub const LABELS: [&'static str; Self::COUNT] = [
        "licensed carrier in reach",
        "unlicensed carrier near",
        "loose ball he is given",
        "high ball he has won",
        "his side lost it",
        "his side won it",
        "beyond their last line",
    ];
    const ALL: [FwdSituation; Self::COUNT] = [
        FwdSituation::LicensedCarrier,
        FwdSituation::UnlicensedCarrier,
        FwdSituation::LooseBall,
        FwdSituation::HighBall,
        FwdSituation::BallLost,
        FwdSituation::BallWon,
        FwdSituation::BeyondLine,
    ];
    /// 12.5 m.
    const NEAR: f32 = 100.0;
    const AFTER_FLIP_AI_TICKS: u32 = 10;

    /// Whether the state he is in, or the one his handler chose, acts on the
    /// situation. For an unlicensed carrier, acting is the press he should not
    /// make.
    fn answered_by(self, state: PlayerState) -> bool {
        let PlayerState::Forward(s) = state else {
            return matches!(self, FwdSituation::BeyondLine)
                && matches!(state, PlayerState::Recovering);
        };
        let family = s.family();
        match self {
            FwdSituation::LicensedCarrier | FwdSituation::UnlicensedCarrier => {
                matches!(family, ForwardFamily::Close | ForwardFamily::Duel)
            }
            FwdSituation::LooseBall => {
                matches!(family, ForwardFamily::Chase | ForwardFamily::Intercept)
            }
            FwdSituation::HighBall => family == ForwardFamily::Aerial,
            FwdSituation::BallLost => family != ForwardFamily::Job,
            FwdSituation::BallWon => !matches!(
                family,
                ForwardFamily::Close | ForwardFamily::Duel | ForwardFamily::Recover
            ),
            FwdSituation::BeyondLine => family == ForwardFamily::Recover,
        }
    }
}

/// A forward's phase on one tick.
pub struct FwdPhase;

impl FwdPhase {
    pub const COUNT: usize = 12;
    pub const LABELS: [&'static str; Self::COUNT] = [
        "relaxed",
        "alert",
        "closing",
        "in a duel",
        "getting back",
        "run in behind",
        "box place",
        "short outlet",
        "touchline",
        "on the ball",
        "Running without the ball",
        "other",
    ];
    const RELAXED: usize = 0;
    const ALERT: usize = 1;

    fn of(state: PlayerState, has_ball: bool) -> usize {
        if has_ball {
            return 9;
        }
        match state {
            PlayerState::Forward(ForwardState::Walking) => Self::RELAXED,
            PlayerState::Forward(ForwardState::Running) => 10,
            PlayerState::Forward(ForwardState::RunningInBehind) => 5,
            PlayerState::Forward(ForwardState::CrossReceiving) => 6,
            PlayerState::Forward(ForwardState::CreatingSpace) => 7,
            PlayerState::Forward(ForwardState::HoldingWidth) => 8,
            PlayerState::Forward(s) => match s.family() {
                ForwardFamily::Shape => Self::ALERT,
                ForwardFamily::Close => 2,
                ForwardFamily::Duel => 3,
                ForwardFamily::Recover => 4,
                ForwardFamily::Start
                | ForwardFamily::Job
                | ForwardFamily::Chase
                | ForwardFamily::Intercept
                | ForwardFamily::Aerial
                | ForwardFamily::Carry
                | ForwardFamily::Play => 11,
            },
            PlayerState::Recovering => 4,
            _ => 11,
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
struct RunSpell {
    place: usize,
    named: bool,
    held_ticks: u64,
    held_speed_cmps: u64,
    played: bool,
    offsides: u16,
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
    limits: [bool; FwdLimit::COUNT],
}

#[derive(Clone, Copy, Debug)]
struct OpenRequest {
    request: FwdRequest,
    /// The proposal of `Passing` this pass carries out.
    parent: Option<FwdRequest>,
    tick: u64,
    /// He has been in the state the request sent him to.
    arrived: bool,
    bounced: bool,
}

#[derive(Clone, Copy, Debug)]
struct ForwardWatch {
    id: u32,
    place: Option<usize>,
    trail: [Vector3<f32>; ForwardCensus::TRAIL],
    trail_len: usize,
    trail_at: usize,
    last_tick: u64,
    last_pos: Vector3<f32>,
    seen_state: Option<PlayerState>,
    seen_in_state_time: u64,
    phase: Option<PhaseTrack>,
    sprint_ticks: u32,
    spell: Option<DuelSpell>,
    run: Option<RunSpell>,
    run_door: Option<(RunDoor, u64)>,
    run_end: Option<(RunEnd, u64)>,
    /// When his last run ended, and his offsides then: a flag inside
    /// `FLAG_TICKS` of it is that run's.
    run_ended: Option<(u64, u16)>,
    run_held: Option<u64>,
    heading_exit: Option<(FwdHeadingExit, u64)>,
    chase: Option<ChaseEntry>,
    control: Option<bool>,
    since_flip: u32,
    reception: Option<ReceptionTrack>,
    request: Option<OpenRequest>,
}

impl ForwardWatch {
    fn new(id: u32) -> Self {
        ForwardWatch {
            id,
            place: None,
            trail: [Vector3::zeros(); ForwardCensus::TRAIL],
            trail_len: 0,
            trail_at: 0,
            last_tick: 0,
            last_pos: Vector3::zeros(),
            seen_state: None,
            seen_in_state_time: 0,
            phase: None,
            sprint_ticks: 0,
            spell: None,
            run: None,
            run_door: None,
            run_end: None,
            run_ended: None,
            run_held: None,
            heading_exit: None,
            chase: None,
            control: None,
            since_flip: 0,
            reception: None,
            request: None,
        }
    }

    /// Whether this sample follows a gap: dead time runs no tick body, and the
    /// restart that ends it relocates him.
    fn push(&mut self, at: Vector3<f32>, tick: u64) -> bool {
        let resumed = self.last_tick + 1 != tick;
        if resumed {
            self.trail_len = 0;
            self.trail_at = 0;
        }
        self.last_tick = tick;
        self.trail[self.trail_at] = at;
        self.trail_at = (self.trail_at + 1) % ForwardCensus::TRAIL;
        self.trail_len = (self.trail_len + 1).min(ForwardCensus::TRAIL);
        resumed
    }

    /// Ground speed over the trail, in metres a second.
    fn speed(&self) -> f32 {
        if self.trail_len < 2 {
            return 0.0;
        }
        let newest = (self.trail_at + ForwardCensus::TRAIL - 1) % ForwardCensus::TRAIL;
        let oldest = if self.trail_len < ForwardCensus::TRAIL {
            0
        } else {
            self.trail_at
        };
        let (a, b) = (self.trail[newest], self.trail[oldest]);
        (a.x - b.x).hypot(a.y - b.y) * ForwardCensus::METRES_PER_UNIT
            / ((self.trail_len - 1) as f32 * ForwardCensus::SECONDS_PER_TICK)
    }

    /// A marker left by his state on this tick or the one before.
    fn fresh<T: Copy>(marker: Option<(T, u64)>, tick: u64) -> Option<T> {
        marker
            .filter(|(_, at)| tick.saturating_sub(*at) <= 2)
            .map(|(m, _)| m)
    }
}

/// Everything the census has counted, for the harness.
#[derive(Clone, Debug)]
pub struct ForwardCensusSnapshot {
    pub motion_ticks: [[[u64; 2]; BallHeld::COUNT]; FrontPlace::COUNT],
    pub motion_path_mm: [[[u64; 2]; BallHeld::COUNT]; FrontPlace::COUNT],
    pub motion_bands: [[[[u64; DefenderCensus::BANDS]; 2]; BallHeld::COUNT]; FrontPlace::COUNT],
    pub sprints: [[[u64; 2]; BallHeld::COUNT]; FrontPlace::COUNT],
    pub state_ticks: [u64; SLOTS + 1],
    pub state_ball_ticks: [u64; SLOTS + 1],
    pub state_held_ticks: [[u64; HELD + 1]; SLOTS + 1],
    pub state_speed_cmps: [u64; SLOTS + 1],
    pub place_state_ticks: [[u64; SLOTS + 1]; FrontPlace::COUNT],
    /// Out of possession, by `[named presser][elected][called off]`.
    pub out_ticks: [u64; 8],
    pub out_state: [[u64; SLOTS]; 2],
    pub carrier_ticks: u64,
    pub crowded_ticks: u64,
    pub press_doors: [[u64; 2]; PressDoor::COUNT],
    pub job_ticks: [u64; FwdJob::COUNT],
    pub job_state: [[u64; SLOTS + 1]; FwdJob::COUNT],
    pub rib_ticks: u64,
    pub rib_unnamed_ticks: u64,
    pub runner_named_ticks: u64,
    pub runner_in_rib_ticks: u64,
    pub striker_possession_ticks: u64,
    pub striker_box_ticks: [u64; 2],
    pub side_possession_ticks: u64,
    pub both_in_rib_ticks: u64,
    pub runs: [u64; FrontPlace::COUNT],
    pub runs_named: [u64; FrontPlace::COUNT],
    pub run_doors: [u64; RunDoor::COUNT],
    pub run_ends: [u64; RunEnd::COUNT],
    pub run_ticks: u64,
    pub run_held_ticks: u64,
    pub run_held_speed_cmps: u64,
    pub run_played: u64,
    pub run_flagged: u64,
    pub run_line_gap: [u64; LineGap::COUNT],
    pub spells: [u64; FrontPlace::COUNT],
    pub spell_closing_ticks: [u64; FrontPlace::COUNT],
    pub spell_containing_ticks: [u64; FrontPlace::COUNT],
    pub spell_ends: [[u64; SpellEnd::COUNT]; FrontPlace::COUNT],
    pub tackling_entries: u64,
    pub challenges: [u64; 2],
    pub challenge_outcomes: [[u64; 3]; 2],
    pub challenge_speed_cmps: u64,
    pub challenge_goal_side: u64,
    pub tackle_takes: [u64; FwdTackleTake::COUNT],
    pub headers: [u64; FwdHeaderRule::COUNT],
    pub header_state: [[u64; SLOTS + 1]; FwdHeaderRule::COUNT],
    pub header_leap: [u64; FwdHeaderRule::COUNT],
    pub header_booked: [[u64; HeaderBooking::COUNT]; FwdHeaderRule::COUNT],
    pub heading_entries: u64,
    pub heading_exits: [u64; FwdHeadingExit::COUNT],
    pub requests: [u64; FwdRequest::COUNT],
    pub approved: [u64; FwdRequest::COUNT],
    pub taken: [u64; FwdRequest::COUNT],
    pub refused: [[u64; FwdRefusal::COUNT]; FwdRequest::COUNT],
    pub doors: [[[u64; 2]; SLOTS]; FwdSituation::COUNT],
    pub phase_ticks: [u64; FwdPhase::COUNT],
    pub relaxed_alert_changes: u64,
    pub relaxed_alert_reversed: u64,
    pub chase_entries: [u64; SLOTS],
    pub chase_reversed: [[u64; ChaseYield::COUNT]; SLOTS],
    pub receptions: [u64; SLOTS + 1],
    pub reception_running_age: [u64; 3],
    pub reception_pressed: u64,
    pub releases: [u64; 2],
    pub release_ticks: [u64; 2],
    pub limits: [u64; FwdLimit::COUNT],
    pub release_limits: [u64; FwdLimit::COUNT],
}

const ZERO: AtomicU64 = AtomicU64::new(0);
const SLOTS: usize = ForwardState::SLOTS;
const PLACES: usize = FrontPlace::COUNT;
const HELD: usize = BallHeld::COUNT;
const BANDS: usize = DefenderCensus::BANDS;
const REQUESTS_N: usize = FwdRequest::COUNT;

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
static PLACE_STATE_TICKS: [[AtomicU64; SLOTS + 1]; PLACES] = [const { [ZERO; SLOTS + 1] }; PLACES];
static OUT_TICKS: [AtomicU64; 8] = [ZERO; 8];
static OUT_STATE: [[AtomicU64; SLOTS]; 2] = [const { [ZERO; SLOTS] }; 2];
static CARRIER_TICKS: AtomicU64 = ZERO;
static CROWDED_TICKS: AtomicU64 = ZERO;
static PRESS_DOORS: [[AtomicU64; 2]; PressDoor::COUNT] = [const { [ZERO; 2] }; PressDoor::COUNT];
static JOB_TICKS: [AtomicU64; FwdJob::COUNT] = [ZERO; FwdJob::COUNT];
static JOB_STATE: [[AtomicU64; SLOTS + 1]; FwdJob::COUNT] =
    [const { [ZERO; SLOTS + 1] }; FwdJob::COUNT];
static RIB_TICKS: AtomicU64 = ZERO;
static RIB_UNNAMED_TICKS: AtomicU64 = ZERO;
static RUNNER_NAMED_TICKS: AtomicU64 = ZERO;
static RUNNER_IN_RIB_TICKS: AtomicU64 = ZERO;
static STRIKER_POSSESSION_TICKS: AtomicU64 = ZERO;
static STRIKER_BOX_TICKS: [AtomicU64; 2] = [ZERO; 2];
static SIDE_POSSESSION_TICKS: AtomicU64 = ZERO;
static BOTH_IN_RIB_TICKS: AtomicU64 = ZERO;
static RUNS: [AtomicU64; PLACES] = [ZERO; PLACES];
static RUNS_NAMED: [AtomicU64; PLACES] = [ZERO; PLACES];
static RUN_DOORS: [AtomicU64; RunDoor::COUNT] = [ZERO; RunDoor::COUNT];
static RUN_ENDS: [AtomicU64; RunEnd::COUNT] = [ZERO; RunEnd::COUNT];
static RUN_TICKS: AtomicU64 = ZERO;
static RUN_HELD_TICKS: AtomicU64 = ZERO;
static RUN_HELD_SPEED_CMPS: AtomicU64 = ZERO;
static RUN_PLAYED: AtomicU64 = ZERO;
static RUN_FLAGGED: AtomicU64 = ZERO;
static RUN_LINE_GAP: [AtomicU64; LineGap::COUNT] = [ZERO; LineGap::COUNT];
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
static TACKLE_TAKES: [AtomicU64; FwdTackleTake::COUNT] = [ZERO; FwdTackleTake::COUNT];
static HEADERS: [AtomicU64; FwdHeaderRule::COUNT] = [ZERO; FwdHeaderRule::COUNT];
static HEADER_STATE: [[AtomicU64; SLOTS + 1]; FwdHeaderRule::COUNT] =
    [const { [ZERO; SLOTS + 1] }; FwdHeaderRule::COUNT];
static HEADER_LEAP: [AtomicU64; FwdHeaderRule::COUNT] = [ZERO; FwdHeaderRule::COUNT];
static HEADER_BOOKED: [[AtomicU64; HeaderBooking::COUNT]; FwdHeaderRule::COUNT] =
    [const { [ZERO; HeaderBooking::COUNT] }; FwdHeaderRule::COUNT];
static HEADING_ENTRIES: AtomicU64 = ZERO;
static HEADING_EXITS: [AtomicU64; FwdHeadingExit::COUNT] = [ZERO; FwdHeadingExit::COUNT];
static REQUESTS: [AtomicU64; REQUESTS_N] = [ZERO; REQUESTS_N];
static APPROVED: [AtomicU64; REQUESTS_N] = [ZERO; REQUESTS_N];
static TAKEN: [AtomicU64; REQUESTS_N] = [ZERO; REQUESTS_N];
static REFUSED: [[AtomicU64; FwdRefusal::COUNT]; REQUESTS_N] =
    [const { [ZERO; FwdRefusal::COUNT] }; REQUESTS_N];
static DOORS: [[[AtomicU64; 2]; SLOTS]; FwdSituation::COUNT] =
    [const { [const { [ZERO; 2] }; SLOTS] }; FwdSituation::COUNT];
static PHASE_TICKS: [AtomicU64; FwdPhase::COUNT] = [ZERO; FwdPhase::COUNT];
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
static LIMITS: [AtomicU64; FwdLimit::COUNT] = [ZERO; FwdLimit::COUNT];
static RELEASE_LIMITS: [AtomicU64; FwdLimit::COUNT] = [ZERO; FwdLimit::COUNT];

/// The open episodes of one match.
#[derive(Debug, Default)]
pub struct ForwardCensus {
    watches: Vec<ForwardWatch>,
    possession: TeamPossession,
    owner_last_tick: Option<u32>,
    /// The last high ball a contest gave each forward, and by which rule.
    awards: Vec<(u32, FwdHeaderRule, u64)>,
    live_ticks: u64,
    slow_live_ticks: u64,
}

impl ForwardCensus {
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
    /// `Running`'s age at a reception, in AI ticks: 0.4 s and 2.4 s.
    const RUNNING_AGE_EDGES: [u64; 2] = [20, 120];
    /// How long a request is followed before it lapses: a second.
    const REQUEST_TICKS: u64 = 100;
    /// An award older than this is not the one he is heading (3 s).
    const AWARD_TICKS: u64 = 300;
    /// A flag this soon after a run ended (3 s) is that run's.
    const FLAG_TICKS: u64 = 300;

    fn bump(counter: &AtomicU64) {
        counter.fetch_add(1, Ordering::Relaxed);
    }

    fn is_forward(player: &MatchPlayer) -> bool {
        player.tactical_position.current_position.position_group()
            == PlayerFieldPositionGroup::Forward
    }

    fn state_slot(state: PlayerState) -> usize {
        match state {
            PlayerState::Forward(s) => s as usize,
            _ => SLOTS,
        }
    }

    pub fn slot_name(slot: usize) -> String {
        ForwardState::from_slot(slot)
            .map_or_else(|| "not a forward state".to_string(), |s| s.to_string())
    }

    fn is_engaging(state: PlayerState) -> bool {
        matches!(
            state,
            PlayerState::Forward(s)
                if matches!(s.family(), ForwardFamily::Close | ForwardFamily::Duel)
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
        let mut census = ctx.context.forward_census.borrow_mut();
        if let Some(spell) = census.watch(ctx.player.id).spell.as_mut() {
            spell.last = Some(outcome);
        }
    }

    /// A ball he came away with from `Tackling`, and the rule that gave it.
    pub fn note_tackle_take(take: FwdTackleTake) {
        Self::bump(&TACKLE_TAKES[take as usize]);
    }

    /// A contest has given `player_id` a high ball.
    pub fn note_award(&mut self, player_id: u32, rule: FwdHeaderRule, tick: u64) {
        self.awards.retain(|(id, _, _)| *id != player_id);
        self.awards.push((player_id, rule, tick));
    }

    /// The rule behind the award he is heading.
    pub fn award_rule(ctx: &StateProcessingContext) -> FwdHeaderRule {
        let tick = ctx.context.current_tick();
        ctx.context
            .forward_census
            .borrow()
            .awards
            .iter()
            .find(|(id, _, at)| {
                *id == ctx.player.id && tick.saturating_sub(*at) <= Self::AWARD_TICKS
            })
            .map_or(FwdHeaderRule::CrossContest, |(_, rule, _)| *rule)
    }

    /// One header a forward played, noted by the rule that decided it.
    pub fn note_header(
        rule: HeaderRule,
        player: &MatchPlayer,
        leapt: bool,
        booking: HeaderBooking,
    ) {
        if !Self::is_forward(player) {
            return;
        }
        Self::note_forward_header(FwdHeaderRule::of(rule), player, leapt, booking);
    }

    /// A header `Heading` itself played.
    pub fn note_heading_header(
        ctx: &StateProcessingContext,
        rule: FwdHeaderRule,
        booking: HeaderBooking,
    ) {
        Self::note_forward_header(rule, ctx.player, false, booking);
    }

    fn note_forward_header(
        rule: FwdHeaderRule,
        player: &MatchPlayer,
        leapt: bool,
        booking: HeaderBooking,
    ) {
        let r = rule as usize;
        Self::bump(&HEADERS[r]);
        Self::bump(&HEADER_STATE[r][Self::state_slot(player.state)]);
        if leapt || player.is_airborne() {
            Self::bump(&HEADER_LEAP[r]);
        }
        Self::bump(&HEADER_BOOKED[r][booking as usize]);
    }

    /// The exit `Heading` chose. Counted when he is seen to have left it.
    pub fn note_heading_exit(ctx: &StateProcessingContext, exit: FwdHeadingExit) {
        let tick = ctx.context.current_tick();
        ctx.context
            .forward_census
            .borrow_mut()
            .watch(ctx.player.id)
            .heading_exit = Some((exit, tick));
    }

    /// A door into a run in behind proposed it.
    pub fn note_run_door(ctx: &StateProcessingContext, door: RunDoor) {
        let tick = ctx.context.current_tick();
        ctx.context
            .forward_census
            .borrow_mut()
            .watch(ctx.player.id)
            .run_door = Some((door, tick));
    }

    /// A rule of `RunningInBehind` ended the run.
    pub fn note_run_end(ctx: &StateProcessingContext, end: RunEnd) {
        let tick = ctx.context.current_tick();
        ctx.context
            .forward_census
            .borrow_mut()
            .watch(ctx.player.id)
            .run_end = Some((end, tick));
    }

    /// The runner is being held on the line this AI tick.
    pub fn note_run_held(ctx: &StateProcessingContext) {
        let tick = ctx.context.current_tick();
        ctx.context
            .forward_census
            .borrow_mut()
            .watch(ctx.player.id)
            .run_held = Some(tick);
    }

    /// A press door `ForwardCensus` follows was proposed.
    pub fn note_press_door(ctx: &StateProcessingContext, door: PressDoor) {
        let licensed = TackleEngagement::may_engage_carrier(ctx);
        Self::bump(&PRESS_DOORS[door as usize][usize::from(licensed)]);
    }

    /// One of the four limits on his time on the ball acted.
    pub fn note_limit(ctx: &StateProcessingContext, limit: FwdLimit) {
        Self::bump(&LIMITS[limit as usize]);
        let mut census = ctx.context.forward_census.borrow_mut();
        if let Some(reception) = census.watch(ctx.player.id).reception.as_mut() {
            reception.limits[limit as usize] = true;
        }
    }

    /// A pass site that plays the ball or turns him back without one.
    pub fn note_pass_site(ctx: &StateProcessingContext, request: FwdRequest) {
        Self::bump(&REQUESTS[request as usize]);
        if request == FwdRequest::PassingBounce {
            let mut census = ctx.context.forward_census.borrow_mut();
            if let Some(open) = census.watch(ctx.player.id).request.as_mut() {
                open.bounced = true;
            }
            return;
        }
        Self::open_request(ctx, request);
    }

    /// A shot site asked its question; the helper answered `verdict`.
    pub fn note_shot_site(ctx: &StateProcessingContext, request: FwdRequest, verdict: FwdVerdict) {
        let r = request as usize;
        Self::bump(&REQUESTS[r]);
        match verdict {
            FwdVerdict::Approved => {
                Self::bump(&APPROVED[r]);
                Self::open_request(ctx, request);
            }
            FwdVerdict::HelperPass => Self::bump(&REFUSED[r][FwdRefusal::HELPER_PASS]),
            FwdVerdict::HelperHold => Self::bump(&REFUSED[r][FwdRefusal::HELPER_HOLD]),
        }
    }

    fn open_request(ctx: &StateProcessingContext, request: FwdRequest) {
        let tick = ctx.context.current_tick();
        let mut census = ctx.context.forward_census.borrow_mut();
        let watch = census.watch(ctx.player.id);
        let mut parent = None;
        if let Some(open) = watch.request.take() {
            if request == FwdRequest::PassingPass && open.request.is_pass_proposal() {
                parent = Some(open.request);
            } else {
                Self::refuse(open, FwdRefusal::ASKED_AGAIN);
            }
        }
        watch.request = Some(OpenRequest {
            request,
            parent,
            tick,
            arrived: false,
            bounced: false,
        });
    }

    /// A shot he struck has been played.
    pub fn note_shot_struck(&mut self, shooter_id: u32) {
        let Some(watch) = self.watches.iter_mut().find(|w| w.id == shooter_id) else {
            return;
        };
        if let Some(open) = watch.request.filter(|o| o.request.is_shot()) {
            Self::take(open);
            watch.request = None;
        }
    }

    /// A pass he struck has been played.
    pub fn note_pass_struck(&mut self, passer_id: u32) {
        let Some(watch) = self.watches.iter_mut().find(|w| w.id == passer_id) else {
            return;
        };
        if let Some(open) = watch.request.filter(|o| !o.request.is_shot()) {
            Self::take(open);
            watch.request = None;
        }
    }

    fn take(open: OpenRequest) {
        Self::bump(&TAKEN[open.request as usize]);
        if let Some(parent) = open.parent {
            Self::bump(&TAKEN[parent as usize]);
        }
    }

    fn refuse(open: OpenRequest, why: usize) {
        Self::bump(&REFUSED[open.request as usize][why]);
        if let Some(parent) = open.parent {
            Self::bump(&REFUSED[parent as usize][why]);
        }
    }

    /// One forward's AI tick, after his state's handler has decided.
    /// `before` is his state before the dispatcher's overrides, `redirect`
    /// what they did.
    pub fn note_dispatch(
        ctx: &StateProcessingContext,
        before: PlayerState,
        redirect: DispatchRedirect,
        result: &StateProcessingResult,
    ) {
        let PlayerState::Forward(current) = ctx.player.state else {
            return;
        };
        let decided = result.state.unwrap_or(ctx.player.state);
        let slot = current as usize;
        let control = ctx.team().is_control_ball();
        let tick = ctx.context.current_tick();

        let since_flip = {
            let mut census = ctx.context.forward_census.borrow_mut();
            let watch = census.watch(ctx.player.id);
            if redirect != DispatchRedirect::None {
                if before == PlayerState::Forward(ForwardState::Heading) {
                    watch.heading_exit = Some((FwdHeadingExit::Dispatcher, tick));
                }
                if before == PlayerState::Forward(ForwardState::RunningInBehind) {
                    watch.run_end = Some((RunEnd::Dispatcher, tick));
                }
            }
            if watch.control != Some(control) {
                watch.since_flip = if watch.control.is_some() { 0 } else { u32::MAX };
                watch.control = Some(control);
            } else {
                watch.since_flip = watch.since_flip.saturating_add(1);
            }
            watch.since_flip
        };

        for situation in FwdSituation::ALL {
            if Self::situation_holds(ctx, situation, control, since_flip) {
                let cell = &DOORS[situation as usize][slot];
                Self::bump(&cell[0]);
                if situation.answered_by(decided) || situation.answered_by(ctx.player.state) {
                    Self::bump(&cell[1]);
                }
            }
        }

        if control {
            let plan = ctx.context.attack_plan_for_team(ctx.player.team_id);
            let staged = BoxMovement::stage(ctx) > 0.0;
            let job = FwdJob::of(plan, ctx.player.id, staged);
            Self::bump(&JOB_TICKS[job]);
            Self::bump(&JOB_STATE[job][slot]);
            let in_run = current == ForwardState::RunningInBehind;
            if in_run {
                Self::bump(&RIB_TICKS);
                if plan.far_runner != Some(ctx.player.id) {
                    Self::bump(&RIB_UNNAMED_TICKS);
                }
            }
            if plan.far_runner == Some(ctx.player.id) {
                Self::bump(&RUNNER_NAMED_TICKS);
                if in_run {
                    Self::bump(&RUNNER_IN_RIB_TICKS);
                }
            }
        } else {
            let presser = TackleEngagement::is_nominated_presser(ctx);
            let elected = ctx.team().is_best_player_to_chase_ball();
            let licensed = ForwardRead::is_man_for_carrier(ctx);
            Self::bump(
                &OUT_TICKS
                    [usize::from(presser) * 4 + usize::from(elected) * 2 + usize::from(licensed)],
            );
            Self::bump(&OUT_STATE[usize::from(presser)][slot]);
        }

        let mut census = ctx.context.forward_census.borrow_mut();
        let watch = census.watch(ctx.player.id);
        if control
            && watch.place == Some(FrontPlace::CENTRAL)
            && ctx.ball().owner_id() != Some(ctx.player.id)
        {
            Self::bump(&STRIKER_POSSESSION_TICKS);
            let plan = ctx.context.attack_plan_for_team(ctx.player.team_id);
            if plan.slot_of(ctx.player.id).is_some() {
                let staged = BoxMovement::stage(ctx) > 0.0;
                Self::bump(&STRIKER_BOX_TICKS[usize::from(!staged)]);
            }
        }
        Self::follow_chase(watch, before, current, redirect, result);
    }

    /// A chase a state's own door started: confirmed when he is in `TakeBall`
    /// at his next AI tick, reversed when the dispatcher yields him within
    /// ten.
    fn follow_chase(
        watch: &mut ForwardWatch,
        before: PlayerState,
        current: ForwardState,
        redirect: DispatchRedirect,
        result: &StateProcessingResult,
    ) {
        let in_chase = before == PlayerState::Forward(ForwardState::TakeBall);
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
        let door_entry = current != ForwardState::TakeBall
            && redirect == DispatchRedirect::None
            && result.state == Some(PlayerState::Forward(ForwardState::TakeBall));
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
        situation: FwdSituation,
        control: bool,
        since_flip: u32,
    ) -> bool {
        match situation {
            FwdSituation::LicensedCarrier | FwdSituation::UnlicensedCarrier => {
                let licensed = situation == FwdSituation::LicensedCarrier;
                let reach = if licensed {
                    TackleEngagement::COMMIT
                } else {
                    FwdSituation::NEAR
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
            FwdSituation::LooseBall => {
                !ctx.tick_context.ball.is_owned
                    && ctx.player.side.is_some_and(|side| {
                        ctx.tick_context.chase.cost_of(ctx.player.id).is_some()
                            && ctx.tick_context.chase.is_designated(side, ctx.player.id)
                    })
            }
            FwdSituation::HighBall => {
                ctx.tick_context.ball.aerial_contest_winner == Some(ctx.player.id)
            }
            FwdSituation::BallLost => !control && since_flip < FwdSituation::AFTER_FLIP_AI_TICKS,
            FwdSituation::BallWon => {
                control
                    && since_flip < FwdSituation::AFTER_FLIP_AI_TICKS
                    && ctx.ball().owner_id() != Some(ctx.player.id)
            }
            FwdSituation::BeyondLine => ctx.player().defensive().is_stranded_offside(),
        }
    }

    // ── Every tick ───────────────────────────────────────────────────

    fn watch(&mut self, id: u32) -> &mut ForwardWatch {
        match self.watches.iter().position(|w| w.id == id) {
            Some(at) => &mut self.watches[at],
            None => {
                self.watches.push(ForwardWatch::new(id));
                self.watches.last_mut().expect("just pushed")
            }
        }
    }

    /// One sample a tick, after everybody has moved and every event has
    /// landed. `attacks` are each team's attack plan, `areas` the left and the
    /// right penalty area.
    pub fn sample(&mut self, field: &MatchField, attacks: [(u32, AttackPlan); 2], tick: u64) {
        let dead = field.ball.awaiting_restart.is_some();
        let owner = field.ball.current_owner;
        self.possession.update(field);
        let holder = self.possession.holder();
        let carrier = owner.and_then(|id| field.players.iter().find(|p| p.id == id));
        let received = owner.filter(|id| self.owner_last_tick != Some(*id));
        self.owner_last_tick = owner;

        self.follow_carrier(field, carrier);
        self.follow_runs_together(field, holder, dead);

        let (mut live_ticks, mut slow_live_ticks) = (0, 0);
        for player in field
            .players
            .iter()
            .filter(|p| !p.off_pitch && Self::is_forward(p))
        {
            let has_ball = owner == Some(player.id);
            let held = BallHeld::of(holder, player.team_id);
            let carrier = carrier.filter(|c| c.team_id != player.team_id);
            let plan = attacks
                .iter()
                .find(|(team, _)| *team == player.team_id)
                .map(|(_, plan)| plan);
            let place = match self.watch(player.id).place {
                Some(place) => place,
                None => {
                    let place = FrontPlace::of(field, player);
                    self.watch(player.id).place = Some(place);
                    place
                }
            };
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
            Self::bump(&PLACE_STATE_TICKS[place][slot]);
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

            if received == Some(player.id) {
                Self::open_reception(watch, field, player, tick);
            } else if !has_ball && let Some(reception) = watch.reception.take() {
                let pressed = usize::from(reception.pressed);
                Self::bump(&RELEASES[pressed]);
                RELEASE_TICKS[pressed].fetch_add(tick - reception.start, Ordering::Relaxed);
                for (i, fired) in reception.limits.iter().enumerate() {
                    if *fired {
                        Self::bump(&RELEASE_LIMITS[i]);
                    }
                }
            }

            let entered = watch
                .seen_state
                .is_some_and(|s| s.compact_id() != state.compact_id());
            let left = watch.seen_state.filter(|_| entered);
            if entered && state == PlayerState::Forward(ForwardState::Tackling) {
                Self::bump(&TACKLING_ENTRIES);
            }
            if entered && state == PlayerState::Forward(ForwardState::Heading) {
                Self::bump(&HEADING_ENTRIES);
            }
            if left == Some(PlayerState::Forward(ForwardState::Heading)) {
                let exit =
                    ForwardWatch::fresh(watch.heading_exit, tick).unwrap_or(FwdHeadingExit::Other);
                Self::bump(&HEADING_EXITS[exit as usize]);
                watch.heading_exit = None;
            }
            Self::follow_request(watch, state, tick);
            Self::follow_run(
                watch, field, player, plan, holder, place, speed, entered, tick,
            );
            watch.seen_state = Some(state);
            watch.seen_in_state_time = player.in_state_time;

            Self::follow_phase(watch, FwdPhase::of(state, has_ball), tick);
            Self::follow_spell(watch, place, player, carrier, owner);
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
        watch: &mut ForwardWatch,
        field: &MatchField,
        player: &MatchPlayer,
        tick: u64,
    ) {
        let state = watch.seen_state.unwrap_or(player.state);
        Self::bump(&RECEPTIONS[Self::state_slot(state)]);
        if state == PlayerState::Forward(ForwardState::Running) {
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
            limits: [false; FwdLimit::COUNT],
        });
    }

    /// An open request: whether he reached the state it asked for, and
    /// whether it lapsed without the shot or the pass.
    fn follow_request(watch: &mut ForwardWatch, state: PlayerState, tick: u64) {
        let Some(mut open) = watch.request else {
            return;
        };
        let arrived = match open.request {
            r if r.is_shot() => matches!(
                state,
                PlayerState::Forward(ForwardState::Shooting | ForwardState::Finishing)
            ),
            r if r.is_pass_proposal() => state == PlayerState::Forward(ForwardState::Passing),
            _ => true,
        };
        open.arrived |= arrived;
        if tick.saturating_sub(open.tick) > Self::REQUEST_TICKS {
            let why = if open.bounced {
                FwdRefusal::BOUNCED
            } else if !open.arrived {
                FwdRefusal::HELD
            } else {
                FwdRefusal::LAPSED
            };
            Self::refuse(open, why);
            watch.request = None;
        } else {
            watch.request = Some(open);
        }
    }

    /// A run in behind from the tick he is seen in it to the tick he is seen
    /// out of it.
    #[allow(clippy::too_many_arguments)]
    fn follow_run(
        watch: &mut ForwardWatch,
        field: &MatchField,
        player: &MatchPlayer,
        plan: Option<&AttackPlan>,
        holder: Option<u32>,
        place: usize,
        speed: f32,
        entered: bool,
        tick: u64,
    ) {
        let in_run = player.state == PlayerState::Forward(ForwardState::RunningInBehind);
        if let Some((ended, offsides)) = watch.run_ended {
            if player.statistics.offsides > offsides {
                Self::bump(&RUN_FLAGGED);
                watch.run_ended = None;
            } else if tick.saturating_sub(ended) > Self::FLAG_TICKS {
                watch.run_ended = None;
            }
        }
        if let Some(mut run) = watch.run {
            if in_run && !entered {
                Self::bump(&RUN_TICKS);
                if ForwardWatch::fresh(watch.run_held.map(|t| ((), t)), tick).is_some() {
                    run.held_ticks += 1;
                    run.held_speed_cmps += (speed * 100.0) as u64;
                }
                if field.ball.current_owner.is_none()
                    && field.ball.pass_target_player_id == Some(player.id)
                {
                    run.played = true;
                }
                watch.run = Some(run);
                return;
            }
            let end = if field.ball.current_owner == Some(player.id) {
                RunEnd::GotBall
            } else if player.statistics.offsides > run.offsides {
                RunEnd::Flagged
            } else if let Some(end) = ForwardWatch::fresh(watch.run_end, tick) {
                if end == RunEnd::Dispatcher && run.played {
                    RunEnd::Collecting
                } else {
                    end
                }
            } else if holder != Some(player.team_id) {
                RunEnd::BallLost
            } else if plan.is_some_and(|plan| plan.far_runner != Some(player.id)) && run.named {
                RunEnd::Read
            } else {
                RunEnd::Other
            };
            Self::bump(&RUNS[run.place]);
            if run.named {
                Self::bump(&RUNS_NAMED[run.place]);
            }
            Self::bump(&RUN_ENDS[end as usize]);
            RUN_HELD_TICKS.fetch_add(run.held_ticks, Ordering::Relaxed);
            RUN_HELD_SPEED_CMPS.fetch_add(run.held_speed_cmps, Ordering::Relaxed);
            if run.played {
                Self::bump(&RUN_PLAYED);
            }
            if player.statistics.offsides > run.offsides {
                Self::bump(&RUN_FLAGGED);
            } else {
                watch.run_ended = Some((tick, player.statistics.offsides));
            }
            Self::bump(&RUN_LINE_GAP[LineGap::of(Self::beyond_line(field, player))]);
            watch.run = None;
            watch.run_end = None;
        }
        if in_run {
            let door = ForwardWatch::fresh(watch.run_door, tick).unwrap_or(RunDoor::Other);
            Self::bump(&RUN_DOORS[door as usize]);
            watch.run_door = None;
            watch.run = Some(RunSpell {
                place,
                named: plan.is_some_and(|plan| plan.far_runner == Some(player.id)),
                held_ticks: 0,
                held_speed_cmps: 0,
                played: false,
                offsides: player.statistics.offsides,
            });
        }
    }

    /// How far beyond the referee's line he stands, in units (negative is
    /// inside it): the second-last opponent, or the halfway line when that is
    /// further forward.
    fn beyond_line(field: &MatchField, player: &MatchPlayer) -> Option<f32> {
        let side = player.side?;
        let line = OffsideLine::second_last(
            field
                .players
                .iter()
                .filter(|p| p.team_id != player.team_id && !p.off_pitch)
                .map(|p| p.position.x),
            side,
        )?;
        let halfway = field.size.width as f32 / 2.0;
        let line = match side {
            PlayerSide::Left => line.max(halfway),
            PlayerSide::Right => line.min(halfway),
        };
        Some((player.position.x - line) * side.forward_dir_x())
    }

    fn follow_phase(watch: &mut ForwardWatch, phase: usize, tick: u64) {
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
                    (FwdPhase::RELAXED, FwdPhase::ALERT) | (FwdPhase::ALERT, FwdPhase::RELAXED)
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

    /// A spell runs while he is closing or containing a carrier, and ends on
    /// a challenge's outcome or on what took the carrier from him.
    fn follow_spell(
        watch: &mut ForwardWatch,
        place: usize,
        player: &MatchPlayer,
        carrier: Option<&MatchPlayer>,
        owner: Option<u32>,
    ) {
        let engaging = Self::is_engaging(player.state);
        let closing = player.state == PlayerState::Forward(ForwardState::Pressing);
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

    /// Two or more forwards closing or containing the carrier, of the ticks a
    /// carrier runs at a side with two forwards or more on the pitch.
    fn follow_carrier(&mut self, field: &MatchField, carrier: Option<&MatchPlayer>) {
        let Some(carrier) = carrier else {
            return;
        };
        let defending = field
            .players
            .iter()
            .filter(|p| p.team_id != carrier.team_id && !p.off_pitch && Self::is_forward(p));
        let (mut forwards, mut engaged) = (0, 0);
        for p in defending {
            forwards += 1;
            if Self::is_engaging(p.state) {
                engaged += 1;
            }
        }
        if forwards < 2 {
            return;
        }
        Self::bump(&CARRIER_TICKS);
        if engaged >= 2 {
            Self::bump(&CROWDED_TICKS);
        }
    }

    /// Of the side holding the ball with two forwards or more on the pitch,
    /// the ticks two or more of them are making a run in behind at once.
    fn follow_runs_together(&mut self, field: &MatchField, holder: Option<u32>, dead: bool) {
        let Some(team) = holder.filter(|_| !dead) else {
            return;
        };
        let (mut forwards, mut running) = (0, 0);
        for p in field
            .players
            .iter()
            .filter(|p| p.team_id == team && !p.off_pitch && Self::is_forward(p))
        {
            forwards += 1;
            if p.state == PlayerState::Forward(ForwardState::RunningInBehind) {
                running += 1;
            }
        }
        if forwards < 2 {
            return;
        }
        Self::bump(&SIDE_POSSESSION_TICKS);
        if running >= 2 {
            Self::bump(&BOTH_IN_RIB_TICKS);
        }
    }

    /// Full time: the forwards' live ticks, and those at or below a walk, go
    /// on the match tally, which is what the realism bands read.
    pub fn finish(&mut self, tally: &mut MatchTally) {
        tally.forward_motion.live_ticks += self.live_ticks;
        tally.forward_motion.slow_live_ticks += self.slow_live_ticks;
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

    pub fn snapshot() -> ForwardCensusSnapshot {
        let load = |c: &AtomicU64| c.load(Ordering::Relaxed);
        ForwardCensusSnapshot {
            motion_ticks: Self::read3(&MOTION_TICKS),
            motion_path_mm: Self::read3(&MOTION_PATH_MM),
            motion_bands: std::array::from_fn(|p| Self::read3(&MOTION_BANDS[p])),
            sprints: Self::read3(&SPRINTS),
            state_ticks: Self::read(&STATE_TICKS),
            state_ball_ticks: Self::read(&STATE_BALL_TICKS),
            state_held_ticks: Self::read2(&STATE_HELD_TICKS),
            state_speed_cmps: Self::read(&STATE_SPEED_CMPS),
            place_state_ticks: Self::read2(&PLACE_STATE_TICKS),
            out_ticks: Self::read(&OUT_TICKS),
            out_state: Self::read2(&OUT_STATE),
            carrier_ticks: load(&CARRIER_TICKS),
            crowded_ticks: load(&CROWDED_TICKS),
            press_doors: Self::read2(&PRESS_DOORS),
            job_ticks: Self::read(&JOB_TICKS),
            job_state: Self::read2(&JOB_STATE),
            rib_ticks: load(&RIB_TICKS),
            rib_unnamed_ticks: load(&RIB_UNNAMED_TICKS),
            runner_named_ticks: load(&RUNNER_NAMED_TICKS),
            runner_in_rib_ticks: load(&RUNNER_IN_RIB_TICKS),
            striker_possession_ticks: load(&STRIKER_POSSESSION_TICKS),
            striker_box_ticks: Self::read(&STRIKER_BOX_TICKS),
            side_possession_ticks: load(&SIDE_POSSESSION_TICKS),
            both_in_rib_ticks: load(&BOTH_IN_RIB_TICKS),
            runs: Self::read(&RUNS),
            runs_named: Self::read(&RUNS_NAMED),
            run_doors: Self::read(&RUN_DOORS),
            run_ends: Self::read(&RUN_ENDS),
            run_ticks: load(&RUN_TICKS),
            run_held_ticks: load(&RUN_HELD_TICKS),
            run_held_speed_cmps: load(&RUN_HELD_SPEED_CMPS),
            run_played: load(&RUN_PLAYED),
            run_flagged: load(&RUN_FLAGGED),
            run_line_gap: Self::read(&RUN_LINE_GAP),
            spells: Self::read(&SPELLS),
            spell_closing_ticks: Self::read(&SPELL_CLOSING_TICKS),
            spell_containing_ticks: Self::read(&SPELL_CONTAINING_TICKS),
            spell_ends: Self::read2(&SPELL_ENDS),
            tackling_entries: load(&TACKLING_ENTRIES),
            challenges: Self::read(&CHALLENGES),
            challenge_outcomes: Self::read2(&CHALLENGE_OUTCOMES),
            challenge_speed_cmps: load(&CHALLENGE_SPEED_CMPS),
            challenge_goal_side: load(&CHALLENGE_GOAL_SIDE),
            tackle_takes: Self::read(&TACKLE_TAKES),
            headers: Self::read(&HEADERS),
            header_state: Self::read2(&HEADER_STATE),
            header_leap: Self::read(&HEADER_LEAP),
            header_booked: Self::read2(&HEADER_BOOKED),
            heading_entries: load(&HEADING_ENTRIES),
            heading_exits: Self::read(&HEADING_EXITS),
            requests: Self::read(&REQUESTS),
            approved: Self::read(&APPROVED),
            taken: Self::read(&TAKEN),
            refused: Self::read2(&REFUSED),
            doors: std::array::from_fn(|s| Self::read2(&DOORS[s])),
            phase_ticks: Self::read(&PHASE_TICKS),
            relaxed_alert_changes: load(&RELAXED_ALERT_CHANGES),
            relaxed_alert_reversed: load(&RELAXED_ALERT_REVERSED),
            chase_entries: Self::read(&CHASE_ENTRIES),
            chase_reversed: Self::read2(&CHASE_REVERSED),
            receptions: Self::read(&RECEPTIONS),
            reception_running_age: Self::read(&RECEPTION_RUNNING_AGE),
            reception_pressed: load(&RECEPTION_PRESSED),
            releases: Self::read(&RELEASES),
            release_ticks: Self::read(&RELEASE_TICKS),
            limits: Self::read(&LIMITS),
            release_limits: Self::read(&RELEASE_LIMITS),
        }
    }

    pub fn reset() {
        let scalars = [
            &CARRIER_TICKS,
            &CROWDED_TICKS,
            &RIB_TICKS,
            &RIB_UNNAMED_TICKS,
            &RUNNER_NAMED_TICKS,
            &RUNNER_IN_RIB_TICKS,
            &STRIKER_POSSESSION_TICKS,
            &SIDE_POSSESSION_TICKS,
            &BOTH_IN_RIB_TICKS,
            &RUN_TICKS,
            &RUN_HELD_TICKS,
            &RUN_HELD_SPEED_CMPS,
            &RUN_PLAYED,
            &RUN_FLAGGED,
            &TACKLING_ENTRIES,
            &CHALLENGE_SPEED_CMPS,
            &CHALLENGE_GOAL_SIDE,
            &HEADING_ENTRIES,
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
            .chain(STATE_HELD_TICKS.iter().flatten())
            .chain(&STATE_SPEED_CMPS)
            .chain(PLACE_STATE_TICKS.iter().flatten())
            .chain(&OUT_TICKS)
            .chain(OUT_STATE.iter().flatten())
            .chain(PRESS_DOORS.iter().flatten())
            .chain(&JOB_TICKS)
            .chain(JOB_STATE.iter().flatten())
            .chain(&STRIKER_BOX_TICKS)
            .chain(&RUNS)
            .chain(&RUNS_NAMED)
            .chain(&RUN_DOORS)
            .chain(&RUN_ENDS)
            .chain(&RUN_LINE_GAP)
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
            .chain(&REQUESTS)
            .chain(&APPROVED)
            .chain(&TAKEN)
            .chain(REFUSED.iter().flatten())
            .chain(DOORS.iter().flatten().flatten())
            .chain(&PHASE_TICKS)
            .chain(&CHASE_ENTRIES)
            .chain(CHASE_REVERSED.iter().flatten())
            .chain(&RECEPTIONS)
            .chain(&RECEPTION_RUNNING_AGE)
            .chain(&RELEASES)
            .chain(&RELEASE_TICKS)
            .chain(&LIMITS)
            .chain(&RELEASE_LIMITS)
        {
            counter.store(0, Ordering::Relaxed);
        }
    }
}
