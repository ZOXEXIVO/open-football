use crate::r#match::goalkeepers::states::common::{ActivityIntensity, GoalkeeperCondition};
use crate::r#match::goalkeepers::states::state::GoalkeeperState;
use crate::r#match::player::events::PlayerEvent;
use crate::r#match::{ConditionContext, PlayerDistanceFromStartPosition, StateChangeResult, StateProcessingContext, StateProcessingHandler, SteeringBehavior};
use nalgebra::Vector3;

#[derive(Default)]
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

        // If ball is too far, transition to ComingOut
        if ctx.ball().distance() > 8.0 {
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

        if ball_distance > 3.0 {
            // Sprint to ball using Pursuit
            Some(
                SteeringBehavior::Pursuit {
                    target: ctx.tick_context.positions.ball.position,
                    target_velocity: ctx.tick_context.positions.ball.velocity,
                }
                .calculate(ctx.player)
                .velocity,
            )
        } else {
            // Close - use Arrive for controlled approach
            Some(
                SteeringBehavior::Arrive {
                    target: ctx.tick_context.positions.ball.position,
                    slowing_distance: 2.0,
                }
                .calculate(ctx.player)
                .velocity,
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
        // CRITICAL: Hard maximum catch distance - no teleporting the ball!
        const MAX_CATCH_DISTANCE: f32 = 6.0; // Realistic goalkeeper reach (arms extended)
        let distance_to_ball = ctx.ball().distance();

        if distance_to_ball > MAX_CATCH_DISTANCE {
            return false; // Ball too far away to physically catch
        }

        // CRITICAL: Goalkeeper can only catch balls that are flying TOWARDS them
        // If the ball is flying away, they cannot catch it (e.g., their own pass/kick)
        if !ctx.ball().is_towards_player_with_angle(0.8) {
            return false; // Ball is flying away from goalkeeper - cannot catch
        }

        // Use goalkeeper-specific skills (handling is key for catching!)
        let handling = ctx.player.skills.technical.first_touch; // Using first_touch as handling proxy
        let reflexes = ctx.player.skills.mental.concentration; // Using concentration as reflexes proxy
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

        let ball_speed = ctx.tick_context.positions.ball.velocity.norm();
        let ball_height = ctx.tick_context.positions.ball.position.z;

        // Base success rate should be high for skilled keepers (0.6 - 0.95 range)
        let mut catch_probability = 0.5 + (base_skill * 0.45);

        // Ball speed modifier (additive, not multiplicative)
        // Slower balls are easier to catch
        if ball_speed < 5.0 {
            catch_probability += 0.15; // Very slow ball - easy catch
        } else if ball_speed < 10.0 {
            catch_probability += 0.10; // Slow ball - easier
        } else if ball_speed < 15.0 {
            catch_probability += 0.05; // Medium speed - slightly easier
        } else if ball_speed > 25.0 {
            catch_probability -= 0.15; // Very fast - harder
        } else if ball_speed > 20.0 {
            catch_probability -= 0.10; // Fast - harder
        }

        // Distance modifier (additive)
        // Close balls are much easier
        if distance_to_ball < 1.0 {
            catch_probability += 0.20; // Very close - very easy
        } else if distance_to_ball < 2.0 {
            catch_probability += 0.15; // Close - easier
        } else if distance_to_ball < 3.0 {
            catch_probability += 0.05; // Reasonable - slightly easier
        } else if distance_to_ball > 5.0 {
            catch_probability -= 0.20; // Too far - much harder
        } else if distance_to_ball > 4.0 {
            catch_probability -= 0.10; // Far - harder
        }

        // Height modifier (additive)
        // Chest height is ideal, ground and high balls are harder
        if ball_height >= 0.8 && ball_height <= 1.8 {
            catch_probability += 0.10; // Ideal catching height (chest to head)
        } else if ball_height < 0.3 {
            catch_probability -= 0.10; // Ground ball - harder to catch cleanly
        } else if ball_height > 2.5 {
            catch_probability -= 0.15; // High ball - difficult
        }

        // Check if ball is coming toward keeper (important!)
        if ctx.ball().is_towards_player_with_angle(0.7) {
            catch_probability += 0.10; // Ball coming straight at keeper
        } else {
            catch_probability -= 0.15; // Ball at awkward angle
        }

        // Bonus for elite keepers
        if base_skill > 0.8 {
            catch_probability += 0.05; // Elite keeper bonus
        }

        // Ensure catch probability is within reasonable range (min 10%, max 98%)
        let clamped_catch_probability = catch_probability.clamp(0.10, 0.98);

        // Random number between 0 and 1
        let random_factor = rand::random::<f32>();

        clamped_catch_probability > random_factor
    }
}
