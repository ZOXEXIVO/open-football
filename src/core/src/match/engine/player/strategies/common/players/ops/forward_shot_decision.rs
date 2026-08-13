use crate::r#match::MatchPlayerLite;
use crate::r#match::PlayerSide;
use crate::r#match::StateProcessingContext;
use crate::r#match::engine::psychology::Psychology;
use crate::r#match::player::strategies::players::ops::skill_composites as sc;
use nalgebra::Vector3;
#[cfg(feature = "match-logs")]
use std::sync::atomic::Ordering;

#[cfg(feature = "match-logs")]
pub mod helper_diag {
    use std::sync::atomic::{AtomicU64, Ordering};
    pub static CALLS: AtomicU64 = AtomicU64::new(0);
    pub static HOLD_HARDGATE: AtomicU64 = AtomicU64::new(0);
    pub static HOLD_FAR: AtomicU64 = AtomicU64::new(0);
    pub static HOLD_XG: AtomicU64 = AtomicU64::new(0);
    pub static HOLD_INSIDE_SIX_XG: AtomicU64 = AtomicU64::new(0);
    pub static HOLD_NO_CLEAR: AtomicU64 = AtomicU64::new(0);
    pub static PASS_DEFERRAL: AtomicU64 = AtomicU64::new(0);
    pub static REACHED_ROLL: AtomicU64 = AtomicU64::new(0);
    pub static ROLL_PASSED: AtomicU64 = AtomicU64::new(0);
    pub static SUM_XG_X1000: AtomicU64 = AtomicU64::new(0);
    pub static SUM_WILLINGNESS_X1000: AtomicU64 = AtomicU64::new(0);
    pub fn reset() {
        for c in [
            &CALLS,
            &HOLD_HARDGATE,
            &HOLD_FAR,
            &HOLD_XG,
            &HOLD_INSIDE_SIX_XG,
            &HOLD_NO_CLEAR,
            &PASS_DEFERRAL,
            &REACHED_ROLL,
            &ROLL_PASSED,
            &SUM_XG_X1000,
            &SUM_WILLINGNESS_X1000,
        ] {
            c.store(0, Ordering::Relaxed);
        }
    }
}

/// Diagnostic counters for the midfielder box-run + cutback redistribution
/// (`match-logs` only). These track the mechanism that funnels chances to
/// arriving central midfielders so the dev harness can see WHY the
/// GOALS-BY-LINE share moved (or didn't):
///   * `RUNNER_BOX_TICKS`   — ticks an elected runner spent in a central
///                            shooting position (≤62u, central corridor).
///   * `FWD_CUTBACK`        — forward laid a cutback to an arriving runner.
///   * `MID_CUTBACK`        — wide/advanced midfielder laid the cutback.
#[cfg(feature = "match-logs")]
pub mod mid_run_diag {
    use crate::r#match::player::strategies::passing::CrossType;
    use std::sync::atomic::{AtomicU64, Ordering};
    pub static RUNNER_BOX_TICKS: AtomicU64 = AtomicU64::new(0);
    pub static FWD_CUTBACK: AtomicU64 = AtomicU64::new(0);
    pub static MID_CUTBACK: AtomicU64 = AtomicU64::new(0);
    /// Ticks a midfielder held the ball within shooting range (≤88u) and
    /// reached the SHOOT-FIRST block — measures whether mids are being
    /// fed into range at all.
    pub static MID_INRANGE_TICKS: AtomicU64 = AtomicU64::new(0);
    /// Times the midfielder SHOOT-FIRST block actually emitted a shot —
    /// the conversion endpoint. INRANGE high + FIRED low ⇒ a shot gate is
    /// blocking; INRANGE low ⇒ the feed isn't completing.
    pub static MID_SHOOT_FIRED: AtomicU64 = AtomicU64::new(0);
    /// Times a centre-back headed ON GOAL from an attacking corner — the
    /// endpoint of the corner / defender-scoring mechanism.
    pub static DEF_CORNER_HEADER: AtomicU64 = AtomicU64::new(0);
    /// Attacking corners awarded (ball placed at the flag for our team).
    pub static CORNERS_AWARDED: AtomicU64 = AtomicU64::new(0);
    /// Ticks a centre-back spent in the AttackingCorner state (pushed up).
    pub static DEF_CORNER_ATTACK_TICKS: AtomicU64 = AtomicU64::new(0);
    /// Corner deliveries (crosses) actually struck.
    pub static CORNER_CROSS_SENT: AtomicU64 = AtomicU64::new(0);
    /// Corner deliveries aimed at a pushed-up centre-back.
    pub static CORNER_CROSS_TO_CB: AtomicU64 = AtomicU64::new(0);
    /// Times an aerial delivery actually came within a CB's heading reach
    /// (the header CHANCE, before the win roll). CHANCE>0 + HEADER=0 ⇒ the
    /// win roll / clearance is the gate; CHANCE=0 ⇒ the ball never reaches
    /// the CB (intercepted / overshoots).
    pub static DEF_CORNER_HEAD_CHANCE: AtomicU64 = AtomicU64::new(0);
    /// Discrete corner contest: armed corner cross seen in flight (before
    /// the z-loft gate). SEEN=0 ⇒ the resolver detection never matches.
    pub static CORNER_CONTEST_SEEN: AtomicU64 = AtomicU64::new(0);
    /// Discrete corner contest: passed every gate and a winner was picked.
    /// SEEN>0 + FIRED=0 ⇒ the loft / box-occupancy gate blocks it.
    pub static CORNER_CONTEST_FIRED: AtomicU64 = AtomicU64::new(0);
    /// Discrete corner contest: the attacker won the aerial and the ball
    /// was dropped on their head. WON>0 + DEF_CORNER_HEADER=0 ⇒ the winner
    /// isn't heading the planted ball.
    pub static CORNER_CONTEST_WON: AtomicU64 = AtomicU64::new(0);
    /// Times the shot-BLOCK "deflect out for a corner" branch fired.
    pub static BLOCK_CORNER_FIRED: AtomicU64 = AtomicU64::new(0);
    /// Times the keeper SAFE-PARRY "palm wide for a corner" branch fired.
    pub static SAVE_PARRY_FIRED: AtomicU64 = AtomicU64::new(0);
    /// Penalties awarded (box foul whistled → spot kick restart).
    /// Real football ≈ 0.25-0.30 per match.
    pub static PENALTY_AWARDED: AtomicU64 = AtomicU64::new(0);
    /// Direct free kicks awarded for fouls outside the box.
    pub static DIRECT_FK_AWARDED: AtomicU64 = AtomicU64::new(0);

    /// Open-play cross deliveries struck, bucketed by
    /// [`CrossType::diag_index`]. Answers "are we producing a MIX of
    /// deliveries, or has one branch swallowed the model?".
    pub static CROSS_BY_TYPE: [AtomicU64; 5] = [
        AtomicU64::new(0),
        AtomicU64::new(0),
        AtomicU64::new(0),
        AtomicU64::new(0),
        AtomicU64::new(0),
    ];
    /// Open-play cross aerial contests: reached the resolver with an
    /// armed lofted delivery over the box.
    pub static CROSS_CONTEST_SEEN: AtomicU64 = AtomicU64::new(0);
    /// …passed every gate and a contest was actually rolled.
    pub static CROSS_CONTEST_FIRED: AtomicU64 = AtomicU64::new(0);
    /// …an attacker won the aerial and the ball was dropped on their head.
    pub static CROSS_CONTEST_WON: AtomicU64 = AtomicU64::new(0);
    /// …the keeper claimed the delivery out of the air.
    pub static CROSS_CONTEST_GK: AtomicU64 = AtomicU64::new(0);
    /// Headers struck ON GOAL off an open-play cross — the endpoint of
    /// the whole crossing chain.
    pub static CROSS_HEADER_ON_GOAL: AtomicU64 = AtomicU64::new(0);

    /// Tactical refreshes seen, and how many produced a live attacking
    /// plan with at least one box slot filled. A plan that is rarely
    /// active explains a shot mix that hasn't moved, and is a different
    /// problem from a plan whose slots are wrong.
    pub static PLAN_REFRESH: AtomicU64 = AtomicU64::new(0);
    pub static PLAN_ACTIVE: AtomicU64 = AtomicU64::new(0);
    /// Box slots filled across all active refreshes — divided by
    /// `PLAN_ACTIVE` this is "how many of the four zones does a live
    /// attack actually occupy".
    pub static PLAN_SLOTS_FILLED: AtomicU64 = AtomicU64::new(0);
    /// Ticks a player spent moving to an assigned slot rather than to a
    /// locally-derived target.
    pub static PLAN_SLOT_TICKS: AtomicU64 = AtomicU64::new(0);

    /// Attacking-plan coverage counters.
    pub struct PlanDiag;

    impl PlanDiag {
        pub fn note_refresh(active: bool, slots_filled: usize) {
            PLAN_REFRESH.fetch_add(1, Ordering::Relaxed);
            if active {
                PLAN_ACTIVE.fetch_add(1, Ordering::Relaxed);
                PLAN_SLOTS_FILLED.fetch_add(slots_filled as u64, Ordering::Relaxed);
            }
        }

        pub fn note_slot_tick() {
            PLAN_SLOT_TICKS.fetch_add(1, Ordering::Relaxed);
        }

        /// `(refreshes, active, slots_filled, slot_ticks)`
        pub fn snapshot() -> (u64, u64, u64, u64) {
            (
                PLAN_REFRESH.load(Ordering::Relaxed),
                PLAN_ACTIVE.load(Ordering::Relaxed),
                PLAN_SLOTS_FILLED.load(Ordering::Relaxed),
                PLAN_SLOT_TICKS.load(Ordering::Relaxed),
            )
        }

        pub fn reset() {
            for c in [
                &PLAN_REFRESH,
                &PLAN_ACTIVE,
                &PLAN_SLOTS_FILLED,
                &PLAN_SLOT_TICKS,
            ] {
                c.store(0, Ordering::Relaxed);
            }
        }
    }

    /// Defensive-shape samples: taken while the opposition has the ball
    /// in our half, so they describe DEFENDING rather than an average
    /// over a match spent attacking.
    pub static DEF_SAMPLES: AtomicU64 = AtomicU64::new(0);
    /// Sum of the back line's depth SPREAD (max x − min x along the
    /// goal-to-goal axis), in units ×100. A real back four staggers 3-8 m
    /// — the cover defender drops, the far full-back tucks in. A number
    /// near zero is a rigid line moving as one body.
    pub static DEF_DEPTH_SPREAD_X100: AtomicU64 = AtomicU64::new(0);
    /// Sum of the LATERAL gap between the widest-apart adjacent pair, ×100.
    pub static DEF_MAX_GAP_X100: AtomicU64 = AtomicU64::new(0);
    /// Attackers sampled in our defensive third, and how many had NO
    /// defender within a marking radius.
    pub static DEF_ATTACKERS_SEEN: AtomicU64 = AtomicU64::new(0);
    pub static DEF_ATTACKERS_UNMARKED: AtomicU64 = AtomicU64::new(0);
    /// Sum of each sampled attacker's distance to the nearest defender,
    /// ×100. Divided by `DEF_ATTACKERS_SEEN` this is "how far away the
    /// nearest defender actually is" — the direct measure of whether
    /// anybody meets the attacker.
    pub static DEF_NEAREST_MARKER_X100: AtomicU64 = AtomicU64::new(0);

    /// Defensive-duty assignment coverage: refreshes seen, refreshes with
    /// a live plan, and how many of the unit held an INDIVIDUAL duty
    /// (press / cover / mark) rather than just holding a zone.
    pub static DEF_PLAN_REFRESH: AtomicU64 = AtomicU64::new(0);
    pub static DEF_PLAN_ACTIVE: AtomicU64 = AtomicU64::new(0);
    pub static DEF_PLAN_INDIVIDUAL: AtomicU64 = AtomicU64::new(0);

    /// Marker-evasion coverage: how often an attacker asked to evade,
    /// how often anybody was actually marking him, and how much room the
    /// contest gave him. A low marked-rate means the read is too strict;
    /// a low edge means the offset is being applied but is too small to
    /// matter.
    pub static EVASION_CALLS: AtomicU64 = AtomicU64::new(0);
    pub static EVASION_MARKED: AtomicU64 = AtomicU64::new(0);
    pub static EVASION_TIGHT_X1000: AtomicU64 = AtomicU64::new(0);
    pub static EVASION_EDGE_X1000: AtomicU64 = AtomicU64::new(0);

    pub struct EvasionDiag;

    impl EvasionDiag {
        pub fn note_call() {
            EVASION_CALLS.fetch_add(1, Ordering::Relaxed);
        }

        pub fn note_marked(tightness: f32, edge: f32) {
            EVASION_MARKED.fetch_add(1, Ordering::Relaxed);
            EVASION_TIGHT_X1000.fetch_add((tightness * 1000.0) as u64, Ordering::Relaxed);
            EVASION_EDGE_X1000.fetch_add((edge * 1000.0) as u64, Ordering::Relaxed);
        }

        /// `(calls, marked, mean_tightness, mean_edge)`
        pub fn snapshot() -> (u64, u64, f32, f32) {
            let m = EVASION_MARKED.load(Ordering::Relaxed).max(1);
            (
                EVASION_CALLS.load(Ordering::Relaxed),
                EVASION_MARKED.load(Ordering::Relaxed),
                EVASION_TIGHT_X1000.load(Ordering::Relaxed) as f32 / 1000.0 / m as f32,
                EVASION_EDGE_X1000.load(Ordering::Relaxed) as f32 / 1000.0 / m as f32,
            )
        }

        pub fn reset() {
            for c in [
                &EVASION_CALLS,
                &EVASION_MARKED,
                &EVASION_TIGHT_X1000,
                &EVASION_EDGE_X1000,
            ] {
                c.store(0, Ordering::Relaxed);
            }
        }
    }

    /// How far a defender holding shape is from the shape target he is
    /// steering to, sampled where `DefensiveLine::hold_shape` computes
    /// it. The shape constraint bounds TARGETS to a 64u span while
    /// measured POSITIONS spread 137u, and this is the quantity that
    /// separates the two explanations: a large lag means he is chronically
    /// failing to arrive (steering, speed, or a target that outruns him),
    /// a small one means the targets themselves are spread and the
    /// constraint is not doing what it claims.
    pub static SHAPE_LAG_X100: AtomicU64 = AtomicU64::new(0);
    pub static SHAPE_LAG_N: AtomicU64 = AtomicU64::new(0);
    /// Same, split by whether the defender is in the half of the sample
    /// nearest his own goal — a lag that only appears deep is a recovery
    /// problem, one that is flat is a steering problem.
    pub static SHAPE_LAG_MAX_X100: AtomicU64 = AtomicU64::new(0);
    /// Summed `in_state_time` over the same samples. A defender who never
    /// arrives because his STATE keeps changing under him is a different
    /// problem from one who is steering too slowly, and this separates
    /// them: a mean dwell of a handful of ticks means he is being handed
    /// a new target before he can act on the last one.
    pub static SHAPE_DWELL: AtomicU64 = AtomicU64::new(0);
    /// Lag split into depth (0) and width (1), so the axis that is
    /// failing is named rather than inferred.
    pub static SHAPE_LAG_AXIS_X100: [AtomicU64; 2] = [AtomicU64::new(0), AtomicU64::new(0)];

    /// Which branch actually sent a defender into `Clearing`.
    ///
    /// Clearances measure 10.2 per defender per match against a real
    /// ~3.5, and three rounds of narrowing individual gates by
    /// inspection moved the total once and then not at all — because the
    /// branch being narrowed was not the one firing. There are nine
    /// separate routes into that state across two files and no way to
    /// tell them apart from the outside.
    pub const CLEAR_REASONS: usize = 9;
    const ZERO_C: AtomicU64 = AtomicU64::new(0);
    pub static CLEAR_BY_REASON: [AtomicU64; CLEAR_REASONS] = [ZERO_C; CLEAR_REASONS];
    pub const CLEAR_REASON_NAMES: [&str; CLEAR_REASONS] = [
        "run:box-pressed",
        "run:congested",
        "run:no-pass-target",
        "run:immediate-pressure",
        "run:250t-force",
        "pass:must-clear",
        "pass:no-safe-option",
        "pass:dangerous-position",
        "pass:65t-no-safe",
    ];

    pub struct ClearDiag;

    impl ClearDiag {
        #[inline]
        pub fn note(reason: usize) {
            if let Some(c) = CLEAR_BY_REASON.get(reason) {
                c.fetch_add(1, Ordering::Relaxed);
            }
        }

        pub fn snapshot() -> [u64; CLEAR_REASONS] {
            let mut out = [0u64; CLEAR_REASONS];
            for (o, c) in out.iter_mut().zip(CLEAR_BY_REASON.iter()) {
                *o = c.load(Ordering::Relaxed);
            }
            out
        }

        pub fn reset() {
            for c in CLEAR_BY_REASON.iter() {
                c.store(0, Ordering::Relaxed);
            }
        }
    }

    /// Defensive-shape counters.
    pub struct DefenceDiag;

    /// Distance between an assigned marker and the man he was given —
    /// the marking duel itself, as opposed to general defensive density.
    pub static DEF_DUELS: AtomicU64 = AtomicU64::new(0);
    pub static DEF_DUEL_GAP_X100: AtomicU64 = AtomicU64::new(0);
    /// Duels where the attacker has genuinely got away (>4 m).
    pub static DEF_DUELS_LOST: AtomicU64 = AtomicU64::new(0);
    /// Duels where the assigned marker was in a state that actually acts
    /// on the assignment (`Marking` / `Guarding`), and their gap sum. A
    /// low share here means the marking DISTANCE is not the thing to
    /// tune — most markers are not marking.
    pub static DEF_DUELS_ON_TASK: AtomicU64 = AtomicU64::new(0);
    pub static DEF_DUEL_GAP_ON_TASK_X100: AtomicU64 = AtomicU64::new(0);
    /// Duels bucketed by what the marker was actually doing: 0 marking,
    /// 1 playing the ball, 2 pressing/covering, 3 running/recovering,
    /// 4 idle. Buckets 3 and 4 are duties nobody is acting on.
    pub static DEF_DUEL_BY_STATE: [AtomicU64; 5] = [
        AtomicU64::new(0),
        AtomicU64::new(0),
        AtomicU64::new(0),
        AtomicU64::new(0),
        AtomicU64::new(0),
    ];
    /// The same duels split by the LINE of the man being marked. The
    /// aggregate cannot answer the question that matters — whether the
    /// marking that is happening is happening to FORWARDS — and the
    /// attacking-side evasion work is only wired into forward states, so
    /// an aggregate dominated by marked midfielders is blind to it.
    /// Index 0 = defender, 1 = midfielder, 2 = forward.
    pub static DEF_DUELS_BY_LINE: [AtomicU64; 3] =
        [AtomicU64::new(0), AtomicU64::new(0), AtomicU64::new(0)];
    pub static DEF_DUEL_GAP_BY_LINE_X100: [AtomicU64; 3] =
        [AtomicU64::new(0), AtomicU64::new(0), AtomicU64::new(0)];

    impl DefenceDiag {
        /// Record one sample of how far a shape-holding defender is from
        /// the target the shape constraint just handed him.
        pub fn note_shape_lag(lag: f32, in_state_time: u64, lag_x: f32, lag_y: f32) {
            SHAPE_LAG_X100.fetch_add((lag * 100.0) as u64, Ordering::Relaxed);
            SHAPE_LAG_N.fetch_add(1, Ordering::Relaxed);
            SHAPE_LAG_MAX_X100.fetch_max((lag * 100.0) as u64, Ordering::Relaxed);
            SHAPE_DWELL.fetch_add(in_state_time, Ordering::Relaxed);
            SHAPE_LAG_AXIS_X100[0].fetch_add((lag_x * 100.0) as u64, Ordering::Relaxed);
            SHAPE_LAG_AXIS_X100[1].fetch_add((lag_y * 100.0) as u64, Ordering::Relaxed);
        }

        /// `(samples, mean_lag, max_lag, mean_dwell_ticks, mean_lag_x, mean_lag_y)`
        pub fn shape_lag() -> (u64, f32, f32, f32, f32, f32) {
            let n = SHAPE_LAG_N.load(Ordering::Relaxed);
            let per = |v: u64| {
                if n == 0 {
                    0.0
                } else {
                    v as f32 / 100.0 / n as f32
                }
            };
            (
                n,
                per(SHAPE_LAG_X100.load(Ordering::Relaxed)),
                SHAPE_LAG_MAX_X100.load(Ordering::Relaxed) as f32 / 100.0,
                if n == 0 {
                    0.0
                } else {
                    SHAPE_DWELL.load(Ordering::Relaxed) as f32 / n as f32
                },
                per(SHAPE_LAG_AXIS_X100[0].load(Ordering::Relaxed)),
                per(SHAPE_LAG_AXIS_X100[1].load(Ordering::Relaxed)),
            )
        }

        pub fn note_duel(gap: f32, marked_line: usize, bucket: usize) {
            DEF_DUELS.fetch_add(1, Ordering::Relaxed);
            DEF_DUEL_GAP_X100.fetch_add((gap * 100.0) as u64, Ordering::Relaxed);
            if let Some(c) = DEF_DUEL_BY_STATE.get(bucket) {
                c.fetch_add(1, Ordering::Relaxed);
            }
            if bucket == 0 {
                DEF_DUELS_ON_TASK.fetch_add(1, Ordering::Relaxed);
                DEF_DUEL_GAP_ON_TASK_X100.fetch_add((gap * 100.0) as u64, Ordering::Relaxed);
            }
            if gap > 32.0 {
                DEF_DUELS_LOST.fetch_add(1, Ordering::Relaxed);
            }
            if let (Some(c), Some(g)) = (
                DEF_DUELS_BY_LINE.get(marked_line),
                DEF_DUEL_GAP_BY_LINE_X100.get(marked_line),
            ) {
                c.fetch_add(1, Ordering::Relaxed);
                g.fetch_add((gap * 100.0) as u64, Ordering::Relaxed);
            }
        }

        /// `(duels, mean_gap, share_lost)`
        pub fn duel_snapshot() -> (u64, f32, f32) {
            let n = DEF_DUELS.load(Ordering::Relaxed);
            if n == 0 {
                return (0, 0.0, 0.0);
            }
            (
                n,
                DEF_DUEL_GAP_X100.load(Ordering::Relaxed) as f32 / 100.0 / n as f32,
                DEF_DUELS_LOST.load(Ordering::Relaxed) as f32 / n as f32,
            )
        }

        /// Duel counts by what the marker was doing — see `DEF_DUEL_BY_STATE`.
        pub fn duel_by_state() -> [u64; 5] {
            let mut out = [0u64; 5];
            for i in 0..5 {
                out[i] = DEF_DUEL_BY_STATE[i].load(Ordering::Relaxed);
            }
            out
        }

        /// `(share of duels where the marker was in a marking state, their mean gap)`
        pub fn duel_on_task() -> (f32, f32) {
            let n = DEF_DUELS.load(Ordering::Relaxed);
            let k = DEF_DUELS_ON_TASK.load(Ordering::Relaxed);
            if n == 0 || k == 0 {
                return (0.0, 0.0);
            }
            (
                k as f32 / n as f32,
                DEF_DUEL_GAP_ON_TASK_X100.load(Ordering::Relaxed) as f32 / 100.0 / k as f32,
            )
        }

        /// Per-line `(count, mean_gap)` for defender / midfielder / forward.
        pub fn duel_by_line() -> [(u64, f32); 3] {
            let mut out = [(0u64, 0.0f32); 3];
            for i in 0..3 {
                let n = DEF_DUELS_BY_LINE[i].load(Ordering::Relaxed);
                out[i] = (
                    n,
                    if n == 0 {
                        0.0
                    } else {
                        DEF_DUEL_GAP_BY_LINE_X100[i].load(Ordering::Relaxed) as f32
                            / 100.0
                            / n as f32
                    },
                );
            }
            out
        }

        pub fn note_plan(active: bool, individual: usize) {
            DEF_PLAN_REFRESH.fetch_add(1, Ordering::Relaxed);
            if active {
                DEF_PLAN_ACTIVE.fetch_add(1, Ordering::Relaxed);
                DEF_PLAN_INDIVIDUAL.fetch_add(individual as u64, Ordering::Relaxed);
            }
        }

        /// `(refreshes, active, mean_individual_duties_when_live)`
        pub fn plan_snapshot() -> (u64, u64, f32) {
            let a = DEF_PLAN_ACTIVE.load(Ordering::Relaxed);
            (
                DEF_PLAN_REFRESH.load(Ordering::Relaxed),
                a,
                if a == 0 {
                    0.0
                } else {
                    DEF_PLAN_INDIVIDUAL.load(Ordering::Relaxed) as f32 / a as f32
                },
            )
        }
    }

    impl DefenceDiag {
        pub fn note_shape(depth_spread: f32, max_lateral_gap: f32) {
            DEF_SAMPLES.fetch_add(1, Ordering::Relaxed);
            DEF_DEPTH_SPREAD_X100.fetch_add((depth_spread * 100.0) as u64, Ordering::Relaxed);
            DEF_MAX_GAP_X100.fetch_add((max_lateral_gap * 100.0) as u64, Ordering::Relaxed);
        }

        pub fn note_attacker(nearest_marker: f32, unmarked: bool) {
            DEF_ATTACKERS_SEEN.fetch_add(1, Ordering::Relaxed);
            DEF_NEAREST_MARKER_X100.fetch_add((nearest_marker * 100.0) as u64, Ordering::Relaxed);
            if unmarked {
                DEF_ATTACKERS_UNMARKED.fetch_add(1, Ordering::Relaxed);
            }
        }

        /// `(samples, mean_depth_spread, mean_max_gap, attackers, unmarked, mean_nearest)`
        pub fn snapshot() -> (u64, f32, f32, u64, u64, f32) {
            let n = DEF_SAMPLES.load(Ordering::Relaxed);
            let a = DEF_ATTACKERS_SEEN.load(Ordering::Relaxed);
            let per = |v: u64, d: u64| {
                if d == 0 {
                    0.0
                } else {
                    v as f32 / 100.0 / d as f32
                }
            };
            (
                n,
                per(DEF_DEPTH_SPREAD_X100.load(Ordering::Relaxed), n),
                per(DEF_MAX_GAP_X100.load(Ordering::Relaxed), n),
                a,
                DEF_ATTACKERS_UNMARKED.load(Ordering::Relaxed),
                per(DEF_NEAREST_MARKER_X100.load(Ordering::Relaxed), a),
            )
        }

        pub fn reset() {
            for c in [
                &DEF_SAMPLES,
                &DEF_DEPTH_SPREAD_X100,
                &DEF_MAX_GAP_X100,
                &DEF_ATTACKERS_SEEN,
                &DEF_ATTACKERS_UNMARKED,
                &DEF_NEAREST_MARKER_X100,
                &DEF_PLAN_REFRESH,
                &DEF_PLAN_ACTIVE,
                &DEF_PLAN_INDIVIDUAL,
                &DEF_DUELS,
                &DEF_DUEL_GAP_X100,
                &DEF_DUELS_LOST,
                &DEF_DUELS_ON_TASK,
                &DEF_DUEL_GAP_ON_TASK_X100,
                &DEF_DUEL_BY_STATE[0],
                &DEF_DUEL_BY_STATE[1],
                &DEF_DUEL_BY_STATE[2],
                &DEF_DUEL_BY_STATE[3],
                &DEF_DUEL_BY_STATE[4],
                &SHAPE_LAG_X100,
                &SHAPE_LAG_N,
                &SHAPE_LAG_MAX_X100,
                &SHAPE_DWELL,
                &SHAPE_LAG_AXIS_X100[0],
                &SHAPE_LAG_AXIS_X100[1],
            ] {
                c.store(0, Ordering::Relaxed);
            }
            for c in DEF_DUELS_BY_LINE
                .iter()
                .chain(DEF_DUEL_GAP_BY_LINE_X100.iter())
            {
                c.store(0, Ordering::Relaxed);
            }
        }
    }

    /// Crossing-chain counters. Bundled so the delivery mix, the contest
    /// funnel and the header endpoint are read and reset as one thing.
    pub struct CrossDiag;

    impl CrossDiag {
        /// Record a delivery against its type.
        pub fn note(cross_type: CrossType) {
            CROSS_BY_TYPE[cross_type.diag_index()].fetch_add(1, Ordering::Relaxed);
        }

        /// Deliveries struck, in [`CrossType::ALL`] order.
        pub fn by_type() -> [u64; 5] {
            let mut out = [0u64; 5];
            for (slot, counter) in out.iter_mut().zip(CROSS_BY_TYPE.iter()) {
                *slot = counter.load(Ordering::Relaxed);
            }
            out
        }

        /// `(seen, fired, attacker_won, keeper_claimed, headers_on_goal)`
        pub fn contest() -> (u64, u64, u64, u64, u64) {
            (
                CROSS_CONTEST_SEEN.load(Ordering::Relaxed),
                CROSS_CONTEST_FIRED.load(Ordering::Relaxed),
                CROSS_CONTEST_WON.load(Ordering::Relaxed),
                CROSS_CONTEST_GK.load(Ordering::Relaxed),
                CROSS_HEADER_ON_GOAL.load(Ordering::Relaxed),
            )
        }

        pub fn reset() {
            for c in CROSS_BY_TYPE.iter() {
                c.store(0, Ordering::Relaxed);
            }
            for c in [
                &CROSS_CONTEST_SEEN,
                &CROSS_CONTEST_FIRED,
                &CROSS_CONTEST_WON,
                &CROSS_CONTEST_GK,
                &CROSS_HEADER_ON_GOAL,
            ] {
                c.store(0, Ordering::Relaxed);
            }
        }
    }

    pub fn reset() {
        CrossDiag::reset();
        PlanDiag::reset();
        DefenceDiag::reset();
        ClearDiag::reset();
        EvasionDiag::reset();
        for c in [
            &RUNNER_BOX_TICKS,
            &FWD_CUTBACK,
            &MID_CUTBACK,
            &MID_INRANGE_TICKS,
            &MID_SHOOT_FIRED,
            &DEF_CORNER_HEADER,
            &CORNERS_AWARDED,
            &DEF_CORNER_ATTACK_TICKS,
            &CORNER_CROSS_SENT,
            &CORNER_CROSS_TO_CB,
            &DEF_CORNER_HEAD_CHANCE,
            &CORNER_CONTEST_SEEN,
            &CORNER_CONTEST_FIRED,
            &CORNER_CONTEST_WON,
            &BLOCK_CORNER_FIRED,
            &SAVE_PARRY_FIRED,
            &PENALTY_AWARDED,
            &DIRECT_FK_AWARDED,
        ] {
            c.store(0, Ordering::Relaxed);
        }
    }
    pub fn snapshot() -> [u64; 16] {
        [
            RUNNER_BOX_TICKS.load(Ordering::Relaxed),
            FWD_CUTBACK.load(Ordering::Relaxed),
            MID_CUTBACK.load(Ordering::Relaxed),
            MID_INRANGE_TICKS.load(Ordering::Relaxed),
            MID_SHOOT_FIRED.load(Ordering::Relaxed),
            DEF_CORNER_HEADER.load(Ordering::Relaxed),
            CORNERS_AWARDED.load(Ordering::Relaxed),
            DEF_CORNER_ATTACK_TICKS.load(Ordering::Relaxed),
            CORNER_CROSS_SENT.load(Ordering::Relaxed),
            CORNER_CROSS_TO_CB.load(Ordering::Relaxed),
            DEF_CORNER_HEAD_CHANCE.load(Ordering::Relaxed),
            CORNER_CONTEST_SEEN.load(Ordering::Relaxed),
            CORNER_CONTEST_FIRED.load(Ordering::Relaxed),
            CORNER_CONTEST_WON.load(Ordering::Relaxed),
            BLOCK_CORNER_FIRED.load(Ordering::Relaxed),
            SAVE_PARRY_FIRED.load(Ordering::Relaxed),
        ]
    }
}

/// Time-of-match production diagnostics (`match-logs` only). Everything
/// is bucketed into six 15-minute bands (index = minute/15, clamped to
/// band 5 so stoppage time folds into 75-90). The dev harness's
/// goals-by-minute histogram showed scoring DECAYING across the match
/// (36% of goals in minutes 0-15 vs real ~11%, rising to ~26% late);
/// these counters split that into volume (shots/band) vs quality
/// (xG/shot) vs conversion (goals/shot) so the calibration lever is
/// identifiable instead of guessed.
#[cfg(feature = "match-logs")]
pub mod time_band_diag {
    use std::sync::atomic::{AtomicU64, Ordering};

    pub const BANDS: usize = 6;
    const ZERO: AtomicU64 = AtomicU64::new(0);

    /// Shots struck (handle_shoot_event reached trajectory resolution).
    pub static SHOTS_BY_BAND: [AtomicU64; BANDS] = [ZERO; BANDS];
    /// Shots whose final aim threatened the frame (same flag the
    /// shooter's on-target memory uses).
    pub static ON_TARGET_BY_BAND: [AtomicU64; BANDS] = [ZERO; BANDS];
    /// Sum of shooter xG ×1000 (location/skill chance value at strike).
    pub static XG_X1000_BY_BAND: [AtomicU64; BANDS] = [ZERO; BANDS];
    /// Real goals (own goals excluded — they carry no shooter xG).
    pub static GOALS_BY_BAND: [AtomicU64; BANDS] = [ZERO; BANDS];
    /// Willingness-roll attempts reaching the RNG in the forward shot
    /// helper — the volume signal BEFORE gates fire.
    pub static ROLL_REACHED_BY_BAND: [AtomicU64; BANDS] = [ZERO; BANDS];

    /// Shots / xG / goals bucketed by DISTANCE from goal rather than
    /// minute. Bands (1u = 0.125m): 0 = <6m, 1 = 6-11m, 2 = 11-16.5m
    /// (box edge), 3 = 16.5-22m, 4 = 22-30m, 5 = 30m+. Real Opta shot
    /// mix is roughly 15 / 25 / 22 / 20 / 13 / 5 %, i.e. ~40% of shots
    /// come from OUTSIDE the box. An engine that concentrates its shots
    /// in bands 0-1 is manufacturing sitters, which shows up as
    /// inflated xG/shot and inflated rating tails no matter how the
    /// willingness dials are set.
    pub static SHOTS_BY_DIST: [AtomicU64; BANDS] = [ZERO; BANDS];
    /// Emitted shots split by POSITION GROUP x distance band
    /// (0=GK 1=DEF 2=MID 3=FWD). Answers "do midfielders shoot from
    /// different places than forwards" — the aggregate mix cannot.
    pub static SHOTS_BY_POS_DIST: [[AtomicU64; BANDS]; 4] = [ZERO_BAND; 4];
    pub fn pos_dist_snapshot() -> [[u64; BANDS]; 4] {
        let mut out = [[0u64; BANDS]; 4];
        for g in 0..4 {
            for b in 0..BANDS {
                out[g][b] = SHOTS_BY_POS_DIST[g][b].load(Ordering::Relaxed);
            }
        }
        out
    }
    pub static XG_X1000_BY_DIST: [AtomicU64; BANDS] = [ZERO; BANDS];
    /// Willingness rolls that REACHED the RNG, by distance band. Read
    /// against SHOTS_BY_DIST this separates "the ball is never out
    /// there in a shooting posture" (rolls concentrated close) from
    /// "players decline long shots" (rolls spread, shots close).
    pub static ROLLS_BY_DIST: [AtomicU64; BANDS] = [ZERO; BANDS];
    /// Helper CALLS by distance, counted before any gate. Read against
    /// ROLLS_BY_DIST this separates "no shot decision is even offered
    /// out here" (calls low) from "offered but gated" (calls high,
    /// rolls low).
    pub static CALLS_BY_DIST: [AtomicU64; BANDS] = [ZERO; BANDS];
    /// Shot decisions the helper APPROVED, by band. Compared against
    /// SHOTS_BY_DIST (shots actually emitted) this isolates loss that
    /// happens AFTER approval — in the caller's branch or the Shooting
    /// state — from loss inside the decision itself.
    pub static APPROVED_BY_DIST: [AtomicU64; BANDS] = [ZERO; BANDS];
    /// Queued shots destroyed: the player had a `pending_shot_reason`
    /// (an APPROVED strike waiting for the Shooting state to run) and
    /// was moved to a non-shot state before it could fire.
    pub static QUEUED_SHOT_LOST: [AtomicU64; BANDS] = [ZERO; BANDS];

    /// Long-range (>176u = 22m) approvals bucketed by the call-site tag
    /// the helper was invoked with — names the caller responsible for
    /// approvals that never become shots.
    pub const TAGS: usize = 13;
    pub static APPROVED_BY_TAG: [AtomicU64; TAGS] = [ZERO_ONE; TAGS];
    const ZERO_ONE: AtomicU64 = AtomicU64::new(0);
    pub const TAG_NAMES: [&str; TAGS] = [
        "FWD_PRIO05",
        "FWD_PRIO06",
        "FWD_POINTBLANK",
        "FWD_ANTIOSC",
        "FWD_FINISHING",
        "FWD_RIB",
        "FWD_LASTMILE",
        "FWD_DRIB/STAND",
        "MID_SHOOT",
        "MID_ANTIOSC",
        "MID_PASS_FWD",
        "MID_BAILOUT",
        "other",
    ];
    #[inline]
    pub fn tag_index(tag: &str) -> usize {
        match tag {
            "FWD_RUN_PRIO05_CLEAR" => 0,
            "FWD_RUN_PRIO06_BOX" => 1,
            "FWD_RUN_POINT_BLANK" => 2,
            "FWD_RUN_ANTI_OSCILLATION" => 3,
            "FWD_FINISHING" => 4,
            "FWD_RIB_SHOT" => 5,
            "FWD_SHOOTING_LASTMILE" => 6,
            t if t.starts_with("FWD_DRIB") || t.starts_with("FWD_STAND") => 7,
            "MID_SHOOT" => 8,
            "AM_RUN_ANTI_OSC_FWD" => 9,
            "MID_PASS_FWD" => 10,
            "AM_PASS_BAILOUT_FWD" => 11,
            _ => 12,
        }
    }
    /// EMITTED shots in the 6-11m band (48-88u), bucketed by the reason
    /// string the Shoot event carries. Mirror of `APPROVED_BY_TAG` — it
    /// names the producer of the close-range over-supply that no
    /// decision-layer gate reaches.
    pub const ETAGS: usize = 10;
    pub static EMITTED_MID_BAND: [AtomicU64; ETAGS] = [ZERO_ONE; ETAGS];
    /// Same, for the <6m band. Added because the aggregate share of shots
    /// from inside six metres cannot tell a tap-in off a cross — which is
    /// real football, ~15% of all shots — from a forward who dribbled into
    /// the goalkeeper, which is not. Two fixes were aimed at that band on
    /// the strength of the share alone and neither could be evaluated.
    pub static EMITTED_CLOSE_BAND: [AtomicU64; ETAGS] = [ZERO_ONE; ETAGS];
    pub const ETAG_NAMES: [&str; ETAGS] = [
        "header",
        "snapshot",
        "MID_CLEAR_CHANCE",
        "finishing",
        "distance-shoot",
        "mid-shooting-*",
        "fwd-shooting-*",
        "helper FWD",
        "helper MID",
        "other",
    ];
    #[inline]
    pub fn emit_tag_index(r: &str) -> usize {
        if r.contains("HEAD") {
            0
        } else if r.contains("SNAPSHOT") {
            1
        } else if r == "MID_CLEAR_CHANCE" {
            2
        } else if r.contains("FINISH") {
            3
        } else if r.contains("DISTANCE") {
            4
        } else if r.starts_with("MID_SHOOTING") {
            5
        } else if r.starts_with("FWD_SHOOTING") {
            6
        } else if r.starts_with("FWD_") {
            7
        } else if r.starts_with("MID_") || r.starts_with("AM_") {
            8
        } else {
            9
        }
    }
    pub fn emit_tag_snapshot() -> [u64; ETAGS] {
        let mut out = [0u64; ETAGS];
        for i in 0..ETAGS {
            out[i] = EMITTED_MID_BAND[i].load(Ordering::Relaxed);
        }
        out
    }
    /// Reason breakdown for shots struck from inside six metres.
    pub fn close_tag_snapshot() -> [u64; ETAGS] {
        let mut out = [0u64; ETAGS];
        for i in 0..ETAGS {
            out[i] = EMITTED_CLOSE_BAND[i].load(Ordering::Relaxed);
        }
        out
    }

    pub fn tag_snapshot() -> [u64; TAGS] {
        let mut out = [0u64; TAGS];
        for i in 0..TAGS {
            out[i] = APPROVED_BY_TAG[i].load(Ordering::Relaxed);
        }
        out
    }
    /// Ticks the ball was OWNED by a player, bucketed by that owner's
    /// distance to the goal he is attacking. This is possession
    /// geography itself — the ground truth behind the shot mix. Real
    /// football spends most of its possession outside 22m; if this
    /// histogram is bottom-heavy the shot mix cannot be fixed by any
    /// shot-side dial.
    pub static POSSESSION_TICKS_BY_DIST: [AtomicU64; BANDS] = [ZERO; BANDS];

    /// Per-distance-band sums (x1000) of each multiplicative factor in
    /// the willingness product, so the distance-correlated suppressor
    /// can be READ rather than guessed at. Order:
    /// 0 urge, 1 reach, 2 angle_quality, 3 lane, 4 poise, 5 boldness,
    /// 6 situational, 7 psychology, 8 final appetite,
    /// 9 pressure_clarity, 10 corridor_clarity, 11 threshold.
    ///
    /// The last three are the ones that make this table decidable.
    /// `lane` is a PRODUCT of the first two, and the two halves argue in
    /// opposite directions with distance — immediate pressure is worst in
    /// the six-yard box, corridor obstruction is worst from range — so a
    /// single number for it cannot say which is suppressing a band.
    /// `threshold` is the bar the appetite is actually being compared
    /// against: without it the table shows an appetite with nothing to
    /// read it against, and the appetite-vs-bar GAP is the whole
    /// question.
    pub const WFACTORS: usize = 12;
    const ZERO_BAND: [AtomicU64; BANDS] = [ZERO; BANDS];
    pub static WILL_FACTOR_SUM: [[AtomicU64; BANDS]; WFACTORS] = [ZERO_BAND; WFACTORS];

    /// Why a shot decision was rejected, PER DISTANCE BAND. Aggregate
    /// waterfall totals hide band-specific blockers: a reason that is
    /// 8% of all rejections can still be 90% of them at 22-30m.
    /// Order: 0 far, 1 min_xg, 2 inside_six_xg, 3 no_clear, 4 pass_defer.
    pub const REASONS: usize = 5;
    pub static REJECT_BY_DIST: [[AtomicU64; BANDS]; REASONS] = [ZERO_BAND; REASONS];

    #[inline]
    pub fn record_reject(reason: usize, distance: f32) {
        REJECT_BY_DIST[reason][band_for_distance(distance)].fetch_add(1, Ordering::Relaxed);
    }

    pub fn reject_snapshot() -> [[u64; BANDS]; REASONS] {
        let mut out = [[0u64; BANDS]; REASONS];
        for r in 0..REASONS {
            for b in 0..BANDS {
                out[r][b] = REJECT_BY_DIST[r][b].load(Ordering::Relaxed);
            }
        }
        out
    }

    /// Record one sample of the willingness factor vector.
    #[inline]
    pub fn record_will_factors(band: usize, f: [f32; WFACTORS]) {
        for (i, v) in f.iter().enumerate() {
            // x1e6: willingness values run ~1e-4, so a x1000 scale
            // truncated them to zero and made weak-team factor tables
            // unreadable.
            WILL_FACTOR_SUM[i][band].fetch_add((v * 1_000_000.0) as u64, Ordering::Relaxed);
        }
    }

    /// Mean of each factor per band (divide by the band's roll count).
    pub fn will_factor_snapshot() -> [[u64; BANDS]; WFACTORS] {
        let mut out = [[0u64; BANDS]; WFACTORS];
        for i in 0..WFACTORS {
            for b in 0..BANDS {
                out[i][b] = WILL_FACTOR_SUM[i][b].load(Ordering::Relaxed);
            }
        }
        out
    }

    /// Distance band for a goal-distance in field units.
    #[inline]
    pub fn band_for_distance(units: f32) -> usize {
        if units < 48.0 {
            0
        } else if units < 88.0 {
            1
        } else if units < 132.0 {
            2
        } else if units < 176.0 {
            3
        } else if units < 240.0 {
            4
        } else {
            5
        }
    }

    /// [shots, xg, rolls, calls, possession, approved, queued_lost] per band.
    pub fn distance_snapshot() -> [[u64; BANDS]; 7] {
        let load = |arr: &[AtomicU64; BANDS]| {
            let mut out = [0u64; BANDS];
            for (o, a) in out.iter_mut().zip(arr.iter()) {
                *o = a.load(Ordering::Relaxed);
            }
            out
        };
        [
            load(&SHOTS_BY_DIST),
            load(&XG_X1000_BY_DIST),
            load(&ROLLS_BY_DIST),
            load(&CALLS_BY_DIST),
            load(&POSSESSION_TICKS_BY_DIST),
            load(&APPROVED_BY_DIST),
            load(&QUEUED_SHOT_LOST),
        ]
    }
    /// Condition samples per band per position group (0=GK 1=DEF 2=MID
    /// 3=FWD): summed condition (0..10000) and sample count. Sampled at
    /// a coarse cadence from the engine loop so the harness can print
    /// the average condition trajectory by role — the suspected driver
    /// of the early-match attack-volume decay.
    const ZERO_ROW: [AtomicU64; 4] = [ZERO; 4];
    pub static COND_SUM_BY_BAND_GROUP: [[AtomicU64; 4]; BANDS] = [ZERO_ROW; BANDS];
    pub static COND_N_BY_BAND_GROUP: [[AtomicU64; 4]; BANDS] = [ZERO_ROW; BANDS];
    /// Outfield velocity-band occupancy from the condition processor:
    /// 0=stationary(<5% max speed) 1=walking(5-30%) 2=jogging(30-60%)
    /// 3=running(60-85%) 4=sprinting(>85%). The fatigue calibration is
    /// a function of this distribution — net drain per tick =
    /// Σ band_share × band_rate — so the harness prints it to make
    /// drain/recovery retuning analytic instead of trial-and-error.
    pub static VELOCITY_BAND_TICKS: [AtomicU64; 5] = [ZERO; 5];

    pub fn band_for_minute(minute: u32) -> usize {
        ((minute / 15) as usize).min(BANDS - 1)
    }

    pub fn reset() {
        for a in APPROVED_BY_TAG
            .iter()
            .chain(EMITTED_MID_BAND.iter())
            .chain(EMITTED_CLOSE_BAND.iter())
        {
            a.store(0, Ordering::Relaxed);
        }
        for arr in WILL_FACTOR_SUM
            .iter()
            .chain(REJECT_BY_DIST.iter())
            .chain(SHOTS_BY_POS_DIST.iter())
        {
            for a in arr.iter() {
                a.store(0, Ordering::Relaxed);
            }
        }
        for arr in [
            &SHOTS_BY_BAND,
            &ON_TARGET_BY_BAND,
            &XG_X1000_BY_BAND,
            &GOALS_BY_BAND,
            &ROLL_REACHED_BY_BAND,
            &SHOTS_BY_DIST,
            &XG_X1000_BY_DIST,
            &ROLLS_BY_DIST,
            &CALLS_BY_DIST,
            &APPROVED_BY_DIST,
            &QUEUED_SHOT_LOST,
            &POSSESSION_TICKS_BY_DIST,
        ] {
            for a in arr.iter() {
                a.store(0, Ordering::Relaxed);
            }
        }
        for band in 0..BANDS {
            for g in 0..4 {
                COND_SUM_BY_BAND_GROUP[band][g].store(0, Ordering::Relaxed);
                COND_N_BY_BAND_GROUP[band][g].store(0, Ordering::Relaxed);
            }
        }
        for a in VELOCITY_BAND_TICKS.iter() {
            a.store(0, Ordering::Relaxed);
        }
    }

    pub fn velocity_band_snapshot() -> [u64; 5] {
        let mut out = [0u64; 5];
        for (o, a) in out.iter_mut().zip(VELOCITY_BAND_TICKS.iter()) {
            *o = a.load(Ordering::Relaxed);
        }
        out
    }

    /// (avg_condition_pct, n) per band per group.
    pub fn condition_snapshot() -> [[(f64, u64); 4]; BANDS] {
        let mut out = [[(0.0, 0u64); 4]; BANDS];
        for band in 0..BANDS {
            for g in 0..4 {
                let n = COND_N_BY_BAND_GROUP[band][g].load(Ordering::Relaxed);
                let sum = COND_SUM_BY_BAND_GROUP[band][g].load(Ordering::Relaxed);
                out[band][g] = (
                    if n > 0 {
                        sum as f64 / n as f64 / 100.0
                    } else {
                        0.0
                    },
                    n,
                );
            }
        }
        out
    }

    /// [shots, on_target, xg_x1000, goals, roll_reached] per band.
    pub fn snapshot() -> [[u64; BANDS]; 5] {
        let load = |arr: &[AtomicU64; BANDS]| {
            let mut out = [0u64; BANDS];
            for (o, a) in out.iter_mut().zip(arr.iter()) {
                *o = a.load(Ordering::Relaxed);
            }
            out
        };
        [
            load(&SHOTS_BY_BAND),
            load(&ON_TARGET_BY_BAND),
            load(&XG_X1000_BY_BAND),
            load(&GOALS_BY_BAND),
            load(&ROLL_REACHED_BY_BAND),
        ]
    }
}

/// Outcome of `evaluate_forward_shot_decision`.
///
/// Centralised so every forward state (Running, RunningInBehind,
/// Finishing, Shooting) consults the same gate-stack: cooldown,
/// xG quality, clear-shot lane, sprint/balance, GK proximity, and
/// pass-vs-shot expected value. Before this helper, RunningInBehind
/// and Finishing transitioned to Shooting on a raw distance check
/// alone, allowing a sprinting forward with no balance to fire any
/// time the ball ended up in their feet under 80u — which is how
/// a Finishing-10 striker racked up 1.7 goals/match.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ShotDecision {
    /// Conditions met — fire now. `reason` mirrors the pass-reason
    /// pattern so the per-shot log shows which gate let the strike
    /// through.
    Shoot { reason: &'static str },
    /// Ball is in shooting range but the player should pass / cutback
    /// instead. Caller routes to Passing.
    Pass,
    /// Conditions failed in a way that doesn't justify burning the
    /// possession on a pass — keep dribbling / running so a real
    /// chance can materialise next tick.
    Hold,
}

/// How far a given player can strike a ball, and the distances that
/// follow from that.
///
/// Grouped because they are one idea measured three ways, and because
/// separating them is how the movement code and the shooting code came to
/// disagree about where a shooting position is.
/// How far the shot-decision bar eases at the very edge of a player's
/// striking range. See the note at the threshold for why the bar moves
/// with distance at all.
/// Raised 0.28 → 0.40 now that the relief is GATED on the lane and the
/// player's own range (`long_shot_licence`). Un-gated it had to be sized
/// for the worst long shot in the game, which left the best one — a
/// striker of the ball with a clear sight from 25 m — unable to clear the
/// bar either. Gated, it can describe the shot it is actually for.
/// Bar units of relief for a SPECULATIVE effort — beyond the edge of the
/// box, gated on `long_shot_licence` and decaying with distance.
/// 0.06 → 0.12 → 0.08. The higher base bar crushed the normal shooting
/// band hardest and this hump is what holds 11-22 m open, but 0.12 put
/// 41% of all shots into 16.5-22 m alone (real 20%). That band sits
/// right on the bar, so it is the most sensitive place on the curve —
/// a 0.06 change in relief swings it by 30 points of share. Move it in
/// steps of 0.02 and read the mix, never the total.
/// Now the WHOLE speculative relief, at full speculation — the
/// ability-gated `SPECIALIST_RELIEF` half is gone (see `range_ease`), so
/// this carries both. Everyone gets it; who actually shoots from there
/// is settled by `reach`, `boldness` and `discernment` in the appetite.
const LONG_RANGE_RELIEF: f32 = 0.48;
/// The bar never falls below this, so a hopeful from 40 m is still a
/// decision and not a reflex.
///
/// Lowered 0.26 → 0.20 because this floor is not only a long-range floor:
/// the close-in relief feeds the same `range_ease`, so it was also
/// capping how far the bar could fall for a striker inside the six-yard
/// box — the one look in football that should be close to automatic.
const LONG_RANGE_FLOOR: f32 = 0.20;
/// Height of the shot bar before the per-opportunity spread and the two
/// distance reliefs. This is the engine's shot-volume knob — see the note
/// at the threshold.
/// Raised 0.530 → 0.575 now that the DEFENCE is doing its share.
///
/// The bar was left deliberately low through the shooting work on the
/// standing instruction that shot volume must be restrained by better
/// defending rather than by suppressing the decision. That side is now
/// done — 16.5% of shots blocked against a real 18-22%, clearances 4.2
/// per defender against ~3.5, tackles 1.75 against ~1.6 — and volume is
/// still 21.7 shots a team against a real 13, so the remainder is the
/// bar's to carry after all.
///
/// Move it in small steps and re-read the whole distance mix, never just
/// the total: the reliefs subtract from this, so a uniform lift falls
/// entirely on the un-eased middle of the pitch.
const SHOT_BAR_BASE: f32 = 0.900;
/// How much of the urge a man stretching at full tilt for a ball he has
/// not got under control gives up. See `poise`.
const STRETCH_COST: f32 = 0.45;
/// How much of the urge survives a corridor that is completely blocked
/// by outfield bodies. Not zero: a shot into traffic is a real shot, and
/// whether the body gets in the way is largely `try_block_shot`'s
/// question rather than the decision's. See `corridor`.
///
/// 0.60 was sized on the assumption that the block model absorbs the
/// traffic. Measured, it does not: `try_block_shot` stops 8.9% of shots
/// against a real 18-22%, and defenders sit in the lane on only ~9.6% of
/// in-window samples. Until the defensive shape puts bodies where they
/// belong, the decision has to carry more of it — and this is the term
/// that discriminates, because corridor clarity runs 0.37 in the
/// six-yard box against 0.94 from thirty metres. Dropping the floor
/// therefore bites almost entirely on point-blank volume and leaves the
/// long-range band alone, which is the exact axis that needed moving.
const CORRIDOR_FLOOR: f32 = 0.15;
/// Exponent on `angle_clarity` — how strongly the SIZE of the visible
/// goal drives the urge, as against merely being central
/// (`angle_quality`). This is the decision's distance spine; there is no
/// other one inside the player's comfortable range, where `reach` is
/// flat at 1.0.
///
/// Shallow on purpose. `angle_clarity` spans 0.93 at the six-yard box
/// down to 0.15 beyond 30 m — raw, it would flatten long shots out of
/// the game altogether, which is the failure this whole term exists to
/// undo. At 0.30 the same span becomes 0.98 → 0.58: a real preference
/// for the closer look, not a veto on the far one.
const TARGET_SIZE_WEIGHT: f32 = 0.22;
/// How much of the full relief a point-blank chance gets.
///
/// Sized against shot QUALITY, not shot count. At 1.0 the close-range
/// band filled with marginal looks: population xG/shot fell to 0.083
/// against a real 0.11, on-target→goal to 24.3% against 30%, and the
/// 6-11 m band took 32% of all shots against a real 25%. The bar should
/// ease enough that a clear chance is always taken and not so much that a
/// half-chance is.
/// Raised 0.30 → 0.70. The note above is about SHOT QUALITY and stands,
/// but 0.30 of a 0.28 relief is 0.084 of bar against a measured shortfall
/// of ~0.25 between a point-blank appetite and the bar — a rounding
/// error dressed as a fix, and the reason a striker five metres out still
/// declined. The cubed falloff is what keeps this honest: it is nearly
/// gone by the edge of the comfortable range, so it lifts the tap-in
/// without lifting the eleven-metre half-chance the earlier linear
/// version did.
/// Trimmed 0.28 → 0.17 alongside the base-bar lift. The reliefs subtract
/// from `SHOT_BAR_BASE`, so raising the base while leaving these alone
/// falls entirely on the bands that have no relief: at 0.575/0.28 the
/// total landed on 12.7 shots (real 13) but <6 m took 34.6% of them
/// (real 15%) and 11-22 m collapsed to 21% (real 42%) — forwards back to
/// hunting tap-ins. The point of the close relief is that a striker in
/// the six-yard box shoots, not that he only shoots there.
/// Trimmed 0.22 → 0.16 now that `target_size` gives the APPETITE its own
/// close-in gradient. Before that the bar was the only thing that knew a
/// five-metre chance is different from an eighteen-metre one; with both
/// carrying it the six-yard box was being paid twice.
const CLOSE_RANGE_RELIEF: f32 = 0.45;
/// Bar units of relief at the peak of the normal-shooting band (~14 m).
/// See the note at `box_relief`.
/// How much a clean run at goal (nothing but the keeper left) lifts the
/// urge. See `clear_run` — `GoalSight::cover` was previously unused.
const THROUGH_ON_GOAL_LIFT: f32 = 0.45;
/// Distance over which the close-range bar relief decays, in units
/// (100u = 12.5 m). An absolute span: point blank is a property of the
/// pitch, not of the player. See `close_ease`.
const POINT_BLANK_SPAN: f32 = 100.0;
const BOX_RELIEF: f32 = 0.22;
/// How much further from goal than the carrier a lay-off target may be
/// and still count as a release rather than a recycle. 20u = 2.5 m — a
/// square ball, not a pass back into midfield. See the release clause.
const RELEASE_BACKWARD_TOLERANCE: f32 = 20.0;

pub struct StrikingRange;

impl StrikingRange {
    /// Fraction of a player's range inside which he is simply "in a
    /// position to shoot" — no keener at eight metres than at eighteen.
    const COMFORTABLE: f32 = 0.45;
    /// Fraction of the comfortable distance a CARRIER will drive to.
    const CARRY_HOLD: f32 = 0.72;

    /// How far this player can strike a ball with something on it, in game
    /// units (1u = 0.125 m).
    ///
    /// His own property, not a league-wide expectation: a centre-half with
    /// a hammer is live from 30 m and a poacher is not live from 20. 200u
    /// = 25 m at the bottom of the range, 390u = 49 m for a specialist who
    /// genuinely tries them from there.
    pub fn of(ctx: &StateProcessingContext) -> f32 {
        let minute = sc::minute_from_ms(ctx.context.total_match_time);
        let tech = sc::EffActionContext::technical(minute);
        let power = (sc::n(sc::eff(ctx.player, tech, |p| p.skills.technical.long_shots)) * 0.45
            + sc::n(sc::eff(ctx.player, tech, |p| p.skills.technical.technique)) * 0.30
            + (ctx.player.skills.physical.strength / 20.0).clamp(0.0, 1.0) * 0.25)
            .clamp(0.0, 1.0);
        200.0 + power * 190.0
    }

    /// Inside this distance the player is already somewhere he could
    /// strike from, and has no footballing reason to carry the ball
    /// closer.
    pub fn comfortable(ctx: &StateProcessingContext) -> f32 {
        Self::of(ctx) * Self::COMFORTABLE
    }

    /// The nearest a ball-CARRIER will drive toward goal, in game units —
    /// about 12 m for an average player.
    ///
    /// A striker clean through does take it on a few yards past the point
    /// he could first have hit it, which is why this sits inside
    /// [`Self::comfortable`] rather than on top of it. But he finishes
    /// from around twelve metres; he does not dribble into the
    /// goalkeeper's shins and poke it at him from two feet. Nothing
    /// stopped him doing precisely that, because every carrying target in
    /// the engine was the goal centre — which is where the keeper stands.
    ///
    /// Tap-ins from inside this radius still happen in the numbers: those
    /// come off crosses, rebounds and cut-backs, where the forward is
    /// arriving onto a ball rather than carrying it in.
    pub fn carry_hold(ctx: &StateProcessingContext) -> f32 {
        Self::comfortable(ctx) * Self::CARRY_HOLD
    }
}

/// How settled a player is over the ball.
pub struct Poise;

impl Poise {
    /// What fraction of his own top speed he is travelling at, 0..1.
    ///
    /// Scaled against his CURRENT top speed rather than a league-wide
    /// one, so a tiring player is not read as composed merely because he
    /// can no longer run.
    pub fn pace(ctx: &StateProcessingContext) -> f32 {
        let top = ctx.player.max_speed_with_condition_cached().max(0.01);
        (ctx.player.velocity.norm() / top).clamp(0.0, 1.0)
    }
}

/// Where a player carrying the ball is actually going.
pub struct BallCarry;

impl BallCarry {
    /// How far to either side the carrier steps to go round his man.
    const SIDESTEP: f32 = 26.0;
    /// Cap on how far ahead the carry target is placed, so `Arrive`
    /// produces a run rather than a teleport-chase.
    const MAX_ADVANCE: f32 = 70.0;
    /// How far in front a defender still counts as being in the way.
    const BLOCKER_SCAN: f32 = 45.0;

    /// The point a carrying forward is running at.
    ///
    /// **Not the goal.** Both carrying states used to `Arrive` at the goal
    /// centre, which is literally an instruction to run at the
    /// goalkeeper, and that is exactly what it produced on the pitch. Two
    /// things are wrong with it:
    ///
    /// * A man beating a defender goes at the space past his shoulder,
    ///   not through him.
    /// * Once he is somewhere he could strike from, closing further makes
    ///   the chance WORSE — the angle narrows and the keeper closes it
    ///   down. There is no version of football in which the answer to "I
    ///   have the ball 18 metres out" is "get nearer the goalkeeper".
    ///
    /// So: advance while outside a shooting position, work across the face
    /// of the defence once inside one, and go round the nearest defender
    /// rather than into him.
    pub fn target(ctx: &StateProcessingContext) -> Vector3<f32> {
        let me = ctx.player.position;
        let goal = ctx.player().opponent_goal_position();
        let to_goal = goal - me;
        let distance = to_goal.magnitude().max(1.0);
        let forward = to_goal / distance;
        let lateral = Vector3::new(-forward.y, forward.x, 0.0);

        // Round the nearest defender ahead of us, on the side he is not.
        let mut blocker_distance = f32::MAX;
        let mut sidestep = 0.0_f32;
        for opponent in ctx.players().opponents().nearby(Self::BLOCKER_SCAN) {
            if opponent.tactical_positions.is_goalkeeper() {
                continue;
            }
            let relative = opponent.position - me;
            // Only someone actually in front of us is in the way.
            if relative.dot(&forward) <= 0.0 {
                continue;
            }
            let d = relative.magnitude();
            if d < blocker_distance {
                blocker_distance = d;
                // A square-on defender has a lateral offset near zero, so
                // a bare signum flips as the carrier jitters: the target
                // jumps from 26u left to 26u right and back, which is a
                // limit cycle with the ball in it. Gating it with a 1 m
                // dead zone and deferring to the centre drift was tried
                // and REVERTED — square on is the common case in front of
                // goal, so carriers stopped dead in front of their man
                // instead of going round him and the ball going nowhere
                // went 178 -> 449 s a match. The requirement recorded
                // there was that whatever replaces it has to keep him
                // COMMITTING to a side.
                //
                // ⚠ A COMMITTED side was then tried — 1 m dead zone, the
                // sign inside it drawn once per (possession, defender),
                // the same device the shot threshold uses — and it is
                // ALSO worse: `Forward: Running` stuck time 10.5 → 18.5 s
                // a match, total ball-stuck 49 → 99 s. Committing to a
                // side the geometry disagrees with walks the carrier into
                // his man. Two attempts, two regressions: leave the
                // signum alone. The freeze it was suspected of is on the
                // CALLER's side — see `settle_over_the_ball` and the
                // box-hold patience cap in `forwarders/states/running`.
                sidestep = if relative.dot(&lateral) > 0.0 {
                    -1.0
                } else {
                    1.0
                };
            }
        }
        if sidestep == 0.0 {
            // Nobody in front: drift toward the middle, where the angle is
            // better — the winger cutting inside.
            let to_centre = goal.y - me.y;
            if to_centre.abs() > 1.0 {
                sidestep = (to_centre * lateral.y).signum();
            }
        }

        // Stop closing once he is in a position to shoot from. A soft stop
        // on THIS target only — a geometric wall applied to every carrying
        // path was tried and reverted: it parked every carrier on the wall
        // and 54% of all shots came from one five-metre band, which the
        // keeper then saved 94% of because he was set for every single
        // one. What actually stops a man dribbling into the six-yard box
        // is the goalkeeper coming out to meet him, and that belongs in
        // the goalkeeper (see `GoalkeeperStandingState::should_rush_out_for_ball`).
        let advance = (distance - StrikingRange::carry_hold(ctx)).clamp(0.0, Self::MAX_ADVANCE);

        let mut target = me + forward * advance + lateral * (sidestep * Self::SIDESTEP);

        // The drift to the middle must CONVERGE, not oscillate.
        //
        // `sidestep` is a signum, and the branch that sets it from
        // `to_centre` flips the instant the carrier crosses the middle of
        // the pitch. The target is also a fixed offset from `me`, so it
        // slides sideways exactly as fast as he does and he never
        // arrives. Together that is a treadmill with a sign flip on the
        // end of it: he strafes towards the centre, crosses it, reverses,
        // crosses back. It is the same shape as every other flicker in
        // this engine — a discrete switch on a saturated steering output.
        //
        // Measured with `dead_ball_diag`: a forward holding the ball
        // inside a 15u (1.9 m) circle for a mean of 12 s, 12.7 times a
        // match — 156 s a match, the largest single source of the ball
        // going nowhere.
        //
        // Clamping the target between where he stands and the middle
        // makes it converge: he works across the face of the defence and
        // stops once he is central, which is what this is for. Only the
        // no-blocker branch needs it — stepping past a man's shoulder is
        // legitimately relative, and it terminates when he beats him.
        if blocker_distance == f32::MAX {
            let lo = me.y.min(goal.y);
            let hi = me.y.max(goal.y);
            target.y = target.y.clamp(lo, hi);
        }
        target
    }
}

/// Does this player hit it?
///
/// # What a footballer actually reads
///
/// Sight of goal, whether it is within his range, whether he is set,
/// whether he is being closed down, and whether someone else is better
/// placed. All five are things a man on the pitch perceives. None of them
/// is a statistic.
///
/// This replaced a model built the other way round. The old decision was
/// a **per-tick lottery**: `base_willingness` ran 0.0006-0.0014 per tick
/// and was multiplied by xG-derived factors, so a player did not decide to
/// shoot — he rolled a hundred dice a second until one came up. Measured
/// on the dev harness: 617,700 evaluations produced 1,085 shots, 0.18%.
///
/// Two things followed, and both were visible on the pitch:
///
/// * **Whether a chance became a shot depended on how long he loitered in
///   range**, not on the chance. A forward who received the ball 20 m out
///   with the goal in front of him would, on the balance of probability,
///   not shoot — he would hold it and re-roll. The only way to accumulate
///   rolls was to keep the ball, and the place the ball drifts while a
///   forward keeps it is toward the goal. 31% of all shots came from
///   inside six metres against a real 15%; players ran at the keeper.
/// * **xG was the spine of the decision** — a floor, a willingness
///   multiplier, a quality pull, a marginal gate. xG is an analyst's
///   summary of what happened to thousands of past shots; it is not
///   available to a man deciding whether to hit one. Worse, the same
///   model scored the shot afterwards, so the engine graded its own
///   homework and no amount of retuning converged. The file's own history
///   records four rounds of it.
///
/// Randomness now lives in the EXECUTION, where a footballer's
/// uncertainty actually lives, not in whether he tries.
///
/// # One opportunity, one decision
///
/// The answer is deterministic given the situation, so the same look gets
/// the same answer for as long as it lasts. Appetite is compared against a
/// threshold drawn once per POSSESSION (seeded from the player and the
/// tick he gained the ball), which is what stops "ask again next tick"
/// from being a strategy. If he decides against, he does not re-roll — he
/// looks for the better-placed team-mate, or carries until the situation
/// itself changes.
///
/// `tag` is the reason string attached to the resulting Shoot event.
/// Keep it stable per call-site so the per-match shot log stays
/// readable.
pub fn evaluate_forward_shot_decision(
    ctx: &StateProcessingContext,
    tag: &'static str,
) -> ShotDecision {
    #[cfg(feature = "match-logs")]
    {
        use std::sync::atomic::Ordering;
        helper_diag::CALLS.fetch_add(1, Ordering::Relaxed);
    }
    // ── Hard gates ────────────────────────────────────────────────────
    let can_team = ctx.team().can_shoot();
    let can_player = ctx.player().can_shoot();
    if !can_team || !can_player {
        #[cfg(feature = "match-logs")]
        helper_diag::HOLD_HARDGATE.fetch_add(1, Ordering::Relaxed);
        return ShotDecision::Hold;
    }

    let distance = ctx.ball().distance_to_opponent_goal();
    #[cfg(feature = "match-logs")]
    time_band_diag::CALLS_BY_DIST[time_band_diag::band_for_distance(distance)]
        .fetch_add(1, Ordering::Relaxed);
    // Anything beyond the absolute long-range cap is hopeless even
    // for elite long-shooters — keep the ball.
    if distance > 320.0 {
        #[cfg(feature = "match-logs")]
        helper_diag::HOLD_FAR.fetch_add(1, Ordering::Relaxed);
        #[cfg(feature = "match-logs")]
        time_band_diag::record_reject(0, distance);
        return ShotDecision::Hold;
    }

    let skills = &ctx.player.skills;
    let minute = sc::minute_from_ms(ctx.context.total_match_time);
    // Unified shot profile — single source of truth for execution_skill,
    // selection_skill, body_control, poor_penalty, etc. The
    // `shooting().shot_profile()` helper builds this from the same
    // inputs `handle_shoot_event` will see in-flight, so the gate and
    // the strike agree on what the shooter can actually do.
    let shooting_ops = ctx.player().shooting();
    let profile = shooting_ops.shot_profile();
    let selection = profile.selection_skill;
    let execution_skill = profile.execution_skill;
    let composure_skill = profile.composure_skill;
    let body_control = profile.body_control;
    let _poor_penalty = profile.poor_penalty;
    let pressure_penalty = profile.pressure_penalty;
    let low_condition_penalty = profile.low_condition_penalty;

    let tech = sc::EffActionContext::technical(minute);
    let mental = sc::EffActionContext::mental(minute);
    // A few raw-band reads still drive 1v1 cool-headedness; routed
    // through effective_skill so fatigue applies.
    let _finishing = sc::n(sc::eff(ctx.player, tech, |p| p.skills.technical.finishing));
    let composure = sc::n(sc::eff(ctx.player, mental, |p| p.skills.mental.composure));
    let _technique = (skills.technical.technique / 20.0).clamp(0.0, 1.0);
    let first_touch = sc::n(sc::eff(ctx.player, tech, |p| {
        p.skills.technical.first_touch
    }));
    let decisions = sc::n(sc::eff(ctx.player, mental, |p| p.skills.mental.decisions));

    // ── Sight of goal ─────────────────────────────────────────────────
    // Geometry and bodies. `angle_quality` is how central he is FOR HIS
    // DISTANCE, `lane` is what stands between the ball and the net.
    // Distance itself is handled by `reach` below, so it is not counted
    // twice — counting it twice is what made every good look a close one.
    let sight = ctx.player().goal_sight();
    if sight.lane <= 0.0 || sight.angle_quality <= 0.0 {
        // Blind angle, or a defender physically in front of the ball.
        #[cfg(feature = "match-logs")]
        helper_diag::HOLD_NO_CLEAR.fetch_add(1, Ordering::Relaxed);
        #[cfg(feature = "match-logs")]
        time_band_diag::record_reject(3, distance);
        return ShotDecision::Hold;
    }

    // ── Is it within HIS range ────────────────────────────────────────
    // How far this player can strike a ball with something on it, in game
    // units (1u = 0.125 m). His own property: a centre-half with a hammer
    // is live from 30 m, a poacher is not live from 20. Nothing here is a
    // league-wide expectation — it is the leg and the technique.
    let striking_range = StrikingRange::of(ctx);
    // Flat inside his comfortable range, then a fade to the edge of it.
    //
    // NOT a decline from zero distance, which is the shape a first cut
    // used and which is wrong about footballers: a striker is no keener
    // from eight metres than from eighteen — both are simply "in range" —
    // and it is thirty where he starts to think about it. Declining from
    // zero made raw distance the dominant term in the decision, so the
    // only looks that cleared the bar were the closest ones and 73% of
    // all shots came from inside six metres.
    //
    // With it flat, what separates one look from another is the QUALITY
    // of the look — the angle and what is in the way — which is what
    // separates them on a pitch.
    let comfortable = striking_range * StrikingRange::COMFORTABLE;
    let reach = if distance <= comfortable {
        1.0
    } else {
        let past = (distance - comfortable) / (striking_range - comfortable).max(1.0);
        (1.0 - past.clamp(0.0, 1.0).powf(1.6)).clamp(0.0, 1.0)
    };

    // ── Is he set? ────────────────────────────────────────────────────
    // A man stretching for a bouncing ball at full tilt does not pick his
    // corner.
    //
    // This read HOW LONG HE HAD BEEN IN THE STATE, which is not his speed
    // and frequently not even correlated with it. A forward stood still
    // with the ball at his feet for a second and a quarter scored a full
    // sprint penalty; so did every striker who had been running for a
    // second and a quarter, which is every striker through on goal.
    // Measured across the dev harness the term was saturated at 1.0
    // essentially always — `poise` came out flat at 0.42 in every
    // distance band from six metres to thirty — so it was not modelling
    // being off-balance at all. It was subtracting a constant from the
    // urge of every shot in the game.
    // …and it was doing it again. Measured across every distance band:
    // 0.485 / 0.415 / 0.377 / 0.365 / 0.366 / 0.366. A term that reads
    // the same from the six-yard box as from thirty-five metres is not
    // telling the decision anything; entered as `0.45 + poise * 0.55` it
    // was a flat 33% haircut on the urge of every shot in the game — the
    // third of four such haircuts (see `boldness`, `situational`).
    //
    // The reason is `physical_balance`: a mean of four normalised skills,
    // which for any real player lands near 0.7 whatever those skills are,
    // multiplied by two sub-1.0 factors. It described the population, not
    // the moment.
    //
    // So invert the shape. Composure over the ball is NEUTRAL — a player
    // in normal control of it strikes as he means to — and what the model
    // owes the decision is the cost of NOT being in control: a man at
    // full tilt with the ball running away from him. That is `sprinting`
    // against `body_control`, which is what "stretching for it" means,
    // and it is a term that varies with the moment instead of with the
    // roster.
    let sprinting = Poise::pace(ctx);
    let physical_balance = (skills.physical.strength / 20.0
        + skills.physical.agility / 20.0
        + first_touch
        + composure)
        / 4.0;
    let stretched = (sprinting * (1.0 - body_control)).clamp(0.0, 1.0);
    let poise = ((1.0 - stretched * STRETCH_COST) * (0.80 + physical_balance * 0.20))
        .clamp(0.0, 1.0);

    // ── Being closed down ─────────────────────────────────────────────
    //
    // A man about to be tackled hits it now or loses it. This is the
    // ONLY place immediate pressure enters the decision, and that is the
    // correction: it used to be counted twice and in both directions —
    // here as a small bonus, and inside `sight.lane` as a large penalty —
    // so the net effect of being closed down was that the player DIDN'T
    // shoot. Football says the opposite, and the engine already resolves
    // what a defender at your shoulder actually does to the strike, twice
    // over: `pressure_penalty` degrades the accuracy of it in
    // `ShotSkillProfile`, and `try_block_shot` puts a body in front of it
    // in flight.
    let pressure = pressure_penalty
        .max(1.0 - sight.pressure_clarity)
        .clamp(0.0, 1.0);

    // ── The urge ──────────────────────────────────────────────────────
    //
    // How big the goal looks, how central he is, what is in the way, can
    // he reach it, is he balanced, and is he about to be tackled.
    //
    // `angle_clarity` — the size of the opening in ABSOLUTE terms — was
    // deliberately excluded here, on the grounds that distance is
    // "handled by `reach` below, so it is not counted twice". But `reach`
    // is flat at 1.0 everywhere inside the player's comfortable range,
    // which for an average player is 17.7 m — so distance was handled
    // NOWHERE inside 17.7 m, and the only term left with any range in it
    // was the corridor, which gets WORSE the closer you are to goal.
    //
    // The engine's appetite therefore rose with distance out to the edge
    // of the box: measured 0.222 at <6 m against 0.382 at 16.5-22 m. It
    // was least willing to shoot in the six-yard box, which is the
    // reported behaviour exactly — a striker who runs into the area and
    // then squares it.
    //
    // `angle_clarity` fixes it with something the player actually
    // perceives rather than a statistic: 2·atan(29/d) is how much of the
    // net is in front of him, and it falls away with range on its own.
    // No xG is consulted (see the standing rule) — this is the size of
    // the target, which is the thing a footballer is looking at.
    let target_size = sight.angle_clarity.clamp(0.0, 1.0).powf(TARGET_SIZE_WEIGHT);
    // Bodies in the corridor are a DISCOUNT, not a veto.
    //
    // A shot through traffic is a worse choice than a shot at an open
    // net, and the model should say so — but as `lane^0.55` it said far
    // more than that: corridor clarity measures 0.380 at <6 m against
    // 0.935 beyond 30 m, so the term swung the appetite by 2.5× in the
    // direction of shooting from range. Whether the body actually gets in
    // the way is `try_block_shot`'s question and it already answers it on
    // ~15% of shots; charging the same defenders again here is a
    // double-count, and it is the one that made the box the least
    // attractive place on the pitch to shoot from.
    let corridor = CORRIDOR_FLOOR + (1.0 - CORRIDOR_FLOOR) * sight.corridor_clarity;

    // ── HE IS THROUGH ON GOAL ─────────────────────────────────────────
    //
    // `GoalSight::cover` answers "has the defence been beaten" — 0.0 when
    // nothing but the goalkeeper stands between this player and the net.
    // It is computed on every tick of every possession and, until now,
    // **read by nothing at all**. The engine could see a one-on-one and
    // the shot decision could not.
    //
    // That is the reported bug in its purest form. Worked through for a
    // striker three metres from the keeper with the defence beaten:
    // angle 1.0, corridor 1.0, reach 1.0, poise 0.78, no pressure ⇒
    // appetite **0.76** against a bar of **0.87**. He declines. The
    // threshold is drawn once per possession, so having declined he
    // declines for the whole of it — and `advance` clamps to zero inside
    // `carry_hold`, so he stops moving as well. A forward stood still,
    // three metres from the goalkeeper, for as long as he holds the ball.
    //
    // A man clean through shoots. It is the highest-value moment in
    // football and the model had no term for it.
    let clear_run = 1.0 + (1.0 - sight.cover).clamp(0.0, 1.0) * THROUGH_ON_GOAL_LIFT;
    let urge = (target_size
        * sight.angle_quality.powf(0.45)
        * corridor
        * reach
        * poise
        * clear_run
        * (1.0 + pressure * 0.30))
        .clamp(0.0, 1.0);

    // Selection SHARPENS rather than shifts. A discerning forward's
    // appetite falls away faster on a poor look and holds up on a good
    // one; a rash one is flatter across the board. Same curve, different
    // steepness — no thresholds.
    let discernment = (1.0 + (selection - 0.5) * 0.9).clamp(0.55, 1.45);
    let mut appetite = urge.powf(discernment);

    // ── Temperament ───────────────────────────────────────────────────
    // Whether he fancies it. Flair and composure decide who takes one on
    // from 25 yards; `initiative_for` carries the in-match swing — a man
    // who has just scored backs himself, one who has shanked two and been
    // booked takes the extra touch.
    //
    // Centred on 1.0. Personality SHIFTS the appetite; it does not halve
    // it. The old form measured 0.814-0.834 across every distance band —
    // i.e. it was a fourth flat tax with a little variation riding on
    // top, and the bar had been calibrated against the depressed
    // distribution the four of them produced together.
    let temperament = sc::n(sc::eff(ctx.player, mental, |p| p.skills.mental.flair)) * 0.45
        + composure_skill * 0.25
        + execution_skill * 0.30;
    let boldness = (1.0 + (temperament - 0.5) * 0.55).clamp(0.70, 1.30);
    appetite *= boldness;
    appetite *= Psychology::initiative_for(&ctx.context.psychology, ctx.player.id);
    appetite *= (1.0 - low_condition_penalty * 0.20).clamp(0.60, 1.0);

    // ── The state of the game ─────────────────────────────────────────
    // These are football, not statistics, so they stay: both sides take a
    // minute to reset after a goal, and both need a few to find the game
    // at kick-off. Without them the engine put 76% of first goals in the
    // opening quarter of an hour against a real ~25%.
    let mut situational = 1.0_f32;
    if let Some(side) = ctx.player.side {
        let opp_side = match side {
            PlayerSide::Left => PlayerSide::Right,
            PlayerSide::Right => PlayerSide::Left,
        };
        if ctx.context.conceded_recently(opp_side, 6000) {
            situational *= 0.85;
        }
        if ctx.context.conceded_recently(side, 4500) {
            situational *= 0.78;
        }
    }
    let settle_window: u64 = 900_000;
    if ctx.context.total_match_time < settle_window {
        let progress = ctx.context.total_match_time as f32 / settle_window as f32;
        situational *= 0.55 + 0.45 * progress;
    }
    appetite *= situational;

    // ── Team-mates demand the ball ────────────────────────────────────
    // A social force, not a quality filter: defenders key on the man who
    // has shot all afternoon and his own side stop giving it to him.
    // Left in because it is real; the version that raised an xG bar
    // instead is gone with the rest of them.
    let shots_so_far = ctx.memory().shots_taken;
    if shots_so_far > 5 {
        appetite *= (1.0 - (shots_so_far - 5) as f32 * 0.08).clamp(0.35, 1.0);
    }

    // Kept for the diagnostics only — the decision above never reads it.
    // The harness prints mean chance value at the decision point, which
    // is still worth seeing; it just no longer DRIVES anything.
    #[cfg(feature = "match-logs")]
    let xg_for_diag = profile.expected_xg(distance, ctx.player().has_clear_shot());

    // ── Decide, once per opportunity ──────────────────────────────────
    //
    // The threshold is drawn ONCE PER POSSESSION rather than once per
    // tick. `ownership_duration` counts ticks since this possession
    // began, so the tick it began on identifies the opportunity; hashed
    // with the player's id it gives a value that is fixed for as long as
    // he has the ball and different the next time he gets it.
    //
    // This is the part that stops "ask again next tick" from being a
    // winning strategy. Under the old per-tick roll, holding the ball was
    // free extra chances to shoot, so the engine's forwards held it — all
    // the way to the goalkeeper. Now a look either is or is not one he
    // takes, and if it is not, the way to get a shot is to improve the
    // situation rather than to wait.
    let possession_start = ctx
        .current_tick()
        .saturating_sub(ctx.tick_context.ball.ownership_duration as u64);
    let opportunity = possession_start.wrapping_mul(0x9E37_79B9_7F4A_7C15)
        ^ (ctx.player.id as u64).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    let spread = ((opportunity >> 40) as f32) / ((1u32 << 24) as f32);
    // Players differ in where their bar sits, and the same player differs
    // between moments. Narrow band: this is personality, not a lottery.
    //
    // Height calibrated against shot volume. Because the answer is now
    // deterministic per opportunity, a forward shoots on the FIRST tick
    // his appetite clears the bar rather than eventually — so the bar
    // alone sets how many of the looks a team works actually get hit.
    // At 0.24-0.50 every touch inside 16 m cleared it and teams took 38
    // shots; real football hits about a third of what it works.
    //
    // ── The bar is not the same at 30 m as at 12 m ────────────────────
    //
    // A single absolute bar treats every strike as the same decision, and
    // it isn't. Close in, both answers are expensive: a declined look
    // wastes a big chance and so does a bad shot, so the bar is high and
    // only a good look clears it. From range the arithmetic inverts — the
    // shot is low-percentage, but so is everything else available, and
    // missing costs a goal kick thirty metres from your own danger. Real
    // players hit them for exactly that reason: ~18% of real shots come
    // from beyond 22 m.
    //
    // This engine produced 4.7% beyond 22 m and, measurably, ZERO beyond
    // 30 m — not rare, impossible. `reach` correctly says a player's
    // appetite is lower from range; nothing said the bar he has to clear
    // is lower too. Working it through: clearing 0.527 at 30 m needed a
    // 390u striking range (the absolute maximum in the game) AND a
    // perfect angle AND a perfect lane AND perfect poise simultaneously.
    //
    // So the bar eases across the same band `reach` fades over. It is
    // still the same appetite model deciding, and still the player's own
    // range that sets where "long" begins for him — a specialist's bar
    // eases later because his range is longer. The floor keeps a
    // 40 m hopeful from ever being a free shot.
    // Keyed to ABSOLUTE distance, not to the fraction of the player's own
    // range. The thing being modelled is the cost of missing, and that
    // depends on where you are on the pitch — a shot from 30 m is cheap
    // for everybody. Whether you can HIT it from there is the part that
    // varies by player, and `reach` already carries it.
    //
    // Normalising by the player's range instead inverts the football: a
    // specialist with a 45 m range is only 40% "past comfortable" at 30 m
    // and so got the LEAST encouragement exactly where he is the man who
    // should be shooting, while a poacher at 25 m got the most.
    //
    // (A plain `((distance - 128) / 160)` ramp used to be bound here and
    // then immediately shadowed by the hump below without ever being
    // read — thirty lines of tuning notes attached to dead code. The
    // shape those notes describe is the one the hump implements.)
    //
    // …and the licence to hit one from out there is not distance alone.
    //
    // Distance is only the argument for WHY a speculative effort is cheap.
    // Whether it is worth taking is the two things a player actually reads
    // before he lets fly from 25 m: is the lane open, and can I strike it
    // from here. Relief keyed to distance by itself gave the same
    // encouragement to a specialist with a clear sight of goal and to a
    // full-back punting one through three bodies — so it had to be kept
    // small enough for the second case, which meant it was never enough
    // for the first, and the first is the shot real football is full of.
    //
    // Both terms are already computed and already mean the right thing:
    // `sight.lane` is what stands between the ball and the net, and
    // `reach` is this player's own range — a hammer of a centre-half is
    // still near 1.0 at 30 m where a poacher has fallen away. Gating the
    // relief on their product means the bar drops for the man who should
    // be shooting and stays up for the man who should not, which is what
    // lets the relief be big enough to matter.
    //
    // CUBED, and that exponent is the whole calibration. Linear in
    // `lane * reach` let 22-30 m take 45.3% of every shot in the game
    // against a real 13%, because out there the lane is usually open
    // (nobody marks a man 25 m out) and `reach` is still respectable for
    // an ordinary player — so "clear path and can hit it" was true of
    // almost everybody, and the band carries 21% of all shot decisions.
    // A near-miss on either term has to cost real licence, because the
    // difference between a specialist with a genuinely clean sight and an
    // average player with a half-open one is exactly the difference
    // between a shot worth taking from there and a giveaway.
    // ── The bar comes down as the shot gets longer ────────────────────
    //
    // One ramp, not a hump. The hump this replaces peaked at the edge of
    // the box and decayed to nothing past ~27 m, on the argument that
    // "real shot volume peaks around the box and falls away outside it".
    // That is true of the SHOT COUNT — and the shot count is the product
    // of the bar and the appetite, not of the bar alone. The appetite now
    // falls away with distance on its own (`target_size`, `reach`), so
    // building the same fall into the bar counted it twice and produced
    // the measured 0.0% beyond 30 m.
    //
    // What the bar is for is the COST OF THE DECISION, and that genuinely
    // falls the further out you are: from 25 m a miss costs a goal kick,
    // and the alternatives — another sideways pass, another recycle — are
    // no better. That argument only strengthens with distance, so the
    // relief only grows with it.
    //
    // Ramps from 11 m rather than from the edge of the box. At 16.5 m the
    // ramp start left the 11-16.5 m band with no relief from either end
    // and the full base bar to clear, which is the trough this file has
    // now diagnosed three times: it is a normal shooting position in
    // football, not a marginal one.
    //
    // Linear, and not full until 37.5 m. A convex version was tried to
    // separate 22-30 m from beyond it and starved the whole 11-22 m
    // stretch instead: it holds only 3% of the relief at 13 m and 16% at
    // 19 m, and those two bands went to 10.2% and 1.3% of all shots
    // against a real 22% and 20%. The bands are ordered by the appetite
    // falling away with distance; the bar does not need to bend as well.
    let speculative = ((distance - 88.0) / 212.0).clamp(0.0, 1.0);

    // …and it is NOT gated on ability.
    //
    // It used to be, through a `(corridor * reach)³` licence, so only a
    // specialist with a clean sight got the eased bar. That is the wrong
    // football and it is why the 30 m+ band measured exactly 0.0% for so
    // long. An ordinary player with no vision hits a hopeful one from
    // thirty metres — he is arguably the man MOST likely to, because the
    // alternative he cannot see is the pass a better player would find.
    // Gating the relief on his ability said the opposite twice over: the
    // appetite already prices whether he can reach it (`reach`), whether
    // he fancies it (`boldness`), and how readily he settles for a poor
    // look (`discernment`, which is DELIBERATELY kinder to a rash player).
    // The bar is not about the player at all — it is the cost of the
    // decision, and from thirty metres a miss costs a goal kick whoever
    // takes it.
    //
    // What that leaves is the hopeful long shot arriving in a crowded
    // goalmouth: blocked, deflected, parried, scrambled. That is the
    // realistic outcome of the shot, and it belongs to the block, save
    // and rebound models rather than to a veto here.
    // ── …and one band that needs no licence at all ────────────────────
    //
    // A strike from the edge of the area is not a speculative effort and
    // not a tap-in — it is the NORMAL shot in football, the single most
    // common shooting position there is, and the only band where nothing
    // in the model was arguing for it. The close relief is gone by 11 m
    // and the speculative ramp has barely started, so 11-16.5 m sat at
    // the full base bar: measured 0.977 against an appetite of 0.344,
    // the highest effective bar on the pitch, and it took **8.2% of all
    // shots against a real 22%**. A midfielder standing fourteen metres
    // out with the ball declined and laid it off — the reported bug.
    //
    // Deliberately a bounded hump rather than a lower ramp. Both were
    // measured: lowering the ramp instead (concave, base 0.87) reaches
    // 11-16.5 m only by also relieving 22-30 m, and that band carries
    // 12% of every shot decision in a match against this band's 8% — it
    // went to **43% of all shots** and the match to 5.29 goals. The
    // relief has to be spent where the football is, and come back up on
    // both sides of it.
    //
    // A PLATEAU, not a peak, and that matters more than its height.
    //
    // The `spread` half of the bar is drawn once per possession, but the
    // relief is a function of DISTANCE — so any gradient in it is
    // something a carrier can walk down. A triangular hump peaking at
    // 14 m was measured doing exactly that: 11-16.5 m went 8.2% → 20.1%
    // and 16.5-22 m FELL 17.3% → 6.4% at an unchanged bar, because men
    // who used to strike from twenty metres now carried to fourteen and
    // struck there. The per-opportunity threshold stops "ask again next
    // tick" from being a winning strategy; a sloped bar reintroduced the
    // same exploit in space instead of time.
    //
    // Flat across 12-23 m, so anywhere in the normal shooting band is
    // equally a shot and there is nothing to walk toward. Rises quickly
    // from 9 m (below that the close relief and the appetite's own
    // `target_size` already own the decision) and fades into the
    // speculative ramp by 30 m.
    let box_relief = BOX_RELIEF
        * if distance < 96.0 {
            ((distance - 72.0) / 24.0).clamp(0.0, 1.0)
        } else if distance <= 184.0 {
            1.0
        } else {
            (1.0 - (distance - 184.0) / 56.0).clamp(0.0, 1.0)
        };
    let range_ease = (speculative * LONG_RANGE_RELIEF).max(box_relief);

    // ── …and it eases CLOSE IN too, for the opposite reason ───────────
    //
    // The bar is a cost-of-the-decision model, and the cost is not
    // monotonic in distance — it is highest in the middle. From 18 m a
    // pass is often genuinely better, so a high bar is right. From 5 m
    // there is no better option that will ever exist: you will not get a
    // cleaner look than this one, and declining it is the expensive
    // answer. Strikers inside the six-yard box shoot.
    //
    // Nothing in the model said so. `reach` is flat at 1.0 inside the
    // comfortable range — correctly, it only measures whether he CAN
    // strike it — so appetite close in was set by `poise`, which is
    // dragged down by the sprint penalty exactly when a forward is
    // arriving onto a chance. Measured: mean appetite at <6 m was 0.22
    // against a bar of 0.53-0.77, so a striker five metres out almost
    // never cleared it, and the release clause below then handed the ball
    // away. That is the "runs around in the box instead of shooting"
    // report, and it is the single largest gap between this model and
    // football.
    //
    // Cubed, so the relief is concentrated where the argument actually
    // holds — genuinely point-blank — and is nearly gone by the edge of
    // the comfortable range, where a pass really can be the better ball.
    // Linear was tried and is far too broad: it lifted the 6-11 m band to
    // 41% of all shots and the match to 6.03 goals, because "there is no
    // better option than this" is simply not true at eleven metres.
    // Keyed to ABSOLUTE distance, not to the player's own range, and no
    // longer cubed.
    //
    // `comfortable` is 11-22 m depending on the player, so dividing by it
    // made "point blank" mean something different for every footballer on
    // the pitch; cubing it then threw away what was left. Measured at
    // three metres from the goal — a tap-in — the pair were worth 0.034
    // of bar relief against a bar of 0.87. The one look in football that
    // is genuinely automatic got nothing.
    //
    // Point blank is a fact about the pitch, not about the man: inside
    // the six-yard box everybody shoots. Linear over 12.5 m so the relief
    // is large where the argument is overwhelming and gone by the edge of
    // the area, where a pass really can be the better ball.
    let close_ease = (1.0 - distance / POINT_BLANK_SPAN).clamp(0.0, 1.0);
    let range_ease = range_ease.max(close_ease * CLOSE_RANGE_RELIEF);

    // ── …but only when there is nothing better on ─────────────────────
    //
    // The whole justification for the eased bar is that from range the
    // alternatives are no better either. When a team-mate is genuinely
    // better placed — nearer the goal and free — that justification is
    // gone, and hitting a hopeful 30-yarder instead of finding him is
    // precisely the decision real players are criticised for.
    //
    // This is load-bearing for the front line, not just tidiness.
    // Measured: with the relief applied unconditionally, midfielders took
    // 73% of all shots and forwards 23%, because a striker who had just
    // worked himself free was passed over for a speculative strike. The
    // outlet test has to be asked BEFORE the bar is eased, not after it
    // has already been cleared.
    let outlet = ctx
        .player()
        .passing()
        .find_best_pass_option_with_distance(140.0);
    let generosity = (0.78 + decisions * 0.14).clamp(0.78, 0.92);
    let opp_goal = ctx.player().opponent_goal_position();
    // "Free" at 12u is 1.5 m — a team-mate with a defender two metres off
    // him failed it, so a genuine lay-off to a better-placed man did not
    // count as an option and the carrier took the shot on himself. 24u
    // (3 m) is the distance at which a pass is actually playable, which is
    // what this test is for.
    let better_placed = outlet.is_some_and(|(t, _)| {
        let their_distance = (opp_goal - t.position).magnitude();
        their_distance < distance * generosity
            && ctx.tick_context.grid.opponents(t.id, 24.0).next().is_none()
    });
    let range_ease = if better_placed { 0.0 } else { range_ease };

    // Base height calibrated against shot VOLUME, which is what it sets:
    // the answer is deterministic per opportunity, so a player shoots on
    // the first tick his appetite clears the bar, and the bar alone
    // decides how many of the looks a team works actually get hit.
    //
    // 0.527 was set before the crossing, defending and long-range work
    // added supply; with those in, teams took 23.8 shots against a real
    // 13.
    //
    // It is a VERY steep knob, and not a share-preserving one. Measured:
    // 0.527 → 23.8 shots/team, 0.60 → 10.1 — a 14% lift more than halved
    // the volume, because the appetite distribution is dense right around
    // here. And because the reliefs subtract from it, a uniform lift
    // falls entirely on the un-eased middle: at 0.60 the 6-11 m band rose
    // to 43% of all shots while 11-22 m collapsed to 19%. Move it in
    // small steps and re-read the whole distance mix, never just the
    // total.
    // `range_ease` is now an ABSOLUTE relief in bar units — each of the
    // three reliefs carries its own magnitude — so it is subtracted
    // directly rather than scaled by a shared constant here.
    let threshold = (SHOT_BAR_BASE + spread * 0.24 - range_ease).max(LONG_RANGE_FLOOR);

    // Sampled HERE rather than at the top of the roll so the table can
    // carry `threshold` alongside the appetite. Nothing between the two
    // points returns, so the sample population is identical.
    #[cfg(feature = "match-logs")]
    {
        use std::sync::atomic::Ordering;
        helper_diag::REACHED_ROLL.fetch_add(1, Ordering::Relaxed);
        let dband = time_band_diag::band_for_distance(distance);
        time_band_diag::ROLLS_BY_DIST[dband].fetch_add(1, Ordering::Relaxed);
        time_band_diag::record_will_factors(
            dband,
            [
                urge,
                reach,
                sight.angle_quality,
                sight.lane,
                poise,
                boldness,
                situational,
                Psychology::initiative_for(&ctx.context.psychology, ctx.player.id),
                appetite,
                sight.pressure_clarity,
                sight.corridor_clarity,
                threshold,
            ],
        );
        helper_diag::SUM_XG_X1000.fetch_add((xg_for_diag * 1000.0) as u64, Ordering::Relaxed);
        helper_diag::SUM_WILLINGNESS_X1000.fetch_add((appetite * 1000.0) as u64, Ordering::Relaxed);
        let band =
            time_band_diag::band_for_minute(sc::minute_from_ms(ctx.context.total_match_time));
        time_band_diag::ROLL_REACHED_BY_BAND[band].fetch_add(1, Ordering::Relaxed);
    }

    if appetite >= threshold {
        #[cfg(feature = "match-logs")]
        helper_diag::ROLL_PASSED.fetch_add(1, Ordering::Relaxed);
        #[cfg(feature = "match-logs")]
        time_band_diag::APPROVED_BY_DIST[time_band_diag::band_for_distance(distance)]
            .fetch_add(1, Ordering::Relaxed);
        #[cfg(feature = "match-logs")]
        if distance > 176.0 {
            time_band_diag::APPROVED_BY_TAG[time_band_diag::tag_index(tag)]
                .fetch_add(1, Ordering::Relaxed);
        }
        return ShotDecision::Shoot { reason: tag };
    }

    // ── Not this time ─────────────────────────────────────────────────
    // Two ways this ends, and neither of them is "carry it closer".
    //
    // Is somebody better placed? Compared as SITUATIONS, not as numbers:
    // a team-mate is worth the ball if he is nearer the goal than I am and
    // nobody is on him, which is the question a player actually asks. A
    // good decision-maker gives it up for a marginally better position; a
    // poor one only when the difference is obvious.
    // Or: he is ALREADY in a position to shoot from and has decided
    // against it. Then he releases, because the one thing he must not do
    // is take it nearer — inside his own range the angle only narrows
    // from here and the keeper only gets closer, so a look he does not
    // fancy at 18 m is a worse one at 8 m. `Hold` used to be the answer
    // here, and `Hold` means "keep doing what you were doing", which for
    // a carrier meant advancing. That is the other half of the
    // ran-at-the-goalkeeper report: the decision said no and the carrying
    // states answered by closing the distance.
    //
    // …but "release" has to mean release to somebody BETTER. It was
    // `outlet.is_some()` — any outlet at all, and `find_best_pass_option`
    // almost always finds one — so a striker inside his own range who
    // declined the shot gave the ball to whoever happened to be nearest,
    // including a team-mate further from goal than he was. Combined with
    // the bar he could not clear close in, that is the reported bug in
    // full: 25% of decisions taken INSIDE SIX METRES resolved as a pass,
    // frequently a backwards one.
    //
    // A man in a shooting position gives it up for a better position or
    // he shoots. `better_placed` is already the test for that, so the
    // whole clause collapses into it.
    if better_placed {
        #[cfg(feature = "match-logs")]
        helper_diag::PASS_DEFERRAL.fetch_add(1, Ordering::Relaxed);
        #[cfg(feature = "match-logs")]
        time_band_diag::record_reject(4, distance);
        return ShotDecision::Pass;
    }

    // …and the note above still did not reach the code.
    //
    // `Hold` remained the answer for a player who is ALREADY in a
    // shooting position and has decided against it, and `Hold` means
    // "carry on doing what you were doing". What he was doing is
    // carrying, and the carry target stops at `carry_hold` — so he
    // arrives twelve metres out, declines, and then stands there with the
    // ball, re-asking a question whose answer is fixed for the whole
    // possession. That is the "hovers around the goal" half of the
    // report, and it is a state football does not have: a man in a
    // shooting position either shoots or gives it, and taking it nearer
    // only narrows the angle and brings the keeper out.
    //
    // ⚠ AND IT HAS TO BE A BALL WORTH PLAYING, OR THIS IS A BACKWARD-PASS
    // MACHINE. It was `distance <= comfortable && outlet.is_some()`, and
    // both halves were far too loose:
    //
    //   * `comfortable` is 0.45 of a player's striking range — 11-22 m —
    //     so the clause covered essentially every touch in the attacking
    //     third rather than a man stood in a shooting position;
    //   * `outlet` is `find_best_pass_option`, which almost always finds
    //     SOMEBODY, and the best-scoring somebody in a congested attacking
    //     third is routinely the man BEHIND the ball.
    //
    // Measured: `pass_defer` went 52k → **734k** and midfielders in
    // shooting positions laid the ball backwards instead of striking it.
    // The anti-hover intent stands, but the situation it describes is
    // narrow: he is being closed down, so he cannot simply keep the ball
    // and work a better angle. Free, with the shot declined, the right
    // answer is still `Hold` — carry, shift the angle, ask again from a
    // better position.
    //
    // And a release is never backwards. A team-mate materially further
    // from goal than the carrier is a recycle, not a lay-off; that is the
    // ball the report is about.
    let releasable = outlet.is_some_and(|(t, _)| {
        (opp_goal - t.position).magnitude() <= distance + RELEASE_BACKWARD_TOLERANCE
    });
    if distance <= comfortable
        && releasable
        && ctx.player().pressure().is_under_immediate_pressure()
    {
        #[cfg(feature = "match-logs")]
        helper_diag::PASS_DEFERRAL.fetch_add(1, Ordering::Relaxed);
        #[cfg(feature = "match-logs")]
        time_band_diag::record_reject(4, distance);
        return ShotDecision::Pass;
    }

    ShotDecision::Hold
}

/// Find a central midfielder arriving unmarked in a shooting position —
/// the cutback target. Shared by the forward and the wide-midfielder
/// ball-carriers so both feed the same arriving-runner pattern, which is
/// the dominant real-football source of midfielder goals (a runner
/// arriving at the penalty spot as the ball is worked to the byline).
///
/// Tightly gated so it never fires as a generic pass: the receiver must
/// be a central midfielder, in the central corridor (real shooting
/// angle), within shooting range of goal, no further from goal than the
/// carrier (a true cutback / square ball, never a backward bail-out),
/// unmarked, and on a clear passing lane.
pub fn find_cutback_to_arriving_runner(ctx: &StateProcessingContext) -> Option<MatchPlayerLite> {
    let goal = ctx.player().opponent_goal_position();
    let field_height = ctx.context.field_size.height as f32;
    let center_y = field_height / 2.0;
    let central_band = field_height * 0.17;
    let carrier_goal_d = (goal - ctx.player.position).magnitude();

    let mut best: Option<(MatchPlayerLite, f32)> = None;
    for t in ctx.players().teammates().nearby(90.0) {
        if !t.tactical_positions.is_central_midfielder() {
            continue;
        }
        // Central corridor — needed for a real shooting angle.
        if (t.position.y - center_y).abs() > central_band {
            continue;
        }
        // In a genuine central shooting position (inside the 62u band the
        // arriving-runner target deepens into; above 14u so it isn't a
        // pass into the keeper's hands).
        let td = (goal - t.position).magnitude();
        if !(36.0..=110.0).contains(&td) {
            continue;
        }
        // Allow the classic lay-BACK to an arriving midfielder at the edge
        // of the box (the iconic Lampard/Gerrard goal): the carrier is in
        // the box but crowded (we only reach here after their own shot
        // blocks declined), and an unmarked runner trailing with a clear
        // strike is the better chance even though they are further from
        // goal. Reject only a runner who is WAY behind (>25u further than
        // the carrier) — that's a recycle, not a cutback.
        if td > carrier_goal_d + 25.0 {
            continue;
        }
        // A marked runner is not a cutback target. The old gate rejected
        // only "2 opponents within 8u" — 8u is one metre, so in practice
        // every arriving runner qualified, and once the state repair made
        // midfielders fitter (arriving ~50% more often) the cutback count
        // doubled and forwards handed their best chances away wholesale
        // (FWD conversion 6.8% → 2.9%, FWD xG/shot 0.178 → 0.118, MID
        // goal share 29.5% → 48%). "Unmarked at the penalty spot" has to
        // mean what it says: no defender within reach of the first-time
        // strike (18u ≈ 2.3m — a tracking defender one stride away blocks
        // the cutback lane or the shot). A tracked runner is exactly the
        // case where the real forward keeps the shot himself.
        if ctx.tick_context.grid.opponents(t.id, 30.0).count() >= 1 {
            continue;
        }
        if !ctx.player().has_clear_pass(t.id) {
            continue;
        }
        // Prefer the most central runner closest to goal.
        let score = 200.0 - td - (t.position.y - center_y).abs();
        if best.as_ref().map(|(_, s)| score > *s).unwrap_or(true) {
            best = Some((t, score));
        }
    }
    best.map(|(t, _)| t)
}

#[cfg(test)]
mod tests {
    // Test the pure scoring math via parameterised helpers. We can't
    // easily fixture a `StateProcessingContext` so we extract the
    // willingness / floor formulas and verify monotonicity directly.

    fn willingness(
        selection: f32,
        execution_skill: f32,
        composure_skill: f32,
        body_control: f32,
        clarity: f32,
        balance_factor: f32,
        xg: f32,
        gk_proximity: f32,
        low_condition_penalty: f32,
        inside_six: bool,
    ) -> f32 {
        let base = 0.06 + selection * 0.22 + composure_skill * 0.10 + execution_skill * 0.12;
        let xg_boost = (xg / 0.20_f32).clamp(0.50, 1.40);
        let clarity_mult = 0.50 + clarity * 0.50;
        let body_control_mult = (0.65 + body_control * 0.40).clamp(0.60, 1.05);
        let condition_mult = (1.0 - low_condition_penalty * 0.55).clamp(0.40, 1.05);
        let mut w = base
            * xg_boost
            * clarity_mult
            * body_control_mult
            * condition_mult
            * gk_proximity
            * balance_factor;
        if inside_six {
            let floor = (0.10 + execution_skill * 0.30).clamp(0.10, 0.45);
            w = w.max(floor);
        }
        let cap = if xg >= 0.35 { 0.60 } else { 0.48 };
        w.clamp(0.012, cap)
    }

    fn min_xg(execution_skill: f32, selection: f32, distance: f32) -> f32 {
        let distance_floor_base = if distance <= 36.0 {
            0.13
        } else if distance <= 60.0 {
            0.13 - (distance - 36.0) / 24.0 * 0.07
        } else {
            0.045
        };
        let mut x = distance_floor_base - execution_skill * 0.06 + (selection - 0.5) * 0.025;
        let (lo, hi) = if execution_skill < 0.25 {
            if distance > 60.0 {
                (0.05, 0.10)
            } else {
                (0.10, 0.18)
            }
        } else if execution_skill < 0.55 {
            if distance > 60.0 {
                (0.035, 0.08)
            } else {
                (0.07, 0.13)
            }
        } else {
            if distance > 60.0 {
                (0.025, 0.07)
            } else {
                (0.045, 0.10)
            }
        };
        x = x.clamp(lo, hi);
        x
    }

    #[test]
    fn elite_finisher_more_willing_than_mediocre() {
        // Same chance — elite (high execution + selection) pulls the
        // trigger materially more often than mediocre.
        let mediocre = willingness(0.45, 0.30, 0.35, 0.50, 0.6, 1.0, 0.10, 1.0, 0.0, false);
        let elite = willingness(0.80, 0.80, 0.80, 0.85, 0.6, 1.0, 0.10, 1.0, 0.0, false);
        assert!(elite > mediocre * 1.4, "elite={elite} mediocre={mediocre}");
    }

    #[test]
    fn xg_floor_scales_with_execution_skill_in_box() {
        // Inside-box distance (30u). Poor finisher demands a
        // meaningfully higher floor than an elite.
        let poor = min_xg(0.10, 0.50, 30.0);
        let elite = min_xg(0.80, 0.50, 30.0);
        assert!(poor >= 0.10, "poor in-box too low={poor}");
        assert!(elite <= 0.10 + 0.001, "elite in-box too high={elite}");
        assert!(poor > elite, "poor={poor} elite={elite}");
    }

    #[test]
    fn long_distance_floor_relaxes_for_speculative_shots() {
        // 70u shot. Real football: ~38% of shots are from outside the
        // box, so the long-distance floor must allow xG ~0.04-0.05
        // attempts through. Earlier the box floor (0.10..0.22) blocked
        // every long-shot — that was the 0-0 bug.
        let elite_long = min_xg(0.80, 0.50, 70.0);
        let avg_long = min_xg(0.40, 0.50, 70.0);
        assert!(
            elite_long <= 0.07,
            "elite long-shot floor too high={elite_long}"
        );
        assert!(avg_long <= 0.08, "avg long-shot floor too high={avg_long}");
    }

    #[test]
    fn smart_forward_demands_higher_floor_than_poacher() {
        // Same execution_skill / distance, different selection: smart
        // picks demand a slightly higher xG floor.
        let smart = min_xg(0.40, 0.85, 30.0);
        let poacher = min_xg(0.40, 0.30, 30.0);
        assert!(smart >= poacher - 0.001, "smart={smart} poacher={poacher}");
    }

    #[test]
    fn sprint_with_low_balance_drops_willingness() {
        // Strength+agility+first_touch+composure all 0.30 → balance 0.30.
        let physical_balance: f32 = (0.30 + 0.30 + 0.30 + 0.30) / 4.0;
        let sprint_120: f32 = 1.0;
        let factor = (1.0 - sprint_120 * (0.45 - physical_balance * 0.40)).clamp(0.55, 1.0);
        // Should drop willingness ~33% vs no-sprint.
        assert!(factor < 0.70, "factor={factor}");
    }

    #[test]
    fn inside_six_floor_scales_with_execution_skill() {
        // 5/20 player floors near 0.16; elite near 0.40. The poor
        // player should NOT inherit the elite floor.
        let poor = willingness(0.20, 0.10, 0.20, 0.30, 0.0, 0.55, 0.05, 1.0, 0.0, true);
        let elite = willingness(0.85, 0.85, 0.85, 0.85, 0.0, 0.55, 0.05, 1.0, 0.0, true);
        assert!(poor < 0.20, "poor floor too high: {poor}");
        assert!(elite > 0.30, "elite floor too low: {elite}");
    }

    #[test]
    fn poor_one_v_one_conversion_lower_than_elite() {
        // 1v1 GK proximity: bonus scales with composure+first_touch+decisions.
        let cool_poor = (0.40 + 0.45 + 0.35) / 3.0_f32;
        let prox_poor = (0.55 + cool_poor * 0.55).clamp(0.55, 1.10);
        let cool_elite = (0.80 + 0.85 + 0.80) / 3.0_f32;
        let prox_elite = (0.55 + cool_elite * 0.55).clamp(0.55, 1.10);
        assert!(prox_elite > prox_poor + 0.10);
    }
}
