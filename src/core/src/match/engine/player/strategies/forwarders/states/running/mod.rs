use crate::r#match::events::Event;
use crate::r#match::forwarders::states::common::{ActivityIntensity, ForwardCondition};
use crate::r#match::forwarders::states::ForwardState;
use crate::r#match::player::events::{PassingEventContext, PlayerEvent};
use crate::r#match::player::strategies::common::players::MatchPlayerIteratorExt;
use crate::r#match::{
    ConditionContext, MatchPlayerLite, PlayerDistanceFromStartPosition, PlayerSide,
    StateChangeResult, StateProcessingContext, StateProcessingHandler, SteeringBehavior,
};
use crate::IntegerUtils;
use nalgebra::Vector3;

// Realistic shooting distances (field is 840 units)
#[allow(dead_code)]
const MAX_SHOOTING_DISTANCE: f32 = 120.0; // ~60m - absolute max for long shots
const MIN_SHOOTING_DISTANCE: f32 = 5.0;
const POINT_BLANK_DISTANCE: f32 = 30.0; // ~15m - must shoot, goalkeeper is right there
#[allow(dead_code)]
const VERY_CLOSE_RANGE_DISTANCE: f32 = 40.0; // ~20m - anyone can shoot
const CLOSE_RANGE_DISTANCE: f32 = 60.0; // ~30m - close range shots
#[allow(dead_code)]
const OPTIMAL_SHOOTING_DISTANCE: f32 = 80.0; // ~40m - ideal shooting distance
#[allow(dead_code)]
const MEDIUM_RANGE_DISTANCE: f32 = 90.0; // ~45m - medium range shots

// Passing decision thresholds for forwards
const SHOOTING_ZONE_DISTANCE: f32 = 60.0; // Only shoot under pressure from close range
const TEAMMATE_ADVANTAGE_STRICT_RATIO: f32 = 0.7; // Teammate must be 30% closer to override

// Performance thresholds
const SPRINT_DURATION_THRESHOLD: u64 = 150; // Ticks before considering fatigue

#[derive(Default, Clone)]
pub struct ForwardRunningState {}

impl StateProcessingHandler for ForwardRunningState {
    fn try_fast(&self, ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        // Handle cases when player has the ball
        if ctx.player.has_ball(ctx) {
            let distance_to_goal = ctx.ball().distance_to_opponent_goal();

            // Priority 0: Point-blank range — MUST shoot to avoid running into the goalkeeper
            if distance_to_goal <= POINT_BLANK_DISTANCE {
                let finishing = ctx.player.skills.technical.finishing / 20.0;
                // Even at point blank, very poor finishers may try to pass
                if finishing > 0.3 || !ctx.players().teammates().nearby(50.0).any(|_| true) {
                    return Some(StateChangeResult::with_forward_state(
                        ForwardState::Shooting,
                    ));
                }
            }

            // Priority 0.5: In shooting range with clear shot — SHOOT
            if ctx.player().shooting().in_shooting_range() && ctx.player().has_clear_shot() {
                return Some(StateChangeResult::with_forward_state(
                    ForwardState::Shooting,
                ));
            }

            // Priority 0.6: Unopposed approach — shoot from close range with decent finishing
            if distance_to_goal < CLOSE_RANGE_DISTANCE && self.has_open_space_ahead(ctx) {
                let finishing = ctx.player.skills.technical.finishing / 20.0;
                if finishing > 0.45 {
                    return Some(StateChangeResult::with_forward_state(
                        ForwardState::Shooting,
                    ));
                }
            }

            // Priority 0.7: Long-range shooting — skilled players shoot from distance
            {
                let long_shots = ctx.player.skills.technical.long_shots / 20.0;
                let finishing = ctx.player.skills.technical.finishing / 20.0;

                if distance_to_goal > CLOSE_RANGE_DISTANCE
                    && distance_to_goal <= MAX_SHOOTING_DISTANCE
                    && long_shots > 0.6
                    && finishing > 0.5
                    && ctx.player().has_clear_shot()
                    && !ctx.players().opponents().exists(8.0)
                {
                    return Some(StateChangeResult::with_forward_state(
                        ForwardState::Shooting,
                    ));
                }
            }

            // Priority 0.8: Open path to goal — KEEP RUNNING, don't pass or dribble
            // Only brave/skilled players carry the ball forward; others prefer passing
            if distance_to_goal > POINT_BLANK_DISTANCE
                && distance_to_goal < 180.0
                && self.has_open_space_ahead(ctx)
            {
                let dribbling = ctx.player.skills.technical.dribbling / 20.0;
                let composure = ctx.player.skills.mental.composure / 20.0;
                let determination = ctx.player.skills.mental.determination / 20.0;
                let pace = ctx.player.skills.physical.pace / 20.0;
                let carry_quality = dribbling * 0.35 + composure * 0.25
                    + determination * 0.2 + pace * 0.2;
                if carry_quality > 0.55 {
                    return None;
                }
            }

            // ONE-TWO COMBINATION: After just receiving ball, check if passer ran into space
            if ctx.ball().has_stable_possession() {
                let ownership_ticks = ctx.tick_context.ball.ownership_duration;
                if ownership_ticks >= 2 && ownership_ticks <= 10
                    && distance_to_goal > POINT_BLANK_DISTANCE
                {
                    if let Some(return_target) = self.find_one_two_return(ctx) {
                        return Some(StateChangeResult::with_forward_state_and_event(
                            ForwardState::Running,
                            Event::PlayerEvent(PlayerEvent::PassTo(
                                PassingEventContext::new()
                                    .with_from_player_id(ctx.player.id)
                                    .with_to_player_id(return_target.id)
                                    .with_reason("FWD_RUNNING_ONE_TWO_RETURN")
                                    .build(ctx),
                            )),
                        ));
                    }
                }
            }

            // HOLD-UP PLAY: When facing away from goal with midfielders arriving,
            // draw defenders and lay off to a supporting teammate.
            // Only when forward genuinely can't advance (opponents blocking ahead).
            if ctx.ball().has_stable_possession()
                && distance_to_goal > CLOSE_RANGE_DISTANCE
                && ctx.tick_context.ball.ownership_duration > 30
                && !self.has_open_space_ahead(ctx)  // Don't lay off if can run forward
            {
                if let Some(layoff_target) = self.find_hold_up_layoff(ctx) {
                    return Some(StateChangeResult::with_forward_state_and_event(
                        ForwardState::Running,
                        Event::PlayerEvent(PlayerEvent::PassTo(
                            PassingEventContext::new()
                                .with_from_player_id(ctx.player.id)
                                .with_to_player_id(layoff_target.id)
                                .with_reason("FWD_RUNNING_HOLD_UP_LAYOFF")
                                .build(ctx),
                        )),
                    ));
                }
            }

            // Clear ball if congested far from goal — only after carrying for a while
            if distance_to_goal > SHOOTING_ZONE_DISTANCE
                && ctx.tick_context.ball.ownership_duration > 15
            {
                if ctx.player().movement().is_congested_near_boundary() || ctx.player().movement().is_congested() {
                    if let Some(_) = ctx.players().teammates().all().next() {
                        return Some(StateChangeResult::with_forward_state(ForwardState::Passing));
                    }
                }
            }

            // DRAW AND RELEASE: When an opponent is committing to tackle, draw them in
            // then pass to space they vacated
            if ctx.ball().has_stable_possession()
                && ctx.tick_context.ball.ownership_duration > 20
                && distance_to_goal > CLOSE_RANGE_DISTANCE
            {
                if let Some(release_target) = self.find_draw_and_release_pass(ctx) {
                    return Some(StateChangeResult::with_forward_state_and_event(
                        ForwardState::Running,
                        Event::PlayerEvent(PlayerEvent::PassTo(
                            PassingEventContext::new()
                                .with_from_player_id(ctx.player.id)
                                .with_to_player_id(release_target.id)
                                .with_reason("FWD_RUNNING_DRAW_AND_RELEASE")
                                .build(ctx),
                        )),
                    ));
                }
            }

            // Under pressure - quick decision needed
            if ctx.player().pressure().is_under_immediate_pressure() {
                if self.should_pass_under_pressure(ctx) {
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
            if ownership_ticks > 12 && self.should_pass(ctx) {
                return Some(StateChangeResult::with_forward_state(ForwardState::Passing));
            }

            if ownership_ticks > 20 && self.should_dribble(ctx) {
                return Some(StateChangeResult::with_forward_state(
                    ForwardState::Dribbling,
                ));
            }

            // Cross from wide position in attacking third
            if self.should_cross(ctx) {
                return Some(StateChangeResult::with_forward_state(
                    ForwardState::Crossing,
                ));
            }

            // ANTI-OSCILLATION: If carrying ball too long without acting, force a decision
            // But allow extended runs when advancing toward goal with open space
            if ctx.in_state_time > 120 {
                let finishing = ctx.player.skills.technical.finishing / 20.0;
                if distance_to_goal < SHOOTING_ZONE_DISTANCE && finishing > 0.4 && ctx.player().has_clear_shot() {
                    return Some(StateChangeResult::with_forward_state(ForwardState::Shooting));
                }
                return Some(StateChangeResult::with_forward_state(ForwardState::Passing));
            }

            // Continue running with ball briefly while looking for an opening
            return None;
        }
        // Handle cases when player doesn't have the ball
        else {
            // Priority 0: Loose ball nearby — only chase if best positioned
            if ctx.ball().should_take_ball_immediately() && ctx.team().is_best_player_to_chase_ball() {
                return Some(StateChangeResult::with_forward_state(
                    ForwardState::TakeBall,
                ));
            }

            // Priority 0.5: Aerial ball approaching — head it
            if ctx.tick_context.positions.ball.position.z >= 1.5
                && ctx.ball().is_towards_player_with_angle(0.5)
                && ctx.ball().distance() < 40.0
                && ctx.ball().distance_to_opponent_goal() < 200.0
            {
                return Some(StateChangeResult::with_forward_state(
                    ForwardState::Heading,
                ));
            }

            // Priority 0.7: Cross incoming — position to receive
            // Detect crosses: ball in flight, coming from wide area, forward in or near the box
            if ctx.ball().is_towards_player_with_angle(0.6)
                && ctx.ball().distance() > 10.0
                && ctx.ball().distance() < 100.0
                && ctx.ball().distance_to_opponent_goal() < 150.0
                && ctx.tick_context.positions.ball.position.z >= 1.0
                && !ctx.ball().is_owned()
            {
                return Some(StateChangeResult::with_forward_state(
                    ForwardState::CrossReceiving,
                ));
            }

            // Priority 1: Ball interception opportunity
            if self.can_intercept_ball(ctx) {
                return Some(StateChangeResult::with_forward_state(
                    ForwardState::Intercepting,
                ));
            }

            // Priority 2: Pressing opportunity
            if self.should_press(ctx) {
                return Some(StateChangeResult::with_forward_state(
                    ForwardState::Pressing,
                ));
            }

            // Priority 3: Create space when team has possession
            if ctx.team().is_control_ball() {
                if self.should_create_space(ctx) {
                    return Some(StateChangeResult::with_forward_state(
                        ForwardState::CreatingSpace,
                    ));
                }

                // Make intelligent runs
                if self.should_make_run_in_behind(ctx) {
                    return Some(StateChangeResult::with_forward_state(
                        ForwardState::RunningInBehind,
                    ));
                }
            }

            // Priority 4: Defensive duties when needed
            if !ctx.team().is_control_ball() {
                if self.should_return_to_position(ctx) {
                    return Some(StateChangeResult::with_forward_state(
                        ForwardState::Returning,
                    ));
                }

                if self.should_help_defend(ctx) {
                    return Some(StateChangeResult::with_forward_state(
                        ForwardState::Pressing,
                    ));
                }
            }

            // Consider fatigue and state duration
            if self.needs_recovery(ctx) {
                return Some(StateChangeResult::with_forward_state(
                    ForwardState::Resting,
                ));
            }

            // Prevent getting stuck in running state
            if ctx.in_state_time > 300 {
                return if ctx.team().is_control_ball() {
                    Some(StateChangeResult::with_forward_state(
                        ForwardState::CreatingSpace,
                    ))
                } else {
                    Some(StateChangeResult::with_forward_state(
                        ForwardState::Walking,
                    ))
                };
            }
        }

        None
    }

    fn process_slow(&self, _ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        None
    }

    fn velocity(&self, ctx: &StateProcessingContext) -> Option<Vector3<f32>> {
        // Fatigue-aware velocity calculation
        let fatigue_factor = self.calculate_fatigue_factor(ctx);

        // If following waypoints (team tactical movement)
        if ctx.player.should_follow_waypoints(ctx) && !ctx.player.has_ball(ctx) {
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
                        * fatigue_factor
                        + ctx.player().separation_velocity(),
                );
            }
        }

        // Movement with ball
        if ctx.player.has_ball(ctx) {
            Some(self.calculate_ball_carrying_movement(ctx) * fatigue_factor)
        }
        // Without ball — spread across pitch using unique player slots
        else {
            let ball_pos = ctx.tick_context.positions.ball.position;
            let start_pos = ctx.player.start_position;
            let field_width = ctx.context.field_size.width as f32;
            let field_height = ctx.context.field_size.height as f32;
            let ball_distance = ctx.ball().distance();

            // ANTI-FOLLOWING: If very close to ball carrier, spread away
            // Use hysteresis: start spreading at 25, stop at 45 to prevent oscillation
            if ball_distance < 25.0 && ctx.team().is_control_ball() {
                let away_from_ball = (ctx.player.position - ball_pos).normalize();
                let lateral = Vector3::new(-away_from_ball.y, away_from_ball.x, 0.0);
                let spread_target = ctx.player.position + away_from_ball * 30.0 + lateral * 15.0;
                let clamped = Vector3::new(
                    spread_target.x.clamp(30.0, field_width - 30.0),
                    spread_target.y.clamp(40.0, field_height - 40.0),
                    0.0,
                );
                let direction = (clamped - ctx.player.position).normalize();
                let speed = ctx.player.skills.physical.pace * 0.5;
                return Some(direction * speed * fatigue_factor);
            }

            let attacking_direction = match ctx.player.side {
                Some(PlayerSide::Left) => 1.0,
                Some(PlayerSide::Right) => -1.0,
                None => 0.0,
            };

            // Smooth ball proximity (no binary switches)
            let proximity = (1.0 - ball_distance / 400.0).clamp(0.05, 0.45);

            // === UNIQUE FORWARD SLOT: spread forwards vertically ===
            let mut teammate_fwd_ids: Vec<u32> = ctx.players().teammates().all()
                .filter(|t| t.tactical_positions.is_forward())
                .map(|t| t.id)
                .collect();
            teammate_fwd_ids.push(ctx.player.id);
            teammate_fwd_ids.sort();
            let slot_index = teammate_fwd_ids.iter().position(|&id| id == ctx.player.id).unwrap_or(0);
            let total_fwds = teammate_fwd_ids.len().max(1);

            // Spread forwards across 25%-75% of field height
            let slot_y = field_height * 0.25
                + (field_height * 0.5) * (slot_index as f32 + 0.5) / total_fwds as f32;

            // Forwards stay HIGH — target pushes well ahead of ball toward opponent goal
            let depth_stagger = attacking_direction * (slot_index as f32 * 20.0);
            let advanced_x = ball_pos.x + attacking_direction * 70.0 + depth_stagger;
            // Clamp advanced_x toward opponent's half so forwards don't drop behind the ball
            let min_forward_x = match ctx.player.side {
                Some(PlayerSide::Left) => (field_width * 0.45).max(ball_pos.x).min(field_width),
                Some(PlayerSide::Right) => 0.0_f32,
                None => 0.0,
            };
            let max_forward_x = match ctx.player.side {
                Some(PlayerSide::Left) => field_width,
                Some(PlayerSide::Right) => (field_width * 0.55).min(ball_pos.x).max(0.0),
                None => field_width,
            };
            let clamped_advanced_x = advanced_x.clamp(min_forward_x, max_forward_x);
            let target_x = start_pos.x + (clamped_advanced_x - start_pos.x) * proximity;

            // Y: blend between assigned slot and start position
            let center_y = field_height / 2.0;
            let is_wide = (start_pos.y - center_y).abs() > field_height * 0.2;
            let slot_weight = if is_wide { 0.4 } else { 0.5 };
            let ball_weight = proximity * (1.0 - slot_weight);
            let start_weight = (1.0 - slot_weight - ball_weight).max(0.0);
            let target_y = slot_y * slot_weight
                + ball_pos.y * ball_weight
                + start_pos.y * start_weight;

            // Per-forward organic drift for unique movement (smaller — slot handles spread)
            // Dampen drift in attacking third to prevent shaking near goal
            let match_time = ctx.context.total_match_time as f32;
            let seed = ctx.player.id as f32 * 2.39;
            let in_attacking_third = ctx.player.position.x > field_width * 0.66
                || ctx.player.position.x < field_width * 0.33;
            let drift_scale = if in_attacking_third { 0.3 } else { 1.0 };
            let drift_x = (seed + match_time * 0.005).sin() * 8.0 * drift_scale;
            let drift_y = (seed * 1.37 + match_time * 0.004).cos() * 6.0 * drift_scale;

            let target = Vector3::new(
                (target_x + drift_x).clamp(30.0, field_width - 30.0),
                (target_y + drift_y).clamp(40.0, field_height - 40.0),
                0.0,
            );

            let dist_to_target = (target - ctx.player.position).magnitude();

            let arrive_velocity = SteeringBehavior::Arrive {
                target,
                slowing_distance: 12.0,
            }
            .calculate(ctx.player)
            .velocity;

            // Dampen separation near target to prevent oscillation
            let sep_damping = (dist_to_target / 40.0).clamp(0.0, 1.0);
            let separation = ctx.player().separation_velocity() * sep_damping;

            Some(
                arrive_velocity * fatigue_factor + separation,
            )
        }
    }

    fn process_conditions(&self, ctx: ConditionContext) {
        // Forwards do a lot of intense running - high intensity with velocity
        ForwardCondition::with_velocity(ActivityIntensity::High).process(ctx);
    }
}

impl ForwardRunningState {

    /// Check if there's open space ahead toward the opponent goal
    fn has_open_space_ahead(&self, ctx: &StateProcessingContext) -> bool {
        let player_pos = ctx.player.position;
        let goal_pos = ctx.player().opponent_goal_position();
        let to_goal = (goal_pos - player_pos).normalize();

        // Check for opponents blocking the path ahead (within 25 units, roughly toward goal)
        let blockers = ctx.players().opponents().nearby(25.0)
            .filter(|opp| {
                let to_opp = (opp.position - player_pos).normalize();
                to_opp.dot(&to_goal) > 0.4
            })
            .count();

        blockers == 0
    }

    /// Check if under immediate pressure
    #[allow(dead_code)]
    fn is_under_immediate_pressure(&self, ctx: &StateProcessingContext) -> bool {
        ctx.player().pressure().is_under_immediate_pressure()
    }

    /// Determine if should pass when under pressure
    fn should_pass_under_pressure(&self, ctx: &StateProcessingContext) -> bool {
        // Check for available passing options
        let safe_pass_available = ctx
            .players()
            .teammates()
            .nearby(200.0)
            .any(|t| ctx.player().has_clear_pass(t.id));

        let composure = ctx.player.skills.mental.composure / 20.0;

        // Low composure players pass more under pressure
        safe_pass_available
            && (composure < 0.7 || ctx.player().pressure().is_under_immediate_pressure())
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

    /// Enhanced interception check
    fn can_intercept_ball(&self, ctx: &StateProcessingContext) -> bool {
        // Don't try to intercept if ball is already owned by teammate
        if ctx.ball().is_owned() {
            if let Some(owner_id) = ctx.ball().owner_id() {
                if let Some(owner) = ctx.context.players.by_id(owner_id) {
                    if owner.team_id == ctx.player.team_id {
                        return false;
                    }
                }
            }
        }

        let ball_distance = ctx.ball().distance();
        let ball_speed = ctx.ball().speed();

        // Static or slow-moving ball nearby - only if nearest teammate
        if ball_distance < 30.0 && ball_speed < 2.0 {
            let ball_pos = ctx.tick_context.positions.ball.position;
            let closer_teammate = ctx.players().teammates().all()
                .any(|t| t.id != ctx.player.id && (t.position - ball_pos).magnitude() < ball_distance - 5.0);
            if !closer_teammate {
                return true;
            }
        }

        // Ball moving toward player
        if ball_distance < 150.0 && ctx.ball().is_towards_player_with_angle(0.8) {
            // Calculate if player can reach interception point
            let player_speed = ctx.player.skills.physical.pace / 20.0 * 10.0;
            let time_to_reach = ball_distance / player_speed;
            let ball_travel_distance = ball_speed * time_to_reach;

            return ball_travel_distance < ball_distance * 1.5;
        }

        false
    }

    /// Improved pressing decision with smart triggers
    fn should_press(&self, ctx: &StateProcessingContext) -> bool {
        // Don't press if team has possession
        if ctx.team().is_control_ball() {
            return false;
        }

        // Only the closest player should initiate the press — prevents swarming
        // Exception: very close range (<30) anyone can press reactively
        let ball_distance = ctx.ball().distance();
        if ball_distance > 30.0 && !ctx.team().is_best_player_to_chase_ball() {
            return false;
        }

        let stamina_level = ctx.player.player_attributes.condition_percentage() as f32 / 100.0;
        let work_rate = ctx.player.skills.mental.work_rate / 20.0;
        let intensity = ctx.team().tactics().pressing_intensity();

        // Adjust pressing distance based on stamina, work rate, and tactical intensity
        let effective_press_distance = 150.0 * stamina_level * (0.5 + work_rate) * (0.5 + intensity * 0.5);

        // Check tactical instruction (high press vs low block)
        let high_press = ctx.team().tactics().is_high_pressing();

        // PRESSING TRAP: Opponent defender receiving ball facing own goal — press aggressively
        if ball_distance < effective_press_distance * 1.5 {
            if let Some(opponent) = ctx.players().opponents().nearby(effective_press_distance * 1.5).with_ball(ctx).next() {
                let opp_velocity = ctx.tick_context.positions.players.velocity(opponent.id);
                let goal_pos = ctx.player().opponent_goal_position();
                let opp_goal = ctx.tick_context.positions.ball.position * 2.0 - goal_pos; // Approximate own goal

                // Opponent facing own goal (velocity pointing away from us)
                if opp_velocity.magnitude() > 0.5 {
                    let to_own_goal = (opp_goal - opponent.position).normalize();
                    if opp_velocity.normalize().dot(&to_own_goal) > 0.4 {
                        return true; // Opponent in trouble — press!
                    }
                }

                // WIDE ISOLATION: Opponent near touchline — trap them
                let field_height = ctx.context.field_size.height as f32;
                if opponent.position.y < field_height * 0.1 || opponent.position.y > field_height * 0.9 {
                    return true;
                }

                // TIRED OPPONENT: Increase pressing range against fatigued players
                if let Some(opp_player) = ctx.context.players.by_id(opponent.id) {
                    if opp_player.player_attributes.condition_percentage() < 50 {
                        return ball_distance < effective_press_distance * 1.4;
                    }
                }
            }
        }

        if high_press {
            ball_distance < effective_press_distance * 1.3
        } else {
            // Only press in attacking third
            ball_distance < effective_press_distance && !ctx.ball().on_own_third()
        }
    }

    /// Determine if should create space
    fn should_create_space(&self, ctx: &StateProcessingContext) -> bool {
        // Don't create space if you're the ball carrier or very close to ball
        let ball_distance = ctx.ball().distance();
        if ball_distance < 5.0 {
            return false;
        }

        // Check if another teammate has the ball - if so, we MUST create space
        if let Some(owner_id) = ctx.ball().owner_id() {
            if owner_id != ctx.player.id {
                // Teammate has ball - check if they're on our team
                if let Some(owner) = ctx.context.players.by_id(owner_id) {
                    if owner.team_id == ctx.player.team_id {
                        // Teammate has ball — create space only if far from ball
                        // Close forwards should stay ready for through-balls, not drift wide
                        let ball_distance = ctx.ball().distance();
                        return ball_distance > 60.0;
                    }
                }
            }
        }

        // No teammate has ball - still try to create space if we're not closest
        let closest_to_ball = !ctx.players().teammates().all().any(|t| {
            let t_dist = (t.position - ctx.tick_context.positions.ball.position).magnitude();
            t_dist < ball_distance * 0.9
        });

        !closest_to_ball
    }

    /// Detect counter-attack opportunity (team just won possession, opponents high)
    fn is_counter_attack_opportunity(&self, ctx: &StateProcessingContext) -> bool {
        let ownership_duration = ctx.tick_context.ball.ownership_duration;
        if ownership_duration >= 15 {
            return false;
        }

        // Count opponents ahead of ball
        let ball_pos = ctx.tick_context.positions.ball.position;
        let goal_pos = ctx.player().opponent_goal_position();
        let to_goal = (goal_pos - ball_pos).normalize();

        let opponents_ahead = ctx.players().opponents().all()
            .filter(|opp| {
                let to_opp = opp.position - ball_pos;
                to_opp.normalize().dot(&to_goal) > 0.3
            })
            .count();

        opponents_ahead < 3
    }

    /// Check if should make run in behind defense
    fn should_make_run_in_behind(&self, ctx: &StateProcessingContext) -> bool {
        // Don't make runs on own half
        if ctx.player().on_own_side() {
            return false;
        }

        // Check player attributes - relaxed requirements
        let pace = ctx.player.skills.physical.pace / 20.0;
        let off_ball = ctx.player.skills.mental.off_the_ball / 20.0;
        let stamina = ctx.player.player_attributes.condition_percentage() as f32 / 100.0;

        // Counter-attack: lower skill threshold — be more aggressive
        let is_counter = self.is_counter_attack_opportunity(ctx);
        let skill_threshold = if is_counter { 0.25 } else { 0.4 };

        // Combined skill check - if player is good at any of these, allow the run
        let skill_score = pace * 0.4 + off_ball * 0.4 + stamina * 0.2;
        if skill_score < skill_threshold {
            return false;
        }

        // Check if there's space behind defense
        let defensive_line = self.find_defensive_line(ctx);
        let space_behind = self.check_space_behind_defense(ctx, defensive_line);

        // Check if a teammate has the ball (any teammate, not just good passers)
        let teammate_has_ball = ctx
            .ball()
            .owner_id()
            .and_then(|id| ctx.context.players.by_id(id))
            .map(|p| p.team_id == ctx.player.team_id)
            .unwrap_or(false);

        // More aggressive: make runs even if space is limited, as long as teammate has ball
        // and we're in attacking third
        let in_attacking_third = self.is_in_good_attacking_position(ctx);

        // During counter-attacks, be much more willing to run
        if is_counter && teammate_has_ball {
            return true;
        }

        (space_behind || in_attacking_third) && teammate_has_ball
    }

    /// Find opponent defensive line position
    fn find_defensive_line(&self, ctx: &StateProcessingContext) -> f32 {
        let defenders: Vec<f32> = ctx
            .players()
            .opponents()
            .all()
            .filter(|p| p.tactical_positions.is_defender())
            .map(|p| match ctx.player.side {
                Some(PlayerSide::Left) => p.position.x,
                Some(PlayerSide::Right) => p.position.x,
                None => p.position.x,
            })
            .collect();

        if defenders.is_empty() {
            ctx.context.field_size.width as f32 / 2.0
        } else {
            // Return the position of the last defender
            match ctx.player.side {
                Some(PlayerSide::Left) => defenders.iter().fold(f32::MIN, |a, &b| a.max(b)),
                Some(PlayerSide::Right) => defenders.iter().fold(f32::MAX, |a, &b| a.min(b)),
                None => defenders.iter().sum::<f32>() / defenders.len() as f32,
            }
        }
    }

    /// Check if there's exploitable space behind defense
    fn check_space_behind_defense(
        &self,
        ctx: &StateProcessingContext,
        defensive_line: f32,
    ) -> bool {
        let player_x = ctx.player.position.x;

        match ctx.player.side {
            Some(PlayerSide::Left) => {
                // Space exists if defensive line is high and there's room behind
                defensive_line < ctx.context.field_size.width as f32 * 0.7
                    && player_x < defensive_line + 20.0
            }
            Some(PlayerSide::Right) => {
                defensive_line > ctx.context.field_size.width as f32 * 0.3
                    && player_x > defensive_line - 20.0
            }
            None => false,
        }
    }

    /// Determine if should return to defensive position
    fn should_return_to_position(&self, ctx: &StateProcessingContext) -> bool {
        ctx.player().position_to_distance() == PlayerDistanceFromStartPosition::Big
    }

    /// Check if forward should help defend
    fn should_help_defend(&self, ctx: &StateProcessingContext) -> bool {
        // Check game situation
        let losing_badly = ctx.team().is_loosing() && ctx.context.time.is_running_out();
        let work_rate = ctx.player.skills.mental.work_rate / 20.0;

        // High work rate forwards help more
        work_rate > 0.7 && losing_badly && ctx.ball().on_own_third()
    }

    /// Check if player needs recovery
    fn needs_recovery(&self, ctx: &StateProcessingContext) -> bool {
        let stamina = ctx.player.player_attributes.condition_percentage();
        let has_been_sprinting = ctx.in_state_time > SPRINT_DURATION_THRESHOLD;

        stamina < 60 && has_been_sprinting
    }

    /// Calculate fatigue factor for movement
    fn calculate_fatigue_factor(&self, ctx: &StateProcessingContext) -> f32 {
        let time_in_state = ctx.in_state_time as f32;

        // Only apply time-based fatigue here.
        // Condition is already factored in by steering behaviors via max_speed_with_condition().
        (1.0 - (time_in_state / 600.0)).max(0.7)
    }

    /// Calculate movement when carrying the ball
    fn calculate_ball_carrying_movement(&self, ctx: &StateProcessingContext) -> Vector3<f32> {
        // First, look for optimal path to goal
        if let Some(target_position) = self.find_optimal_attacking_path(ctx) {
            SteeringBehavior::Arrive {
                target: target_position,
                slowing_distance: 20.0,
            }
                .calculate(ctx.player)
                .velocity
                + ctx.player().separation_velocity()
        } else {
            // Default to moving toward goal
            SteeringBehavior::Arrive {
                target: ctx.player().opponent_goal_position(),
                slowing_distance: 100.0,
            }
                .calculate(ctx.player)
                .velocity
                + ctx.player().separation_velocity()
        }
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

        let opponents: Vec<MatchPlayerLite> = ctx
            .players()
            .opponents()
            .nearby(100.0)
            .filter(|opp| {
                // Only consider opponents between player and goal
                let to_goal = goal_pos - player_pos;
                let to_opp = opp.position - player_pos;
                to_goal.normalize().dot(&to_opp.normalize()) > 0.5
            })
            .collect();

        if opponents.len() < 2 {
            return None;
        }

        // Find largest gap
        let mut best_gap = None;
        let mut best_gap_size = 0.0;

        for i in 0..opponents.len() {
            for j in i + 1..opponents.len() {
                let gap_center = (opponents[i].position + opponents[j].position) * 0.5;
                let gap_size = (opponents[i].position - opponents[j].position).magnitude();

                if gap_size > best_gap_size && gap_size > 20.0 {
                    best_gap_size = gap_size;
                    best_gap = Some(gap_center);
                }
            }
        }

        best_gap
    }

    /// Calculate supporting movement when team has ball

    fn should_pass(&self, ctx: &StateProcessingContext) -> bool {
        let teammates: Vec<MatchPlayerLite> = ctx.players().teammates().nearby(300.0).collect();

        if teammates.is_empty() {
            return false;
        }

        // Core skills affecting passing decisions
        let vision = ctx.player.skills.mental.vision / 20.0;
        let passing = ctx.player.skills.technical.passing / 20.0;
        let decisions = ctx.player.skills.mental.decisions / 20.0;
        let teamwork = ctx.player.skills.mental.teamwork / 20.0;

        // Situational factors — use common pressure check
        let under_pressure = ctx.player().pressure().is_under_immediate_pressure();
        let distance_to_goal = ctx.ball().distance_to_opponent_goal();
        let stamina = ctx.player.player_attributes.condition_percentage() as f32 / 100.0;

        // 1. MUST PASS: Heavy pressure or exhaustion
        if under_pressure && (passing > 0.5 || stamina < 0.4) {
            return self.has_safe_passing_option(ctx, &teammates);
        }

        // 2. PREFER TO RUN/SHOOT: Very close to goal - only pass if teammate is much better positioned
        if distance_to_goal < CLOSE_RANGE_DISTANCE && !under_pressure {
            return self.has_better_positioned_teammate(ctx, &teammates, distance_to_goal);
        }

        if distance_to_goal < SHOOTING_ZONE_DISTANCE && !under_pressure {
            // Enhanced shooting zone - only forward passes to significantly better teammates
            return self.has_forward_pass_to_better_teammate(ctx, &teammates, distance_to_goal);
        }

        // 3. LOOK FOR QUALITY OPPORTUNITIES: Good vision/passing players find better passes
        if vision > 0.7 || passing > 0.7 {
            // Check for teammates in free zones or making runs
            if self.has_teammate_in_dangerous_position(ctx, &teammates, distance_to_goal) {
                return true;
            }
        }

        // 4. TEAM PLAY: High teamwork players share more
        if teamwork > 0.7 && decisions > 0.6 {
            return self.has_good_passing_option(ctx, &teammates);
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
    /// Used in try_fast() to distribute goals across team
    fn has_teammate_with_much_better_shot(
        &self,
        ctx: &StateProcessingContext,
        own_distance: f32,
    ) -> bool {
        // Don't pass at point-blank range
        if own_distance < POINT_BLANK_DISTANCE {
            return false;
        }

        let vision = ctx.player.skills.mental.vision / 20.0;
        let teamwork = ctx.player.skills.mental.teamwork / 20.0;

        // Selfish players with low vision/teamwork don't look for teammates
        if vision < 0.4 && teamwork < 0.4 {
            return false;
        }

        ctx.players()
            .teammates()
            .nearby(200.0)
            .any(|teammate| {
                let teammate_distance =
                    (teammate.position - ctx.player().opponent_goal_position()).magnitude();
                // Teammate must be significantly closer (at least 40% closer)
                let is_much_closer = teammate_distance < own_distance * 0.6;
                let has_clear_pass = ctx.player().has_clear_pass(teammate.id);
                let not_heavily_marked = ctx.tick_context.distances
                    .opponents(teammate.id, 8.0).count() < 2;

                is_much_closer && has_clear_pass && not_heavily_marked
            })
    }

    /// Check if any teammate is in a significantly better scoring position
    fn has_better_positioned_teammate(
        &self,
        ctx: &StateProcessingContext,
        teammates: &[MatchPlayerLite],
        current_distance: f32,
    ) -> bool {
        teammates.iter().any(|teammate| {
            let teammate_distance =
                (teammate.position - ctx.player().opponent_goal_position()).magnitude();
            let is_much_closer = teammate_distance < current_distance * 0.6;
            let not_heavily_marked = !self.is_teammate_heavily_marked(ctx, teammate);
            let has_clear_lane = ctx.player().has_clear_pass(teammate.id);

            is_much_closer && not_heavily_marked && has_clear_lane
        })
    }

    /// Check for forward passes to better positioned teammates (prevents backward passes near goal)
    fn has_forward_pass_to_better_teammate(
        &self,
        ctx: &StateProcessingContext,
        teammates: &[MatchPlayerLite],
        current_distance: f32,
    ) -> bool {
        let player_pos = ctx.player.position;

        teammates.iter().any(|teammate| {
            // Must be a forward pass direction
            let is_forward_pass = match ctx.player.side {
                Some(PlayerSide::Left) => teammate.position.x > player_pos.x,
                Some(PlayerSide::Right) => teammate.position.x < player_pos.x,
                None => false,
            };

            if !is_forward_pass {
                return false; // Reject backward passes
            }

            // Teammate must be much closer to goal
            let teammate_distance =
                (teammate.position - ctx.player().opponent_goal_position()).magnitude();
            let is_much_closer = teammate_distance < current_distance * TEAMMATE_ADVANTAGE_STRICT_RATIO;
            let not_heavily_marked = !self.is_teammate_heavily_marked(ctx, teammate);
            let has_clear_lane = ctx.player().has_clear_pass(teammate.id);

            is_much_closer && not_heavily_marked && has_clear_lane
        })
    }

    /// Check for teammates in dangerous attacking positions (free zones or making runs)
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
            let in_free_space = ctx.tick_context.distances
                .opponents(teammate.id, 12.0).count() < 2;

            // Check if teammate is making a forward run
            let teammate_velocity = ctx.tick_context.positions.players.velocity(teammate.id);
            let making_run = teammate_velocity.magnitude() > 2.0 && {
                let to_goal = ctx.player().opponent_goal_position() - teammate.position;
                teammate_velocity.normalize().dot(&to_goal.normalize()) > 0.5
            };

            let has_clear_pass = ctx.player().has_clear_pass(teammate.id);

            has_clear_pass && in_attacking_position && (in_free_space || making_run)
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
            let has_space = ctx.tick_context.distances
                .opponents(teammate.id, 10.0).count() < 2;

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
        // Single scan at max distance, bucket by distance
        let mut markers = 0;
        let mut very_close = 0;
        for (_id, dist) in ctx.tick_context.distances.opponents(ctx.player.id, 8.0) {
            markers += 1;
            if dist <= 3.0 {
                very_close += 1;
            }
        }

        markers >= 2 || (markers >= 1 && very_close > 0)
    }

    fn should_cross(&self, ctx: &StateProcessingContext) -> bool {
        let field_height = ctx.context.field_size.height as f32;
        let y = ctx.player.position.y;
        let wide_margin = field_height * 0.2;

        // Must be in a wide channel
        let is_wide = y < wide_margin || y > field_height - wide_margin;
        if !is_wide {
            return false;
        }

        // Must be in attacking third
        if !self.is_in_good_attacking_position(ctx) {
            return false;
        }

        // Must have teammates in the box to cross to
        let goal_pos = ctx.player().opponent_goal_position();
        let teammates_in_box = ctx
            .players()
            .teammates()
            .all()
            .filter(|t| (t.position - goal_pos).magnitude() < 120.0)
            .count();

        let crossing = ctx.player.skills.technical.crossing / 20.0;

        teammates_in_box >= 1 && crossing > 0.4
    }

    fn should_dribble(&self, ctx: &StateProcessingContext) -> bool {
        let dribbling_skill = ctx.player.skills.technical.dribbling / 20.0;
        let pace = ctx.player.skills.physical.pace / 20.0;

        // Check for opponents directly ahead (not just any nearby)
        let goal_pos = ctx.player().opponent_goal_position();
        let player_pos = ctx.player.position;
        let to_goal = (goal_pos - player_pos).normalize();

        let opponents_blocking = ctx.players().opponents().nearby(25.0)
            .filter(|opp| {
                let to_opp = (opp.position - player_pos).normalize();
                to_opp.dot(&to_goal) > 0.5 && (opp.position - player_pos).magnitude() < 20.0
            })
            .count();

        // No opponents blocking — just keep running, don't dribble
        if opponents_blocking == 0 {
            return false;
        }

        // Skilled dribblers take on opponents
        if dribbling_skill > 0.7 && pace > 0.6 {
            opponents_blocking <= 2
        } else if dribbling_skill > 0.5 {
            opponents_blocking <= 1
        } else {
            false
        }
    }

    fn is_in_good_attacking_position(&self, ctx: &StateProcessingContext) -> bool {
        // Check if player is well-positioned in attacking third
        let field_width = ctx.context.field_size.width as f32;
        let attacking_third_start = match ctx.player.side {
            Some(PlayerSide::Left) => field_width * 0.65,
            Some(PlayerSide::Right) => field_width * 0.35,
            None => field_width * 0.5,
        };

        match ctx.player.side {
            Some(PlayerSide::Left) => ctx.player.position.x > attacking_third_start,
            Some(PlayerSide::Right) => ctx.player.position.x < attacking_third_start,
            None => false,
        }
    }

    // Calculate tactical run position for better support when team has possession
    #[allow(dead_code)]
    fn calculate_tactical_run_position(&self, ctx: &StateProcessingContext) -> Vector3<f32> {
        let player_position = ctx.player.position;
        let field_width = ctx.context.field_size.width as f32;
        let field_height = ctx.context.field_size.height as f32;

        // Find teammate with the ball
        let ball_holder = ctx
            .players()
            .teammates()
            .all()
            .find(|t| ctx.ball().owner_id() == Some(t.id));

        if let Some(holder) = ball_holder {
            // Calculate position based on ball holder's position
            let holder_position = holder.position;

            // Make runs beyond the ball holder
            let forward_position = match ctx.player.side {
                Some(PlayerSide::Left) => Vector3::new(
                    holder_position.x + 80.0,
                    // Vary Y-position based on player's current position
                    if player_position.y < field_height / 2.0 {
                        holder_position.y - 40.0 // Make run to left side
                    } else {
                        holder_position.y + 40.0 // Make run to right side
                    },
                    0.0,
                ),
                Some(PlayerSide::Right) => Vector3::new(
                    holder_position.x - 80.0,
                    if player_position.y < field_height / 2.0 {
                        holder_position.y - 40.0 // Make run to left side
                    } else {
                        holder_position.y + 40.0 // Make run to right side
                    },
                    0.0,
                ),
                None => Vector3::new(holder_position.x, holder_position.y + 30.0, 0.0),
            };

            // Ensure position is within field boundaries
            return Vector3::new(
                forward_position.x.clamp(20.0, field_width - 20.0),
                forward_position.y.clamp(20.0, field_height - 20.0),
                0.0,
            );
        }

        // Default to moving toward opponent's goal if no teammate has the ball
        let goal_direction = (ctx.player().opponent_goal_position() - player_position).normalize();
        player_position + goal_direction * 50.0
    }

    // Calculate defensive position when team doesn't have possession
    #[allow(dead_code)]
    fn calculate_defensive_position(&self, ctx: &StateProcessingContext) -> Vector3<f32> {
        let field_width = ctx.context.field_size.width as f32;

        // Forwards generally stay higher up the pitch
        let forward_line = match ctx.player.side {
            Some(PlayerSide::Left) => field_width * 0.6,
            Some(PlayerSide::Right) => field_width * 0.4,
            None => field_width * 0.5,
        };

        // Use player's start position Y-coordinate for width positioning
        let target_y = ctx.player.start_position.y;

        Vector3::new(forward_line, target_y, 0.0)
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

        let passer_lite = ctx.players().teammates().all()
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
        let opponents_near_passer = ctx.tick_context.distances
            .opponents(passer_id, 50.0).count();
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
        let opponents_ahead = ctx.players().opponents().nearby(25.0)
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

        ctx.players().teammates().nearby(150.0)
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
                let in_space = ctx.tick_context.distances
                    .opponents(t.id, 10.0).count() < 2;
                // Prefer midfielders who can carry forward
                let is_midfielder_or_attacker = t.tactical_positions.is_midfielder()
                    || t.tactical_positions.is_forward();
                // Clear passing lane
                let clear_pass = ctx.player().has_clear_pass(t.id);
                // Reject recent passers to prevent cycling
                let not_recent = ctx.ball().passer_recency_penalty(t.id) > 0.3;

                is_behind && in_space && is_midfielder_or_attacker && clear_pass && not_recent
            })
            .max_by(|a, b| {
                // Prefer farther teammates — they're more likely outside the congested group
                let da = (a.position - player_pos).magnitude();
                let db = (b.position - player_pos).magnitude();
                da.partial_cmp(&db).unwrap_or(std::cmp::Ordering::Equal)
            })
    }

    /// DRAW AND RELEASE: Detect an opponent committing to a tackle and find
    /// a teammate in the space they're vacating
    fn find_draw_and_release_pass<'a>(&self, ctx: &StateProcessingContext<'a>) -> Option<MatchPlayerLite> {
        let player_pos = ctx.player.position;

        // Find closest approaching opponent (within 15-35 units, closing in)
        let approaching_opponent = ctx.players().opponents().nearby(35.0)
            .filter(|opp| {
                let dist = (opp.position - player_pos).magnitude();
                if dist < 15.0 || dist > 35.0 { return false; }

                let opp_velocity = ctx.tick_context.positions.players.velocity(opp.id);
                if opp_velocity.magnitude() < 1.0 { return false; }

                let to_us = (player_pos - opp.position).normalize();
                let opp_dir = opp_velocity.normalize();
                opp_dir.dot(&to_us) > 0.6
            })
            .min_by(|a, b| {
                let da = (a.position - player_pos).magnitude();
                let db = (b.position - player_pos).magnitude();
                da.partial_cmp(&db).unwrap_or(std::cmp::Ordering::Equal)
            })?;

        // Space the opponent is vacating
        let opp_velocity = ctx.tick_context.positions.players.velocity(approaching_opponent.id);
        let vacated_zone = approaching_opponent.position - opp_velocity.normalize() * 30.0;

        // Find teammate near vacated space (at least 25 units away to avoid short group passes)
        ctx.players().teammates().nearby(200.0)
            .filter(|t| {
                let t_dist = (t.position - player_pos).magnitude();
                let t_dist_to_vacated = (t.position - vacated_zone).magnitude();
                t_dist > 25.0
                    && t_dist_to_vacated < 60.0
                    && ctx.player().has_clear_pass(t.id)
                    && ctx.ball().passer_recency_penalty(t.id) > 0.3
                    && ctx.tick_context.distances
                        .opponents(t.id, 10.0).count() < 2
            })
            .min_by(|a, b| {
                let da = (a.position - vacated_zone).magnitude();
                let db = (b.position - vacated_zone).magnitude();
                da.partial_cmp(&db).unwrap_or(std::cmp::Ordering::Equal)
            })
    }

    /// Check if player is stuck in a corner/boundary with multiple players around
    #[allow(dead_code)]
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
        let total_nearby = ctx.tick_context.distances
            .teammates(player_id, 0.0, 15.0).count()
            + ctx.tick_context.distances
            .opponents(player_id, 15.0).count();

        // If 3 or more players nearby (congestion), need to clear
        total_nearby >= 3
    }
}
