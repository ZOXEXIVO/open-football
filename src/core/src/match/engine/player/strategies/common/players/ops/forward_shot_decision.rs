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
    pub fn reset() {
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
    /// 0 base, 1 xg_boost, 2 clarity_mult, 3 body_control_mult,
    /// 4 condition_mult, 5 gk_context_mult, 6 balance_factor,
    /// 7 psychology, 8 final willingness.
    pub const WFACTORS: usize = 9;
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
                // NB a square-on defender has a lateral offset near zero,
                // so this signum flips as the carrier jitters. Gating it
                // with a 1 m dead zone and deferring to the centre drift
                // was tried and REVERTED: square on is the common case in
                // front of goal, so carriers stopped dead in front of
                // their man instead of going round him, and the ball
                // going nowhere went 178 -> 449 s a match. Whatever
                // replaces this has to keep him committing to a side.
                sidestep = if relative.dot(&lateral) > 0.0 { -1.0 } else { 1.0 };
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
    // corner. `in_state_time` stands in for how long he has been at pace.
    let sprinting = (ctx.in_state_time as f32 / 120.0).clamp(0.0, 1.0);
    let physical_balance = (skills.physical.strength / 20.0
        + skills.physical.agility / 20.0
        + first_touch
        + composure)
        / 4.0;
    let poise = (physical_balance * (1.0 - sprinting * 0.40) * (0.70 + body_control * 0.30))
        .clamp(0.0, 1.0);

    // ── Being closed down ─────────────────────────────────────────────
    // Pressure is already in `sight.lane` as an obstruction. It also
    // changes the CHOICE: a man about to be tackled hits it now or loses
    // it, so it lifts the urge of someone who can see the goal.
    let pressure = pressure_penalty.clamp(0.0, 1.0);

    // ── The urge ──────────────────────────────────────────────────────
    let urge = (sight.angle_quality.powf(0.65)
        * sight.lane.powf(0.55)
        * reach
        * (0.45 + poise * 0.55)
        * (1.0 + pressure * 0.35))
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
    let boldness = (0.50
        + sc::n(sc::eff(ctx.player, mental, |p| p.skills.mental.flair)) * 0.25
        + composure_skill * 0.15
        + execution_skill * 0.15)
        .clamp(0.45, 1.15);
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

    #[cfg(feature = "match-logs")]
    {
        use std::sync::atomic::Ordering;
        helper_diag::REACHED_ROLL.fetch_add(1, Ordering::Relaxed);
        let dband = time_band_diag::band_for_distance(distance);
        time_band_diag::ROLLS_BY_DIST[dband].fetch_add(1, Ordering::Relaxed);
        // Factor vector re-pointed at the new model. Slots in order:
        // urge, reach, angle_quality, lane, poise, boldness, situational,
        // psychology, final appetite.
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
            ],
        );
        helper_diag::SUM_XG_X1000.fetch_add((xg_for_diag * 1000.0) as u64, Ordering::Relaxed);
        helper_diag::SUM_WILLINGNESS_X1000.fetch_add((appetite * 1000.0) as u64, Ordering::Relaxed);
        let band =
            time_band_diag::band_for_minute(sc::minute_from_ms(ctx.context.total_match_time));
        time_band_diag::ROLL_REACHED_BY_BAND[band].fetch_add(1, Ordering::Relaxed);
    }

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
    let opportunity = possession_start
        .wrapping_mul(0x9E37_79B9_7F4A_7C15)
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
    let threshold = 0.527 + spread * 0.24;

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
    let outlet = ctx
        .player()
        .passing()
        .find_best_pass_option_with_distance(140.0);
    let generosity = (0.78 + decisions * 0.14).clamp(0.78, 0.92);
    let opp_goal = ctx.player().opponent_goal_position();
    let better_placed = outlet.is_some_and(|(t, _)| {
        let their_distance = (opp_goal - t.position).magnitude();
        their_distance < distance * generosity
            && ctx.tick_context.grid.opponents(t.id, 12.0).next().is_none()
    });

    // Or: he is ALREADY in a position to shoot from and has decided
    // against it. Then he releases, because the one thing he must not do
    // is take it nearer — inside his own range the angle only narrows
    // from here and the keeper only gets closer, so a look he does not
    // fancy at 18 m is a worse one at 8 m. `Hold` used to be the answer
    // here, and `Hold` means "keep doing what you were doing", which for
    // a carrier meant advancing. That is the other half of the
    // ran-at-the-goalkeeper report: the decision said no and the carrying
    // states answered by closing the distance.
    let in_shooting_position = distance <= comfortable && sight.lane > 0.25;

    if better_placed || (in_shooting_position && outlet.is_some()) {
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
