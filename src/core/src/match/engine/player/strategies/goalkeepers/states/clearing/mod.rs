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
#[derive(Default, Clone)]
pub struct GoalkeeperClearingState {}

impl StateProcessingHandler for GoalkeeperClearingState {
    fn process(&self, ctx: &StateProcessingContext) -> Option<StateChangeResult> {
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
    /// Execute a clearance — lofted hoof toward the halfway line.
    ///
    /// Old implementation used `MoveBall` with z=0, so the "clearance"
    /// was a ground roll that got intercepted 20m upfield. Now it emits
    /// a proper `ClearBall` event with significant vertical velocity so
    /// the ball flies over pressing opponents and lands in contested
    /// midfield. In-engine gravity is strong, so z needs to be ~5 u/tick
    /// for the ball to stay airborne through its horizontal travel.
    fn execute_clearance(&self, ctx: &StateProcessingContext) -> Option<Event> {
        use crate::r#match::PlayerSide;

        let kicking_power = ctx.player.skills.goalkeeping.kicking / 20.0;

        let field_width = ctx.context.field_size.width as f32;
        let field_height = ctx.context.field_size.height as f32;
        let halfway_x = field_width * 0.5;
        let mid_y = field_height * 0.5;

        let keeper_pos = ctx.player.position;

        // Target the halfway line, slightly off-centre (random to avoid
        // predictability). Clearances aim central-ish so they land where
        // the midfielders can contest them rather than near a sideline.
        let mut rng = rand::rng();
        let y_jitter: f32 = rng.random_range(-field_height * 0.15..field_height * 0.15);
        let target_y = mid_y + y_jitter;

        // Direction always upfield — away from own goal, toward opponent half.
        let target_x = match ctx.player.side {
            Some(PlayerSide::Left) => halfway_x, // home kicks toward x = halfway
            Some(PlayerSide::Right) => halfway_x, // away kicks toward x = halfway (same spot)
            None => halfway_x,
        };

        let horizontal_to_target =
            Vector3::new(target_x - keeper_pos.x, target_y - keeper_pos.y, 0.0);
        let horizontal_dist = horizontal_to_target.norm().max(0.1);
        let horizontal_dir = horizontal_to_target / horizontal_dist;

        // Horizontal speed scaled by kicking skill.
        let horizontal_speed = 3.8 + kicking_power * 1.0; // 3.8 - 4.8 u/tick
        let horizontal_velocity = horizontal_dir * horizontal_speed;

        // Lofted z — strong vertical so the ball flies over the defensive
        // line and clears the danger zone. Skilled keepers loft it a
        // touch higher.
        let z_velocity = 4.5 + kicking_power * 1.0; // 4.5 - 5.5 u/tick

        let ball_velocity = Vector3::new(horizontal_velocity.x, horizontal_velocity.y, z_velocity);

        Some(Event::PlayerEvent(PlayerEvent::ClearBall(ball_velocity)))
    }
}
