use crate::r#match::goalkeepers::states::common::{ActivityIntensity, GoalkeeperCondition};
use crate::r#match::goalkeepers::states::state::GoalkeeperState;
use crate::r#match::player::events::PlayerEvent;
use crate::r#match::{ConditionContext, PlayerDistanceFromStartPosition, StateChangeResult, StateProcessingContext, StateProcessingHandler, SteeringBehavior};
use nalgebra::Vector3;

#[derive(Default, Clone)]
pub struct GoalkeeperCatchingState {}

impl StateProcessingHandler for GoalkeeperCatchingState {
    fn try_fast(&self, ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        if self.is_catch_successful(ctx) {
            let mut holding_result =
                StateChangeResult::with_goalkeeper_state(GoalkeeperState::HoldingBall);

            holding_result
                .events
                .add_player_event(PlayerEvent::CaughtBall(ctx.player.id));

            return Some(holding_result);
        }

        // If ball is moving away (not towards with angle 0.6 and speed > 2.0), give up
        let ball_speed = ctx.tick_context.positions.ball.velocity.norm();
        if ball_speed > 2.0 && !ctx.ball().is_towards_player_with_angle(0.6) {
            return Some(StateChangeResult::with_goalkeeper_state(
                GoalkeeperState::Standing,
            ));
        }

        // If ball is too far, decide based on distance from goal
        if ctx.ball().distance() > 12.0 {
            // If already far from goal, return rather than chasing further
            if ctx.player().distance_from_start_position() > 40.0 {
                return Some(StateChangeResult::with_goalkeeper_state(
                    GoalkeeperState::ReturningToGoal,
                ));
            }
            return Some(StateChangeResult::with_goalkeeper_state(
                GoalkeeperState::ComingOut,
            ));
        }

        if ctx.player().position_to_distance() == PlayerDistanceFromStartPosition::Big {
            return Some(StateChangeResult::with_goalkeeper_state(
                GoalkeeperState::ReturningToGoal,
            ))
        }

        if ctx.in_state_time > 30 {
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
        let ball_distance = ctx.ball().distance();
        let agility = ctx.player.skills.physical.agility / 20.0;

        // GK sprints explosively to catch the ball
        let speed_boost = 1.5 + agility * 0.5; // 1.5x - 2.0x

        if ball_distance > 3.0 {
            // Sprint to ball using Pursuit with speed boost
            Some(
                SteeringBehavior::Pursuit {
                    target: ctx.tick_context.positions.ball.position,
                    target_velocity: ctx.tick_context.positions.ball.velocity,
                }
                .calculate(ctx.player)
                .velocity * speed_boost,
            )
        } else {
            // Close - use Arrive for controlled approach but still fast
            Some(
                SteeringBehavior::Arrive {
                    target: ctx.tick_context.positions.ball.position,
                    slowing_distance: 1.5,
                }
                .calculate(ctx.player)
                .velocity * (speed_boost * 0.8),
            )
        }
    }

    fn process_conditions(&self, ctx: ConditionContext) {
        // Catching is a moderate intensity activity requiring focused effort
        GoalkeeperCondition::new(ActivityIntensity::Moderate).process(ctx);
    }
}

impl GoalkeeperCatchingState {
    fn is_catch_successful(&self, ctx: &StateProcessingContext) -> bool {
        // Maximum catch distance — goalkeeper's full reaching/stretching range
        const MAX_CATCH_DISTANCE: f32 = 12.0; // Extended reach including stretch
        let distance_to_ball = ctx.ball().distance();

        if distance_to_ball > MAX_CATCH_DISTANCE {
            return false; // Ball too far away to physically catch
        }

        // Goalkeeper can only catch balls that are flying TOWARDS them or are stationary/slow
        let ball_speed = ctx.tick_context.positions.ball.velocity.norm();
        if ball_speed > 0.5 && !ctx.ball().is_towards_player_with_angle(0.6) {
            return false; // Ball is flying away from goalkeeper
        }

        // Use goalkeeper-specific skills
        let handling = ctx.player.skills.technical.first_touch;
        let reflexes = ctx.player.skills.mental.concentration;
        let positioning = ctx.player.skills.technical.technique;
        let agility = ctx.player.skills.physical.agility;

        // Scale skills from 1-20 range to 0-1 range
        let scaled_handling = (handling - 1.0) / 19.0;
        let scaled_reflexes = (reflexes - 1.0) / 19.0;
        let scaled_positioning = (positioning - 1.0) / 19.0;
        let scaled_agility = (agility - 1.0) / 19.0;

        // Base catch skill (weighted toward handling and reflexes)
        let base_skill = scaled_handling * 0.4 + scaled_reflexes * 0.3 +
                          scaled_positioning * 0.2 + scaled_agility * 0.1;

        let ball_height = ctx.tick_context.positions.ball.position.z;

        // Base success rate calibrated for real shot speeds (~1.0-2.0/tick)
        // In real football, GKs save ~70% of shots on target
        let mut catch_probability = 0.60 + (base_skill * 0.35);

        // Ball speed modifier calibrated for actual speeds
        if ball_speed < 0.8 {
            catch_probability += 0.15; // Very slow - easy catch
        } else if ball_speed < 1.2 {
            catch_probability += 0.08; // Moderate speed
        } else if ball_speed > 1.8 {
            catch_probability -= 0.10; // Strong shot - harder
        }

        // Distance modifier
        if distance_to_ball < 2.0 {
            catch_probability += 0.15; // Very close - easy
        } else if distance_to_ball < 5.0 {
            catch_probability += 0.08; // Close
        } else if distance_to_ball > 10.0 {
            catch_probability -= 0.12; // Stretched
        } else if distance_to_ball > 7.0 {
            catch_probability -= 0.06; // Far
        }

        // Height modifier
        if ball_height >= 0.5 && ball_height <= 1.8 {
            catch_probability += 0.08; // Ideal catching height
        } else if ball_height < 0.2 {
            catch_probability -= 0.06; // Ground ball
        } else if ball_height > 2.5 {
            catch_probability -= 0.10; // High ball
        }

        // Direction modifier
        if ctx.ball().is_towards_player_with_angle(0.7) {
            catch_probability += 0.08;
        } else {
            catch_probability -= 0.08;
        }

        // Elite keeper bonus
        if base_skill > 0.8 {
            catch_probability += 0.06;
        }

        let clamped_catch_probability = catch_probability.clamp(0.20, 0.95);

        rand::random::<f32>() < clamped_catch_probability
    }
}
