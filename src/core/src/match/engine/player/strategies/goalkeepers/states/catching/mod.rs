use crate::r#match::goalkeepers::states::common::{ActivityIntensity, GoalkeeperCondition};
use crate::r#match::goalkeepers::states::state::GoalkeeperState;
use crate::r#match::player::events::PlayerEvent;
use crate::r#match::{ConditionContext, PlayerDistanceFromStartPosition, StateChangeResult, StateProcessingContext, StateProcessingHandler, SteeringBehavior};
use nalgebra::Vector3;

#[derive(Default, Clone)]
pub struct GoalkeeperCatchingState {}

impl StateProcessingHandler for GoalkeeperCatchingState {
    fn process(&self, ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        if self.is_catch_successful(ctx) {
            let mut holding_result =
                StateChangeResult::with_goalkeeper_state(GoalkeeperState::HoldingBall);

            holding_result
                .events
                .add_player_event(PlayerEvent::CaughtBall(ctx.player.id));

            return Some(holding_result);
        }

        // Shot is live: stay in Catching and keep sprinting toward the
        // intercept line. The old logic exited to Standing / ComingOut
        // the moment the ball was >12u away, which meant a keeper
        // aiming for the far post gave up the instant the shot was
        // fired. With a cached shot target the keeper commits.
        if ctx.tick_context.ball.cached_shot_target.is_some() {
            return None;
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


    fn velocity(&self, ctx: &StateProcessingContext) -> Option<Vector3<f32>> {
        let agility = ctx.player.skills.physical.agility / 20.0;
        let reflexes = ctx.player.skills.goalkeeping.reflexes / 20.0;

        // GK sprints explosively to catch — reflexes + agility drive reaction speed
        let speed_boost = 1.7 + agility * 0.5 + reflexes * 0.5; // 1.7x - 2.7x

        // Shot in flight → commit to the intercept line, don't chase
        // the current ball position (it's moving at 5.6 u/tick and
        // outrunning the keeper's pursuit steering).
        if let Some(target) = &ctx.tick_context.ball.cached_shot_target {
            let goal_pos = ctx.ball().direction_to_own_goal();
            let intercept = Vector3::new(goal_pos.x, target.goal_line_y, 0.0);
            return Some(
                SteeringBehavior::Arrive {
                    target: intercept,
                    slowing_distance: 2.0,
                }
                .calculate(ctx.player)
                .velocity
                    * speed_boost,
            );
        }

        let ball_distance = ctx.ball().distance();
        if ball_distance > 3.0 {
            Some(
                SteeringBehavior::Pursuit {
                    target: ctx.tick_context.positions.ball.position,
                    target_velocity: ctx.tick_context.positions.ball.velocity,
                }
                .calculate(ctx.player)
                .velocity
                    * speed_boost,
            )
        } else {
            Some(
                SteeringBehavior::Arrive {
                    target: ctx.tick_context.positions.ball.position,
                    slowing_distance: 1.5,
                }
                .calculate(ctx.player)
                .velocity
                    * (speed_boost * 0.8),
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
        // Shot-in-flight: judge the save from the *intercept line*, not
        // from current ball distance. A ball aimed into the corner
        // passes the GK 8-15 units wide of their current position —
        // real keepers reach 3-4 m (6-8 u) diving, so the relevant
        // metric is "how far off the line am I?", not "am I touching
        // the ball right now?".
        if let Some(target) = &ctx.tick_context.ball.cached_shot_target {
            // Ball over the bar — no save attempt worth making.
            if target.goal_line_z > 2.44 {
                return false;
            }
            let handling = ctx.player.skills.goalkeeping.handling;
            let reflexes = ctx.player.skills.goalkeeping.reflexes;
            let agility = ctx.player.skills.physical.agility;
            let scaled_handling = (handling - 1.0) / 19.0;
            let scaled_reflexes = (reflexes - 1.0) / 19.0;
            let scaled_agility = (agility - 1.0) / 19.0;

            // Diving reach. Elite keeper: 4.5 + 3 + 2 = 9.5 u (~4.7 m
            // lateral, matches real-world). Mediocre: 4.5 + 1.2 + 1 ≈
            // 6.7 u (~3.3 m).
            let reach = 4.5 + scaled_agility * 3.0 + scaled_reflexes * 2.0;
            let lateral_error = (ctx.player.position.y - target.goal_line_y).abs();
            if lateral_error > reach {
                return false; // Out of reach — shot beats the keeper.
            }
            // Base save chance — quadratic falloff rather than linear.
            // A shot straight at the keeper is ~95% saved (reflex), a
            // shot at half-reach still ~80%, only at full stretch does
            // it drop to ~30%. Previous linear curve saved too few of
            // the central shots — real keepers save the middle reliably
            // and lose placement to the corners.
            //   ratio 0.0 → 0.95   (on the line)
            //   ratio 0.3 → 0.89
            //   ratio 0.5 → 0.79
            //   ratio 0.7 → 0.63
            //   ratio 1.0 → 0.30   (fully stretched)
            let reach_ratio = (lateral_error / reach).clamp(0.0, 1.0);
            let base = 0.95 - reach_ratio * reach_ratio * 0.65;

            // Shot-speed penalty — the dominant footballing reason
            // keepers don't save everything is that elite shooters
            // generate 100+ km/h shots the keeper can't react to
            // quickly enough. Ball velocity 3.0 is a tame shot, 5.0+
            // is elite power. Every unit of speed over 3 knocks a
            // skill-mitigated chunk off the save probability.
            let ball_speed = ctx.tick_context.positions.ball.velocity.norm();
            let speed_excess = (ball_speed - 3.0).max(0.0);
            // Reflexes mitigate fast shots — elite keeper loses ~half
            // as much to a piledriver as a mediocre one.
            let speed_penalty = (speed_excess * 0.10 * (1.0 - scaled_reflexes * 0.5)).min(0.45);

            let skill = scaled_handling * 0.4 + scaled_reflexes * 0.4 + scaled_agility * 0.2;
            let save_prob = ((base - speed_penalty) * (0.6 + skill * 0.5)).clamp(0.05, 0.95);
            return rand::random::<f32>() < save_prob;
        }

        let distance_to_ball = ctx.ball().distance();

        // Use goalkeeper-specific skills
        let handling = ctx.player.skills.goalkeeping.handling;
        let reflexes = ctx.player.skills.goalkeeping.reflexes;
        let positioning = ctx.player.skills.goalkeeping.command_of_area;
        let agility = ctx.player.skills.physical.agility;

        // Scale skills from 1-20 range to 0-1 range
        let scaled_handling = (handling - 1.0) / 19.0;
        let scaled_reflexes = (reflexes - 1.0) / 19.0;
        let scaled_positioning = (positioning - 1.0) / 19.0;
        let scaled_agility = (agility - 1.0) / 19.0;

        // Maximum catch distance — skill-dependent reach
        // Elite GK: 10 + 6 + 4 = 20, mediocre: 10 + 2.8 + 1.9 = 14.7
        let max_catch_distance = 10.0 + scaled_agility * 6.0 + scaled_handling * 4.0;

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
        // Elite (~1.0): 0.30 + 0.68 = 0.98, mediocre (~0.47): 0.30 + 0.32 = 0.62
        let mut catch_probability = 0.30 + (base_skill * 0.68);

        // Ball speed modifier calibrated for actual speeds
        if ball_speed < 0.8 {
            catch_probability += 0.15; // Very slow - easier catch
        } else if ball_speed < 1.2 {
            catch_probability += 0.08; // Moderate speed
        } else if ball_speed > 2.0 {
            // Strong shot - harder, but skilled keepers mitigate significantly
            catch_probability -= 0.15 * (1.0 - scaled_reflexes * 0.65);
        } else if ball_speed > 1.5 {
            catch_probability -= 0.08 * (1.0 - scaled_reflexes * 0.55);
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

        // Elite keeper bonus — scales smoothly above 0.65 rather than flat steps
        if base_skill > 0.65 {
            catch_probability += (base_skill - 0.65) * 0.15; // up to +0.053 for elite
        }

        let clamped_catch_probability = catch_probability.clamp(0.04, 0.95);

        rand::random::<f32>() < clamped_catch_probability
    }
}
