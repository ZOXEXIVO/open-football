use crate::r#match::events::Event;
use crate::r#match::midfielders::states::common::{ActivityIntensity, MidfielderCondition};
use crate::r#match::midfielders::states::MidfielderState;
use crate::r#match::player::events::{PassingEventContext, PlayerEvent};
use crate::r#match::{ConditionContext, MatchPlayerLite, PassEvaluator, PlayerSide, StateChangeResult, StateProcessingContext, StateProcessingHandler, SteeringBehavior};
use nalgebra::Vector3;

#[derive(Default, Clone)]
pub struct MidfielderPassingState {}

impl StateProcessingHandler for MidfielderPassingState {
    fn try_fast(&self, ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        // Check if the midfielder still has the ball
        if !ctx.player.has_ball(ctx) {
            // Lost possession, transition to Running
            return Some(StateChangeResult::with_midfielder_state(
                MidfielderState::Running,
            ));
        }

        // Check if should shoot instead
        if self.should_shoot_instead_of_pass(ctx) {
            return Some(StateChangeResult::with_midfielder_state(
                MidfielderState::Shooting,
            ));
        }

        // Brief scanning delay before executing pass (unless under pressure)
        let under_pressure = self.is_under_heavy_pressure(ctx);
        let min_scan_time: u64 = if under_pressure { 3 } else { 8 };

        if ctx.in_state_time >= min_scan_time {
            if !ctx.ball().on_own_side() {
                // First, look for high-value breakthrough passes (for skilled players)
                if let Some(breakthrough_target) = self.find_breakthrough_pass_option(ctx) {
                    // Execute the high-quality breakthrough pass
                    return Some(StateChangeResult::with_midfielder_state_and_event(
                        MidfielderState::Standing,
                        Event::PlayerEvent(PlayerEvent::PassTo(
                            PassingEventContext::new()
                                .with_from_player_id(ctx.player.id)
                                .with_to_player_id(breakthrough_target.id)
                                .with_reason("MID_PASSING_BREAKTHROUGH")
                                .build(ctx),
                        )),
                    ));
                }
            }

            // Find the best regular pass option with improved logic
            if let Some((target_teammate, _reason)) = self.find_best_pass_option(ctx) {
                return Some(StateChangeResult::with_midfielder_state_and_event(
                    MidfielderState::Running,
                    Event::PlayerEvent(PlayerEvent::PassTo(
                        PassingEventContext::new()
                            .with_from_player_id(ctx.player.id)
                            .with_to_player_id(target_teammate.id)
                            .with_reason("MID_PASSING_STATE")
                            .build(ctx),
                    )),
                ));
            }
        }

        // If no good passing option after waiting, try something else
        // Under heavy pressure, bail out faster to dribble away
        let bail_time = if self.is_under_heavy_pressure(ctx) { 15 } else { 30 };
        if ctx.in_state_time > bail_time {
            let goal_dist = ctx.ball().distance_to_opponent_goal();
            return if goal_dist < 120.0 {
                // Close to goal — shoot rather than cycling to dribbling
                Some(StateChangeResult::with_midfielder_state(
                    MidfielderState::Shooting,
                ))
            } else if goal_dist < 200.0 {
                Some(StateChangeResult::with_midfielder_state(
                    MidfielderState::DistanceShooting,
                ))
            } else {
                // Far from goal — dribble forward
                Some(StateChangeResult::with_midfielder_state(
                    MidfielderState::Dribbling,
                ))
            };
        }

        None
    }

    fn process_slow(&self, _ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        None
    }

    fn velocity(&self, ctx: &StateProcessingContext) -> Option<Vector3<f32>> {
        // If under heavy pressure, shield the ball and create space
        if self.is_under_heavy_pressure(ctx) {
            // Move away from nearest opponent to create passing space
            if let Some(nearest_opponent) = ctx.players().opponents().nearby(15.0).next() {
                let away_from_opponent = (ctx.player.position - nearest_opponent.position).normalize();
                // Shield ball by moving perpendicular to goal direction
                let to_goal = (ctx.player().opponent_goal_position() - ctx.player.position).normalize();
                let perpendicular = Vector3::new(-to_goal.y, to_goal.x, 0.0);
                let escape_direction = (away_from_opponent * 0.7 + perpendicular * 0.3).normalize();
                return Some(escape_direction * 2.5 + ctx.player().separation_velocity());
            }
        }

        // Adjust position to find better passing angles if needed
        if self.should_adjust_position(ctx) {
            if let Some(nearest_teammate) = ctx.players().teammates().nearby_to_opponent_goal() {
                return Some(
                    SteeringBehavior::Arrive {
                        target: self.calculate_better_passing_position(ctx, &nearest_teammate),
                        slowing_distance: 30.0,
                    }
                        .calculate(ctx.player)
                        .velocity + ctx.player().separation_velocity(),
                );
            }
        }

        // Default: slow, controlled movement with ball - like scanning for options
        // Use separation to avoid colliding with other players
        Some(ctx.player().separation_velocity() * 0.5)
    }

    fn process_conditions(&self, ctx: ConditionContext) {
        // Passing is low intensity - minimal fatigue
        MidfielderCondition::new(ActivityIntensity::Low).process(ctx);
    }
}

impl MidfielderPassingState {
    /// Find breakthrough pass opportunities for players with high vision
    fn find_breakthrough_pass_option<'a>(
        &self,
        ctx: &StateProcessingContext<'a>,
    ) -> Option<MatchPlayerLite> {
        let vision_skill = ctx.player.skills.mental.vision;
        let passing_skill = ctx.player.skills.technical.passing;

        // Lowered thresholds from 15.0/14.0 to allow more players to attempt through balls
        if vision_skill < 12.0 || passing_skill < 11.0 {
            return None;
        }

        let vision_range = vision_skill * 20.0;
        let teammates = ctx.players().teammates();

        let breakthrough_targets = teammates.all()
            .filter(|teammate| {
                let velocity = ctx.tick_context.positions.players.velocity(teammate.id);
                let is_moving_forward = velocity.magnitude() > 1.0;
                let is_attacking_player = teammate.tactical_positions.is_forward() ||
                    teammate.tactical_positions.is_midfielder();
                let distance = (teammate.position - ctx.player.position).magnitude();
                let would_break_lines = self.would_pass_break_defensive_lines(ctx, teammate);

                distance < vision_range && is_moving_forward &&
                    is_attacking_player && would_break_lines
            })
            .collect::<Vec<_>>();

        breakthrough_targets.into_iter()
            .max_by(|a, b| {
                let a_value = self.calculate_breakthrough_value(ctx, a);
                let b_value = self.calculate_breakthrough_value(ctx, b);
                a_value.partial_cmp(&b_value).unwrap_or(std::cmp::Ordering::Equal)
            })
    }

    /// Improved best pass option finder that prevents clustering
    fn find_best_pass_option<'a>(
        &self,
        ctx: &StateProcessingContext<'a>,
    ) -> Option<(MatchPlayerLite, &'static str)> {
        PassEvaluator::find_best_pass_option(ctx, 400.0)
    }

    /// Improved space calculation around player
    fn calculate_improved_space_score(
        &self,
        ctx: &StateProcessingContext,
        teammate: &MatchPlayerLite,
    ) -> f32 {
        // Single scan at max distance, bucket by distance zones
        let mut very_close_opponents = 0;
        let mut close_opponents = 0;
        let mut medium_opponents = 0;
        for (_id, dist) in ctx.tick_context.distances.opponents(teammate.id, 15.0) {
            if dist < 5.0 {
                very_close_opponents += 1;
            } else if dist < 10.0 {
                close_opponents += 1;
            } else {
                medium_opponents += 1;
            }
        }

        // Calculate weighted score
        let space_score: f32 = 1.0
            - (very_close_opponents as f32 * 0.5)
            - (close_opponents as f32 * 0.3)
            - (medium_opponents as f32 * 0.1);

        space_score.max(0.0)
    }

    /// Check if pass would break defensive lines
    fn would_pass_break_defensive_lines(
        &self,
        ctx: &StateProcessingContext,
        teammate: &MatchPlayerLite,
    ) -> bool {
        let player_pos = ctx.player.position;
        let teammate_pos = teammate.position;
        let pass_direction = (teammate_pos - player_pos).normalize();
        let pass_distance = (teammate_pos - player_pos).magnitude();

        let opponents_between = ctx.players().opponents().all()
            .filter(|opponent| {
                let to_opponent = opponent.position - player_pos;
                let projection_distance = to_opponent.dot(&pass_direction);

                if projection_distance <= 0.0 || projection_distance >= pass_distance {
                    return false;
                }

                let projected_point = player_pos + pass_direction * projection_distance;
                let perp_distance = (opponent.position - projected_point).magnitude();

                perp_distance < 8.0
            })
            .collect::<Vec<_>>();

        if opponents_between.len() >= 2 {
            let vision_skill = ctx.player.skills.mental.vision / 20.0;
            let passing_skill = ctx.player.skills.technical.passing / 20.0;
            let skill_factor = (vision_skill + passing_skill) / 2.0;
            let max_opponents = 2.0 + (skill_factor * 2.0);

            return opponents_between.len() as f32 <= max_opponents;
        }

        false
    }

    /// Calculate breakthrough value
    fn calculate_breakthrough_value(
        &self,
        ctx: &StateProcessingContext,
        teammate: &MatchPlayerLite,
    ) -> f32 {
        let goal_distance = (teammate.position - ctx.player().opponent_goal_position()).magnitude();
        let field_width = ctx.context.field_size.width as f32;

        let goal_distance_value = 1.0 - (goal_distance / field_width).clamp(0.0, 1.0);
        let space_value = self.calculate_improved_space_score(ctx, teammate);

        let player = ctx.player();
        let target_skills = player.skills(teammate.id);
        let finishing_skill = target_skills.technical.finishing / 20.0;

        (goal_distance_value * 0.5) + (space_value * 0.3) + (finishing_skill * 0.2)
    }

    /// Check for clear passing lanes with improved logic
    #[allow(dead_code)]
    fn has_clear_passing_lane(&self, ctx: &StateProcessingContext, teammate: &MatchPlayerLite) -> bool {
        let player_position = ctx.player.position;
        let teammate_position = teammate.position;
        let passing_direction = (teammate_position - player_position).normalize();
        let pass_distance = (teammate_position - player_position).magnitude();

        let pass_skill = ctx.player.skills.technical.passing / 20.0;
        let vision_skill = ctx.player.skills.mental.vision / 20.0;

        let base_lane_width = 3.0;
        let skill_factor = 0.6 + (pass_skill * 0.2) + (vision_skill * 0.2);
        let lane_width = base_lane_width * skill_factor;

        let intercepting_opponents = ctx.players().opponents().all()
            .filter(|opponent| {
                let to_opponent = opponent.position - player_position;
                let projection_distance = to_opponent.dot(&passing_direction);

                if projection_distance <= 0.0 || projection_distance >= pass_distance {
                    return false;
                }

                let projected_point = player_position + passing_direction * projection_distance;
                let perp_distance = (opponent.position - projected_point).magnitude();

                let interception_skill = ctx.player().skills(opponent.id).technical.tackling / 20.0;
                let effective_width = lane_width * (1.0 - interception_skill * 0.3);

                perp_distance < effective_width
            })
            .count();

        intercepting_opponents == 0
    }

    /// Check if player is heavily marked
    #[allow(dead_code)]
    fn is_heavily_marked(&self, ctx: &StateProcessingContext, teammate: &MatchPlayerLite) -> bool {
        const MARKING_DISTANCE: f32 = 5.0;
        const MAX_MARKERS: usize = 2;

        // Use pre-computed distances: opponents near teammate
        let mut marker_count = 0;
        let mut single_marker_id = 0u32;
        let mut single_marker_dist = 0.0f32;
        for (opp_id, dist) in ctx.tick_context.distances.opponents(teammate.id, MARKING_DISTANCE) {
            marker_count += 1;
            single_marker_id = opp_id;
            single_marker_dist = dist;
        }

        if marker_count >= MAX_MARKERS {
            return true;
        }

        if marker_count == 1 {
            let marking_skill = ctx.player().skills(single_marker_id).mental.positioning;
            if marking_skill > 16.0 && single_marker_dist < 2.5 {
                return true;
            }
        }

        false
    }

    /// Check if teammate is in good position
    #[allow(dead_code)]
    fn is_in_good_position(&self, ctx: &StateProcessingContext, teammate: &MatchPlayerLite) -> bool {
        let is_backward_pass = match ctx.player.side {
            Some(PlayerSide::Left) => teammate.position.x < ctx.player.position.x,
            Some(PlayerSide::Right) => teammate.position.x > ctx.player.position.x,
            None => false,
        };

        let player_goal_distance =
            (ctx.player.position - ctx.player().opponent_goal_position()).magnitude();
        let teammate_goal_distance =
            (teammate.position - ctx.player().opponent_goal_position()).magnitude();
        let advances_toward_goal = teammate_goal_distance < player_goal_distance;

        if is_backward_pass {
            let under_pressure = self.is_under_heavy_pressure(ctx);
            let has_good_vision = ctx.player.skills.mental.vision > 15.0;
            return under_pressure || has_good_vision;
        }

        let teammate_will_be_pressured = ctx.tick_context.distances
            .opponents(teammate.id, 15.0)
            .any(|(opp_id, _dist)| {
                let opp_pos = ctx.tick_context.positions.players.position(opp_id);
                let opponent_velocity = ctx.tick_context.positions.players.velocity(opp_id);
                let future_opponent_pos = opp_pos + opponent_velocity * 10.0;
                let future_distance = (future_opponent_pos - teammate.position).magnitude();
                future_distance < 5.0
            });

        advances_toward_goal && !teammate_will_be_pressured
    }

    /// Determine if should shoot instead of pass
    fn should_shoot_instead_of_pass(&self, ctx: &StateProcessingContext) -> bool {
        let distance_to_goal = ctx.ball().distance_to_opponent_goal();
        let shooting_skill = ctx.player.skills.technical.long_shots / 20.0;
        let finishing_skill = ctx.player.skills.technical.finishing / 20.0;

        let shooting_ability = (shooting_skill * 0.7) + (finishing_skill * 0.3);
        let effective_shooting_range = 150.0 + (shooting_ability * 100.0);

        distance_to_goal < effective_shooting_range && ctx.player().has_clear_shot()
    }

    /// Check if under heavy pressure
    fn is_under_heavy_pressure(&self, ctx: &StateProcessingContext) -> bool {
        ctx.player().pressure().is_under_heavy_pressure()
    }

    /// Check if should adjust position
    fn should_adjust_position(&self, ctx: &StateProcessingContext) -> bool {
        self.find_best_pass_option(ctx).is_none() &&
            self.find_breakthrough_pass_option(ctx).is_none() &&
            !self.is_under_heavy_pressure(ctx)
    }

    /// Calculate better position for passing
    fn calculate_better_passing_position(
        &self,
        ctx: &StateProcessingContext,
        target: &MatchPlayerLite,
    ) -> Vector3<f32> {
        let player_pos = ctx.player.position;
        let target_pos = target.position;

        let to_target = target_pos - player_pos;
        let direction = to_target.normalize();

        let perpendicular = Vector3::new(-direction.y, direction.x, 0.0);
        let adjustment = perpendicular * 5.0;

        player_pos + adjustment
    }
}