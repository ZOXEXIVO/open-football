use crate::r#match::events::Event;
use crate::r#match::goalkeepers::states::common::{ActivityIntensity, GoalkeeperCondition};
use crate::r#match::goalkeepers::states::state::GoalkeeperState;
use crate::r#match::player::events::PlayerEvent;
use crate::r#match::{
    ConditionContext, StateChangeResult, StateProcessingContext, StateProcessingHandler,
};
use nalgebra::Vector3;
use rand::RngExt;

/// Goalkeeper clearing state - emergency clearance of the ball away from danger
#[derive(Default)]
pub struct GoalkeeperClearingState {}

impl StateProcessingHandler for GoalkeeperClearingState {
    fn try_fast(&self, ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        // If we don't have the ball anymore, return to standing
        if !ctx.player.has_ball(ctx) {
            return Some(StateChangeResult::with_goalkeeper_state(
                GoalkeeperState::Standing,
            ));
        }

        // Execute the clearance kick
        if let Some(event) = self.execute_clearance(ctx) {
            return Some(StateChangeResult::with_goalkeeper_state_and_event(
                GoalkeeperState::Standing,
                event,
            ));
        }

        None
    }

    fn process_slow(&self, _ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        None
    }

    fn velocity(&self, _ctx: &StateProcessingContext) -> Option<Vector3<f32>> {
        // Stand still while preparing to clear
        Some(Vector3::new(0.0, 0.0, 0.0))
    }

    fn process_conditions(&self, ctx: ConditionContext) {
        // Clearing requires moderate intensity with focused effort
        GoalkeeperCondition::new(ActivityIntensity::Moderate).process(ctx);
    }
}

impl GoalkeeperClearingState {
    /// Execute a clearance - boot the ball away from danger
    fn execute_clearance(&self, ctx: &StateProcessingContext) -> Option<Event> {
        let kicking_power = ctx.player.skills.technical.long_throws / 20.0;

        // Calculate clearance target - aim for sideline or upfield
        let field_width = ctx.context.field_size.width as f32;
        let field_height = ctx.context.field_size.height as f32;

        let keeper_pos = ctx.player.position;

        // Determine which direction to clear based on position
        let mut rng = rand::rng();
        let random_factor: f32 = rng.random_range(-0.3..0.3);

        // Aim for a moderate distance upfield and toward sideline
        let target_x = keeper_pos.x + (field_width * 0.4); // 40% of field upfield
        let target_y = if keeper_pos.y > 0.0 {
            field_height * 0.35 + random_factor * 15.0 // Toward top sideline
        } else {
            -field_height * 0.35 + random_factor * 15.0 // Toward bottom sideline
        };

        let clearance_target = Vector3::new(target_x, target_y, 0.0);

        // Moderate power clearance
        let kick_force = 5.0 + (kicking_power * 1.5); // 5.0-6.5 range

        // Use MoveBall event for clearance
        let ball_direction = (clearance_target - keeper_pos).normalize();
        let ball_velocity = ball_direction * kick_force * 2.5;

        Some(Event::PlayerEvent(PlayerEvent::MoveBall(
            ctx.player.id,
            ball_velocity,
        )))
    }
}
