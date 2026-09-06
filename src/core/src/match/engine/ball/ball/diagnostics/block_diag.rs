//! Why shot blocks don't happen. `blocks` reads ~0.01 per defender per
//! match against a real ~0.9, and the counter alone cannot say whether
//! the shot never reaches the check, no defender is ever in the lane, or
//! the roll simply fails. `match-logs` only.

use std::sync::atomic::{AtomicU64, Ordering};

/// `try_block_shot` reached with a live shot in flight.
pub static SHOTS_SEEN: AtomicU64 = AtomicU64::new(0);
/// Rejected because the ball was above blocking height.
pub static TOO_HIGH: AtomicU64 = AtomicU64::new(0);
/// A defender was found inside the lane.
pub static CANDIDATES: AtomicU64 = AtomicU64::new(0);
/// The roll succeeded.
pub static FIRED: AtomicU64 = AtomicU64::new(0);

// ── Per-opponent rejection lanes ────────────────────────────────
//
// `CANDIDATES` alone says "no defender in the lane" without saying
// WHY, and the three possible causes want opposite fixes: defenders
// standing behind the ball is a positioning problem, defenders past
// the lookahead is a window problem, defenders goal-side but wide is
// a corridor-width problem. These split the rejection so the next
// reader doesn't have to re-derive it.
/// Opposition outfielders examined across all shot-ticks.
pub static OPP_SEEN: AtomicU64 = AtomicU64::new(0);
/// Rejected: level with or behind the ball along the shot line.
pub static BEHIND_BALL: AtomicU64 = AtomicU64::new(0);
/// Rejected: goal-side but further than `BLOCK_LOOKAHEAD` ahead.
pub static BEYOND_LOOKAHEAD: AtomicU64 = AtomicU64::new(0);
/// Rejected: inside the lookahead window but wider than the corridor.
pub static OUTSIDE_CORRIDOR: AtomicU64 = AtomicU64::new(0);
/// Sum of perpendicular distances for opponents inside the lookahead
/// window, x100 — divided by `IN_WINDOW` it gives the mean miss
/// distance, which is what says whether the corridor is merely too
/// narrow or the defenders are nowhere near the line.
pub static PERP_SUM_X100: AtomicU64 = AtomicU64::new(0);
/// Opponents inside the lookahead window (the `PERP_SUM_X100` denom).
pub static IN_WINDOW: AtomicU64 = AtomicU64::new(0);

// ── At the moment of the strike ─────────────────────────────────
//
// The per-tick counters above sample the whole flight, which biases
// "behind the ball" upward: a defender the ball has already passed
// counts as behind on every remaining tick. These sample ONCE, when
// the shot is struck, and answer the football question directly —
// was anybody between the shooter and the goal at all?
/// Shots struck with a projected target (one sample each).
pub static SHOTS_STRUCK: AtomicU64 = AtomicU64::new(0);
/// Opposition outfielders goal-side of the ball at the strike,
/// summed over `SHOTS_STRUCK`.
pub static GOALSIDE_AT_STRIKE: AtomicU64 = AtomicU64::new(0);
/// Of those, the ones also within 30u of the ball's line to goal —
/// i.e. actually in a position to get a body in the way.
pub static GOALSIDE_NEAR_LINE: AtomicU64 = AtomicU64::new(0);

/// Distance from the ball to the goal it is aimed at, x100, summed
/// over `SHOTS_STRUCK`. Says where shots are actually taken from.
pub static SHOT_RANGE_X100: AtomicU64 = AtomicU64::new(0);
/// Mean distance of the DEFENDING outfielders from their own goal
/// line at the strike, x100, summed over `SHOTS_STRUCK`. Read against
/// `SHOT_RANGE_X100`: if the defenders sit further out than the ball,
/// the line never dropped; if they sit closer but nobody is in the
/// lane, the line dropped and scattered.
pub static DEF_DEPTH_X100: AtomicU64 = AtomicU64::new(0);

/// Histogram of which `DefenderState` the defending back line is in
/// at the moment a shot is struck, indexed by the enum's discriminant
/// (21 variants). Without this the depth number says the line did not
/// drop but not WHY — and the answer decides whether the fix belongs
/// in a state's steering target or in the state selection above it.
pub static DEF_STATE_AT_STRIKE: [AtomicU64; 21] = [const { AtomicU64::new(0) }; 21];

/// How close the nearest defending outfielder is to the SHOOTER when
/// he strikes it, banded in metres: `<1 | 1-2 | 2-3 | 3-5 | 5-8 | 8+`.
///
/// # Why this band list and not another
///
/// The shot models read pressure through
/// `ShotInputs::pressure_count_5u` / `_10u`, and 1u is 0.125 m — so
/// those are radii of **0.62 m and 1.25 m**, i.e. a defender who is
/// physically touching the shooter. Everything from a stride away
/// outward is priced identically to an empty stadium, in both the
/// accuracy model (`ShotOutcome::pressure_penalty`) and the reported
/// xG (`XgModel::pressure_factor`). Real pressure runs out at 5-8 m,
/// which is most of a defended box.
///
/// The bands therefore straddle the model's own cut-offs: everything
/// in band 0 and part of band 1 is seen by the models, and bands 2-4
/// are the shots taken under real pressure that the engine currently
/// treats as free.
pub static NEAREST_DEF_AT_STRIKE: [AtomicU64; 6] = [const { AtomicU64::new(0) }; 6];

// ── WHERE THE CONTACT ACTUALLY HAPPENS ──────────────────────────
//
// Reported from the viewer, 2026-09-06: *"the ball bounces not off the
// player, but off an invisible object — it often bounces off something
// above the player."*
//
// A block is the only model in the engine that turns a ball in FLIGHT
// at an outfield player, and it was the only one that did it without
// asking where the ball was relative to him. Both channels decide the
// RATE where the read happens and defer the CONTACT until the ball
// reaches the man (`ShotTarget::blocked_by`, `Ball::pass_blocked_by`),
// and that deferral tested `hypot(dx, dy)` and nothing else. The
// height gate sits at the roll, up to eleven metres earlier — so a
// shot rolled for at knee height was deflected wherever the ball had
// climbed to by the time it got to him.
//
// Nothing is drawn there. The replay rig attributes a contact to a man
// only within `Actors::STRIKE_REACH` (1.7 m) and below
// `Actors::OVERHEAD` (2.8 m); above or beyond that the ball simply
// turns in mid-air over an idle defender, which is the report.
//
// Every resolved block is booked here by where the ball was when it
// came off him. Index 0 is the shot block, 1 the pass block — see
// [`BlockDiag::CHANNELS`].
/// Blocks resolved, per channel.
pub static CONTACTS: [AtomicU64; 2] = [const { AtomicU64::new(0) }; 2];
/// …of which arrived through the deferred `blocked_by` commitment
/// rather than firing on the tick the roll was won.
pub static CONTACT_DEFERRED: [AtomicU64; 2] = [const { AtomicU64::new(0) }; 2];
/// Ball height at the contact, metres x100, summed.
pub static CONTACT_HEIGHT_X100: [AtomicU64; 2] = [const { AtomicU64::new(0) }; 2];
/// Distance across the grass from the blocker, metres x100, summed.
pub static CONTACT_GAP_X100: [AtomicU64; 2] = [const { AtomicU64::new(0) }; 2];
/// Contacts above the channel's OWN stated ceiling — the number the
/// roll checked and the contact did not.
pub static CONTACT_OVER_CEILING: [AtomicU64; 2] = [const { AtomicU64::new(0) }; 2];
/// Contacts the replay rig cannot attribute to anybody: further than
/// `Actors::STRIKE_REACH` across the grass, or above `Actors::OVERHEAD`.
/// **This is the reported artefact, counted.**
pub static CONTACT_UNDRAWABLE: [AtomicU64; 2] = [const { AtomicU64::new(0) }; 2];
/// Height bands, shared by both channels:
/// `deck (<0.4) | shin-to-volley (<1.45) | head (<2.2) | jump (<2.8) | OVER`.
pub static CONTACT_HEIGHT_BANDS: [AtomicU64; 5] = [const { AtomicU64::new(0) }; 5];

/// Diagnostic accessors. Grouped on a struct so the module exposes
/// no free functions — the statics stay module-level because Rust
/// has no associated statics.
pub struct BlockDiag;

impl BlockDiag {
    /// Labels for the contact census's two channels, in index order.
    pub const CHANNELS: [&'static str; 2] = ["shot", "pass"];
    /// …and for [`CONTACT_HEIGHT_BANDS`].
    pub const HEIGHT_BANDS: [&'static str; 5] = ["deck", "<1.45m", "<2.2m", "<2.8m", "over 2.8m"];

    /// Book one resolved block by where the ball was when it came off
    /// the blocker.
    ///
    /// `channel` indexes [`Self::CHANNELS`]; `ceiling_m` is the
    /// channel's own stated blocking height, so a contact above it is
    /// one the roll would have refused.
    ///
    /// ⚠ The two distances arrive in the ball's own mixed frame — the
    /// ground axes are game units and the vertical is metres — because
    /// that is how the caller holds them. They are converted here, once,
    /// rather than at each call site.
    pub fn note_contact(channel: usize, height_m: f32, gap_u: f32, ceiling_m: f32, deferred: bool) {
        /// 1u = 0.125 m.
        const M_PER_U: f32 = 0.125;
        /// The rig's own reach and ceiling — see the module note. Copied
        /// rather than shared because the viewer is a different crate
        /// and this is a measurement OF the disagreement.
        const DRAWN_REACH: f32 = 1.7;
        const DRAWN_CEILING: f32 = 2.8;

        let height = height_m;
        let gap = gap_u * M_PER_U;
        let ceiling = ceiling_m;
        let c = channel.min(Self::CHANNELS.len() - 1);
        CONTACTS[c].fetch_add(1, Ordering::Relaxed);
        if deferred {
            CONTACT_DEFERRED[c].fetch_add(1, Ordering::Relaxed);
        }
        CONTACT_HEIGHT_X100[c].fetch_add((height.max(0.0) * 100.0) as u64, Ordering::Relaxed);
        CONTACT_GAP_X100[c].fetch_add((gap.max(0.0) * 100.0) as u64, Ordering::Relaxed);
        if height > ceiling {
            CONTACT_OVER_CEILING[c].fetch_add(1, Ordering::Relaxed);
        }
        if gap > DRAWN_REACH || height > DRAWN_CEILING {
            CONTACT_UNDRAWABLE[c].fetch_add(1, Ordering::Relaxed);
        }
        let band = if height < 0.4 {
            0
        } else if height < 1.45 {
            1
        } else if height < 2.2 {
            2
        } else if height < 2.8 {
            3
        } else {
            4
        };
        CONTACT_HEIGHT_BANDS[band].fetch_add(1, Ordering::Relaxed);
    }

    /// `(channel, contacts, deferred share, mean height m, mean gap m,
    ///   over-ceiling share, undrawable share)` — one row per channel
    /// that saw anything.
    pub fn contact_snapshot() -> Vec<(&'static str, u64, f32, f32, f32, f32, f32)> {
        (0..Self::CHANNELS.len())
            .filter_map(|c| {
                let n = CONTACTS[c].load(Ordering::Relaxed);
                if n == 0 {
                    return None;
                }
                let per = |v: &AtomicU64| v.load(Ordering::Relaxed) as f32 / n as f32;
                Some((
                    Self::CHANNELS[c],
                    n,
                    per(&CONTACT_DEFERRED[c]),
                    per(&CONTACT_HEIGHT_X100[c]) / 100.0,
                    per(&CONTACT_GAP_X100[c]) / 100.0,
                    per(&CONTACT_OVER_CEILING[c]),
                    per(&CONTACT_UNDRAWABLE[c]),
                ))
            })
            .collect()
    }

    /// Counts per [`Self::HEIGHT_BANDS`], both channels folded together.
    pub fn contact_height_snapshot() -> [u64; 5] {
        std::array::from_fn(|i| CONTACT_HEIGHT_BANDS[i].load(Ordering::Relaxed))
    }

    /// Book one back-line defender's state at a strike.
    pub fn note_defender_state(state_id: usize) {
        if let Some(c) = DEF_STATE_AT_STRIKE.get(state_id) {
            c.fetch_add(1, Ordering::Relaxed);
        }
    }

    /// Per-state counts, in discriminant order.
    pub fn defender_state_snapshot() -> [u64; 21] {
        std::array::from_fn(|i| DEF_STATE_AT_STRIKE[i].load(Ordering::Relaxed))
    }

    /// Book the nearest defending outfielder's distance to the
    /// shooter, in UNITS. Banding lives here so the caller cannot
    /// disagree with [`NEAREST_DEF_AT_STRIKE`]'s documented edges.
    pub fn note_shot_pressure(gap_units: f32) {
        let metres = gap_units / 8.0;
        let band = if metres < 1.0 {
            0
        } else if metres < 2.0 {
            1
        } else if metres < 3.0 {
            2
        } else if metres < 5.0 {
            3
        } else if metres < 8.0 {
            4
        } else {
            5
        };
        NEAREST_DEF_AT_STRIKE[band].fetch_add(1, Ordering::Relaxed);
    }

    /// Per-band counts, closest first.
    pub fn shot_pressure_snapshot() -> [u64; 6] {
        std::array::from_fn(|i| NEAREST_DEF_AT_STRIKE[i].load(Ordering::Relaxed))
    }

    /// Sample the defensive picture at the moment a shot is struck.
    /// `goalside` / `near_line` are counts for this one strike;
    /// `shot_range` / `def_depth` are distances to the defended goal.
    pub fn note_strike(goalside: u64, near_line: u64, shot_range: f32, def_depth: f32) {
        SHOTS_STRUCK.fetch_add(1, Ordering::Relaxed);
        GOALSIDE_AT_STRIKE.fetch_add(goalside, Ordering::Relaxed);
        GOALSIDE_NEAR_LINE.fetch_add(near_line, Ordering::Relaxed);
        SHOT_RANGE_X100.fetch_add((shot_range.max(0.0) * 100.0) as u64, Ordering::Relaxed);
        DEF_DEPTH_X100.fetch_add((def_depth.max(0.0) * 100.0) as u64, Ordering::Relaxed);
    }

    /// `(shots_struck, goalside_per_shot, near_line_per_shot,
    ///   mean_shot_range, mean_defender_depth)`
    pub fn strike_snapshot() -> (u64, f32, f32, f32, f32) {
        let n = SHOTS_STRUCK.load(Ordering::Relaxed);
        if n == 0 {
            return (0, 0.0, 0.0, 0.0, 0.0);
        }
        let per = |c: &AtomicU64| c.load(Ordering::Relaxed) as f32 / 100.0 / n as f32;
        (
            n,
            GOALSIDE_AT_STRIKE.load(Ordering::Relaxed) as f32 / n as f32,
            GOALSIDE_NEAR_LINE.load(Ordering::Relaxed) as f32 / n as f32,
            per(&SHOT_RANGE_X100),
            per(&DEF_DEPTH_X100),
        )
    }

    pub fn reset() {
        for c in [
            &SHOTS_SEEN,
            &TOO_HIGH,
            &CANDIDATES,
            &FIRED,
            &OPP_SEEN,
            &BEHIND_BALL,
            &BEYOND_LOOKAHEAD,
            &OUTSIDE_CORRIDOR,
            &PERP_SUM_X100,
            &IN_WINDOW,
            &SHOTS_STRUCK,
            &GOALSIDE_AT_STRIKE,
            &GOALSIDE_NEAR_LINE,
            &SHOT_RANGE_X100,
            &DEF_DEPTH_X100,
        ] {
            c.store(0, Ordering::Relaxed);
        }
        for c in &NEAREST_DEF_AT_STRIKE {
            c.store(0, Ordering::Relaxed);
        }
        for c in &DEF_STATE_AT_STRIKE {
            c.store(0, Ordering::Relaxed);
        }
        for bank in [
            &CONTACTS,
            &CONTACT_DEFERRED,
            &CONTACT_HEIGHT_X100,
            &CONTACT_GAP_X100,
            &CONTACT_OVER_CEILING,
            &CONTACT_UNDRAWABLE,
        ] {
            for c in bank {
                c.store(0, Ordering::Relaxed);
            }
        }
        for c in &CONTACT_HEIGHT_BANDS {
            c.store(0, Ordering::Relaxed);
        }
    }

    /// `(shots_seen, too_high, candidates, fired)`
    pub fn snapshot() -> (u64, u64, u64, u64) {
        (
            SHOTS_SEEN.load(Ordering::Relaxed),
            TOO_HIGH.load(Ordering::Relaxed),
            CANDIDATES.load(Ordering::Relaxed),
            FIRED.load(Ordering::Relaxed),
        )
    }

    /// `(opp_seen, behind_ball, beyond_lookahead, outside_corridor,
    ///   in_window, mean_perp)`
    pub fn lane_snapshot() -> (u64, u64, u64, u64, u64, f32) {
        let in_window = IN_WINDOW.load(Ordering::Relaxed);
        let mean_perp = if in_window == 0 {
            0.0
        } else {
            PERP_SUM_X100.load(Ordering::Relaxed) as f32 / 100.0 / in_window as f32
        };
        (
            OPP_SEEN.load(Ordering::Relaxed),
            BEHIND_BALL.load(Ordering::Relaxed),
            BEYOND_LOOKAHEAD.load(Ordering::Relaxed),
            OUTSIDE_CORRIDOR.load(Ordering::Relaxed),
            in_window,
            mean_perp,
        )
    }
}
