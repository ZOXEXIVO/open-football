use crate::r#match::defenders::states::DefenderState;
use crate::r#match::{
    ConditionContext, StateChangeResult, StateProcessingContext, StateProcessingHandler,
    SteeringBehavior,
};
use nalgebra::Vector3;

const TAKEBALL_TIMEOUT: u64 = 1000; // Give up after 1000 ticks
const MAX_TAKEBALL_DISTANCE: f32 = 500.0; // Don't chase balls further than this - increased to ensure someone always goes

#[derive(Default)]
pub struct DefenderTakeBallState {}

impl StateProcessingHandler for DefenderTakeBallState {
    fn try_fast(&self, ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        // Check if ball is now owned by someone
        if ctx.ball().is_owned() {
            return Some(StateChangeResult::with_defender_state(
                DefenderState::Running,
            ));
        }

        let ball_distance = ctx.ball().distance();
        let ball_position = ctx.tick_context.positions.ball.landing_position;

        // 1. Distance check - ball too far away
        if ball_distance > MAX_TAKEBALL_DISTANCE {
            return Some(StateChangeResult::with_defender_state(
                DefenderState::Returning,
            ));
        }

        // 2. Check if opponent will reach ball first
        if let Some(closest_opponent) = ctx.players().opponents().all().min_by(|a, b| {
            let dist_a = (a.position - ball_position).magnitude();
            let dist_b = (b.position - ball_position).magnitude();
            dist_a.partial_cmp(&dist_b).unwrap()
        }) {
            let opponent_distance = (closest_opponent.position - ball_position).magnitude();

            // If opponent is significantly closer (by 20+ units), give up and prepare to defend
            if opponent_distance < ball_distance - 20.0 {
                return Some(StateChangeResult::with_defender_state(
                    DefenderState::Pressing,
                ));
            }
        }

        // 3. Check if teammate is closer to the ball
        if let Some(closest_teammate) = ctx.players().teammates().all().filter(|t| t.id != ctx.player.id).min_by(|a, b| {
            let dist_a = (a.position - ball_position).magnitude();
            let dist_b = (b.position - ball_position).magnitude();
            dist_a.partial_cmp(&dist_b).unwrap()
        }) {
            let teammate_distance = (closest_teammate.position - ball_position).magnitude();

            // If teammate is significantly closer (by 15+ units), let them take it
            if teammate_distance < ball_distance - 15.0 {
                return Some(StateChangeResult::with_defender_state(
                    DefenderState::HoldingLine,
                ));
            }
        }

        // Continue trying to take the ball
        None
    }

    fn process_slow(&self, _ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        None
    }

    fn velocity(&self, ctx: &StateProcessingContext) -> Option<Vector3<f32>> {
        // If ball is aerial, target the landing position instead of current position
        let target = ctx.tick_context.positions.ball.landing_position;

        // Calculate base Arrive behavior
        let mut arrive_velocity = SteeringBehavior::Arrive {
            target,
            slowing_distance: 15.0,
        }
        .calculate(ctx.player)
        .velocity;

        // Add separation force to prevent player stacking
        // BUT reduce separation when very close to ball to allow claiming
        const SEPARATION_RADIUS: f32 = 25.0;
        const SEPARATION_WEIGHT: f32 = 0.4;
        const BALL_CLAIM_DISTANCE: f32 = 10.0; // Reduce separation within this distance to ball

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

    fn process_conditions(&self, _ctx: ConditionContext) {}
}
