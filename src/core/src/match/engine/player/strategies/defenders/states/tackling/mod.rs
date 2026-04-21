use crate::r#match::defenders::states::DefenderState;
use crate::r#match::defenders::states::common::{DefenderCondition, ActivityIntensity};
use crate::r#match::events::Event;
use crate::r#match::player::events::{FoulSeverity, PlayerEvent};
use crate::r#match::{
    ConditionContext, MatchPlayerLite, StateChangeResult,
    StateProcessingContext, StateProcessingHandler, SteeringBehavior,
};
use nalgebra::Vector3;
use rand::RngExt;

const TACKLE_DISTANCE_THRESHOLD: f32 = 25.0; // Close down earlier — aggressive defending
const PRESSING_DISTANCE: f32 = 80.0;
const RETURN_DISTANCE: f32 = 120.0;

#[derive(Default, Clone)]
pub struct DefenderTacklingState {}

impl StateProcessingHandler for DefenderTacklingState {
    fn process(&self, ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        // If we have the ball or our team controls it, transition to running
        if ctx.player.has_ball(ctx) || ctx.team().is_control_ball() {
            return Some(StateChangeResult::with_defender_state(
                DefenderState::Running,
            ));
        }

        // CRITICAL: Don't try to claim ball if it's in protected flight state
        // Transition OUT of tackling to avoid clustering around the ball carrier
        if ctx.ball().is_in_flight() {
            return Some(StateChangeResult::with_defender_state(
                DefenderState::Returning,
            ));
        }

        // Check if there's an opponent with the ball
        if let Some(opponent) = ctx.players().opponents().with_ball().next() {
            let distance_to_opponent = opponent.distance(ctx);

            // If opponent is too far for tackling, press instead
            if distance_to_opponent > PRESSING_DISTANCE {
                return Some(StateChangeResult::with_defender_state(
                    DefenderState::Pressing,
                ));
            }

            // If opponent is close but not in tackle range, keep pressing
            if distance_to_opponent > TACKLE_DISTANCE_THRESHOLD {
                return Some(StateChangeResult::with_defender_state(
                    DefenderState::Pressing,
                ));
            }

            // Per-player tackle cooldown: a single per-state-machine gate
            // (e.g. Pressing→Tackling cadence) never held because the
            // Tackling state can be re-entered from Standing / Running /
            // Covering / Guarding / HoldingLine, each with its own
            // distance trigger and no shared cooldown. The cooldown lives
            // on the player itself — whatever path routed us here, if we
            // just tackled, we can't tackle again for ~1 s.
            if !ctx.player.can_attempt_tackle() {
                return Some(StateChangeResult::with_defender_state(
                    DefenderState::Pressing,
                ));
            }

            // We're close enough to tackle! One shot per Tackling entry,
            // enforced by the cooldown.
            let (tackle_success, committed_foul, foul_severity) =
                self.attempt_sliding_tackle(ctx, &opponent);

            return if tackle_success {
                let mut result = StateChangeResult::with_defender_state_and_event(
                    DefenderState::Standing,
                    Event::PlayerEvent(PlayerEvent::TacklingBall(ctx.player.id)),
                );
                result.start_tackle_cooldown = true;
                Some(result)
            } else if committed_foul {
                let mut result = StateChangeResult::with_defender_state_and_event(
                    DefenderState::Standing,
                    Event::PlayerEvent(PlayerEvent::CommitFoul(ctx.player.id, foul_severity)),
                );
                result.start_tackle_cooldown = true;
                Some(result)
            } else {
                let mut result = StateChangeResult::with_defender_state(
                    DefenderState::Pressing,
                );
                result.start_tackle_cooldown = true;
                Some(result)
            };
        } else {
            // Ball is loose - check for interception
            // Double-check not in flight before claiming
            if self.can_intercept_ball(ctx) && !ctx.ball().is_in_flight() {
                // Ball is loose and we can intercept it
                return Some(StateChangeResult::with_defender_state_and_event(
                    DefenderState::Running,
                    Event::PlayerEvent(PlayerEvent::ClaimBall(ctx.player.id)),
                ));
            }

            // If ball is too far away and not coming toward us, return to position
            let ball_distance = ctx.ball().distance();
            if ball_distance > RETURN_DISTANCE && !ctx.ball().is_towards_player_with_angle(0.8) {
                return Some(StateChangeResult::with_defender_state(
                    DefenderState::Returning,
                ));
            }

            // Fallback: if ball is loose and very close, try to claim it
            // Double-check not in flight before claiming
            if !ctx.tick_context.ball.is_owned && ball_distance < 5.0 && !ctx.ball().is_in_flight() {
                return Some(StateChangeResult::with_defender_state_and_event(
                    DefenderState::Running,
                    Event::PlayerEvent(PlayerEvent::ClaimBall(ctx.player.id)),
                ));
            }

            // If opponent is near the player but doesn't have the ball, maybe it's better to transition to pressing
            if let Some(close_opponent) = ctx.players().opponents().nearby(15.0).next() {
                if close_opponent.distance(ctx) < 10.0 {
                    return Some(StateChangeResult::with_defender_state(
                        DefenderState::Pressing,
                    ));
                }
            }
        }

        if ctx.in_state_time > 30 {
            let ball_distance = ctx.ball().distance();
            if ball_distance > PRESSING_DISTANCE {
                return Some(StateChangeResult::with_defender_state(DefenderState::Returning));
            }
            // Stuck in tackling too long without engaging — drop back to standing
            return Some(StateChangeResult::with_defender_state(DefenderState::Standing));
        }

        None
    }


    fn velocity(&self, ctx: &StateProcessingContext) -> Option<Vector3<f32>> {
        let target = self.calculate_intelligent_target(ctx);

        let tackling_skill = ctx.player.skills.technical.tackling / 20.0;
        let acceleration = ctx.player.skills.physical.acceleration / 20.0;
        // Explosive closing speed — skilled defenders close gaps faster
        let speed_boost = 1.4 + tackling_skill * 0.3 + acceleration * 0.3; // 1.4x - 2.0x

        Some(
            SteeringBehavior::Pursuit {
                target,
                target_velocity: Vector3::zeros(),
            }
                .calculate(ctx.player)
                .velocity * speed_boost
                + ctx.player().separation_velocity() * 0.15,
        )
    }

    fn process_conditions(&self, ctx: ConditionContext) {
        // Tackling is explosive and very demanding physically
        DefenderCondition::new(ActivityIntensity::VeryHigh).process(ctx);
    }
}

impl DefenderTacklingState {
    fn calculate_intelligent_target(&self, ctx: &StateProcessingContext) -> Vector3<f32> {
        let ball_position = ctx.tick_context.positions.ball.position;
        let player_position = ctx.player.position;
        let own_goal_position = ctx.ball().direction_to_own_goal();

        // Check if ball is dangerously close to own goal
        let ball_distance_to_own_goal = (ball_position - own_goal_position).magnitude();
        let is_ball_near_own_goal = ball_distance_to_own_goal < ctx.context.field_size.width as f32 * 0.2;

        // Check if we're between the ball and our goal
        let player_distance_to_own_goal = (player_position - own_goal_position).magnitude();
        let is_player_closer_to_goal = player_distance_to_own_goal < ball_distance_to_own_goal;

        if is_ball_near_own_goal && !is_player_closer_to_goal {
            // If ball is near our goal and we're not between ball and goal,
            // position ourselves between the ball and the goal
            let ball_to_goal_direction = (own_goal_position - ball_position).normalize();
            let intercept_distance = 5.0; // Stand 5 units in front of the ball towards our goal
            ball_position + ball_to_goal_direction * intercept_distance
        } else {
            // Otherwise, pursue the ball directly
            ball_position
        }
    }

    fn attempt_sliding_tackle(
        &self,
        ctx: &StateProcessingContext,
        opponent: &MatchPlayerLite,
    ) -> (bool, bool, FoulSeverity) {
        let mut rng = rand::rng();

        let tackling_skill = ctx.player.skills.technical.tackling / 20.0;
        let aggression = ctx.player.skills.mental.aggression / 20.0;
        let composure = ctx.player.skills.mental.composure / 20.0;
        let strength = ctx.player.skills.physical.strength / 20.0;

        // Defender composite: tackling is dominant, strength and composure support
        let overall_skill = tackling_skill * 0.50 + strength * 0.25 + composure * 0.25;

        let opponent_dribbling = ctx.player().skills(opponent.id).technical.dribbling / 20.0;
        let opponent_agility = ctx.player().skills(opponent.id).physical.agility / 20.0;

        let skill_difference = overall_skill - (opponent_dribbling + opponent_agility) / 2.0;

        // Defenders have home advantage in tackles — they pick the moment
        let success_chance = 0.62 + skill_difference * 0.40;
        let clamped_success_chance = success_chance.clamp(0.18, 0.95);

        let tackle_success = rng.random::<f32>() < clamped_success_chance;

        // Foul chance is skill-driven. Old formula produced 10-15% foul
        // per tackle attempt, which combined with the engine's ~300+
        // tackle-attempts-per-match rate meant 40+ fouls in the first
        // five minutes (real football: ~12-14 fouls per team per whole
        // match). New formula anchors at ~3-5% per attempt for average
        // players, scaling up to ~10-12% for the most aggressive and
        // down to <1% for composed, high-tackling defenders.
        //
        // Drivers (all 0..1 normalized):
        //   aggression   — dominant positive factor
        //   composure    — strong protective factor (picks the moment)
        //   tackling     — clean technical tackler doesn't need to foul
        // Clamped to a 0.5% floor so even an elite defender has some
        // risk on a 50/50 challenge.
        let base_foul = 0.02
            + aggression * 0.10
            - composure * 0.05
            - tackling_skill * 0.03;
        let base_foul = base_foul.max(0.005);

        // Clean successful tackles rarely foul — you won the ball
        // first. Missed tackles are the trailing-foot / mistimed-slide
        // scenario where fouls mostly happen.
        let foul_chance = if tackle_success {
            base_foul * 0.40
        } else {
            base_foul * 1.60
        };

        let committed_foul = rng.random::<f32>() < foul_chance;

        // Classify the foul. Missed tackles by aggressive players are
        // reckless; clean-skilled tackles that still trip up an opponent
        // are normal. Rare violent = very late + high-aggression.
        let severity = if !committed_foul {
            FoulSeverity::Normal
        } else if aggression > 0.75 && !tackle_success && rng.random::<f32>() < 0.12 {
            FoulSeverity::Violent
        } else if !tackle_success && aggression > 0.55 {
            FoulSeverity::Reckless
        } else {
            FoulSeverity::Normal
        };

        (tackle_success, committed_foul, severity)
    }

    fn exists_nearby(&self, ctx: &StateProcessingContext) -> bool {
        const DISTANCE: f32 = 30.0;

        ctx.players().opponents().exists(DISTANCE) || ctx.players().teammates().exists(DISTANCE)
    }

    fn can_intercept_ball(&self, ctx: &StateProcessingContext) -> bool {
        if self.exists_nearby(ctx) {
            return false;
        }

        let ball_position = ctx.tick_context.positions.ball.position;
        let ball_velocity = ctx.tick_context.positions.ball.velocity;
        let player_position = ctx.player.position;
        let player_speed = ctx.player.skills.physical.pace;

        if !ctx.tick_context.ball.is_owned && ball_velocity.magnitude() > 0.1 {
            let time_to_ball = (ball_position - player_position).magnitude() / player_speed;
            let ball_travel_distance = ball_velocity.magnitude() * time_to_ball;
            let ball_intercept_position =
                ball_position + ball_velocity.normalize() * ball_travel_distance;
            let player_intercept_distance = (ball_intercept_position - player_position).magnitude();

            player_intercept_distance <= TACKLE_DISTANCE_THRESHOLD
        } else {
            false
        }
    }
}
