//! **The goal-kick census.** Where a goal kick's wait goes, what the keeper
//! does while it waits, whether the other twenty-one are standing still,
//! how it is put back into play and what it leads to.
//!
//! A [`GoalKickTrack`] rides on the ball from the first dead-ball tick of a
//! goal kick until [`GoalKickCensus::FOLLOW_TICKS`] after it is handed to
//! its taker; its totals land in the atomics when it closes.
//!
//! ⚠ **Speeds are displacement, never `MatchPlayer::velocity`.** A man
//! standing in his slot steers back and forth across it, so his velocity
//! reads a jog while he goes nowhere — what the replay records (it drops
//! moves under 0.3 u) and what the viewer draws is the net displacement.
//! Every speed here is the ground covered over [`GoalKickCensus::SAMPLE_TICKS`].

use crate::r#match::engine::ball::ball::{AwaitedRestart, ShotTarget};
use crate::r#match::engine::corner_shape::CornerShape;
use crate::r#match::goalkeepers::states::state::GoalkeeperState;
use crate::r#match::player::state::PlayerState;
use crate::r#match::{MatchPlayer, PlayerSide};
use std::sync::atomic::{AtomicU64, Ordering};

/// How a goal kick left its taker.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GoalKickRelease {
    Short,
    Long,
    Clearance,
    OtherPass,
    Throw,
    Punt,
    OtherFromHands,
    /// Not released by him within the follow window.
    None,
}

impl GoalKickRelease {
    pub const COUNT: usize = 8;
    pub const NAMES: [&'static str; Self::COUNT] = [
        "short pass",
        "long kick",
        "clearance",
        "other pass",
        "THROWN",
        "PUNTED",
        "other from the hands",
        "not released by him",
    ];

    fn index(self) -> usize {
        self as usize
    }

    /// What a pass reason says the keeper meant to do with it.
    pub fn intent_of(reason: &str) -> Self {
        match reason {
            "GK_DISTRIBUTING" => GoalKickRelease::Short,
            "GK_KICKING" => GoalKickRelease::Long,
            "GK_THROWING" => GoalKickRelease::Throw,
            _ => GoalKickRelease::OtherPass,
        }
    }

    fn settled(intent: Option<Self>, from_hands: bool) -> Self {
        match (intent, from_hands) {
            (Some(GoalKickRelease::Throw), _) => GoalKickRelease::Throw,
            (Some(GoalKickRelease::Clearance), true) => GoalKickRelease::Punt,
            (_, true) => GoalKickRelease::OtherFromHands,
            (Some(intent), false) => intent,
            (None, false) => GoalKickRelease::OtherPass,
        }
    }
}

/// Ground covered over the last sample window, in u/tick.
#[derive(Clone, Copy, Debug)]
struct Pace {
    mates_mean: f32,
    mates_fastest: f32,
    fastest_state: u16,
}

/// One goal kick, from its first dead-ball tick to the end of its follow
/// window.
#[derive(Clone, Copy, Debug)]
pub struct GoalKickTrack {
    taker: u32,
    team: u32,
    side: Option<PlayerSide>,
    opened: u64,
    phases: [u64; GoalKickCensus::PHASE_COUNT],
    settled_at: Option<u64>,
    marks: [(u32, f32, f32); GoalKickCensus::MAX_PLAYERS],
    marked: usize,
    pace: Option<Pace>,
    still_mean: Option<f32>,
    still_run: u64,
    still_best: u64,
    quiet_run: [u64; GoalKickCensus::QUIET_COUNT],
    first_quiet: [Option<u64>; GoalKickCensus::QUIET_COUNT],
    handed: Option<u64>,
    setup_ticks: u64,
    teleported: bool,
    intent: Option<GoalKickRelease>,
    release: Option<GoalKickRelease>,
    handled: bool,
    lost_at: Option<u64>,
    shot_at: Option<u64>,
}

/// Everything the census has counted.
#[derive(Clone, Debug)]
pub struct GoalKickSnapshot {
    pub kicks: u64,
    pub phase_ticks: [u64; GoalKickCensus::PHASE_COUNT],
    pub wait_ticks: u64,
    pub on_setup: u64,
    pub at_ceiling: u64,
    pub teleported: u64,
    pub release: [u64; GoalKickRelease::COUNT],
    pub handled: u64,
    pub still_hist: [u64; GoalKickCensus::STILL_BINS],
    pub still_ticks: u64,
    pub quiet_reached: [u64; GoalKickCensus::QUIET_COUNT],
    pub quiet_ticks: [u64; GoalKickCensus::QUIET_COUNT],
    pub lost: u64,
    pub shot: u64,
    pub wait_state: [u64; GoalKickCensus::STATE_SLOTS],
    pub handover_state: [u64; GoalKickCensus::STATE_SLOTS],
    pub at_spot_ticks: u64,
    pub opponent_in_area_ticks: u64,
    pub opponents_in_area: u64,
    pub intruder_depth_hist: [u64; GoalKickCensus::DEPTH_BINS],
    pub intruder_state: Vec<(u16, u64)>,
    pub clear_by_a_stride_ticks: u64,
    pub fast_mate_state: Vec<(u16, u64)>,
    pub mean_speed_hist: [u64; GoalKickCensus::SPEED_BINS],
    pub max_speed_hist: [u64; GoalKickCensus::SPEED_BINS],
}

const ZERO: AtomicU64 = AtomicU64::new(0);
static KICKS: AtomicU64 = ZERO;
static PHASE_TICKS: [AtomicU64; GoalKickCensus::PHASE_COUNT] = [ZERO; GoalKickCensus::PHASE_COUNT];
static WAIT_TICKS: AtomicU64 = ZERO;
static ON_SETUP: AtomicU64 = ZERO;
static AT_CEILING: AtomicU64 = ZERO;
static TELEPORTED: AtomicU64 = ZERO;
static RELEASE: [AtomicU64; GoalKickRelease::COUNT] = [ZERO; GoalKickRelease::COUNT];
static HANDLED: AtomicU64 = ZERO;
static STILL_HIST: [AtomicU64; GoalKickCensus::STILL_BINS] = [ZERO; GoalKickCensus::STILL_BINS];
static STILL_TICKS: AtomicU64 = ZERO;
static QUIET_REACHED: [AtomicU64; GoalKickCensus::QUIET_COUNT] =
    [ZERO; GoalKickCensus::QUIET_COUNT];
static QUIET_TICKS: [AtomicU64; GoalKickCensus::QUIET_COUNT] = [ZERO; GoalKickCensus::QUIET_COUNT];
static LOST: AtomicU64 = ZERO;
static SHOT: AtomicU64 = ZERO;
static WAIT_STATE: [AtomicU64; GoalKickCensus::STATE_SLOTS] = [ZERO; GoalKickCensus::STATE_SLOTS];
static HANDOVER_STATE: [AtomicU64; GoalKickCensus::STATE_SLOTS] =
    [ZERO; GoalKickCensus::STATE_SLOTS];
static AT_SPOT_TICKS: AtomicU64 = ZERO;
static OPPONENT_IN_AREA_TICKS: AtomicU64 = ZERO;
static OPPONENTS_IN_AREA: AtomicU64 = ZERO;
static INTRUDER_DEPTH_HIST: [AtomicU64; GoalKickCensus::DEPTH_BINS] =
    [ZERO; GoalKickCensus::DEPTH_BINS];
static INTRUDER_STATE: [AtomicU64; GoalKickCensus::STATE_IDS] = [ZERO; GoalKickCensus::STATE_IDS];
static CLEAR_BY_A_STRIDE_TICKS: AtomicU64 = ZERO;
static FAST_MATE_STATE: [AtomicU64; GoalKickCensus::STATE_IDS] = [ZERO; GoalKickCensus::STATE_IDS];
static MEAN_SPEED_HIST: [AtomicU64; GoalKickCensus::SPEED_BINS] =
    [ZERO; GoalKickCensus::SPEED_BINS];
static MAX_SPEED_HIST: [AtomicU64; GoalKickCensus::SPEED_BINS] = [ZERO; GoalKickCensus::SPEED_BINS];

pub struct GoalKickCensus;

impl GoalKickCensus {
    pub const PHASE_COUNT: usize = 6;
    pub const PHASES: [&'static str; Self::PHASE_COUNT] =
        ["running out", "fetch", "carry", "routine", "set-up", "held"];
    const RUN_OUT: usize = 0;
    const FETCH: usize = 1;
    const CARRY: usize = 2;
    const ROUTINE: usize = 3;
    const SET_UP: usize = 4;
    const HELD: usize = 5;

    /// How long a goal kick is followed after its hand-over: long enough
    /// for the shot a lost goal kick gives away.
    pub const FOLLOW_TICKS: u64 = 1500;
    /// …and the window a lost ball is charged to it.
    pub const LOST_TICKS: u64 = 1000;
    /// The fixed wait a goal kick had before `goal-kick-stall`, from the
    /// keeper reaching the ball: a hand-over before it was taken on the
    /// state of the pitch.
    pub const CEILING_TICKS: u64 = 2000;

    const MAX_PLAYERS: usize = 32;
    /// The displacement window. 250 ms spans the back-and-forth of a man
    /// holding his slot, so it cancels out.
    pub const SAMPLE_TICKS: u64 = 25;
    /// Still is a 1 s mean speed under 0.5 m/s, the recording census's
    /// definition: an exponential mean over the samples with a 1 s
    /// constant.
    const STILL_SPEED: f32 = 0.04;
    const STILL_RATE: f32 = 0.25;
    /// Half-second bins up to 40 s.
    pub const STILL_BINS: usize = 80;
    pub const STILL_BIN_TICKS: u64 = 50;

    /// Candidate "the side is quiet" tests for the set-up, each held for
    /// [`Self::QUIET_HOLD`] with nobody more than a stride inside the
    /// area: (label, test the fastest
    /// man rather than the mean, bound in u/tick).
    pub const QUIET_COUNT: usize = 5;
    pub const QUIET_BOUNDS: [(&'static str, bool, f32); Self::QUIET_COUNT] = [
        ("fastest < 1.0 m/s", true, 0.08),
        ("fastest < 1.5 m/s", true, 0.12),
        ("fastest < 2.0 m/s", true, 0.16),
        ("mean < 0.5 m/s", false, 0.04),
        ("mean < 1.0 m/s", false, 0.08),
    ];
    const QUIET_HOLD: u64 = 100;

    /// Speed bins for the taker's team-mates while he waits, upper edges
    /// in m/s; the last bin is open.
    pub const SPEED_BINS: usize = 7;
    pub const SPEED_EDGES: [f32; Self::SPEED_BINS - 1] = [0.25, 0.5, 1.0, 1.5, 2.0, 3.0];
    const MPS: f32 = 12.5;

    /// How far inside his area an opponent stands while the goal kick
    /// waits, measured in from the 18-yard line: upper edges in metres.
    pub const DEPTH_BINS: usize = 5;
    pub const DEPTH_EDGES: [f32; Self::DEPTH_BINS - 1] = [1.0, 2.0, 4.0, 8.0];
    const AREA_DEPTH: f32 = 132.0;
    /// A man standing on the 18-yard line is out of the area for the
    /// referee: 8 u = 1 m, one stride.
    const LINE_TOLERANCE: f32 = 8.0;
    /// A team-mate covering more than this is still going somewhere:
    /// 2 m/s.
    const FAST_MATE: f32 = 0.16;
    /// `PlayerState::compact_id` space.
    const STATE_IDS: usize = 500;

    /// `GoalkeeperState` discriminants, and one slot for a taker who is
    /// not in a goalkeeper state.
    pub const STATE_SLOTS: usize = 21;

    fn state_slot(state: PlayerState) -> usize {
        match state {
            PlayerState::Goalkeeper(s) => s as usize,
            _ => Self::STATE_SLOTS - 1,
        }
    }

    pub fn slot_name(slot: usize) -> String {
        GoalkeeperState::ALL
            .iter()
            .find(|s| **s as usize == slot)
            .map_or_else(|| "not a keeper state".to_string(), |s| s.to_string())
    }

    fn speed_bin(speed: f32) -> usize {
        let mps = speed * Self::MPS;
        Self::SPEED_EDGES
            .iter()
            .position(|edge| mps < *edge)
            .unwrap_or(Self::SPEED_BINS - 1)
    }

    /// One dead-ball tick of a goal kick.
    #[allow(clippy::too_many_arguments)]
    pub fn note_wait(
        track: &mut Option<GoalKickTrack>,
        restart: &AwaitedRestart,
        taker: &MatchPlayer,
        players: &[MatchPlayer],
        now: u64,
        routine: u64,
        goal_x: f32,
        field_height: f32,
    ) {
        if track.is_none_or(|t| t.handed.is_some() || t.taker != taker.id) {
            Self::close(track);
            *track = Some(GoalKickTrack {
                taker: taker.id,
                team: taker.team_id,
                side: taker.side,
                opened: now,
                phases: [0; Self::PHASE_COUNT],
                settled_at: None,
                marks: [(0, 0.0, 0.0); Self::MAX_PLAYERS],
                marked: 0,
                pace: None,
                still_mean: None,
                still_run: 0,
                still_best: 0,
                quiet_run: [0; Self::QUIET_COUNT],
                first_quiet: [None; Self::QUIET_COUNT],
                handed: None,
                setup_ticks: 0,
                teleported: false,
                intent: None,
                release: None,
                handled: false,
                lost_at: None,
                shot_at: None,
            });
        }
        let Some(t) = track.as_mut() else {
            return;
        };
        let phase = match restart.settled_tick {
            _ if !restart.settled => Self::RUN_OUT,
            Some(_) if restart.is_held(now) => Self::HELD,
            Some(at) if now.saturating_sub(at) < routine => Self::ROUTINE,
            Some(_) => Self::SET_UP,
            None if restart.carrying => Self::CARRY,
            None => Self::FETCH,
        };
        t.phases[phase] += 1;
        let Some(settled_at) = restart.settled_tick else {
            return;
        };
        t.settled_at = Some(settled_at);

        AT_SPOT_TICKS.fetch_add(1, Ordering::Relaxed);
        WAIT_STATE[Self::state_slot(taker.state)].fetch_add(1, Ordering::Relaxed);
        let mut in_area = 0u64;
        let mut deep = 0u64;
        for p in players.iter().filter(|p| {
            p.team_id != taker.team_id
                && !p.off_pitch
                && CornerShape::is_in_penalty_area(p.position, goal_x, field_height)
        }) {
            in_area += 1;
            let depth = Self::AREA_DEPTH - (p.position.x - goal_x).abs();
            if depth > Self::LINE_TOLERANCE {
                deep += 1;
            }
            let depth_m = depth * 0.125;
            let bin = Self::DEPTH_EDGES
                .iter()
                .position(|edge| depth_m < *edge)
                .unwrap_or(Self::DEPTH_BINS - 1);
            INTRUDER_DEPTH_HIST[bin].fetch_add(1, Ordering::Relaxed);
            INTRUDER_STATE[(p.state.compact_id() as usize).min(Self::STATE_IDS - 1)]
                .fetch_add(1, Ordering::Relaxed);
        }
        if in_area > 0 {
            OPPONENT_IN_AREA_TICKS.fetch_add(1, Ordering::Relaxed);
            OPPONENTS_IN_AREA.fetch_add(in_area, Ordering::Relaxed);
        }
        if deep == 0 {
            CLEAR_BY_A_STRIDE_TICKS.fetch_add(1, Ordering::Relaxed);
        }
        let Some(pace) = t.pace else {
            return;
        };
        MEAN_SPEED_HIST[Self::speed_bin(pace.mates_mean)].fetch_add(1, Ordering::Relaxed);
        MAX_SPEED_HIST[Self::speed_bin(pace.mates_fastest)].fetch_add(1, Ordering::Relaxed);
        if pace.mates_fastest > Self::FAST_MATE {
            FAST_MATE_STATE[(pace.fastest_state as usize).min(Self::STATE_IDS - 1)]
                .fetch_add(1, Ordering::Relaxed);
        }
        for (i, (_, by_fastest, bound)) in Self::QUIET_BOUNDS.iter().enumerate() {
            let read = if *by_fastest {
                pace.mates_fastest
            } else {
                pace.mates_mean
            };
            if deep == 0 && read < *bound {
                t.quiet_run[i] += 1;
                if t.quiet_run[i] >= Self::QUIET_HOLD && t.first_quiet[i].is_none() {
                    t.first_quiet[i] = Some(now.saturating_sub(settled_at));
                }
            } else {
                t.quiet_run[i] = 0;
            }
        }
    }

    /// The restart has been handed to its taker.
    pub fn note_handed(
        track: &mut Option<GoalKickTrack>,
        taker: &MatchPlayer,
        now: u64,
        arrived: bool,
    ) {
        let Some(t) = track
            .as_mut()
            .filter(|t| t.taker == taker.id && t.handed.is_none())
        else {
            return;
        };
        t.handed = Some(now);
        t.teleported = !arrived;
        t.setup_ticks = t.settled_at.map_or(0, |at| {
            now.saturating_sub(at).saturating_sub(t.phases[Self::HELD])
        });
        HANDOVER_STATE[Self::state_slot(taker.state)].fetch_add(1, Ordering::Relaxed);
    }

    /// What the taker asked to do with the ball, ahead of the release that
    /// carries it out.
    pub fn note_intent(track: &mut Option<GoalKickTrack>, player_id: u32, intent: GoalKickRelease) {
        if let Some(t) = track
            .as_mut()
            .filter(|t| t.taker == player_id && t.handed.is_some() && t.release.is_none())
        {
            t.intent = Some(intent);
        }
    }

    pub fn note_release(track: &mut Option<GoalKickTrack>, player_id: u32, from_hands: bool) {
        if let Some(t) = track
            .as_mut()
            .filter(|t| t.taker == player_id && t.handed.is_some() && t.release.is_none())
        {
            t.release = Some(GoalKickRelease::settled(t.intent, from_hands));
        }
    }

    pub fn note_touch(
        track: &mut Option<GoalKickTrack>,
        team_id: u32,
        controlled: bool,
        tick: u64,
    ) {
        if let Some(t) = track.as_mut().filter(|t| {
            t.handed.is_some() && t.lost_at.is_none() && controlled && team_id != t.team
        }) {
            t.lost_at = Some(tick);
        }
    }

    /// Every tick: the pace sample, the still stretch until he releases it,
    /// a hand possession he should never have, the shot it gives away, and
    /// the end of the follow window.
    pub fn follow(
        track: &mut Option<GoalKickTrack>,
        held_in_hands: bool,
        shot: Option<&ShotTarget>,
        players: &[MatchPlayer],
        now: u64,
    ) {
        let Some(t) = track.as_mut() else {
            return;
        };
        if now % Self::SAMPLE_TICKS == 0 {
            Self::sample(t, players);
        }
        let Some(handed) = t.handed else {
            return;
        };
        if t.release.is_none() && held_in_hands {
            t.handled = true;
        }
        if t.shot_at.is_none() && shot.is_some_and(|s| Some(s.defending_side) == t.side) {
            t.shot_at = Some(now);
        }
        if now.saturating_sub(handed) > Self::FOLLOW_TICKS {
            Self::close(track);
        }
    }

    fn sample(t: &mut GoalKickTrack, players: &[MatchPlayer]) {
        let window = Self::SAMPLE_TICKS as f32;
        let (mut mates, mut mates_n, mut fastest, mut others, mut others_n) =
            (0.0, 0usize, 0.0f32, 0.0, 0usize);
        let mut fastest_state = 0u16;
        for p in players.iter().filter(|p| p.id != t.taker && !p.off_pitch) {
            let Some(&(_, x, y)) = t.marks[..t.marked].iter().find(|(id, _, _)| *id == p.id) else {
                continue;
            };
            let pace = (p.position.x - x).hypot(p.position.y - y) / window;
            others += pace;
            others_n += 1;
            if p.team_id == t.team && !p.tactical_position.current_position.is_goalkeeper() {
                mates += pace;
                mates_n += 1;
                if pace > fastest {
                    fastest = pace;
                    fastest_state = p.state.compact_id();
                }
            }
        }
        t.marked = 0;
        for p in players
            .iter()
            .filter(|p| !p.off_pitch)
            .take(Self::MAX_PLAYERS)
        {
            t.marks[t.marked] = (p.id, p.position.x, p.position.y);
            t.marked += 1;
        }
        if mates_n == 0 || others_n == 0 {
            return;
        }
        t.pace = Some(Pace {
            mates_mean: mates / mates_n as f32,
            mates_fastest: fastest,
            fastest_state,
        });
        if t.release.is_some() {
            return;
        }
        let mean = others / others_n as f32;
        let smoothed = t
            .still_mean
            .map_or(mean, |m| m + (mean - m) * Self::STILL_RATE);
        t.still_mean = Some(smoothed);
        if smoothed < Self::STILL_SPEED {
            t.still_run += Self::SAMPLE_TICKS;
            t.still_best = t.still_best.max(t.still_run);
        } else {
            t.still_run = 0;
        }
    }

    fn close(track: &mut Option<GoalKickTrack>) {
        let Some(t) = track.take() else {
            return;
        };
        let Some(handed) = t.handed else {
            return;
        };
        KICKS.fetch_add(1, Ordering::Relaxed);
        for (total, ticks) in PHASE_TICKS.iter().zip(t.phases) {
            total.fetch_add(ticks, Ordering::Relaxed);
        }
        WAIT_TICKS.fetch_add(handed.saturating_sub(t.opened), Ordering::Relaxed);
        if t.teleported {
            TELEPORTED.fetch_add(1, Ordering::Relaxed);
        } else if t.setup_ticks + 10 < Self::CEILING_TICKS {
            ON_SETUP.fetch_add(1, Ordering::Relaxed);
        } else {
            AT_CEILING.fetch_add(1, Ordering::Relaxed);
        }
        RELEASE[t.release.unwrap_or(GoalKickRelease::None).index()].fetch_add(1, Ordering::Relaxed);
        if t.handled {
            HANDLED.fetch_add(1, Ordering::Relaxed);
        }
        let bin = ((t.still_best / Self::STILL_BIN_TICKS) as usize).min(Self::STILL_BINS - 1);
        STILL_HIST[bin].fetch_add(1, Ordering::Relaxed);
        STILL_TICKS.fetch_add(t.still_best, Ordering::Relaxed);
        for (i, first) in t.first_quiet.iter().enumerate() {
            if let Some(ticks) = first.filter(|ticks| *ticks < Self::CEILING_TICKS) {
                QUIET_REACHED[i].fetch_add(1, Ordering::Relaxed);
                QUIET_TICKS[i].fetch_add(ticks, Ordering::Relaxed);
            }
        }
        if t.lost_at
            .is_some_and(|at| at.saturating_sub(handed) <= Self::LOST_TICKS)
        {
            LOST.fetch_add(1, Ordering::Relaxed);
        }
        if t.shot_at.is_some() {
            SHOT.fetch_add(1, Ordering::Relaxed);
        }
    }

    pub fn reset() {
        let all = [
            &KICKS,
            &WAIT_TICKS,
            &ON_SETUP,
            &AT_CEILING,
            &TELEPORTED,
            &HANDLED,
            &STILL_TICKS,
            &LOST,
            &SHOT,
            &AT_SPOT_TICKS,
            &OPPONENT_IN_AREA_TICKS,
            &OPPONENTS_IN_AREA,
        ];
        for counter in all
            .into_iter()
            .chain(&PHASE_TICKS)
            .chain(&RELEASE)
            .chain(&STILL_HIST)
            .chain(&QUIET_REACHED)
            .chain(&QUIET_TICKS)
            .chain(&WAIT_STATE)
            .chain(&HANDOVER_STATE)
            .chain(&MEAN_SPEED_HIST)
            .chain(&MAX_SPEED_HIST)
            .chain(&INTRUDER_DEPTH_HIST)
            .chain(&INTRUDER_STATE)
            .chain(&FAST_MATE_STATE)
            .chain(std::iter::once(&CLEAR_BY_A_STRIDE_TICKS))
        {
            counter.store(0, Ordering::Relaxed);
        }
    }

    fn read<const N: usize>(counters: &[AtomicU64; N]) -> [u64; N] {
        std::array::from_fn(|i| counters[i].load(Ordering::Relaxed))
    }

    pub fn snapshot() -> GoalKickSnapshot {
        GoalKickSnapshot {
            kicks: KICKS.load(Ordering::Relaxed),
            phase_ticks: Self::read(&PHASE_TICKS),
            wait_ticks: WAIT_TICKS.load(Ordering::Relaxed),
            on_setup: ON_SETUP.load(Ordering::Relaxed),
            at_ceiling: AT_CEILING.load(Ordering::Relaxed),
            teleported: TELEPORTED.load(Ordering::Relaxed),
            release: Self::read(&RELEASE),
            handled: HANDLED.load(Ordering::Relaxed),
            still_hist: Self::read(&STILL_HIST),
            still_ticks: STILL_TICKS.load(Ordering::Relaxed),
            quiet_reached: Self::read(&QUIET_REACHED),
            quiet_ticks: Self::read(&QUIET_TICKS),
            lost: LOST.load(Ordering::Relaxed),
            shot: SHOT.load(Ordering::Relaxed),
            wait_state: Self::read(&WAIT_STATE),
            handover_state: Self::read(&HANDOVER_STATE),
            at_spot_ticks: AT_SPOT_TICKS.load(Ordering::Relaxed),
            opponent_in_area_ticks: OPPONENT_IN_AREA_TICKS.load(Ordering::Relaxed),
            opponents_in_area: OPPONENTS_IN_AREA.load(Ordering::Relaxed),
            intruder_depth_hist: Self::read(&INTRUDER_DEPTH_HIST),
            intruder_state: INTRUDER_STATE
                .iter()
                .enumerate()
                .map(|(id, n)| (id as u16, n.load(Ordering::Relaxed)))
                .filter(|(_, n)| *n > 0)
                .collect(),
            clear_by_a_stride_ticks: CLEAR_BY_A_STRIDE_TICKS.load(Ordering::Relaxed),
            fast_mate_state: FAST_MATE_STATE
                .iter()
                .enumerate()
                .map(|(id, n)| (id as u16, n.load(Ordering::Relaxed)))
                .filter(|(_, n)| *n > 0)
                .collect(),
            mean_speed_hist: Self::read(&MEAN_SPEED_HIST),
            max_speed_hist: Self::read(&MAX_SPEED_HIST),
        }
    }
}
