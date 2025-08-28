use crate::r#match::forwarders::states::ForwardState;
use crate::r#match::{
    ConditionContext, PlayerSide, StateChangeResult, StateProcessingContext,
    StateProcessingHandler, SteeringBehavior,
};
use nalgebra::Vector3;

const CREATING_SPACE_THRESHOLD: f32 = 150.0;
const MAX_DISTANCE_FROM_BALL: f32 = 80.0; // Don't move too far from ball
const MIN_DISTANCE_FROM_BALL: f32 = 15.0; // Don't get too close to ball carrier
const MAX_TIME_IN_STATE: u64 = 150; // Reduced max time
const SUPPORT_DISTANCE: f32 = 30.0; // Ideal support distance from teammates

#[derive(Default)]
pub struct ForwardCreatingSpaceState {}

impl StateProcessingHandler for ForwardCreatingSpaceState {
    fn try_fast(&self, ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        // Check if player has the ball - immediate transition
        if ctx.player.has_ball(ctx) {
            return Some(StateChangeResult::with_forward_state(
                ForwardState::Running,
            ));
        }

        // Check if team lost possession - switch to defensive positioning
        if !ctx.team().is_control_ball() {
            return Some(StateChangeResult::with_forward_state(ForwardState::Returning));
        }

        // If the ball is close and moving toward player, try to intercept
        if ctx.ball().distance() < 100.0 && ctx.ball().is_towards_player_with_angle(0.8) {
            return Some(StateChangeResult::with_forward_state(
                ForwardState::Intercepting,
            ));
        }

        // Check if the player has successfully created space and should receive the ball
        if self.has_created_good_space(ctx) {
            return Some(StateChangeResult::with_forward_state(
                ForwardState::Assisting,
            ));
        }

        // Check if the player should make an intelligent run forward
        if self.should_make_forward_run(ctx) {
            return Some(StateChangeResult::with_forward_state(
                ForwardState::RunningInBehind,
            ));
        }

        // Add a time limit for staying in this state to prevent getting stuck
        if ctx.in_state_time > MAX_TIME_IN_STATE {
            return Some(StateChangeResult::with_forward_state(ForwardState::Running));
        }

        // Check if player is too far from the ball/action
        if ctx.ball().distance() > MAX_DISTANCE_FROM_BALL * 1.5 {
            return Some(StateChangeResult::with_forward_state(ForwardState::Running));
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

impl ForwardCreatingSpaceState {
    /// Check if the player has created good space for receiving a pass
    fn has_created_good_space(&self, ctx: &StateProcessingContext) -> bool {
        // Check if there are no opponents within the space threshold
        let space_created = !ctx.players().opponents().exists(20.0); // Reduced threshold for realism

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
    fn should_make_forward_run(&self, ctx: &StateProcessingContext) -> bool {
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
        let player_pos = ctx.player.position;
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

        // Determine attacking direction
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
        attacking_direction: f32
    ) -> Vector3<f32> {
        let player_position = ctx.player.position;
        let holder_position = holder.position;

        // Create space by moving diagonally away from holder toward goal
        let away_from_holder = (player_position - holder_position).normalize();
        let toward_goal = Vector3::new(attacking_direction, 0.0, 0.0);

        // Blend the two directions
        let movement_direction = (away_from_holder + toward_goal * 0.7).normalize();

        holder_position + movement_direction * SUPPORT_DISTANCE
    }

    /// Move closer to ball holder for better support
    fn move_closer_to_ball_holder(
        &self,
        ctx: &StateProcessingContext,
        holder: &crate::r#match::MatchPlayerLite
    ) -> Vector3<f32> {
        let player_position = ctx.player.position;
        let holder_position = holder.position;

        // Move toward holder but maintain some separation
        let toward_holder = (holder_position - player_position).normalize();

        // Don't move directly toward holder - offset slightly
        let field_height = ctx.context.field_size.height as f32;
        let offset_direction = if player_position.y < field_height / 2.0 {
            Vector3::new(0.0, 1.0, 0.0)  // Move up field
        } else {
            Vector3::new(0.0, -1.0, 0.0) // Move down field
        };

        let movement_direction = (toward_holder + offset_direction * 0.3).normalize();
        player_position + movement_direction * 20.0
    }

    /// Adjust position for optimal passing angles
    fn adjust_for_passing_angles(
        &self,
        ctx: &StateProcessingContext,
        holder: &crate::r#match::MatchPlayerLite,
        attacking_direction: f32
    ) -> Vector3<f32> {
        let player_position = ctx.player.position;
        let holder_position = holder.position;

        // Move to create better passing angles - slightly forward and wide
        let forward_offset = attacking_direction * 25.0;
        let wide_offset = if player_position.y > holder_position.y {
            15.0  // Move wider if already on the "upper" side
        } else {
            -15.0 // Move wider if on the "lower" side
        };

        Vector3::new(
            holder_position.x + forward_offset,
            holder_position.y + wide_offset,
            0.0
        )
    }

    /// Calculate a general support position when no specific ball holder
    fn calculate_general_support_position(
        &self,
        ctx: &StateProcessingContext,
        ball_pos: Vector3<f32>,
        field_width: f32,
        field_height: f32
    ) -> Vector3<f32> {
        let player_position = ctx.player.position;

        // Move toward ball but maintain reasonable distance
        let distance_to_ball = (player_position - ball_pos).magnitude();

        if distance_to_ball > MAX_DISTANCE_FROM_BALL {
            // Move closer to ball
            let toward_ball = (ball_pos - player_position).normalize();
            player_position + toward_ball * 30.0
        } else if distance_to_ball < MIN_DISTANCE_FROM_BALL {
            // Move away from ball
            let away_from_ball = (player_position - ball_pos).normalize();
            player_position + away_from_ball * 20.0
        } else {
            // Adjust position slightly for better support
            let attacking_direction = match ctx.player.side.unwrap_or(PlayerSide::Left) {
                PlayerSide::Left => Vector3::new(1.0, 0.0, 0.0),
                PlayerSide::Right => Vector3::new(-1.0, 0.0, 0.0),
            };

            player_position + attacking_direction * 15.0
        }
    }

    /// Constrain position to field boundaries
    fn constrain_position_to_field(
        &self,
        target_position: Vector3<f32>,
        field_width: f32,
        field_height: f32
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

        let check_position = player_position + attacking_direction * 40.0; // Reduced distance

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