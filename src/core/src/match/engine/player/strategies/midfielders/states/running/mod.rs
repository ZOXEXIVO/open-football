use crate::PlayerFieldPositionGroup;
use crate::r#match::events::Event;
use crate::r#match::midfielders::states::MidfielderState;
use crate::r#match::midfielders::states::common::{ActivityIntensity, MidfielderCondition};
use crate::r#match::player::events::{PassingEventContext, PlayerEvent};
use crate::r#match::player::strategies::common::players::MatchPlayerIteratorExt;
use crate::r#match::player::strategies::common::players::ops::forward_shot_decision::{
    ShotDecision, evaluate_forward_shot_decision,
};
use crate::r#match::player::strategies::common::players::ops::midfielder_skill::MidfielderSkillProfile;
use crate::r#match::player::strategies::players::skills::SkillCurve;
use crate::r#match::{
    ConditionContext, GamePhase, MatchPlayerLite, PassEvaluator, PlayerSide, StateChangeResult,
    StateProcessingContext, StateProcessingHandler, SteeringBehavior,
};
use nalgebra::Vector3;
use std::cmp::Ordering;

// Shooting distance constants for midfielders — more conservative than forwards
const MAX_SHOOTING_DISTANCE: f32 = 88.0; // Edge-of-box / arriving-midfielder strikes
const STANDARD_SHOOTING_DISTANCE: f32 = 52.0; // Standard shooting range for midfielders
const POINT_BLANK_DISTANCE: f32 = 20.0; // ~10m - must shoot, goalkeeper is right there

#[derive(Default, Clone)]
pub struct MidfielderRunningState {}

impl StateProcessingHandler for MidfielderRunningState {
    fn process(&self, ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        // Offside discipline — if we don't have the ball and we've run
        // past the opposing defensive line, drop back before a teammate
        // plays a pass that finds us offside.
        if !ctx.player.has_ball(ctx) && ctx.player().defensive().is_stranded_offside() {
            return Some(StateChangeResult::with_midfielder_state(
                MidfielderState::Returning,
            ));
        }

        // COUNTER-PRESS: we just lost the ball. The closest midfielder
        // to the new carrier committing to an immediate press is the
        // single biggest recovery mechanism in real football — it's
        // why modern high-tempo sides look so relentless. Mirrors the
        // defender counter-press in `defenders/running` line ~108. Only
        // fires for the midfielder best positioned to chase (avoids
        // whole midfield collapsing on one runner).
        if !ctx.player.has_ball(ctx)
            && ctx.team().counterpress_window()
            && !ctx.team().is_control_ball()
        {
            let ball_dist = ctx.ball().distance();
            // Use the per-player eligibility helper so only the
            // best-positioned 2-3 midfielders engage. The ball-best
            // chaser check (already enforced at squad level) layers on
            // top so we don't double-up with a defender or forward who
            // also won the eligibility roll.
            let elected = ctx.player().pressure().should_counterpress()
                && ctx.team().is_best_player_to_chase_ball();
            let immediate = ball_dist < 25.0;
            if immediate {
                return Some(StateChangeResult::with_midfielder_state(
                    MidfielderState::Tackling,
                ));
            }
            if elected && ball_dist < 80.0 {
                return Some(StateChangeResult::with_midfielder_state(
                    MidfielderState::Pressing,
                ));
            }
        }

        // Phase-first dispatch — midfielders are the engine's pivot
        // between defence and attack, so the phase signal matters most
        // for them. See `phase_dispatch` for behaviour per phase.
        if let Some(phase_action) = self.phase_dispatch(ctx) {
            return Some(phase_action);
        }

        if ctx.player.has_ball(ctx) {
            // Corner taker: set the corner up via Crossing (which holds the
            // delivery until centre-backs have pushed up to attack it).
            if ctx.ball().is_team_attacking_corner() {
                return Some(StateChangeResult::with_midfielder_state(
                    MidfielderState::Crossing,
                ));
            }

            let distance_to_goal = ctx.ball().distance_to_opponent_goal();
            let coach = ctx.team().coach_instruction();
            let can_shoot = ctx.team().can_shoot();

            // ── Midfielder snapshot under pressure ─────────────────────
            //
            // Same asymmetric pattern as the forward snapshot in
            // `forwarders/states/running/mod.rs`: a midfielder who just
            // received the ball (in_state_time < 8) inside shooting
            // range (< 60u) with a defender right on them (within 10u),
            // AND whose first_touch is below the defender's tackling
            // (by ≥0.5), fires immediately instead of going through
            // the normal control + decision tree. Without this,
            // arriving box-runners who would-be cut-back recipients
            // get tackled before they can shoot — they're MIDfielders
            // and have lower first-touch than dedicated forwards, so
            // strong defenders out-touch them on virtually every
            // reception. Adding the midfielder path lifts the
            // weak-team scoring contribution from runners-into-the-
            // box, which the forward-only path missed entirely.
            //
            // Calibration-neutral at equal skill: at first_touch =
            // tackling, 11 < 10.5 is false, snapshot doesn't fire.
            if can_shoot && ctx.in_state_time < 8 && distance_to_goal < 60.0 {
                let nearest_threat = ctx
                    .players()
                    .opponents()
                    .nearby_raw(10.0)
                    .min_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(Ordering::Equal))
                    .map(|(id, _)| id);
                if let Some(threat_id) = nearest_threat {
                    let defender_tackling = ctx.player().skills(threat_id).technical.tackling;
                    let attacker_first_touch = ctx.player.skills.technical.first_touch;
                    if attacker_first_touch < defender_tackling - 0.5 {
                        return Some(
                            StateChangeResult::with_midfielder_state(MidfielderState::Shooting)
                                .with_shot_reason("MID_SNAPSHOT_PRESSED"),
                        );
                    }
                }
            }

            // Emergency clearance: under heavy pressure in our own box.
            // Route to Passing so its emergency-clearance code path fires
            // (Passing already has `emit_emergency_clearance` gated on
            // `in_box_danger_zone` + `is_under_heavy_pressure`). Running
            // previously had no such escape hatch — a midfielder under
            // two-defender press in their own area kept trying to play
            // out, lost the ball, and conceded via the ensuing turnover.
            if ctx.player().pressure().is_under_heavy_pressure()
                && ctx.ball().distance_to_own_goal() < ctx.context.field_size.width as f32 * 0.18
            {
                return Some(StateChangeResult::with_midfielder_state(
                    MidfielderState::Passing,
                ));
            }

            // ── MIDFIELDER SHOOTING (unified, skill-driven) ──────────────
            // Every midfielder in shooting range consults the SAME shot
            // helper the forwards use, so the shoot/pass/hold decision
            // scales continuously with the player's actual shooting
            // attributes (selection / execution / composure) rather than
            // the old binary `mid_shot_selection >= 0.32/0.42` cliffs —
            // which scored a default central mid ~0 yet let an elite one
            // fire unlimited, additive shots. The helper applies its xG
            // floor, inside-six floor, GK / 1v1 read, pass-EV deferral
            // (a playmaker lays it off when a teammate is better placed),
            // and the anti-monopoly volume cap. Net effect: a deep regista
            // rarely shoots, a box-to-box #8 arriving centrally shoots like
            // a forward, and no single player monopolises the attempts.
            // Hoisted above the possession / recycle defaults so a real
            // opening isn't recycled back to a forward.
            if can_shoot && distance_to_goal <= MAX_SHOOTING_DISTANCE {
                #[cfg(feature = "match-logs")]
                {
                    use std::sync::atomic::Ordering;
                    crate::r#match::player::strategies::common::players::ops::forward_shot_decision::mid_run_diag::MID_INRANGE_TICKS.fetch_add(1, Ordering::Relaxed);
                }
                // Tier 1 — a CLEAR, good-angle chance in range is taken by
                // ANY midfielder. This is chance-quality, NOT a skill gate:
                // whether you SHOOT a clear central look doesn't depend on
                // how good a finisher you are (skill decides whether it goes
                // IN — see the conversion gradient). Without this the
                // willingness roll declines half of even the clean chances
                // and they're recycled away, dropping mid goals AND team
                // totals. The anti-monopoly cap still applies: once a player
                // has hogged (>6 attempts) this falls through to the
                // willingness roll like everyone else, so it can't be abused.
                let sp = ctx.player().shooting().shot_profile();
                // Lowered xG bar from 0.12 → 0.085. A midfielder arriving
                // centrally with a clear sight of goal in standard range
                // (≤52u, edge of box) at xG ~0.10 is a quintessential
                // box-to-box chance — the previous bar of 0.12 made the
                // tier near-unreachable, leaving midfielders dependent on
                // the forward-tuned willingness roll. Effect on goal
                // distribution: MID share lifts from ~10% toward the
                // realistic ~30% without touching forward behaviour.
                let clear_good = distance_to_goal <= STANDARD_SHOOTING_DISTANCE
                    && coach.shooting_reluctance() < 0.5
                    && ctx.player().has_clear_shot()
                    && ctx.player().shooting().has_good_angle()
                    && sp.expected_xg(distance_to_goal, true) >= 0.085;
                if clear_good && ctx.memory().shots_taken <= 6 {
                    #[cfg(feature = "match-logs")]
                    {
                        use std::sync::atomic::Ordering;
                        crate::r#match::player::strategies::common::players::ops::forward_shot_decision::mid_run_diag::MID_SHOOT_FIRED.fetch_add(1, Ordering::Relaxed);
                    }
                    return Some(
                        StateChangeResult::with_midfielder_state(MidfielderState::Shooting)
                            .with_shot_reason("MID_CLEAR_CHANCE"),
                    );
                }
                // Tier 2 — speculative / long-range / hogger: skill-driven
                // willingness via the shared helper.
                match evaluate_forward_shot_decision(ctx, "MID_SHOOT") {
                    ShotDecision::Shoot { reason } => {
                        #[cfg(feature = "match-logs")]
                        {
                            use std::sync::atomic::Ordering;
                            crate::r#match::player::strategies::common::players::ops::forward_shot_decision::mid_run_diag::MID_SHOOT_FIRED.fetch_add(1, Ordering::Relaxed);
                        }
                        // Beyond standard range, route to the dedicated
                        // long-range strike; closer is a normal finish.
                        let state = if distance_to_goal > STANDARD_SHOOTING_DISTANCE {
                            MidfielderState::DistanceShooting
                        } else {
                            MidfielderState::Shooting
                        };
                        return Some(
                            StateChangeResult::with_midfielder_state(state)
                                .with_shot_reason(reason),
                        );
                    }
                    ShotDecision::Pass => {
                        // Helper judged a teammate the better option — lay
                        // it off (the playmaker's creative choice).
                        if let Some((target, _)) = self.find_best_pass_option(ctx) {
                            return Some(StateChangeResult::with_midfielder_state_and_event(
                                MidfielderState::Standing,
                                Event::PlayerEvent(PlayerEvent::PassTo(
                                    PassingEventContext::new()
                                        .with_from_player_id(ctx.player.id)
                                        .with_to_player_id(target.id)
                                        .with_reason("MID_SHOOT_LAYOFF")
                                        .build(ctx),
                                )),
                            ));
                        }
                    }
                    ShotDecision::Hold => {}
                }
            }

            // Coach tempo: if wasting time or slowing down, prefer possession
            if coach.prefer_possession() && distance_to_goal > POINT_BLANK_DISTANCE {
                let ownership_ticks = ctx.tick_context.ball.ownership_duration;
                if ownership_ticks < coach.min_possession_ticks() {
                    return None;
                }
            }

            // PATIENT POSSESSION: use the team-level
            // `should_play_possession` check so all real-football
            // triggers apply (just won ball, tired, leading, late
            // game, no attack ready). See team/team.rs for the rules.
            let under_pressure = ctx.players().opponents().exists(15.0);
            if !under_pressure
                && distance_to_goal > 70.0
                && ctx.tick_context.ball.ownership_duration > 8
                && ctx.team().should_play_possession()
            {
                if let Some(target) = self.find_best_pass_option(ctx).map(|(t, _)| t) {
                    // Score the candidate: only pass if it's a safe,
                    // sideways/backward option (we don't want to fire
                    // the ball forward into a covered attacker).
                    let player_pos = ctx.player.position;
                    let goal_pos = ctx.player().opponent_goal_position();
                    let to_goal = (goal_pos - player_pos).normalize();
                    let to_t = (target.position - player_pos).normalize();
                    let forward_component = to_t.dot(&to_goal);
                    let target_in_space =
                        ctx.tick_context.grid.opponents(target.id, 10.0).count() < 2;
                    // Accept lateral, backward, or mildly-forward only
                    if forward_component < 0.4 && target_in_space {
                        return Some(StateChangeResult::with_midfielder_state_and_event(
                            MidfielderState::Standing,
                            Event::PlayerEvent(PlayerEvent::PassTo(
                                PassingEventContext::new()
                                    .with_from_player_id(ctx.player.id)
                                    .with_to_player_id(target.id)
                                    .with_reason("MID_PATIENT_POSSESSION")
                                    .build(ctx),
                            )),
                        ));
                    }
                }
                // No safe outlet yet — keep the ball, re-evaluate next tick
                return None;
            }

            // (Shooting — including the box arrival / cutback finish and
            // point-blank chances — is handled by the unified skill-driven
            // helper block hoisted above; no separate carve-outs needed.)

            // Priority: Clear ball if congested anywhere (not just boundaries)
            // Only attempt after carrying ball for a while to prevent instant pass-after-receive
            if (self.is_congested_near_boundary(ctx) || ctx.player().movement().is_congested())
                && ctx.in_state_time > 20
                && ctx.tick_context.ball.ownership_duration > 15
            {
                // Try to find a good pass option first using the standard evaluator
                if let Some((target_teammate, _reason)) = self.find_best_pass_option(ctx) {
                    let dist = (target_teammate.position - ctx.player.position).magnitude();
                    // Only pass if target is far enough away to escape congestion
                    if dist > 40.0 {
                        return Some(StateChangeResult::with_midfielder_state_and_event(
                            MidfielderState::Standing,
                            Event::PlayerEvent(PlayerEvent::PassTo(
                                PassingEventContext::new()
                                    .with_from_player_id(ctx.player.id)
                                    .with_to_player_id(target_teammate.id)
                                    .with_reason("MID_RUNNING_EMERGENCY_CLEARANCE_BEST")
                                    .build(ctx),
                            )),
                        ));
                    }
                }

                // Fallback: find teammate at least 40 units away, not a recent passer,
                // and in open space (outside congestion zone)
                if let Some(target_teammate) = ctx
                    .players()
                    .teammates()
                    .nearby(200.0)
                    .filter(|t| {
                        let dist = (t.position - ctx.player.position).magnitude();
                        dist > 40.0
                            && ctx.ball().passer_recency_penalty(t.id) > 0.3
                            && ctx.tick_context.grid.opponents(t.id, 15.0).count() < 2
                    })
                    .max_by(|a, b| {
                        // Prefer the farthest teammate in open space
                        let da = (a.position - ctx.player.position).magnitude();
                        let db = (b.position - ctx.player.position).magnitude();
                        da.partial_cmp(&db).unwrap_or(Ordering::Equal)
                    })
                {
                    return Some(StateChangeResult::with_midfielder_state_and_event(
                        MidfielderState::Standing,
                        Event::PlayerEvent(PlayerEvent::PassTo(
                            PassingEventContext::new()
                                .with_from_player_id(ctx.player.id)
                                .with_to_player_id(target_teammate.id)
                                .with_reason("MID_RUNNING_EMERGENCY_CLEARANCE_NEARBY")
                                .build(ctx),
                        )),
                    ));
                }
            }

            // Shooting is evaluated earlier (the SHOOT-FIRST block above,
            // hoisted ahead of the possession / pass-recycling defaults).
            // `mid_profile` / `goal_dist` are still needed by the
            // carry-forward and dribble gates below.
            let goal_dist = ctx.ball().distance_to_opponent_goal();
            let mid_profile = MidfielderSkillProfile::from_ctx(ctx);

            // CARRY FORWARD: Open path to goal — gate on carry_selection
            // (dribbling / decisions / composure / acceleration / agility
            // composite). Replaces the ad-hoc dribbling+composure+pace
            // blend with the unified midfielder profile.
            let field_width = ctx.context.field_size.width as f32;
            if goal_dist > POINT_BLANK_DISTANCE
                && goal_dist < field_width * 0.45
                && self.has_open_space_ahead(ctx)
                && mid_profile.allows_carry_into_space()
            {
                return None;
            }

            // Minimum carry time before considering passes — let midfielders run with the ball
            let ownership_ticks = ctx.tick_context.ball.ownership_duration;

            // DRIBBLE: If there's space ahead and player has carry skill,
            // beat opponents. Gated by carry_selection thresholds:
            //   * `allows_take_on_one`  → at most 1 opponent ahead.
            //   * `allows_take_on_two`  → up to 2.
            //   * lower → never dribble into opponents.
            if ownership_ticks > 5 && ownership_ticks < 60 {
                let goal_pos = ctx.player().opponent_goal_position();
                let player_pos = ctx.player.position;
                let to_goal = (goal_pos - player_pos).normalize();

                let opponents_ahead = ctx
                    .players()
                    .opponents()
                    .nearby(40.0)
                    .filter(|opp| {
                        let to_opp = (opp.position - player_pos).normalize();
                        to_opp.dot(&to_goal) > 0.3
                            && (opp.position - player_pos).norm_squared() < 35.0 * 35.0
                    })
                    .count();

                let should_dribble = if opponents_ahead == 0 {
                    false
                } else if mid_profile.allows_take_on_two() {
                    opponents_ahead <= 2
                } else if mid_profile.allows_take_on_one() {
                    opponents_ahead == 1
                } else {
                    false
                };

                let goal_dist_from_opp = ctx.ball().distance_to_opponent_goal();
                let field_width = ctx.context.field_size.width as f32;

                if should_dribble && goal_dist_from_opp < field_width * 0.75 {
                    return Some(StateChangeResult::with_midfielder_state(
                        MidfielderState::Dribbling,
                    ));
                }
            }

            // COUNTER-ATTACK: Quick transition but not instant — need a few ticks to assess
            if ownership_ticks > 8
                && ctx.ball().has_stable_possession()
                && self.is_counter_attack_opportunity(ctx)
            {
                if let Some(forward_target) = self.find_counter_attack_pass(ctx) {
                    return Some(StateChangeResult::with_midfielder_state_and_event(
                        MidfielderState::Running,
                        Event::PlayerEvent(PlayerEvent::PassTo(
                            PassingEventContext::new()
                                .with_from_player_id(ctx.player.id)
                                .with_to_player_id(forward_target.id)
                                .with_reason("MID_RUNNING_COUNTER_ATTACK")
                                .build(ctx),
                        )),
                    ));
                }
            }

            // ONE-TWO COMBINATION: After carrying briefly, check if passer has run ahead
            // into space — return the ball for a wall-pass / give-and-go
            if ownership_ticks >= 10 && ownership_ticks <= 30 && ctx.ball().has_stable_possession()
            {
                if let Some(return_target) = self.find_one_two_return(ctx) {
                    return Some(StateChangeResult::with_midfielder_state_and_event(
                        MidfielderState::Running,
                        Event::PlayerEvent(PlayerEvent::PassTo(
                            PassingEventContext::new()
                                .with_from_player_id(ctx.player.id)
                                .with_to_player_id(return_target.id)
                                .with_reason("MID_RUNNING_ONE_TWO_RETURN")
                                .build(ctx),
                        )),
                    ));
                }
            }

            // DRAW AND RELEASE: If opponent is committing to tackle, draw them in
            // then pass to space they vacated — requires carrying to draw them
            if ownership_ticks > 30 && ctx.ball().has_stable_possession() {
                if let Some(release_target) = self.find_draw_and_release_pass(ctx) {
                    return Some(StateChangeResult::with_midfielder_state_and_event(
                        MidfielderState::Running,
                        Event::PlayerEvent(PlayerEvent::PassTo(
                            PassingEventContext::new()
                                .with_from_player_id(ctx.player.id)
                                .with_to_player_id(release_target.id)
                                .with_reason("MID_RUNNING_DRAW_AND_RELEASE")
                                .build(ctx),
                        )),
                    ));
                }
            }

            // CUTBACK FROM WIDE: a wide carrier near the byline plays a low
            // cutback to a central midfielder arriving unmarked in the box
            // (a first-time shot for the runner) in preference to always
            // launching an aerial cross at the forwards. In a 442 the byline
            // carrier is usually a wide mid, so this is the main engine of
            // midfielder goals. Restricted to a true cutback origin (wide +
            // deep); the shared finder enforces the rest (central runner,
            // in range, unmarked, clear lane). Checked just before CROSSING
            // so a genuine cutback chance is taken over the speculative cross.
            if ownership_ticks > 12 && ctx.ball().has_stable_possession() {
                let field_h = ctx.context.field_size.height as f32;
                let mid_goal = ctx.player().opponent_goal_position();
                // "Deep" = near the byline in X; "off-centre" = poor own
                // angle. Using goal-CENTRE distance here is wrong (a wide
                // carrier is always far from centre), so key off byline X.
                let carrier_byline = (mid_goal.x - ctx.player.position.x).abs() < 90.0;
                let carrier_offcenter =
                    (ctx.player.position.y - field_h / 2.0).abs() > field_h * 0.15;
                if carrier_byline && carrier_offcenter {
                    if let Some(runner) =
                        crate::r#match::player::strategies::common::players::ops::forward_shot_decision::find_cutback_to_arriving_runner(ctx)
                    {
                        #[cfg(feature = "match-logs")]
                        {
                            use std::sync::atomic::Ordering;
                            crate::r#match::player::strategies::common::players::ops::forward_shot_decision::mid_run_diag::MID_CUTBACK.fetch_add(1, Ordering::Relaxed);
                        }
                        return Some(StateChangeResult::with_midfielder_state_and_event(
                            MidfielderState::Standing,
                            Event::PlayerEvent(PlayerEvent::PassTo(
                                PassingEventContext::new()
                                    .with_from_player_id(ctx.player.id)
                                    .with_to_player_id(runner.id)
                                    .with_reason("MID_CUTBACK_TO_RUNNER")
                                    .build(ctx),
                            )),
                        ));
                    }
                }
            }

            // CROSSING: Wide midfielder in attacking third with teammates in the box
            if ownership_ticks > 20 && ctx.ball().has_stable_possession() && self.should_cross(ctx)
            {
                return Some(StateChangeResult::with_midfielder_state(
                    MidfielderState::Crossing,
                ));
            }

            // SWITCH PLAY: When teammates are overloaded on one side, switch to the other flank
            if ownership_ticks > 20 && ctx.ball().has_stable_possession() {
                let field_height = ctx.context.field_size.height as f32;
                let field_center_y = field_height / 2.0;
                let ball_side = if ctx.player.position.y > field_center_y {
                    1.0
                } else {
                    -1.0
                };

                // Count teammates on ball's side
                let teammates_on_side = ctx
                    .players()
                    .teammates()
                    .all()
                    .filter(|t| {
                        let t_side = if t.position.y > field_center_y {
                            1.0
                        } else {
                            -1.0
                        };
                        t_side == ball_side
                    })
                    .count();

                // Switch if overloaded (3+ teammates on same side) or congested
                let should_switch =
                    teammates_on_side >= 3 || ctx.player().movement().is_congested();

                if should_switch && mid_profile.allows_switch_play() {
                    return Some(StateChangeResult::with_midfielder_state(
                        MidfielderState::SwitchingPlay,
                    ));
                }
            }

            // COACH TEMPO: When instructed to slow down, prefer passing back to defenders
            if coach.prefer_possession()
                && ownership_ticks > coach.min_possession_ticks()
                && ctx.ball().has_stable_possession()
            {
                if let Some(safe_target) = self.find_safe_backward_pass(ctx) {
                    return Some(StateChangeResult::with_midfielder_state_and_event(
                        MidfielderState::Standing,
                        Event::PlayerEvent(PlayerEvent::PassTo(
                            PassingEventContext::new()
                                .with_from_player_id(ctx.player.id)
                                .with_to_player_id(safe_target.id)
                                .with_reason("MID_COACH_TEMPO_PASS_BACK")
                                .build(ctx),
                        )),
                    ));
                }
            }

            // Enhanced passing decision — look for a good pass
            if ownership_ticks > 15 && ctx.ball().has_stable_possession() && self.should_pass(ctx) {
                if let Some((target_teammate, _reason)) = self.find_best_pass_option(ctx) {
                    return Some(StateChangeResult::with_midfielder_state_and_event(
                        MidfielderState::Running,
                        Event::PlayerEvent(PlayerEvent::PassTo(
                            PassingEventContext::new()
                                .with_from_player_id(ctx.player.id)
                                .with_to_player_id(target_teammate.id)
                                .with_reason("MID_RUNNING_SHOULD_PASS")
                                .build(ctx),
                        )),
                    ));
                }
            }
        } else {
            // Without ball - check for opponent with ball first
            // Only the closest player should chase — others hold tactical shape
            if let Some(opponent) = ctx
                .players()
                .opponents()
                .nearby(150.0)
                .with_ball(ctx)
                .next()
            {
                let opponent_distance = (opponent.position - ctx.player.position).magnitude();

                // Close — tackle regardless (reactive)
                if opponent_distance < 30.0 {
                    return Some(StateChangeResult::with_midfielder_state(
                        MidfielderState::Tackling,
                    ));
                }

                // Only the best-positioned player presses — prevents team swarming
                if ctx.team().is_best_player_to_chase_ball() {
                    if opponent_distance < 50.0 {
                        return Some(StateChangeResult::with_midfielder_state(
                            MidfielderState::Tackling,
                        ));
                    }
                    return Some(StateChangeResult::with_midfielder_state(
                        MidfielderState::Pressing,
                    ));
                }
                // Others: stay in running (will follow waypoints via velocity)
            }

            // Teammate has the ball — actively support the attack
            if ctx.team().is_control_ball() {
                let ball_distance = ctx.ball().distance();
                let goal_dist = ctx.ball().distance_to_opponent_goal();
                let field_width = ctx.context.field_size.width as f32;

                // ANTI-CLUSTERING: If too many teammates nearby, go find space
                let nearby_teammates = ctx.players().teammates().nearby(25.0).count();
                if nearby_teammates >= 2 && ball_distance > 30.0 {
                    return Some(StateChangeResult::with_midfielder_state(
                        MidfielderState::CreatingSpace,
                    ));
                }

                // If ball is in attacking third and we're nearby, make attacking runs
                if goal_dist < field_width * 0.4 && ball_distance < 300.0 {
                    return Some(StateChangeResult::with_midfielder_state(
                        MidfielderState::AttackSupporting,
                    ));
                }

                // If far from ball, create space to offer a passing option
                if ball_distance > 200.0 {
                    return Some(StateChangeResult::with_midfielder_state(
                        MidfielderState::CreatingSpace,
                    ));
                }

                // Medium range: actively support (don't just drift)
                // Require enough time in Running to avoid rapid oscillation with AttackSupporting
                if ctx.in_state_time > 80 {
                    return Some(StateChangeResult::with_midfielder_state(
                        MidfielderState::AttackSupporting,
                    ));
                }

                // First 80 ticks: stay in Running with active velocity
                return None;
            }

            // Loose ball nearby — only chase if we're the best positioned teammate
            if ctx.ball().distance() < 50.0 && !ctx.ball().is_owned() {
                let ball_velocity = ctx.tick_context.positions.ball.velocity.norm();
                if ball_velocity < 3.0 && ctx.team().is_best_player_to_chase_ball() {
                    return Some(StateChangeResult::with_midfielder_state(
                        MidfielderState::TakeBall,
                    ));
                }
            }

            // Loose-ball claim is handled universally at the dispatcher
            // (`PlayerFieldPositionGroup::process`). Duplicating the check
            // here with the tolerance-banded `is_best_player_to_chase_ball`
            // let multiple players enter TakeBall simultaneously.

            // Also respond to ball system notifications
            if ctx.ball().should_take_ball_immediately()
                && ctx.team().is_best_player_to_chase_ball()
            {
                return Some(StateChangeResult::with_midfielder_state(
                    MidfielderState::TakeBall,
                ));
            }

            // Track dangerous runners — opponent forwards sprinting toward our goal
            if ctx.ball().on_own_side() {
                let own_goal = ctx.ball().direction_to_own_goal();
                let has_dangerous_runner = ctx.players().opponents().forwards().any(|opp| {
                    let dist = (opp.position - ctx.player.position).magnitude();
                    if dist > 60.0 {
                        return false;
                    }
                    let vel = opp.velocity(ctx);
                    let speed = vel.norm();
                    if speed < 2.0 {
                        return false;
                    }
                    let to_goal = (own_goal - opp.position).normalize();
                    let alignment = vel.normalize().dot(&to_goal);
                    alignment > 0.5
                });

                // Dangerous runner detected — close them down via Guarding.
                // TrackingRunner was a single-entry ghost state that did the
                // same "stay goal-side of the runner" thing Guarding already
                // does; keeping Guarding removes the duplicate.
                if has_dangerous_runner {
                    return Some(StateChangeResult::with_midfielder_state(
                        MidfielderState::Guarding,
                    ));
                }
            }

            // Guard unmarked attackers on our side when we can't press/intercept
            if ctx.ball().on_own_side() && ctx.ball().distance() > 100.0 {
                return Some(StateChangeResult::with_midfielder_state(
                    MidfielderState::Guarding,
                ));
            }
        }

        // ANTI-OSCILLATION: If carrying ball too long without acting, force a decision
        // POSSESSION RETENTION: Allow longer holding when team is comfortable
        let anti_oscillation_threshold = if self.should_retain_possession(ctx) {
            250
        } else {
            150
        };
        if ctx.player.has_ball(ctx) && ctx.in_state_time > anti_oscillation_threshold {
            // Prefer passing first
            if let Some((target_teammate, _reason)) = self.find_best_pass_option(ctx) {
                return Some(StateChangeResult::with_midfielder_state_and_event(
                    MidfielderState::Running,
                    Event::PlayerEvent(PlayerEvent::PassTo(
                        PassingEventContext::new()
                            .with_from_player_id(ctx.player.id)
                            .with_to_player_id(target_teammate.id)
                            .with_reason("MID_RUNNING_ANTI_OSCILLATION")
                            .build(ctx),
                    )),
                ));
            }
            // AM carve-out: forward helper picks the anti-oscillation
            // trigger so a low-skill #10 still has a path to a shot
            // when they've been carrying too long without acting.
            if ctx
                .player
                .tactical_position
                .current_position
                .is_attacking_midfielder()
            {
                if let ShotDecision::Shoot { reason } =
                    evaluate_forward_shot_decision(ctx, "AM_RUN_ANTI_OSC_FWD")
                {
                    return Some(
                        StateChangeResult::with_midfielder_state(MidfielderState::Shooting)
                            .with_shot_reason(reason),
                    );
                }
            }
            // Only shoot as fallback at point-blank range with clear shot
            // AND a midfielder with adequate shot selection.
            let distance_to_goal = ctx.ball().distance_to_opponent_goal();
            if distance_to_goal < 25.0 && ctx.player().has_clear_shot() {
                let mid_profile = MidfielderSkillProfile::from_ctx(ctx);
                if mid_profile.mid_shot_selection >= 0.40 {
                    return Some(
                        StateChangeResult::with_midfielder_state(MidfielderState::Shooting)
                            .with_shot_reason("MID_RUN_ANTI_OSCILLATION"),
                    );
                }
            }
            // Last resort: pass to any nearby teammate ahead of the ball (toward opponent goal)
            let player_pos = ctx.player.position;
            let goal_pos = ctx.player().opponent_goal_position();
            let to_goal = (goal_pos - player_pos).normalize();
            if let Some(target_teammate) = ctx
                .players()
                .teammates()
                .nearby(200.0)
                .filter(|t| {
                    let to_teammate = (t.position - player_pos).normalize();
                    to_teammate.dot(&to_goal) > 0.0 // Teammate is ahead (toward opponent goal)
                })
                .next()
            {
                return Some(StateChangeResult::with_midfielder_state_and_event(
                    MidfielderState::Running,
                    Event::PlayerEvent(PlayerEvent::PassTo(
                        PassingEventContext::new()
                            .with_from_player_id(ctx.player.id)
                            .with_to_player_id(target_teammate.id)
                            .with_reason("MID_RUNNING_ANTI_OSCILLATION_FALLBACK")
                            .build(ctx),
                    )),
                ));
            }
            // Absolute last resort: pass to any nearby teammate (even backward)
            if let Some(target_teammate) = ctx.players().teammates().nearby(200.0).next() {
                return Some(StateChangeResult::with_midfielder_state_and_event(
                    MidfielderState::Running,
                    Event::PlayerEvent(PlayerEvent::PassTo(
                        PassingEventContext::new()
                            .with_from_player_id(ctx.player.id)
                            .with_to_player_id(target_teammate.id)
                            .with_reason("MID_RUNNING_ANTI_OSCILLATION_FALLBACK_ANY")
                            .build(ctx),
                    )),
                ));
            }
        }

        None
    }

    fn velocity(&self, ctx: &StateProcessingContext) -> Option<Vector3<f32>> {
        // Simplified waypoint following
        if ctx.player.should_follow_waypoints(ctx) {
            let waypoints = ctx.player.get_waypoints_as_vectors();
            if !waypoints.is_empty() {
                return Some(
                    SteeringBehavior::FollowPath {
                        waypoints,
                        current_waypoint: ctx.player.waypoint_manager.current_index,
                        path_offset: 5.0,
                    }
                    .calculate(ctx.player)
                    .velocity,
                );
            }
        }

        if ctx.player.has_ball(ctx) {
            // POSSESSION RETENTION: When in control mode, move slower with more lateral sway
            // to keep ball and tire opponents instead of always charging forward
            if self.should_retain_possession(ctx) {
                Some(self.calculate_possession_retention_movement(ctx))
            } else {
                Some(self.calculate_simple_ball_movement(ctx))
            }
        } else {
            let start_pos = ctx.player.start_position;
            let field_width = ctx.context.field_size.width as f32;
            let field_height = ctx.context.field_size.height as f32;

            // Off-ball movement when our team isn't in control. Compact
            // defensive block rather than a frozen formation line:
            //   - Ball-side compaction: the whole midfield shifts toward
            //     the ball's lateral (y) position.
            //   - Depth compaction: the line pushes up when the ball
            //     advances, drops back when the ball retreats.
            //   - Ball-side / far-side stagger: the midfielder on the
            //     ball's half steps slightly forward to engage, the
            //     far-side one drops slightly back to cover. This is
            //     what breaks the straight-line "robot" look.
            //   - Separation from nearby teammates naturally spreads
            //     players when formation positions overlap.
            // Loose ball pulls harder (45%) because the designated chaser
            // has already transitioned to TakeBall via `process()`; the
            // rest gently close the gap in support.
            if !ctx.team().is_control_ball() {
                let ball_pos = ctx.tick_context.positions.ball.position;
                let ball_loose = !ctx.ball().is_owned();
                let field_half_x = field_width * 0.5;
                let field_half_y = field_height * 0.5;
                let attacking_left = ctx.player.side == Some(PlayerSide::Left);

                // Lateral (y) shift: always track ball laterally so the
                // midfield slides as a block. 0.3 = a normal compact block,
                // more during a loose-ball scramble (urgency).
                let lateral_coef = if ball_loose { 0.45 } else { 0.30 };
                let lateral_shift = (ball_pos.y - field_half_y) * lateral_coef;

                // Depth (x) shift: push with ball depth. Low coefficient so
                // we don't abandon the defensive line when ball is deep.
                let depth_shift = (ball_pos.x - field_half_x) * 0.15;

                // Ball-side stagger: player on the same lateral half as the
                // ball steps ~10 units toward the opponent's goal to
                // engage; the far-side player drops ~10 units back. A
                // diagonal stagger instead of a flat line.
                let ball_top = ball_pos.y < field_half_y;
                let player_top = start_pos.y < field_half_y;
                let on_ball_side = ball_top == player_top;
                let forward_sign = if attacking_left { 1.0 } else { -1.0 };
                let stagger_x = if on_ball_side { 10.0 } else { -10.0 } * forward_sign;

                let target_x =
                    (start_pos.x + depth_shift + stagger_x).clamp(30.0, field_width - 30.0);
                let target_y = (start_pos.y + lateral_shift).clamp(30.0, field_height - 30.0);

                let target = Vector3::new(target_x, target_y, 0.0);

                // No outer deadzone / zero-velocity early return: the hard
                // stop produced the "arrive-and-jitter" look. Arrive's own
                // quadratic slowing + 3-unit brake zone handles settling.
                // Add separation so players shuffle apart when formation
                // shifts stack them together.
                let arrive = SteeringBehavior::Arrive {
                    target,
                    slowing_distance: 25.0,
                }
                .calculate(ctx.player)
                .velocity;

                return Some(arrive + ctx.player().separation_velocity() * 0.4);
            }

            // Team has ball — off-ball movement: spread across the pitch using unique player slots
            let ball_pos = ctx.tick_context.positions.ball.position;
            let ball_distance = ctx.ball().distance();

            // ANTI-FOLLOWING: If very close to ball carrier, move toward start position
            // to create space instead of calculating a volatile escape direction
            if ball_distance < 40.0 {
                let spread_target = Vector3::new(
                    start_pos.x.clamp(30.0, field_width - 30.0),
                    start_pos.y.clamp(30.0, field_height - 30.0),
                    0.0,
                );
                let dist = (spread_target - ctx.player.position).magnitude();
                if dist < 8.0 {
                    return Some(Vector3::zeros());
                }
                return Some(
                    SteeringBehavior::Arrive {
                        target: spread_target,
                        slowing_distance: 20.0,
                    }
                    .calculate(ctx.player)
                    .velocity,
                );
            }

            let attacking_direction = match ctx.player.side {
                Some(PlayerSide::Left) => 1.0,
                Some(PlayerSide::Right) => -1.0,
                None => 0.0,
            };

            // Quantize ball position to 20-unit grid to prevent wobble
            let qball_x = (ball_pos.x / 20.0).round() * 20.0;
            let qball_y = (ball_pos.y / 20.0).round() * 20.0;

            // Formation-based positioning: stay near start_pos, shift slightly toward ball.
            // Each player keeps their unique formation position — no slot convergence.
            let ball_pull = 0.15; // How much the ball pulls the player (low = keep formation)

            // X: mostly start_pos, pulled slightly toward ball + forward offset
            let forward_offset = attacking_direction * 40.0;
            let target_x = start_pos.x * (1.0 - ball_pull) + (qball_x + forward_offset) * ball_pull;

            // Y: mostly start_pos, pulled slightly toward ball Y
            let target_y = start_pos.y * (1.0 - ball_pull) + qball_y * ball_pull;

            let target = Vector3::new(
                target_x.clamp(30.0, field_width - 30.0),
                target_y.clamp(30.0, field_height - 30.0),
                0.0,
            );

            let dist_to_target = (target - ctx.player.position).magnitude();

            if dist_to_target < 8.0 {
                return Some(Vector3::zeros());
            }

            let arrive_velocity = SteeringBehavior::Arrive {
                target,
                slowing_distance: 20.0,
            }
            .calculate(ctx.player)
            .velocity;

            Some(arrive_velocity)
        }
    }

    fn process_conditions(&self, ctx: ConditionContext) {
        // Midfielders cover the most ground during a match - box to box running
        // High intensity with velocity-based adjustment
        MidfielderCondition::with_velocity(ActivityIntensity::High).process(ctx);
    }
}

impl MidfielderRunningState {
    /// Phase-first dispatch. Midfielders sit in the spine of the team,
    /// so the phase cue drives more of their behaviour than any other
    /// role. Settled phases fall through to the existing decision tree.
    fn phase_dispatch(&self, ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        let phase = ctx.team().phase();
        let has_ball = ctx.player.has_ball(ctx);
        let ball_dist = ctx.ball().distance();
        match phase {
            // Counter-press window after losing the ball. The closest
            // midfielder to the ball engages; others drop into
            // Returning to rebuild shape. Without this window the
            // engine had no concept of "hunt the ball back now" —
            // every press was reactive to distance alone.
            GamePhase::DefensiveTransition if !has_ball => {
                if ball_dist < 45.0 && ctx.team().is_best_player_to_chase_ball() {
                    return Some(StateChangeResult::with_midfielder_state(
                        MidfielderState::Pressing,
                    ));
                }
                // Further-away midfielders reset shape rather than
                // ball-chase into space.
                if ball_dist > 80.0 {
                    return Some(StateChangeResult::with_midfielder_state(
                        MidfielderState::Returning,
                    ));
                }
            }
            // Fast-break window after winning the ball. Midfielder not
            // on the ball makes a forward run to support. The carrier
            // falls through to passing/dribbling below.
            GamePhase::AttackingTransition if !has_ball => {
                if ctx.ball().distance_to_opponent_goal() > 40.0 {
                    return Some(StateChangeResult::with_midfielder_state(
                        MidfielderState::AttackSupporting,
                    ));
                }
            }
            // Coach-triggered high press — midfielders hunt the ball in
            // the opposition half alongside the forwards.
            GamePhase::HighPress if !has_ball => {
                if ball_dist < 70.0 && ctx.team().is_best_player_to_chase_ball() {
                    return Some(StateChangeResult::with_midfielder_state(
                        MidfielderState::Pressing,
                    ));
                }
            }
            // Low-block: cut passing lanes by dropping into the gap
            // between defenders and the ball. Midfielders shouldn't
            // continue chasing upfield in this phase.
            GamePhase::LowBlock if !has_ball => {
                if ball_dist > 50.0 {
                    return Some(StateChangeResult::with_midfielder_state(
                        MidfielderState::Returning,
                    ));
                }
            }
            _ => {}
        }
        None
    }

    fn find_best_pass_option<'a>(
        &self,
        ctx: &StateProcessingContext<'a>,
    ) -> Option<(MatchPlayerLite, &'static str)> {
        PassEvaluator::find_best_pass_option(ctx, 300.0)
    }

    /// Simplified ball carrying movement
    fn calculate_simple_ball_movement(&self, ctx: &StateProcessingContext) -> Vector3<f32> {
        let goal_pos = ctx.player().opponent_goal_position();
        let player_pos = ctx.player.position;

        // Simple decision: move toward goal with slight variation
        let to_goal = (goal_pos - player_pos).normalize();

        // Smooth sinusoidal lateral sway instead of binary flip
        let phase = (ctx.in_state_time as f32) * std::f32::consts::TAU / 60.0;
        let sway = phase.sin() * 0.2;
        let lateral = Vector3::new(-to_goal.y * sway, to_goal.x * sway, 0.0);

        let target = player_pos + (to_goal + lateral).normalize() * 40.0;

        SteeringBehavior::Arrive {
            target,
            slowing_distance: 20.0,
        }
        .calculate(ctx.player)
        .velocity
    }

    /// Enhanced passing decision driven by the unified midfielder
    /// skill profile (pass execution, progressive selection, press
    /// resistance) instead of raw vision/passing/decisions thresholds.
    fn should_pass(&self, ctx: &StateProcessingContext) -> bool {
        let profile = MidfielderSkillProfile::from_ctx(ctx);
        let pressing_intensity = self.calculate_pressing_intensity(ctx);
        let distance_to_goal = ctx.ball().distance_to_opponent_goal();

        // 1. MUST PASS: Heavy pressing — low press_resistance forces a
        // release; high resistance lets us carry/shield briefly.
        if pressing_intensity > 0.7 {
            return profile.press_resistance < 0.55 || profile.pass_execution > 0.30;
        }

        // 2. FORCED PASS: Moderate pressure + middling press resistance.
        if pressing_intensity > 0.5 && profile.press_resistance < 0.60 {
            return true;
        }

        // 3. TACTICAL PASS: Elite progressive playmakers look for the
        // line-breaking option even without pressure.
        if profile.allows_killer_ball()
            && self.has_better_positioned_teammate(ctx, distance_to_goal)
        {
            return true;
        }

        // 4. TEAM PLAY: Good distributors (decent execution + high
        // teamwork curve via pass_execution) pass to maintain tempo.
        if pressing_intensity > 0.3 && profile.pass_execution > 0.55 {
            return self.find_best_pass_option(ctx).is_some();
        }

        // 5. LIGHT PRESSURE: continuous pass likelihood from execution.
        if pressing_intensity > 0.2 {
            return profile.pass_execution > 0.45;
        }

        // 6. NO PRESSURE: midfielders distribute — gate on execution.
        if distance_to_goal > 200.0 && profile.pass_execution > 0.45 {
            return self.has_better_positioned_teammate(ctx, distance_to_goal);
        }

        let ownership_ticks = ctx.tick_context.ball.ownership_duration;
        if ownership_ticks > 60 {
            return true;
        }

        false
    }

    /// Calculate pressing intensity based on number and proximity of opponents
    fn calculate_pressing_intensity(&self, ctx: &StateProcessingContext) -> f32 {
        // Use pre-computed distance closure instead of scanning all players
        let mut weighted_pressure = 0.0f32;
        for (_opp_id, dist) in ctx.tick_context.grid.opponents(ctx.player.id, 50.0) {
            if dist < 15.0 {
                weighted_pressure += 0.5; // very close
            } else if dist < 30.0 {
                weighted_pressure += 0.3; // close
            } else {
                weighted_pressure += 0.1; // medium
            }
        }

        (weighted_pressure / 2.0).min(1.0)
    }

    /// Check if there's a teammate in a better position
    fn has_better_positioned_teammate(
        &self,
        ctx: &StateProcessingContext,
        current_distance: f32,
    ) -> bool {
        ctx.players().teammates().nearby(300.0).any(|teammate| {
            let teammate_distance =
                (teammate.position - ctx.player().opponent_goal_position()).magnitude();
            let is_closer = teammate_distance < current_distance * 0.8;
            if !is_closer {
                return false;
            }
            let has_space = ctx.tick_context.grid.opponents(teammate.id, 30.0).count() < 2;
            if !has_space {
                return false;
            }
            ctx.player().has_clear_pass(teammate.id)
        })
    }

    /// ONE-TWO COMBINATION: Check if the player who just passed to us has run into
    /// a better forward position with space. If so, return the ball for a wall-pass.
    fn find_one_two_return<'a>(&self, ctx: &StateProcessingContext<'a>) -> Option<MatchPlayerLite> {
        let recent_passers = ctx.tick_context.ball.recent_passers();
        // Get the most recent passer (last element in the ring buffer vec)
        let passer_id = *recent_passers.last()?;

        // Passer must be a teammate
        let passer = ctx.context.players.by_id(passer_id)?;
        if passer.team_id != ctx.player.team_id {
            return None;
        }

        // Find passer in nearby players
        let passer_lite = ctx
            .players()
            .teammates()
            .all()
            .find(|t| t.id == passer_id)?;

        let player_pos = ctx.player.position;
        let goal_pos = ctx.player().opponent_goal_position();
        let passer_pos = passer_lite.position;

        // Passer must now be closer to opponent goal than us (they continued their run)
        let our_goal_dist = (goal_pos - player_pos).magnitude();
        let passer_goal_dist = (goal_pos - passer_pos).magnitude();
        if passer_goal_dist >= our_goal_dist * 0.9 {
            return None; // Passer didn't run ahead enough
        }

        // Passer must be in open space (no opponents within 50 units)
        let opponents_near_passer = ctx.tick_context.grid.opponents(passer_id, 50.0).count();
        if opponents_near_passer >= 1 {
            return None;
        }

        // Must have clear passing lane back to passer
        if !ctx.player().has_clear_pass(passer_id) {
            return None;
        }

        // Passer must be within reasonable passing distance
        let pass_distance = (passer_pos - player_pos).magnitude();
        if pass_distance > 200.0 || pass_distance < 10.0 {
            return None;
        }

        Some(passer_lite)
    }

    /// DRAW AND RELEASE: Detect an opponent committing to a tackle (approaching fast
    /// within 15-35 units). Find a teammate in the space the opponent is vacating.
    fn find_draw_and_release_pass<'a>(
        &self,
        ctx: &StateProcessingContext<'a>,
    ) -> Option<MatchPlayerLite> {
        let player_pos = ctx.player.position;

        // Find the closest approaching opponent (within 15-35 units, closing in)
        let approaching_opponent = ctx
            .players()
            .opponents()
            .nearby(35.0)
            .filter(|opp| {
                let dist = (opp.position - player_pos).magnitude();
                if dist < 15.0 || dist > 35.0 {
                    return false;
                }

                // Check if opponent is moving toward us
                let opp_velocity = ctx.tick_context.positions.players.velocity(opp.id);
                if opp_velocity.magnitude() < 1.0 {
                    return false;
                }

                let to_us = (player_pos - opp.position).normalize();
                let opp_dir = opp_velocity.normalize();
                opp_dir.dot(&to_us) > 0.6 // Moving toward us
            })
            .min_by(|a, b| {
                let da = (a.position - player_pos).magnitude();
                let db = (b.position - player_pos).magnitude();
                da.partial_cmp(&db).unwrap_or(Ordering::Equal)
            })?;

        // The space the opponent is vacating is roughly behind them (opposite of their movement)
        let opp_velocity = ctx
            .tick_context
            .positions
            .players
            .velocity(approaching_opponent.id);
        let vacated_zone = approaching_opponent.position - opp_velocity.normalize() * 30.0;

        // Find a teammate near the vacated space (or in the channel the opponent left)
        let best_teammate = ctx
            .players()
            .teammates()
            .nearby(200.0)
            .filter(|t| {
                let t_dist_to_vacated = (t.position - vacated_zone).magnitude();
                // Teammate should be near the vacated space or generally in that direction
                t_dist_to_vacated < 60.0
                    && ctx.player().has_clear_pass(t.id)
                    && ctx.tick_context.grid.opponents(t.id, 10.0).count() < 2
            })
            .min_by(|a, b| {
                let da = (a.position - vacated_zone).magnitude();
                let db = (b.position - vacated_zone).magnitude();
                da.partial_cmp(&db).unwrap_or(Ordering::Equal)
            })?;

        Some(best_teammate)
    }

    /// POSSESSION RETENTION: Determine if team should retain possession rather than
    /// attack directly. True when team is comfortable (not losing), in own/mid third,
    /// and not under heavy pressure.
    fn should_retain_possession(&self, ctx: &StateProcessingContext) -> bool {
        // Never retain if losing
        if ctx.team().is_loosing() {
            return false;
        }

        // Don't retain in attacking third - keep pressing forward
        let goal_dist = ctx.ball().distance_to_opponent_goal();
        let field_width = ctx.context.field_size.width as f32;
        if goal_dist < field_width * 0.35 {
            return false;
        }

        // Don't retain under heavy pressure
        let pressing = self.calculate_pressing_intensity(ctx);
        if pressing > 0.5 {
            return false;
        }

        // Don't retain if open space ahead — advance to create tension
        if self.has_open_space_ahead(ctx) {
            return false;
        }

        // Retain possession when team is in control
        ctx.team().is_control_ball()
    }

    /// Movement for possession retention mode: slower, more lateral, controlled tempo
    fn calculate_possession_retention_movement(
        &self,
        ctx: &StateProcessingContext,
    ) -> Vector3<f32> {
        let goal_pos = ctx.player().opponent_goal_position();
        let player_pos = ctx.player.position;
        let field_height = ctx.context.field_size.height as f32;

        // Move laterally rather than directly toward goal
        // Wider sinusoidal sway with slower forward progress
        let to_goal = (goal_pos - player_pos).normalize();
        let phase = (ctx.in_state_time as f32) * std::f32::consts::TAU / 100.0; // Slower period
        let sway = phase.sin() * 0.5; // Wider lateral sway
        let lateral = Vector3::new(-to_goal.y * sway, to_goal.x * sway, 0.0);

        // Move toward a midfield position rather than directly at goal
        // Blend between lateral movement and slight forward progress
        let mid_y = if player_pos.y < field_height / 2.0 {
            field_height * 0.35
        } else {
            field_height * 0.65
        };
        let retention_target = Vector3::new(
            player_pos.x + to_goal.x * 15.0, // Slow forward drift
            mid_y,
            0.0,
        );

        let blended_target =
            player_pos + (retention_target - player_pos).normalize() * 20.0 + lateral * 10.0;

        SteeringBehavior::Arrive {
            target: blended_target,
            slowing_distance: 30.0,
        }
        .calculate(ctx.player)
        .velocity
            * 0.6 // Slower overall speed in retention mode
    }

    /// COUNTER-ATTACK: Detect if a counter-attack opportunity exists.
    /// True when team just won possession, opponents are high, and space ahead is open.
    fn is_counter_attack_opportunity(&self, ctx: &StateProcessingContext) -> bool {
        let ownership_duration = ctx.tick_context.ball.ownership_duration;

        // Must have just won possession (< 15 ticks)
        if ownership_duration >= 15 {
            return false;
        }

        // Ball must be on own side or midfield (counter goes forward)
        if !ctx.ball().on_own_side() {
            // Allow early midfield counters too
            let goal_dist = ctx.ball().distance_to_opponent_goal();
            let field_width = ctx.context.field_size.width as f32;
            if goal_dist < field_width * 0.4 {
                return false; // Already in attacking third, no need for counter
            }
        }

        // Count opponents ahead of ball (between ball and opponent goal)
        let ball_pos = ctx.tick_context.positions.ball.position;
        let goal_pos = ctx.player().opponent_goal_position();
        let to_goal = (goal_pos - ball_pos).normalize();

        let opponents_ahead = ctx
            .players()
            .opponents()
            .all()
            .filter(|opp| {
                let to_opp = opp.position - ball_pos;
                to_opp.normalize().dot(&to_goal) > 0.3 // Opponent is ahead of ball
            })
            .count();

        // Counter-attack opportunity if few opponents ahead
        opponents_ahead < 3
    }

    /// COUNTER-ATTACK: Find a forward pass target for quick transition.
    /// Prefers forwards making runs toward goal with space around them.
    fn find_counter_attack_pass<'a>(
        &self,
        ctx: &StateProcessingContext<'a>,
    ) -> Option<MatchPlayerLite> {
        let player_pos = ctx.player.position;
        let goal_pos = ctx.player().opponent_goal_position();
        let to_goal = (goal_pos - player_pos).normalize();

        let mut best_target: Option<(MatchPlayerLite, f32)> = None;

        for teammate in ctx.players().teammates().nearby(300.0) {
            let to_teammate = teammate.position - player_pos;

            // Must be ahead of us (toward opponent goal)
            if to_teammate.normalize().dot(&to_goal) < 0.3 {
                continue;
            }

            // Must have space (no opponent within 10 units)
            let opponents_near = ctx.tick_context.grid.opponents(teammate.id, 10.0).count();
            if opponents_near >= 2 {
                continue;
            }

            // Must have clear passing lane
            if !ctx.player().has_clear_pass(teammate.id) {
                continue;
            }

            // Score: prefer forwards, closer to goal, making runs
            let is_forward = teammate.tactical_positions.is_forward();
            let goal_dist = (goal_pos - teammate.position).magnitude();
            let teammate_velocity = ctx.tick_context.positions.players.velocity(teammate.id);
            let making_run = teammate_velocity.magnitude() > 1.0
                && teammate_velocity.normalize().dot(&to_goal) > 0.3;

            let mut score = 1000.0 - goal_dist; // Closer to goal = better
            if is_forward {
                score += 200.0;
            }
            if making_run {
                score += 150.0;
            }
            if opponents_near == 0 {
                score += 100.0;
            }

            if let Some((_, best_score)) = &best_target {
                if score > *best_score {
                    best_target = Some((teammate, score));
                }
            } else {
                best_target = Some((teammate, score));
            }
        }

        best_target.map(|(t, _)| t)
    }

    /// Check if player is stuck in a corner/boundary with multiple players around
    /// Check if there's open space ahead toward the opponent goal
    fn has_open_space_ahead(&self, ctx: &StateProcessingContext) -> bool {
        let player_pos = ctx.player.position;
        let goal_pos = ctx.player().opponent_goal_position();
        let to_goal = (goal_pos - player_pos).normalize();

        // Check for opponents blocking the path ahead (within 30 units, roughly toward goal)
        let blockers = ctx
            .players()
            .opponents()
            .nearby(30.0)
            .filter(|opp| {
                let to_opp = (opp.position - player_pos).normalize();
                to_opp.dot(&to_goal) > 0.4
            })
            .count();

        blockers == 0
    }

    fn is_congested_near_boundary(&self, ctx: &StateProcessingContext) -> bool {
        // Check if near any boundary (within 20 units)
        let field_width = ctx.context.field_size.width as f32;
        let field_height = ctx.context.field_size.height as f32;
        let pos = ctx.player.position;

        let near_boundary = pos.x < 20.0
            || pos.x > field_width - 20.0
            || pos.y < 20.0
            || pos.y > field_height - 20.0;

        if !near_boundary {
            return false;
        }

        // Count all nearby players (teammates + opponents) within 15 units
        let nearby_teammates = ctx
            .tick_context
            .grid
            .teammates(ctx.player.id, 0.0, 15.0)
            .count();
        let nearby_opponents = ctx.tick_context.grid.opponents(ctx.player.id, 15.0).count();
        let total_nearby = nearby_teammates + nearby_opponents;

        // If 3 or more players nearby (congestion), need to clear
        total_nearby >= 3
    }

    /// Check if wide midfielder should deliver a cross into the box
    fn should_cross(&self, ctx: &StateProcessingContext) -> bool {
        let field_height = ctx.context.field_size.height as f32;
        let y = ctx.player.position.y;
        let wide_margin = field_height * 0.2;

        // Must be in a wide channel (top or bottom 20%)
        let is_wide = y < wide_margin || y > field_height - wide_margin;
        if !is_wide {
            return false;
        }

        // Must be in attacking third
        let goal_dist = ctx.ball().distance_to_opponent_goal();
        let field_width = ctx.context.field_size.width as f32;
        if goal_dist > field_width * 0.35 {
            return false;
        }

        // Must have at least 1 teammate within 150 units of opponent goal
        let goal_pos = ctx.player().opponent_goal_position();
        let teammates_in_box = ctx.players().teammates().nearby_at(goal_pos, 150.0).count();
        if teammates_in_box < 1 {
            return false;
        }

        // Crossing skill scales the willingness smoothly (sigmoid pivot
        // at 8/20). A bad crosser still attempts a deep cross
        // occasionally; an elite one almost always does.
        let p = SkillCurve::new(ctx.player.skills.technical.crossing, 8.0, 0.6).probability();
        ctx.context.rng.unit_f32() < p
    }

    /// Find a safe backward/lateral pass target for tempo control.
    /// Prefers defenders and GK when coach says to slow down.
    fn find_safe_backward_pass(&self, ctx: &StateProcessingContext) -> Option<MatchPlayerLite> {
        let player_pos = ctx.player.position;
        let own_goal = ctx.ball().direction_to_own_goal();

        let mut best: Option<(MatchPlayerLite, f32)> = None;

        for teammate in ctx.players().teammates().nearby(250.0) {
            let dist = (teammate.position - player_pos).magnitude();
            if dist < 15.0 {
                continue;
            }

            if !ctx.player().has_clear_pass(teammate.id) {
                continue;
            }

            let opp_near = ctx.tick_context.grid.opponents(teammate.id, 12.0).count();
            if opp_near >= 2 {
                continue;
            }

            let group = teammate.tactical_positions.position_group();
            let mut score = 0.0f32;

            match group {
                PlayerFieldPositionGroup::Goalkeeper => score += 45.0,
                PlayerFieldPositionGroup::Defender => score += 35.0,
                PlayerFieldPositionGroup::Midfielder => score += 10.0,
                _ => {}
            }

            // Prefer players closer to own goal (backward direction)
            let teammate_to_own = (own_goal - teammate.position).magnitude();
            let self_to_own = (own_goal - player_pos).magnitude();
            if teammate_to_own < self_to_own {
                score += 20.0;
            }

            if opp_near == 0 {
                score += 15.0;
            }

            if let Some((_, best_score)) = &best {
                if score > *best_score {
                    best = Some((teammate, score));
                }
            } else {
                best = Some((teammate, score));
            }
        }

        best.map(|(t, _)| t)
    }
}
