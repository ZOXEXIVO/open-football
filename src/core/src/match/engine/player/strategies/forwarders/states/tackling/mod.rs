use crate::r#match::events::Event;
use crate::r#match::forwarders::states::common::{ActivityIntensity, ForwardCondition};
use crate::r#match::forwarders::states::ForwardState;
use crate::r#match::player::events::PlayerEvent;
use crate::r#match::{
    ConditionContext, MatchPlayerLite, StateChangeResult, StateProcessingContext,
    StateProcessingHandler, SteeringBehavior,
};
use nalgebra::Vector3;
use rand::RngExt;

const TACKLE_DISTANCE_THRESHOLD: f32 = 20.0; // Maximum distance to attempt a tackle
const CLOSE_TACKLE_DISTANCE: f32 = 10.0; // Distance for immediate tackle attempt
const FOUL_CHANCE_BASE: f32 = 0.15; // Base chance of committing a foul
const CHASE_DISTANCE_THRESHOLD: f32 = 100.0; // Maximum distance to chase for tackle
const PRESSURE_DISTANCE: f32 = 20.0; // Distance to apply pressure without tackling

#[derive(Default)]
pub struct ForwardTacklingState {}

impl StateProcessingHandler for ForwardTacklingState {
    fn try_fast(&self, ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        // If player has gained possession, transition to running
        if ctx.player.has_ball(ctx) {
            return Some(StateChangeResult::with_forward_state(ForwardState::Running));
        }

        // CRITICAL: Don't try to claim ball if it's in protected flight state
        // This prevents the flapping issue where two players repeatedly claim
        if ctx.ball().is_in_flight() {
            return None;
        }

        let opponents = ctx.players().opponents();
        let opponents_with_ball: Vec<MatchPlayerLite> = opponents.with_ball().collect();

        if let Some(opponent) = opponents_with_ball.first() {
            let opponent_distance = ctx.tick_context.distances.get(ctx.player.id, opponent.id);

            // Immediate tackle if very close
            if opponent_distance <= CLOSE_TACKLE_DISTANCE {
                let (tackle_success, committed_foul) = self.attempt_tackle(ctx, opponent);

                if committed_foul {
                    return Some(StateChangeResult::with_forward_state_and_event(
                        ForwardState::Standing,
                        Event::PlayerEvent(PlayerEvent::CommitFoul),
                    ));
                }

                if tackle_success {
                    // Double-check ball is not in flight before claiming
                    if !ctx.ball().is_in_flight() {
                        return Some(StateChangeResult::with_forward_state_and_event(
                            ForwardState::Running,
                            Event::PlayerEvent(PlayerEvent::ClaimBall(ctx.player.id)),
                        ));
                    }
                }

                // Failed tackle - continue pressuring
                return None;
            }

            // If within tackle range but not close enough for immediate attempt
            if opponent_distance <= TACKLE_DISTANCE_THRESHOLD {
                // Wait for better opportunity or attempt tackle based on situation
                if self.should_attempt_tackle_now(ctx, opponent) {
                    let (tackle_success, committed_foul) = self.attempt_tackle(ctx, opponent);

                    if committed_foul {
                        return Some(StateChangeResult::with_forward_state_and_event(
                            ForwardState::Standing,
                            Event::PlayerEvent(PlayerEvent::CommitFoul),
                        ));
                    }

                    if tackle_success {
                        // Double-check ball is not in flight before claiming
                        if !ctx.ball().is_in_flight() {
                            return Some(StateChangeResult::with_forward_state_and_event(
                                ForwardState::Running,
                                Event::PlayerEvent(PlayerEvent::ClaimBall(ctx.player.id)),
                            ));
                        }
                    }
                }

                // Continue positioning for tackle
                return None;
            }

            // If opponent is further but still chaseable, continue pursuit
            if opponent_distance <= CHASE_DISTANCE_THRESHOLD {
                return None; // Continue chasing
            }
        }

        // Check for loose ball interception opportunities
        // Already checks is_in_flight in can_intercept_ball
        if !ctx.ball().is_owned() && self.can_intercept_ball(ctx) {
            return Some(StateChangeResult::with_forward_state_and_event(
                ForwardState::Running,
                Event::PlayerEvent(PlayerEvent::ClaimBall(ctx.player.id)),
            ));
        }

        let ball_distance = ctx.ball().distance();

        if ctx.team().is_control_ball() {
            if ball_distance > CHASE_DISTANCE_THRESHOLD {
                return Some(StateChangeResult::with_forward_state(ForwardState::Returning));
            }

            return Some(StateChangeResult::with_forward_state(ForwardState::Assisting));
        } else if ball_distance <= PRESSURE_DISTANCE {
            return Some(StateChangeResult::with_forward_state(ForwardState::Pressing));
        }

        None
    }

    fn process_slow(&self, _ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        None
    }

    fn velocity(&self, ctx: &StateProcessingContext) -> Option<Vector3<f32>> {
        let opponents = ctx.players().opponents();
        let opponents_with_ball: Vec<MatchPlayerLite> = opponents.with_ball().collect();

        if let Some(opponent) = opponents_with_ball.first() {
            let opponent_distance = ctx.tick_context.distances.get(ctx.player.id, opponent.id);

            // If very close, move more carefully to avoid overrunning
            if opponent_distance <= TACKLE_DISTANCE_THRESHOLD {
                return Some(
                    SteeringBehavior::Arrive {
                        target: opponent.position,
                        slowing_distance: 1.0,
                    }
                        .calculate(ctx.player)
                        .velocity,
                );
            } else {
                // Chase more aggressively when further away
                return Some(
                    SteeringBehavior::Pursuit {
                        target: opponent.position,
                        target_velocity: Vector3::zeros(), // Opponent velocity not available in lite struct
                    }
                        .calculate(ctx.player)
                        .velocity,
                );
            }
        }

        // If no opponent with ball, go for loose ball
        if !ctx.ball().is_owned() {
            return Some(
                SteeringBehavior::Pursuit {
                    target: ctx.tick_context.positions.ball.position,
                    target_velocity: ctx.tick_context.positions.ball.velocity,
                }
                    .calculate(ctx.player)
                    .velocity,
            );
        }

        // Default movement toward ball position
        Some(
            SteeringBehavior::Arrive {
                target: ctx.tick_context.positions.ball.position,
                slowing_distance: 20.0,
            }
                .calculate(ctx.player)
                .velocity,
        )
    }

    fn process_conditions(&self, ctx: ConditionContext) {
        // Tackling is very high intensity - explosive action
        ForwardCondition::new(ActivityIntensity::VeryHigh).process(ctx);
    }
}

impl ForwardTacklingState {
    /// Determine if the player should attempt a tackle right now
    fn should_attempt_tackle_now(&self, ctx: &StateProcessingContext, opponent: &MatchPlayerLite) -> bool {
        let tackling_skill = ctx.player.skills.technical.tackling / 20.0;
        let aggression = ctx.player.skills.mental.aggression / 20.0;

        // More skilled/aggressive players tackle more readily
        let tackle_eagerness = (tackling_skill * 0.7) + (aggression * 0.3);

        // Check opponent's situation
        let opponent_velocity = ctx.tick_context.positions.players.velocity(opponent.id);
        let opponent_is_stationary = opponent_velocity.magnitude() < 0.5;

        // More likely to tackle if opponent is stationary or moving slowly
        if opponent_is_stationary {
            return rand::random::<f32>() < tackle_eagerness * 1.2;
        }

        // Check if opponent is moving toward our goal (more urgent to tackle)
        let to_our_goal = ctx.ball().direction_to_own_goal() - opponent.position;
        let opponent_direction = opponent_velocity.normalize();
        let threat_level = to_our_goal.normalize().dot(&opponent_direction);

        if threat_level > 0.5 {
            // Opponent moving toward our goal - tackle more eagerly
            return rand::random::<f32>() < tackle_eagerness * 1.4;
        }

        // Standard tackle decision
        rand::random::<f32>() < tackle_eagerness * 0.8
    }

    /// Attempt a tackle with improved physics and skill-based calculation
    fn attempt_tackle(
        &self,
        ctx: &StateProcessingContext,
        opponent: &MatchPlayerLite,
    ) -> (bool, bool) {
        let mut rng = rand::rng();

        // Player skills
        let tackling_skill = ctx.player.skills.technical.tackling / 20.0;
        let aggression = ctx.player.skills.mental.aggression / 20.0;
        let composure = ctx.player.skills.mental.composure / 20.0;
        let pace = ctx.player.skills.physical.pace / 20.0;

        // Opponent skills
        let player = ctx.player();
        let opponent_skills = player.skills(opponent.id);
        let opponent_dribbling = opponent_skills.technical.dribbling / 20.0;
        let opponent_agility = opponent_skills.physical.agility / 20.0;
        let opponent_balance = opponent_skills.physical.balance / 20.0;
        let opponent_composure = opponent_skills.mental.composure / 20.0;

        // Calculate relative positioning advantage
        let distance = ctx.tick_context.distances.get(ctx.player.id, opponent.id);
        let distance_factor = (TACKLE_DISTANCE_THRESHOLD - distance) / TACKLE_DISTANCE_THRESHOLD;
        let distance_factor = distance_factor.clamp(0.0, 1.0);

        // Calculate angle advantage (tackling from behind is harder but less likely to be seen)
        let opponent_velocity = ctx.tick_context.positions.players.velocity(opponent.id);
        let tackle_angle_factor = if opponent_velocity.magnitude() > 0.1 {
            let to_opponent = (opponent.position - ctx.player.position).normalize();
            let opponent_direction = opponent_velocity.normalize();
            let angle_dot = to_opponent.dot(&opponent_direction);

            // Tackling from the side (perpendicular) is most effective
            1.0 - angle_dot.abs()
        } else {
            0.8 // Stationary opponent - moderate advantage
        };

        // Calculate tackle effectiveness
        let player_tackle_ability = (tackling_skill * 0.5) + (pace * 0.2) + (composure * 0.3);
        let opponent_evasion_ability = (opponent_dribbling * 0.4) + (opponent_agility * 0.3) +
            (opponent_balance * 0.2) + (opponent_composure * 0.1);

        // Final success calculation
        let base_success = player_tackle_ability - opponent_evasion_ability;
        let situational_bonus = distance_factor * 0.3 + tackle_angle_factor * 0.2;
        let success_chance = (0.5 + base_success * 0.4 + situational_bonus).clamp(0.05, 0.95);

        let tackle_success = rng.random::<f32>() < success_chance;

        // Calculate foul probability - more refined
        let foul_base_risk = FOUL_CHANCE_BASE;
        let aggression_risk = aggression * 0.1;
        let desperation_risk = if ctx.team().is_loosing() && ctx.context.time.is_running_out() {
            0.05 // More desperate when losing late in game
        } else {
            0.0
        };

        let skill_protection = composure * 0.05; // Better composure reduces foul risk
        let situation_risk = if tackle_angle_factor < 0.3 {
            0.08 // Higher risk when tackling from behind
        } else {
            0.0
        };

        let foul_chance = if tackle_success {
            // Lower foul chance for successful tackles, but still possible
            (foul_base_risk * 0.3) + aggression_risk + desperation_risk + situation_risk - skill_protection
        } else {
            // Higher foul chance for failed tackles
            foul_base_risk + aggression_risk + desperation_risk + situation_risk + 0.05 - skill_protection
        };

        let foul_chance = foul_chance.clamp(0.0, 0.4); // Cap maximum foul chance
        let committed_foul = rng.random::<f32>() < foul_chance;

        (tackle_success, committed_foul)
    }

    /// Check if player can intercept a loose ball
    fn can_intercept_ball(&self, ctx: &StateProcessingContext) -> bool {
        // Don't try to intercept if ball is owned or in flight
        if ctx.ball().is_owned() || ctx.ball().is_in_flight() {
            return false;
        }

        let ball_position = ctx.tick_context.positions.ball.position;
        let ball_velocity = ctx.tick_context.positions.ball.velocity;
        let player_position = ctx.player.position;
        let player_speed = ctx.player.skills.physical.pace / 20.0 * 10.0; // Convert to game units

        // If ball is moving, calculate interception
        if ball_velocity.magnitude() > 0.5 {
            // Calculate if player can reach ball before it goes too far
            let time_to_ball = (ball_position - player_position).magnitude() / player_speed;
            let ball_future_position = ball_position + ball_velocity * time_to_ball;
            let intercept_distance = (ball_future_position - player_position).magnitude();

            // Check if interception is feasible
            if intercept_distance <= TACKLE_DISTANCE_THRESHOLD * 2.0 {
                // Also check if any opponent is closer to the interception point
                let closest_opponent_distance = ctx.players().opponents().all()
                    .map(|opp| (ball_future_position - opp.position).magnitude())
                    .min_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal))
                    .unwrap_or(f32::MAX);

                return intercept_distance < closest_opponent_distance * 0.9; // Need to be clearly closer
            }
        } else {
            // Ball is stationary - simple distance check
            let ball_distance = (ball_position - player_position).magnitude();

            if ball_distance <= TACKLE_DISTANCE_THRESHOLD {
                // Check if any opponent is closer
                let closest_opponent_distance = ctx.players().opponents().all()
                    .map(|opp| (ball_position - opp.position).magnitude())
                    .min_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal))
                    .unwrap_or(f32::MAX);

                return ball_distance < closest_opponent_distance * 0.8;
            }
        }

        false
    }
}