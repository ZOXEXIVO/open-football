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
        // Add timeout to avoid getting stuck
        if ctx.in_state_time > 60 {
            return Some(StateChangeResult::with_midfielder_state(
                MidfielderState::Running
            ));
        }

        if ctx.player.has_ball(ctx) {
            // If the player has the ball, consider shooting, passing, or dribbling
            if self.is_in_shooting_position(ctx) {
                return Some(StateChangeResult::with_midfielder_state(
                    MidfielderState::DistanceShooting,
                ));
            }

            if self.find_open_teammate(ctx).is_some() {
                return Some(StateChangeResult::with_midfielder_state(
                    MidfielderState::Passing
                ));
            }
        } else {
            // If they don't have the ball anymore, change state
            return Some(StateChangeResult::with_midfielder_state(
                MidfielderState::Running,
            ));
        }

        None
    }

    fn process_slow(&self, _ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        None
    }

    fn velocity(&self, ctx: &StateProcessingContext) -> Option<Vector3<f32>> {
        // Instead of returning zero velocity, actually dribble toward goal
        if ctx.player.has_ball(ctx) {
            // Dribble toward the goal with some variance
            let goal_pos = ctx.player().opponent_goal_position();
            let player_pos = ctx.player.position;
            let direction = (goal_pos - player_pos).normalize();

            // Add some randomness to dribbling direction for realism
            let jitter_x = (rand::random::<f32>() - 0.5) * 0.2;
            let jitter_y = (rand::random::<f32>() - 0.5) * 0.2;
            let jitter = Vector3::new(jitter_x, jitter_y, 0.0);

            // Calculate speed based on player's dribbling and pace
            let dribble_skill = ctx.player.skills.technical.dribbling / 20.0;
            let pace = ctx.player.skills.physical.pace / 20.0;
            let speed = 3.0 * (0.7 * dribble_skill + 0.3 * pace);

            Some((direction + jitter).normalize() * speed)
        } else {
            // If player doesn't have the ball anymore, move toward it
            let ball_pos = ctx.tick_context.positions.ball.position;
            let player_pos = ctx.player.position;
            let direction = (ball_pos - player_pos).normalize();
            let speed = ctx.player.skills.physical.pace * 0.3;

            Some(direction * speed)
        }
    }

    fn process_conditions(&self, _ctx: ConditionContext) {}
}

impl MidfielderDribblingState {
    fn find_open_teammate<'a>(&self, ctx: &StateProcessingContext<'a>) -> Option<u32> {
        // Find an open teammate to pass to
        let players = ctx.players();
        let teammates = players.teammates();

        let teammates = teammates.nearby_ids(150.0);

        if let Some((teammate_id, _)) = teammates.choose(&mut rand::rng()) {
            return Some(teammate_id);
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
