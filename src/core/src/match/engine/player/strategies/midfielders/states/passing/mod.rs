use crate::r#match::events::Event;
use crate::r#match::midfielders::states::MidfielderState;
use crate::r#match::player::events::{PassingEventContext, PlayerEvent};
use crate::r#match::{
    ConditionContext, MatchPlayerLite, StateChangeResult, StateProcessingContext,
    StateProcessingHandler, SteeringBehavior,
};
use nalgebra::Vector3;

#[derive(Default)]
pub struct MidfielderPassingState {}

impl StateProcessingHandler for MidfielderPassingState {
    fn try_fast(&self, ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        // Check if the midfielder still has the ball
        if !ctx.player.has_ball(ctx) {
            // Lost possession, transition to Pressing
            return Some(StateChangeResult::with_midfielder_state(
                MidfielderState::Pressing,
            ));
        }

        // Determine the best teammate to pass to
        if let Some(target_teammate) = self.find_best_pass_option(ctx) {
            return Some(StateChangeResult::with_midfielder_state_and_event(
                MidfielderState::Standing,
                Event::PlayerEvent(PlayerEvent::PassTo(
                    PassingEventContext::build()
                        .with_from_player_id(ctx.player.id)
                        .with_to_player_id(target_teammate.id)
                        .with_target(target_teammate.position)
                        .with_force(ctx.player().pass_teammate_power(target_teammate.id))
                        .build(),
                )),
            ));
        }

        if ctx.ball().distance_to_opponent_goal() < 200.0 {
            return Some(StateChangeResult::with_midfielder_state(
                MidfielderState::Shooting,
            ));
        }

        None
    }

    fn process_slow(&self, _ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        None
    }

    fn velocity(&self, ctx: &StateProcessingContext) -> Option<Vector3<f32>> {
        if let Some(nearest_teammate) = ctx.players().teammates().nearby_to_opponent_goal() {
            return Some(
                SteeringBehavior::Arrive {
                    target: nearest_teammate.position,
                    slowing_distance: 30.0,
                }
                .calculate(ctx.player)
                .velocity,
            );
        }

        None
    }

    fn process_conditions(&self, _ctx: ConditionContext) {}
}

impl MidfielderPassingState {
    fn find_best_pass_option<'a>(
        &self,
        ctx: &StateProcessingContext<'a>,
    ) -> Option<MatchPlayerLite> {
        let players = ctx.players();
        let teammates = players.teammates();
        let vision_range = ctx.player.skills.mental.vision * 20.0;

        teammates
            .nearby(vision_range)
            .filter(|t| self.is_teammate_open(ctx, t) && ctx.player().has_clear_pass(t.id))
            .max_by(|a, b| a.position.x.total_cmp(&b.position.x))


        // let tensor = Tensor::from_data([[0, 0]], &DEFAULT_NEURAL_DEVICE);
        // let result = MIDFIELDER_PASSING_NEURAL_NETWORK.forward(tensor);
        //
        // let tensor_data_string = result
        //     .to_data()
        //     .iter()
        //     .map(|x: f32| format!("{:.4}", x))
        //     .collect::<Vec<String>>()
        //     .join(", ");
        //
        // println!("### {}", tensor_data_string);
        //
        // None
    }

    fn is_teammate_open(&self, ctx: &StateProcessingContext, teammate: &MatchPlayerLite) -> bool {
        let opponent_distance_threshold = 5.0;
        ctx.players().opponents().all()
            .filter(|o| (o.position - teammate.position).magnitude() <= opponent_distance_threshold)
            .count() == 0
    }
}
