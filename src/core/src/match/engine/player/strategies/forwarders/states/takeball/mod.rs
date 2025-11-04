use crate::r#match::forwarders::states::ForwardState;
use crate::r#match::{
    ConditionContext, StateChangeResult, StateProcessingContext, StateProcessingHandler,
    SteeringBehavior,
};
use nalgebra::Vector3;

const TAKEBALL_TIMEOUT: u64 = 200; // Give up after 200 ticks (~3.3 seconds)
const MAX_TAKEBALL_DISTANCE: f32 = 500.0; // Don't chase balls further than this - increased to ensure someone always goes

#[derive(Default)]
pub struct ForwardTakeBallState {}

impl StateProcessingHandler for ForwardTakeBallState {
    fn try_fast(&self, ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        // Check if ball is now owned by someone
        if ctx.ball().is_owned() {
            return Some(StateChangeResult::with_forward_state(ForwardState::Running));
        }

        let ball_distance = ctx.ball().distance();
        let ball_position = ctx.tick_context.positions.ball.landing_position;

        // 1. Timeout check - give up after too long
        if ctx.in_state_time > TAKEBALL_TIMEOUT {
            return Some(StateChangeResult::with_forward_state(
                ForwardState::Running,
            ));
        }

        // 2. Distance check - ball too far away
        if ball_distance > MAX_TAKEBALL_DISTANCE {
            return Some(StateChangeResult::with_forward_state(
                ForwardState::Running,
            ));
        }

        // 3. Check if opponent will reach ball first
        if let Some(closest_opponent) = ctx.players().opponents().all().min_by(|a, b| {
            let dist_a = (a.position - ball_position).magnitude();
            let dist_b = (b.position - ball_position).magnitude();
            dist_a.partial_cmp(&dist_b).unwrap()
        }) {
            let opponent_distance = (closest_opponent.position - ball_position).magnitude();

            // If opponent is significantly closer (by 20+ units), give up and press
            if opponent_distance < ball_distance - 20.0 {
                return Some(StateChangeResult::with_forward_state(
                    ForwardState::Pressing,
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
                return Some(StateChangeResult::with_forward_state(
                    ForwardState::CreatingSpace,
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
        // BUT reduce separation MUCH more aggressively when close to ball
        const SEPARATION_RADIUS: f32 = 25.0;
        const SEPARATION_WEIGHT: f32 = 0.25; // Reduced from 0.4
        const BALL_CLAIM_DISTANCE: f32 = 15.0; // Increased from 10.0
        const BALL_PRIORITY_DISTANCE: f32 = 5.0; // New: disable separation when very close

        let distance_to_ball = (ctx.player.position - target).magnitude();

        // Much more aggressive separation reduction
        let separation_factor = if distance_to_ball < BALL_PRIORITY_DISTANCE {
            // Within claiming distance - almost no separation (0-5% depending on distance)
            (distance_to_ball / BALL_PRIORITY_DISTANCE) * 0.05
        } else if distance_to_ball < BALL_CLAIM_DISTANCE {
            // Approaching claim distance - reduced separation (5-30%)
            let ratio = (distance_to_ball - BALL_PRIORITY_DISTANCE) / (BALL_CLAIM_DISTANCE - BALL_PRIORITY_DISTANCE);
            0.05 + (ratio * 0.25)
        } else {
            // Far from ball - normal separation
            0.30
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
