use crate::r#match::goalkeepers::states::common::{ActivityIntensity, GoalkeeperCondition};
use crate::r#match::goalkeepers::states::state::GoalkeeperState;
use crate::r#match::{
    ConditionContext, StateChangeResult, StateProcessingContext, StateProcessingHandler,
    SteeringBehavior,
};
use nalgebra::Vector3;

#[derive(Default)]
pub struct GoalkeeperTakeBallState {}

impl StateProcessingHandler for GoalkeeperTakeBallState {
    fn try_fast(&self, ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        if ctx.player.has_ball(ctx) {
            return Some(StateChangeResult::with_goalkeeper_state(
                GoalkeeperState::ReturningToGoal,
            ));
        }

        None
    }

    fn process_slow(&self, _ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        None
    }

    fn velocity(&self, ctx: &StateProcessingContext) -> Option<Vector3<f32>> {
        // Calculate base Arrive behavior
        let mut arrive_velocity = SteeringBehavior::Arrive {
            target: ctx.tick_context.positions.ball.position,
            slowing_distance: 15.0,
        }
        .calculate(ctx.player)
        .velocity;

        // Add separation force to prevent player stacking
        // BUT reduce separation when very close to ball to allow claiming
        const SEPARATION_RADIUS: f32 = 25.0;
        const SEPARATION_WEIGHT: f32 = 0.4;
        const BALL_CLAIM_DISTANCE: f32 = 6.7; // Reduced by 1.5x from 10.0

        let target = ctx.tick_context.positions.ball.position;
        let distance_to_ball = (ctx.player.position - target).magnitude();
        let separation_factor = if distance_to_ball < BALL_CLAIM_DISTANCE {
            // Reduce separation force when close to ball (linear falloff)
            distance_to_ball / BALL_CLAIM_DISTANCE
        } else {
            1.0
        };

        let mut separation_force = Vector3::zeros();
        let mut neighbor_count = 0;

        // Check all nearby players (teammates and opponents)
        let all_players: Vec<_> = ctx.players().teammates().all()
            .chain(ctx.players().opponents().all())
            .filter(|p| p.id != ctx.player.id)
            .collect();

        for other_player in all_players {
            let to_player = ctx.player.position - other_player.position;
            let distance = to_player.magnitude();

            if distance > 0.0 && distance < SEPARATION_RADIUS {
                // Repulsive force inversely proportional to distance
                let repulsion_strength = (SEPARATION_RADIUS - distance) / SEPARATION_RADIUS;
                separation_force += to_player.normalize() * repulsion_strength;
                neighbor_count += 1;
            }
        }

        if neighbor_count > 0 {
            // Average and scale the separation force
            separation_force = separation_force / (neighbor_count as f32);
            separation_force = separation_force * ctx.player.skills.max_speed() * SEPARATION_WEIGHT * separation_factor;

            // Blend arrive and separation velocities
            arrive_velocity = arrive_velocity + separation_force;

            // Limit to max speed
            let magnitude = arrive_velocity.magnitude();
            if magnitude > ctx.player.skills.max_speed() {
                arrive_velocity = arrive_velocity * (ctx.player.skills.max_speed() / magnitude);
            }
        }

        Some(arrive_velocity)
    }

    fn process_conditions(&self, ctx: ConditionContext) {
        // Taking ball requires high intensity as goalkeeper moves to claim the ball
        GoalkeeperCondition::with_velocity(ActivityIntensity::High).process(ctx);
    }
}
