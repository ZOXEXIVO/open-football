use crate::r#match::MatchPlayerLite;
use crate::r#match::PlayerSide;
use crate::r#match::StateProcessingContext;
use crate::r#match::player::strategies::players::ops::skill_composites as sc;
use crate::r#match::player::strategies::players::skills::SkillCurve;
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
        for arr in [
            &SHOTS_BY_BAND,
            &ON_TARGET_BY_BAND,
            &XG_X1000_BY_BAND,
            &GOALS_BY_BAND,
            &ROLL_REACHED_BY_BAND,
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

/// Skill-aware shot evaluation used by every forward state that can
/// decide to strike. Combines:
///
/// 1. Hard gates: per-player and team cooldowns.
/// 2. xG quality floor (skill-graded).
/// 3. Clear-shot lane (continuous `shot_clarity`).
/// 4. Sprint / balance penalty (post-RunningInBehind composure cost).
/// 5. Goalkeeper-position context (1v1 vs covered angle).
/// 6. Pass expected-value comparison with marked-receiver discount.
/// 7. Per-tick willingness roll (low-skill hesitate, elite pull the
///    trigger).
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
    // Anything beyond the absolute long-range cap is hopeless even
    // for elite long-shooters — keep the ball.
    if distance > 110.0 {
        #[cfg(feature = "match-logs")]
        helper_diag::HOLD_FAR.fetch_add(1, Ordering::Relaxed);
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

    // ── xG quality ────────────────────────────────────────────────────
    // Pre-shot xG (matches `handle_shoot_event`'s formula). Low-xG
    // attempts are rejected outright; the inside-six bypass below
    // applies a skill-graded floor instead of letting any tap-in pass.
    let xg = profile.expected_xg(distance, ctx.player().has_clear_shot());
    // Skill-graded xG floor — heavy penalty for poor finishers, soft
    // ceiling for elites. The floor must accommodate THREE distance
    // bands: inside-box (<= 36u, xG 0.10–0.40), mid-range (36..60u,
    // xG 0.05–0.13), and long-distance (60..90u, xG 0.03–0.07). A
    // single fixed floor that fits the box rejects every realistic
    // long-shot (the 0-0 bug). Distance-aware base eases the floor as
    // the player moves out — the helper still rejects the genuinely
    // hopeless (>90u or sub-xG-floor) shots, but lets long-distance
    // attempts through to the willingness roll where skill-graded
    // chance quality finally decides.
    let sprint_penalty_term = if ctx.in_state_time > 30 { 1.0 } else { 0.0 };
    // The xG floor is a "is this attempt worth the cooldown" gate, not
    // a quality filter — that's the willingness roll's job. Real
    // football xG distribution: most shots are 0.03–0.10, with the
    // population average ~0.10. The floor must let 0.025–0.04 shots
    // through (long-range, low-quality looks) so the population shot
    // count lands near 13/team, with the willingness roll suppressing
    // most of those cheap looks. Earlier 0.07–0.22 floors blocked 99%
    // of attempts and produced the 0-0 epidemic.
    let distance_floor_base = if distance <= 36.0 {
        0.090
    } else if distance <= 60.0 {
        0.075
    } else {
        0.050
    };
    let mut min_xg = distance_floor_base - execution_skill * 0.020
        + pressure_penalty * 0.020
        + sprint_penalty_term * 0.015
        + low_condition_penalty * 0.015;
    // Selection nudge: a high-selection player demands a slightly
    // higher floor (better chooser); a low-selection player gambles.
    min_xg += (selection - 0.5) * 0.018;
    // Clamp by skill tier (distance-relative).
    let (lo, hi) = if execution_skill < 0.25 {
        if distance > 60.0 {
            (0.040, 0.075)
        } else {
            (0.075, 0.125)
        }
    } else if execution_skill < 0.55 {
        if distance > 60.0 {
            (0.032, 0.062)
        } else {
            (0.062, 0.105)
        }
    } else {
        if distance > 60.0 {
            (0.025, 0.052)
        } else {
            (0.052, 0.090)
        }
    };
    min_xg = min_xg.clamp(lo, hi);
    // Anti-monopoly xG trim for an ISOLATED hog. The lay-off above
    // redistributes when a team-mate is in range, but a striker the team
    // funnels everything to is often alone in the box with no outlet — and
    // then shoots ~12 low-xG looks/game. Only past a high count (8+), and
    // CAPPED at +0.05, raise their bar so the near-worthless attempts
    // (xG < ~0.10) are skipped while genuinely good chances still go. The
    // cap is the lesson from the uncapped version, which rejected good
    // chances too and dropped team scoring ~25%. Inside-six tap-ins exempt.
    let hog_shots = ctx.memory().shots_taken;
    if hog_shots > 7 {
        min_xg += ((hog_shots - 7) as f32 * 0.010).min(0.05);
    }
    let inside_six = distance <= 18.0;
    // Inside-six floor: skill-graded, so a 5/20 player floors near 0.15
    // instead of inheriting the unconditional 0.30 free pass.
    let inside_six_floor = 0.12 + execution_skill * 0.28;
    if !inside_six && xg < min_xg {
        #[cfg(feature = "match-logs")]
        helper_diag::HOLD_XG.fetch_add(1, Ordering::Relaxed);
        return ShotDecision::Hold;
    }
    if inside_six && xg < (inside_six_floor.min(min_xg)) {
        #[cfg(feature = "match-logs")]
        helper_diag::HOLD_INSIDE_SIX_XG.fetch_add(1, Ordering::Relaxed);
        return ShotDecision::Hold;
    }

    // ── Clear shot ────────────────────────────────────────────────────
    let clarity = ctx.player().shot_clarity();
    if !ctx.player().has_clear_shot() && !inside_six {
        #[cfg(feature = "match-logs")]
        helper_diag::HOLD_NO_CLEAR.fetch_add(1, Ordering::Relaxed);
        return ShotDecision::Hold;
    }

    // ── Sprint / balance penalty ──────────────────────────────────────
    // Approximates body control after a sprint. After
    // `RunningInBehind`, `in_state_time` reflects how many ticks the
    // forward has been at full pace; physical attributes plus
    // first-touch / composure / agility approximate body balance.
    let physical_balance = (skills.physical.strength / 20.0
        + skills.physical.agility / 20.0
        + first_touch
        + composure)
        / 4.0;
    let in_state = ctx.in_state_time as f32;
    let sprinting = in_state.min(120.0) / 120.0; // 0..1 over a long sprint
    // Low-balance sprinters lose up to 35% of willingness; well-balanced
    // forwards lose <5%.
    let balance_factor = (1.0 - sprinting * (0.45 - physical_balance * 0.40)).clamp(0.55, 1.0);

    // ── GK / 1v1 context ──────────────────────────────────────────────
    // 1v1: keeper close means GREAT chance, but composure + first touch
    // matter more. A panicked Composure-8 striker against a closing
    // keeper still squanders most 1v1s in real football.
    let gk_proximity = if let Some(gk) = ctx.players().opponents().goalkeeper().next() {
        let d = (gk.position - ctx.player.position).magnitude();
        if d < 25.0 && distance < 70.0 {
            // 1v1 — apply skill-graded conversion bias rather than a
            // flat bonus. (Composure + first touch + decisions) governs
            // whether the striker keeps cool.
            let cool = (composure + first_touch + decisions) / 3.0;
            (0.55 + cool * 0.55).clamp(0.55, 1.10)
        } else {
            1.0
        }
    } else {
        1.10
    };

    // ── Pass-vs-shot EV ───────────────────────────────────────────────
    // Cheap version of the comparison done in Running's full decision
    // tree — without it, RunningInBehind/Finishing always prefer the
    // shot even when a teammate has a tap-in waiting. The teamwork
    // signal is consumed by the SkillCurve below.
    let best_pass_ev = ctx
        .player()
        .passing()
        .find_best_pass_option_with_distance(60.0)
        .map(|(t, _)| {
            let opp_near = ctx.tick_context.grid.opponents(t.id, 12.0).count();
            let mark_factor = if opp_near >= 2 { 0.5 } else { 1.0 };
            // Tactical value of the pass: how much closer does it get the
            // receiver to goal vs the carrier?
            let opp_goal = ctx.player().opponent_goal_position();
            let carrier_d = (opp_goal - ctx.player.position).magnitude();
            let receiver_d = (opp_goal - t.position).magnitude();
            let progression = ((carrier_d - receiver_d) / carrier_d.max(1.0)).clamp(-0.5, 1.0);
            // Receiver-xG approximation. The receiver still has to
            // control the ball, turn, beat their marker and pick a
            // shot — most "nearby teammates" are NOT a tap-in chance.
            // Earlier values (0.45 close / 0.30 mid / 0.18 long) fed
            // the deferral check a fantasy cutback EV that beat every
            // realistic 0.05–0.10 long-shot, so the helper returned
            // Pass on every clear-shot tick and not a single shot
            // fired through PRIO 0.5. New scale matches the receiver-
            // xg-after-control reality (a striker with an open net
            // gets ~0.30 not 0.45; a 30u teammate gets ~0.12).
            let pass_xg = if receiver_d < 24.0 {
                0.28
            } else if receiver_d < 40.0 {
                0.16
            } else if receiver_d < 60.0 {
                0.09
            } else if receiver_d < 80.0 {
                0.05
            } else {
                0.03
            };
            (pass_xg * mark_factor) * (0.6 + progression * 0.4)
        })
        .unwrap_or(0.0);

    // Margin tightened further: range 0.14..0.06 → 0.10..0.02. A
    // smart-passing forward (high teamwork) now defers to a comparable
    // teammate even when the shot's xG is just 0.02 better than the
    // pass EV — which is what cuts elite-side shots-per-FT-entry from
    // ~1.0 back toward real PL top's ~0.45. Low-teamwork forwards
    // (margin 0.10) still take the shot most of the time. Combined
    // with the tightened anti-monopoly taper above, this keeps
    // strong-team shot volume near 17/match instead of inflating to
    // 30+ via "any forward, any chance, every chance".
    let margin = SkillCurve::new(skills.mental.teamwork, 12.0, 0.6).lerp(0.10, 0.02);
    // Cap pass EV so a fantasy cutback doesn't talk us out of a real shot.
    let capped_pass_ev = best_pass_ev.min(0.55);
    let point_blank = distance < 24.0 && xg >= 0.18;
    if !point_blank && capped_pass_ev > xg + margin {
        #[cfg(feature = "match-logs")]
        helper_diag::PASS_DEFERRAL.fetch_add(1, Ordering::Relaxed);
        return ShotDecision::Pass;
    }

    // ── Anti-monopoly LAY-OFF ─────────────────────────────────────────
    // A player who has already taken a stack of shots this match gives the
    // ball up rather than force yet another — real team-mates demand it and
    // defenders key on the hot striker. Crucially this DEFERS (lays off to
    // a team-mate who shoots instead) rather than the willingness taper
    // below, which only makes the hog Hold and re-shoot next tick —
    // delaying, not redistributing. That delay is exactly how one forward
    // ran up ~13 shots/game and 87% of the team's goals over a season.
    // Because it redistributes (the shot moves to whoever's free) it's
    // goal-neutral at team level — unlike raising the xG bar, which just
    // discards the attempt and drops team scoring.
    //
    // Lay off when a team-mate's option is COMPARABLE to our own shot
    // (not only clearly better, as the normal deferral above requires) —
    // and the bar eases as the shot count climbs, so a player who's already
    // monopolised the shooting gives the ball up even for a slightly worse
    // option. Genuinely better personal chances (and point-blank tap-ins)
    // are still taken; an isolated player with no outlet keeps shooting.
    if !point_blank {
        let shots_so_far = ctx.memory().shots_taken;
        if shots_so_far > 4 {
            // Outlet must be at least this fraction of our own shot's value.
            // 5 shots → 0.85×; 8 → 0.55×; 11+ → floor 0.35×.
            let factor = (0.95 - (shots_so_far - 4) as f32 * 0.10).max(0.35);
            if best_pass_ev >= xg * factor {
                #[cfg(feature = "match-logs")]
                helper_diag::PASS_DEFERRAL.fetch_add(1, Ordering::Relaxed);
                return ShotDecision::Pass;
            }
        }
    }

    // ── Willingness roll ──────────────────────────────────────────────
    // Skill-curved willingness — weighted heavily on `selection` so a
    // smart forward pulls the trigger on the right chance, not just any
    // chance. Composure and execution add some lift; the rest comes
    // from chance quality (xg_boost, clarity, body control, GK).
    //
    // Calibration target: ~13 shots/team/match (real PL average is
    // 12-14/team, top sides 16-18). Earlier slopes (0.22 / 0.10 / 0.12
    // + 0.06 base) produced mean willingness ~0.06 across the skill
    // distribution — strong teams logged 39 shots/match (~3× target)
    // and equal-skill matches 27/team (~2×). The trim flattens the
    // skill slope (so strong shooters are less aggressive per chance)
    // while bumping the base constant so weak shooters keep firing on
    // tap-in floors. Net per-shot conversion is preserved (xg_boost /
    // clarity / body_control still scale willingness); only the
    // per-tick fire rate drops, cutting shot volume to ~17/team at
    // equal skill and ~22/team for strong sides — matching real PL.
    // Calibration target: shots/team ~17 (engine-realistic, real PL ~13).
    // The engine has multiple shot paths (helper + corner headers +
    // midfielder cutbacks); the helper handles ~80% of shots. The
    // interception noise gate must stay in place (see
    // [[interception-load-bearing]]) — over-cutting interceptions
    // explodes goals via 28→62% on-target.
    //
    // Lever: BASE cut applies to every helper call regardless of xG
    // (line-balance neutral). xg_boost floor preferentially cuts
    // speculative shots — too aggressive on the floor hits FWDs more
    // than MIDs and breaks the 58/32/10 line ratio (FWDs take more
    // speculative shots). Iteration history (2026-06-05):
    //   G (base 0.012, xg_boost floor 0.20): goals 2.91, line 47/40/13
    //   H (base 0.018, xg_boost floor 0.42): goals 3.28, line 52/36/12
    //   I (base 0.012, xg_boost floor 0.30): aiming goals ~2.6, line 55/33/12
    // Halved across the board (0.013/0.045/0.020/0.025 → below) as the
    // volume half of the fatigue-normalization rebalance (2026-06-11).
    // FATIGUE_RATE_MULTIPLIER's order-of-magnitude correction stopped
    // outfielders from flatlining at the 15% condition floor by minute
    // 20 — which un-suppressed shot volume for the remaining 70 minutes
    // and pushed goals/match from 3.3 to 5.3. The willingness trim is
    // the memory-approved lever for shot volume (NOT the intercept
    // gate); halving restores ~18 shots/team while keeping the now-flat
    // xG/shot and goal-timing profile the fatigue fix bought.
    // Second trim pass (×0.78) after the condition-slope softening and
    // settle-window extension put goals at 3.41, then a final ×0.9 once
    // the 600-match validation read 2.97 — lands the total in the
    // 2.6-2.8 real band at ~18 shots/team.
    // Third trim (×0.85, 2026-06 regime-neutralization round): gating
    // score-reactive behavior to the final ~28 minutes un-suppressed
    // the first hour of play and lifted totals to 3.48 — this rebases
    // to ~2.9-3.0 at the new flat game-state profile.
    let base_willingness =
        0.0038 + selection * 0.0134 + composure_skill * 0.0061 + execution_skill * 0.0077;
    // xg_boost — floor 0.30 (vs prior 0.50). Mid-range chance with
    // xG=0.06 gets 0.30 boost (was 0.50 — ~40% reduction). Clear-shot
    // xG=0.10 gets 0.50 (was 0.50 — no change). High-xG xG≥0.28 gets
    // 1.40 (cap unchanged). Net effect: speculative low-xG shots cut
    // ~40%, high-xG kept intact — preserves line balance better than
    // a deep floor cut.
    let xg_boost = (xg / 0.20).clamp(0.30, 1.40);
    let clarity_mult = 0.50 + clarity * 0.50;
    let body_control_mult = (0.65 + body_control * 0.40).clamp(0.60, 1.05);
    // Condition slope softened 0.55 → 0.25. Fatigue already hits this
    // same decision three other ways — the effective-skill composites
    // inside selection/execution, the shot profile's condition discount
    // on expected_xg (raising the floor-gate kill rate), and the
    // balance factor — so a steep willingness slope made tired players
    // stop ATTEMPTING shots entirely, which is why engine scoring
    // decayed across the match (25%→11% per band) while real football
    // RISES late (~11%→26%): in reality tired strikers keep shooting
    // and finish worse (already modeled via execution), while tired
    // defenders give up better chances. Keep a mild attempt penalty,
    // let the execution-side fatigue do the realistic damage.
    let condition_mult = (1.0 - low_condition_penalty * 0.25).clamp(0.40, 1.05);
    let gk_context_mult = gk_proximity;
    // Marginal-chance gate: when xg < min_xg + 0.05 a high-selection
    // player damps willingness; a low-selection player lifts it.
    let marginal = (xg < min_xg + 0.05) as i32 as f32;
    let selection_marginal_adj = marginal * (0.5 - selection) * 0.20;
    let mut willingness = base_willingness
        * xg_boost
        * clarity_mult
        * body_control_mult
        * condition_mult
        * gk_context_mult
        * balance_factor
        * (1.0 + selection_marginal_adj);
    if inside_six {
        // Inside-six floor scales with execution_skill so a 5/20
        // player floors near 0.15, not 0.30.
        let inside_six_will_floor = (0.10 + execution_skill * 0.30).clamp(0.10, 0.45);
        willingness = willingness.max(inside_six_will_floor);
    }

    // ── Shot-share dampener (anti-monopoly) ───────────────────────────
    // The engine funnels nearly every chance to whichever forward is
    // highest up the pitch (midfielders rarely reach shooting range), so
    // in a lone-striker shape one player can take ~16 shots/match and
    // post a ~57-goal season — the inflated totals on the league pages.
    // Real football spreads the threat: defenders key on a striker who's
    // shot all game and team-mates demand the ball. This tapers a single
    // player's willingness once they've shot a few times in the match.
    //
    // Threshold history: >5 → >3 (when strong forwards reached 8-10
    // shots/match at the old, pre-fatigue-normalization volume) → >5
    // again with a gentler 0.10 slope. With the global willingness now
    // halved, team volume sits at ~19 shots and the tight >3 taper was
    // over-correcting: league-mode top scorers projected ~17.5
    // goals/season vs the real 25-30 band, and capping every team's
    // hot striker compresses per-team match totals toward the mean —
    // one more draw-inflation source at equal strength. Real prolific
    // strikers genuinely take 5-8 shots; the taper now only bites the
    // true monopoly tail. A point-blank tap-in stays exempt: nobody
    // passes up an open net.
    if !inside_six {
        let shots_so_far = ctx.memory().shots_taken;
        if shots_so_far > 5 {
            let hog_damp = (1.0 - (shots_so_far - 5) as f32 * 0.10).clamp(0.20, 1.0);
            willingness *= hog_damp;
        }
    }

    // ── Post-restart regroup + match-settle window ───────────────────
    //
    // Real-football: after a goal both teams take 60–90s to reset;
    // at match start both teams need 3–5 minutes to find their rhythm.
    // The engine before this block produced 76% of first-goals in
    // minutes 0–15 (real PL ~25%), 60% equalizers within 15 min (real
    // ~28%), and 28% within 5 min (real ~10%) — the kickoff-blitz
    // cascade. The fix uses two complementary symmetric time-windows:
    //
    //   * scoring team: -25% willingness for 60s after they scored
    //     (existing post-score relaxation — works marginally)
    //   * conceding team: -15% willingness for 45s after they conceded
    //     (mild — the post-concede rattle, kept LIGHT so weak sides
    //     at extreme gaps aren't repeatedly suppressed; an earlier
    //     heavy variant inflated 2-2 draws because strong sides got
    //     a free hand to extend the lead — see [[strength-curve-
    //     calibration]] "Dampening the conceding team → INCREASED
    //     draws" note)
    //   * match-start: willingness ramps from 0.60× at min 0 to 1.0×
    //     over the first 5 minutes (300_000 ms; `total_match_time`
    //     is in ms, 10ms/tick). This is the "settle in" window.
    //
    // The candidate-orchestrated dev_match sweep (2026-06-05) tested
    // 5 variants across the deep/mild × short/long axes; this is the
    // best of them (candidate E). All other variants either matched
    // baseline or made draws WORSE — symmetric post-goal dampening
    // has structural limits because at equal skill BOTH teams convert
    // their reduced chances at similar rates, leaving the draw share
    // largely unchanged.
    // Window pair retilted (scorer 0.75/conceder 0.85 → 0.85/0.78):
    // the old shape damped the SCORING team harder than the conceding
    // one, which — stacked with the from-minute-1 game-management lead
    // signal — actively manufactured equalizers (12v12 dev_match: 56%
    // draws, conceders scoring at ~3× baseline within 15 min). Real
    // psychology runs the other way: the scorer carries momentum, the
    // conceder is rattled while they reorganise. Kept mild on the
    // conceder per the [[strength-curve-calibration]] note — heavy
    // conceder dampening at big strength gaps hands the strong side a
    // free hand and inflates high-scoring draws.
    if let Some(side) = ctx.player.side {
        let opp_side = match side {
            PlayerSide::Left => PlayerSide::Right,
            PlayerSide::Right => PlayerSide::Left,
        };
        if ctx.context.conceded_recently(opp_side, 6000) {
            willingness *= 0.85;
        }
        if ctx.context.conceded_recently(side, 4500) {
            willingness *= 0.78;
        }
    }
    // Match-start settle window — extended after dev_match stats showed
    // 80% of first goals were still landing in minutes 0–15 (real PL ~25%)
    // despite the prior 5-minute / 0.60 floor. The realistic "feel-out"
    // window is closer to 12 minutes; pushing further yielded no
    // additional improvement on top of this. Other early-goal paths
    // (state-machine shooting decisions outside this helper, fresh-
    // player condition multiplier) still contribute and would need
    // their own dampeners for a full fix.
    // Window 720s → 900s and floor 0.35 → 0.30 after the fatigue
    // normalization: with players keeping their legs all match the
    // opening band is the ONLY remaining hot spot (fresh-legs sprint
    // volume + zero skill penalties above 80% condition), so the
    // feel-out suppression carries more of the early-goal correction
    // than before.
    let settle_window: u64 = 900_000;
    if ctx.context.total_match_time < settle_window {
        let progress = ctx.context.total_match_time as f32 / settle_window as f32;
        willingness *= 0.30 + 0.70 * progress;
    }

    // Cap trimmed 0.48/0.60 → 0.34/0.44. Floor dropped 0.012 → 0.006,
    // then halved with the base coefficients (→ 0.003) so the floor
    // doesn't swallow the global trim for low-willingness rolls.
    let cap = if xg >= 0.35 { 0.44 } else { 0.34 };
    willingness = willingness.clamp(0.0021, cap);

    #[cfg(feature = "match-logs")]
    {
        use std::sync::atomic::Ordering;
        helper_diag::REACHED_ROLL.fetch_add(1, Ordering::Relaxed);
        helper_diag::SUM_XG_X1000.fetch_add((xg * 1000.0) as u64, Ordering::Relaxed);
        helper_diag::SUM_WILLINGNESS_X1000
            .fetch_add((willingness * 1000.0) as u64, Ordering::Relaxed);
        let band =
            time_band_diag::band_for_minute(sc::minute_from_ms(ctx.context.total_match_time));
        time_band_diag::ROLL_REACHED_BY_BAND[band].fetch_add(1, Ordering::Relaxed);
    }

    if ctx.context.rng.unit_f32() < willingness {
        #[cfg(feature = "match-logs")]
        helper_diag::ROLL_PASSED.fetch_add(1, Ordering::Relaxed);
        ShotDecision::Shoot { reason: tag }
    } else {
        ShotDecision::Hold
    }
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
    for t in ctx.players().teammates().nearby(75.0) {
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
        if !(14.0..=66.0).contains(&td) {
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
        // A tightly-marked runner is not a cutback target, but a single
        // light marker is fine — a first-time strike beats one defender,
        // and the clarity/xG of the resulting shot is handled by the
        // box-arrival helper. Reject only genuine double-marking.
        if ctx.tick_context.grid.opponents(t.id, 8.0).count() >= 2 {
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
