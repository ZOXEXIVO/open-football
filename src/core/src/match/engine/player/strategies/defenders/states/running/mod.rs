use crate::r#match::defenders::states::DefenderState;
use crate::r#match::defenders::states::common::{DefenderCondition, ActivityIntensity};
use crate::r#match::events::Event;
use crate::r#match::player::events::{PassingEventContext, PlayerEvent};
use crate::r#match::{
    ConditionContext, MatchPlayerLite, PlayerDistanceFromStartPosition, PlayerSide,
    StateChangeResult, StateProcessingContext, StateProcessingHandler, SteeringBehavior,
};
use crate::IntegerUtils;
use nalgebra::Vector3;

const MAX_SHOOTING_DISTANCE: f32 = 50.0; // Defenders almost never shoot, only from very close

#[derive(Default, Clone)]
pub struct DefenderRunningState {}

impl StateProcessingHandler for DefenderRunningState {
    fn try_fast(&self, ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        if ctx.player.has_ball(ctx) {
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

            if self.should_pass(ctx) {
                return Some(StateChangeResult::with_defender_state(
                    DefenderState::Passing,
                ));
            }
        } else {
            if ctx.player().position_to_distance() == PlayerDistanceFromStartPosition::Big {
                return Some(StateChangeResult::with_defender_state(
                    DefenderState::Returning,
                ));
            }

            // Only tackle if an opponent has the ball
            if let Some(_opponent) = ctx.players().opponents().with_ball().next() {
                if ctx.ball().distance() < 200.0 {
                    return Some(StateChangeResult::with_defender_state(
                        DefenderState::Tackling,
                    ));
                }
            }

            // Loose ball nearby — go claim it, but only if nearest teammate
            if !ctx.ball().is_owned() && ctx.ball().distance() < 50.0 && ctx.ball().speed() < 3.0 {
                let ball_pos = ctx.tick_context.positions.ball.position;
                let my_dist = ctx.ball().distance();
                let closer_teammate = ctx.players().teammates().all()
                    .any(|t| t.id != ctx.player.id && (t.position - ball_pos).magnitude() < my_dist - 5.0);

                if !closer_teammate {
                    return Some(StateChangeResult::with_defender_state(
                        DefenderState::TakeBall,
                    ));
                }
            }

            // Notification system: if ball system notified us to take the ball, act immediately
            if ctx.ball().should_take_ball_immediately() {
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

    fn process_slow(&self, _ctx: &StateProcessingContext) -> Option<StateChangeResult> {
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
            // With ball: move toward opponent goal
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
            // Without ball: return to tactical position
            Some(
                SteeringBehavior::Arrive {
                    target: ctx.player.start_position,
                    slowing_distance: 30.0,
                }
                .calculate(ctx.player)
                .velocity
                    + ctx.player().separation_velocity(),
            )
        }
    }

    fn process_conditions(&self, ctx: ConditionContext) {
        // Running is physically demanding - reduce condition based on intensity and player's stamina
        // Use velocity-based calculation to account for sprinting vs jogging
        DefenderCondition::with_velocity(ActivityIntensity::High).process(ctx);
    }
}

impl DefenderRunningState {
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

        // Check for immediate pressure: opponent very close = must pass/clear
        if ctx.players().opponents().exists(10.0) {
            return false;
        }

        // No opponent within moderate range — safe to carry
        let has_space_ahead = !ctx.players().opponents().exists(25.0);

        // Dribbling skill threshold — even average defenders can carry when safe
        let dribbling = ctx.player.skills.technical.dribbling / 20.0;
        let composure = ctx.player.skills.mental.composure / 20.0;
        let carry_ability = dribbling * 0.6 + composure * 0.4;

        if has_space_ahead && carry_ability > 0.3 {
            return true;
        }

        // Also carry if opponents are only at medium distance and player has decent skills
        let opponents_medium = ctx.players().opponents().exists(20.0);
        if !opponents_medium && carry_ability > 0.2 {
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

        // Count all nearby players (teammates + opponents) within 15 units
        let nearby_teammates = ctx.players().teammates().nearby(15.0).count();
        let nearby_opponents = ctx.players().opponents().nearby(15.0).count();
        let total_nearby = nearby_teammates + nearby_opponents;

        // If 3 or more players nearby (congestion), need to clear
        total_nearby >= 3
    }

    pub fn should_pass(&self, ctx: &StateProcessingContext) -> bool {
        // ANTI-LOOP: Don't pass immediately after entering Running state.
        // This prevents the Standing→Running→Passing→Standing loop where
        // a weak pass gets reclaimed and the cycle repeats endlessly.
        if ctx.in_state_time < 10 {
            return false;
        }

        // Also check ownership duration — if we just reclaimed the ball after a pass,
        // hold it for a moment before trying again
        let ownership_duration = ctx.tick_context.ball.ownership_duration;
        if ownership_duration < 5 {
            return false;
        }

        // Only pass under genuine pressure — opponent closing in
        if ctx.players().opponents().exists(12.0) {
            return true;
        }

        // If teammates are tired, carry the ball instead of passing to let them rest
        // Only pass if under actual pressure
        if self.are_teammates_tired(ctx) && ctx.players().opponents().exists(15.0) {
            return true;
        }

        // BUILD FROM BACK: If no opponents within 30 units and team controls ball,
        // look for progressive pass (advance play toward opponent goal)
        if !ctx.players().opponents().exists(30.0) && ctx.team().is_control_ball() {
            let player_pos = ctx.player.position;
            let goal_pos = ctx.player().opponent_goal_position();
            let to_goal = (goal_pos - player_pos).normalize();

            // Find a teammate ahead who is in space
            let has_progressive_target = ctx.players().teammates().nearby(200.0)
                .any(|t| {
                    let to_t = (t.position - player_pos).normalize();
                    let is_ahead = to_t.dot(&to_goal) > 0.2;
                    let in_space = ctx.players().opponents().all()
                        .filter(|opp| (opp.position - t.position).magnitude() < 15.0)
                        .count() < 2;
                    let has_lane = ctx.player().has_clear_pass(t.id);
                    is_ahead && in_space && has_lane
                });
            if has_progressive_target {
                return true;
            }
        }

        let game_vision_skill = ctx.player.skills.mental.vision;
        let game_vision_threshold = 14.0;

        if game_vision_skill >= game_vision_threshold {
            if let Some(_) = self.find_open_teammate_on_opposite_side(ctx) {
                return true;
            }
        }

        false
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

        let mut open_teammates: Vec<MatchPlayerLite> = ctx
            .players()
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
                let is_open = !ctx
                    .players()
                    .opponents()
                    .all()
                    .any(|opponent| (opponent.position - teammate.position).magnitude() < 20.0);
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

    fn is_in_shooting_range(&self, ctx: &StateProcessingContext) -> bool {
        let distance_to_goal = ctx.ball().distance_to_opponent_goal();

        distance_to_goal <= MAX_SHOOTING_DISTANCE && ctx.player().has_clear_shot()
    }

    /// Find a safe build-up pass target within max_distance.
    /// Prefers midfielders in space, same-side or central players, with clear pass lanes.
    fn find_safe_buildup_pass(&self, ctx: &StateProcessingContext, max_distance: f32) -> Option<MatchPlayerLite> {
        let player_pos = ctx.player.position;
        let goal_pos = ctx.player().opponent_goal_position();
        let to_goal = (goal_pos - player_pos).normalize();

        let mut best_target: Option<(MatchPlayerLite, f32)> = None;

        for teammate in ctx.players().teammates().nearby(max_distance) {
            // Skip goalkeeper
            if teammate.tactical_positions.is_goalkeeper() {
                continue;
            }

            let pass_dist = (teammate.position - player_pos).magnitude();
            if pass_dist < 30.0 {
                continue; // Too close — weak passes to nearby players create claim-pass loops
            }

            // Check pass lane is clear
            if !ctx.player().has_clear_pass(teammate.id) {
                continue;
            }

            // Check teammate is in space (no opponent within 15 units)
            let opponents_near = ctx.players().opponents().all()
                .filter(|opp| (opp.position - teammate.position).magnitude() < 15.0)
                .count();
            if opponents_near >= 2 {
                continue;
            }

            // Score the pass option
            let mut score: f32 = 0.0;

            // Prefer midfielders
            if teammate.tactical_positions.is_midfielder() {
                score += 50.0;
            }

            // Prefer passes that advance play (toward opponent goal)
            let to_teammate = (teammate.position - player_pos).normalize();
            let forward_component = to_teammate.dot(&to_goal);
            score += forward_component * 30.0;

            // Prefer teammates in space
            if opponents_near == 0 {
                score += 30.0;
            }

            // Prefer shorter passes (safer)
            score += (max_distance - pass_dist) / max_distance * 20.0;

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

    /// Check if this defender should make an overlapping run.
    /// True when: wide defender, teammate has ball on same flank, space ahead,
    /// and not the last defender.
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
            return false; // Already ahead of ball carrier
        }

        // Must not be the last defender (at least one CB between defender and own goal)
        let own_goal = ctx.ball().direction_to_own_goal();
        let own_dist = (ctx.player.position - own_goal).magnitude();
        let defenders_behind = ctx.players().teammates().defenders()
            .filter(|d| {
                let d_dist = (d.position - own_goal).magnitude();
                d_dist < own_dist && d.id != ctx.player.id
            })
            .count();
        if defenders_behind < 1 {
            return false;
        }

        // Check space ahead on the wing
        let wing_y = if player_on_left_flank { field_height * 0.1 } else { field_height * 0.9 };
        let ahead_pos = Vector3::new(ball_pos.x + to_goal.x * 50.0, wing_y, 0.0);
        let opponents_blocking = ctx.players().opponents().all()
            .filter(|opp| (opp.position - ahead_pos).magnitude() < 30.0)
            .count();

        opponents_blocking == 0
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
