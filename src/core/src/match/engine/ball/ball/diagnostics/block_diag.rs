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
pub static CONTACTS: [AtomicU64; 5] = [const { AtomicU64::new(0) }; 5];
/// …of which arrived through the deferred `blocked_by` commitment
/// rather than firing on the tick the roll was won.
pub static CONTACT_DEFERRED: [AtomicU64; 5] = [const { AtomicU64::new(0) }; 5];
/// Ball height at the contact, metres x100, summed.
pub static CONTACT_HEIGHT_X100: [AtomicU64; 5] = [const { AtomicU64::new(0) }; 5];
/// Distance across the grass from the blocker, metres x100, summed.
pub static CONTACT_GAP_X100: [AtomicU64; 5] = [const { AtomicU64::new(0) }; 5];
/// Contacts above the channel's OWN stated ceiling — the number the
/// roll checked and the contact did not.
pub static CONTACT_OVER_CEILING: [AtomicU64; 5] = [const { AtomicU64::new(0) }; 5];
/// Contacts the replay rig cannot attribute to anybody: further than
/// `Actors::STRIKE_REACH` across the grass, or above `Actors::OVERHEAD`.
/// **This is the reported artefact, counted.**
pub static CONTACT_UNDRAWABLE: [AtomicU64; 5] = [const { AtomicU64::new(0) }; 5];
/// Height bands, shared by both channels:
/// `deck (<0.4) | shin-to-volley (<1.45) | head (<2.2) | jump (<2.8) | OVER`.
pub static CONTACT_HEIGHT_BANDS: [AtomicU64; 5] = [const { AtomicU64::new(0) }; 5];

/// Diagnostic accessors. Grouped on a struct so the module exposes
/// no free functions — the statics stay module-level because Rust
/// has no associated statics.
pub struct BlockDiag;

impl BlockDiag {
    /// Labels for the contact census channels, in index order.
    pub const CHANNELS: [&'static str; 5] = [
        "shot",
        "pass",
        "cross clear",
        "cross behind",
        "cross keeper",
    ];
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
        /// rather than shared because the viewer is a different crate and
        /// this is a measurement OF the disagreement.
        ///
        /// ⚠ **Keep this equal to `Actors::STRIKE_REACH`.** It was 1.7
        /// until 2026-09-07, when the rig was raised to the engine's own
        /// `KICKABLE_DISTANCE` (15u = 1.875 m) because the 17.5 cm between
        /// them was a shell around every player in which the engine grants
        /// a touch and the picture draws nobody touching anything. A
        /// counter that keeps the old number reports contacts as
        /// undrawable that the rig now draws.
        const DRAWN_REACH: f32 = 1.875;
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
        PassBlockCensus::reset();
        StrikeOrigin::reset();
        CrossPriceCensus::reset();
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

// ── Where a blocked pass goes ───────────────────────────────────
//
// A pass block decides a loose ball, and what that ball then does is
// the whole of what the block is worth: a corner, a throw-in, a
// scramble, or the blocker's own ball. Booked once per block, by the
// strike it stopped and how far from that strike the blocker stood.
/// Pass blocks resolved, by strike ([`PassBlockCensus::STRIKES`]) and
/// outcome ([`PassBlockCensus::OUTCOMES`]).
static PASS_BLOCK_OUTCOMES: [[AtomicU64; 4]; 2] = [const { [const { AtomicU64::new(0) }; 4] }; 2];
/// …by strike and the blocker's distance from the strike point
/// ([`PassBlockCensus::GAP_BANDS`]).
static PASS_BLOCK_GAPS: [[AtomicU64; 5]; 2] = [const { [const { AtomicU64::new(0) }; 5] }; 2];
/// …by strike and kind ([`PassBlockCensus::KINDS`]): a lunge, or a
/// charge-down the man had no time to react to.
static PASS_BLOCK_KINDS: [[AtomicU64; 2]; 2] = [const { [const { AtomicU64::new(0) }; 2] }; 2];

/// One block whose loose ball has not yet ended. The ball carries it
/// until the next touch, the ball going out, or a restart.
#[derive(Clone, Copy, Debug)]
pub struct PassBlockCensus {
    strike: usize,
}

impl PassBlockCensus {
    pub const STRIKES: [&'static str; 2] = ["pass", "cross"];
    pub const OUTCOMES: [&'static str; 4] = ["behind", "touch", "loose", "kept"];
    pub const GAP_BANDS: [&'static str; 5] = ["<1m", "1-2m", "2-3m", "3-5m", "5m+"];
    pub const KINDS: [&'static str; 2] = ["lunge", "charge-down"];

    pub const BEHIND: usize = 0;
    pub const TOUCH: usize = 1;
    pub const LOOSE: usize = 2;
    pub const KEPT: usize = 3;

    /// Book the block itself. `gap_u` is the blocker's distance from
    /// the strike point in units, `None` when the strike point is gone.
    pub fn open(cross: bool, gap_u: Option<f32>, charge_down: bool) -> Self {
        let strike = usize::from(cross);
        PASS_BLOCK_KINDS[strike][usize::from(charge_down)].fetch_add(1, Ordering::Relaxed);
        if let Some(gap) = gap_u {
            let metres = gap / 8.0;
            let band = if metres < 1.0 {
                0
            } else if metres < 2.0 {
                1
            } else if metres < 3.0 {
                2
            } else if metres < 5.0 {
                3
            } else {
                4
            };
            PASS_BLOCK_GAPS[strike][band].fetch_add(1, Ordering::Relaxed);
        }
        PassBlockCensus { strike }
    }

    pub fn close(self, outcome: usize) {
        PASS_BLOCK_OUTCOMES[self.strike][outcome].fetch_add(1, Ordering::Relaxed);
    }

    /// `(outcomes, gaps, kinds)` per strike, in [`Self::STRIKES`] order.
    pub fn snapshot() -> [([u64; 4], [u64; 5], [u64; 2]); 2] {
        std::array::from_fn(|s| {
            (
                std::array::from_fn(|o| PASS_BLOCK_OUTCOMES[s][o].load(Ordering::Relaxed)),
                std::array::from_fn(|g| PASS_BLOCK_GAPS[s][g].load(Ordering::Relaxed)),
                std::array::from_fn(|k| PASS_BLOCK_KINDS[s][k].load(Ordering::Relaxed)),
            )
        })
    }

    fn reset() {
        for bank in &PASS_BLOCK_OUTCOMES {
            for c in bank {
                c.store(0, Ordering::Relaxed);
            }
        }
        for bank in &PASS_BLOCK_GAPS {
            for c in bank {
                c.store(0, Ordering::Relaxed);
            }
        }
        for bank in &PASS_BLOCK_KINDS {
            for c in bank {
                c.store(0, Ordering::Relaxed);
            }
        }
    }
}

// ── Who struck a blocked ball, and what he priced ───────────────
//
// A charge-down lands on a ball struck into a body. Whether that is a
// striker who never priced the man, or one who priced him and played it
// anyway, are opposite fixes — so every strike carries where it came
// from until the next one.
/// Blocks by the striker's state at the strike (`PlayerState::compact_id`,
/// with [`StrikeOrigin::CLEARANCE`] for a clearance), split by kind
/// ([`PassBlockCensus::KINDS`]).
static BLOCKS_BY_STATE: [[AtomicU64; 2]; 501] = [const { [const { AtomicU64::new(0) }; 2] }; 501];
/// Passes struck, by the block price the striker's lane price put on that
/// exact pass at the strike ([`StrikeOrigin::PRICE_BANDS`]).
static STRUCK_BY_PRICE: [AtomicU64; 5] = [const { AtomicU64::new(0) }; 5];
/// …and of those, the ones then charged down.
static CHARGED_BY_PRICE: [AtomicU64; 5] = [const { AtomicU64::new(0) }; 5];

/// Where the ball in flight was struck from — see the section note.
#[derive(Clone, Copy, Debug)]
pub struct StrikeOrigin {
    state: usize,
    price_band: Option<usize>,
}

impl StrikeOrigin {
    pub const CLEARANCE: usize = 500;
    pub const PRICE_BANDS: [&'static str; 5] = ["<2%", "2-5%", "5-10%", "10-20%", "20%+"];

    /// A pass, struck from `state` with the block priced at `price`.
    pub fn pass(state: usize, price: f32) -> Self {
        let band = if price < 0.02 {
            0
        } else if price < 0.05 {
            1
        } else if price < 0.10 {
            2
        } else if price < 0.20 {
            3
        } else {
            4
        };
        STRUCK_BY_PRICE[band].fetch_add(1, Ordering::Relaxed);
        StrikeOrigin {
            state: state.min(Self::CLEARANCE - 1),
            price_band: Some(band),
        }
    }

    pub fn clearance() -> Self {
        StrikeOrigin {
            state: Self::CLEARANCE,
            price_band: None,
        }
    }

    /// Book a block of the ball this strike put in flight.
    pub fn note_block(&self, charge_down: bool) {
        BLOCKS_BY_STATE[self.state][usize::from(charge_down)].fetch_add(1, Ordering::Relaxed);
        if charge_down && let Some(band) = self.price_band {
            CHARGED_BY_PRICE[band].fetch_add(1, Ordering::Relaxed);
        }
    }

    /// `(state, lunges, charge-downs)`, most charge-downs first.
    pub fn by_state() -> Vec<(usize, u64, u64)> {
        let mut rows: Vec<(usize, u64, u64)> = (0..BLOCKS_BY_STATE.len())
            .map(|s| {
                (
                    s,
                    BLOCKS_BY_STATE[s][0].load(Ordering::Relaxed),
                    BLOCKS_BY_STATE[s][1].load(Ordering::Relaxed),
                )
            })
            .filter(|(_, lunges, charges)| lunges + charges > 0)
            .collect();
        rows.sort_by_key(|row| std::cmp::Reverse(row.2));
        rows
    }

    /// `(struck, charged down)` per price band.
    pub fn by_price() -> [(u64, u64); 5] {
        std::array::from_fn(|b| {
            (
                STRUCK_BY_PRICE[b].load(Ordering::Relaxed),
                CHARGED_BY_PRICE[b].load(Ordering::Relaxed),
            )
        })
    }

    fn reset() {
        for bank in &BLOCKS_BY_STATE {
            for c in bank {
                c.store(0, Ordering::Relaxed);
            }
        }
        for c in STRUCK_BY_PRICE.iter().chain(CHARGED_BY_PRICE.iter()) {
            c.store(0, Ordering::Relaxed);
        }
    }
}

// ── What the price does to the crosser ──────────────────────────
//
// Counted per tick a wide carrier reaches the delivery question, so the
// shares are of ticks, not of possessions.
/// Ticks the delivery was asked about.
static CROSS_ASKED: AtomicU64 = AtomicU64::new(0);
/// …with the best delivery's chance of getting past under 95%: a man on
/// its line.
static CROSS_PRESSED: AtomicU64 = AtomicU64::new(0);
/// …of those, the ones the price alone took under the bar.
static CROSS_PRICED_OUT: AtomicU64 = AtomicU64::new(0);
/// Deliveries struck, and the ones struck with a man on the line.
static CROSS_STRUCK: AtomicU64 = AtomicU64::new(0);
static CROSS_STRUCK_PRESSED: AtomicU64 = AtomicU64::new(0);
/// The chance of getting past, x10000, summed over the pressed ticks.
static CROSS_PAST_X10000: AtomicU64 = AtomicU64::new(0);
/// Where the man nearest the carrier stood ([`CrossPriceCensus::CLOSERS`]),
/// on the ticks asked and on the deliveries struck. "On the line" is
/// position only: the block rule can reach the delivery from where he is.
static CLOSER_ASKED: [AtomicU64; 5] = [const { AtomicU64::new(0) }; 5];
static CLOSER_STRUCK: [AtomicU64; 5] = [const { AtomicU64::new(0) }; 5];
/// Open-play crosses struck, by the closer's state at the strike
/// (`PlayerState::compact_id`, [`CrossPriceCensus::NO_CLOSER`] for
/// nobody near), off and on the delivery's line.
static CLOSER_STATE: [[AtomicU64; 2]; 501] = [const { [const { AtomicU64::new(0) }; 2] }; 501];

pub struct CrossPriceCensus;

impl CrossPriceCensus {
    /// How near the carrier the closer has to be (~5 m) — the grass the
    /// crosser reads for his run (`FlankPlay::RUN_ROOM`).
    pub const CLOSER_RADIUS: f32 = 40.0;
    pub const CLOSERS: [&'static str; 5] = [
        "on the line",
        "on the run",
        "goal-side elsewhere",
        "beaten",
        "nobody",
    ];
    pub const ON_THE_LINE: usize = 0;
    pub const ON_THE_RUN: usize = 1;
    pub const GOAL_SIDE: usize = 2;
    pub const BEATEN: usize = 3;
    pub const NOBODY: usize = 4;
    pub const NO_CLOSER: usize = 500;

    /// One delivery question: `past` is the chosen delivery's chance of
    /// getting past, `priced` the appetite with it, `bar` the
    /// possession's bar and `closer` where the nearest man stood.
    pub fn note(past: f32, priced: f32, bar: f32, closer: usize) {
        CROSS_ASKED.fetch_add(1, Ordering::Relaxed);
        CLOSER_ASKED[closer].fetch_add(1, Ordering::Relaxed);
        let pressed = past < 0.95;
        if pressed {
            CROSS_PRESSED.fetch_add(1, Ordering::Relaxed);
            CROSS_PAST_X10000.fetch_add((past * 10_000.0) as u64, Ordering::Relaxed);
            if priced < bar && priced / past.max(1.0e-3) >= bar {
                CROSS_PRICED_OUT.fetch_add(1, Ordering::Relaxed);
            }
        }
        if priced >= bar {
            CROSS_STRUCK.fetch_add(1, Ordering::Relaxed);
            CLOSER_STRUCK[closer].fetch_add(1, Ordering::Relaxed);
            if pressed {
                CROSS_STRUCK_PRESSED.fetch_add(1, Ordering::Relaxed);
            }
        }
    }

    /// `(asked, pressed, priced out, struck, struck pressed, mean past
    /// when pressed)`
    pub fn snapshot() -> (u64, u64, u64, u64, u64, f32) {
        let pressed = CROSS_PRESSED.load(Ordering::Relaxed);
        (
            CROSS_ASKED.load(Ordering::Relaxed),
            pressed,
            CROSS_PRICED_OUT.load(Ordering::Relaxed),
            CROSS_STRUCK.load(Ordering::Relaxed),
            CROSS_STRUCK_PRESSED.load(Ordering::Relaxed),
            CROSS_PAST_X10000.load(Ordering::Relaxed) as f32 / 10_000.0 / pressed.max(1) as f32,
        )
    }

    /// One open-play cross struck, with the closer's state and whether
    /// he was on its line.
    pub fn note_strike(state: usize, on_line: bool) {
        CLOSER_STATE[state.min(Self::NO_CLOSER)][usize::from(on_line)]
            .fetch_add(1, Ordering::Relaxed);
    }

    /// `(asked, struck)` per closer class.
    pub fn closers() -> [(u64, u64); 5] {
        std::array::from_fn(|c| {
            (
                CLOSER_ASKED[c].load(Ordering::Relaxed),
                CLOSER_STRUCK[c].load(Ordering::Relaxed),
            )
        })
    }

    /// `(state, off the line, on the line)`, most strikes first.
    pub fn closer_states() -> Vec<(usize, u64, u64)> {
        let mut rows: Vec<(usize, u64, u64)> = (0..CLOSER_STATE.len())
            .map(|s| {
                (
                    s,
                    CLOSER_STATE[s][0].load(Ordering::Relaxed),
                    CLOSER_STATE[s][1].load(Ordering::Relaxed),
                )
            })
            .filter(|(_, off, on)| off + on > 0)
            .collect();
        rows.sort_by_key(|row| std::cmp::Reverse(row.1 + row.2));
        rows
    }

    fn reset() {
        for c in CLOSER_ASKED.iter().chain(CLOSER_STRUCK.iter()) {
            c.store(0, Ordering::Relaxed);
        }
        for bank in &CLOSER_STATE {
            for c in bank {
                c.store(0, Ordering::Relaxed);
            }
        }
        for c in [
            &CROSS_ASKED,
            &CROSS_PRESSED,
            &CROSS_PRICED_OUT,
            &CROSS_STRUCK,
            &CROSS_STRUCK_PRESSED,
            &CROSS_PAST_X10000,
        ] {
            c.store(0, Ordering::Relaxed);
        }
    }
}
