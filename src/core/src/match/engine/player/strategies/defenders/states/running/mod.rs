use crate::r#match::defenders::states::DefenderState;
use crate::r#match::defenders::states::common::{ActivityIntensity, DefenderCondition};
use crate::r#match::events::Event;
use crate::r#match::player::events::{PassingEventContext, PlayerEvent};
use crate::r#match::player::strategies::players::DefensiveRole;
use crate::r#match::{
    ConditionContext, GamePhase, MatchPlayerLite, PlayerDistanceFromStartPosition, PlayerSide,
    StateChangeResult, StateProcessingContext, StateProcessingHandler, SteeringBehavior,
};
use crate::{IntegerUtils, PlayerFieldPositionGroup};
use nalgebra::Vector3;

const MAX_SHOOTING_DISTANCE: f32 = 30.0; // Defenders almost never shoot, only from very close

#[derive(Default, Clone)]
pub struct DefenderRunningState {}

impl StateProcessingHandler for DefenderRunningState {
    fn process(&self, ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        // Phase-first dispatch — defenders need the team signal most of
        // all. In a settled phase (LowBlock / MidBlock / Attack) they
        // default to HoldingLine; in transitions they recover or press.
        // Ball-handling falls through.
        if let Some(phase_action) = self.phase_dispatch(ctx) {
            return Some(phase_action);
        }

        if ctx.player.has_ball(ctx) {
            let coach = ctx.team().coach_instruction();

            // COACH INSTRUCTION: When told to waste time or slow down,
            // hold the ball longer before passing (unless under pressure)
            if coach.prefer_possession() && !self.is_opponent_closing_fast(ctx) {
                let ownership_ticks = ctx.tick_context.ball.ownership_duration;
                let min_hold = coach.min_possession_ticks();
                if ownership_ticks < min_hold && !ctx.players().opponents().exists(15.0) {
                    // Keep holding the ball — slow play
                    return None;
                }
                // When finally passing during slow tempo, prefer backward/lateral passes
                // to other defenders or GK rather than forward
                if ownership_ticks >= min_hold {
                    if let Some(safe_target) = self.find_safe_backward_pass(ctx) {
                        return Some(StateChangeResult::with_defender_state_and_event(
                            DefenderState::Standing,
                            Event::PlayerEvent(PlayerEvent::PassTo(
                                PassingEventContext::new()
                                    .with_from_player_id(ctx.player.id)
                                    .with_to_player_id(safe_target.id)
                                    .with_reason("DEF_COACH_TEMPO_PASS_BACK")
                                    .build(ctx),
                            )),
                        ));
                    }
                }
            }

            // EMERGENCY PASS: Opponent closing fast — pass immediately before they arrive
            if self.is_opponent_closing_fast(ctx) {
                if let Some(target) = self.find_emergency_pass_target(ctx) {
                    return Some(StateChangeResult::with_defender_state_and_event(
                        DefenderState::Standing,
                        Event::PlayerEvent(PlayerEvent::PassTo(
                            PassingEventContext::new()
                                .with_from_player_id(ctx.player.id)
                                .with_to_player_id(target.id)
                                .with_reason("DEF_EMERGENCY_PASS")
                                .build(ctx),
                        )),
                    ));
                }
                // No pass target — clear the ball rather than lose it
                return Some(StateChangeResult::with_defender_state(
                    DefenderState::Clearing,
                ));
            }

            // Defenders should almost always pass — only shoot if very close with clear shot
            if self.is_in_shooting_range(ctx) {
                let finishing = ctx.player.skills.technical.finishing / 20.0;
                if finishing > 0.4 {
                    return Some(StateChangeResult::with_defender_state(
                        DefenderState::Shooting,
                    ));
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
        } else {
            // COUNTER-PRESS: route through the team-shared
            // counter-press window. `should_counterpress` already
            // factors distance, work-rate, anticipation, condition,
            // and the team's `press_intensity` floor — so a tired or
            // game-managing team naturally drops more.
            //
            // We still keep a proximity override: the ball within ~30u
            // forces an engagement regardless of the eligibility score
            // (instinct beats math when the carrier is on top of you).
            if ctx.team().counterpress_window() {
                if let Some(opponent) = ctx.players().opponents().with_ball().next() {
                    let role = ctx.player().defensive().defensive_role_for_ball_carrier();
                    let immediate = opponent.distance(ctx) < 30.0;
                    let elected =
                        role == DefensiveRole::Primary && ctx.player().pressure().should_counterpress();
                    if immediate || elected {
                        return Some(StateChangeResult::with_defender_state(
                            DefenderState::Tackling,
                        ));
                    }
                }
            }

            if ctx.player().position_to_distance() == PlayerDistanceFromStartPosition::Big {
                return Some(StateChangeResult::with_defender_state(
                    DefenderState::Returning,
                ));
            }

            // Role-based shape: only drop out of chase when a teammate
            // has actually reached the ball carrier — otherwise everyone
            // holding shape means nobody is contesting possession, and
            // the attacker runs unopposed into shooting range.
            if let Some(opponent) = ctx.players().opponents().with_ball().next() {
                let ball_dist = ctx.ball().distance();
                let teammate_engaging = ctx
                    .players()
                    .teammates()
                    .nearby(30.0)
                    .any(|t| (t.position - opponent.position).magnitude() < 20.0);
                if ball_dist > 30.0 && teammate_engaging {
                    match ctx.player().defensive().defensive_role_for_ball_carrier() {
                        DefensiveRole::Cover => {
                            return Some(StateChangeResult::with_defender_state(
                                DefenderState::Covering,
                            ));
                        }
                        DefensiveRole::Help => {
                            return Some(StateChangeResult::with_defender_state(
                                DefenderState::Marking,
                            ));
                        }
                        DefensiveRole::Hold => {
                            return Some(StateChangeResult::with_defender_state(
                                DefenderState::HoldingLine,
                            ));
                        }
                        DefensiveRole::Primary => {
                            // Primary — fall through to tackle/press checks below.
                        }
                    }
                }
            }

            // Only tackle when we're the designated closer — Primary role
            // or box emergency. The reactive "any defender within 30u
            // lunges" rule produced pileups of 3-4 defenders all
            // attempting tackles simultaneously, each with an independent
            // foul roll. One committing defender + the shape behind
            // them is the right football picture.
            if let Some(opponent) = ctx.players().opponents().with_ball().next() {
                let ball_dist = ctx.ball().distance();
                let is_primary = matches!(
                    ctx.player().defensive().defensive_role_for_ball_carrier(),
                    DefensiveRole::Primary
                );
                let is_emergency = ctx.player().defensive().is_box_emergency_for_me();
                // Very close — only the designated closer tackles
                if ball_dist < 30.0 && (is_primary || is_emergency) {
                    return Some(StateChangeResult::with_defender_state(
                        DefenderState::Tackling,
                    ));
                }
                // Best-positioned chase at medium range
                if ball_dist < 100.0 && ctx.team().is_best_player_to_chase_ball() {
                    return Some(StateChangeResult::with_defender_state(
                        DefenderState::Tackling,
                    ));
                }
                // Carrier running AT us — still engage (we're the wall).
                // Gate this too so a carrier cutting across a line of
                // defenders doesn't trigger all of them.
                if ball_dist < 80.0 && (is_primary || is_emergency) {
                    let carrier_vel = ctx.tick_context.positions.players.velocity(opponent.id);
                    let carrier_speed = carrier_vel.magnitude();
                    if carrier_speed > 0.1 {
                        let to_defender = (ctx.player.position - opponent.position).normalize();
                        let approach = carrier_vel.normalize().dot(&to_defender);
                        if approach > 0.3 {
                            return Some(StateChangeResult::with_defender_state(
                                DefenderState::Tackling,
                            ));
                        }
                    }
                }
            }

            // Loose ball nearby — only chase if we're the best positioned teammate
            if !ctx.ball().is_owned() && ctx.ball().distance() < 50.0 && ctx.ball().speed() < 3.0 {
                if ctx.team().is_best_player_to_chase_ball() {
                    return Some(StateChangeResult::with_defender_state(
                        DefenderState::TakeBall,
                    ));
                }
            }

            // Loose-ball claim lives in the dispatcher.

            // Also respond to ball system notifications
            if ctx.ball().should_take_ball_immediately()
                && ctx.team().is_best_player_to_chase_ball()
            {
                return Some(StateChangeResult::with_defender_state(
                    DefenderState::TakeBall,
                ));
            }

            if !ctx.ball().is_owned() && self.should_intercept(ctx) {
                return Some(StateChangeResult::with_defender_state(
                    DefenderState::Intercepting,
                ));
            }

            // OVERLAPPING RUN: Wide defender pushes up when teammate has ball on same flank
            if self.should_overlap(ctx) {
                return Some(StateChangeResult::with_defender_state(
                    DefenderState::PushingUp,
                ));
            }

            // DEFAULT SHAPE: nothing reactive to do — drop into HoldingLine so
            // the back four holds a coherent defensive line instead of idling
            // in Running (which keeps evaluating chase triggers every tick and
            // pulls shape apart). HoldingLine's own transitions will bring the
            // defender back out if an opponent actually advances on goal.
            // Gated on "near start position" so a defender caught upfield
            // doesn't stall there; the earlier `Returning` branch handles Big
            // displacement and this covers the Small/Medium idle case.
            if ctx.in_state_time > 20
                && ctx.player().position_to_distance() != PlayerDistanceFromStartPosition::Big
            {
                return Some(StateChangeResult::with_defender_state(
                    DefenderState::HoldingLine,
                ));
            }
        }

        // SMART BUILD-UP: Graduated response instead of always clearing
        if ctx.player.has_ball(ctx) && ctx.in_state_time > 80 {
            // Under immediate pressure — clear immediately
            if ctx.players().opponents().exists(15.0) && ctx.in_state_time > 80 {
                return Some(StateChangeResult::with_defender_state(
                    DefenderState::Clearing,
                ));
            }

            // 80-150 ticks: Look for safe short pass to nearby midfielder or defender
            if ctx.in_state_time <= 150 {
                if let Some(target) = self.find_safe_buildup_pass(ctx, 150.0) {
                    return Some(StateChangeResult::with_defender_state_and_event(
                        DefenderState::Standing,
                        Event::PlayerEvent(PlayerEvent::PassTo(
                            PassingEventContext::new()
                                .with_from_player_id(ctx.player.id)
                                .with_to_player_id(target.id)
                                .with_reason("DEF_RUNNING_BUILDUP_SHORT")
                                .build(ctx),
                        )),
                    ));
                }
            }

            // 150-250 ticks: Look for longer pass upfield
            if ctx.in_state_time <= 250 {
                if let Some(target) = self.find_safe_buildup_pass(ctx, 300.0) {
                    return Some(StateChangeResult::with_defender_state_and_event(
                        DefenderState::Standing,
                        Event::PlayerEvent(PlayerEvent::PassTo(
                            PassingEventContext::new()
                                .with_from_player_id(ctx.player.id)
                                .with_to_player_id(target.id)
                                .with_reason("DEF_RUNNING_BUILDUP_LONG")
                                .build(ctx),
                        )),
                    ));
                }
            }

            // 250+ ticks: Force clear as last resort
            if ctx.in_state_time > 250 {
                return Some(StateChangeResult::with_defender_state(
                    DefenderState::Clearing,
                ));
            }
        }

        None
    }

    fn velocity(&self, ctx: &StateProcessingContext) -> Option<Vector3<f32>> {
        if ctx.player.should_follow_waypoints(ctx) {
            let waypoints = ctx.player.get_waypoints_as_vectors();

            if !waypoints.is_empty() {
                return Some(
                    SteeringBehavior::FollowPath {
                        waypoints,
                        current_waypoint: ctx.player.waypoint_manager.current_index,
                        path_offset: IntegerUtils::random(1, 10) as f32,
                    }
                    .calculate(ctx.player)
                    .velocity
                        + ctx.player().separation_velocity(),
                );
            }
        }

        if ctx.player.has_ball(ctx) {
            // With ball: move toward opponent goal, separation matters
            Some(
                SteeringBehavior::Arrive {
                    target: ctx.player().opponent_goal_position(),
                    slowing_distance: 100.0,
                }
                .calculate(ctx.player)
                .velocity
                    + ctx.player().separation_velocity(),
            )
        } else {
            // Without ball: close down on nearby ball carrier, or return to position

            // Check if an opponent has the ball nearby — engage them instead of returning home
            if let Some(ball_carrier) = ctx.players().opponents().with_ball().next() {
                let dist_to_carrier = (ctx.player.position - ball_carrier.position).magnitude();

                if dist_to_carrier < 120.0 {
                    // Get carrier's velocity for pursuit prediction
                    let carrier_velocity =
                        ctx.tick_context.positions.players.velocity(ball_carrier.id);

                    // Use Pursuit to intercept the ball carrier's predicted path
                    let base = SteeringBehavior::Pursuit {
                        target: ball_carrier.position,
                        target_velocity: carrier_velocity,
                    }
                    .calculate(ctx.player)
                    .velocity;

                    return Some(base + ctx.player().separation_velocity());
                }
            }

            // No nearby ball carrier — return to tactical position
            let base = SteeringBehavior::Arrive {
                target: ctx.player.start_position,
                slowing_distance: 30.0,
            }
            .calculate(ctx.player)
            .velocity;

            // Only compute separation if close to start (near other defenders)
            let dist_to_start = (ctx.player.position - ctx.player.start_position).magnitude();
            if dist_to_start < 40.0 {
                Some(base + ctx.player().separation_velocity())
            } else {
                Some(base)
            }
        }
    }

    fn process_conditions(&self, ctx: ConditionContext) {
        // Running is physically demanding - reduce condition based on intensity and player's stamina
        // Use velocity-based calculation to account for sprinting vs jogging
        DefenderCondition::with_velocity(ActivityIntensity::High).process(ctx);
    }
}

impl DefenderRunningState {
    /// Phase-first dispatch for defenders. Defenders hold shape most
    /// of the time — the big wins from phase awareness are "do we
    /// hold a line, drop into a low block, or push up".
    fn phase_dispatch(&self, ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        let phase = ctx.team().phase();
        let has_ball = ctx.player.has_ball(ctx);
        if has_ball {
            return None; // Ball-carrying defenders use build-up logic.
        }
        let ball_dist = ctx.ball().distance();
        let near_start =
            ctx.player().position_to_distance() != PlayerDistanceFromStartPosition::Big;
        match phase {
            // Settled defence in a low block — four defenders form a
            // compact horizontal line. Route to HoldingLine if we're
            // roughly in place; Returning if we're out of position.
            GamePhase::LowBlock => {
                if !near_start {
                    return Some(StateChangeResult::with_defender_state(
                        DefenderState::Returning,
                    ));
                }
                return Some(StateChangeResult::with_defender_state(
                    DefenderState::HoldingLine,
                ));
            }
            // Mid-block: similar to low block but higher up. Same
            // shape-first defaults.
            GamePhase::MidBlock => {
                if !near_start {
                    return Some(StateChangeResult::with_defender_state(
                        DefenderState::Returning,
                    ));
                }
                // Only the closest defender tracks the ball; the rest
                // hold the line.
                if ball_dist < 60.0 && ctx.team().is_best_player_to_chase_ball() {
                    return None; // fall through to the tackling logic
                }
                return Some(StateChangeResult::with_defender_state(
                    DefenderState::HoldingLine,
                ));
            }
            // Defensive transition — the one closest defender engages,
            // others recover shape.
            GamePhase::DefensiveTransition => {
                if ball_dist < 40.0 && ctx.team().is_best_player_to_chase_ball() {
                    return Some(StateChangeResult::with_defender_state(
                        DefenderState::Tackling,
                    ));
                }
                if !near_start {
                    return Some(StateChangeResult::with_defender_state(
                        DefenderState::TrackingBack,
                    ));
                }
            }
            // Attacking phases: wide defenders may overlap; centre-backs
            // hold the line. Falls through to the overlap check below.
            GamePhase::Attack | GamePhase::AttackingTransition | GamePhase::Progression => {}
            _ => {}
        }
        None
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

        // Dribbling skill threshold — even average defenders can carry when safe
        let dribbling = ctx.player.skills.technical.dribbling / 20.0;
        let composure = ctx.player.skills.mental.composure / 20.0;
        let carry_ability = dribbling * 0.6 + composure * 0.4;

        if has_space_ahead && carry_ability > 0.55 {
            return true;
        }

        false
    }

    pub fn should_clear(&self, ctx: &StateProcessingContext) -> bool {
        // Clear if in own penalty area with opponents pressing close
        if ctx.ball().in_own_penalty_area() && ctx.players().opponents().exists(30.0) {
            return true;
        }

        // Clear if congested anywhere (not just boundaries)
        if self.is_congested_near_boundary(ctx) || ctx.player().movement().is_congested() {
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
                    DefenderState::Standing,
                    Event::PlayerEvent(PlayerEvent::PassTo(
                        PassingEventContext::new()
                            .with_from_player_id(ctx.player.id)
                            .with_to_player_id(target.id)
                            .with_reason(reason)
                            .build(ctx),
                    )),
                ));
            }
        }

        // High vision: switch play to opposite flank
        if ctx.player.skills.mental.vision >= 14.0 {
            if let Some(target) = self.find_open_teammate_on_opposite_side(ctx) {
                return Some(StateChangeResult::with_defender_state_and_event(
                    DefenderState::Standing,
                    Event::PlayerEvent(PlayerEvent::PassTo(
                        PassingEventContext::new()
                            .with_from_player_id(ctx.player.id)
                            .with_to_player_id(target.id)
                            .with_reason("DEF_SWITCH_PLAY")
                            .build(ctx),
                    )),
                ));
            }
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

            if !ctx.player().has_clear_pass(t.id) {
                continue;
            }

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

        let mut open_teammates: Vec<MatchPlayerLite> =
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
                    let is_open =
                        !ctx.players().opponents().all().any(|opponent| {
                            (opponent.position - teammate.position).magnitude() < 20.0
                        });
                    is_on_opposite_side && is_open
                })
                .collect();

        if open_teammates.is_empty() {
            None
        } else {
            open_teammates.sort_by(|a, b| {
                let dist_a = (a.position - player_position).magnitude();
                let dist_b = (b.position - player_position).magnitude();
                dist_a.partial_cmp(&dist_b).unwrap()
            });
            Some(open_teammates[0])
        }
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

            // Must be moving fast (sprinting)
            if opp_speed < 1.5 {
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
            if teammate.tactical_positions.is_goalkeeper() {
                continue;
            }

            let dist = (teammate.position - player_pos).magnitude();
            if dist < 15.0 {
                continue; // Too close — pass would be intercepted
            }

            if !ctx.player().has_clear_pass(teammate.id) {
                continue;
            }

            // Prefer teammates with more space around them
            let opponents_near = ctx.tick_context.grid.opponents(teammate.id, 10.0).count();

            let mut score = 100.0 - dist * 0.2; // Closer is slightly better for safety
            if opponents_near == 0 {
                score += 30.0;
            }
            if teammate.tactical_positions.is_midfielder() {
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

    fn is_in_shooting_range(&self, ctx: &StateProcessingContext) -> bool {
        let distance_to_goal = ctx.ball().distance_to_opponent_goal();

        distance_to_goal <= MAX_SHOOTING_DISTANCE && ctx.player().has_clear_shot()
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
    fn find_safe_buildup_pass(
        &self,
        ctx: &StateProcessingContext,
        max_distance: f32,
    ) -> Option<MatchPlayerLite> {
        let player_pos = ctx.player.position;
        let goal_pos = ctx.player().opponent_goal_position();
        let to_goal_vec = goal_pos - player_pos;
        let to_goal = to_goal_vec.normalize();

        let coach = ctx.team().coach_instruction();
        let prefer_safe = coach.prefer_possession();
        // Count how many opponents are within pressing range of us —
        // if we're being hunted, shorten the decision window and accept
        // a backward outlet rather than try to play through the press.
        let pressers_on_me = ctx.tick_context.grid.opponents(ctx.player.id, 18.0).count();
        let under_heavy_press = pressers_on_me >= 2;

        // Skill signal: high passing + vision = attempt more ambitious
        // diagonals; low passers stick to short safe options.
        let pass_skill = ctx.player.skills.technical.passing / 20.0;
        let vision_skill = ctx.player.skills.mental.vision / 20.0;
        let creativity = (pass_skill + vision_skill) * 0.5;

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

            if !ctx.player().has_clear_pass(teammate.id) {
                continue;
            }

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
                    if (t.position - teammate.position).magnitude() < 15.0 {
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

            let score =
                space_score + progression_score + third_man_bonus + position_bonus + distance_pref
                    - lane_penalty;

            if best_target.as_ref().is_none_or(|(_, s)| score > *s) {
                best_target = Some((teammate, score));
            }
        }

        best_target.map(|(t, _)| t)
    }

    /// Should this fullback push up on an overlapping run?
    ///
    /// Polish-spec gate. The classic checks (wide, ball on same flank,
    /// behind carrier, space ahead) are still here, but the "is it safe
    /// to leave shape?" question is now answered by the team-level
    /// `rest_defense_count` rather than a bare "at least one CB
    /// behind me" heuristic. Without the rest-defence gate, both
    /// fullbacks would routinely vacate the back line on the same
    /// possession, and a single counter-attack pass landed straight
    /// behind the team. The gate also requires Attack/Progression phase,
    /// healthy condition + work-rate, and a team_width_target above
    /// 0.45 so a deliberately compact / low-block side never overlaps.
    fn should_overlap(&self, ctx: &StateProcessingContext) -> bool {
        // Must be a wide defender (starting position near touchline)
        let field_height = ctx.context.field_size.height as f32;
        let start_y = ctx.player.start_position.y;
        let is_wide = start_y < field_height * 0.25 || start_y > field_height * 0.75;
        if !is_wide {
            return false;
        }

        // Team must control ball
        if !ctx.team().is_control_ball() {
            return false;
        }

        // Phase gate: only overlap in established attacking phases.
        // Transitions and build-up are too fragile — overlap during a
        // counter break invites a 2v1 the other way the moment the move
        // breaks down.
        let phase = ctx.team().phase();
        if !matches!(phase, GamePhase::Attack | GamePhase::Progression) {
            return false;
        }

        // Width gate: a deliberately compact tactic (low block,
        // possession through the middle) should not push fullbacks wide.
        if ctx.team().team_width_target() <= 0.45 {
            return false;
        }

        // Player must have the legs and willingness for a 50-60m sprint.
        if ctx.player.player_attributes.condition_percentage() <= 55 {
            return false;
        }
        if ctx.player.skills.mental.work_rate <= 10.0 {
            return false;
        }

        // Find teammate with ball on same flank
        let player_on_left_flank = start_y < field_height * 0.5;
        let has_ball_on_same_flank = if let Some(owner_id) = ctx.ball().owner_id() {
            if let Some(owner) = ctx.context.players.by_id(owner_id) {
                if owner.team_id != ctx.player.team_id {
                    return false;
                }
                let ball_pos = ctx.tick_context.positions.ball.position;
                let ball_on_left = ball_pos.y < field_height * 0.5;
                ball_on_left == player_on_left_flank
            } else {
                false
            }
        } else {
            false
        };

        if !has_ball_on_same_flank {
            return false;
        }

        // Defender must be behind the ball carrier
        let ball_pos = ctx.tick_context.positions.ball.position;
        let goal_pos = ctx.player().opponent_goal_position();
        let to_goal = (goal_pos - ctx.player.position).normalize();
        let ball_ahead = (ball_pos - ctx.player.position).normalize().dot(&to_goal) > 0.0;
        if !ball_ahead {
            return false;
        }

        // Rest-defence gate: count teammates whose attacking_progress is
        // strictly behind the ball (with a 0.03 deadband to avoid
        // flapping on the line). Need at least `rest_defense_count + 1`
        // back there before this fullback may advance. Late-lead game
        // management adds one more required defender.
        let side = match ctx.player.side {
            Some(s) => s,
            None => return false,
        };
        let field_width = ctx.context.field_size.width as f32;
        let ball_progress = side.attacking_progress_x(ball_pos.x, field_width);
        let behind_threshold = ball_progress - 0.03;
        let behind_ball_count = ctx
            .players()
            .teammates()
            .all()
            .filter(|t| {
                if t.id == ctx.player.id {
                    return false;
                }
                let progress = side.attacking_progress_x(t.position.x, field_width);
                progress < behind_threshold
            })
            .count();
        let mut required_behind = ctx.team().rest_defense_count() as usize + 1;
        let minute = (ctx.context.total_match_time as f32) / 60_000.0;
        if minute > 75.0 && ctx.team().score_diff() > 0 {
            required_behind += 1;
        }
        if behind_ball_count < required_behind {
            return false;
        }

        // Check space ahead on the wing
        let wing_y = if player_on_left_flank {
            field_height * 0.1
        } else {
            field_height * 0.9
        };
        let ahead_pos = Vector3::new(ball_pos.x + to_goal.x * 50.0, wing_y, 0.0);
        let opponents_blocking = ctx
            .players()
            .opponents()
            .all()
            .filter(|opp| (opp.position - ahead_pos).magnitude() < 30.0)
            .count();

        opponents_blocking == 0
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

            // Must have clear pass lane
            if !ctx.player().has_clear_pass(teammate.id) {
                continue;
            }

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

    fn should_intercept(&self, ctx: &StateProcessingContext) -> bool {
        // Don't intercept if a teammate has the ball
        if let Some(owner_id) = ctx.ball().owner_id() {
            if let Some(owner) = ctx.context.players.by_id(owner_id) {
                if owner.team_id == ctx.player.team_id {
                    // A teammate has the ball, don't try to intercept
                    return false;
                }
            }
        }

        // Only intercept if you're the best player to chase the ball
        if !ctx.team().is_best_player_to_chase_ball() {
            return false;
        }

        // Check if the ball is moving toward this player and is close enough
        if ctx.ball().distance() < 200.0 && ctx.ball().is_towards_player_with_angle(0.8) {
            return true;
        }

        // Check if the ball is very close and no teammate is clearly going for it
        if ctx.ball().distance() < 50.0 && !ctx.team().is_teammate_chasing_ball() {
            return true;
        }

        false
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
