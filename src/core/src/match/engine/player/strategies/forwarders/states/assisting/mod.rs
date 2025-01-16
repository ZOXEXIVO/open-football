use crate::r#match::forwarders::states::ForwardState;
use crate::r#match::{ConditionContext, StateChangeResult, StateProcessingContext, StateProcessingHandler, SteeringBehavior};
use nalgebra::Vector3;

#[derive(Default)]
pub struct ForwardAssistingState {}

impl StateProcessingHandler for ForwardAssistingState {
    fn try_fast(&self, ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        if !ctx.team().is_control_ball(){
            return Some(StateChangeResult::with_forward_state(
                ForwardState::Running,
            ));
        }

        if ctx.ball().distance() < 200.0 && ctx.ball().is_towards_player_with_angle(0.9) {
            return Some(StateChangeResult::with_forward_state(
                ForwardState::Intercepting
            ));
        }

        // Check if the player is on the opponent's side of the field
        if ctx.team().is_control_ball() && !ctx.player().on_own_side() && ctx.players().opponents().exists(100.0){
            // If not on the opponent's side, focus on creating space and moving forward
            return Some(StateChangeResult::with_forward_state(
                ForwardState::CreatingSpace,
            ));
        }

        // Check if there's an immediate threat from an opponent
        if self.is_under_pressure(ctx) {
            // If under high pressure, decide between quick pass or dribbling
            if self.should_make_quick_pass(ctx) {
                if let Some(_teammate_id) = self.find_best_teammate_to_assist(ctx) {
                    //result.events.add_player_event(PlayerEvent::Pass(ctx.player.player_id, teammate_id));
                    return Some(StateChangeResult::with_forward_state(
                        ForwardState::Dribbling,
                    ));
                }
            }
            // If no good passing option, try to dribble
            return Some(StateChangeResult::with_forward_state(
                ForwardState::Dribbling,
            ));
        }

        // If not under immediate pressure, look for assist opportunities
        if let Some(_) = self.find_best_teammate_to_assist(ctx) {
            return Some(StateChangeResult::with_forward_state(ForwardState::Passing));
        }

        if self.is_in_shooting_range(ctx) && ctx.player.has_ball(ctx) {
            return Some(StateChangeResult::with_forward_state(
                ForwardState::Shooting,
            ));
        } else if self.should_create_space(ctx) {
            return Some(StateChangeResult::with_forward_state(
                ForwardState::CreatingSpace,
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
                target: ctx.player().opponent_goal_position(),
                slowing_distance: 10.0,
            }
            .calculate(ctx.player)
            .velocity,
        )
    }

    fn process_conditions(&self, _ctx: ConditionContext) {}
}

impl ForwardAssistingState {
    fn is_under_pressure(&self, ctx: &StateProcessingContext) -> bool {
        ctx.players().opponents().exists(10.0)
    }

    fn should_make_quick_pass(&self, ctx: &StateProcessingContext) -> bool {
        // Decision based on player's skills and game situation
        ctx.player.skills.technical.passing > 70.0 && ctx.player.skills.mental.decisions > 65.0
    }

    fn find_best_teammate_to_assist(&self, ctx: &StateProcessingContext) -> Option<u32> {
        ctx.players()
            .teammates()
            .nearby_ids(200.0)
            .filter(|(id, _)| self.is_in_good_scoring_position(ctx, *id))
            .min_by(|(_, dist_a), (_, dist_b)| {
                dist_a
                    .partial_cmp(dist_b)
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
            .map(|(id, _)| id)
    }

    fn is_in_good_scoring_position(&self, ctx: &StateProcessingContext, _player_id: u32) -> bool {
        // TODO
        let distance_to_goal = ctx.ball().distance_to_opponent_goal();
        distance_to_goal < 20.0
    }

    fn is_in_shooting_range(&self, ctx: &StateProcessingContext) -> bool {
        let distance_to_goal = ctx.ball().distance_to_opponent_goal();
        distance_to_goal < 25.0
    }

    fn should_create_space(&self, ctx: &StateProcessingContext) -> bool {
        ctx.player.skills.mental.off_the_ball > 15.0
            && ctx.players().teammates().exists(100.0)
    }
}
