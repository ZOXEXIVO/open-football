use crate::PlayerFieldPositionGroup;
use crate::r#match::defenders::states::DefenderState;
use crate::r#match::defenders::states::shared::{
    ActivityIntensity, DefenderCondition, DefenderRead, DefensiveSlot,
};
use crate::r#match::events::Event;
use crate::r#match::midfielders::states::shared::LaneAhead;
use crate::r#match::player::events::{PassingEventContext, PlayerEvent};
use crate::r#match::player::strategies::players::ops::skill_composites as sc;
use crate::r#match::player::strategies::shared::passing::{FlankAction, FlankPlay};
use crate::r#match::player::strategies::shared::players::ops::clearance::{
    ClearanceCall, TouchlineCall,
};
use crate::r#match::player::strategies::shared::players::ops::defender_skill::DefenderSkillProfile;
use crate::r#match::player::strategies::shared::players::ops::forward_shot_decision::{
    ShotDecision, evaluate_forward_shot_decision,
};
use crate::r#match::{
    ConditionContext, MatchContext, MatchPlayerLite, PlayerSide, StateChangeResult,
    StateProcessingContext, StateProcessingHandler, SteeringBehavior,
};
#[cfg(feature = "match-logs")]
use crate::mid_run_diag::DefenderShotDiag;
use nalgebra::Vector3;

/// Reachability cap only — the same 320u (40 m) the forwards and
/// midfielders use. The shared shot helper owns the decision from there.
///
/// 2026-08-16: was **30.0 — three and three-quarter metres**. A defender
/// could only shoot from inside the six-yard box, essentially on the goal
/// line, while every other line was live from 40 m. Defenders took 1.4%
/// of all shots as a result. A centre-back arriving at a corner, or a
/// full-back cutting in, does shoot in football; whether HE shoots is a
/// question about his finishing and the look, and the helper prices both
/// continuously. A blanket range veto answered it before it was asked.
/// Part of the shooting-blocker teardown — see `PlayerMemory::can_shoot`.
const MAX_SHOOTING_DISTANCE: f32 = 320.0;

#[derive(Default, Clone)]
pub struct DefenderRunningState {}

impl StateProcessingHandler for DefenderRunningState {
    #[expect(clippy::too_many_lines)]
    fn process(&self, ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        // The man on the ball. Without it, the read.
        if !ctx.player.has_ball(ctx) {
            return Some(DefenderRead::change(ctx));
        }

        {
            // WINNING IT IN YOUR OWN AREA IS NOT A BUILD-UP SITUATION.
            //
            // Everything below this used to come first — a counter-attack
            // outlet, a coach-tempo hold that returns `None` for any
            // opponent beyond 15u, an emergency pass whose only test is
            // that the selector returned somebody, the wide ladder, the
            // shot — and `should_clear` sat at the bottom. So the one
            // model that reads the squeeze, the danger and whether the
            // out is genuinely an out was the last thing asked and, under
            // `prefer_possession`, was not reached at all between 15u and
            // `ClearanceCall::SQUEEZE_FAR`.
            if let Some(release) = ClearanceCall::decide(ctx).or_else(|| TouchlineCall::decide(ctx))
            {
                return Some(release.resolve(ctx));
            }

            // Counter-attack outlet (see `defenders/states/passing/mod.rs`
            // for the rationale). Defenders frequently fire passes from
            // the Running state on the first tick after winning the ball
            // — long before they're allowed to transition into Passing
            // state — so the same opportunity check has to live here
            // too, or counters that fire here would be missed and the
            // defender would end up recycling backwards via the buildup
            // branches further down.
            if let Some(target) = Self::find_counter_attack_target(ctx) {
                return Some(StateChangeResult::with_defender_state_and_event(
                    DefenderRead::released(ctx),
                    Event::PlayerEvent(PlayerEvent::PassTo(
                        PassingEventContext::builder()
                            .with_from_player_id(ctx.player.id)
                            .with_to_player_id(target.id)
                            .with_reason("DEF_COUNTER_ATTACK")
                            .build(ctx),
                    )),
                ));
            }

            // A side keeping the ball plays it back or across rather than
            // forward — unless pressed or a counter is on, when the break
            // beats it (see TeamOps::counter_window).
            if ctx.team().prefers_keeping_the_ball()
                && !self.is_opponent_closing_fast(ctx)
                && !ctx.team().counter_window()
            {
                if let Some(safe_target) = self.find_safe_backward_pass(ctx) {
                    return Some(StateChangeResult::with_defender_state_and_event(
                        DefenderRead::released(ctx),
                        Event::PlayerEvent(PlayerEvent::PassTo(
                            PassingEventContext::builder()
                                .with_from_player_id(ctx.player.id)
                                .with_to_player_id(safe_target.id)
                                .with_reason("DEF_COACH_TEMPO_PASS_BACK")
                                .build(ctx),
                        )),
                    ));
                }
            }

            // EMERGENCY PASS: Opponent closing fast — pass immediately before they arrive
            if self.is_opponent_closing_fast(ctx) {
                if let Some(target) = self.find_emergency_pass_target(ctx) {
                    return Some(StateChangeResult::with_defender_state_and_event(
                        DefenderRead::released(ctx),
                        Event::PlayerEvent(PlayerEvent::PassTo(
                            PassingEventContext::builder()
                                .with_from_player_id(ctx.player.id)
                                .with_to_player_id(target.id)
                                .with_reason("DEF_EMERGENCY_PASS")
                                .build(ctx),
                        )),
                    ));
                }
                // Nothing from the bespoke search — ask the real one
                // before hoofing it.
                //
                // `find_emergency_pass_target` is a local, hand-rolled
                // scan and it comes up empty far more often than the
                // situation warrants: instrumented, this branch alone is
                // 59% of every clearance in the match (~25 per team per
                // match) and is why defenders measure 10 clearances each
                // against a real ~3.5. The engine already has a pass
                // search that knows about congestion, interception risk,
                // personality and direction; a defender under pressure
                // should consult THAT before deciding there is no ball on.
                // NB `find_safe_pass_option()` is NOT the search to use
                // here: it scans `nearby(50.0)` — fifty units is 6.25 m —
                // so it only ever sees a team-mate close enough to be in
                // the same huddle, and it comes up empty for the same
                // reason the bespoke scan does. `find_best_pass_option`
                // is the evaluator every other line on the pitch uses,
                // at a distance a defender can actually pass.
                if let Some((safe, _)) = ctx
                    .player()
                    .passing()
                    .find_best_pass_option_with_distance(220.0)
                {
                    return Some(StateChangeResult::with_defender_state_and_event(
                        DefenderRead::released(ctx),
                        Event::PlayerEvent(PlayerEvent::PassTo(
                            PassingEventContext::builder()
                                .with_from_player_id(ctx.player.id)
                                .with_to_player_id(safe.id)
                                .with_reason("DEF_EMERGENCY_SAFE_PASS")
                                .build(ctx),
                        )),
                    ));
                }
                #[cfg(feature = "match-logs")]
                crate::r#match::player::strategies::players::ops::forward_shot_decision::mid_run_diag::ClearDiag::note(2);
                return Some(StateChangeResult::with_defender_state(
                    DefenderState::Clearing,
                ));
            }

            // THE WIDE AREA — the same ladder a winger gets, because it
            // is decided by where he is standing and not by what it says
            // on his shirt. An overlapping full-back at the byline is in
            // the identical situation to the man he overlapped, and the
            // engine used to give him a narrower, defender-only version
            // of the decision. See `FlankPlay`.
            match FlankPlay::decide(ctx) {
                Some(FlankAction::ReleaseOutside { target }) => {
                    #[cfg(feature = "match-logs")]
                    crate::mid_run_diag::WideDiag::note(4);
                    return Some(StateChangeResult::with_defender_state_and_event(
                        DefenderState::Running,
                        Event::PlayerEvent(PlayerEvent::PassTo(
                            PassingEventContext::builder()
                                .with_from_player_id(ctx.player.id)
                                .with_to_player_id(target)
                                .with_reason("DEF_FLANK_RELEASE")
                                .build(ctx),
                        )),
                    ));
                }
                Some(FlankAction::Deliver) => {
                    return Some(StateChangeResult::with_defender_state(
                        DefenderState::Crossing,
                    ));
                }
                None => {}
            }

            // Defenders consult the SAME shot helper as everyone else.
            //
            // This used to be a bespoke `shooting_close(..) > 0.40`
            // threshold behind a 3.75 m range cap — the only line in the
            // engine that never reached `evaluate_forward_shot_decision`.
            // Two consequences: a defender's shoot/no-shoot call ignored
            // the angle, the lane, the keeper and the game state that
            // every other player weighs, and no amount of work on the
            // shared model could reach him.
            //
            // Through the helper, a centre-back with 6 finishing still
            // rarely shoots — `execution_skill` and `selection_skill`
            // price exactly that — but he does so because of who he is
            // rather than because of a blanket veto on his position.
            #[cfg(feature = "match-logs")]
            DefenderShotDiag::note_onball(self.is_in_shooting_range(ctx));

            if self.is_in_shooting_range(ctx) && ctx.team().can_shoot() && ctx.player().can_shoot()
            {
                #[cfg(feature = "match-logs")]
                DefenderShotDiag::note_decision();
                match evaluate_forward_shot_decision(ctx, "DEF_SHOOT") {
                    ShotDecision::Shoot { reason } => {
                        return Some(
                            StateChangeResult::with_defender_state(DefenderState::Shooting)
                                .with_shot_reason(reason),
                        );
                    }
                    // A defender who is better off laying it off does that
                    // through his normal passing path, as before.
                    ShotDecision::Pass | ShotDecision::Hold => {}
                }
            }

            if self.should_clear(ctx) {
                return Some(StateChangeResult::with_defender_state(
                    DefenderState::Clearing,
                ));
            }

            // Allow defenders to carry the ball forward when safe
            // This wastes time (good for protecting a lead) and lets teammates rest
            if self.should_carry_ball(ctx) {
                // Stay in Running state — don't pass yet
                return None;
            }

            if let Some(result) = self.try_pass(ctx) {
                return Some(result);
            }
        }

        // SMART BUILD-UP: Graduated response instead of always clearing
        if ctx.player.has_ball(ctx) && ctx.in_state_time > 80 {
            // Under immediate pressure — clear, but only with nothing on.
            //
            // This sat ABOVE the safe-pass search below it and returned
            // unconditionally, so the "graduated response" the block is
            // named for never graduated: any opponent within 15u (1.9 m)
            // after 0.8 s on the ball meant hoof, and the 80-150 tick
            // look-for-a-pass rung underneath was unreachable whenever a
            // defender was actually being closed down — which is the only
            // time it matters. A defender under pressure looks for the
            // out FIRST; that is what playing out of the back is.
            if ctx.players().opponents().exists(15.0) {
                let def_profile = DefenderSkillProfile::from_ctx(ctx);
                if def_profile.must_clear_under_pressure()
                    || self.find_safe_buildup_pass(ctx, 150.0).is_none()
                {
                    #[cfg(feature = "match-logs")]
                    crate::r#match::player::strategies::players::ops::forward_shot_decision::mid_run_diag::ClearDiag::note(3);
                    return Some(StateChangeResult::with_defender_state(
                        DefenderState::Clearing,
                    ));
                }
            }

            // The out first: a safe short pass, then a longer one upfield.
            if let Some(target) = self.find_safe_buildup_pass(ctx, 150.0) {
                return Some(StateChangeResult::with_defender_state_and_event(
                    DefenderRead::released(ctx),
                    Event::PlayerEvent(PlayerEvent::PassTo(
                        PassingEventContext::builder()
                            .with_from_player_id(ctx.player.id)
                            .with_to_player_id(target.id)
                            .with_reason("DEF_RUNNING_BUILDUP_SHORT")
                            .build(ctx),
                    )),
                ));
            }

            if let Some(target) = self.find_safe_buildup_pass(ctx, 300.0) {
                return Some(StateChangeResult::with_defender_state_and_event(
                    DefenderRead::released(ctx),
                    Event::PlayerEvent(PlayerEvent::PassTo(
                        PassingEventContext::builder()
                            .with_from_player_id(ctx.player.id)
                            .with_to_player_id(target.id)
                            .with_reason("DEF_RUNNING_BUILDUP_LONG")
                            .build(ctx),
                    )),
                ));
            }

            // Nothing on. With an opponent closing on him it goes;
            // otherwise he carries on until the stall guard.
            const DANGER_RADIUS: f32 = 48.0;
            if ctx.players().opponents().exists(DANGER_RADIUS) || ctx.in_state_time > 750 {
                #[cfg(feature = "match-logs")]
                crate::r#match::player::strategies::players::ops::forward_shot_decision::mid_run_diag::ClearDiag::note(4);
                return Some(StateChangeResult::with_defender_state(
                    DefenderState::Clearing,
                ));
            }
        }

        None
    }

    fn velocity(&self, ctx: &StateProcessingContext) -> Option<Vector3<f32>> {
        // NOTE: no waypoint branch here, deliberately.
        //
        // This state used to lead with `should_follow_waypoints`, which
        // steered the defender along his tactical route — a straight line
        // running 320u UPFIELD from his base position. The two problems
        // with that compounded each other.
        //
        // First, the predicate is not stable: it returns false while the
        // player is the designated ball-chaser, and that designation is a
        // tolerance band that swaps between team-mates from one tick to
        // the next. So the velocity alternated between "march up my route"
        // and "go to the ball" — frequently opposite directions — without
        // the player ever leaving `Running`. `Defender: Running` was by
        // some distance the largest single source of position flicker
        // left in the engine at that point: **11.7 velocity reversals per
        // second held, 694k of them across three matches, with 94% of all
        // this state's reversals happening with no state change under
        // them** (`dev_match trace`). Zeroing the crowding term changed
        // nothing, which is what identified the branch alternation rather
        // than the steering composition as the cause.
        //
        // Second, and more basic: marching an attacking route is not what
        // this state is for. `Running` is where a defender recovers his
        // shape or goes to engage; pushing up the pitch has its own state
        // (`PushingUp`) and its own phase dispatch. Following the route
        // here also dragged defenders off the defensive line, which is
        // what kept `DefensiveLine::is_in_line` failing in the first
        // place. The route branches remain in `Standing` and `Walking`,
        // where idle shape drift is the intent.
        //
        // What is left are the two ball-relative behaviours below, which
        // cannot invert against each other: engage a nearby carrier, or
        // recover the mark.
        if !ctx.player.has_ball(ctx) {
            return Some(
                SteeringBehavior::Arrive {
                    target: DefensiveSlot::point(ctx),
                    slowing_distance: 30.0,
                }
                .calculate(ctx.player)
                .velocity,
            );
        }
        // A full-back wide and high with the ball drives the byline the way
        // a winger does — `FlankPlay` owns that, and returns `None` for
        // every carrier who is not being shown down the line.
        let target = (!MatchContext::cross_byline_off())
            .then(|| FlankPlay::carry_aim(ctx, LaneAhead::read(ctx).openness))
            .flatten()
            .unwrap_or_else(|| ctx.player().opponent_goal_position());
        Some(
            SteeringBehavior::Arrive {
                target,
                slowing_distance: 100.0,
            }
            .calculate(ctx.player)
            .velocity
                + ctx.player().separation_velocity(),
        )
    }

    fn process_conditions(&self, ctx: ConditionContext) {
        // Running is physically demanding - reduce condition based on intensity and player's stamina
        // Use velocity-based calculation to account for sprinting vs jogging
        DefenderCondition::new(ActivityIntensity::High).process(ctx);
    }
}

impl DefenderRunningState {
    /// Counter-attack outlet — same gates as `DefenderPassingState`.
    /// Returns the long-ball target when (a) we won possession recently,
    /// (b) the opposition over-committed forward, and (c) a teammate is
    /// already in the opposition half. See `passing/mod.rs` for the
    /// rationale and exact gate values.
    fn find_counter_attack_target(ctx: &StateProcessingContext) -> Option<MatchPlayerLite> {
        let ownership_duration = ctx.tick_context.ball.ownership_duration;
        if ownership_duration > 100 {
            return None;
        }
        let side = ctx.player.side?;
        let field_w = ctx.context.field_size.width as f32;
        let halfway_x = field_w * 0.5;
        let opponents_in_our_half = ctx
            .players()
            .opponents()
            .all()
            .filter(|opp| match side {
                PlayerSide::Left => opp.position.x < halfway_x,
                PlayerSide::Right => opp.position.x > halfway_x,
            })
            .count();
        if opponents_in_our_half < 5 {
            return None;
        }
        let target = ctx.players().teammates().nearby_to_opponent_goal()?;
        let target_in_opp_half = match side {
            PlayerSide::Left => target.position.x > halfway_x,
            PlayerSide::Right => target.position.x < halfway_x,
        };
        if !target_in_opp_half {
            return None;
        }
        let pass_distance = (target.position - ctx.player.position).magnitude();
        if !(50.0..=350.0).contains(&pass_distance) {
            return None;
        }
        // Lane-clearness gate — at most one opponent within 14u of the
        // lane. See `defenders/states/passing/mod.rs` for the
        // rationale; without this the counter outlet sprays passes
        // through congested midfield.
        let to_target = target.position - ctx.player.position;
        let target_dir = to_target.normalize();
        let lane_length = to_target.magnitude();
        let opponents_in_lane = ctx
            .players()
            .opponents()
            .all()
            .filter(|opp| {
                let rel = opp.position - ctx.player.position;
                let along = rel.dot(&target_dir);
                if along < 5.0 || along > lane_length - 5.0 {
                    return false;
                }
                let proj = ctx.player.position + target_dir * along;
                (opp.position - proj).magnitude() < 14.0
            })
            .count();
        if opponents_in_lane > 1 {
            return None;
        }
        Some(target)
    }

    /// Determine if the defender should carry the ball forward instead of passing immediately.
    /// Useful for time-wasting, letting teammates recover, and advancing play when safe.
    fn should_carry_ball(&self, ctx: &StateProcessingContext) -> bool {
        // Need to have been in state for a few ticks (don't override initial entry logic)
        if ctx.in_state_time < 5 {
            return false;
        }

        // Don't carry too long — eventually must pass (max ~3-4 seconds of carrying)
        if ctx.in_state_time > 200 {
            return false;
        }

        // Never carry in own penalty area — too dangerous
        if ctx.ball().in_own_penalty_area() {
            return false;
        }

        // Check for immediate pressure: opponent closing in = must pass/clear
        if ctx.players().opponents().exists(20.0) {
            return false;
        }

        // No opponent within moderate range — safe to carry
        let has_space_ahead = !ctx.players().opponents().exists(25.0);

        // Dribbling skill threshold — even average defenders can carry when safe.
        // Routed through `dribble_attack` so fatigue + late-game
        // mental drift dampen a tired CB's willingness to carry. The
        // 0.55 threshold preserves the pre-composite calibration; the
        // composite floor (~0.05) keeps the decision well-defined for
        // weak carriers.
        let minute = sc::minute_from_ms(ctx.context.total_match_time);
        let carry_ability = sc::dribble_attack(ctx.player, minute);

        if has_space_ahead && carry_ability > 0.55 {
            return true;
        }

        false
    }

    /// Clear it because he is stuck, not because he is in his own area —
    /// that call belongs to [`ClearanceCall::decide`], which runs before
    /// every other on-ball branch in this state.
    pub fn should_clear(&self, ctx: &StateProcessingContext) -> bool {
        if self.is_congested_near_boundary(ctx) || ctx.player().movement().is_congested() {
            #[cfg(feature = "match-logs")]
            crate::r#match::player::strategies::players::ops::forward_shot_decision::mid_run_diag::ClearDiag::note(1);
            return true;
        }

        false
    }

    /// Check if player is stuck in a corner/boundary with multiple players around
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

        // Count all nearby players (teammates + opponents) within 15 units using pre-computed distances
        let player_id = ctx.player.id;
        let total_nearby = ctx
            .tick_context
            .grid
            .teammates(player_id, 0.0, 15.0)
            .count()
            + ctx.tick_context.grid.opponents(player_id, 15.0).count();

        // If 3 or more players nearby (congestion), need to clear
        total_nearby >= 3
    }

    /// Try to pass the ball. Returns a state change with pass event for direct passes
    /// (build-up, switch play), or transitions to Passing state for pressure situations.
    fn try_pass(&self, ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        // Under direct pressure — skip wait timers and pass immediately
        let under_pressure = ctx.players().opponents().exists(15.0);

        if !under_pressure {
            // ANTI-LOOP: Don't pass immediately after entering Running state (only when safe).
            if ctx.in_state_time < 10 {
                return None;
            }

            // Hold the ball briefly after reclaiming (only when safe)
            let ownership_duration = ctx.tick_context.ball.ownership_duration;
            if ownership_duration < 5 {
                return None;
            }
        }

        // In congested area — prefer carrying OUT of congestion instead of passing into it
        // Only pass if there's a teammate in open space (not in the same cluster)
        let player_id = ctx.player.id;
        let nearby_players = ctx.tick_context.grid.opponents(player_id, 20.0).count()
            + ctx
                .tick_context
                .grid
                .teammates(player_id, 0.0, 20.0)
                .count();
        if nearby_players >= 4 {
            // Heavily congested — only pass to someone FAR from this cluster
            let has_open_target = ctx.players().teammates().nearby(300.0).any(|t| {
                let dist = (t.position - ctx.player.position).magnitude();
                if dist <= 50.0 {
                    return false;
                }
                let opp_near_t = ctx.tick_context.grid.opponents(t.id, 15.0).count();
                opp_near_t < 2 && ctx.player().has_clear_pass(t.id)
            });
            if !has_open_target {
                return None; // No open target — carry the ball out
            }
        }

        // Single scan: count opponents at 12, 15, 30 thresholds
        let mut opp_within_12 = false;
        let mut opp_within_15 = false;
        let mut opp_within_30 = false;
        for (_id, dist) in ctx.tick_context.grid.opponents(player_id, 30.0) {
            opp_within_30 = true;
            if dist <= 15.0 {
                opp_within_15 = true;
            }
            if dist <= 12.0 {
                opp_within_12 = true;
            }
        }

        // Under direct pressure — delegate to Passing state for detailed evaluation
        if opp_within_12 {
            return Some(StateChangeResult::with_defender_state(
                DefenderState::Passing,
            ));
        }

        // If teammates are tired and under moderate pressure — delegate to Passing
        if opp_within_15 && self.are_teammates_tired(ctx) {
            return Some(StateChangeResult::with_defender_state(
                DefenderState::Passing,
            ));
        }

        // BUILD FROM BACK. Routing depends on whether we should play
        // in possession mode (several real-football triggers — just
        // won the ball, tired, leading, late game, attack not ready)
        // or actively progress. `should_play_possession` is the single
        // check; see its docs for the triggers.
        //   * Possession mode → recycle through safe build-up.
        //   * Otherwise        → find a progressive pass forward.
        if !opp_within_30 && ctx.team().is_control_ball() {
            let possession_mode = ctx.team().should_play_possession();
            let target = if possession_mode {
                self.find_safe_buildup_pass(ctx, 200.0)
            } else {
                self.find_progressive_pass_target(ctx)
            };
            if let Some(target) = target {
                let reason = if possession_mode {
                    "DEF_PATIENT_POSSESSION"
                } else {
                    "DEF_BUILD_FROM_BACK"
                };
                return Some(StateChangeResult::with_defender_state_and_event(
                    DefenderRead::released(ctx),
                    Event::PlayerEvent(PlayerEvent::PassTo(
                        PassingEventContext::builder()
                            .with_from_player_id(ctx.player.id)
                            .with_to_player_id(target.id)
                            .with_reason(reason)
                            .build(ctx),
                    )),
                ));
            }
        }

        // Switch play to opposite flank — gated on the defender switch
        // profile (passing/vision/technique/decisions blend) instead of
        // a raw `vision >= 14.0` threshold so technique/composure also
        // count and a fatigued reading drops the gate.
        let def_profile = DefenderSkillProfile::from_ctx(ctx);
        if def_profile.allows_switch()
            && let Some(target) = self.find_open_teammate_on_opposite_side(ctx)
        {
            return Some(StateChangeResult::with_defender_state_and_event(
                DefenderRead::released(ctx),
                Event::PlayerEvent(PlayerEvent::PassTo(
                    PassingEventContext::builder()
                        .with_from_player_id(ctx.player.id)
                        .with_to_player_id(target.id)
                        .with_reason("DEF_SWITCH_PLAY")
                        .build(ctx),
                )),
            ));
        }

        None
    }

    /// Check if nearby teammates are tired (average condition below threshold)
    fn are_teammates_tired(&self, ctx: &StateProcessingContext) -> bool {
        let mut total_condition = 0u32;
        let mut count = 0u32;

        for teammate in ctx.players().teammates().nearby(150.0) {
            if let Some(player) = ctx.context.players.by_id(teammate.id) {
                total_condition += player.player_attributes.condition_percentage();
                count += 1;
            }
        }

        if count == 0 {
            return false;
        }

        let avg_condition = total_condition / count;
        avg_condition < 40
    }

    /// Find the best progressive pass target: teammate ahead in space with clear pass lane.
    /// Returns the highest-scored option factoring direction, position group, and space.
    fn find_progressive_pass_target(
        &self,
        ctx: &StateProcessingContext,
    ) -> Option<MatchPlayerLite> {
        let player_pos = ctx.player.position;
        let goal_pos = ctx.player().opponent_goal_position();
        let to_goal = (goal_pos - player_pos).normalize();

        let mut best: Option<(MatchPlayerLite, f32)> = None;

        for t in ctx.players().teammates().nearby(200.0) {
            if t.tactical_positions.is_goalkeeper() {
                continue;
            }

            let to_t = (t.position - player_pos).normalize();
            let forward = to_t.dot(&to_goal);
            if forward <= 0.2 {
                continue; // Not ahead enough
            }

            let dist = (t.position - player_pos).magnitude();
            if dist < 30.0 {
                continue; // Too close — weak passes create claim loops
            }

            let opponents_near = ctx.tick_context.grid.opponents(t.id, 15.0).count();
            if opponents_near >= 2 {
                continue; // Not in space
            }

            let Some(odds) = ctx.player().clear_pass(t.id) else {
                continue;
            };

            let mut score = forward * 40.0;
            if t.tactical_positions.is_midfielder() {
                score += 20.0;
            }
            if t.tactical_positions.is_forward() {
                score += 30.0;
            }
            if opponents_near == 0 {
                score += 15.0;
            }
            let score = odds.price(score);

            if best.as_ref().is_none_or(|(_, s)| score > *s) {
                best = Some((t, score));
            }
        }

        best.map(|(t, _)| t)
    }

    fn find_open_teammate_on_opposite_side(
        &self,
        ctx: &StateProcessingContext,
    ) -> Option<MatchPlayerLite> {
        let player_position = ctx.player.position;
        let field_width = ctx.context.field_size.width as f32;
        let opposite_side_x = match ctx.player.side {
            Some(PlayerSide::Left) => field_width * 0.75,
            Some(PlayerSide::Right) => field_width * 0.25,
            None => return None,
        };

        ctx.players()
            .teammates()
            .nearby(200.0)
            .filter(|teammate| {
                if teammate.tactical_positions.is_goalkeeper() {
                    return false;
                }

                let is_on_opposite_side = match ctx.player.side {
                    Some(PlayerSide::Left) => teammate.position.x > opposite_side_x,
                    Some(PlayerSide::Right) => teammate.position.x < opposite_side_x,
                    None => false,
                };
                let is_open = ctx
                    .players()
                    .opponents()
                    .nearby_at(teammate.position, 20.0)
                    .next()
                    .is_none();
                is_on_opposite_side && is_open
            })
            .filter_map(|teammate| {
                let odds = ctx.player().clear_pass(teammate.id)?;
                Some((
                    teammate,
                    odds.price(-(teammate.position - player_position).magnitude()),
                ))
            })
            .max_by(|a, b| a.1.total_cmp(&b.1))
            .map(|(teammate, _)| teammate)
    }

    /// Detect if an opponent is sprinting toward this defender and will arrive soon.
    /// Triggers emergency pass to avoid losing possession.
    fn is_opponent_closing_fast(&self, ctx: &StateProcessingContext) -> bool {
        let player_pos = ctx.player.position;
        let player_id = ctx.player.id;

        for (_opp_id, dist) in ctx.tick_context.grid.opponents(player_id, 35.0) {
            if dist < 10.0 {
                // Already very close — emergency
                return true;
            }

            let opp_velocity = ctx.tick_context.positions.players.velocity(_opp_id);
            let opp_speed = opp_velocity.magnitude();

            // Must be moving fast (sprinting).
            //
            // 1.5 is in u/tick, and a human's maximum in this engine is
            // 0.63 — so this `continue` skipped EVERY opponent and the
            // whole predictive read below it (alignment, closing speed,
            // time-to-arrival) has never once executed. The same
            // wrong-scale class of bug as the old `DANGEROUS_RUN_SPEED`
            // values, which "exceeded human max speed, so run-tracking
            // never fired".
            //
            // With it dead, `is_opponent_closing_fast` collapsed to the
            // crude `dist < 10.0` above — an opponent within 1.25 m,
            // whatever he was doing — and that is what has been sending
            // defenders to `Clearing`: 59.5% of every clearance in the
            // match comes from this branch failing to find a pass.
            // 0.40 u/tick is 5 m/s, a genuine attacking run.
            if opp_speed < 0.40 {
                continue;
            }

            // Check if opponent is moving TOWARD the defender
            let opp_pos = ctx.tick_context.positions.players.position(_opp_id);
            let to_defender = (player_pos - opp_pos).normalize();
            let alignment = opp_velocity.normalize().dot(&to_defender);

            // alignment > 0.5 = roughly heading toward defender
            if alignment > 0.5 {
                // Estimate time to arrival: distance / closing_speed
                let closing_speed = opp_speed * alignment;
                let ticks_to_arrive = dist / closing_speed;

                // If they'll arrive within ~20 ticks (~200ms), it's urgent
                if ticks_to_arrive < 20.0 {
                    return true;
                }
            }
        }

        false
    }

    /// Find the best emergency pass target — any open teammate with a clear lane.
    /// Allows shorter passes than normal build-up since urgency overrides preference.
    fn find_emergency_pass_target(&self, ctx: &StateProcessingContext) -> Option<MatchPlayerLite> {
        let player_pos = ctx.player.position;
        let mut best: Option<(MatchPlayerLite, f32)> = None;

        for teammate in ctx.players().teammates().nearby(250.0) {
            // The keeper is an OUTLET, not an exclusion.
            //
            // Skipping him outright meant a defender with his back to
            // goal and a man on him had no ball to play except forward
            // into traffic — and when that wasn't on, the caller hoofed
            // it. Playing back to the goalkeeper is the single most
            // common answer to exactly this situation in real football,
            // and the pass evaluator's own build-up logic already treats
            // it as a legitimate pattern under press.
            //
            // Scored below every outfield option so it stays the last
            // resort it should be, and only when he is genuinely free —
            // a back-pass to a pressed keeper is how goals get given
            // away.
            let is_gk = teammate.tactical_positions.is_goalkeeper();
            if is_gk
                && ctx
                    .tick_context
                    .grid
                    .opponents(teammate.id, 40.0)
                    .next()
                    .is_some()
            {
                continue;
            }

            let dist = (teammate.position - player_pos).magnitude();
            if dist < 15.0 {
                continue; // Too close — pass would be intercepted
            }

            let Some(odds) = ctx.player().clear_pass(teammate.id) else {
                continue;
            };

            // Prefer teammates with more space around them
            let opponents_near = ctx.tick_context.grid.opponents(teammate.id, 10.0).count();

            let mut score = 100.0 - dist * 0.2; // Closer is slightly better for safety
            if opponents_near == 0 {
                score += 30.0;
            }
            if teammate.tactical_positions.is_midfielder() {
                score += 15.0;
            }
            // Behind every outfield option: taken when there is nothing
            // else, which is the point of him.
            if is_gk {
                score -= 60.0;
            }
            let score = odds.price(score);

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

    /// Reachability only.
    ///
    /// `has_clear_shot()` is dropped here for the reason it was dropped
    /// on both forward paths and the midfielder path: it is a hard binary
    /// veto on lane quality, and lane is already priced continuously
    /// inside the helper, which also still rejects a genuinely blocked
    /// one. The defender path was the last place it survived.
    fn is_in_shooting_range(&self, ctx: &StateProcessingContext) -> bool {
        ctx.ball().distance_to_opponent_goal() <= MAX_SHOOTING_DISTANCE
    }

    /// Find the best build-up pass target within `max_distance`.
    ///
    /// Scoring weighs six factors:
    ///   A. **Space around receiver** — distance to the nearest opponent
    ///      is a continuous signal (15u clearance scores better than 8u).
    ///   B. **Pass-lane pressure** — fewer opponents in the passing lane
    ///      means a safer delivery.
    ///   C. **Forward progression** — dot-product toward opponent goal,
    ///      weighted heavily since we want to move the ball up-field.
    ///   D. **Third-man potential** — reward receivers who ALSO have
    ///      forward passing options, because an outlet that can in turn
    ///      progress the ball is strictly more valuable than a dead-end.
    ///   E. **Tactical fit** — high-press response favours quick outlets
    ///      to the goalkeeper or a free centre-back; normal play and
    ///      low-block responses favour midfielders / forwards.
    ///   F. **Coach override** — under WasteTime / ParkTheBus the
    ///      scoring inverts: backward and lateral safe options win.
    #[expect(clippy::too_many_lines)]
    fn find_safe_buildup_pass(
        &self,
        ctx: &StateProcessingContext,
        max_distance: f32,
    ) -> Option<MatchPlayerLite> {
        let player_pos = ctx.player.position;
        let goal_pos = ctx.player().opponent_goal_position();
        let to_goal_vec = goal_pos - player_pos;
        let to_goal = to_goal_vec.normalize();

        let prefer_safe = ctx.team().prefers_keeping_the_ball();
        // Count how many opponents are within pressing range of us —
        // if we're being hunted, shorten the decision window and accept
        // a backward outlet rather than try to play through the press.
        let pressers_on_me = ctx.tick_context.grid.opponents(ctx.player.id, 18.0).count();
        let under_heavy_press = pressers_on_me >= 2;

        // Skill signal: build-up profile (passing/technique/composure/
        // decisions/vision blend, condition-aware) drives how ambitious
        // the diagonals get. Replaces the raw (passing+vision)/2.
        let creativity = DefenderSkillProfile::from_ctx(ctx).buildup_profile;

        let mut best_target: Option<(MatchPlayerLite, f32)> = None;

        for teammate in ctx.players().teammates().nearby(max_distance) {
            if teammate.tactical_positions.is_goalkeeper() && !under_heavy_press {
                // GK is only a valid outlet under pressure — recycling
                // to the keeper in open play kills tempo.
                continue;
            }

            let pass_dist = (teammate.position - player_pos).magnitude();
            if pass_dist < 30.0 {
                continue; // Too close — weak passes create ping-pong claim loops
            }

            let Some(odds) = ctx.player().clear_pass(teammate.id) else {
                continue;
            };

            // A. SPACE AROUND RECEIVER — continuous signal. Finds nearest
            // opponent; closer = penalty. 20u of clearance is "in space",
            // 5u is "tightly marked". Continuous scoring beats the old
            // hard cut-off at 2 opponents.
            let mut nearest_opp_dist = f32::MAX;
            for (_opp_id, d) in ctx.tick_context.grid.opponents(teammate.id, 25.0) {
                if d < nearest_opp_dist {
                    nearest_opp_dist = d;
                }
            }
            // Reject if receiver is literally being tackled
            if nearest_opp_dist < 4.0 {
                continue;
            }
            let space_score = (nearest_opp_dist / 25.0).clamp(0.0, 1.0) * 40.0;

            // B. PASS-LANE PRESSURE — count opponents within 6u of the
            // pass line. Three opponents in the lane is a turnover
            // waiting to happen even if has_clear_pass passed.
            let to_teammate = teammate.position - player_pos;
            let pass_len = to_teammate.magnitude();
            let lane_blockers = if pass_len > 0.1 {
                let lane_dir = to_teammate / pass_len;
                ctx.players()
                    .opponents()
                    .all()
                    .filter(|opp| {
                        let to_opp = opp.position - player_pos;
                        let projection = to_opp.x * lane_dir.x + to_opp.y * lane_dir.y;
                        if projection < 3.0 || projection > pass_len - 3.0 {
                            return false;
                        }
                        let closest = player_pos + lane_dir * projection;
                        let perp = ((opp.position.x - closest.x).powi(2)
                            + (opp.position.y - closest.y).powi(2))
                        .sqrt();
                        perp < 6.0
                    })
                    .count()
            } else {
                0
            };
            let lane_penalty = (lane_blockers as f32) * 10.0;

            // C. FORWARD PROGRESSION — dot product, heavily weighted.
            let forward_component = if pass_len > 0.1 {
                (to_teammate / pass_len).dot(&to_goal)
            } else {
                0.0
            };

            // Hard reject deep-backward passes unless under pressure
            // (where the keeper / a covering centre-back is the right
            // outlet regardless).
            if forward_component < -0.4 && !under_heavy_press {
                continue;
            }

            let progression_score = if prefer_safe {
                // Coach wants tempo control — backward / lateral is FINE
                forward_component.abs() * (-20.0) + 30.0
            } else if under_heavy_press {
                // Pressed — any safe outlet is good, slight forward bias
                forward_component * 20.0 + 10.0
            } else {
                // Open play — reward forward progression strongly
                if forward_component > 0.0 {
                    forward_component * 55.0
                } else {
                    forward_component * 35.0
                }
            };

            // D. THIRD-MAN POTENTIAL — does the receiver have a forward
            // option of their own? Cheap check: count teammates who are
            // both ahead of the receiver AND in space. One such outlet
            // is enough; more doesn't add much.
            let receiver_ahead_options = ctx
                .players()
                .teammates()
                .all()
                .filter(|t| t.id != teammate.id && t.id != ctx.player.id)
                .filter(|t| {
                    let to_t = (t.position - teammate.position).normalize();
                    if to_t.dot(&to_goal) <= 0.25 {
                        return false;
                    }
                    if (t.position - teammate.position).norm_squared() < 15.0 * 15.0 {
                        return false;
                    }
                    let opp_near_t = ctx.tick_context.grid.opponents(t.id, 12.0).count();
                    opp_near_t < 2
                })
                .count();
            let third_man_bonus = (receiver_ahead_options.min(2) as f32) * 10.0;

            // E. TACTICAL / POSITION FIT
            let position_bonus = if teammate.tactical_positions.is_midfielder() {
                22.0
            } else if teammate.tactical_positions.is_forward() {
                if under_heavy_press {
                    // Long ball to a forward under pressure is a
                    // legitimate outlet — score boosted.
                    28.0
                } else if creativity > 0.7 {
                    30.0 // Visionary passer can pick the forward
                } else {
                    15.0
                }
            } else if teammate.tactical_positions.is_goalkeeper() {
                // Only reached if under_heavy_press (gate above). Keeper
                // outlet under pressure is the safest possible option.
                35.0
            } else {
                // Defender — useful as a side switch
                if forward_component.abs() < 0.3 {
                    18.0 // lateral → switch play
                } else {
                    8.0
                }
            };

            // F. DISTANCE PREFERENCE — smooth, no sharp cliff
            let distance_pref = if pass_dist < 80.0 {
                10.0 // short safe pass
            } else if pass_dist < 150.0 {
                8.0
            } else {
                // Long pass costs accuracy; only skilled passers should pick it
                (creativity - 0.5).max(0.0) * 20.0
            };

            let score = odds.price(
                space_score + progression_score + third_man_bonus + position_bonus + distance_pref
                    - lane_penalty,
            );

            if best_target.as_ref().is_none_or(|(_, s)| score > *s) {
                best_target = Some((teammate, score));
            }
        }

        best_target.map(|(t, _)| t)
    }

    /// Pure helper: how many defenders we need behind the ball before
    /// a fullback may overlap. Late-lead game management adds one more.
    /// Exposed so tests can assert the gate's arithmetic directly,
    /// without spinning up an engine fixture.
    #[cfg(test)]
    pub(crate) fn required_behind_ball(
        rest_defense_count: u8,
        minute: f32,
        score_diff: i8,
    ) -> usize {
        let mut required = rest_defense_count as usize + 1;
        if minute > 75.0 && score_diff > 0 {
            required += 1;
        }
        required
    }

    /// Find a safe backward/lateral pass target for tempo control.
    /// Prefers: GK, other defenders, defensive midfielders — prioritizes safety over progression.
    fn find_safe_backward_pass(&self, ctx: &StateProcessingContext) -> Option<MatchPlayerLite> {
        let player_pos = ctx.player.position;
        let own_goal = ctx.ball().direction_to_own_goal();

        let mut best: Option<(MatchPlayerLite, f32)> = None;

        for teammate in ctx.players().teammates().nearby(250.0) {
            let dist = (teammate.position - player_pos).magnitude();
            if dist < 15.0 {
                continue;
            } // too close

            let Some(odds) = ctx.player().clear_pass(teammate.id) else {
                continue;
            };

            // Must not be under heavy pressure
            let opp_near = ctx.tick_context.grid.opponents(teammate.id, 12.0).count();
            if opp_near >= 2 {
                continue;
            }

            let group = teammate.tactical_positions.position_group();
            let mut score = 0.0f32;

            // Strongly prefer goalkeeper and defenders (backward passes)
            match group {
                PlayerFieldPositionGroup::Goalkeeper => score += 50.0,
                PlayerFieldPositionGroup::Defender => score += 35.0,
                PlayerFieldPositionGroup::Midfielder => score += 15.0,
                _ => score += 0.0,
            }

            // Prefer players closer to own goal (backward direction)
            let teammate_to_own_goal = (own_goal - teammate.position).magnitude();
            let self_to_own_goal = (own_goal - player_pos).magnitude();
            if teammate_to_own_goal < self_to_own_goal {
                score += 20.0; // backward pass
            }

            // Prefer open teammates
            if opp_near == 0 {
                score += 15.0;
            }
            let score = odds.price(score);

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

#[cfg(test)]
mod overlap_gate_tests {
    use super::DefenderRunningState;

    #[test]
    fn overlap_requires_rest_defense_plus_one() {
        // Standard back four with 4 nominal rest defenders → need ≥5 behind.
        assert_eq!(DefenderRunningState::required_behind_ball(4, 60.0, 0), 5);
    }

    #[test]
    fn late_lead_adds_one_more_required_defender() {
        let normal = DefenderRunningState::required_behind_ball(4, 60.0, 0);
        let late_lead = DefenderRunningState::required_behind_ball(4, 80.0, 1);
        assert_eq!(late_lead, normal + 1);
    }

    #[test]
    fn early_lead_does_not_add_extra_defender() {
        assert_eq!(
            DefenderRunningState::required_behind_ball(4, 60.0, 1),
            DefenderRunningState::required_behind_ball(4, 60.0, 0),
        );
    }

    #[test]
    fn late_chase_does_not_add_extra_defender() {
        // Game management drops `rest_defense_count` upstream — the
        // overlap gate doesn't pin extra defenders for a chasing side.
        assert_eq!(
            DefenderRunningState::required_behind_ball(4, 80.0, -1),
            DefenderRunningState::required_behind_ball(4, 80.0, 0),
        );
    }
}
