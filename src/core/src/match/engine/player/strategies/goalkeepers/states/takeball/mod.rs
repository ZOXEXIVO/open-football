use crate::club::player::skills::GoalkeeperSpeedContext;
use crate::r#match::goalkeepers::states::common::{ActivityIntensity, GoalkeeperCondition};
use crate::r#match::goalkeepers::states::state::GoalkeeperState;
use crate::r#match::player::strategies::common::states::LooseBallChase;
use crate::r#match::{
    ConditionContext, StateChangeResult, StateProcessingContext, StateProcessingHandler,
    SteeringBehavior,
};
use nalgebra::Vector3;

#[derive(Default, Clone)]
pub struct GoalkeeperTakeBallState {}

impl StateProcessingHandler for GoalkeeperTakeBallState {
    fn process(&self, ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        if ctx.player.has_ball(ctx) {
            return Some(StateChangeResult::with_goalkeeper_state(
                GoalkeeperState::ReturningToGoal,
            ));
        }

        // Transition to Catching when ball is very close and not owned
        if ctx.ball().distance() < 3.0 && !ctx.ball().is_owned() {
            return Some(StateChangeResult::with_goalkeeper_state(
                GoalkeeperState::Catching,
            ));
        }

        if ctx.ball().is_owned() {
            return Some(StateChangeResult::with_goalkeeper_state(
                GoalkeeperState::ReturningToGoal,
            ));
        }

        // Timeout after 120 ticks — but only if ball isn't very close
        // If ball is close, keep trying instead of giving up
        if ctx.in_state_time > 120 && ctx.ball().distance() > 10.0 {
            return Some(StateChangeResult::with_goalkeeper_state(
                GoalkeeperState::Standing,
            ));
        }

        // Hard timeout after 200 ticks regardless
        if ctx.in_state_time > 200 {
            return Some(StateChangeResult::with_goalkeeper_state(
                GoalkeeperState::Standing,
            ));
        }

        None
    }

    fn velocity(&self, ctx: &StateProcessingContext) -> Option<Vector3<f32>> {
        // Use Seek for full-speed approach - no slowing when chasing a loose ball
        let target = ctx.tick_context.positions.ball.position;

        // A keeper sprinting off their line to claim a loose ball moves
        // like one rushing out, not like one shuffling along the six-yard
        // box. `Seek` caps at the OUTFIELD base speed, and the movement
        // integrator then applies the goalkeeper `Active` ceiling — so
        // without scaling here the ceiling can never bind and the keeper
        // arrives at walking pace. Scale by the same agility/acceleration
        // profile the ceiling is derived from, capped so this stays a
        // ceiling-filling multiplier and never outruns it.
        let gk_ceiling = ctx.player.skills.goalkeeper_max_speed(
            ctx.player.player_attributes.condition,
            GoalkeeperSpeedContext::Active,
        );
        let base_speed = ctx.player.max_speed_with_condition_cached();
        let urgency = if base_speed > 0.0 {
            (gk_ceiling / base_speed).clamp(1.0, 1.5)
        } else {
            1.0
        };
        let max_speed = base_speed * urgency;

        let mut arrive_velocity = SteeringBehavior::Seek { target }
            .calculate(ctx.player)
            .velocity
            * urgency;

        // Add separation force to prevent player stacking
        // BUT reduce separation when very close to ball to allow claiming
        const SEPARATION_RADIUS: f32 = 25.0;
        const SEPARATION_WEIGHT: f32 = 0.4;
        const BALL_CLAIM_DISTANCE: f32 = 10.0;
        const NO_SEPARATION_DISTANCE: f32 = 5.0;

        let distance_to_ball = (ctx.player.position - target).magnitude();
        let separation_factor = if distance_to_ball < NO_SEPARATION_DISTANCE {
            0.0 // No separation at all — let the keeper reach the ball
        } else if distance_to_ball < BALL_CLAIM_DISTANCE {
            let linear_factor = (distance_to_ball - NO_SEPARATION_DISTANCE)
                / (BALL_CLAIM_DISTANCE - NO_SEPARATION_DISTANCE);
            linear_factor * 0.3
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
            separation_force = separation_force * max_speed * SEPARATION_WEIGHT * separation_factor;

            // ⚠ SEPARATION MUST NEVER SLOW THE RACE — see `LooseBallChase`.
            // A keeper coming for a loose ball is the last man; being
            // repelled off it by the striker he is racing is the worst
            // version of this bug on the pitch.
            separation_force =
                LooseBallChase::keep_non_opposing(separation_force, target - ctx.player.position);

            // Blend arrive and separation velocities
            arrive_velocity = arrive_velocity + separation_force;

            // Limit to the keeper's own chase ceiling — NOT the outfield
            // base, which would undo the urgency scaling above the moment
            // any other player came within the separation radius.
            let magnitude = arrive_velocity.magnitude();
            if magnitude > max_speed {
                arrive_velocity *= max_speed / magnitude;
            }
        }

        Some(arrive_velocity)
    }

    fn process_conditions(&self, ctx: ConditionContext) {
        // Taking ball requires high intensity as goalkeeper moves to claim the ball
        GoalkeeperCondition::with_velocity(ActivityIntensity::High).process(ctx);
    }
}
