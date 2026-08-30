//! Waypoint-route census. Dev diagnostic only — the whole module and
//! every call site compile out without `match-logs`.
//!
//! Answers the four questions the route layer cannot answer by reading:
//!
//!   1. **Is it used?** How often `should_follow_waypoints` is asked, how
//!      often it says yes, and which of its exits it takes. Split per
//!      state, because only seven states consult it at all.
//!   2. **Where along the route is the player?** The index histogram plus
//!      the share of takes sitting on the terminus. The route walk is
//!      monotonic and only re-arms at waypoint 0, so a route that runs
//!      out early parks every later take on one fixed point.
//!   3. **Where does the route send him?** Target depth as a fraction of
//!      the pitch measured toward the goal he is attacking, against his
//!      own depth and the depth the team plan wanted.
//!   4. **Does it agree with the shape?** The angle between "go to my
//!      waypoint" and "go to my anchor". Two destinations pulling apart
//!      is the tug-of-war `ShapeDiscipline` then has to arbitrate every
//!      tick.

use crate::PlayerFieldPositionGroup;
use crate::r#match::player::waypoints::WaypointExit;
use crate::r#match::{PlayerSide, StateProcessingContext};
use std::sync::atomic::{AtomicU64, Ordering};

/// Per-state slots, keyed by `PlayerState::compact_id` (max 418).
const SLOTS: usize = 512;
/// Position groups, keyed by `PlayerFieldPositionGroup as u8`.
const GROUPS: usize = 4;
/// Route-index histogram width. Every generated route is 1-4 waypoints
/// long, so 8 is slack rather than a limit.
pub const IDX_BINS: usize = 8;

#[allow(clippy::declare_interior_mutable_const)]
const ZERO: AtomicU64 = AtomicU64::new(0);

static EVALS: [AtomicU64; SLOTS] = [ZERO; SLOTS];
static TAKES: [AtomicU64; SLOTS] = [ZERO; SLOTS];

static G_EVALS: [AtomicU64; GROUPS] = [ZERO; GROUPS];
static G_TAKES: [AtomicU64; GROUPS] = [ZERO; GROUPS];
/// Asks that produced a route target, whether or not it was followed —
/// the denominator for every geometry mean below. With the routes
/// disarmed the geometry is still read, so the "route vs shape" table
/// answers the same question in both arms: where WOULD the route have
/// sent him, against where the plan wanted him.
static G_GEOM: [AtomicU64; GROUPS] = [ZERO; GROUPS];
static G_DISARMED: [AtomicU64; GROUPS] = [ZERO; GROUPS];
static G_SKIP_CARRIER: [AtomicU64; GROUPS] = [ZERO; GROUPS];
static G_SKIP_CHASER: [AtomicU64; GROUPS] = [ZERO; GROUPS];
static G_SKIP_EMPTY: [AtomicU64; GROUPS] = [ZERO; GROUPS];
static G_CROWDED: [AtomicU64; GROUPS] = [ZERO; GROUPS];
static G_TERMINUS: [AtomicU64; GROUPS] = [ZERO; GROUPS];
static G_COMPLETED: [AtomicU64; GROUPS] = [ZERO; GROUPS];
static G_IDX: [AtomicU64; GROUPS * IDX_BINS] = [ZERO; GROUPS * IDX_BINS];

/// Distance sums, in centi-units, so an integer atomic can carry them.
static G_TO_TARGET: [AtomicU64; GROUPS] = [ZERO; GROUPS];
static G_TARGET_TO_ANCHOR: [AtomicU64; GROUPS] = [ZERO; GROUPS];
static G_TO_ANCHOR: [AtomicU64; GROUPS] = [ZERO; GROUPS];
/// Depth sums, in basis points of pitch length toward the attacked goal.
static G_TARGET_DEPTH: [AtomicU64; GROUPS] = [ZERO; GROUPS];
static G_PLAYER_DEPTH: [AtomicU64; GROUPS] = [ZERO; GROUPS];
static G_ANCHOR_DEPTH: [AtomicU64; GROUPS] = [ZERO; GROUPS];
/// Takes whose target sits in the attacking third / the opposing box.
static G_TARGET_FINAL_THIRD: [AtomicU64; GROUPS] = [ZERO; GROUPS];
static G_TARGET_OPP_BOX: [AtomicU64; GROUPS] = [ZERO; GROUPS];
/// Takes where the route and the anchor point the same way (dot > 0),
/// and takes where they point more than 90 degrees apart.
static G_AGREE: [AtomicU64; GROUPS] = [ZERO; GROUPS];
static G_OPPOSED: [AtomicU64; GROUPS] = [ZERO; GROUPS];

/// `WaypointManager::update` bookkeeping.
static MGR_TICKS: AtomicU64 = AtomicU64::new(0);
static MGR_ADVANCES: AtomicU64 = AtomicU64::new(0);
static MGR_ADVANCES_PAST_NEXT: AtomicU64 = AtomicU64::new(0);
static MGR_COMPLETIONS: AtomicU64 = AtomicU64::new(0);
static MGR_REARMS: AtomicU64 = AtomicU64::new(0);

pub struct WaypointCensus;

impl WaypointCensus {
    /// One sample per `should_follow_waypoints` call, i.e. one per AI
    /// tick per player in each of the states that consult it.
    pub fn note(ctx: &StateProcessingContext, exit: WaypointExit) {
        let group = (ctx
            .player
            .tactical_position
            .current_position
            .position_group() as usize)
            .min(GROUPS - 1);
        let slot = ctx.player.state.compact_id() as usize;
        if slot < SLOTS {
            EVALS[slot].fetch_add(1, Ordering::Relaxed);
        }
        G_EVALS[group].fetch_add(1, Ordering::Relaxed);

        match exit {
            WaypointExit::Carrier => {
                G_SKIP_CARRIER[group].fetch_add(1, Ordering::Relaxed);
                return;
            }
            WaypointExit::Chaser => {
                G_SKIP_CHASER[group].fetch_add(1, Ordering::Relaxed);
                return;
            }
            WaypointExit::Disarmed => {
                G_DISARMED[group].fetch_add(1, Ordering::Relaxed);
            }
            WaypointExit::Crowded => {
                G_CROWDED[group].fetch_add(1, Ordering::Relaxed);
            }
            WaypointExit::Default => {}
        }

        let route = ctx.player.get_waypoints_as_vectors();
        if route.is_empty() {
            G_SKIP_EMPTY[group].fetch_add(1, Ordering::Relaxed);
            return;
        }

        let idx = ctx.player.waypoint_manager.current_index.min(route.len() - 1);
        let target = route[idx];
        let pos = ctx.player.position;
        let anchor = ctx.team().my_anchor();
        let width = ctx.context.field_size.width as f32;

        if exit.follows() {
            if slot < SLOTS {
                TAKES[slot].fetch_add(1, Ordering::Relaxed);
            }
            G_TAKES[group].fetch_add(1, Ordering::Relaxed);
        }
        G_GEOM[group].fetch_add(1, Ordering::Relaxed);
        G_IDX[group * IDX_BINS + idx.min(IDX_BINS - 1)].fetch_add(1, Ordering::Relaxed);
        if idx == route.len() - 1 {
            G_TERMINUS[group].fetch_add(1, Ordering::Relaxed);
        }
        if ctx.player.waypoint_manager.path_completed {
            G_COMPLETED[group].fetch_add(1, Ordering::Relaxed);
        }

        let to_target = target - pos;
        let to_anchor = anchor - pos;
        G_TO_TARGET[group].fetch_add((to_target.magnitude() * 100.0) as u64, Ordering::Relaxed);
        G_TO_ANCHOR[group].fetch_add((to_anchor.magnitude() * 100.0) as u64, Ordering::Relaxed);
        G_TARGET_TO_ANCHOR[group].fetch_add(
            ((target - anchor).magnitude() * 100.0) as u64,
            Ordering::Relaxed,
        );

        let target_depth = Self::depth(ctx.player.side, target.x, width);
        G_TARGET_DEPTH[group].fetch_add(target_depth, Ordering::Relaxed);
        G_PLAYER_DEPTH[group].fetch_add(
            Self::depth(ctx.player.side, pos.x, width),
            Ordering::Relaxed,
        );
        G_ANCHOR_DEPTH[group].fetch_add(
            Self::depth(ctx.player.side, anchor.x, width),
            Ordering::Relaxed,
        );
        if target_depth >= 6_667 {
            G_TARGET_FINAL_THIRD[group].fetch_add(1, Ordering::Relaxed);
        }
        if target_depth >= 8_030 {
            G_TARGET_OPP_BOX[group].fetch_add(1, Ordering::Relaxed);
        }

        // Do the two destinations agree about which way to move? Only
        // meaningful when both are a real distance away — a player sat on
        // his anchor has no anchor direction to disagree with.
        if to_target.magnitude() > 4.0 && to_anchor.magnitude() > 4.0 {
            if to_target.normalize().dot(&to_anchor.normalize()) > 0.0 {
                G_AGREE[group].fetch_add(1, Ordering::Relaxed);
            } else {
                G_OPPOSED[group].fetch_add(1, Ordering::Relaxed);
            }
        }
    }

    /// How far up the pitch `x` is, toward the goal this side attacks,
    /// in basis points.
    #[inline]
    fn depth(side: Option<PlayerSide>, x: f32, width: f32) -> u64 {
        let frac = match side {
            Some(PlayerSide::Right) => (width - x) / width,
            _ => x / width,
        };
        (frac.clamp(0.0, 1.0) * 10_000.0) as u64
    }

    /// One sample per `WaypointManager::update`, from inside the manager.
    #[inline]
    pub fn note_manager(advances: u32, past_next: u32, completed: bool, rearmed: bool) {
        MGR_TICKS.fetch_add(1, Ordering::Relaxed);
        if advances > 0 {
            MGR_ADVANCES.fetch_add(advances as u64, Ordering::Relaxed);
        }
        if past_next > 0 {
            MGR_ADVANCES_PAST_NEXT.fetch_add(past_next as u64, Ordering::Relaxed);
        }
        if completed {
            MGR_COMPLETIONS.fetch_add(1, Ordering::Relaxed);
        }
        if rearmed {
            MGR_REARMS.fetch_add(1, Ordering::Relaxed);
        }
    }

    pub fn reset() {
        for i in 0..SLOTS {
            EVALS[i].store(0, Ordering::Relaxed);
            TAKES[i].store(0, Ordering::Relaxed);
        }
        for i in 0..GROUPS {
            G_EVALS[i].store(0, Ordering::Relaxed);
            G_TAKES[i].store(0, Ordering::Relaxed);
            G_GEOM[i].store(0, Ordering::Relaxed);
            G_DISARMED[i].store(0, Ordering::Relaxed);
            G_SKIP_CARRIER[i].store(0, Ordering::Relaxed);
            G_SKIP_CHASER[i].store(0, Ordering::Relaxed);
            G_SKIP_EMPTY[i].store(0, Ordering::Relaxed);
            G_CROWDED[i].store(0, Ordering::Relaxed);
            G_TERMINUS[i].store(0, Ordering::Relaxed);
            G_COMPLETED[i].store(0, Ordering::Relaxed);
            G_TO_TARGET[i].store(0, Ordering::Relaxed);
            G_TARGET_TO_ANCHOR[i].store(0, Ordering::Relaxed);
            G_TO_ANCHOR[i].store(0, Ordering::Relaxed);
            G_TARGET_DEPTH[i].store(0, Ordering::Relaxed);
            G_PLAYER_DEPTH[i].store(0, Ordering::Relaxed);
            G_ANCHOR_DEPTH[i].store(0, Ordering::Relaxed);
            G_TARGET_FINAL_THIRD[i].store(0, Ordering::Relaxed);
            G_TARGET_OPP_BOX[i].store(0, Ordering::Relaxed);
            G_AGREE[i].store(0, Ordering::Relaxed);
            G_OPPOSED[i].store(0, Ordering::Relaxed);
        }
        for i in 0..GROUPS * IDX_BINS {
            G_IDX[i].store(0, Ordering::Relaxed);
        }
        MGR_TICKS.store(0, Ordering::Relaxed);
        MGR_ADVANCES.store(0, Ordering::Relaxed);
        MGR_ADVANCES_PAST_NEXT.store(0, Ordering::Relaxed);
        MGR_COMPLETIONS.store(0, Ordering::Relaxed);
        MGR_REARMS.store(0, Ordering::Relaxed);
    }

    /// Per-state `(evals, takes)`, keyed by `PlayerState::compact_id`.
    pub fn by_state(compact_id: u16) -> (u64, u64) {
        let slot = compact_id as usize;
        if slot >= SLOTS {
            return (0, 0);
        }
        (
            EVALS[slot].load(Ordering::Relaxed),
            TAKES[slot].load(Ordering::Relaxed),
        )
    }

    pub fn by_group(group: PlayerFieldPositionGroup) -> GroupRow {
        let g = (group as usize).min(GROUPS - 1);
        let takes = G_TAKES[g].load(Ordering::Relaxed);
        let geom = G_GEOM[g].load(Ordering::Relaxed);
        let mut idx = [0u64; IDX_BINS];
        for (b, slot) in idx.iter_mut().enumerate() {
            *slot = G_IDX[g * IDX_BINS + b].load(Ordering::Relaxed);
        }
        let per_take = |a: &[AtomicU64; GROUPS], scale: f64| {
            if geom == 0 {
                0.0
            } else {
                a[g].load(Ordering::Relaxed) as f64 / geom as f64 / scale
            }
        };
        GroupRow {
            evals: G_EVALS[g].load(Ordering::Relaxed),
            takes,
            geom,
            disarmed: G_DISARMED[g].load(Ordering::Relaxed),
            skip_carrier: G_SKIP_CARRIER[g].load(Ordering::Relaxed),
            skip_chaser: G_SKIP_CHASER[g].load(Ordering::Relaxed),
            skip_empty: G_SKIP_EMPTY[g].load(Ordering::Relaxed),
            crowded: G_CROWDED[g].load(Ordering::Relaxed),
            terminus: G_TERMINUS[g].load(Ordering::Relaxed),
            completed: G_COMPLETED[g].load(Ordering::Relaxed),
            idx,
            mean_to_target_u: per_take(&G_TO_TARGET, 100.0),
            mean_target_to_anchor_u: per_take(&G_TARGET_TO_ANCHOR, 100.0),
            mean_to_anchor_u: per_take(&G_TO_ANCHOR, 100.0),
            mean_target_depth: per_take(&G_TARGET_DEPTH, 10_000.0),
            mean_player_depth: per_take(&G_PLAYER_DEPTH, 10_000.0),
            mean_anchor_depth: per_take(&G_ANCHOR_DEPTH, 10_000.0),
            target_final_third: G_TARGET_FINAL_THIRD[g].load(Ordering::Relaxed),
            target_opp_box: G_TARGET_OPP_BOX[g].load(Ordering::Relaxed),
            agree: G_AGREE[g].load(Ordering::Relaxed),
            opposed: G_OPPOSED[g].load(Ordering::Relaxed),
        }
    }

    pub fn manager() -> ManagerRow {
        ManagerRow {
            ticks: MGR_TICKS.load(Ordering::Relaxed),
            advances: MGR_ADVANCES.load(Ordering::Relaxed),
            advances_past_next: MGR_ADVANCES_PAST_NEXT.load(Ordering::Relaxed),
            completions: MGR_COMPLETIONS.load(Ordering::Relaxed),
            rearms: MGR_REARMS.load(Ordering::Relaxed),
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct GroupRow {
    pub evals: u64,
    pub takes: u64,
    /// Asks that produced a route target — the denominator for the
    /// geometry means and the index histogram.
    pub geom: u64,
    pub disarmed: u64,
    pub skip_carrier: u64,
    pub skip_chaser: u64,
    pub skip_empty: u64,
    pub crowded: u64,
    pub terminus: u64,
    pub completed: u64,
    pub idx: [u64; IDX_BINS],
    pub mean_to_target_u: f64,
    pub mean_target_to_anchor_u: f64,
    pub mean_to_anchor_u: f64,
    pub mean_target_depth: f64,
    pub mean_player_depth: f64,
    pub mean_anchor_depth: f64,
    pub target_final_third: u64,
    pub target_opp_box: u64,
    pub agree: u64,
    pub opposed: u64,
}

#[derive(Debug, Clone, Default)]
pub struct ManagerRow {
    pub ticks: u64,
    pub advances: u64,
    pub advances_past_next: u64,
    pub completions: u64,
    pub rearms: u64,
}
