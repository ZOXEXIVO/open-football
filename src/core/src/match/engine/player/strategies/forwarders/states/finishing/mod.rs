use crate::r#match::events::Event;
use crate::r#match::forwarders::states::common::{ActivityIntensity, ForwardCondition};
use crate::r#match::forwarders::states::ForwardState;
use crate::r#match::player::events::PlayerEvent;
use crate::r#match::{
    ConditionContext, StateChangeResult, StateProcessingContext, StateProcessingHandler,
};
use nalgebra::Vector3;

#[derive(Default)]
pub struct ForwardFinishingState {}

impl StateProcessingHandler for ForwardFinishingState {
    fn try_fast(&self, ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        if !ctx.player.has_ball(ctx) {
            // Transition to Running state if the player doesn't have the ball
            return Some(StateChangeResult::with_forward_state(ForwardState::Running));
        }

        // Check if the player is within shooting range
        if !self.is_within_shooting_range(ctx) {
            // Transition to Dribbling state if the player is not within shooting range
            return Some(StateChangeResult::with_forward_state(
                ForwardState::Dribbling,
            ));
        }

        // AGGRESSIVE SHOOTING: In the box, forwards should shoot even without clear shot
        let distance_to_goal = ctx.ball().distance_to_opponent_goal();
        let has_clear_shot = ctx.player().has_clear_shot();
        let should_shoot = has_clear_shot || self.should_shoot_anyway(ctx, distance_to_goal);

        if !should_shoot {
            // Only pass if really no chance to shoot
            return Some(StateChangeResult::with_forward_state(ForwardState::Passing));
        }

        // Calculate the shooting direction and power
        let (shooting_direction, _) = self.calculate_shooting_parameters(ctx);

        // Transition to Running state after taking the shot
        Some(StateChangeResult::with_forward_state_and_event(
            ForwardState::Running,
            Event::PlayerEvent(PlayerEvent::RequestShot(ctx.player.id, shooting_direction)),
        ))
    }

    fn process_slow(&self, _ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        None
    }

    fn velocity(&self, _ctx: &StateProcessingContext) -> Option<Vector3<f32>> {
        Some(Vector3::new(0.0, 0.0, 0.0))
    }

    fn process_conditions(&self, ctx: ConditionContext) {
        // Finishing is very high intensity - explosive action
        ForwardCondition::new(ActivityIntensity::VeryHigh).process(ctx);
    }
}

impl ForwardFinishingState {
    fn is_within_shooting_range(&self, ctx: &StateProcessingContext) -> bool {
        ctx.ball().distance_to_opponent_goal() <= 150.0
    }

    fn calculate_shooting_parameters(&self, ctx: &StateProcessingContext) -> (Vector3<f32>, f32) {
        let goal_position = ctx.player().opponent_goal_position();
        let shooting_direction = (goal_position - ctx.player.position).normalize();
        let shooting_power = 1.0; // Adjust based on your game's mechanics

        (shooting_direction, shooting_power)
    }

    /// Determine if forward should shoot even without a completely clear shot
    fn should_shoot_anyway(&self, ctx: &StateProcessingContext, distance_to_goal: f32) -> bool {
        // Very close to goal (inside 6-yard box) - always shoot
        if distance_to_goal < 60.0 {
            return true;
        }

        // Inside penalty area - shoot with good finishing skill
        if distance_to_goal < 165.0 {
            let finishing = ctx.player.skills.technical.finishing;
            let composure = ctx.player.skills.mental.composure;

            // Good finishers can score even with defenders nearby
            if finishing > 12.0 || composure > 14.0 {
                return true;
            }

            // Check how many opponents are very close (blocking)
            let close_blockers = ctx.players().opponents().nearby(5.0).count();
            // Only 1 blocker and decent skill - take the shot
            if close_blockers <= 1 && finishing > 10.0 {
                return true;
            }
        }

        // Edge of box - only shoot if good skills and minimal blocking
        if distance_to_goal < 200.0 {
            let finishing = ctx.player.skills.technical.finishing;
            let long_shots = ctx.player.skills.technical.long_shots;

            let close_blockers = ctx.players().opponents().nearby(4.0).count();
            if close_blockers == 0 && (finishing > 14.0 || long_shots > 14.0) {
                return true;
            }
        }

        false
    }
}
