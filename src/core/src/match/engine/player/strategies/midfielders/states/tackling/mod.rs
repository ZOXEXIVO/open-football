use crate::r#match::events::Event;
use crate::r#match::midfielders::states::MidfielderState;
use crate::r#match::midfielders::states::common::{ActivityIntensity, MidfielderCondition};
use crate::r#match::player::events::{FoulSeverity, PlayerEvent};
use crate::r#match::{
    ConditionContext, MatchPlayerLite, StateChangeResult, StateProcessingContext,
    StateProcessingHandler, SteeringBehavior,
};
use nalgebra::Vector3;
use rand::RngExt;

const TACKLE_DISTANCE_THRESHOLD: f32 = 12.0; // ~1.5m — midfielder ball-winner engagement range.

#[derive(Default, Clone)]
pub struct MidfielderTacklingState {}

impl StateProcessingHandler for MidfielderTacklingState {
    fn process(&self, ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        #[cfg(feature = "match-logs")]
        crate::tackle_stats::MID_ENTRIES.fetch_add(1, std::sync::atomic::Ordering::Relaxed);

        if ctx.player.has_ball(ctx) {
            return Some(StateChangeResult::with_midfielder_state(
                MidfielderState::Running,
            ));
        }

        // CRITICAL: Don't try to claim ball if it's in protected flight state
        // Transition OUT of tackling to avoid clustering around the ball carrier
        if ctx.ball().is_in_flight() {
            return Some(StateChangeResult::with_midfielder_state(
                MidfielderState::Running,
            ));
        }

        let ball_distance = ctx.ball().distance();

        if ball_distance > 150.0 {
            return Some(StateChangeResult::with_midfielder_state(
                MidfielderState::Returning,
            ));
        }

        // If ball is moving away but opponent still nearby, keep pressing
        if ball_distance > 80.0 && !ctx.ball().is_towards_player_with_angle(0.8) {
            return if ctx.team().is_control_ball() {
                Some(StateChangeResult::with_midfielder_state(
                    MidfielderState::AttackSupporting,
                ))
            } else {
                Some(StateChangeResult::with_midfielder_state(
                    MidfielderState::Pressing,
                ))
            };
        }

        // Per-player tackle cooldown. Midfielders re-enter Tackling via
        // Pressing, Running, and Standing roles; without a shared cooldown
        // each one re-fires a tackle attempt next tick, driving fouls and
        // successful tackles 5-10× above real-football rates.
        if !ctx.player.can_attempt_tackle() {
            return Some(StateChangeResult::with_midfielder_state(
                MidfielderState::Pressing,
            ));
        }

        // Closest-teammate duel gate. Midfielders were the single biggest
        // tackle-event source (208/team/match vs real ~8) because 3-4 of
        // them inside the 50u pressing radius simultaneously entered
        // Tackling. Only the best-positioned one actually engages; the
        // rest revert to Pressing to cover passing lanes.
        if !ctx.team().is_best_player_to_chase_ball() {
            return Some(StateChangeResult::with_midfielder_state(
                MidfielderState::Pressing,
            ));
        }

        let opponents = ctx.players().opponents();
        let mut opponents_with_ball = opponents.with_ball();

        if let Some(opponent) = opponents_with_ball.next() {
            let opponent_distance = ctx.tick_context.grid.get(ctx.player.id, opponent.id);
            if opponent_distance <= TACKLE_DISTANCE_THRESHOLD {
                #[cfg(feature = "match-logs")]
                crate::tackle_stats::MID_ATTEMPTS
                    .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                let (tackle_success, committed_foul, foul_severity) =
                    self.attempt_tackle(ctx, &opponent);
                if tackle_success {
                    // Double-check ball is not in flight before claiming.
                    if !ctx.ball().is_in_flight() {
                        #[cfg(feature = "match-logs")]
                        crate::tackle_stats::MID_SUCCESSES
                            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                        let mut result = StateChangeResult::with_midfielder_state_and_event(
                            MidfielderState::Standing,
                            Event::PlayerEvent(PlayerEvent::TacklingBall(ctx.player.id)),
                        );
                        result.start_tackle_cooldown = true;
                        return Some(result);
                    }
                } else if committed_foul {
                    let mut result = StateChangeResult::with_midfielder_state_and_event(
                        MidfielderState::Standing,
                        Event::PlayerEvent(PlayerEvent::CommitFoul(ctx.player.id, foul_severity)),
                    );
                    result.start_tackle_cooldown = true;
                    return Some(result);
                } else {
                    // Missed tackle, no foul — still cooldown so we don't
                    // re-attempt next tick
                    let mut result =
                        StateChangeResult::with_midfielder_state(MidfielderState::Pressing);
                    result.start_tackle_cooldown = true;
                    return Some(result);
                }
            }
        } else if self.can_intercept_ball(ctx) {
            // can_intercept_ball already checks is_in_flight
            return Some(StateChangeResult::with_midfielder_state_and_event(
                MidfielderState::Running,
                Event::PlayerEvent(PlayerEvent::ClaimBall(ctx.player.id)),
            ));
        }

        None
    }

    fn velocity(&self, ctx: &StateProcessingContext) -> Option<Vector3<f32>> {
        let tackling_skill = ctx.player.skills.technical.tackling / 20.0;
        let pace = ctx.player.skills.physical.acceleration / 20.0;
        // Explosive closing speed — skilled tacklers close gaps faster
        let speed_boost = 1.3 + tackling_skill * 0.3 + pace * 0.3; // 1.3x - 1.9x

        Some(
            SteeringBehavior::Pursuit {
                target: ctx.tick_context.positions.ball.position,
                target_velocity: ctx.tick_context.positions.ball.velocity,
            }
            .calculate(ctx.player)
            .velocity
                * speed_boost
                + ctx.player().separation_velocity() * 0.2,
        )
    }

    fn process_conditions(&self, ctx: ConditionContext) {
        // Tackling is explosive and very demanding physically
        MidfielderCondition::new(ActivityIntensity::VeryHigh).process(ctx);
    }
}

impl MidfielderTacklingState {
    /// Attempts a tackle and returns whether it was successful and if a foul was committed.
    fn attempt_tackle(
        &self,
        ctx: &StateProcessingContext,
        opponent: &MatchPlayerLite,
    ) -> (bool, bool, FoulSeverity) {
        let mut rng = rand::rng();

        let tackling_skill = ctx.player.skills.technical.tackling / 20.0;
        let aggression = ctx.player.skills.mental.aggression / 20.0;
        let composure = ctx.player.skills.mental.composure / 20.0;

        let overall_skill = (tackling_skill + composure) / 2.0;

        let opponent_dribbling = ctx.player().skills(opponent.id).technical.dribbling / 20.0;
        let opponent_agility = ctx.player().skills(opponent.id).physical.agility / 20.0;

        let skill_difference = overall_skill - (opponent_dribbling + opponent_agility) / 2.0;

        // Midfielders contest rather than commit — base anchored 0.05
        // below the defender's. Tracks the second defender base drop
        // 0.35→0.25 to keep MID/DEF success ratios stable while total
        // tackles converge on the real ~18/team/match.
        let success_chance = 0.20 + skill_difference * 0.35;
        let clamped_success_chance = success_chance.clamp(0.05, 0.62);

        let tackle_success = rng.random::<f32>() < clamped_success_chance;

        // Skill-driven foul rate — see defender tackling for the rationale.
        // Same curve, applied to midfielders because they also tackle
        // frequently during pressing.
        let base_foul = 0.02 + aggression * 0.10 - composure * 0.05 - tackling_skill * 0.03;
        let base_foul = base_foul.max(0.005);

        let foul_chance = if tackle_success {
            base_foul * 0.40
        } else {
            base_foul * 1.60
        };

        let committed_foul = rng.random::<f32>() < foul_chance;

        let severity = if !committed_foul {
            FoulSeverity::Normal
        } else if aggression > 0.75 && !tackle_success && rng.random::<f32>() < 0.10 {
            FoulSeverity::Violent
        } else if !tackle_success && aggression > 0.55 {
            FoulSeverity::Reckless
        } else {
            FoulSeverity::Normal
        };

        (tackle_success, committed_foul, severity)
    }

    fn can_intercept_ball(&self, ctx: &StateProcessingContext) -> bool {
        if ctx.ball().is_in_flight() {
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
