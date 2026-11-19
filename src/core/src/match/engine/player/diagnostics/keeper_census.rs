//! **The goalkeeper action census** — how each shot and each high ball a
//! keeper faces is decided, what the next shot finds him doing after he has
//! knocked a ball loose, how long he stays down after a dive and how fast he
//! moves while he is down, how fast he is moving when a shot is struck, and
//! how often his readiness phase changes.
//!
//! The open episodes are per match and live on
//! [`MatchContext`](crate::r#match::MatchContext), because matches run in
//! parallel. The totals are process-wide counters the `dev_match` harness
//! prints, summed over every match of a run.

use crate::r#match::goalkeepers::states::state::GoalkeeperState;
use crate::r#match::player::state::PlayerState;
use crate::r#match::{MatchField, MatchPlayer, MatchTally, PlayerSide, StateProcessingContext};
use nalgebra::Vector3;
use std::sync::atomic::{AtomicU64, Ordering};

/// How the ball-side save model settled a shot it rolled for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SaveSettlement {
    Held,
    /// Over the bar or round the post.
    Tipped,
    /// Pushed wide and kept in play.
    Parried,
    Spilled,
    /// A lapse: the save he should have made squirming loose at his feet.
    Fumbled,
    /// A lapse: the shot going on through him.
    Howler,
    /// A touch he could not stop it with: it carries on at reduced pace.
    Deflected,
    Beaten,
}

impl SaveSettlement {
    pub const COUNT: usize = 8;
    pub const LABELS: [&'static str; Self::COUNT] = [
        "held",
        "tipped",
        "parried",
        "spilled",
        "fumbled",
        "howler",
        "deflected",
        "beaten",
    ];
}

/// A place in the engine that rolls for a keeper's claim of a high ball.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ClaimSite {
    Contest,
    Flap,
    Jumping,
    Catching,
    Punching,
    Diving,
}

impl ClaimSite {
    pub const COUNT: usize = 6;
    pub const LABELS: [&'static str; Self::COUNT] = [
        "contest", "flap", "Jumping", "Catching", "Punching", "Diving",
    ];
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ClaimAction {
    Catch,
    Punch,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ClaimEnd {
    Caught,
    Punched,
    Flapped,
    Lost,
}

impl ClaimEnd {
    pub const COUNT: usize = 4;
    pub const LABELS: [&'static str; Self::COUNT] = ["caught", "punched clear", "flapped", "lost"];
}

/// How a shot ended, as the shot-lifecycle census in `Ball` reads it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShotEnd {
    Goal,
    OutOfPlay,
    Keeper,
    Attack,
    Defence,
    Stopped,
    TimedOut,
    /// Another shot was struck before this one ended.
    Superseded,
}

impl ShotEnd {
    pub const COUNT: usize = 8;
    pub const LABELS: [&'static str; Self::COUNT] = [
        "goal",
        "out of play",
        "keeper has it",
        "attackers have it",
        "defenders have it",
        "died",
        "timed out",
        "another shot first",
    ];
}

/// Where a keeper's readiness stands on this tick. Until the phases are
/// states of their own, relaxed is `Walking`, alert is `Standing` and set
/// is `PreparingForSave`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Phase {
    Relaxed,
    Alert,
    Set,
    Acting,
}

impl Phase {
    fn of(state: PlayerState) -> Phase {
        match state {
            PlayerState::Goalkeeper(GoalkeeperState::Walking) => Phase::Relaxed,
            PlayerState::Goalkeeper(GoalkeeperState::Standing) => Phase::Alert,
            PlayerState::Goalkeeper(GoalkeeperState::PreparingForSave) => Phase::Set,
            _ => Phase::Acting,
        }
    }

    fn is_relaxed_alert_pair(a: Phase, b: Phase) -> bool {
        matches!(
            (a, b),
            (Phase::Relaxed, Phase::Alert) | (Phase::Alert, Phase::Relaxed)
        )
    }
}

#[derive(Clone, Copy, Debug)]
struct PhaseTrack {
    phase: Phase,
    since: u64,
    previous: Option<Phase>,
}

#[derive(Clone, Copy, Debug)]
struct DiveTrack {
    airborne: bool,
    landed: bool,
}

#[derive(Clone, Copy, Debug)]
struct DownSpell {
    /// Live ticks he has spent down: the post-goal pause runs no tick body,
    /// so it is not part of any spell.
    ticks: u64,
    with_ball: bool,
    path: f32,
}

#[derive(Clone, Copy, Debug)]
struct ClaimEpisode {
    opened: u64,
    sites: u8,
    rolls: u64,
    last_site: Option<ClaimSite>,
    action: Option<ClaimAction>,
}

#[derive(Clone, Copy, Debug)]
struct KeeperWatch {
    id: u32,
    trail: [Vector3<f32>; KeeperCensus::TRAIL],
    trail_len: usize,
    trail_at: usize,
    last_tick: u64,
    dive: Option<DiveTrack>,
    down: Option<DownSpell>,
    claim: Option<ClaimEpisode>,
    phase: Option<PhaseTrack>,
}

impl KeeperWatch {
    fn new(id: u32) -> Self {
        KeeperWatch {
            id,
            trail: [Vector3::zeros(); KeeperCensus::TRAIL],
            trail_len: 0,
            trail_at: 0,
            last_tick: 0,
            dive: None,
            down: None,
            claim: None,
            phase: None,
        }
    }

    /// Ground speed over the trail, in metres a second.
    fn speed(&self) -> f32 {
        if self.trail_len < 2 {
            return 0.0;
        }
        let newest = (self.trail_at + KeeperCensus::TRAIL - 1) % KeeperCensus::TRAIL;
        let oldest = if self.trail_len < KeeperCensus::TRAIL {
            0
        } else {
            self.trail_at
        };
        let a = self.trail[newest];
        let b = self.trail[oldest];
        let travelled = (a.x - b.x).hypot(a.y - b.y);
        travelled * KeeperCensus::METRES_PER_UNIT
            / ((self.trail_len - 1) as f32 * KeeperCensus::SECONDS_PER_TICK)
    }

    /// Distance covered on the last tick, in field units.
    fn last_step(&self) -> f32 {
        if self.trail_len < 2 {
            return 0.0;
        }
        let newest = (self.trail_at + KeeperCensus::TRAIL - 1) % KeeperCensus::TRAIL;
        let before = (self.trail_at + KeeperCensus::TRAIL - 2) % KeeperCensus::TRAIL;
        let (a, b) = (self.trail[newest], self.trail[before]);
        (a.x - b.x).hypot(a.y - b.y)
    }

    /// The trail starts again after a gap in the samples: the post-goal
    /// pause runs no tick body, and the restart that ends it relocates him.
    /// Whether this sample follows a gap: dead time runs no tick body.
    fn push(&mut self, at: Vector3<f32>, tick: u64) -> bool {
        let resumed = self.last_tick + 1 != tick;
        if resumed {
            self.trail_len = 0;
            self.trail_at = 0;
        }
        self.last_tick = tick;
        self.trail[self.trail_at] = at;
        self.trail_at = (self.trail_at + 1) % KeeperCensus::TRAIL;
        self.trail_len = (self.trail_len + 1).min(KeeperCensus::TRAIL);
        resumed
    }
}

/// Everything the census has counted, for the harness.
#[derive(Clone, Debug)]
pub struct KeeperCensusSnapshot {
    pub physics: [u64; SaveSettlement::COUNT],
    pub beyond_reach: u64,
    pub machine: u64,
    pub both: u64,
    pub claims: u64,
    pub claim_actions: [u64; 3],
    pub claim_ends: [u64; ClaimEnd::COUNT],
    pub claim_site_count: [u64; ClaimSite::COUNT + 1],
    pub claim_sites: [u64; ClaimSite::COUNT],
    pub claim_decided_by: [u64; ClaimSite::COUNT + 1],
    pub claim_rolls: u64,
    pub contest_uncommitted: u64,
    pub rebounds: u64,
    pub rebound_state: [u64; KeeperCensus::SLOTS],
    pub rebound_height_cm: u64,
    pub rebound_speed_cmps: u64,
    pub rebound_ends: [u64; ShotEnd::COUNT],
    pub dives: u64,
    pub dives_landed: u64,
    pub dives_up_airborne: u64,
    pub dives_never_airborne: u64,
    /// Dives and spells a stoppage ran into: a goal or a substitution
    /// break stops the AI, and the restart puts him on his feet.
    pub spells_cut: u64,
    pub redives: u64,
    pub down_spells: [u64; 2],
    pub down_ticks: [u64; 2],
    pub down_path_mm: [u64; 2],
    pub down_time_hist: [[u64; KeeperCensus::TIME_BINS]; 2],
    pub down_speed_hist: [[u64; KeeperCensus::SPEED_BINS]; 2],
    pub strike_shots: [u64; KeeperCensus::SLOTS],
    pub strike_speed_cmps: [u64; KeeperCensus::SLOTS],
    /// How far across he stood from the line the shot left on, at his own
    /// depth, as it was struck.
    pub strike_off_line_cm: [u64; KeeperCensus::SLOTS],
    pub strike_pace: [[u64; 3]; KeeperCensus::SLOTS],
    pub phase_ticks: [u64; KeeperCensus::PHASES],
    pub phase_changes: u64,
    pub phase_reversed: u64,
    pub relaxed_alert_changes: u64,
    pub relaxed_alert_reversed: u64,
}

const ZERO: AtomicU64 = AtomicU64::new(0);
static PHYSICS: [AtomicU64; SaveSettlement::COUNT] = [ZERO; SaveSettlement::COUNT];
static BEYOND_REACH: AtomicU64 = ZERO;
static MACHINE: AtomicU64 = ZERO;
static BOTH: AtomicU64 = ZERO;
static CLAIMS: AtomicU64 = ZERO;
static CLAIM_ACTIONS: [AtomicU64; 3] = [ZERO; 3];
static CLAIM_ENDS: [AtomicU64; ClaimEnd::COUNT] = [ZERO; ClaimEnd::COUNT];
static CLAIM_SITE_COUNT: [AtomicU64; ClaimSite::COUNT + 1] = [ZERO; ClaimSite::COUNT + 1];
static CLAIM_SITES: [AtomicU64; ClaimSite::COUNT] = [ZERO; ClaimSite::COUNT];
static CLAIM_DECIDED_BY: [AtomicU64; ClaimSite::COUNT + 1] = [ZERO; ClaimSite::COUNT + 1];
static CLAIM_ROLLS: AtomicU64 = ZERO;
static CONTEST_UNCOMMITTED: AtomicU64 = ZERO;
static REBOUNDS: AtomicU64 = ZERO;
static REBOUND_STATE: [AtomicU64; KeeperCensus::SLOTS] = [ZERO; KeeperCensus::SLOTS];
static REBOUND_HEIGHT_CM: AtomicU64 = ZERO;
static REBOUND_SPEED_CMPS: AtomicU64 = ZERO;
static REBOUND_ENDS: [AtomicU64; ShotEnd::COUNT] = [ZERO; ShotEnd::COUNT];
static DIVES: AtomicU64 = ZERO;
static DIVES_LANDED: AtomicU64 = ZERO;
static DIVES_UP_AIRBORNE: AtomicU64 = ZERO;
static DIVES_NEVER_AIRBORNE: AtomicU64 = ZERO;
static SPELLS_CUT: AtomicU64 = ZERO;
static REDIVES: AtomicU64 = ZERO;
static DOWN_SPELLS: [AtomicU64; 2] = [ZERO; 2];
static DOWN_TICKS: [AtomicU64; 2] = [ZERO; 2];
static DOWN_PATH_MM: [AtomicU64; 2] = [ZERO; 2];
static DOWN_TIME_HIST: [[AtomicU64; KeeperCensus::TIME_BINS]; 2] =
    [const { [ZERO; KeeperCensus::TIME_BINS] }; 2];
static DOWN_SPEED_HIST: [[AtomicU64; KeeperCensus::SPEED_BINS]; 2] =
    [const { [ZERO; KeeperCensus::SPEED_BINS] }; 2];
static STRIKE_SHOTS: [AtomicU64; KeeperCensus::SLOTS] = [ZERO; KeeperCensus::SLOTS];
static STRIKE_SPEED_CMPS: [AtomicU64; KeeperCensus::SLOTS] = [ZERO; KeeperCensus::SLOTS];
static STRIKE_OFF_LINE_CM: [AtomicU64; KeeperCensus::SLOTS] = [ZERO; KeeperCensus::SLOTS];
static STRIKE_PACE: [[AtomicU64; 3]; KeeperCensus::SLOTS] =
    [const { [ZERO; 3] }; KeeperCensus::SLOTS];
static PHASE_TICKS: [AtomicU64; KeeperCensus::PHASES] = [ZERO; KeeperCensus::PHASES];
static PHASE_CHANGES: AtomicU64 = ZERO;
static PHASE_REVERSED: AtomicU64 = ZERO;
static RELAXED_ALERT_CHANGES: AtomicU64 = ZERO;
static RELAXED_ALERT_REVERSED: AtomicU64 = ZERO;

/// The open episodes of one match.
#[derive(Debug, Default)]
pub struct KeeperCensus {
    keepers: Vec<KeeperWatch>,
    /// By defending side: the tick his keeper last left the ball loose.
    loose_touch: [Option<u64>; 2],
    seen_touch: u64,
    /// The live shot is a rebound: struck within [`Self::REBOUND_TICKS`]
    /// of a loose touch by the keeper it is aimed at.
    rebound_live: bool,
    ground: (u64, u64, u64),
}

impl KeeperCensus {
    /// Every `GoalkeeperState` slot, and one for a keeper standing in a
    /// state that is not a goalkeeper's.
    pub const SLOTS: usize = GoalkeeperState::SLOTS + 1;
    pub const REBOUND_TICKS: u64 = 300;
    pub const TIME_BINS: usize = 40;
    /// 50 ms bins of time down, the last one open.
    pub const TIME_BIN_TICKS: u64 = 5;
    pub const SPEED_BINS: usize = 40;
    /// 0.25 m/s bins of mean speed while down, the last one open.
    pub const SPEED_BIN_MPS: f32 = 0.25;
    /// Strike-posture pace bands, upper edges in m/s: set, adjusting, and
    /// above the slowest set shuffle.
    pub const PACE_EDGES: [f32; 2] = [0.5, 2.4];
    pub const PHASES: usize = 4;
    pub const PHASE_LABELS: [&'static str; Self::PHASES] = ["relaxed", "alert", "set", "acting"];
    /// A claim nothing has settled after 3 s is lost.
    const CLAIM_TICKS: u64 = 300;
    const TRAIL: usize = 6;
    const METRES_PER_UNIT: f32 = 0.125;
    const SECONDS_PER_TICK: f32 = 0.01;

    pub fn slot(state: PlayerState) -> usize {
        match state {
            PlayerState::Goalkeeper(s) => s as usize,
            _ => GoalkeeperState::SLOTS,
        }
    }

    pub fn slot_name(slot: usize) -> String {
        GoalkeeperState::from_slot(slot)
            .map_or_else(|| "not a keeper state".to_string(), |s| s.to_string())
    }

    fn side_index(side: PlayerSide) -> usize {
        match side {
            PlayerSide::Left => 0,
            PlayerSide::Right => 1,
        }
    }

    fn bump(counter: &AtomicU64) {
        counter.fetch_add(1, Ordering::Relaxed);
    }

    // ── Shots ────────────────────────────────────────────────────────

    pub fn note_physics(settled: SaveSettlement) {
        Self::bump(&PHYSICS[settled as usize]);
    }

    /// The ball reached him wider than his reach: no roll, and the shot
    /// stays live.
    pub fn note_beyond_reach() {
        Self::bump(&BEYOND_REACH);
    }

    /// The state machine handed him a live shot at his goal. `after_physics`
    /// is the ball-side model having already rolled for it.
    pub fn note_machine(after_physics: bool) {
        Self::bump(&MACHINE);
        if after_physics {
            Self::bump(&BOTH);
        }
    }

    pub fn note_strike(&mut self, field: &MatchField, defending: PlayerSide, tick: u64) {
        if self.rebound_live {
            self.close_shot(ShotEnd::Superseded);
        }
        let Some(keeper) = field.players.iter().find(|p| {
            p.side == Some(defending)
                && !p.off_pitch
                && p.tactical_position.current_position.is_goalkeeper()
        }) else {
            return;
        };
        let slot = Self::slot(keeper.state);
        let speed = self
            .keepers
            .iter()
            .find(|w| w.id == keeper.id)
            .map_or(0.0, KeeperWatch::speed);
        Self::bump(&STRIKE_SHOTS[slot]);
        STRIKE_SPEED_CMPS[slot].fetch_add((speed * 100.0) as u64, Ordering::Relaxed);
        let (ball, flight) = (field.ball.position, field.ball.velocity);
        if flight.x.abs() > 1e-3 {
            let line_y = ball.y + flight.y * (keeper.position.x - ball.x) / flight.x;
            let off_line_m = (keeper.position.y - line_y).abs() * 0.125;
            STRIKE_OFF_LINE_CM[slot].fetch_add((off_line_m * 100.0) as u64, Ordering::Relaxed);
        }
        let band = Self::PACE_EDGES
            .iter()
            .position(|edge| speed < *edge)
            .unwrap_or(Self::PACE_EDGES.len());
        Self::bump(&STRIKE_PACE[slot][band]);

        let side = Self::side_index(defending);
        if self.loose_touch[side].is_some_and(|at| tick.saturating_sub(at) <= Self::REBOUND_TICKS) {
            self.rebound_live = true;
            Self::bump(&REBOUNDS);
            Self::bump(&REBOUND_STATE[slot]);
            REBOUND_HEIGHT_CM.fetch_add((keeper.height.max(0.0) * 100.0) as u64, Ordering::Relaxed);
            REBOUND_SPEED_CMPS.fetch_add((speed * 100.0) as u64, Ordering::Relaxed);
        }
    }

    pub fn close_shot(&mut self, end: ShotEnd) {
        if std::mem::take(&mut self.rebound_live) {
            Self::bump(&REBOUND_ENDS[end as usize]);
        }
    }

    // ── Claims ───────────────────────────────────────────────────────

    /// Whether this opened one: a claim already open is the same claim.
    pub fn open_claim(&mut self, keeper_id: u32, tick: u64) -> bool {
        let watch = self.watch(keeper_id);
        if watch.claim.is_some() {
            return false;
        }
        watch.claim = Some(ClaimEpisode {
            opened: tick,
            sites: 0,
            rolls: 0,
            last_site: None,
            action: None,
        });
        Self::bump(&CLAIMS);
        true
    }

    /// One roll at `site` for the claim he is making. A leap or a punch is
    /// a claim however he came to it; a catch or a dive is one only inside
    /// a claim already open, because both also gather balls on the floor.
    pub fn claim_roll(&mut self, keeper_id: u32, site: ClaimSite, tick: u64) {
        if matches!(site, ClaimSite::Jumping | ClaimSite::Punching) {
            self.open_claim(keeper_id, tick);
        }
        let claim = self
            .keepers
            .iter_mut()
            .find(|w| w.id == keeper_id)
            .and_then(|w| w.claim.as_mut());
        match claim {
            Some(claim) => {
                claim.sites |= 1 << site as u8;
                claim.rolls += 1;
                claim.last_site = Some(site);
            }
            None if site == ClaimSite::Contest => Self::bump(&CONTEST_UNCOMMITTED),
            None => {}
        }
    }

    /// [`Self::open_claim`], [`Self::claim_roll`], [`Self::claim_action`]
    /// and [`Self::claim_end`] for the keeper a state handler is running.
    pub fn open_claim_in(ctx: &StateProcessingContext) -> bool {
        let tick = ctx.context.current_tick();
        ctx.context
            .keeper_census
            .borrow_mut()
            .open_claim(ctx.player.id, tick)
    }

    pub fn claim_roll_in(ctx: &StateProcessingContext, site: ClaimSite) {
        let tick = ctx.context.current_tick();
        ctx.context
            .keeper_census
            .borrow_mut()
            .claim_roll(ctx.player.id, site, tick);
    }

    pub fn claim_action_in(ctx: &StateProcessingContext, action: ClaimAction) {
        ctx.context
            .keeper_census
            .borrow_mut()
            .claim_action(ctx.player.id, action);
    }

    pub fn claim_end_in(ctx: &StateProcessingContext, end: ClaimEnd) {
        ctx.context
            .keeper_census
            .borrow_mut()
            .claim_end(ctx.player.id, end);
    }

    pub fn claim_action(&mut self, keeper_id: u32, action: ClaimAction) {
        if let Some(claim) = self
            .keepers
            .iter_mut()
            .find(|w| w.id == keeper_id)
            .and_then(|w| w.claim.as_mut())
            && claim.action.is_none()
        {
            claim.action = Some(action);
        }
    }

    pub fn claim_end(&mut self, keeper_id: u32, end: ClaimEnd) {
        if let Some(watch) = self.keepers.iter_mut().find(|w| w.id == keeper_id) {
            Self::close_claim(watch, end);
        }
    }

    fn close_claim(watch: &mut KeeperWatch, end: ClaimEnd) {
        let Some(claim) = watch.claim.take() else {
            return;
        };
        Self::bump(&CLAIM_ENDS[end as usize]);
        Self::bump(
            &CLAIM_ACTIONS[match claim.action {
                Some(ClaimAction::Catch) => 0,
                Some(ClaimAction::Punch) => 1,
                None => 2,
            }],
        );
        Self::bump(&CLAIM_SITE_COUNT[claim.sites.count_ones() as usize]);
        for (site, counter) in CLAIM_SITES.iter().enumerate() {
            if claim.sites & (1 << site) != 0 {
                Self::bump(counter);
            }
        }
        Self::bump(&CLAIM_DECIDED_BY[claim.last_site.map_or(ClaimSite::COUNT, |s| s as usize)]);
        CLAIM_ROLLS.fetch_add(claim.rolls, Ordering::Relaxed);
    }

    // ── Every tick ───────────────────────────────────────────────────

    fn watch(&mut self, id: u32) -> &mut KeeperWatch {
        match self.keepers.iter().position(|w| w.id == id) {
            Some(at) => &mut self.keepers[at],
            None => {
                self.keepers.push(KeeperWatch::new(id));
                self.keepers.last_mut().expect("just pushed")
            }
        }
    }

    pub fn sample(&mut self, field: &MatchField, tick: u64) {
        let ball = &field.ball;
        if ball.last_touch_tick != self.seen_touch {
            self.seen_touch = ball.last_touch_tick;
            if !ball.last_touch_was_controlled
                && let Some(keeper) = ball.last_touch_player_id.and_then(|id| {
                    field.players.iter().find(|p| {
                        p.id == id && p.tactical_position.current_position.is_goalkeeper()
                    })
                })
                && let Some(side) = keeper.side
            {
                self.loose_touch[Self::side_index(side)] = Some(ball.last_touch_tick);
            }
        }

        for keeper in field
            .players
            .iter()
            .filter(|p| !p.off_pitch && p.tactical_position.current_position.is_goalkeeper())
        {
            let owns = ball.current_owner == Some(keeper.id);
            let held = owns && ball.held_in_hands;
            let other_owner = ball.current_owner.is_some_and(|id| id != keeper.id);
            let dead = ball.awaiting_restart.is_some();
            let watch = self.watch(keeper.id);
            // A stoppage stops the AI with him on the floor and the restart
            // stands him up: how long he would have stayed down is not in it.
            if watch.push(keeper.position, tick)
                && (watch.down.take().is_some() | watch.dive.take().is_some())
            {
                Self::bump(&SPELLS_CUT);
            }
            let step = watch.last_step();
            let ended = Self::follow_ground(watch, keeper, owns, step);
            Self::follow_phase(watch, keeper.state, tick);
            Self::follow_claim(watch, keeper.state, held, other_owner || dead, tick);
            if let Some((false, ticks, path)) = ended {
                self.ground.0 += 1;
                self.ground.1 += ticks;
                self.ground.2 += (path * Self::METRES_PER_UNIT * 1000.0) as u64;
            }
        }
    }

    /// The dive, its landing and the time down, which runs from the landing
    /// until he is up out of `GettingUp`. Returns a spell that has just
    /// ended: whether he had the ball, its ticks and its path.
    fn follow_ground(
        watch: &mut KeeperWatch,
        keeper: &MatchPlayer,
        owns: bool,
        step: f32,
    ) -> Option<(bool, u64, f32)> {
        let diving = keeper.state == PlayerState::Goalkeeper(GoalkeeperState::Diving);
        let getting_up = keeper.state == PlayerState::Goalkeeper(GoalkeeperState::GettingUp);
        let airborne = keeper.is_airborne();
        let mut ended = None;
        if diving {
            let dive = watch.dive.get_or_insert_with(|| {
                Self::bump(&DIVES);
                DiveTrack {
                    airborne: false,
                    landed: false,
                }
            });
            if airborne {
                if let Some(spell) = watch.down.take() {
                    Self::bump(&REDIVES);
                    ended = Some(Self::book_spell(spell, owns));
                }
                dive.airborne = true;
            } else if dive.airborne && watch.down.is_none() {
                if !dive.landed {
                    dive.landed = true;
                    Self::bump(&DIVES_LANDED);
                }
                watch.down = Some(DownSpell {
                    ticks: 0,
                    with_ball: owns,
                    path: 0.0,
                });
            } else if let Some(spell) = watch.down.as_mut() {
                spell.ticks += 1;
                spell.path += step;
            }
        } else if getting_up {
            // On the floor whatever the dive was: the physics save hands him
            // a dive without a leap, and that keeper is down all the same.
            if let Some(dive) = watch.dive.take() {
                if !dive.airborne {
                    Self::bump(&DIVES_NEVER_AIRBORNE);
                } else if !dive.landed {
                    Self::bump(&DIVES_LANDED);
                }
            }
            match watch.down.as_mut() {
                Some(spell) => {
                    spell.ticks += 1;
                    spell.path += step;
                }
                None => {
                    watch.down = Some(DownSpell {
                        ticks: 0,
                        with_ball: owns,
                        path: 0.0,
                    })
                }
            }
        } else if let Some(dive) = watch.dive.take() {
            if dive.airborne {
                // Up before he was ever seen lying down — still in the air,
                // or landed and stood on the same tick — is no time down.
                if airborne {
                    Self::bump(&DIVES_UP_AIRBORNE);
                }
                let spell = watch.down.take().unwrap_or(DownSpell {
                    ticks: 0,
                    with_ball: owns,
                    path: 0.0,
                });
                ended = Some(Self::book_spell(spell, owns));
            } else {
                Self::bump(&DIVES_NEVER_AIRBORNE);
            }
        } else if let Some(spell) = watch.down.take() {
            ended = Some(Self::book_spell(spell, owns));
        }
        ended
    }

    fn book_spell(spell: DownSpell, owns_now: bool) -> (bool, u64, f32) {
        let with_ball = spell.with_ball || owns_now;
        let ticks = spell.ticks;
        let kind = usize::from(with_ball);
        Self::bump(&DOWN_SPELLS[kind]);
        DOWN_TICKS[kind].fetch_add(ticks, Ordering::Relaxed);
        DOWN_PATH_MM[kind].fetch_add(
            (spell.path * Self::METRES_PER_UNIT * 1000.0) as u64,
            Ordering::Relaxed,
        );
        let time_bin = ((ticks / Self::TIME_BIN_TICKS) as usize).min(Self::TIME_BINS - 1);
        Self::bump(&DOWN_TIME_HIST[kind][time_bin]);
        let speed = if ticks == 0 {
            0.0
        } else {
            spell.path * Self::METRES_PER_UNIT / (ticks as f32 * Self::SECONDS_PER_TICK)
        };
        let speed_bin = ((speed / Self::SPEED_BIN_MPS) as usize).min(Self::SPEED_BINS - 1);
        Self::bump(&DOWN_SPEED_HIST[kind][speed_bin]);
        (with_ball, ticks, spell.path)
    }

    fn follow_phase(watch: &mut KeeperWatch, state: PlayerState, tick: u64) {
        let phase = Phase::of(state);
        Self::bump(&PHASE_TICKS[phase as usize]);
        match watch.phase {
            None => {
                watch.phase = Some(PhaseTrack {
                    phase,
                    since: tick,
                    previous: None,
                })
            }
            Some(track) if track.phase != phase => {
                Self::bump(&PHASE_CHANGES);
                let reversed = track.previous == Some(phase)
                    && tick.saturating_sub(track.since) <= Self::REVERSAL_TICKS;
                if reversed {
                    Self::bump(&PHASE_REVERSED);
                }
                if Phase::is_relaxed_alert_pair(track.phase, phase) {
                    Self::bump(&RELAXED_ALERT_CHANGES);
                    if reversed {
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

    /// 300 ms: a phase change undone inside this is the two-cycle the
    /// phase hysteresis exists to stop.
    const REVERSAL_TICKS: u64 = 30;

    fn follow_claim(
        watch: &mut KeeperWatch,
        state: PlayerState,
        held: bool,
        gone: bool,
        tick: u64,
    ) {
        let Some(claim) = watch.claim.as_mut() else {
            return;
        };
        if claim.action.is_none() {
            claim.action = match state {
                PlayerState::Goalkeeper(GoalkeeperState::Catching) => Some(ClaimAction::Catch),
                PlayerState::Goalkeeper(GoalkeeperState::Punching) => Some(ClaimAction::Punch),
                _ => None,
            };
        }
        let opened = claim.opened;
        if held {
            Self::close_claim(watch, ClaimEnd::Caught);
        } else if gone || tick.saturating_sub(opened) > Self::CLAIM_TICKS {
            Self::close_claim(watch, ClaimEnd::Lost);
        }
    }

    /// Full time: the spells that ended without the ball go on the match
    /// tally, which is what the realism bands read.
    pub fn finish(&mut self, tally: &mut MatchTally) {
        let (spells, ticks, path_mm) = self.ground;
        tally.keeper_ground.spells += spells;
        tally.keeper_ground.ticks += ticks;
        tally.keeper_ground.path_mm += path_mm;
        for watch in self.keepers.iter_mut() {
            Self::close_claim(watch, ClaimEnd::Lost);
        }
        self.close_shot(ShotEnd::TimedOut);
    }

    // ── Totals ───────────────────────────────────────────────────────

    fn read<const N: usize>(counters: &[AtomicU64; N]) -> [u64; N] {
        std::array::from_fn(|i| counters[i].load(Ordering::Relaxed))
    }

    pub fn snapshot() -> KeeperCensusSnapshot {
        KeeperCensusSnapshot {
            physics: Self::read(&PHYSICS),
            beyond_reach: BEYOND_REACH.load(Ordering::Relaxed),
            machine: MACHINE.load(Ordering::Relaxed),
            both: BOTH.load(Ordering::Relaxed),
            claims: CLAIMS.load(Ordering::Relaxed),
            claim_actions: Self::read(&CLAIM_ACTIONS),
            claim_ends: Self::read(&CLAIM_ENDS),
            claim_site_count: Self::read(&CLAIM_SITE_COUNT),
            claim_sites: Self::read(&CLAIM_SITES),
            claim_decided_by: Self::read(&CLAIM_DECIDED_BY),
            claim_rolls: CLAIM_ROLLS.load(Ordering::Relaxed),
            contest_uncommitted: CONTEST_UNCOMMITTED.load(Ordering::Relaxed),
            rebounds: REBOUNDS.load(Ordering::Relaxed),
            rebound_state: Self::read(&REBOUND_STATE),
            rebound_height_cm: REBOUND_HEIGHT_CM.load(Ordering::Relaxed),
            rebound_speed_cmps: REBOUND_SPEED_CMPS.load(Ordering::Relaxed),
            rebound_ends: Self::read(&REBOUND_ENDS),
            dives: DIVES.load(Ordering::Relaxed),
            dives_landed: DIVES_LANDED.load(Ordering::Relaxed),
            dives_up_airborne: DIVES_UP_AIRBORNE.load(Ordering::Relaxed),
            dives_never_airborne: DIVES_NEVER_AIRBORNE.load(Ordering::Relaxed),
            spells_cut: SPELLS_CUT.load(Ordering::Relaxed),
            redives: REDIVES.load(Ordering::Relaxed),
            down_spells: Self::read(&DOWN_SPELLS),
            down_ticks: Self::read(&DOWN_TICKS),
            down_path_mm: Self::read(&DOWN_PATH_MM),
            down_time_hist: [
                Self::read(&DOWN_TIME_HIST[0]),
                Self::read(&DOWN_TIME_HIST[1]),
            ],
            down_speed_hist: [
                Self::read(&DOWN_SPEED_HIST[0]),
                Self::read(&DOWN_SPEED_HIST[1]),
            ],
            strike_shots: Self::read(&STRIKE_SHOTS),
            strike_speed_cmps: Self::read(&STRIKE_SPEED_CMPS),
            strike_off_line_cm: Self::read(&STRIKE_OFF_LINE_CM),
            strike_pace: std::array::from_fn(|slot| Self::read(&STRIKE_PACE[slot])),
            phase_ticks: Self::read(&PHASE_TICKS),
            phase_changes: PHASE_CHANGES.load(Ordering::Relaxed),
            phase_reversed: PHASE_REVERSED.load(Ordering::Relaxed),
            relaxed_alert_changes: RELAXED_ALERT_CHANGES.load(Ordering::Relaxed),
            relaxed_alert_reversed: RELAXED_ALERT_REVERSED.load(Ordering::Relaxed),
        }
    }

    pub fn reset() {
        let scalars = [
            &BEYOND_REACH,
            &MACHINE,
            &BOTH,
            &CLAIMS,
            &CLAIM_ROLLS,
            &CONTEST_UNCOMMITTED,
            &REBOUNDS,
            &REBOUND_HEIGHT_CM,
            &REBOUND_SPEED_CMPS,
            &DIVES,
            &DIVES_LANDED,
            &DIVES_UP_AIRBORNE,
            &DIVES_NEVER_AIRBORNE,
            &SPELLS_CUT,
            &REDIVES,
            &PHASE_CHANGES,
            &PHASE_REVERSED,
            &RELAXED_ALERT_CHANGES,
            &RELAXED_ALERT_REVERSED,
        ];
        for counter in scalars
            .into_iter()
            .chain(&PHYSICS)
            .chain(&CLAIM_ACTIONS)
            .chain(&CLAIM_ENDS)
            .chain(&CLAIM_SITE_COUNT)
            .chain(&CLAIM_SITES)
            .chain(&CLAIM_DECIDED_BY)
            .chain(&REBOUND_STATE)
            .chain(&REBOUND_ENDS)
            .chain(&DOWN_SPELLS)
            .chain(&DOWN_TICKS)
            .chain(&DOWN_PATH_MM)
            .chain(DOWN_TIME_HIST.iter().flatten())
            .chain(DOWN_SPEED_HIST.iter().flatten())
            .chain(&STRIKE_SHOTS)
            .chain(&STRIKE_SPEED_CMPS)
            .chain(STRIKE_PACE.iter().flatten())
            .chain(&PHASE_TICKS)
        {
            counter.store(0, Ordering::Relaxed);
        }
    }

    /// The `p`-quantile of a histogram, as the bin's upper edge.
    pub fn quantile(hist: &[u64], p: f64, bin_width: f64) -> f64 {
        let total: u64 = hist.iter().sum();
        if total == 0 {
            return 0.0;
        }
        let want = (total as f64 * p).ceil() as u64;
        let mut seen = 0;
        for (bin, n) in hist.iter().enumerate() {
            seen += n;
            if seen >= want {
                return (bin + 1) as f64 * bin_width;
            }
        }
        hist.len() as f64 * bin_width
    }
}
