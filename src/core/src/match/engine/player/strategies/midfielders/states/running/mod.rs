use crate::r#match::events::Event;
use crate::r#match::midfielders::states::common::{ActivityIntensity, MidfielderCondition};
use crate::r#match::midfielders::states::MidfielderState;
use crate::r#match::player::events::{PassingEventContext, PlayerEvent};
use crate::r#match::player::strategies::common::players::MatchPlayerIteratorExt;
use crate::r#match::{ConditionContext, MatchPlayerLite, PassEvaluator, StateChangeResult, StateProcessingContext, StateProcessingHandler, SteeringBehavior};
use nalgebra::Vector3;

// Shooting distance constants for midfielders
const MAX_SHOOTING_DISTANCE: f32 = 80.0; // Midfielders rarely shoot from beyond ~40m
const STANDARD_SHOOTING_DISTANCE: f32 = 55.0; // Standard shooting range for midfielders
const PRESSURE_CHECK_DISTANCE: f32 = 10.0; // Distance to check for opponent pressure before shooting
const POINT_BLANK_DISTANCE: f32 = 20.0; // ~10m - must shoot, goalkeeper is right there
const MIN_SHOOTING_DISTANCE: f32 = 5.0;

#[derive(Default, Clone)]
pub struct MidfielderRunningState {}

impl StateProcessingHandler for MidfielderRunningState {
    fn try_fast(&self, ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        if ctx.player.has_ball(ctx) {
            // Priority 0: Point-blank range - MUST shoot regardless of clear shot check
            // This prevents players from colliding with goalkeeper instead of shooting
            let distance_to_goal = ctx.ball().distance_to_opponent_goal();
            if distance_to_goal <= POINT_BLANK_DISTANCE && distance_to_goal > MIN_SHOOTING_DISTANCE {
                return Some(StateChangeResult::with_midfielder_state(
                    MidfielderState::Shooting,
                ));
            }

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
                if let Some(target_teammate) = ctx.players().teammates().nearby(200.0)
                    .filter(|t| {
                        let dist = (t.position - ctx.player.position).magnitude();
                        dist > 40.0
                            && ctx.ball().passer_recency_penalty(t.id) > 0.3
                            && ctx.tick_context.distances
                                .opponents(t.id, 15.0)
                                .count() < 2
                    })
                    .max_by(|a, b| {
                        // Prefer the farthest teammate in open space
                        let da = (a.position - ctx.player.position).magnitude();
                        let db = (b.position - ctx.player.position).magnitude();
                        da.partial_cmp(&db).unwrap_or(std::cmp::Ordering::Equal)
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

            // Shooting evaluation for midfielders
            let goal_dist = ctx.ball().distance_to_opponent_goal();
            let long_shots = ctx.player.skills.technical.long_shots / 20.0;
            let finishing = ctx.player.skills.technical.finishing / 20.0;

            // Standard shooting - in range with reasonable skill
            if goal_dist <= STANDARD_SHOOTING_DISTANCE
                && ctx.player().shooting().in_shooting_range()
            {
                return Some(StateChangeResult::with_midfielder_state(
                    MidfielderState::Shooting,
                ));
            }

            // Unopposed approach — shoot from closer range when path to goal is clear
            if goal_dist < STANDARD_SHOOTING_DISTANCE && self.has_open_space_ahead(ctx) {
                return Some(StateChangeResult::with_midfielder_state(
                    MidfielderState::Shooting,
                ));
            }

            // Distance shooting - long range with good long shot skills
            if goal_dist <= MAX_SHOOTING_DISTANCE
                && long_shots > 0.6
                && finishing > 0.5
            {
                return Some(StateChangeResult::with_midfielder_state(
                    MidfielderState::DistanceShooting,
                ));
            }

            // CARRY FORWARD: Open path to goal — only skilled/brave players carry forward
            // Average midfielders should prefer passing to maintain team play
            let field_width = ctx.context.field_size.width as f32;
            if goal_dist > POINT_BLANK_DISTANCE
                && goal_dist < field_width * 0.45
                && self.has_open_space_ahead(ctx)
            {
                let dribbling = ctx.player.skills.technical.dribbling / 20.0;
                let composure = ctx.player.skills.mental.composure / 20.0;
                let determination = ctx.player.skills.mental.determination / 20.0;
                let pace = ctx.player.skills.physical.pace / 20.0;
                let carry_quality = dribbling * 0.35 + composure * 0.25
                    + determination * 0.2 + pace * 0.2;
                if carry_quality > 0.6 {
                    return None;
                }
            }

            // Minimum carry time before considering passes — let midfielders run with the ball
            let ownership_ticks = ctx.tick_context.ball.ownership_duration;

            // DRIBBLE: If there's space ahead and player has dribbling skill, beat opponents
            if ownership_ticks > 5 && ownership_ticks < 60 {
                let dribbling_skill = ctx.player.skills.technical.dribbling / 20.0;
                let pace = ctx.player.skills.physical.pace / 20.0;

                // Check if there's space to dribble into (no opponent blocking ahead)
                let goal_pos = ctx.player().opponent_goal_position();
                let player_pos = ctx.player.position;
                let to_goal = (goal_pos - player_pos).normalize();

                let opponents_ahead = ctx.players().opponents().nearby(40.0)
                    .filter(|opp| {
                        let to_opp = (opp.position - player_pos).normalize();
                        to_opp.dot(&to_goal) > 0.3 // Opponent is ahead in our direction
                            && (opp.position - player_pos).magnitude() < 35.0
                    })
                    .count();

                // Skilled dribblers take on opponents; others only dribble into open space
                let should_dribble = if dribbling_skill > 0.7 && pace > 0.6 {
                    opponents_ahead <= 2 // Skilled: take on 1-2 opponents
                } else if dribbling_skill > 0.5 {
                    opponents_ahead <= 1 // Decent: take on 1 opponent
                } else {
                    opponents_ahead == 0 // Poor: only open space
                };

                // Don't dribble in own defensive third — too risky
                let goal_dist_from_opp = ctx.ball().distance_to_opponent_goal();
                let field_width = ctx.context.field_size.width as f32;

                if should_dribble && goal_dist_from_opp < field_width * 0.75 {
                    return Some(StateChangeResult::with_midfielder_state(
                        MidfielderState::Dribbling,
                    ));
                }
            }

            // COUNTER-ATTACK: Quick transition but not instant — need a few ticks to assess
            if ownership_ticks > 8 && ctx.ball().has_stable_possession()
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
            if ownership_ticks >= 10 && ownership_ticks <= 30 && ctx.ball().has_stable_possession() {
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

            // CROSSING: Wide midfielder in attacking third with teammates in the box
            if ownership_ticks > 20 && ctx.ball().has_stable_possession()
                && self.should_cross(ctx)
            {
                return Some(StateChangeResult::with_midfielder_state(
                    MidfielderState::Crossing,
                ));
            }

            // SWITCH PLAY: When teammates are overloaded on one side, switch to the other flank
            if ownership_ticks > 20 && ctx.ball().has_stable_possession() {
                let field_height = ctx.context.field_size.height as f32;
                let field_center_y = field_height / 2.0;
                let ball_side = if ctx.player.position.y > field_center_y { 1.0 } else { -1.0 };

                // Count teammates on ball's side
                let teammates_on_side = ctx.players().teammates().all()
                    .filter(|t| {
                        let t_side = if t.position.y > field_center_y { 1.0 } else { -1.0 };
                        t_side == ball_side
                    })
                    .count();

                // Switch if overloaded (3+ teammates on same side) or congested
                let should_switch = teammates_on_side >= 3 || ctx.player().movement().is_congested();

                if should_switch {
                    let vision = ctx.player.skills.mental.vision / 20.0;
                    let passing = ctx.player.skills.technical.passing / 20.0;
                    if vision > 0.4 && passing > 0.4 {
                        return Some(StateChangeResult::with_midfielder_state(
                            MidfielderState::SwitchingPlay,
                        ));
                    }
                }
            }

            // Enhanced passing decision — look for a good pass
            if ownership_ticks > 15 && ctx.ball().has_stable_possession()
                && self.should_pass(ctx)
            {
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
            if let Some(opponent) = ctx.players().opponents().nearby(150.0).with_ball(ctx).next() {
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

            // Notification system: if ball system notified us to take the ball, act immediately
            // But still check we're the best option to prevent swarming
            if ctx.ball().should_take_ball_immediately() && ctx.team().is_best_player_to_chase_ball() {
                return Some(StateChangeResult::with_midfielder_state(
                    MidfielderState::TakeBall,
                ));
            }

            if ctx.ball().distance() < 30.0 && !ctx.ball().is_owned() {
                return Some(StateChangeResult::with_midfielder_state(
                    MidfielderState::Intercepting,
                ));
            }

            // Track dangerous runners — opponent forwards sprinting toward our goal
            if ctx.ball().on_own_side() {
                let own_goal = ctx.ball().direction_to_own_goal();
                let has_dangerous_runner = ctx.players().opponents().forwards()
                    .any(|opp| {
                        let dist = (opp.position - ctx.player.position).magnitude();
                        if dist > 60.0 { return false; }
                        let vel = opp.velocity(ctx);
                        let speed = vel.norm();
                        if speed < 2.0 { return false; }
                        let to_goal = (own_goal - opp.position).normalize();
                        let alignment = vel.normalize().dot(&to_goal);
                        alignment > 0.5
                    });

                if has_dangerous_runner {
                    return Some(StateChangeResult::with_midfielder_state(
                        MidfielderState::TrackingRunner,
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
        let anti_oscillation_threshold = if self.should_retain_possession(ctx) { 250 } else { 150 };
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
            // Only shoot as fallback at point-blank range with clear shot
            let distance_to_goal = ctx.ball().distance_to_opponent_goal();
            if distance_to_goal < 25.0 && ctx.player().has_clear_shot() {
                return Some(StateChangeResult::with_midfielder_state(
                    MidfielderState::Shooting,
                ));
            }
            // Last resort: pass to any nearby teammate ahead of the ball (toward opponent goal)
            let player_pos = ctx.player.position;
            let goal_pos = ctx.player().opponent_goal_position();
            let to_goal = (goal_pos - player_pos).normalize();
            if let Some(target_teammate) = ctx.players().teammates().nearby(200.0)
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

    fn process_slow(&self, _ctx: &StateProcessingContext) -> Option<StateChangeResult> {
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
                        .velocity + ctx.player().separation_velocity(),
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

            // When opponent has ball: hold tactical shape at start position
            // DO NOT converge toward ball — that creates clusters in front of goalkeeper
            if !ctx.team().is_control_ball() {
                let target = Vector3::new(
                    start_pos.x.clamp(30.0, field_width - 30.0),
                    start_pos.y.clamp(30.0, field_height - 30.0),
                    0.0,
                );

                let dist = (target - ctx.player.position).magnitude();
                if dist < 8.0 {
                    return Some(Vector3::new(0.0, 0.0, 0.0));
                }

                let arrive_velocity = SteeringBehavior::Arrive {
                    target,
                    slowing_distance: 20.0,
                }
                .calculate(ctx.player)
                .velocity;

                return Some(arrive_velocity);
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
                return Some(
                    SteeringBehavior::Arrive { target: spread_target, slowing_distance: 20.0 }
                        .calculate(ctx.player).velocity,
                );
            }

            let attacking_direction = match ctx.player.side {
                Some(crate::r#match::PlayerSide::Left) => 1.0,
                Some(crate::r#match::PlayerSide::Right) => -1.0,
                None => 0.0,
            };

            // Moderate ball proximity — don't rush toward the ball
            let proximity = (1.0 - ball_distance / 400.0).clamp(0.05, 0.35);

            let center_y = field_height / 2.0;
            let is_wide = (start_pos.y - center_y).abs() > field_height * 0.2;

            // === UNIQUE PLAYER SLOT: count teammates with lower ID (no alloc, no sort) ===
            let my_id = ctx.player.id;
            let mut slot_index = 0u32;
            let mut total_mids = 1u32; // count self
            for t in ctx.players().teammates().all() {
                if t.tactical_positions.is_midfielder() {
                    total_mids += 1;
                    if t.id < my_id {
                        slot_index += 1;
                    }
                }
            }

            // Spread midfielders across 10%-90% of field height for full width usage
            let slot_y = field_height * 0.10
                + (field_height * 0.80) * (slot_index as f32 + 0.5) / total_mids as f32;

            // X: push ahead of ball to offer passing options, stagger by slot
            let depth_stagger = attacking_direction * (20.0 + (slot_index as f32) * 15.0);
            let support_x = ball_pos.x + attacking_direction * 50.0 + depth_stagger;
            let width_stagger = if is_wide { attacking_direction * 20.0 } else { 0.0 };
            let target_x = start_pos.x + (support_x - start_pos.x) * proximity + width_stagger;

            // Y: blend between assigned slot, start position, and ball — slot dominant for width
            let slot_weight = if is_wide { 0.65 } else { 0.55 };
            let ball_weight = proximity * 0.2;
            let start_weight = (1.0 - slot_weight - ball_weight).max(0.10);
            let target_y = slot_y * slot_weight
                + ball_pos.y * ball_weight
                + start_pos.y * start_weight;

            // Organic drift for natural movement — slow-changing to prevent twitching
            // Quantize time to 100-tick intervals so drift doesn't change every tick
            let quantized_time = (ctx.context.total_match_time / 100) as f32;
            let player_seed = my_id as f32 * 2.39;
            let drift_x = (player_seed + quantized_time * 0.5).sin() * 10.0;
            let drift_y = (player_seed * 1.37 + quantized_time * 0.4).cos() * 8.0;

            let target = Vector3::new(
                (target_x + drift_x).clamp(30.0, field_width - 30.0),
                (target_y + drift_y).clamp(30.0, field_height - 30.0),
                0.0,
            );

            let dist_to_target = (target - ctx.player.position).magnitude();

            // If already close to target, stand still to prevent twitching
            if dist_to_target < 8.0 {
                return Some(Vector3::zeros());
            }

            let arrive_velocity = SteeringBehavior::Arrive {
                target,
                slowing_distance: 20.0,
            }
            .calculate(ctx.player)
            .velocity;

            // Only apply separation when far from target to prevent oscillation
            if dist_to_target > 15.0 {
                let sep_damping = ((dist_to_target - 15.0) / 40.0).min(0.5);
                Some(arrive_velocity + ctx.player().separation_velocity() * sep_damping)
            } else {
                Some(arrive_velocity)
            }
        }
    }

    fn process_conditions(&self, ctx: ConditionContext) {
        // Midfielders cover the most ground during a match - box to box running
        // High intensity with velocity-based adjustment
        MidfielderCondition::with_velocity(ActivityIntensity::High).process(ctx);
    }
}

impl MidfielderRunningState {
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
            .velocity + ctx.player().separation_velocity()
    }


    /// Enhanced passing decision that considers player skills and pressing intensity
    fn should_pass(&self, ctx: &StateProcessingContext) -> bool {
        // Get player skills
        let vision = ctx.player.skills.mental.vision / 20.0;
        let passing = ctx.player.skills.technical.passing / 20.0;
        let decisions = ctx.player.skills.mental.decisions / 20.0;
        let composure = ctx.player.skills.mental.composure / 20.0;
        let teamwork = ctx.player.skills.mental.teamwork / 20.0;

        // Assess pressing situation
        let pressing_intensity = self.calculate_pressing_intensity(ctx);
        let distance_to_goal = ctx.ball().distance_to_opponent_goal();

        // 1. MUST PASS: Heavy pressing (multiple opponents very close)
        if pressing_intensity > 0.7 {
            // Even low-skilled players should pass under heavy pressure
            return passing > 0.3 || composure < 0.5;
        }

        // 2. FORCED PASS: Under moderate pressure with limited skills
        if pressing_intensity > 0.5 && (passing < 0.6 || composure < 0.6) {
            return true;
        }

        // 3. TACTICAL PASS: Skilled players looking for opportunities
        // Players with high vision and passing can spot good passes even without pressure
        if vision > 0.7 && passing > 0.7 {
            // Check if there's a better-positioned teammate
            if self.has_better_positioned_teammate(ctx, distance_to_goal) {
                return true;
            }
        }

        // 4. TEAM PLAY: High teamwork players distribute more
        if teamwork > 0.7 && decisions > 0.6 && pressing_intensity > 0.3 {
            // Midfielders with good teamwork pass to maintain possession and tempo
            return self.find_best_pass_option(ctx).is_some();
        }

        // 5. UNDER LIGHT PRESSURE: Decide based on skills and options
        if pressing_intensity > 0.2 {
            let pass_likelihood = (decisions * 0.4) + (vision * 0.3) + (passing * 0.3);
            return pass_likelihood > 0.5;
        }

        // 6. NO PRESSURE: Midfielders should still distribute — it's their primary role
        // Look for a well-positioned teammate even without pressure
        if distance_to_goal > 200.0 {
            // Good passers actively look for passing options
            let distribution_quality = vision * 0.4 + passing * 0.3 + teamwork * 0.3;
            if distribution_quality > 0.5 {
                return self.has_better_positioned_teammate(ctx, distance_to_goal);
            }
        }

        // Even average midfielders should pass if they've been carrying too long
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
        for (_opp_id, dist) in ctx.tick_context.distances.opponents(ctx.player.id, 50.0) {
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
    fn has_better_positioned_teammate(&self, ctx: &StateProcessingContext, current_distance: f32) -> bool {
        ctx.players()
            .teammates()
            .nearby(300.0)
            .any(|teammate| {
                let teammate_distance = (teammate.position - ctx.player().opponent_goal_position()).magnitude();
                let is_closer = teammate_distance < current_distance * 0.8;
                if !is_closer {
                    return false;
                }
                let has_space = ctx.tick_context.distances
                    .opponents(teammate.id, 30.0)
                    .count() < 2;
                if !has_space {
                    return false;
                }
                ctx.player().has_clear_pass(teammate.id)
            })
    }

    /// Check if there's a teammate in a dangerous attacking position
    fn has_teammate_in_dangerous_position(&self, ctx: &StateProcessingContext) -> bool {
        let goal_pos = ctx.player().opponent_goal_position();
        let field_width = ctx.context.field_size.width as f32;
        let attacking_third_dist = field_width * 0.4;

        ctx.players()
            .teammates()
            .nearby(350.0)
            .any(|teammate| {
                // Prefer forwards and attacking midfielders
                let is_attacker = teammate.tactical_positions.is_forward() ||
                                 teammate.tactical_positions.is_midfielder();
                if !is_attacker {
                    return false;
                }

                // Check if in attacking third
                let teammate_distance = (teammate.position - goal_pos).magnitude();
                if teammate_distance >= attacking_third_dist {
                    return false;
                }

                // Check if in free space (use pre-computed distances)
                let in_free_space = ctx.tick_context.distances
                    .opponents(teammate.id, 12.0)
                    .count() < 2;

                // Check if making a forward run
                let making_run = if !in_free_space {
                    let teammate_velocity = ctx.tick_context.positions.players.velocity(teammate.id);
                    teammate_velocity.magnitude() > 1.5 && {
                        let to_goal = goal_pos - teammate.position;
                        teammate_velocity.normalize().dot(&to_goal.normalize()) > 0.5
                    }
                } else {
                    false // don't need to check if already in free space
                };

                if !in_free_space && !making_run {
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
        let passer_lite = ctx.players().teammates().all()
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
        let opponents_near_passer = ctx.tick_context.distances
            .opponents(passer_id, 50.0)
            .count();
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
    fn find_draw_and_release_pass<'a>(&self, ctx: &StateProcessingContext<'a>) -> Option<MatchPlayerLite> {
        let player_pos = ctx.player.position;

        // Find the closest approaching opponent (within 15-35 units, closing in)
        let approaching_opponent = ctx.players().opponents().nearby(35.0)
            .filter(|opp| {
                let dist = (opp.position - player_pos).magnitude();
                if dist < 15.0 || dist > 35.0 { return false; }

                // Check if opponent is moving toward us
                let opp_velocity = ctx.tick_context.positions.players.velocity(opp.id);
                if opp_velocity.magnitude() < 1.0 { return false; }

                let to_us = (player_pos - opp.position).normalize();
                let opp_dir = opp_velocity.normalize();
                opp_dir.dot(&to_us) > 0.6 // Moving toward us
            })
            .min_by(|a, b| {
                let da = (a.position - player_pos).magnitude();
                let db = (b.position - player_pos).magnitude();
                da.partial_cmp(&db).unwrap_or(std::cmp::Ordering::Equal)
            })?;

        // The space the opponent is vacating is roughly behind them (opposite of their movement)
        let opp_velocity = ctx.tick_context.positions.players.velocity(approaching_opponent.id);
        let vacated_zone = approaching_opponent.position - opp_velocity.normalize() * 30.0;

        // Find a teammate near the vacated space (or in the channel the opponent left)
        let best_teammate = ctx.players().teammates().nearby(200.0)
            .filter(|t| {
                let t_dist_to_vacated = (t.position - vacated_zone).magnitude();
                // Teammate should be near the vacated space or generally in that direction
                t_dist_to_vacated < 60.0
                    && ctx.player().has_clear_pass(t.id)
                    && ctx.tick_context.distances
                        .opponents(t.id, 10.0)
                        .count() < 2
            })
            .min_by(|a, b| {
                let da = (a.position - vacated_zone).magnitude();
                let db = (b.position - vacated_zone).magnitude();
                da.partial_cmp(&db).unwrap_or(std::cmp::Ordering::Equal)
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
    fn calculate_possession_retention_movement(&self, ctx: &StateProcessingContext) -> Vector3<f32> {
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

        let blended_target = player_pos + (retention_target - player_pos).normalize() * 20.0
            + lateral * 10.0;

        SteeringBehavior::Arrive {
            target: blended_target,
            slowing_distance: 30.0,
        }
        .calculate(ctx.player)
        .velocity * 0.6 // Slower overall speed in retention mode
            + ctx.player().separation_velocity()
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

        let opponents_ahead = ctx.players().opponents().all()
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
    fn find_counter_attack_pass<'a>(&self, ctx: &StateProcessingContext<'a>) -> Option<MatchPlayerLite> {
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
            let opponents_near = ctx.tick_context.distances
                .opponents(teammate.id, 10.0)
                .count();
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
            if is_forward { score += 200.0; }
            if making_run { score += 150.0; }
            if opponents_near == 0 { score += 100.0; }

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
        let blockers = ctx.players().opponents().nearby(30.0)
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
        let nearby_teammates = ctx.tick_context.distances
            .teammates(ctx.player.id, 0.0, 15.0).count();
        let nearby_opponents = ctx.tick_context.distances
            .opponents(ctx.player.id, 15.0).count();
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
        let teammates_in_box = ctx.players().teammates().all()
            .filter(|t| (t.position - goal_pos).magnitude() < 150.0)
            .count();
        if teammates_in_box < 1 {
            return false;
        }

        // Crossing skill must be decent (> 8.0 on 0-20 scale)
        ctx.player.skills.technical.crossing > 8.0
    }
}