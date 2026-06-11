use crate::r#match::events::Event;
use crate::r#match::midfielders::states::MidfielderState;
use crate::r#match::midfielders::states::common::{ActivityIntensity, MidfielderCondition};
use crate::r#match::player::events::{FoulSeverity, PlayerEvent};
use crate::r#match::player::strategies::common::players::ops::midfielder_skill::MidfielderSkillProfile;
use crate::r#match::{
    ConditionContext, MatchPlayerLite, PlayerSide, StateChangeResult, StateProcessingContext,
    StateProcessingHandler, SteeringBehavior,
};
use nalgebra::Vector3;
#[cfg(feature = "match-logs")]
use std::sync::atomic::Ordering;

const TACKLE_DISTANCE_THRESHOLD: f32 = 8.0; // ~1m — midfielder ball-winner contact range. Tightened from 12u after dev_match showed MID tackles at 21/match/team vs real ~4: the 12u "engagement" radius was a soft press circle, not a tackle zone. Real midfielders win the ball by getting CLOSE to the carrier; the 8u threshold matches the actual contact distance.

#[derive(Default, Clone)]
pub struct MidfielderTacklingState {}

impl StateProcessingHandler for MidfielderTacklingState {
    fn process(&self, ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        #[cfg(feature = "match-logs")]
        crate::tackle_stats::MID_ENTRIES.fetch_add(1, Ordering::Relaxed);

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
                crate::tackle_stats::MID_ATTEMPTS.fetch_add(1, Ordering::Relaxed);
                let (tackle_success, committed_foul, foul_severity) =
                    self.attempt_tackle(ctx, &opponent);
                if tackle_success {
                    // Double-check ball is not in flight before claiming.
                    if !ctx.ball().is_in_flight() {
                        #[cfg(feature = "match-logs")]
                        crate::tackle_stats::MID_SUCCESSES.fetch_add(1, Ordering::Relaxed);
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
    /// Uses the unified midfielder profile's `tackle_profile` and
    /// `discipline` instead of raw `(tackling+composure)/2` blends.
    fn attempt_tackle(
        &self,
        ctx: &StateProcessingContext,
        opponent: &MatchPlayerLite,
    ) -> (bool, bool, FoulSeverity) {
        let rng = &ctx.context.rng;

        let mid_profile = MidfielderSkillProfile::from_ctx(ctx);
        let aggression01 = (ctx.player.skills.mental.aggression / 20.0).clamp(0.0, 1.0);

        // Opponent carry profile via a dribble_attack-shaped blend.
        // Without a direct MatchPlayer ref for the opponent we read the
        // skill snapshot via the helper and apply the composite weights
        // in-place; mirrors the structure of `sc::dribble_attack`.
        let opp_helper = ctx.player();
        let opponent_carry = {
            let opp_skills = opp_helper.skills(opponent.id);
            ((opp_skills.technical.dribbling / 20.0) * 0.30
                + (opp_skills.technical.technique / 20.0) * 0.20
                + (opp_skills.physical.agility / 20.0) * 0.20
                + (opp_skills.physical.acceleration / 20.0) * 0.12
                + (opp_skills.physical.balance / 20.0) * 0.10
                + (opp_skills.mental.composure / 20.0) * 0.08)
                .clamp(0.0, 1.0)
        };

        // Logistic success: tackle_profile vs opponent carry. Same trim
        // as `defenders/tackling`: 3.0 → 2.4 sigmoid and cap 0.70 → 0.55.
        // Equal-skill matchups still resolve at 0.50 so calibration-
        // neutral. Asymmetric softening of the strong-mid-vs-weak-
        // carrier rate that compounded with the defender tackle to
        // crush weak teams' possession survival.
        let raw_diff = mid_profile.tackle_profile - opponent_carry;
        let logistic = 1.0 / (1.0 + (-raw_diff * 2.4).exp());
        let success_chance = logistic.clamp(0.06, 0.55);
        let tackle_success = rng.random::<f32>() < success_chance;

        // Foul model driven by discipline (composure/decisions/tackling/
        // concentration blend) instead of raw composure/aggression.
        // Base 0.025 → 0.044 — see defenders/tackling for the 2026-06
        // discipline recalibration rationale (fouls ran at half the
        // real rate; reds at ~6× it).
        // 0.044 → 0.062 in the second lift — see defenders/tackling.
        let mut base_foul = 0.062 + aggression01 * 0.11 - mid_profile.discipline * 0.07;
        if !tackle_success {
            base_foul *= 1.75;
        }
        // Tired midfielders foul more.
        base_foul += (1.0 - mid_profile.mid_condition_mult).max(0.0) * 0.08;
        // Own-box restraint — same rationale as the defender model:
        // nobody dives in inside their own penalty area. Rectangle
        // check matches the restart-award geometry.
        let in_own_box = ctx
            .context
            .penalty_area(ctx.player.side == Some(PlayerSide::Left))
            .contains(&ctx.tick_context.positions.ball.position);
        if in_own_box {
            base_foul *= 0.30;
        }
        // Self-preservation on a booking — see defenders/tackling.
        if ctx.player.yellow_cards > 0 {
            base_foul *= 0.70;
        }
        let foul_chance = base_foul.max(0.005);

        let committed_foul = rng.random::<f32>() < foul_chance;

        // Violent 0.10 → 0.02, Reckless gated at 0.35 — most failed
        // contact is a plain foul, not a card-worthy lunge.
        let severity = if !committed_foul {
            FoulSeverity::Normal
        } else if aggression01 > 0.75 && !tackle_success && rng.random::<f32>() < 0.008 {
            FoulSeverity::Violent
        } else if !tackle_success && aggression01 > 0.55 && rng.random::<f32>() < 0.35 {
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
