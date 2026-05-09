use crate::r#match::StateProcessingContext;

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
    // ── Hard gates ────────────────────────────────────────────────────
    let can_team = ctx.team().can_shoot();
    let can_player = ctx.player().can_shoot();
    if !can_team || !can_player {
        return ShotDecision::Hold;
    }

    let distance = ctx.ball().distance_to_opponent_goal();
    // Anything beyond the absolute long-range cap is hopeless even
    // for elite long-shooters — keep the ball.
    if distance > 110.0 {
        return ShotDecision::Hold;
    }

    let skills = &ctx.player.skills;
    let finishing = (skills.technical.finishing / 20.0).clamp(0.0, 1.0);
    let composure = (skills.mental.composure / 20.0).clamp(0.0, 1.0);
    let _technique = (skills.technical.technique / 20.0).clamp(0.0, 1.0);
    let first_touch = (skills.technical.first_touch / 20.0).clamp(0.0, 1.0);
    let decisions = (skills.mental.decisions / 20.0).clamp(0.0, 1.0);

    // ── xG quality ────────────────────────────────────────────────────
    // Pre-shot xG (matches `handle_shoot_event`'s formula). Low-xG
    // attempts are rejected outright unless we're inside the 6-yard
    // box where ANY shot has a meaningful chance.
    let xg = ctx.player().shooting().expected_xg();
    // Skill-aware xG floor:
    //   * elite finisher (≥0.80) — accepts 0.06 xG  (will speculate)
    //   * average     (~0.50)    — accepts 0.09 xG
    //   * poor       (≤0.30)     — needs 0.13 xG
    let mut min_xg = 0.13 - finishing * 0.08;
    min_xg = min_xg.clamp(0.06, 0.13);
    let inside_six = distance <= 18.0;
    if !inside_six && xg < min_xg {
        return ShotDecision::Hold;
    }

    // ── Clear shot ────────────────────────────────────────────────────
    let clarity = ctx.player().shot_clarity();
    if !ctx.player().has_clear_shot() && !inside_six {
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
    // shot even when a teammate has a tap-in waiting.
    let teamwork = (skills.mental.teamwork / 20.0).clamp(0.0, 1.0);
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
            // Receiver-xG approximation: closer to goal + clear of
            // markers = higher value pass.
            let pass_xg = if receiver_d < 40.0 {
                0.45
            } else if receiver_d < 60.0 {
                0.30
            } else if receiver_d < 80.0 {
                0.18
            } else {
                0.10
            };
            (pass_xg * mark_factor) * (0.6 + progression * 0.4)
        })
        .unwrap_or(0.0);

    let margin = if teamwork > 0.75 {
        0.04
    } else if teamwork < 0.40 {
        0.12
    } else {
        0.08
    };
    // Cap pass EV so a fantasy cutback doesn't talk us out of a real shot.
    let capped_pass_ev = best_pass_ev.min(0.55);
    let point_blank = distance < 24.0 && xg >= 0.18;
    if !point_blank && capped_pass_ev > xg + margin {
        return ShotDecision::Pass;
    }

    // ── Willingness roll ──────────────────────────────────────────────
    // Per-tick base 0.10 (poor) … 0.35 (elite). Modulated by clarity,
    // sprint balance, GK proximity, and xG quality. Inside the six-yard
    // box willingness floors at 0.30 so a scuffed bobble there still
    // resolves into a strike most of the time.
    let base = (finishing * 0.22 + composure * 0.06 + decisions * 0.04 + 0.07).clamp(0.10, 0.35);
    let xg_boost = (xg / 0.30).clamp(0.30, 1.20);
    let mut willingness =
        base * (0.40 + clarity * 0.60) * balance_factor * xg_boost * gk_proximity;
    if inside_six {
        willingness = willingness.max(0.30);
    }
    willingness = willingness.clamp(0.0, 0.60);

    if rand::random::<f32>() < willingness {
        ShotDecision::Shoot { reason: tag }
    } else {
        ShotDecision::Hold
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Test the pure scoring math via a small parameterised helper. We
    // can't easily fixture a `StateProcessingContext` so we extract the
    // willingness formula and verify monotonicity directly.
    fn willingness(
        finishing: f32,
        composure: f32,
        decisions: f32,
        clarity: f32,
        balance_factor: f32,
        xg: f32,
        gk_proximity: f32,
        inside_six: bool,
    ) -> f32 {
        let base =
            (finishing * 0.22 + composure * 0.06 + decisions * 0.04 + 0.07).clamp(0.10, 0.35);
        let xg_boost = (xg / 0.30f32).clamp(0.30, 1.20);
        let mut w = base * (0.40 + clarity * 0.60) * balance_factor * xg_boost * gk_proximity;
        if inside_six {
            w = w.max(0.30);
        }
        w.clamp(0.0, 0.60)
    }

    #[test]
    fn elite_finisher_more_willing_than_mediocre() {
        // Same chance, same context — elite striker pulls the trigger
        // about twice as often as the mediocre one.
        let mediocre = willingness(0.50, 0.40, 0.35, 0.6, 1.0, 0.10, 1.0, false);
        let elite = willingness(0.85, 0.75, 0.70, 0.6, 1.0, 0.10, 1.0, false);
        assert!(elite > mediocre * 1.4, "elite={elite} mediocre={mediocre}");
    }

    #[test]
    fn xg_floor_scales_with_finishing() {
        // Skill-aware xG floor: poor finishers need ~0.13, elite need ~0.06.
        let poor_floor = (0.13_f32 - 0.30 * 0.08).clamp(0.06, 0.13);
        let elite_floor = (0.13_f32 - 0.85 * 0.08).clamp(0.06, 0.13);
        assert!(poor_floor > elite_floor + 0.04);
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
    fn inside_six_floors_willingness() {
        // Even hopeless context (clarity 0.0, xg 0.05, balance 0.55):
        // inside-six floor lifts to 0.30 so a scuffed close-range
        // chance still becomes a shot most of the time.
        let w = willingness(0.30, 0.30, 0.30, 0.0, 0.55, 0.05, 1.0, true);
        assert!(w >= 0.30 - f32::EPSILON, "w={w}");
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
