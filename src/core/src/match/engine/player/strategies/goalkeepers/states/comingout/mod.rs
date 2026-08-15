#[cfg(feature = "match-logs")]
use crate::mid_run_diag::KeeperSweepDiag;
use crate::r#match::goalkeepers::states::common::{ActivityIntensity, GoalkeeperCondition};
use crate::r#match::goalkeepers::states::state::GoalkeeperState;
use crate::r#match::player::strategies::players::ops::goalkeeper_skill::GoalkeeperSkillProfile;
use crate::r#match::{
    ConditionContext, MatchPlayerLite, StateChangeResult, StateProcessingContext,
    StateProcessingHandler, SteeringBehavior,
};
use nalgebra::Vector3;

/// Close enough to gather it (~2.5 m).
const CLAIM_BALL_DISTANCE: f32 = 20.0;
/// Furthest a keeper will chase a loose ball (~20 m), before the
/// `rushing_out_profile` scaling either side of it.
///
/// Was 60u — **7.5 m**, written as if units were metres. A sweeper
/// keeper covers twenty-five metres of space behind his line; this one
/// would not leave his six-yard box, so the whole sweeping behaviour the
/// state exists for could never fire.
///
/// ⚠ THIS MUST STAY STRICTLY WIDER THAN THE DISTANCE AT WHICH
/// `should_rush_out_for_ball` COMMITS, or the pair is a two-cycle by
/// construction — the same ordering invariant `TackleEngagement`
/// documents (`COMMIT < DISENGAGE` is what makes the hand-off one-way).
///
/// 160u scaled by `0.85 + rushing * 0.55` gives up at 136-224 u, while
/// entry commits out to 180 u. They OVERLAP, so a keeper with modest
/// `rushing_out` entered and abandoned on the same tick: measured, he
/// committed to coming out **175 times a match** and `ComingOut` still
/// held under 0.25% of his ticks. He was flickering, not sweeping.
///
/// 240u (30 m) puts the give-up band at 204-336 u — strictly outside any
/// entry distance, so once he commits he actually goes.
const MAX_COMING_OUT_DISTANCE: f32 = 240.0;

#[derive(Default, Clone)]
pub struct GoalkeeperComingOutState {}

impl StateProcessingHandler for GoalkeeperComingOutState {
    fn process(&self, ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        // Got it — get on with it. Every sibling state (Standing,
        // PreparingForSave, ReturningToGoal, TakeBall) opens with this
        // branch; `ComingOut` never had one. A keeper who ended up owning
        // the ball here had no way out of the state and stood over it
        // until the ball's stall detector fired ~19.5 s later — measured
        // at four such episodes a match, one of them with the ball on the
        // goal line at x = 2.2.
        if ctx.player.has_ball(ctx) {
            #[cfg(feature = "match-logs")]
            KeeperSweepDiag::note_exit(0);
            return Some(StateChangeResult::with_goalkeeper_state(
                GoalkeeperState::Distributing,
            ));
        }

        // Shot in flight at our goal — abandon the rush and prepare
        // for the save. Staying in ComingOut leaves the goal wide open.
        if let Some(target) = &ctx.tick_context.ball.cached_shot_target {
            if Some(target.defending_side) == ctx.player.side {
                return Some(StateChangeResult::with_goalkeeper_state(
                    GoalkeeperState::PreparingForSave,
                ));
            }
        }

        let ball_distance = ctx.ball().distance();

        if self.should_dive(ctx) {
            #[cfg(feature = "match-logs")]
            KeeperSweepDiag::note_exit(2);
            return Some(StateChangeResult::with_goalkeeper_state(
                GoalkeeperState::Diving,
            ));
        }

        // If goalkeeper has reached the ball, claim it immediately
        // IMPORTANT: Only catch if goalkeeper is reasonably close to their goal
        // This prevents catching balls at center field
        let distance_from_goal = ctx.player().distance_from_start_position();
        const MAX_DISTANCE_FROM_GOAL_TO_CATCH: f32 = 50.0; // Only catch near goal area

        // Only a LOOSE ball can be claimed. Same defect `PreparingForSave`
        // carried: reaching the ball was tested on distance alone, so a
        // keeper who had come out gathered the ball off his own
        // defender's feet — or off an opponent's — as soon as he got
        // within 2.5 m of it. Every other route into `Catching`
        // (Standing, Walking, TakeBall, ReturningToGoal) has always
        // required the ball to be unowned; these two did not, and between
        // them they are why the keeper collected 154 balls a match
        // against a real 8-12.
        if !ctx.ball().is_owned()
            && ball_distance < CLAIM_BALL_DISTANCE
            && distance_from_goal < MAX_DISTANCE_FROM_GOAL_TO_CATCH
        {
            return Some(StateChangeResult::with_goalkeeper_state(
                GoalkeeperState::Catching,
            ));
        }

        // Check if ball is moving very fast toward goalkeeper at close range - prepare for save
        // Only trigger for actual shots (fast balls), not slow rolling balls the GK should claim
        let ball_speed = ctx.tick_context.positions.ball.velocity.norm();
        let ball_toward_keeper = ctx.ball().is_towards_player_with_angle(0.7);
        if ball_toward_keeper && ball_distance < 150.0 && ball_speed > 8.0 {
            return Some(StateChangeResult::with_goalkeeper_state(
                GoalkeeperState::PreparingForSave,
            ));
        }

        // Check if ball is too far. Maximum sweeper-keeper pursuit
        // distance now scales with the unified rushing-out profile so
        // weak keepers don't roam.
        let prof = GoalkeeperSkillProfile::from_ctx(ctx);
        let max_pursuit_distance =
            MAX_COMING_OUT_DISTANCE * (0.85 + prof.rushing_out_profile * 0.55);
        if ball_distance > max_pursuit_distance {
            #[cfg(feature = "match-logs")]
            KeeperSweepDiag::note_exit(5);
            return Some(StateChangeResult::with_goalkeeper_state(
                GoalkeeperState::ReturningToGoal,
            ));
        }

        // Check if ball is on opponent's half - return to goal
        if !ctx.ball().on_own_side() {
            #[cfg(feature = "match-logs")]
            KeeperSweepDiag::note_exit(6);
            return Some(StateChangeResult::with_goalkeeper_state(
                GoalkeeperState::ReturningToGoal,
            ));
        }

        // Check if opponent has the ball
        if let Some(opponent) = ctx.players().opponents().with_ball().next() {
            let opponent_distance = opponent.distance(ctx);
            let opponent_ball_distance =
                (opponent.position - ctx.tick_context.positions.ball.position).magnitude();

            // If opponent has control and is very close
            if opponent_ball_distance < 2.0 && opponent_distance < 20.0 {
                // Close opponent with ball - prepare for save/1v1
                return Some(StateChangeResult::with_goalkeeper_state(
                    GoalkeeperState::PreparingForSave,
                ));
            } else if opponent_ball_distance < 4.0 {
                // Opponent is near ball but we might intercept
                let keeper_advantage = self.can_reach_ball_first(ctx, &opponent);

                if keeper_advantage && ball_distance < 30.0 {
                    // We can reach it first - stay aggressive!
                    return None;
                } else if opponent_distance < 25.0 {
                    // Too risky - prepare for save
                    return Some(StateChangeResult::with_goalkeeper_state(
                        GoalkeeperState::PreparingForSave,
                    ));
                }
            }

            // Check if opponent with ball is moving AWAY from our goal
            // If so, don't chase them — return to goal position instead
            let own_goal = ctx.ball().direction_to_own_goal();
            let opponent_velocity = ctx.tick_context.positions.players.velocity(opponent.id);
            let opponent_speed = opponent_velocity.magnitude();
            if opponent_speed > 0.5 {
                let opponent_to_goal = (own_goal - opponent.position).normalize();
                let moving_toward_goal = opponent_velocity.normalize().dot(&opponent_to_goal);
                // Opponent is moving away from our goal (dot < 0) and GK is already out
                if moving_toward_goal < -0.2 && distance_from_goal > 20.0 {
                    return Some(StateChangeResult::with_goalkeeper_state(
                        GoalkeeperState::ReturningToGoal,
                    ));
                }
            }
        }

        // Ball is loose - be aggressive!
        if !ctx.ball().is_owned() {
            // Loose ball within reasonable distance - continue pursuit
            if ball_distance < max_pursuit_distance * 0.8 {
                return None; // Keep going!
            }

            // Even if far, continue if ball is moving towards us
            if ball_toward_keeper && ball_distance < max_pursuit_distance {
                return None; // Ball coming to us - keep pursuing
            }
        }

        // Check distance from goal — GK must not wander far
        let goal_distance = ctx.player().distance_from_start_position();
        let max_goal_distance =
            40.0 * (0.85 + prof.rushing_out_profile * 0.35 + prof.communication * 0.15);

        if goal_distance > max_goal_distance {
            // Getting far from goal - only continue if ball is very close and loose
            if !ctx.ball().is_owned() && ball_distance < 15.0 {
                return None; // Ball very close and loose, commit!
            } else {
                return Some(StateChangeResult::with_goalkeeper_state(
                    GoalkeeperState::ReturningToGoal,
                ));
            }
        }

        None
    }

    fn velocity(&self, ctx: &StateProcessingContext) -> Option<Vector3<f32>> {
        let ball_position = ctx.tick_context.positions.ball.position;
        let ball_velocity = ctx.tick_context.positions.ball.velocity;
        let ball_distance = ctx.ball().distance();
        let ball_speed = ball_velocity.norm();
        let prof = GoalkeeperSkillProfile::from_ctx(ctx);

        // Speed multiplier (1.2..2.05) gated by explosive multiplier
        // and rushing-out skill so weak/tired keepers arrive late.
        let speed_multiplier = (1.2 + prof.rushing_out_profile * 0.85) * prof.explosive_mult;

        // Add urgency bonus based on ball distance and speed
        let urgency_multiplier = if ball_distance < 20.0 {
            1.4 // Very close - maximum urgency
        } else if ball_distance < 40.0 {
            1.2 // Medium distance - high urgency
        } else {
            1.1 // Far distance - moderate urgency
        };

        // Calculate interception point for moving balls
        let target_position = if ball_speed > 1.0 {
            let keeper_sprint_speed =
                ctx.player.skills.physical.pace * (1.0 + prof.rushing_out_profile * 0.55);
            let time_to_intercept = ball_distance / keeper_sprint_speed.max(1.0);
            ball_position + ball_velocity * time_to_intercept * 0.8
        } else {
            ball_position
        };

        // Decide steering behavior based on distance
        if ball_distance < 5.0 {
            // Very close - use Arrive for controlled claiming
            Some(
                SteeringBehavior::Arrive {
                    target: target_position,
                    slowing_distance: 1.0,
                }
                .calculate(ctx.player)
                .velocity
                    * (speed_multiplier * 0.9), // Still fast but controllable
            )
        } else if ball_distance < 15.0 {
            // Close - sprint with slight deceleration zone
            Some(
                SteeringBehavior::Arrive {
                    target: target_position,
                    slowing_distance: 6.0,
                }
                .calculate(ctx.player)
                .velocity
                    * (speed_multiplier * urgency_multiplier),
            )
        } else {
            // Far - full sprint using Pursuit for maximum speed
            // For fast-moving balls, add extra boost
            let final_multiplier = if ball_speed > 5.0 {
                speed_multiplier * urgency_multiplier * 1.15
            } else {
                speed_multiplier * urgency_multiplier
            };

            Some(
                SteeringBehavior::Pursuit {
                    target: target_position,
                    target_velocity: Vector3::zeros(), // Static target position
                }
                .calculate(ctx.player)
                .velocity
                    * final_multiplier,
            )
        }
    }

    fn process_conditions(&self, ctx: ConditionContext) {
        // Coming out requires high intensity as goalkeeper moves out of penalty area aggressively
        GoalkeeperCondition::with_velocity(ActivityIntensity::High).process(ctx);
    }
}

impl GoalkeeperComingOutState {
    /// Check if goalkeeper can reach ball before opponent
    fn can_reach_ball_first(
        &self,
        ctx: &StateProcessingContext,
        opponent: &MatchPlayerLite,
    ) -> bool {
        let ball_position = ctx.tick_context.positions.ball.position;
        let ball_velocity = ctx.tick_context.positions.ball.velocity;
        let keeper_position = ctx.player.position;
        let opponent_position = opponent.position;

        // Distance calculations
        let keeper_to_ball = (ball_position - keeper_position).magnitude();
        let opponent_to_ball = (ball_position - opponent_position).magnitude();

        // Keeper sprint speed reads raw pace + acceleration — both are
        // direct kinematic skills, fatigue is already folded in via the
        // engine's `max_speed_with_condition` clamp. The keeper's
        // *decision* to commit (anticipation, agility, hands) is folded
        // into `gk_rush_out` below; this branch is the foot-race math.
        let keeper_pace = ctx.player.skills.physical.pace;
        let keeper_acceleration = ctx.player.skills.physical.acceleration / 20.0;
        let opponent_pace = ctx.player().skills(opponent.id).physical.pace;
        let opponent_acceleration = ctx.player().skills(opponent.id).physical.acceleration / 20.0;

        let keeper_sprint_speed = keeper_pace * (1.0 + keeper_acceleration * 0.5);
        let opponent_sprint_speed = opponent_pace * (1.0 + opponent_acceleration * 0.3);

        // If ball is moving, predict interception point
        let (keeper_distance, opponent_distance) = if ball_velocity.norm() > 1.0 {
            // Ball is moving - calculate interception distances

            // Simple prediction: where will ball be when keeper/opponent reaches it
            let keeper_intercept_time = keeper_to_ball / keeper_sprint_speed.max(1.0);
            let opponent_intercept_time = opponent_to_ball / opponent_sprint_speed.max(1.0);

            let keeper_intercept_pos = ball_position + ball_velocity * keeper_intercept_time;
            let opponent_intercept_pos = ball_position + ball_velocity * opponent_intercept_time;

            (
                (keeper_intercept_pos - keeper_position).magnitude(),
                (opponent_intercept_pos - opponent_position).magnitude(),
            )
        } else {
            // Ball is stationary
            (keeper_to_ball, opponent_to_ball)
        };

        // Time estimates with actual distances
        let keeper_time = keeper_distance / keeper_sprint_speed.max(1.0);
        let opponent_time = opponent_distance / opponent_sprint_speed.max(1.0);

        // Keeper foot-race advantage routed through the unified
        // rushing-out profile. Keeps the constant 1.2 hand advantage
        // (real rule — keepers use hands to claim from further away).
        // Eccentricity tilts the *judgement*, not the execution: an
        // eccentric keeper gambles on races an ordinary keeper would
        // decline (±6% margin at the extremes), a conservative one
        // demands a clearer head start. Centered at 10/20 so the
        // ordinary keeper keeps the pre-existing gate.
        let prof = GoalkeeperSkillProfile::from_ctx(ctx);
        let hand_advantage = 1.2;
        let risk_appetite = prof.eccentricity - 0.5;
        let total_advantage =
            hand_advantage * (0.85 + prof.rushing_out_profile * 0.50 + risk_appetite * 0.12);

        // Keeper wins if their time is less than opponent's time adjusted for advantages
        keeper_time < (opponent_time * total_advantage)
    }

    fn should_dive(&self, ctx: &StateProcessingContext) -> bool {
        let ball_distance = ctx.ball().distance();
        let ball_velocity = ctx.tick_context.positions.ball.velocity;
        let ball_speed = ball_velocity.norm();
        let ball_position = ctx.tick_context.positions.ball.position;
        let keeper_position = ctx.player.position;
        let prof = GoalkeeperSkillProfile::from_ctx(ctx);

        // Reuse profile fields for the dive-decision branches below.
        let reflexes = prof.shot_stopping;
        let agility = prof.dive_reach;
        let bravery = prof.one_v_one;
        let anticipation = prof.positioning;
        let positioning = prof.positioning;

        // Ball must be moving with reasonable speed to consider diving
        if ball_speed < 2.5 {
            return false;
        }

        // Don't dive if ball is too far - even skilled keepers have limits
        const MAX_DIVE_DISTANCE: f32 = 8.0;
        let skill_adjusted_max_dive = MAX_DIVE_DISTANCE + (agility * 4.0) + (reflexes * 3.0);

        if ball_distance > skill_adjusted_max_dive {
            return false;
        }

        // Check if ball is moving towards the keeper (important - don't dive at balls going away!)
        let ball_towards_keeper = ctx.ball().is_towards_player_with_angle(0.5);
        if !ball_towards_keeper {
            // Ball moving away or parallel - no need to dive
            return false;
        }

        // Calculate how close the ball will get if keeper doesn't dive
        let keeper_to_ball_dir = (ball_position - keeper_position).normalize();
        let ball_vel_normalized = if ball_speed > 0.01 {
            ball_velocity.normalize()
        } else {
            return false;
        };

        // Dot product to see if ball trajectory will bring it close
        let trajectory_alignment = keeper_to_ball_dir.dot(&ball_vel_normalized);

        // If ball is moving towards keeper but not directly, adjust dive threshold
        let trajectory_factor = trajectory_alignment.max(0.0);

        // Calculate time until ball reaches keeper's position
        let time_to_reach = ball_distance / ball_speed.max(1.0);

        // Predict where ball will be in near future
        let predicted_ball_pos = ball_position + ball_velocity * time_to_reach.min(2.0);
        let predicted_distance = (predicted_ball_pos - keeper_position).magnitude();

        // Decision factors:
        // 1. Ball is getting closer (predicted distance < current distance)
        // 2. Ball will be very close soon
        // 3. Keeper doesn't have time to run there

        let ball_getting_closer = predicted_distance < ball_distance;

        // Calculate if keeper can reach the ball by running
        let keeper_sprint_speed = ctx.player.skills.physical.pace
            * (1.0 + ctx.player.skills.physical.acceleration / 40.0);
        let time_to_run_to_ball = ball_distance / keeper_sprint_speed.max(1.0);

        // Dive if:
        // - Ball is close and getting closer
        // - Ball speed suggests keeper can't reach by running
        // - Keeper's skills support the dive decision

        let urgency_threshold = 0.8 + (anticipation * 0.3); // Better anticipation = earlier dive decision
        let dive_urgency = time_to_reach < urgency_threshold;

        // Different scenarios based on ball speed
        if ball_speed > 15.0 {
            // Very fast shot - need quick reflexes
            let reflex_threshold = 0.5 - (reflexes * 0.3);
            ball_getting_closer
                && ball_distance < (6.0 + reflexes * 5.0)
                && time_to_reach < reflex_threshold
                && trajectory_factor > 0.6
        } else if ball_speed > 10.0 {
            // Fast shot - need good positioning and reflexes
            let can_catch_running = time_to_run_to_ball < time_to_reach * 1.3;

            if can_catch_running {
                // Can probably catch it running - only dive if very close or excellent skills
                ball_distance < 4.0 && bravery > 0.6 && ball_getting_closer
            } else {
                // Need to dive to reach
                ball_distance < (8.0 + agility * 3.0 + positioning * 2.0)
                    && dive_urgency
                    && trajectory_factor > 0.5
                    && bravery > 0.4
            }
        } else if ball_speed > 5.0 {
            // Medium speed - keeper has more time to decide
            let can_catch_running = time_to_run_to_ball < time_to_reach * 1.5;

            if can_catch_running {
                // Prefer to run and catch - only dive if skills are high and ball is very close
                ball_distance < 3.0
                    && (reflexes + agility) > 1.3
                    && bravery > 0.7
                    && ball_getting_closer
            } else {
                // Ball will pass before keeper can run - dive needed
                ball_distance < (6.0 + agility * 4.0)
                    && predicted_distance < 8.0
                    && trajectory_factor > 0.4
                    && bravery > 0.5
            }
        } else {
            // Slow ball - generally shouldn't dive, should run and catch
            // Only dive if extremely close and keeper is brave/skilled
            ball_distance < 2.5
                && ball_getting_closer
                && bravery > 0.8
                && (reflexes + agility + positioning) > 2.0
                && trajectory_factor > 0.7
        }
    }
}
