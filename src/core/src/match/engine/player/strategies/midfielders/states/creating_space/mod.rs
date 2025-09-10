use crate::r#match::{
    ConditionContext, PlayerSide, StateChangeResult, StateProcessingContext,
    StateProcessingHandler, SteeringBehavior,
};
use nalgebra::Vector3;
use crate::r#match::midfielders::states::MidfielderState;

const MAX_DISTANCE_FROM_BALL: f32 = 80.0; // Don't move too far from ball
const MIN_DISTANCE_FROM_BALL: f32 = 15.0; // Don't get too close to ball carrier
const SUPPORT_DISTANCE: f32 = 30.0; // Ideal support distance from teammates
const MAX_LATERAL_MOVEMENT: f32 = 40.0; // Maximum sideways movement from current position

#[derive(Default)]
pub struct MidfielderCreatingSpaceState {}

impl StateProcessingHandler for MidfielderCreatingSpaceState {
    fn try_fast(&self, ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        // Check if player has the ball - immediate transition
        if ctx.player.has_ball(ctx) {
            return Some(StateChangeResult::with_midfielder_state(
                MidfielderState::Running,
            ));
        }

        // Check if team lost possession - switch to defensive positioning
        if !ctx.team().is_control_ball() {
            return Some(StateChangeResult::with_midfielder_state(MidfielderState::Running));
        }

        // If the ball is close and moving toward player, try to intercept
        if ctx.ball().distance() < 100.0 && ctx.ball().is_towards_player_with_angle(0.8) {
            return Some(StateChangeResult::with_midfielder_state(
                MidfielderState::Intercepting,
            ));
        }

        // Check if the player has successfully created space and should receive the ball
        if self.has_created_good_space(ctx) {
            return Some(StateChangeResult::with_midfielder_state(
                MidfielderState::Running,
            ));
        }

        // Check if the player should make an intelligent run forward
        if self.should_make_midfielder_run(ctx) {
            return Some(StateChangeResult::with_midfielder_state(
                MidfielderState::Running
            ));
        }

        None
    }

    fn process_slow(&self, _ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        None
    }

    fn velocity(&self, ctx: &StateProcessingContext) -> Option<Vector3<f32>> {
        let target_position = self.calculate_space_creating_position(ctx);

        Some(
            SteeringBehavior::Arrive {
                target: target_position,
                slowing_distance: 15.0,
            }
                .calculate(ctx.player)
                .velocity
                + ctx.player().separation_velocity(),
        )
    }

    fn process_conditions(&self, _ctx: ConditionContext) {}
}

impl MidfielderCreatingSpaceState {
    /// Check if the player has created good space for receiving a pass
    fn has_created_good_space(&self, ctx: &StateProcessingContext) -> bool {
        // Check if there are no opponents within the space threshold
        let space_created = !ctx.players().opponents().exists(20.0);

        // Check if player is in a reasonable supporting position
        let in_support_position = self.is_in_good_support_position(ctx);

        // Check if there's a clear passing lane from ball holder
        let has_clear_lane = self.has_clear_passing_lane_from_ball_holder(ctx);

        // Minimum time to avoid rapid state changes
        let minimum_time_in_state = 30;

        // Don't be too far from the action
        let reasonable_distance = ctx.ball().distance() < MAX_DISTANCE_FROM_BALL;

        space_created && in_support_position && has_clear_lane
            && ctx.in_state_time > minimum_time_in_state && reasonable_distance
    }

    /// Determine if the player should make a forward run
    fn should_make_midfielder_run(&self, ctx: &StateProcessingContext) -> bool {
        // Only make runs when team has possession and ball is in good position
        if !ctx.team().is_control_ball() {
            return false;
        }

        // Check if ball holder is in a position to make a forward pass
        let ball_holder_can_pass = self.ball_holder_can_make_forward_pass(ctx);

        // Check if there's space to run into ahead
        let space_ahead = self.has_space_ahead_for_run(ctx);

        // Check if player isn't offside
        let not_offside = ctx.player().on_own_side();

        // Make runs more likely when in attacking phase but not too deep
        let in_good_phase = self.is_in_good_attacking_phase(ctx);

        // Don't make runs if already too far from ball
        let not_too_far = ctx.ball().distance() < MAX_DISTANCE_FROM_BALL;

        ball_holder_can_pass && space_ahead && not_offside && in_good_phase && not_too_far
    }

    /// Calculate an intelligent position for creating space that maintains team shape
    fn calculate_space_creating_position(&self, ctx: &StateProcessingContext) -> Vector3<f32> {
        let ball_pos = ctx.tick_context.positions.ball.position;
        let field_width = ctx.context.field_size.width as f32;
        let field_height = ctx.context.field_size.height as f32;

        // Get ball holder information
        let ball_holder = self.get_ball_holder(ctx);

        if let Some(holder) = ball_holder {
            return self.calculate_support_position_relative_to_holder(ctx, &holder);
        }

        // No specific ball holder - calculate general support position
        self.calculate_general_support_position(ctx, ball_pos, field_width, field_height)
    }

    /// Calculate position relative to the ball holder that creates good support
    fn calculate_support_position_relative_to_holder(
        &self,
        ctx: &StateProcessingContext,
        holder: &crate::r#match::MatchPlayerLite,
    ) -> Vector3<f32> {
        let player_position = ctx.player.position;
        let holder_position = holder.position;
        let field_width = ctx.context.field_size.width as f32;
        let field_height = ctx.context.field_size.height as f32;

        // Determine attacking direction based on side
        let attacking_direction = match ctx.player.side.unwrap_or(PlayerSide::Left) {
            PlayerSide::Left => 1.0,  // Moving towards positive X
            PlayerSide::Right => -1.0, // Moving towards negative X
        };

        // Calculate distance from ball holder
        let distance_from_holder = (player_position - holder_position).magnitude();

        // Determine movement type based on current situation
        let target_position = if distance_from_holder < MIN_DISTANCE_FROM_BALL {
            // Too close to ball holder - move away to create space
            self.move_away_from_ball_holder(ctx, holder, attacking_direction)
        } else if distance_from_holder > MAX_DISTANCE_FROM_BALL {
            // Too far from ball holder - move closer for support
            self.move_closer_to_ball_holder(ctx, holder)
        } else {
            // Good distance - adjust position for optimal passing angles
            self.adjust_for_passing_angles(ctx, holder, attacking_direction)
        };

        // Ensure the position is within field boundaries
        self.constrain_position_to_field(target_position, field_width, field_height)
    }

    /// Move away from ball holder while maintaining attacking intent
    fn move_away_from_ball_holder(
        &self,
        ctx: &StateProcessingContext,
        holder: &crate::r#match::MatchPlayerLite,
        attacking_direction: f32,
    ) -> Vector3<f32> {
        let player_position = ctx.player.position;
        let holder_position = holder.position;

        // Determine player's natural side based on starting position
        let player_natural_side = if ctx.player.start_position.y < ctx.context.field_size.height as f32 / 2.0 {
            -1.0 // Left side player
        } else {
            1.0  // Right side player
        };

        // Create space by moving diagonally away from holder toward goal on player's natural side
        let away_from_holder = (player_position - holder_position).normalize();
        let toward_goal = Vector3::new(attacking_direction, 0.0, 0.0);
        let to_natural_side = Vector3::new(0.0, player_natural_side, 0.0);

        // Blend the directions with emphasis on maintaining side
        let movement_direction = (away_from_holder * 0.4 + toward_goal * 0.4 + to_natural_side * 0.2).normalize();

        holder_position + movement_direction * SUPPORT_DISTANCE
    }

    /// Move closer to ball holder for better support
    fn move_closer_to_ball_holder(
        &self,
        ctx: &StateProcessingContext,
        holder: &crate::r#match::MatchPlayerLite,
    ) -> Vector3<f32> {
        let player_position = ctx.player.position;
        let holder_position = holder.position;
        let field_height = ctx.context.field_size.height as f32;

        // Move toward holder but maintain natural side
        let toward_holder = (holder_position - player_position).normalize();

        // Determine if player should stay on their natural side
        let player_natural_side = if ctx.player.start_position.y < field_height / 2.0 {
            -1.0 // Left side
        } else {
            1.0  // Right side
        };

        // Calculate offset to maintain width
        let current_y_diff = player_position.y - holder_position.y;
        let needs_width_adjustment = current_y_diff.abs() < 15.0; // Too narrow

        let offset_direction = if needs_width_adjustment {
            Vector3::new(0.0, player_natural_side, 0.0)
        } else {
            // Maintain current width relationship
            Vector3::new(0.0, current_y_diff.signum() * 0.3, 0.0)
        };

        let movement_direction = (toward_holder * 0.7 + offset_direction * 0.3).normalize();
        player_position + movement_direction * 20.0
    }

    /// Adjust position for optimal passing angles
    fn adjust_for_passing_angles(
        &self,
        ctx: &StateProcessingContext,
        holder: &crate::r#match::MatchPlayerLite,
        attacking_direction: f32,
    ) -> Vector3<f32> {
        let player_position = ctx.player.position;
        let holder_position = holder.position;
        let field_height = ctx.context.field_size.height as f32;

        // Determine player's preferred side
        let player_preferred_y = if ctx.player.start_position.y < field_height / 2.0 {
            holder_position.y - 25.0 // Stay on left/lower side
        } else {
            holder_position.y + 25.0 // Stay on right/upper side
        };

        // Calculate forward offset based on attacking direction
        let forward_offset = attacking_direction * 20.0;

        // Don't move too far laterally from current position
        let target_y = if (player_preferred_y - player_position.y).abs() > MAX_LATERAL_MOVEMENT {
            // Limit lateral movement
            player_position.y + (player_preferred_y - player_position.y).signum() * MAX_LATERAL_MOVEMENT
        } else {
            player_preferred_y
        };

        Vector3::new(
            holder_position.x + forward_offset,
            target_y,
            0.0,
        )
    }

    /// Calculate a general support position when no specific ball holder
    fn calculate_general_support_position(
        &self,
        ctx: &StateProcessingContext,
        ball_pos: Vector3<f32>,
        field_width: f32,
        field_height: f32,
    ) -> Vector3<f32> {
        let player_position = ctx.player.position;

        // Maintain natural positioning relative to starting position
        let natural_y_position = ctx.player.start_position.y;
        let y_deviation = (player_position.y - natural_y_position).abs();

        // Move toward ball but maintain reasonable distance and natural side
        let distance_to_ball = (player_position - ball_pos).magnitude();

        if distance_to_ball > MAX_DISTANCE_FROM_BALL {
            // Move closer to ball but stay on natural side
            let toward_ball = (ball_pos - player_position).normalize();
            let to_natural_side = Vector3::new(0.0, (natural_y_position - player_position.y).signum(), 0.0);

            let movement = (toward_ball * 0.8 + to_natural_side * 0.2).normalize();
            player_position + movement * 30.0
        } else if distance_to_ball < MIN_DISTANCE_FROM_BALL {
            // Move away from ball
            let away_from_ball = (player_position - ball_pos).normalize();
            player_position + away_from_ball * 20.0
        } else {
            // Adjust position slightly for better support while maintaining side
            let attacking_direction = match ctx.player.side.unwrap_or(PlayerSide::Left) {
                PlayerSide::Left => Vector3::new(1.0, 0.0, 0.0),
                PlayerSide::Right => Vector3::new(-1.0, 0.0, 0.0),
            };

            // Correct excessive deviation from natural position
            let y_correction = if y_deviation > MAX_LATERAL_MOVEMENT {
                Vector3::new(0.0, (natural_y_position - player_position.y) * 0.1, 0.0)
            } else {
                Vector3::zeros()
            };

            player_position + attacking_direction * 15.0 + y_correction
        }
    }

    /// Constrain position to field boundaries with margins
    fn constrain_position_to_field(
        &self,
        target_position: Vector3<f32>,
        field_width: f32,
        field_height: f32,
    ) -> Vector3<f32> {
        Vector3::new(
            target_position.x.clamp(field_width * 0.05, field_width * 0.95),
            target_position.y.clamp(field_height * 0.05, field_height * 0.95),
            0.0,
        )
    }

    // Helper methods for tactical decision making

    fn get_ball_holder(&self, ctx: &StateProcessingContext) -> Option<crate::r#match::MatchPlayerLite> {
        ctx.players()
            .teammates()
            .all()
            .find(|t| ctx.ball().owner_id() == Some(t.id))
    }

    fn is_in_good_support_position(&self, ctx: &StateProcessingContext) -> bool {
        let ball_distance = ctx.ball().distance();

        // Good support position is not too close or too far from ball
        ball_distance >= MIN_DISTANCE_FROM_BALL && ball_distance <= MAX_DISTANCE_FROM_BALL
    }

    fn has_clear_passing_lane_from_ball_holder(&self, ctx: &StateProcessingContext) -> bool {
        if let Some(holder) = self.get_ball_holder(ctx) {
            let pass_direction = (ctx.player.position - holder.position).normalize();
            let pass_distance = (ctx.player.position - holder.position).magnitude();

            // Check for opponents in the passing lane
            let blocking_opponents = ctx.players().opponents().all()
                .filter(|opp| {
                    let to_opponent = opp.position - holder.position;
                    let projection = to_opponent.dot(&pass_direction);

                    if projection <= 0.0 || projection >= pass_distance {
                        return false;
                    }

                    let projected_point = holder.position + pass_direction * projection;
                    let perp_distance = (opp.position - projected_point).magnitude();

                    perp_distance < 5.0 // Slightly more lenient
                })
                .count();

            blocking_opponents == 0
        } else {
            true // No specific holder, assume clear
        }
    }

    fn ball_holder_can_make_forward_pass(&self, ctx: &StateProcessingContext) -> bool {
        if let Some(holder) = self.get_ball_holder(ctx) {
            let player = ctx.player();
            let holder_skills = player.skills(holder.id);
            let vision_skill = holder_skills.mental.vision;
            let passing_skill = holder_skills.technical.passing;

            // Check if holder is under pressure
            let holder_under_pressure = ctx.players().opponents().all()
                .any(|opp| (opp.position - holder.position).magnitude() < 8.0);

            // More lenient skill requirements
            (vision_skill > 10.0 || passing_skill > 12.0) && !holder_under_pressure
        } else {
            false
        }
    }

    fn has_space_ahead_for_run(&self, ctx: &StateProcessingContext) -> bool {
        let player_position = ctx.player.position;
        let attacking_direction = match ctx.player.side.unwrap_or(PlayerSide::Left) {
            PlayerSide::Left => Vector3::new(1.0, 0.0, 0.0),
            PlayerSide::Right => Vector3::new(-1.0, 0.0, 0.0),
        };

        let check_position = player_position + attacking_direction * 40.0;

        // Check if there are opponents in the space we want to run into
        let opponents_in_space = ctx.players().opponents().all()
            .filter(|opp| (opp.position - check_position).magnitude() < 15.0)
            .count();

        opponents_in_space < 2
    }

    fn is_in_good_attacking_phase(&self, ctx: &StateProcessingContext) -> bool {
        let ball_distance_to_goal = ctx.ball().distance_to_opponent_goal();
        let field_width = ctx.context.field_size.width as f32;
        let player_distance_to_goal = ctx.ball().distance_to_opponent_goal();

        // Good attacking phase: ball is in attacking half and player isn't too deep
        ball_distance_to_goal < field_width * 0.7 && player_distance_to_goal > field_width * 0.2
    }
}