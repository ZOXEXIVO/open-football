use crate::r#match::midfielders::states::MidfielderState;
use crate::r#match::{
    ConditionContext, StateChangeResult, StateProcessingContext, StateProcessingHandler,
};
use nalgebra::Vector3;
use rand::prelude::IteratorRandom;

#[derive(Default)]
pub struct MidfielderDribblingState {}

impl StateProcessingHandler for MidfielderDribblingState {
    fn try_fast(&self, ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        if ctx.player.has_ball(ctx) {
            // If the player has the ball, consider shooting, passing, or dribbling
            if self.is_in_shooting_position(ctx) {
                return Some(StateChangeResult::with_midfielder_state(
                    MidfielderState::DistanceShooting,
                ));
            }

            if let Some(_) = self.find_open_teammate(ctx) {
                return Some(StateChangeResult::with_midfielder_state(
                    MidfielderState::Passing
                ));
            }

            if ctx.in_state_time > 100 {
                return Some(StateChangeResult::with_midfielder_state(
                    MidfielderState::Passing
                ));
            }
        } else {
            if self.should_press(ctx) {
                return Some(StateChangeResult::with_midfielder_state(
                    MidfielderState::Pressing,
                ));
            }

            if self.should_support_attack(ctx) {
                return Some(StateChangeResult::with_midfielder_state(
                    MidfielderState::AttackSupporting,
                ));
            }

            if self.should_return_to_position(ctx) {
                return Some(StateChangeResult::with_midfielder_state(
                    MidfielderState::Returning,
                ));
            }
        }

        None
    }

    fn process_slow(&self, _ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        None
    }

    fn velocity(&self, _ctx: &StateProcessingContext) -> Option<Vector3<f32>> {
        Some(Vector3::new(0.0, 0.0, 0.0))
    }

    fn process_conditions(&self, _ctx: ConditionContext) {}
}

impl MidfielderDribblingState {
    fn find_open_teammate<'a>(&self, ctx: &StateProcessingContext<'a>) -> Option<u32> {
        // Find an open teammate to pass to
        let players = ctx.players();
        let teammates = players.teammates();

        let teammates = teammates.nearby_ids(150.0);

        if let Some((teammate_id, _)) = teammates.choose(&mut rand::thread_rng()) {
            return Some(teammate_id)
        }

        None
    }

    fn is_in_shooting_position(&self, ctx: &StateProcessingContext) -> bool {
        let shooting_range = 25.0; // Distance from goal to consider shooting
        let player_position = ctx.player.position;
        let goal_position = ctx.player().opponent_goal_position();

        let distance_to_goal = (player_position - goal_position).magnitude();

        distance_to_goal <= shooting_range
    }

    fn should_support_attack(&self, ctx: &StateProcessingContext) -> bool {
        // Check if the team is in possession and the player is in a good position to support the attack
        let team_in_possession = ctx.team().is_control_ball();
        let in_attacking_half = ctx.player.position.x > ctx.context.field_size.width as f32 / 2.0;

        team_in_possession && in_attacking_half
    }

    fn should_return_to_position(&self, ctx: &StateProcessingContext) -> bool {
        // Check if the player is far from their starting position and the team is not in possession
        let distance_from_start = ctx.player().distance_from_start_position();
        let team_in_possession = ctx.team().is_control_ball();

        distance_from_start > 20.0 && !team_in_possession
    }

    fn should_press(&self, ctx: &StateProcessingContext) -> bool {
        // Check if the player should press the opponent with the ball
        let ball_distance = ctx.ball().distance();
        let pressing_distance = 150.0; // Adjust the threshold as needed

        !ctx.team().is_control_ball() && ball_distance < pressing_distance
    }
}
