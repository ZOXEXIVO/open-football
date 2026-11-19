use crate::PlayerPositionType;
use crate::r#match::engine::psychology::Psychology;
use crate::r#match::events::Event;
use crate::r#match::forwarders::states::ForwardState;
use crate::r#match::forwarders::states::common::{
    ActivityIntensity, ForwardCondition, ForwardRead, ForwardShot,
};
use crate::r#match::midfielders::states::common::{CarryHeading, LaneAhead};
#[cfg(feature = "match-logs")]
use crate::r#match::player::diagnostics::forward_census::{
    ForwardCensus, FwdLimit, FwdRequest, FwdVerdict,
};
use crate::r#match::player::events::{PassingEventContext, PlayerEvent};
use crate::r#match::player::strategies::common::players::ops::clearance::TouchlineCall;
use crate::r#match::player::strategies::common::passing::{FlankAction, FlankPlay};
use crate::r#match::player::strategies::common::players::ops::forward_shot_decision::{
    BallCarry, ShotDecision, evaluate_forward_shot_decision,
};
use crate::r#match::player::strategies::players::skills::SkillCurve;
#[cfg(feature = "match-logs")]
use crate::r#match::engine::engine::shot_gate_stats;
use crate::r#match::engine::engine::stuck_exit_stats;
use crate::r#match::{
    ConditionContext, MatchPlayerLite, PlayerSide, StateChangeResult, StateProcessingContext,
    StateProcessingHandler, SteeringBehavior,
};
use nalgebra::Vector3;
use std::cmp::Ordering;

/// Depth of the penalty area from the goal line (16.5 m). Inside this, a
/// forward with the ball is in a shooting position by definition.
const PENALTY_AREA_DEPTH: f32 = 132.0;

#[derive(Default, Clone)]
pub struct ForwardRunningState {}

impl StateProcessingHandler for ForwardRunningState {
    fn process(&self, ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        // `Running` is the man on the ball; without it the read answers.
        if !ctx.player.has_ball(ctx) {
            return Some(ForwardRead::change(ctx));
        }

        // Pinned at his own touchline with no safe out: over the line, up
        // it — the rule every role plays by (`TouchlineCall`).
        if TouchlineCall::decide(ctx).is_some() {
            return Some(StateChangeResult::with_forward_state_and_event(
                ForwardRead::released(ctx),
                TouchlineCall::put_out(ctx),
            ));
        }

        let distance_to_goal = ctx.ball().distance_to_opponent_goal();

        // ── Snapshot under heavy pressure ──────────────────────────
        //
        // A forward who has JUST received the ball (his first 0.16 s
        // on it) within 7.5 m of goal (60u) with
        // a defender right on him (within 1.25 m, 10u) fires
        // immediately — real football's "first-time shot" pattern. Without this, the forward enters the
        // normal decision tree, applies first-touch + control
        // settling + pass-EV deferral + willingness roll, and by the
        // time those gates resolve the defender has tackled them out
        // of the buildup. The audit data showed weak-side forwards
        // reaching the final third 65×/match but only shooting ~5
        // times — almost every chance died here.
        //
        // Tight gates (a defender within 10u, 60u from goal, 0.16 s
        // on the ball) keep the trigger asymmetric: a STRONG forward is
        // rarely that close to a defender at the moment of reception
        // (their team's structure protects them and they get the
        // ball with space). A WEAK forward facing a strong defence
        // gets a tight defender on every reception. The wider
        // 12u/70u version fired equally for both sides and added
        // strong-side shots; this 8u/50u tightening keeps the
        // benefit asymmetric.
        //
        // The snapshot routes through Shooting state's existing
        // physics (tagged `FWD_SNAPSHOT_PRESSED`). Error scaling,
        // body-control penalty, and GK reach all still apply — so a
        // weak striker firing a rushed snapshot still misses the
        // target more often than not. The contribution is per-shot
        // VOLUME, not per-shot quality.
        if Self::just_received(ctx) && distance_to_goal < 60.0 {
            // Asymmetric snapshot gate: the forward only fires
            // instinctively when they KNOW they can't out-touch the
            // defender — i.e. the nearest opposing tackler's skill
            // is higher than their own first-touch. A skilled
            // forward (first_touch > defender's tackling) trusts
            // their control and takes the time to set up the chance;
            // a weaker forward facing a strong defender snapshots
            // before the inevitable tackle. This is the canonical
            // real-football skill calculus and keeps the trigger
            // calibration-neutral at equal skill (same skill on both
            // sides → snapshot rarely fires) while asymmetrically
            // unlocking shots for weak teams against strong
            // defenders.
            //
            // A "desperate snapshot" variant (looser gates when
            // losing by ≥2 goals) was tried and reverted: it added
            // shot VOLUME but tanked per-shot xG so badly (0.066 →
            // 0.055) that net goals dropped. Quality of opportunity
            // matters more than quantity for low-skill teams.
            let nearest_threat = ctx
                .players()
                .opponents()
                .nearby_raw(10.0)
                .min_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(Ordering::Equal))
                .map(|(id, _)| id);
            if let Some(threat_id) = nearest_threat {
                let defender_tackling = ctx.player().skills(threat_id).technical.tackling;
                let attacker_first_touch = ctx.player.skills.technical.first_touch;
                // Buffer 1.0 → 0.5 — fires at SMALLER skill gaps
                // too. At equal skill (both ~11) the gate still
                // doesn't fire (11 < 10.5 is false), so calibration
                // remains neutral. But at gap 3-5 buckets (where
                // defender tackling exceeds attacker first_touch
                // by ~1) the gate now triggers, covering the
                // moderate-asymmetry case the original buffer
                // missed.
                // Chance-quality floor: the snapshot skips the
                // unified helper (pending_shot_reason bypasses it in
                // Shooting), so it must carry its own xG bar — a
                // pressured snapshot is still a SHOT SELECTION, not
                // a reflex to fire from anywhere. 0.15 ≈ a clear
                // look inside ~11m for an ordinary finisher.
                // Probability gate: this path bypasses the unified
                // helper entirely (it tags a reason, so Shooting
                // skips the roll), and its window spans 0.16 s —
                // so without one it fires essentially every time
                // its conditions hold. A pressured snapshot is a
                // real football action, not an automatic one.
                if attacker_first_touch < defender_tackling - 0.5
                    && ctx.player().shooting().expected_xg() >= 0.15
                    && ctx.context.rng.unit_f32() < 0.10
                {
                    #[cfg(feature = "match-logs")]
                    {
                        ForwardCensus::note_limit(ctx, FwdLimit::Snapshot);
                        ForwardCensus::note_shot_site(
                            ctx,
                            FwdRequest::Snapshot,
                            FwdVerdict::Approved,
                        );
                    }
                    return Some(
                        StateChangeResult::with_forward_state(ForwardState::Shooting)
                            .with_shot_reason("FWD_SNAPSHOT_PRESSED"),
                    );
                }
            }
        }
        // PATIENT POSSESSION (forward recycle): only fires when the
        // forward is genuinely STUCK — outside the finishing zone,
        // boxed in by multiple opponents, with no progressive path
        // to goal. The previous version triggered on any
        // `should_play_possession()` reading, which made forwards
        // recycle every time the team won the ball (300-tick
        // post-possession window) regardless of whether the
        // forward had a path forward. That single condition was
        // the dominant shot-volume suppressor.
        //
        // The new gate keeps the safety valve (forward genuinely
        // can't progress → lay off) without abandoning the attack
        // every time the team enters possession mode.
        let under_pressure = ctx.player().pressure().is_under_immediate_pressure();
        let stuck = ctx.players().opponents().nearby(15.0).count() >= 2
            && !self.has_open_space_ahead(ctx);
        if !under_pressure
            && stuck
            && distance_to_goal > 50.0
            && ctx.tick_context.ball.ownership_duration > 10
            && ctx.team().should_play_possession()
        {
            let player_pos = ctx.player.position;
            let goal_pos = ctx.player().opponent_goal_position();
            let to_goal = (goal_pos - player_pos).normalize();
            let safe_outlet = ctx
                .players()
                .teammates()
                .nearby(180.0)
                .filter(|t| {
                    if t.id == ctx.player.id {
                        return false;
                    }
                    let dist = (t.position - player_pos).magnitude();
                    if dist < 25.0 || dist > 180.0 {
                        return false;
                    }
                    let to_t = (t.position - player_pos).normalize();
                    let fwd = to_t.dot(&to_goal);
                    if fwd >= 0.3 {
                        return false;
                    }
                    ctx.tick_context.grid.opponents(t.id, 12.0).count() < 2
                })
                .filter_map(|t| {
                    let odds = ctx.player().clear_pass(t.id)?;
                    let preference = if t.tactical_positions.is_midfielder() {
                        10.0
                    } else {
                        0.0
                    };
                    Some((t, odds.price(preference)))
                })
                .max_by(|a, b| a.1.total_cmp(&b.1))
                .map(|(t, _)| t);
            if let Some(target) = safe_outlet {
                #[cfg(feature = "match-logs")]
                ForwardCensus::note_pass_site(ctx, FwdRequest::PatientPossession);
                return Some(StateChangeResult::with_forward_state_and_event(
                    ForwardRead::released(ctx),
                    Event::PlayerEvent(PlayerEvent::PassTo(
                        PassingEventContext::builder()
                            .with_from_player_id(ctx.player.id)
                            .with_to_player_id(target.id)
                            .with_reason("FWD_PATIENT_POSSESSION")
                            .build(ctx),
                    )),
                ));
            }
        }

        // Priority 0: point blank, inside six metres
        // (`ForwardShot::POINT_BLANK`).
        // Routed through the centralised `evaluate_forward_shot_decision`
        // so the same xG / sprint-balance / 1v1 / pass-EV / willingness
        // gates apply that the rest of the forward states use. The
        // previous bespoke 0.025..0.10 per-tick curve here was the
        // last in-tree willingness duplicate that diverged from the
        // helper, and Shooting state's `pending_shot_reason.is_some()`
        // skip meant nothing else was checking these gates after
        // the transition.
        if distance_to_goal <= ForwardShot::POINT_BLANK {
            let point_blank = evaluate_forward_shot_decision(ctx, "FWD_RUN_POINT_BLANK");
            #[cfg(feature = "match-logs")]
            ForwardCensus::note_shot_site(
                ctx,
                FwdRequest::PointBlank,
                Self::verdict(&point_blank),
            );
            match point_blank {
                ShotDecision::Shoot { reason } => {
                    // Inside the box this is a FINISH, not a shot from
                    // range: `Finishing` carries no distance-based
                    // abort gates, which is what a tap-in needs (the
                    // Shooting state would happily reject a six-yard
                    // chance from a sub-0.5 finisher).
                    return Some(
                        StateChangeResult::with_forward_state(ForwardState::Finishing)
                            .with_shot_reason(reason),
                    );
                }
                ShotDecision::Pass => {
                    #[cfg(feature = "match-logs")]
                    ForwardCensus::note_pass_site(ctx, FwdRequest::PassOther);
                    return Some(StateChangeResult::with_forward_state(ForwardState::Passing));
                }
                ShotDecision::Hold => {
                    // "Hesitated this tick — stay in Running and
                    // re-roll next tick" is what this used to say, and
                    // THERE IS NO RE-ROLL. The shot threshold is drawn
                    // once per POSSESSION (see the note at `spread`),
                    // so a forward who declines a point-blank chance
                    // declines it for as long as he keeps the ball —
                    // and `return None` keeps him in `Running`, where
                    // the only thing left to do is carry. He circles
                    // in front of goal until somebody takes it off him
                    // or the stall detector fires 20 s later.
                    //
                    // Measured with a stuck-exit census over exactly
                    // the ticks the stall table counts (owner 250-plus
                    // AI ticks into `Running`): **97% of them leave
                    // through this branch**, at a mean speed of 0.245
                    // u/tick — not frozen, circling. It is the last
                    // genuine freeze in the ball-stuck table and the
                    // same fault as the box `Hold` one block below.
                    //
                    // The appetite itself is NOT fixed — distance,
                    // clarity and pressure all move — so a short hold
                    // is worth having: a touch that improves the angle
                    // can clear the standing bar. What is not worth
                    // having is an unbounded one. Past the cap the
                    // chance has not materialised and he squares it,
                    // which is what a striker who did not fancy it
                    // actually does. Going backwards out of the area
                    // is separately prevented by
                    // `PassEvaluator::retreat_penalty`.
                    stuck_exit_stats::note(1, ctx.in_state_time, ctx.player.velocity.norm());
                    if Self::on_the_ball(ctx) > Self::POINT_BLANK_PATIENCE {
                        #[cfg(feature = "match-logs")]
                        {
                            ForwardCensus::note_limit(ctx, FwdLimit::PointBlankPatience);
                            ForwardCensus::note_pass_site(ctx, FwdRequest::PassOther);
                        }
                        return Some(StateChangeResult::with_forward_state(
                            ForwardState::Passing,
                        ));
                    }
                    return None;
                }
            }
        }

        // Teammate-in-better-position pass-over-shot heuristic.
        // Real strikers pick PASS over SHOT when a teammate is in a
        // significantly better position — even with a clear sight
        // of goal. The old logic fired on any "clear shot in range"
        // which produced hat-tricks every match; in real football
        // only ~half of clear shooting opportunities become shots,
        // the rest become assists.
        let defer_to_teammate = self.has_teammate_with_much_better_shot(ctx, distance_to_goal);

        // Flat 320u (40 m) ceiling — every forward attempts, and the
        // xG/accuracy maths inside the helper decides whether the
        // shot is good. Skill-stratified caps double-punished low-fin
        // strikers (penalised by xG already and by willingness, then
        // additionally vetoed by a hard distance cap).
        // Reachability gate only — the helper owns the decision and
        // the per-player range. See `MAX_SHOOTING_DISTANCE`.
        let max_shot_distance = 320.0f32;
        // Gate waterfall instrumentation (match-logs only). Counts
        // how many forward-has-ball ticks survive each independent
        // pre-gate (everything except the final helper roll, which
        // is bumped after the helper returns Shoot).
        #[cfg(feature = "match-logs")]
        {
            use std::sync::atomic::Ordering;
            if distance_to_goal <= 90.0 {
                shot_gate_stats::HAS_BALL_IN_RANGE.fetch_add(1, Ordering::Relaxed);
                shot_gate_stats::PASSED_NOT_POSSESSION.fetch_add(1, Ordering::Relaxed);
                shot_gate_stats::PASSED_CAN_SHOOT.fetch_add(1, Ordering::Relaxed);
                shot_gate_stats::PASSED_SETTLED.fetch_add(1, Ordering::Relaxed);
                if !defer_to_teammate {
                    shot_gate_stats::PASSED_NOT_DEFER.fetch_add(1, Ordering::Relaxed);
                    if distance_to_goal <= max_shot_distance {
                        shot_gate_stats::PASSED_MAX_DIST.fetch_add(1, Ordering::Relaxed);
                        if ctx.player().has_clear_shot() {
                            shot_gate_stats::PASSED_CLEAR_SHOT.fetch_add(1, Ordering::Relaxed);
                        }
                    }
                }
            }
        }

        // Priority 0.5: Clear shot in range — route through the
        // unified `evaluate_forward_shot_decision` helper so PRIO 0.5,
        // PRIO 0.6 (box), point-blank, RunningInBehind, and Finishing
        // ALL share the same per-tick shot decision (xG floor,
        // clarity, sprint/balance, GK proximity, pass-EV, willingness
        // roll). Previous version kept a parallel inline willingness
        // calc that drifted ~14× too low after `shot_profile()` was
        // recalibrated, killing 99.7% of in-range clear-shot ticks
        // and turning every match into 0-0.
        //
        // Pre-gates kept locally:
        //   * !defer_to_teammate — open-net teammate gets the cutback
        //   * dist <= max_shot_distance
        //
        // GM suppression is applied as a probability gate on top of
        // the helper's Shoot decision so a leading-late team still
        // game-manages without dropping the helper's calibration.
        // NB no `has_clear_shot()` here. It was a hard veto on the
        // forward's main shot path while the midfielder's equivalent
        // (MID_SHOOT, Tier-2) has no such gate — so midfielders got
        // far more shot DECISIONS than forwards and ended up taking
        // 46-54% of all shots (real ~32%). Lane quality is priced
        // continuously by `clarity_mult` inside the helper; the
        // helper also still rejects a genuinely blocked lane.
        let pre_gate_passed = !defer_to_teammate && distance_to_goal <= max_shot_distance;

        if pre_gate_passed {
            let _decision = evaluate_forward_shot_decision(ctx, "FWD_RUN_PRIO05_CLEAR");
            #[cfg(feature = "match-logs")]
            {
                use std::sync::atomic::Ordering;
                ForwardCensus::note_shot_site(ctx, FwdRequest::Prio05, Self::verdict(&_decision));
                match _decision {
                    ShotDecision::Shoot { .. } => {
                        shot_gate_stats::HELPER_SHOOT.fetch_add(1, Ordering::Relaxed)
                    }
                    ShotDecision::Pass => {
                        shot_gate_stats::HELPER_PASS.fetch_add(1, Ordering::Relaxed)
                    }
                    ShotDecision::Hold => {
                        shot_gate_stats::HELPER_HOLD.fetch_add(1, Ordering::Relaxed)
                    }
                };
            }
            match _decision {
                ShotDecision::Shoot { reason } => {
                    #[cfg(feature = "match-logs")]
                    {
                        use std::sync::atomic::Ordering;
                        shot_gate_stats::PASSED_WILLINGNESS.fetch_add(1, Ordering::Relaxed);
                        shot_gate_stats::FIRED.fetch_add(1, Ordering::Relaxed);
                    }
                    return Some(
                        StateChangeResult::with_forward_state(ForwardState::Shooting)
                            .with_shot_reason(reason),
                    );
                }
                ShotDecision::Pass => {
                    // Helper decided pass-EV beat the shot — defer.
                    #[cfg(feature = "match-logs")]
                    ForwardCensus::note_pass_site(ctx, FwdRequest::PassOther);
                    return Some(StateChangeResult::with_forward_state(ForwardState::Passing));
                }
                ShotDecision::Hold => {
                    // Helper said "not now" — under pressure lay off,
                    // otherwise keep running and re-evaluate next tick.
                    let under_pressure = ctx.player().pressure().is_under_immediate_pressure();
                    if under_pressure {
                        #[cfg(feature = "match-logs")]
                        ForwardCensus::note_pass_site(ctx, FwdRequest::PassAfterHold);
                        return Some(StateChangeResult::with_forward_state(
                            ForwardState::Passing,
                        ));
                    }
                    // Stay in Running — keep dribbling toward goal.
                    // Don't return None unconditionally: fall through
                    // to PRIO 0.6 / 0.8 below so close-range or
                    // open-path branches still trigger.
                }
            }
        }

        // Priority 0.6: inside the penalty area (16.5 m,
        // `PENALTY_AREA_DEPTH`) with the helper having just held. It
        // is asked here as `FWD_RUN_PRIO06_BOX` whenever the clear-shot
        // block above did not ask it, and the lay-off it answers is played;
        // the shot it never approved in 300 matches is left out.
        //
        // A forward six metres out who is not shooting this instant
        // does NOT turn and lay it off. That was the reported bug in
        // its purest form: the helper says "not yet", the caller
        // answers "then pass", and the pass evaluator — which rewards
        // an isolated receiver and, for most personalities, does not
        // care which way the ball goes — finds the spare man BEHIND the
        // ball. Ball in front of goal, played backwards, every time the
        // appetite came up short.
        //
        // ⚠ BUT IT MUST NOT `return None` FOR EVER EITHER. `return`
        // skips the whole rest of the tree — the cutback, the one-two,
        // and the stall release at the bottom of it — and the appetite
        // threshold is drawn ONCE PER POSSESSION, so `Hold` is the
        // fixed answer for as long as he keeps the ball. Meanwhile
        // `BallCarry::target` collapses onto the carrier's own feet in
        // exactly this situation (`advance` is zero inside
        // `carry_hold`, and a central man has no sidestep left), and
        // the carry deadband turns that into a hard
        // `Vector3::zeros()`: a fixed point only the stall detector
        // breaks, 20 s later. Measured with the per-state dwell split:
        // `Forward: Running` was the largest genuine freeze in the game
        // — 98% of its stuck ticks with the owner 250-plus AI ticks
        // into the state, ~10.5 s an episode.
        //
        // So he holds, but not forever. Under the cap he keeps the ball
        // and works for the chance; over it the possession has become a
        // stall and the rest of the tree gets its say.
        //
        // ⚠ Do NOT drop the cap and fall through always. Measured: the
        // first branch below is the cutback to an arriving runner, so a
        // striker holding in the area handed it to a midfielder instead
        // — FWD goal share 60.7% → 49.1%, MID 38.5% → 50.4%, against a
        // real 58/32.
        if !defer_to_teammate && distance_to_goal < PENALTY_AREA_DEPTH {
            if let ShotDecision::Pass = evaluate_forward_shot_decision(ctx, "FWD_RUN_PRIO06_BOX") {
                #[cfg(feature = "match-logs")]
                ForwardCensus::note_pass_site(ctx, FwdRequest::PassOther);
                return Some(StateChangeResult::with_forward_state(ForwardState::Passing));
            }
            if Self::on_the_ball(ctx) <= Self::HOLD_PATIENCE {
                stuck_exit_stats::note(2, ctx.in_state_time, ctx.player.velocity.norm());
                return None;
            }
            #[cfg(feature = "match-logs")]
            ForwardCensus::note_limit(ctx, FwdLimit::BoxHoldPatience);
        }

        // CUTBACK TO ARRIVING RUNNER. A central midfielder arriving
        // unmarked in the box is the textbook cutback target and a
        // major source of midfielder goals in real football. We reach
        // this point only AFTER the forward's own shot blocks declined
        // (no clean shot this tick), so laying off to an unmarked
        // central runner in a better spot is strictly additive — not a
        // stolen chance. Fires only when the forward is genuinely STUCK
        // (pressured or with no open path to goal) — exactly when a
        // real forward releases the ball rather than forcing it — so it
        // stays low-volume and never trades away a chance the forward
        // could take themselves. The finder is tightly gated (runner
        // central, in range, unmarked, clear lane, ≥8u closer to goal),
        // and the runner converts via the box-arrival carve-out.
        let fwd_stuck = ctx.player().pressure().is_under_immediate_pressure()
            || !self.has_open_space_ahead(ctx);
        if distance_to_goal < 130.0 && fwd_stuck
            && let Some(runner) =
                crate::r#match::player::strategies::common::players::ops::forward_shot_decision::find_cutback_to_arriving_runner(ctx)
            {
                #[cfg(feature = "match-logs")]
                {
                    use std::sync::atomic::Ordering;
                    crate::r#match::player::strategies::common::players::ops::forward_shot_decision::mid_run_diag::FWD_CUTBACK.fetch_add(1, Ordering::Relaxed);
                    ForwardCensus::note_pass_site(ctx, FwdRequest::Cutback);
                }
                return Some(StateChangeResult::with_forward_state_and_event(
                    ForwardRead::released(ctx),
                    Event::PlayerEvent(PlayerEvent::PassTo(
                        PassingEventContext::builder()
                            .with_from_player_id(ctx.player.id)
                            .with_to_player_id(runner.id)
                            .with_reason("FWD_RUNNING_CUTBACK_TO_RUNNER")
                            .build(ctx),
                    )),
                ));
            }

        // If we deferred to a better-positioned teammate, route
        // through Passing — the pass state picks the target. The
        // pass-EV-beats-shot-xG case is now handled inside the
        // unified helper (returns ShotDecision::Pass).
        if defer_to_teammate {
            #[cfg(feature = "match-logs")]
            ForwardCensus::note_pass_site(ctx, FwdRequest::PassOther);
            return Some(StateChangeResult::with_forward_state(ForwardState::Passing));
        }

        // Priority 0.8: Open path to goal — KEEP RUNNING, don't pass or dribble
        // But only if still far from shooting range. Once in range, shooting
        // checks above should have triggered. If they didn't (no clear shot),
        // don't keep running into the GK — fall through to passing/dribbling.
        // `> MAX_SHOOTING_DISTANCE (320u) && < 180u` — a condition no
        // distance can satisfy, so this branch had never once run and
        // the forward had no drive-at-goal path at all. What it means
        // to say is the comment above it: keep running from beyond
        // 22.5 m (180u) with space ahead. The shot blocks above ask the
        // helper out to 320u (40 m), so here it has already declined
        // this tick.
        if distance_to_goal > 180.0 && self.has_open_space_ahead(ctx) {
            let dribbling = ctx.player.skills.technical.dribbling / 20.0;
            let composure = ctx.player.skills.mental.composure / 20.0;
            let determination = ctx.player.skills.mental.determination / 20.0;
            let pace = ctx.player.skills.physical.pace / 20.0;
            let carry_quality =
                dribbling * 0.35 + composure * 0.25 + determination * 0.2 + pace * 0.2;
            if carry_quality > 0.55 {
                stuck_exit_stats::note(3, ctx.in_state_time, ctx.player.velocity.norm());
                return None;
            }
        }

        // ONE-TWO COMBINATION: After just receiving ball, check if passer ran into space
        if ctx.ball().has_stable_possession() {
            let ownership_ticks = ctx.tick_context.ball.ownership_duration;
            if (2..=10).contains(&ownership_ticks)
                && let Some(return_target) = self.find_one_two_return(ctx)
            {
                #[cfg(feature = "match-logs")]
                ForwardCensus::note_pass_site(ctx, FwdRequest::OneTwo);
                return Some(StateChangeResult::with_forward_state_and_event(
                    ForwardRead::released(ctx),
                    Event::PlayerEvent(PlayerEvent::PassTo(
                        PassingEventContext::builder()
                            .with_from_player_id(ctx.player.id)
                            .with_to_player_id(return_target.id)
                            .with_reason("FWD_RUNNING_ONE_TWO_RETURN")
                            .build(ctx),
                    )),
                ));
            }
        }

        // HOLD-UP PLAY: When facing away from goal with midfielders arriving,
        // draw defenders and lay off to a supporting teammate.
        // Only when forward genuinely can't advance (opponents blocking ahead).
        if ctx.ball().has_stable_possession()
            && ctx.tick_context.ball.ownership_duration > 30
            && !self.has_open_space_ahead(ctx)
        // Don't lay off if can run forward
            && let Some(layoff_target) = self.find_hold_up_layoff(ctx)
        {
            #[cfg(feature = "match-logs")]
            ForwardCensus::note_pass_site(ctx, FwdRequest::HoldUpLayoff);
            return Some(StateChangeResult::with_forward_state_and_event(
                ForwardRead::released(ctx),
                Event::PlayerEvent(PlayerEvent::PassTo(
                    PassingEventContext::builder()
                        .with_from_player_id(ctx.player.id)
                        .with_to_player_id(layoff_target.id)
                        .with_reason("FWD_RUNNING_HOLD_UP_LAYOFF")
                        .build(ctx),
                )),
            ));
        }

        // Clear ball if congested far from goal — only after carrying for a while
        if ctx.tick_context.ball.ownership_duration > 15
            && (ctx.player().movement().is_congested_near_boundary()
                || ctx.player().movement().is_congested())
            && let Some(_) = ctx.players().teammates().all().next()
        {
            #[cfg(feature = "match-logs")]
            ForwardCensus::note_pass_site(ctx, FwdRequest::PassOther);
            return Some(StateChangeResult::with_forward_state(ForwardState::Passing));
        }

        // Under pressure - quick decision needed
        if ctx.player().pressure().is_under_immediate_pressure() {
            if self.should_pass_under_pressure(ctx) {
                #[cfg(feature = "match-logs")]
                ForwardCensus::note_pass_site(ctx, FwdRequest::PassOther);
                return Some(StateChangeResult::with_forward_state(ForwardState::Passing));
            } else if self.can_dribble_out_of_pressure(ctx) {
                return Some(StateChangeResult::with_forward_state(
                    ForwardState::Dribbling,
                ));
            }
        }

        // Evaluate best action based on game context
        // Require minimum carry time to prevent instant pass-after-receive
        let ownership_ticks = ctx.tick_context.ball.ownership_duration;

        // THE WIDE AREA, above the generic pass.
        //
        // A wide forward in the final third is not looking for
        // "somebody better placed" — he is looking for the box, and
        // the generic evaluator has no idea that is what the flank
        // is for. Decided purely on where he is standing, so it
        // covers a striker who has drifted wide as readily as a
        // nominal winger. See `FlankPlay`.
        if ownership_ticks > 8 {
            match FlankPlay::decide(ctx) {
                Some(FlankAction::ReleaseOutside { target }) => {
                    #[cfg(feature = "match-logs")]
                    {
                        crate::mid_run_diag::WideDiag::note(4);
                        ForwardCensus::note_pass_site(ctx, FwdRequest::FlankRelease);
                    }
                    return Some(StateChangeResult::with_forward_state_and_event(
                        ForwardRead::released(ctx),
                        Event::PlayerEvent(PlayerEvent::PassTo(
                            PassingEventContext::builder()
                                .with_from_player_id(ctx.player.id)
                                .with_to_player_id(target)
                                .with_reason("FWD_FLANK_RELEASE")
                                .build(ctx),
                        )),
                    ));
                }
                Some(FlankAction::Deliver) => {
                    return Some(StateChangeResult::with_forward_state(
                        ForwardState::Crossing,
                    ));
                }
                None => {}
            }
        }

        if ownership_ticks > 12 && self.should_pass(ctx) {
            #[cfg(feature = "match-logs")]
            ForwardCensus::note_pass_site(ctx, FwdRequest::PassShouldPass);
            return Some(StateChangeResult::with_forward_state(ForwardState::Passing));
        }

        if ownership_ticks > 20 && self.should_dribble(ctx) {
            return Some(StateChangeResult::with_forward_state(
                ForwardState::Dribbling,
            ));
        }

        // ANTI-OSCILLATION: carrying the ball too long without acting
        // (2.4 s on it) — he passes. It asked the shot helper
        // here as `FWD_RUN_ANTI_OSCILLATION`, and that never approved:
        // this is only reached after the helper held above, on the same
        // tick, and it gives the same answer.
        if Self::on_the_ball(ctx) > Self::STALL {
            #[cfg(feature = "match-logs")]
            {
                ForwardCensus::note_limit(ctx, FwdLimit::Stall);
                ForwardCensus::note_pass_site(ctx, FwdRequest::PassAfterStall);
            }
            return Some(StateChangeResult::with_forward_state(ForwardState::Passing));
        }

        // Continue running with ball briefly while looking for an opening
        stuck_exit_stats::note(4, ctx.in_state_time, ctx.player.velocity.norm());
        None
    }

    fn velocity(&self, ctx: &StateProcessingContext) -> Option<Vector3<f32>> {
        if ctx.player.has_ball(ctx) {
            return Some(self.calculate_ball_carrying_movement(ctx) * Self::carry_age_factor(ctx));
        }
        // The read moves him on the tick he is here without it; whatever is
        // left of that tick he spends on his place.
        Some(
            SteeringBehavior::Arrive {
                target: ctx.team().my_anchor(),
                slowing_distance: 25.0,
            }
            .calculate(ctx.player)
            .velocity,
        )
    }

    fn process_conditions(&self, ctx: ConditionContext) {
        // Forwards do a lot of intense running - high intensity with velocity
        ForwardCondition::new(ActivityIntensity::High).process(ctx);
    }
}

impl ForwardRunningState {
    /// 30% off the carry over 3.6 s in the state. Time on the ball is not
    /// tiredness, but this is the only brake on a carry the on-ball tree
    /// holds too long: without it goals rose from 2.88 to 3.74 a match.
    fn carry_age_factor(ctx: &StateProcessingContext) -> f32 {
        (1.0 - ctx.in_state_time as f32 / 600.0).max(0.7)
    }

    /// Engine ticks in a second: the clock `ownership_duration` runs on.
    const TICKS_PER_SECOND: f32 = 100.0;
    /// The first moment after he gets the ball, 0.16 s.
    const JUST_RECEIVED: u32 = (0.16 * Self::TICKS_PER_SECOND) as u32;
    /// On the ball 2.4 s without acting, he passes.
    const STALL: u32 = (2.4 * Self::TICKS_PER_SECOND) as u32;
    /// A point-blank hold lasts 3 s before he squares it.
    const POINT_BLANK_PATIENCE: u32 = (3.0 * Self::TICKS_PER_SECOND) as u32;
    /// He holds the ball in the area 6 s for the chance to come.
    const HOLD_PATIENCE: u32 = (6.0 * Self::TICKS_PER_SECOND) as u32;

    /// How long he has had the ball, in engine ticks — counted from when he
    /// got it, whatever state he was in then.
    fn on_the_ball(ctx: &StateProcessingContext) -> u32 {
        ctx.tick_context.ball.ownership_duration
    }

    fn just_received(ctx: &StateProcessingContext) -> bool {
        Self::on_the_ball(ctx) < Self::JUST_RECEIVED
    }

    #[cfg(feature = "match-logs")]
    fn verdict(decision: &ShotDecision) -> FwdVerdict {
        match decision {
            ShotDecision::Shoot { .. } => FwdVerdict::Approved,
            ShotDecision::Pass => FwdVerdict::HelperPass,
            ShotDecision::Hold => FwdVerdict::HelperHold,
        }
    }

    /// Check if there's open space ahead toward the opponent goal
    fn has_open_space_ahead(&self, ctx: &StateProcessingContext) -> bool {
        let player_pos = ctx.player.position;
        let goal_pos = ctx.player().opponent_goal_position();
        let to_goal = (goal_pos - player_pos).normalize();

        // How far ahead a body has to be before the space stops being
        // open, in GAME UNITS (1u = 0.125 m).
        //
        // Was `25.0` — **3.1 m**. That is the forward's half of the same
        // defect the midfielder's carry gate had (`CARRY_ROOM` in
        // `midfielders/states/running`, which was 4 m): a player was
        // deciding to drive at goal on the basis that nobody was inside
        // three metres of him, which is not open space, it is a defender
        // about to arrive. `prog_carries` read **23.6 per forward per
        // match against a real ~2**.
        //
        // 50u = 6.25 m rather than the midfielder's ~10 m because this is
        // a WIDE cone (`dot > 0.4`, ~66°) where the midfielder reads a
        // narrow channel — the same radius would sweep several times the
        // area and refuse every carry on the pitch.
        //
        // Note this test is read with BOTH polarities: open space feeds
        // the drive-at-goal branch, and its negation feeds the hold-up
        // lay-off. Widening it therefore moves play from carrying toward
        // passing on both sides, which is the intent.
        //
        // ⚠ MEASURED AS A NO-OP, KEPT ON ITS OWN MERITS. 200 matches at
        // L14, 25u → 50u: forward `prog_carries` 23.56 → 23.55 and shots
        // 102.3 → 103.6, both inside run-to-run noise. So the forward's
        // carrying does NOT flow through this test — the branch it gates
        // is reachable but rare (`FWD Running stuck-exit` reports
        // drive-at-goal at 100% of only 54.7k ticks). If you are hunting
        // forward carry volume, this is not the path; find the one that
        // falls through to the state's own `return None`. The radius is
        // left corrected because 3.1 m is not open space by any reading,
        // not because it bought anything.
        const OPEN_SPACE_RADIUS: f32 = 50.0;
        let blockers = ctx
            .players()
            .opponents()
            .nearby(OPEN_SPACE_RADIUS)
            .filter(|opp| {
                let to_opp = (opp.position - player_pos).normalize();
                to_opp.dot(&to_goal) > 0.4
            })
            .count();

        blockers == 0
    }

    /// Determine if should pass when under pressure
    fn should_pass_under_pressure(&self, ctx: &StateProcessingContext) -> bool {
        // Check for available passing options
        ctx.players()
            .teammates()
            .nearby(200.0)
            .any(|t| ctx.player().has_clear_pass(t.id))
    }

    /// Check if can dribble out of pressure
    fn can_dribble_out_of_pressure(&self, ctx: &StateProcessingContext) -> bool {
        let dribbling = ctx.player.skills.technical.dribbling / 20.0;
        let agility = ctx.player.skills.physical.agility / 20.0;
        let composure = ctx.player.skills.mental.composure / 20.0;

        let skill_factor = dribbling * 0.5 + agility * 0.3 + composure * 0.2;

        // Check for escape route
        let has_space = self.find_dribbling_space(ctx).is_some();

        skill_factor > 0.5 && has_space
    }

    /// Find space to dribble into
    fn find_dribbling_space(&self, ctx: &StateProcessingContext) -> Option<Vector3<f32>> {
        let player_pos = ctx.player.position;
        let goal_direction = (ctx.player().opponent_goal_position() - player_pos).normalize();

        // Check multiple angles for space
        let angles = [-45.0f32, -30.0, 0.0, 30.0, 45.0];

        for angle_deg in angles.iter() {
            let angle_rad = angle_deg.to_radians();
            let cos_a = angle_rad.cos();
            let sin_a = angle_rad.sin();

            // Rotate goal direction by angle
            let check_direction = Vector3::new(
                goal_direction.x * cos_a - goal_direction.y * sin_a,
                goal_direction.x * sin_a + goal_direction.y * cos_a,
                0.0,
            );

            let check_position = player_pos + check_direction * 15.0;

            // Check if this direction is clear
            let opponents_in_path = ctx
                .players()
                .opponents()
                .nearby(20.0)
                .filter(|opp| {
                    let to_opp = (opp.position - player_pos).normalize();
                    to_opp.dot(&check_direction) > 0.7
                })
                .count();

            if opponents_in_path == 0 {
                return Some(check_position);
            }
        }

        None
    }

    /// Calculate movement when carrying the ball
    fn calculate_ball_carrying_movement(&self, ctx: &StateProcessingContext) -> Vector3<f32> {
        // A gap in the defensive line is worth running at — that is a real
        // read and it stays, unclamped: running into space is not the
        // problem, and walling it off just piled every carrier up on the
        // wall (54% of all shots came from one 5-metre band, and the
        // keeper saved 94% of them because he was set for every one).
        // A carry target within touching distance is not a run, and
        // chasing it is what produces the limit cycle: `Arrive` cannot
        // settle on a point closer than the carrier's own per-tick step,
        // so he overshoots, reverses, and overshoots back. Measured off a
        // replay dump as a clean 240 ms oscillation of 2.1u amplitude,
        // held for 20-38 s at a time with the ball a few metres in front
        // of the goalkeeper. Standing still is the honest outcome — and
        // it stops the ball vibrating on the spot, which is what this
        // reads as on the pitch.
        const CARRY_DEADBAND: f32 = 6.0; // 0.75 m
        // Inside it the carry is about the shot, and the box movement
        // below takes over from heading for open grass.
        const BOX_APPROACH: f32 = 120.0; // 15 m

        // The byline, ahead of everything else a wide carrier might do.
        //
        // `find_optimal_attacking_path` and `BallCarry::target` both aim
        // at goal, which from a touchline is a diagonal into the centre-
        // backs. Substituting the aim — not the steering — is what turns
        // "he cut inside and lost it" into "he got to the byline", and
        // `FlankPlay` returns `None` for every carrier who is not
        // actually wide, so a central striker's carry is untouched.
        let openness = LaneAhead::read(ctx).openness;
        if let Some(byline) = FlankPlay::carry_aim(ctx, openness)
            && (byline - ctx.player.position).magnitude() >= CARRY_DEADBAND
        {
            return SteeringBehavior::Arrive {
                target: byline,
                slowing_distance: 20.0,
            }
            .calculate(ctx.player)
            .velocity;
        }

        if ctx.ball().distance_to_opponent_goal() > BOX_APPROACH {
            return SteeringBehavior::Arrive {
                target: CarryHeading::aim(ctx),
                slowing_distance: 20.0,
            }
            .calculate(ctx.player)
            .velocity;
        }
        if let Some(target_position) = self.find_optimal_attacking_path(ctx) {
            if (target_position - ctx.player.position).magnitude() < CARRY_DEADBAND {
                return self.settle_over_the_ball(ctx);
            }
            return SteeringBehavior::Arrive {
                target: target_position,
                slowing_distance: 20.0,
            }
            .calculate(ctx.player)
            .velocity;
        }
        // Otherwise carry toward a shooting position rather than toward
        // the goal itself. The fallback here was `Arrive` at the goal
        // centre, which is an instruction to run into the goalkeeper and
        // was doing exactly that whenever no gap was found — the common
        // case against a set defence.
        let carry_target = BallCarry::target(ctx);
        if (carry_target - ctx.player.position).magnitude() < CARRY_DEADBAND {
            return self.settle_over_the_ball(ctx);
        }
        SteeringBehavior::Arrive {
            target: carry_target,
            slowing_distance: 30.0,
        }
        .calculate(ctx.player)
        .velocity
    }

    /// What a carrier does when the carry model has run out of things to
    /// say — the target has collapsed to within touching distance of his
    /// own feet.
    ///
    /// This used to be `Vector3::zeros()`, and the note above explains
    /// why: chasing a target inside your own per-tick step is a limit
    /// cycle, and standing still is honester than vibrating. True, but
    /// the situation it produces is a man standing motionless on the ball
    /// in the penalty area, and paired with a shot threshold drawn once
    /// per possession that is a fixed point the stall detector has to
    /// break twenty seconds later.
    ///
    /// A footballer in that position is never still. Which thing he does
    /// depends on one reading — is anybody near him:
    ///
    /// * **Being closed down** — he shields, turning his body between the
    ///   defender and the ball, stepping away from the nearest man.
    /// * **Nobody near him** — then he is through, and the thing to do is
    ///   go at the goalkeeper. `carry_hold` normally stops him closing,
    ///   and that rule is right in general, but it cannot be right when
    ///   the alternative is not moving at all.
    ///
    /// Both are absolute directions rather than offsets from his own
    /// position, so neither reproduces the treadmill the deadband was
    /// introduced to stop: he moves, the geometry changes, and ordinary
    /// steering takes over again on the next tick.
    pub(crate) fn settle_over_the_ball(&self, ctx: &StateProcessingContext) -> Vector3<f32> {
        const SHIELD_REACH: f32 = 24.0; // 3 m
        let me = ctx.player.position;

        let nearest = ctx
            .players()
            .opponents()
            .nearby(SHIELD_REACH)
            .min_by(|a, b| {
                let da = (a.position - me).magnitude();
                let db = (b.position - me).magnitude();
                da.partial_cmp(&db).unwrap_or(Ordering::Equal)
            });

        let direction = match nearest {
            Some(opponent) => {
                let away = me - opponent.position;
                if away.norm_squared() < 1.0e-4 {
                    return Vector3::zeros();
                }
                away.normalize()
            }
            None => {
                let to_goal = ctx.player().opponent_goal_position() - me;
                if to_goal.norm_squared() < 1.0e-4 {
                    return Vector3::zeros();
                }
                to_goal.normalize()
            }
        };

        SteeringBehavior::Seek {
            target: me + direction * SHIELD_REACH,
        }
        .calculate(ctx.player)
        .velocity
    }

    /// Find optimal path considering opponents and teammates
    fn find_optimal_attacking_path(&self, ctx: &StateProcessingContext) -> Option<Vector3<f32>> {
        let player_pos = ctx.player.position;
        let goal_pos = ctx.player().opponent_goal_position();

        // Look for gaps in defense
        if let Some(gap) = self.find_best_gap_in_defense(ctx) {
            return Some(gap);
        }

        // Try to move toward goal while avoiding opponents
        let to_goal = goal_pos - player_pos;
        let goal_direction = to_goal.normalize();

        // Check if direct path is clear
        if !ctx.players().opponents().nearby(30.0).any(|opp| {
            let to_opp = opp.position - player_pos;
            let dot = to_opp.normalize().dot(&goal_direction);
            dot > 0.8 && to_opp.magnitude() < 40.0
        }) {
            return Some(player_pos + goal_direction * 50.0);
        }

        None
    }

    /// Find the best gap in opponent defense
    fn find_best_gap_in_defense(&self, ctx: &StateProcessingContext) -> Option<Vector3<f32>> {
        let player_pos = ctx.player.position;
        let goal_pos = ctx.player().opponent_goal_position();

        // Inline fixed-size buffer: at most 11 opponents can pass
        // the dot-product filter (one team), and we only need their
        // positions for the O(n²) gap scan.
        const MAX_OPPONENTS: usize = 11;
        let mut positions: [Vector3<f32>; MAX_OPPONENTS] = [Vector3::zeros(); MAX_OPPONENTS];
        let mut n: usize = 0;
        let to_goal = goal_pos - player_pos;
        // Skip the filter entirely when the goal direction is
        // degenerate to avoid a NaN normalize.
        if to_goal.norm_squared() < 1e-6 {
            return None;
        }
        let goal_dir = to_goal.normalize();
        for opp in ctx.players().opponents().nearby(100.0) {
            let to_opp = opp.position - player_pos;
            if to_opp.norm_squared() < 1e-6 {
                continue;
            }
            if goal_dir.dot(&to_opp.normalize()) > 0.5 {
                if n >= MAX_OPPONENTS {
                    break;
                }
                positions[n] = opp.position;
                n += 1;
            }
        }

        if n < 2 {
            return None;
        }

        // Find largest gap
        let mut best_gap = None;
        let mut best_gap_size = 0.0;

        for i in 0..n {
            for j in i + 1..n {
                let gap_center = (positions[i] + positions[j]) * 0.5;
                let gap_size = (positions[i] - positions[j]).magnitude();

                if gap_size > best_gap_size && gap_size > 20.0 {
                    // A gap has to be somewhere to RUN, not where he is
                    // already standing.
                    //
                    // Two defenders either side of the carrier produce a
                    // midpoint at his own feet, and `Arrive` at your own
                    // feet is a limit cycle: he reaches it, the candidate
                    // set shifts by one man as he moves a few centimetres
                    // (the `dot > 0.5` cone is a discrete membership test
                    // on a normalised vector, so it flips on sub-metre
                    // movement), the midpoint jumps a couple of units, and
                    // he accelerates back to it. Measured off a replay
                    // dump: a clean 240 ms limit cycle, 2.1u amplitude,
                    // held for 20 s with the ball four metres in front of
                    // the goalkeeper. `dead_ball_diag` puts the whole
                    // pattern at ~156 s a match of the ball going nowhere
                    // in a forward's feet — the largest single source.
                    //
                    // Requiring the gap to be a real advance ahead of him
                    // means he only targets one he has to run to; once
                    // through it the gap stops qualifying and
                    // `BallCarry::target` takes over.
                    const MIN_GAP_ADVANCE: f32 = 25.0; // ~3 m
                    if (gap_center - player_pos).dot(&goal_dir) < MIN_GAP_ADVANCE {
                        continue;
                    }
                    best_gap_size = gap_size;
                    best_gap = Some(gap_center);
                }
            }
        }

        best_gap
    }

    fn should_pass(&self, ctx: &StateProcessingContext) -> bool {
        // ≤10 teammates are on the pitch — a stack buffer replaces the
        // per-evaluation Vec collect (same iteration order, the helper
        // predicates below take a slice either way).
        let mut teammates_buf = [MatchPlayerLite {
            id: 0,
            position: Vector3::zeros(),
            tactical_positions: PlayerPositionType::Goalkeeper,
        }; 11];
        let mut teammates_len = 0usize;
        for t in ctx.players().teammates().nearby(300.0) {
            if teammates_len == teammates_buf.len() {
                break;
            }
            teammates_buf[teammates_len] = t;
            teammates_len += 1;
        }
        let teammates = &teammates_buf[..teammates_len];

        if teammates.is_empty() {
            return false;
        }

        // Core skill curves — sigmoid-rolled per evaluation so the
        // full 1-20 range matters for each decision branch, replacing
        // `> 0.7` cliffs that flattened mid-skill players.
        let vision_raw = ctx.player.skills.mental.vision;
        let passing_raw = ctx.player.skills.technical.passing;
        let decisions_raw = ctx.player.skills.mental.decisions;
        let teamwork_raw = ctx.player.skills.mental.teamwork;

        // Situational factors — use common pressure check
        let under_pressure = ctx.player().pressure().is_under_immediate_pressure();
        let distance_to_goal = ctx.ball().distance_to_opponent_goal();
        let stamina = ctx.player.player_attributes.condition_percentage() as f32 / 100.0;

        // 1. MUST PASS: Heavy pressure or exhaustion
        if under_pressure {
            let pass_p = SkillCurve::new(passing_raw, 10.0, 0.6).probability();
            if ctx.context.rng.unit_f32() < pass_p || stamina < 0.4 {
                return self.has_safe_passing_option(ctx, teammates);
            }
        }

        // 2. LOOK FOR QUALITY OPPORTUNITIES: vision/passing scale smoothly
        let quality_p = SkillCurve::new(vision_raw, 14.0, 0.6)
            .probability()
            .max(SkillCurve::new(passing_raw, 14.0, 0.6).probability());
        if ctx.context.rng.unit_f32() < quality_p
            && self.has_teammate_in_dangerous_position(ctx, teammates, distance_to_goal)
        {
            return true;
        }

        // 3. TEAM PLAY: teamwork and decisions blend smoothly
        let team_p = SkillCurve::new(teamwork_raw, 14.0, 0.6).probability()
            * SkillCurve::new(decisions_raw, 12.0, 0.6).probability();
        if ctx.context.rng.unit_f32() < team_p {
            return self.has_good_passing_option(ctx, teammates);
        }

        // 5. DEFAULT: Keep the ball unless there's a clear benefit to passing
        false
    }

    /// Check if there's a safe pass available under pressure
    fn has_safe_passing_option(
        &self,
        ctx: &StateProcessingContext,
        teammates: &[MatchPlayerLite],
    ) -> bool {
        teammates.iter().any(|teammate| {
            let has_clear_lane = ctx.player().has_clear_pass(teammate.id);
            let not_marked = !self.is_teammate_heavily_marked(ctx, teammate);

            has_clear_lane && not_marked
        })
    }

    /// Check if a teammate has a MUCH better shot opportunity (vision/teamwork-aware)
    /// Used in process() to defer to a better-positioned teammate.
    fn has_teammate_with_much_better_shot(
        &self,
        ctx: &StateProcessingContext,
        own_distance: f32,
    ) -> bool {
        // Don't pass at point-blank range
        if own_distance < ForwardShot::POINT_BLANK {
            return false;
        }

        let vision = ctx.player.skills.mental.vision / 20.0;
        let teamwork = ctx.player.skills.mental.teamwork / 20.0;

        // Selfish players with low vision/teamwork don't look for teammates
        if vision < 0.4 && teamwork < 0.4 {
            return false;
        }

        ctx.players().teammates().nearby(200.0).any(|teammate| {
            let teammate_distance =
                (teammate.position - ctx.player().opponent_goal_position()).magnitude();
            // Teammate must be much closer: at most 45% of his own distance
            // to goal. "Much better shot" has to mean MUCH better, or the
            // carrier hands off a perfectly good look.
            let is_much_closer = teammate_distance <= own_distance * 0.45;
            let has_clear_pass = ctx.player().has_clear_pass(teammate.id);
            // "Not heavily marked" asked for TWO opponents inside 8u — and
            // 8u is one metre, so it took two defenders standing on a man's
            // toes to disqualify him. In practice every team-mate passed,
            // which turned this into an unconditional "is anybody nearer the
            // goal than me" veto — and somebody almost always is.
            //
            // That matters more than a mis-set radius, because this veto
            // does not go through the shot helper: it removes the shot
            // BLOCK, so the striker never asks the question at all and falls
            // through to Passing. Measured: it vetoed 12.8% of all in-range
            // forward ticks.
            //
            // Free means free — nobody within 30u (3.75 m), the same
            // standard `find_cutback_to_arriving_runner` uses for an
            // arriving runner, because it is the same judgement: can I
            // actually give him the ball in a better position than mine.
            let is_free = ctx
                .tick_context
                .grid
                .opponents(teammate.id, 30.0)
                .next()
                .is_none();
            // …and better placed means a better SIGHT of goal, not merely a
            // smaller number. A man three metres from the byline is nearer
            // the goal than a striker on the penalty spot and has no angle
            // to shoot from; deferring to him is how a clear chance became a
            // square ball. Compare where each of them actually is.
            let centre_y = ctx.context.field_size.height as f32 / 2.0;
            let more_central =
                (teammate.position.y - centre_y).abs() < (ctx.player.position.y - centre_y).abs();

            is_much_closer && has_clear_pass && is_free && more_central
        })
    }

    fn has_teammate_in_dangerous_position(
        &self,
        ctx: &StateProcessingContext,
        teammates: &[MatchPlayerLite],
        current_distance: f32,
    ) -> bool {
        teammates.iter().any(|teammate| {
            let teammate_distance =
                (teammate.position - ctx.player().opponent_goal_position()).magnitude();

            // Check if teammate is in a good attacking position
            let in_attacking_position = teammate_distance < current_distance * 1.1;

            // Check if teammate is in free space (use pre-computed distances)
            let in_free_space = ctx.tick_context.grid.opponents(teammate.id, 12.0).count() < 2;

            let has_clear_pass = ctx.player().has_clear_pass(teammate.id);

            has_clear_pass && in_attacking_position && in_free_space
        })
    }

    /// Check for any good passing option (balanced assessment)
    fn has_good_passing_option(
        &self,
        ctx: &StateProcessingContext,
        teammates: &[MatchPlayerLite],
    ) -> bool {
        teammates.iter().any(|teammate| {
            let has_clear_lane = ctx.player().has_clear_pass(teammate.id);
            let has_space = ctx.tick_context.grid.opponents(teammate.id, 10.0).count() < 2;

            // Prefer forward passes (side-aware)
            let is_forward_pass = match ctx.player.side {
                Some(PlayerSide::Left) => teammate.position.x > ctx.player.position.x,
                Some(PlayerSide::Right) => teammate.position.x < ctx.player.position.x,
                None => false,
            };

            has_clear_lane && has_space && is_forward_pass
        })
    }

    fn is_teammate_heavily_marked(
        &self,
        ctx: &StateProcessingContext,
        _teammate: &MatchPlayerLite,
    ) -> bool {
        // KNOWN LATENT BUG, deliberately preserved (same class as the
        // event-only drop in `merge_state_change`): this scans the
        // CARRIER's markers (`ctx.player.id`), not `_teammate`'s, so
        // under pressure every teammate reads as "marked" and the
        // forward holds instead of releasing. That selfish hold is
        // load-bearing for chance volume: scanning the real teammate
        // (dev_match A/B, 200 matches) collapsed forward in-range
        // possession ~6× and goals 0.27→0.12/match, because carriers
        // released to open (backward) outlets instead of settling and
        // shooting. Fixing the entity must land together with a
        // shot-willingness / pass-economy recalibration.
        let mut markers = 0;
        let mut very_close = 0;
        for (_id, dist) in ctx.tick_context.grid.opponents(ctx.player.id, 8.0) {
            markers += 1;
            if dist <= 3.0 {
                very_close += 1;
            }
        }

        markers >= 2 || (markers >= 1 && very_close > 0)
    }

    fn should_dribble(&self, ctx: &StateProcessingContext) -> bool {
        let dribbling_raw = ctx.player.skills.technical.dribbling;
        let pace_raw = ctx.player.skills.physical.pace;

        // Check for opponents directly ahead (not just any nearby)
        let goal_pos = ctx.player().opponent_goal_position();
        let player_pos = ctx.player.position;
        let to_goal = (goal_pos - player_pos).normalize();

        let opponents_blocking = ctx
            .players()
            .opponents()
            .nearby(25.0)
            .filter(|opp| {
                let to_opp = (opp.position - player_pos).normalize();
                to_opp.dot(&to_goal) > 0.5
                    && (opp.position - player_pos).norm_squared() < 20.0 * 20.0
            })
            .count();

        // No opponents blocking — just keep running, don't dribble
        if opponents_blocking == 0 {
            return false;
        }

        // Take-on willingness scales smoothly with dribbling + pace.
        // Two thresholds (skilled vs. modest) become two sigmoid pivots
        // (14/12 dribbling, 12/10 pace) so a 7/20 dribbler still
        // *sometimes* takes on a single defender and a 17/20 elite
        // attempts 2-man take-ons most of the time.
        //
        // Both pivots are then tilted by the player's head: running at a
        // defender is the most self-expressive thing a footballer does, so
        // a winger who has just beaten his man twice goes again, and one
        // who has been dispossessed and booked keeps it simple. Narrow
        // (±~15%) — psychology colours how often the take-on is tried, not
        // whether it comes off (`dribble_duel` owns that).
        let initiative = Psychology::initiative_for(&ctx.context.psychology, ctx.player.id);
        let elite_take_on = SkillCurve::new(dribbling_raw, 14.0, 0.6).probability()
            * SkillCurve::new(pace_raw, 12.0, 0.6).probability()
            * initiative;
        let modest_take_on = SkillCurve::new(dribbling_raw, 10.0, 0.6).probability() * initiative;
        let roll = ctx.context.rng.unit_f32();
        if roll < elite_take_on {
            opponents_blocking <= 2
        } else if roll < modest_take_on {
            opponents_blocking <= 1
        } else {
            false
        }
    }

    /// ONE-TWO COMBINATION: Check if the player who just passed to us has run
    /// ahead into space — return the ball for a wall-pass / give-and-go
    fn find_one_two_return<'a>(&self, ctx: &StateProcessingContext<'a>) -> Option<MatchPlayerLite> {
        let recent_passers = ctx.tick_context.ball.recent_passers();
        let passer_id = *recent_passers.last()?;

        // Passer must be a teammate
        let passer = ctx.context.players.by_id(passer_id)?;
        if passer.team_id != ctx.player.team_id {
            return None;
        }

        let passer_lite = ctx
            .players()
            .teammates()
            .all()
            .find(|t| t.id == passer_id)?;

        let player_pos = ctx.player.position;
        let goal_pos = ctx.player().opponent_goal_position();
        let passer_pos = passer_lite.position;

        // Passer must now be closer to goal than us (continued their run)
        let our_goal_dist = (goal_pos - player_pos).magnitude();
        let passer_goal_dist = (goal_pos - passer_pos).magnitude();
        if passer_goal_dist >= our_goal_dist * 0.85 {
            return None;
        }

        // Passer must be in open space (no opponents within 50 units)
        let opponents_near_passer = ctx.tick_context.grid.opponents(passer_id, 50.0).count();
        if opponents_near_passer >= 1 {
            return None;
        }

        // Clear passing lane and reasonable distance
        if !ctx.player().has_clear_pass(passer_id) {
            return None;
        }
        let pass_distance = (passer_pos - player_pos).magnitude();
        if pass_distance > 180.0 || pass_distance < 25.0 {
            return None;
        }

        Some(passer_lite)
    }

    /// HOLD-UP PLAY: When forward is under pressure from behind and a supporting
    /// midfielder/teammate is arriving behind them, lay the ball off.
    /// Only triggers when there are opponents AHEAD blocking the path to goal.
    fn find_hold_up_layoff<'a>(&self, ctx: &StateProcessingContext<'a>) -> Option<MatchPlayerLite> {
        let player_pos = ctx.player.position;
        let goal_pos = ctx.player().opponent_goal_position();

        // Need opponents actively blocking the forward path (ahead of us, toward goal)
        let to_goal = (goal_pos - player_pos).normalize();
        let opponents_ahead = ctx
            .players()
            .opponents()
            .nearby(25.0)
            .filter(|opp| {
                let to_opp = (opp.position - player_pos).normalize();
                to_opp.dot(&to_goal) > 0.3 // Opponent is roughly between us and goal
            })
            .count();
        if opponents_ahead < 1 {
            return None; // Path to goal is clear — run, don't lay off
        }

        // Find a supporting teammate who is behind us (closer to own goal)
        // and in space — this is the classic target man layoff
        let our_goal_dist = (goal_pos - player_pos).magnitude();

        ctx.players()
            .teammates()
            .nearby(150.0)
            .filter(|t| {
                let t_dist = (t.position - player_pos).magnitude();
                // Reject very close teammates to prevent short group passes
                if t_dist < 30.0 {
                    return false;
                }
                let t_goal_dist = (goal_pos - t.position).magnitude();
                // Teammate must be further from opponent goal (behind us)
                let is_behind = t_goal_dist > our_goal_dist * 1.1;
                // Teammate must be in space
                let in_space = ctx.tick_context.grid.opponents(t.id, 10.0).count() < 2;
                // Prefer midfielders who can carry forward
                let is_midfielder_or_attacker =
                    t.tactical_positions.is_midfielder() || t.tactical_positions.is_forward();
                // Reject recent passers to prevent cycling
                let not_recent = ctx.ball().passer_recency_penalty(t.id) > 0.3;

                is_behind && in_space && is_midfielder_or_attacker && not_recent
            })
            .filter_map(|t| {
                let odds = ctx.player().clear_pass(t.id)?;
                // Prefer farther teammates — they're more likely outside the congested group
                Some((t, odds.price((t.position - player_pos).magnitude())))
            })
            .max_by(|a, b| a.1.total_cmp(&b.1))
            .map(|(t, _)| t)
    }

}
