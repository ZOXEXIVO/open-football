use crate::r#match::events::Event;
use crate::r#match::goalkeepers::states::common::{ActivityIntensity, GoalkeeperCondition};
use crate::r#match::goalkeepers::states::state::GoalkeeperState;
use crate::r#match::player::events::PlayerEvent;
use crate::r#match::{
    ConditionContext, StateChangeResult, StateProcessingContext, StateProcessingHandler,
};
use nalgebra::Vector3;

const MAX_DIVE_TIME: f32 = 1.8; // Maximum time to stay in diving state (in seconds)
const BALL_CLAIM_DISTANCE: f32 = 14.0;

#[derive(Default, Clone)]
pub struct GoalkeeperDivingState {}

impl StateProcessingHandler for GoalkeeperDivingState {
    fn try_fast(&self, ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        if ctx.player.has_ball(ctx) {
            return Some(StateChangeResult::with_goalkeeper_state(
                GoalkeeperState::Passing,
            ));
        }

        let ball_velocity = ctx.tick_context.positions.ball.velocity;
        let ball_moving_away = ball_velocity.dot(&(ctx.player.position - ctx.ball().direction_to_own_goal())) > 0.0;

        if ctx.ball().distance() > 100.0 && ball_moving_away {
            return Some(StateChangeResult::with_goalkeeper_state(
                GoalkeeperState::ReturningToGoal,
            ));
        }

        if ctx.in_state_time as f32 / 100.0 > MAX_DIVE_TIME {
            return Some(StateChangeResult::with_goalkeeper_state(
                GoalkeeperState::ReturningToGoal,
            ));
        }

        if self.is_ball_caught(ctx) {
            return Some(StateChangeResult::with_goalkeeper_state_and_event(
                GoalkeeperState::Standing,
                Event::PlayerEvent(PlayerEvent::CaughtBall(ctx.player.id)),
            ));
        } else if self.is_ball_nearby(ctx) {
            let mut result = StateChangeResult::with_goalkeeper_state(GoalkeeperState::Catching);
            result.events.add_player_event(PlayerEvent::ClaimBall(ctx.player.id));
            return Some(result);
        }

        if ctx.in_state_time > 90 {
            return Some(StateChangeResult::with_goalkeeper_state(
                GoalkeeperState::Standing,
            ));
        }

        None
    }

    fn process_slow(&self, _ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        None
    }

    fn velocity(&self, ctx: &StateProcessingContext) -> Option<Vector3<f32>> {
        let dive_direction = self.calculate_dive_direction(ctx);
        let dive_speed = self.calculate_dive_speed(ctx);

        Some(dive_direction * dive_speed)
    }

    fn process_conditions(&self, ctx: ConditionContext) {
        // Diving is a very high intensity activity requiring maximum energy expenditure
        GoalkeeperCondition::new(ActivityIntensity::VeryHigh).process(ctx);
    }
}

impl GoalkeeperDivingState {
    fn calculate_dive_direction(&self, ctx: &StateProcessingContext) -> Vector3<f32> {
        let ball_position = ctx.tick_context.positions.ball.position;
        let ball_velocity = ctx.tick_context.positions.ball.velocity;

        // Predict ball position based on reflexes (better GK = better prediction)
        let reflexes = ctx.player.skills.mental.concentration / 20.0;
        let anticipation = ctx.player.skills.mental.anticipation / 20.0;
        let prediction_time = 0.3 + (reflexes * 0.3 + anticipation * 0.2); // 0.3-0.8 seconds ahead
        let future_ball_position = ball_position + ball_velocity * prediction_time;

        let to_future_ball = future_ball_position - ctx.player.position;
        let mut dive_direction = to_future_ball.normalize();

        // Small randomness based on skill — elite GKs barely deviate
        // Squared so elite keepers are dramatically more accurate
        let skill_factor = (1.0 - reflexes) * (1.0 - reflexes);
        let max_deviation = skill_factor * std::f32::consts::PI / 10.0; // 0-18 degrees max, elite ~0
        let random_angle = (rand::random::<f32>() - 0.5) * max_deviation;
        dive_direction = nalgebra::Rotation3::new(Vector3::z() * random_angle) * dive_direction;

        dive_direction
    }

    fn calculate_dive_speed(&self, ctx: &StateProcessingContext) -> f32 {
        let urgency = self.calculate_urgency(ctx);
        let reflexes = ctx.player.skills.mental.concentration / 20.0;
        let agility = ctx.player.skills.physical.agility / 20.0;
        // Explosive dive speed — reflexes and agility are primary drivers
        let base_speed = (ctx.player.skills.physical.acceleration + ctx.player.skills.physical.agility) * 0.5;
        // Elite: 1.1 + 0.9 + 0.5 = 2.5x, mediocre: 1.1 + 0.41 + 0.23 = 1.74x
        let skill_boost = 1.1 + reflexes * 0.9 + agility * 0.5;
        base_speed * urgency * skill_boost
    }

    fn calculate_urgency(&self, ctx: &StateProcessingContext) -> f32 {
        let ball_position = ctx.tick_context.positions.ball.position;
        let ball_velocity = ctx.tick_context.positions.ball.velocity;

        let own_goal = ctx.ball().direction_to_own_goal();
        let distance_to_goal = (ball_position - own_goal).magnitude();
        let velocity_towards_goal = ball_velocity.dot(&(own_goal - ball_position).normalize()).max(0.0);

        // Scale for actual ball speeds (~1.0-2.0/tick)
        let urgency: f32 = (1.0 - distance_to_goal / 100.0) * (1.0 + velocity_towards_goal / 2.0);
        urgency.clamp(1.0, 2.5)
    }

    fn is_ball_caught(&self, ctx: &StateProcessingContext) -> bool {
        // Goalkeeper can only catch balls that are flying TOWARDS them or very close
        let ball_distance = ctx.ball().distance();
        if ball_distance > 5.0 && !ctx.ball().is_towards_player_with_angle(0.6) {
            return false;
        }

        let ball_speed = ctx.tick_context.positions.ball.velocity.magnitude();

        // Catch distance based on GK reach (diving extends range significantly)
        let handling = ctx.player.skills.technical.first_touch / 20.0;
        let agility = ctx.player.skills.physical.agility / 20.0;
        let reflexes = ctx.player.skills.mental.concentration / 20.0;
        // Elite GK: 7 + 7 + 3.5 + 2.5 = 20, mediocre: 7 + 3.2 + 1.6 + 1.1 = 12.9
        let catch_distance = 7.0 + agility * 7.0 + handling * 3.5 + reflexes * 2.5;

        if ball_distance > catch_distance {
            return false;
        }

        // Catch probability with strong skill differentiation
        // Elite GKs make saves that mediocre keepers cannot
        let skill_blend = handling * 0.4 + reflexes * 0.3 + agility * 0.3;
        // Speed penalty — elite reflexes mitigate fast shot difficulty
        let effective_speed_penalty = (ball_speed / 6.0).min(0.35) * (1.0 - reflexes * 0.55);
        // Elite: 0.15 + 1.0*0.82 = 0.97, mediocre: 0.15 + 0.47*0.82 = 0.54
        let base_catch = 0.15 + skill_blend * 0.82;
        let catch_probability = base_catch * (1.0 - effective_speed_penalty);

        // Elite GK vs fast shot: ~0.97 * 0.84 = 0.82
        // Mediocre GK vs fast shot: ~0.54 * 0.72 = 0.39
        rand::random::<f32>() < catch_probability.clamp(0.12, 0.97)
    }

    fn is_ball_nearby(&self, ctx: &StateProcessingContext) -> bool {
        ctx.ball().distance() < BALL_CLAIM_DISTANCE
    }
}