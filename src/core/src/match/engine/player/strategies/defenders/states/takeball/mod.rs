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

        // Aim point crosses smoothly from the ball itself to where it will
        // land as it rises, instead of snapping between them at a fixed
        // height.
        //
        // This used to be `if ball_pos.z > 2.3 { landing } else { ball_pos }`.
        // A bouncing ball crosses 2.3 repeatedly, and the two targets can
        // be tens of units apart in DIFFERENT directions — so the chaser's
        // velocity inverted on every crossing. `Defender: Take Ball`
        // measured 6.6-7.8 velocity reversals per second held with the
        // player never leaving the state (`dev_match trace`), which is a
        // chaser visibly shivering next to a loose ball instead of
        // collecting it. Blending across a band means there is no height
        // at which the aim point can jump.
        const GROUND_H: f32 = 1.5;
        const AERIAL_H: f32 = 3.0;
        let t = ((ball_pos.z - GROUND_H) / (AERIAL_H - GROUND_H)).clamp(0.0, 1.0);
        // Smoothstep: zero gradient at both ends, so the aim point has no
        // corner where it starts or finishes moving.
        let aerial = t * t * (3.0 - 2.0 * t);
        let target = ball_pos + (landing - ball_pos) * aerial;

        // Same treatment for the behaviour itself: `Arrive` brakes into a
        // landing spot, `Pursuit` leads a rolling ball, and switching
        // between them at a threshold is its own discontinuity.
        let mut arrive_velocity = if aerial >= 1.0 {
            SteeringBehavior::Arrive {
                target,
                slowing_distance: 10.0,
            }
            .calculate(ctx.player)
            .velocity
        } else if aerial <= 0.0 {
            SteeringBehavior::Pursuit {
                target,
                target_velocity: ball_vel,
            }
            .calculate(ctx.player)
            .velocity
        } else {
            let brake = SteeringBehavior::Arrive {
                target,
                slowing_distance: 10.0,
            }
            .calculate(ctx.player)
            .velocity;
            let lead = SteeringBehavior::Pursuit {
                target,
                target_velocity: ball_vel,
            }
            .calculate(ctx.player)
            .velocity;
            lead * (1.0 - aerial) + brake * aerial
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
        let players_view = ctx.players();
        let teammates_view = players_view.teammates();
        let opponents_view = players_view.opponents();
        let all_players = teammates_view
            .all()
            .chain(opponents_view.all())
            .filter(|p| p.id != ctx.player.id);

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
                * ctx.player.max_speed_with_condition_cached()
                * SEPARATION_WEIGHT
                * separation_factor;

            // Blend arrive and separation velocities
            arrive_velocity = arrive_velocity + separation_force;

            // Limit to max speed
            let magnitude = arrive_velocity.magnitude();
            if magnitude > ctx.player.max_speed_with_condition_cached() {
                arrive_velocity =
                    arrive_velocity * (ctx.player.max_speed_with_condition_cached() / magnitude);
            }
        }

        Some(arrive_velocity)
    }

    fn process_conditions(&self, ctx: ConditionContext) {
        // Taking ball involves movement towards ball - moderate intensity
        DefenderCondition::with_velocity(ActivityIntensity::Moderate).process(ctx);
    }
}
