use crate::r#match::events::Event;
use crate::r#match::midfielders::states::common::{ActivityIntensity, MidfielderCondition};
use crate::r#match::midfielders::states::MidfielderState;
use crate::r#match::player::events::{PassingEventContext, PlayerEvent};
use crate::r#match::{
    ConditionContext, MatchPlayerLite, StateChangeResult, StateProcessingContext,
    StateProcessingHandler, SteeringBehavior,
};
use nalgebra::Vector3;

#[derive(Default)]
pub struct MidfielderSwitchingPlayState {}

impl StateProcessingHandler for MidfielderSwitchingPlayState {
    fn try_fast(&self, ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        if !ctx.player.has_ball(ctx) {
            return Some(StateChangeResult::with_midfielder_state(
                MidfielderState::Returning,
            ));
        }

        // Check if there's a good opportunity to switch play
        if let Some((teammate_id, _)) = self.find_switch_play_target(ctx) {
            // If a suitable target position is found, switch play
            return Some(StateChangeResult::with_midfielder_state_and_event(
                MidfielderState::Passing,
                Event::PlayerEvent(PlayerEvent::PassTo(
                    PassingEventContext::new()
                        .with_from_player_id(ctx.player.id)
                        .with_to_player_id(teammate_id)
                        .build(ctx)
                )),
            ));
        }

        // If no suitable opportunity to switch play, continue with the current state
        None
    }

    fn process_slow(&self, _ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        None
    }

    fn velocity(&self, ctx: &StateProcessingContext) -> Option<Vector3<f32>> {
        // Move towards the best position to switch play
        if let Some((_, teammate_position)) = self.find_switch_play_target(ctx) {
            let steering = SteeringBehavior::Seek {
                target: teammate_position,
            }
                .calculate(ctx.player);

            Some(steering.velocity)
        } else {
            // If no suitable target position is found, stay in the current position
            Some(Vector3::new(0.0, 0.0, 0.0))
        }
    }

    fn process_conditions(&self, ctx: ConditionContext) {
        // Switching play is low intensity - tactical passing
        MidfielderCondition::new(ActivityIntensity::Low).process(ctx);
    }
}

impl MidfielderSwitchingPlayState {
    fn find_switch_play_target(&self, ctx: &StateProcessingContext) -> Option<(u32, Vector3<f32>)> {
        // Find the best position to switch play to
        let player_position = ctx.player.position;
        let ball_position = ctx.tick_context.positions.ball.position;

        // Calculate the direction perpendicular to the player's forward direction
        let forward_direction = (ball_position - player_position).normalize();
        let perpendicular_direction = Vector3::new(-forward_direction.y, forward_direction.x, 0.0);

        // Find teammates on the opposite side of the field
        let opposite_side_teammates: Vec<MatchPlayerLite> = ctx.players().teammates()
            .all()
            .filter(|teammate| {
                let teammate_to_player = player_position - teammate.position;
                let dot_product = teammate_to_player.dot(&perpendicular_direction);
                dot_product > 0.0 // Teammate is on the opposite side
            })
            .collect();

        // Find the teammate with the most space
        let best_teammate = opposite_side_teammates.iter().max_by(|a, b| {
            let space_a = self.calculate_space_around_player(ctx, *a);
            let space_b = self.calculate_space_around_player(ctx, *b);
            space_a.partial_cmp(&space_b).unwrap()
        });

        if let Some(teammate) = best_teammate.map(|teammate| teammate) {
            return Some((teammate.id, teammate.position));
        }

        None
    }

    fn calculate_space_around_player(
        &self,
        ctx: &StateProcessingContext,
        player: &MatchPlayerLite,
    ) -> f32 {
        // Calculate the amount of free space around a player
        let space_radius = 10.0; // Adjust the radius as needed
        let num_opponents_nearby = ctx
            .players()
            .opponents()
            .all()
            .filter(|opponent| {
                let distance = (opponent.position - player.position).magnitude();
                distance <= space_radius
            })
            .count();

        space_radius - num_opponents_nearby as f32
    }
}
