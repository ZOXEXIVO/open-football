use crate::r#match::StateProcessingContext;
use crate::r#match::player::strategies::players::skills::SkillCurve;
use crate::r#match::player::strategies::players::ops::skill_composites as sc;

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
        helper_diag::HOLD_HARDGATE.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        return ShotDecision::Hold;
    }

    let distance = ctx.ball().distance_to_opponent_goal();
    // Anything beyond the absolute long-range cap is hopeless even
    // for elite long-shooters — keep the ball.
    if distance > 110.0 {
        #[cfg(feature = "match-logs")]
        helper_diag::HOLD_FAR.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
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
    let inside_six = distance <= 18.0;
    // Inside-six floor: skill-graded, so a 5/20 player floors near 0.15
    // instead of inheriting the unconditional 0.30 free pass.
    let inside_six_floor = 0.12 + execution_skill * 0.28;
    if !inside_six && xg < min_xg {
        #[cfg(feature = "match-logs")]
        helper_diag::HOLD_XG.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        return ShotDecision::Hold;
    }
    if inside_six && xg < (inside_six_floor.min(min_xg)) {
        #[cfg(feature = "match-logs")]
        helper_diag::HOLD_INSIDE_SIX_XG.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        return ShotDecision::Hold;
    }

    // ── Clear shot ────────────────────────────────────────────────────
    let clarity = ctx.player().shot_clarity();
    if !ctx.player().has_clear_shot() && !inside_six {
        #[cfg(feature = "match-logs")]
        helper_diag::HOLD_NO_CLEAR.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
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

    // Margin tightened — even a smart-passing forward should not
    // lay off a viable shot every time a teammate is somewhere upfield.
    // Sigmoid-blended (pivot 12/20 teamwork) so the margin sweeps
    // smoothly from 0.14 at low teamwork to 0.06 at high — instead of
    // 3 hard tiers where 14/20 and 11/20 land in the same bucket.
    let margin = SkillCurve::new(skills.mental.teamwork, 12.0, 0.6).lerp(0.14, 0.06);
    // Cap pass EV so a fantasy cutback doesn't talk us out of a real shot.
    let capped_pass_ev = best_pass_ev.min(0.55);
    let point_blank = distance < 24.0 && xg >= 0.18;
    if !point_blank && capped_pass_ev > xg + margin {
        #[cfg(feature = "match-logs")]
        helper_diag::PASS_DEFERRAL.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        return ShotDecision::Pass;
    }

    // ── Willingness roll ──────────────────────────────────────────────
    // Skill-curved willingness — weighted heavily on `selection` so a
    // smart forward pulls the trigger on the right chance, not just any
    // chance. Composure and execution add some lift; the rest comes
    // from chance quality (xg_boost, clarity, body control, GK).
    //
    // Calibration target: ~13 shots/team/match. With ~440
    // reach-roll ticks/team/match (the count of clear-shot-in-range
    // ticks that pass min_xg and pass-EV), required mean willingness ≈
    // 0.030. Earlier coefficients produced mean 0.018 (floor-bound)
    // → ~5 shots/team. New coefficients land mean ~0.035 across the
    // skill distribution while keeping low-skill shots rare and elite
    // shots ~3× more frequent than poor.
    let base_willingness =
        0.06 + selection * 0.22 + composure_skill * 0.10 + execution_skill * 0.12;
    // xg_boost — lift the floor on the multiplicative chain so a 0.04
    // xG shot still has ~50% of the elite-chance willingness,
    // not 30%. Real football: even speculative long shots get fired
    // every other minute when the lane is open.
    let xg_boost = (xg / 0.20).clamp(0.50, 1.40);
    let clarity_mult = 0.50 + clarity * 0.50;
    let body_control_mult = (0.65 + body_control * 0.40).clamp(0.60, 1.05);
    let condition_mult = (1.0 - low_condition_penalty * 0.55).clamp(0.40, 1.05);
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
    let cap = if xg >= 0.35 { 0.60 } else { 0.48 };
    willingness = willingness.clamp(0.012, cap);

    #[cfg(feature = "match-logs")]
    {
        use std::sync::atomic::Ordering;
        helper_diag::REACHED_ROLL.fetch_add(1, Ordering::Relaxed);
        helper_diag::SUM_XG_X1000.fetch_add((xg * 1000.0) as u64, Ordering::Relaxed);
        helper_diag::SUM_WILLINGNESS_X1000
            .fetch_add((willingness * 1000.0) as u64, Ordering::Relaxed);
    }

    if rand::random::<f32>() < willingness {
        #[cfg(feature = "match-logs")]
        helper_diag::ROLL_PASSED.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        ShotDecision::Shoot { reason: tag }
    } else {
        ShotDecision::Hold
    }
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
