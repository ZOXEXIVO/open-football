use crate::r#match::forwarders::states::ForwardState;
use crate::r#match::{
    ConditionContext, PlayerSide, StateChangeResult, StateProcessingContext,
    StateProcessingHandler, SteeringBehavior,
};
use nalgebra::Vector3;

const MAX_DISTANCE_FROM_BALL: f32 = 80.0; // Don't move too far from ball
const MIN_DISTANCE_FROM_BALL: f32 = 50.0; // Don't get too close to ball carrier
const SUPPORT_DISTANCE: f32 = 30.0; // Ideal support distance from teammates
const MAX_LATERAL_MOVEMENT: f32 = 40.0; // Maximum sideways movement from current position

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
            return Some(StateChangeResult::with_forward_state(ForwardState::Running));
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

        None
    }

    fn process_slow(&self, _ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        None
    }

    fn velocity(&self, ctx: &StateProcessingContext) -> Option<Vector3<f32>> {
        Some(
            SteeringBehavior::Arrive {
                target: ctx.ball().direction_to_opponent_goal(),
                slowing_distance: 150.0,
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