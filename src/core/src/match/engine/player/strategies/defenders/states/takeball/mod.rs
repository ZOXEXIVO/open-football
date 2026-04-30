use crate::r#match::defenders::states::DefenderState;
use crate::r#match::defenders::states::common::{ActivityIntensity, DefenderCondition};
use crate::r#match::{
    ConditionContext, StateChangeResult, StateProcessingContext, StateProcessingHandler,
    SteeringBehavior,
};
use nalgebra::Vector3;

#[derive(Default, Clone)]
pub struct DefenderTakeBallState {}

impl StateProcessingHandler for DefenderTakeBallState {
    fn process(&self, ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        // WE own the ball → TakeBall is the wrong state. Drop to
        // Running so the ball-on-foot paths can clear / pass / dribble.
        if ctx.player.has_ball(ctx) {
            return Some(StateChangeResult::with_defender_state(
                DefenderState::Running,
            ));
        }
        // Ball got claimed. Running handles teammate/opponent ownership —
        // hand off there instead of duplicating the dispatch here.
        if ctx.ball().is_owned() {
            return Some(StateChangeResult::with_defender_state(
                DefenderState::Running,
            ));
        }

        // Ball is loose: commit. No distance cap, no teammate yield, no
        // "opponent is closer" bailout. The Running state's
        // `is_best_player_to_chase_ball` already committed this player.
        // Spatial-proximity checks against stationary rivals created
        // stalemates where nobody actually went for the ball.
        None
    }

    fn velocity(&self, ctx: &StateProcessingContext) -> Option<Vector3<f32>> {
        // Aerial balls: Arrive to the landing spot so we brake into the
        // claim radius instead of plowing through it at full speed.
        // Ground balls: Pursuit, which already has its own slowing ramp
        // and uses the ball's velocity to predict interception.
        // Seek alone would chase a moving ball's *current* position and
        // always lag behind — fatal for a ground pass rolling through us.
        let ball_pos = ctx.tick_context.positions.ball.position;
        let ball_vel = ctx.tick_context.positions.ball.velocity;
        let landing = ctx.tick_context.positions.ball.landing_position;
        let is_aerial = ball_pos.z > 2.3;
        let target = if is_aerial { landing } else { ball_pos };

        let mut arrive_velocity = if is_aerial {
            SteeringBehavior::Arrive {
                target,
                slowing_distance: 10.0,
            }
            .calculate(ctx.player)
            .velocity
        } else {
            SteeringBehavior::Pursuit {
                target,
                target_velocity: ball_vel,
            }
            .calculate(ctx.player)
            .velocity
        };

        // Add separation force to prevent player stacking
        // Reduce separation when approaching ball, but keep minimum to prevent clustering
        const SEPARATION_RADIUS: f32 = 25.0;
        const SEPARATION_WEIGHT: f32 = 0.4;
        const BALL_CLAIM_DISTANCE: f32 = 10.0;
        const NO_SEPARATION_DISTANCE: f32 = 5.0; // Completely disable separation within this distance

        let distance_to_ball = (ctx.player.position - target).magnitude();
        let separation_factor = if distance_to_ball < NO_SEPARATION_DISTANCE {
            0.0 // No separation at all — let the player reach the ball
        } else if distance_to_ball < BALL_CLAIM_DISTANCE {
            let linear_factor = (distance_to_ball - NO_SEPARATION_DISTANCE)
                / (BALL_CLAIM_DISTANCE - NO_SEPARATION_DISTANCE);
            linear_factor * 0.3 // Gentle ramp from 0 to 0.3
        } else {
            1.0
        };

        let mut separation_force = Vector3::zeros();
        let mut neighbor_count = 0;

        // Check all nearby players (teammates and opponents)
        let all_players: Vec<_> = ctx
            .players()
            .teammates()
            .all()
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
            separation_force = separation_force
                * ctx
                    .player
                    .skills
                    .max_speed_with_condition(ctx.player.player_attributes.condition)
                * SEPARATION_WEIGHT
                * separation_factor;

            // Blend arrive and separation velocities
            arrive_velocity = arrive_velocity + separation_force;

            // Limit to max speed
            let magnitude = arrive_velocity.magnitude();
            if magnitude
                > ctx
                    .player
                    .skills
                    .max_speed_with_condition(ctx.player.player_attributes.condition)
            {
                arrive_velocity = arrive_velocity
                    * (ctx
                        .player
                        .skills
                        .max_speed_with_condition(ctx.player.player_attributes.condition)
                        / magnitude);
            }
        }

        Some(arrive_velocity)
    }

    fn process_conditions(&self, ctx: ConditionContext) {
        // Taking ball involves movement towards ball - moderate intensity
        DefenderCondition::with_velocity(ActivityIntensity::Moderate).process(ctx);
    }
}
