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
        let reflexes = ctx.player.skills.mental.concentration / 20.0;

        // GK sprints explosively to catch — reflexes + agility drive reaction speed
        let speed_boost = 1.5 + agility * 0.4 + reflexes * 0.4; // 1.5x - 2.3x

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
        let distance_to_ball = ctx.ball().distance();

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

        // Maximum catch distance — skill-dependent reach
        // Elite GK: 8 + 5 + 3 = 16, mediocre: 8 + 2.3 + 1.4 = 11.7
        let max_catch_distance = 8.0 + scaled_agility * 5.0 + scaled_handling * 3.0;

        if distance_to_ball > max_catch_distance {
            return false; // Ball too far away to physically catch
        }

        // Goalkeeper can only catch balls that are flying TOWARDS them or are stationary/slow
        let ball_speed = ctx.tick_context.positions.ball.velocity.norm();
        if ball_speed > 0.5 && !ctx.ball().is_towards_player_with_angle(0.6) {
            return false; // Ball is flying away from goalkeeper
        }

        // Base catch skill (weighted toward handling and reflexes)
        let base_skill = scaled_handling * 0.35 + scaled_reflexes * 0.30 +
                          scaled_positioning * 0.20 + scaled_agility * 0.15;

        let ball_height = ctx.tick_context.positions.ball.position.z;

        // Base success rate — strong skill differentiation
        // Elite (~1.0): 0.25 + 0.70 = 0.95, mediocre (~0.47): 0.25 + 0.33 = 0.58
        let mut catch_probability = 0.25 + (base_skill * 0.70);

        // Ball speed modifier calibrated for actual speeds
        if ball_speed < 0.8 {
            catch_probability += 0.12; // Very slow - easier catch
        } else if ball_speed < 1.2 {
            catch_probability += 0.05; // Moderate speed
        } else if ball_speed > 2.0 {
            // Strong shot - much harder, skilled keepers mitigate significantly
            catch_probability -= 0.20 * (1.0 - scaled_reflexes * 0.6);
        } else if ball_speed > 1.5 {
            catch_probability -= 0.12 * (1.0 - scaled_reflexes * 0.5);
        }

        // Distance modifier
        if distance_to_ball < 2.0 {
            catch_probability += 0.12; // Very close - easy
        } else if distance_to_ball < 5.0 {
            catch_probability += 0.06; // Close
        } else if distance_to_ball > max_catch_distance * 0.85 {
            catch_probability -= 0.15; // Fully stretched
        } else if distance_to_ball > max_catch_distance * 0.6 {
            catch_probability -= 0.08; // Extended
        }

        // Height modifier — agility helps with awkward heights
        if ball_height >= 0.5 && ball_height <= 1.8 {
            catch_probability += 0.06; // Ideal catching height
        } else if ball_height < 0.2 {
            catch_probability -= 0.08 * (1.0 - scaled_agility * 0.6); // Ground ball
        } else if ball_height > 2.5 {
            catch_probability -= 0.12 * (1.0 - scaled_agility * 0.5); // High ball
        }

        // Direction modifier
        if ctx.ball().is_towards_player_with_angle(0.7) {
            catch_probability += 0.06;
        } else {
            catch_probability -= 0.10;
        }

        // Elite keeper bonus — top GKs make extraordinary saves
        if base_skill > 0.8 {
            catch_probability += 0.10;
        } else if base_skill > 0.65 {
            catch_probability += 0.05;
        }

        let clamped_catch_probability = catch_probability.clamp(0.08, 0.97);

        rand::random::<f32>() < clamped_catch_probability
    }
}
