use crate::r#match::defenders::states::DefenderState;
use crate::r#match::defenders::states::common::{DefenderCondition, ActivityIntensity};
use crate::r#match::{
    ConditionContext, StateChangeResult, StateProcessingContext, StateProcessingHandler,
    SteeringBehavior,
};
use nalgebra::Vector3;

// TakeBall distance and threshold constants
const MAX_TAKEBALL_DISTANCE: f32 = 500.0;
const OPPONENT_ADVANTAGE_THRESHOLD: f32 = 20.0; // Opponent must be this much closer to give up
const TEAMMATE_ADVANTAGE_THRESHOLD: f32 = 8.0; // Teammate must be this much closer to give up (reduced from 15.0)

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

            // If opponent is significantly closer, give up and prepare to defend
            if opponent_distance < ball_distance - OPPONENT_ADVANTAGE_THRESHOLD {
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

            // If teammate is significantly closer, let them take it (stricter threshold)
            if teammate_distance < ball_distance - TEAMMATE_ADVANTAGE_THRESHOLD {
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
        // Reduce separation when approaching ball, but keep minimum to prevent clustering
        const SEPARATION_RADIUS: f32 = 25.0;
        const SEPARATION_WEIGHT: f32 = 0.5; // Increased from 0.4 for stronger separation
        const BALL_CLAIM_DISTANCE: f32 = 10.0;
        const MIN_SEPARATION_FACTOR: f32 = 0.25; // Minimum 25% separation - allows closer approach with larger claiming radius

        let distance_to_ball = (ctx.player.position - target).magnitude();
        let separation_factor = if distance_to_ball < BALL_CLAIM_DISTANCE {
            // Reduce separation force when close to ball, but never below 25%
            let linear_factor = distance_to_ball / BALL_CLAIM_DISTANCE;
            MIN_SEPARATION_FACTOR + (linear_factor * (1.0 - MIN_SEPARATION_FACTOR))
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
            separation_force = separation_force * ctx.player.skills.max_speed_with_condition(
                ctx.player.player_attributes.condition,
                ctx.player.player_attributes.fitness,
                ctx.player.player_attributes.jadedness,
            ) * SEPARATION_WEIGHT * separation_factor;

            // Blend arrive and separation velocities
            arrive_velocity = arrive_velocity + separation_force;

            // Limit to max speed
            let magnitude = arrive_velocity.magnitude();
            if magnitude > ctx.player.skills.max_speed_with_condition(
                ctx.player.player_attributes.condition,
                ctx.player.player_attributes.fitness,
                ctx.player.player_attributes.jadedness,
            ) {
                arrive_velocity = arrive_velocity * (ctx.player.skills.max_speed_with_condition(
                    ctx.player.player_attributes.condition,
                    ctx.player.player_attributes.fitness,
                    ctx.player.player_attributes.jadedness,
                ) / magnitude);
            }
        }

        Some(arrive_velocity)
    }

    fn process_conditions(&self, ctx: ConditionContext) {
        // Taking ball involves movement towards ball - moderate intensity
        DefenderCondition::with_velocity(ActivityIntensity::Moderate).process(ctx);
    }
}
