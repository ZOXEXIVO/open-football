use crate::r#match::events::Event;
use crate::r#match::forwarders::states::ForwardState;
use crate::r#match::forwarders::states::common::{ActivityIntensity, ForwardCondition};
use crate::r#match::player::events::{FoulSeverity, PlayerEvent};
use crate::r#match::player::strategies::common::states::{TackleDecision, TackleEngagement};
use crate::r#match::player::strategies::players::ops::skill_composites as sc;
use crate::r#match::player::strategies::players::skills::SkillCurve;
use crate::r#match::{
    ConditionContext, MatchPlayerLite, StateChangeResult, StateProcessingContext,
    StateProcessingHandler, SteeringBehavior,
};
use nalgebra::Vector3;

const TACKLE_DISTANCE_THRESHOLD: f32 = 8.0; // ~1m — forwards rarely tackle from range. Tightened from 12u after dev_match showed FWD tackles at 15/match/team vs real ~2.
// `CLOSE_TACKLE_DISTANCE` is gone with the per-tick immediate-attempt
// branch it gated — `TackleDecision` prices proximity continuously
// through its `reach` term, so a separate "right on top of him" range
// with its own unconditional roll is exactly the double-counting the
// jockey model exists to remove.
const FOUL_CHANCE_BASE: f32 = 0.15; // Base chance of committing a foul
const CHASE_DISTANCE_THRESHOLD: f32 = 100.0; // Maximum distance to chase for tackle
const PRESSURE_DISTANCE: f32 = 20.0; // Distance to apply pressure without tackling

#[derive(Default, Clone)]
pub struct ForwardTacklingState {}

impl StateProcessingHandler for ForwardTacklingState {
    fn process(&self, ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        #[cfg(feature = "match-logs")]
        crate::tackle_stats::FWD_ENTRIES.fetch_add(1, std::sync::atomic::Ordering::Relaxed);

        // If player has gained possession, transition to running
        if ctx.player.has_ball(ctx) {
            return Some(StateChangeResult::with_forward_state(ForwardState::Running));
        }

        // CRITICAL: Don't try to claim ball if it's in protected flight state
        // Transition OUT of tackling to avoid clustering around the ball carrier
        if ctx.ball().is_in_flight() {
            return Some(StateChangeResult::with_forward_state(ForwardState::Running));
        }

        // Per-player tackle cooldown. Without it a forward in Tackling
        // state attempts a fresh tackle every tick — 100 attempts × 15%
        // base foul chance = 15 fouls per forward per match, and with
        // three forwards on the field that compounds into the 150+
        // team-foul counts seen in the metrics.
        if !ctx.player.can_attempt_tackle() {
            return Some(StateChangeResult::with_forward_state(
                ForwardState::Pressing,
            ));
        }

        // Closest-teammate duel gate — see def/mid tackling for rationale.
        // Forwards rarely lead the team in chase-score, so this mostly
        // defers the counter-press to whichever midfielder is closer.
        // Entry condition only. `TackleEngagement::should_commit` already
        // applies this before anyone is sent here, so re-checking it on
        // every tick could only ever ABANDON a challenge already under
        // way — and the designation is a tolerance band that swaps
        // between team-mates tick to tick, so it did exactly that. A
        // committed engagement now runs to contact or to `DISENGAGE`.
        // …and the plan's nomination WINS it, with the chase election as
        // the fallback for a ball the plan declines to nominate for. A
        // forward who is merely NEAR the duel does not take it off the
        // defender whose duel it is — see
        // `TackleEngagement::may_engage_carrier`.
        if ctx.in_state_time == 0 && !TackleEngagement::may_engage_carrier(ctx) {
            return Some(StateChangeResult::with_forward_state(
                ForwardState::Pressing,
            ));
        }

        // Skill gate — most strikers don't drill defensive tackles
        // (Haaland, Mbappe profile). Sigmoid pivot at 8/20: a tackling=4
        // pure attacker very rarely commits to a tackle; a tackling=14
        // ball-winning forward almost always does. Smooth replacement
        // for the hard cliff that flattened the 1-8 range.
        let tackling_p =
            SkillCurve::new(ctx.player.skills.technical.tackling, 8.0, 0.6).probability();
        if ctx.context.rng.unit_f32() >= tackling_p {
            return Some(StateChangeResult::with_forward_state(
                ForwardState::Pressing,
            ));
        }

        let opponents = ctx.players().opponents();

        if let Some(opponent) = opponents.with_ball().next() {
            let opponent_distance = ctx.tick_context.grid.get(ctx.player.id, opponent.id);
            // JOCKEY FIRST — the same model the back line and the
            // midfield use, and the one this state was never given.
            //
            // `DefenderTacklingState` and `MidfielderTacklingState` route
            // every challenge through [`TackleDecision`]: one roll per
            // SECOND while containing, priced by temperament, cover,
            // danger, the carrier's committed weight and the angle.
            // This state kept the shape both of those were fixed away
            // from — an attempt rolled EVERY TICK inside 5u, and a
            // second per-tick roll (`should_attempt_tackle_now`) inside
            // 8u — bounded only by the ~1 s tackle cooldown.
            //
            // Measured over 120 fixtures, that is 78 AI ticks per attempt
            // against the back line's 409: **4.81 tackles per forward per
            // match against a real ~0.8**, and forwards out-tackling
            // defenders 2:1 in an engine whose own `TackleEngagement`
            // note says that ladder is upside down. It is also most of
            // the engine's foul surplus, since a foul here is a failed
            // challenge.
            //
            // A forward pressing does not lunge at every touch; he
            // shepherds, and goes in when the moment is there. Declining
            // keeps him in the state, containing, exactly as it does for
            // a defender.
            if opponent_distance <= TACKLE_DISTANCE_THRESHOLD {
                if !TackleDecision::is_decision_tick(ctx)
                    || !ctx
                        .context
                        .rng
                        .bernoulli(TackleDecision::commit_probability(ctx, opponent_distance))
                {
                    return None; // contain
                }
                #[cfg(feature = "match-logs")]
                crate::tackle_stats::FWD_ATTEMPTS
                    .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                let (tackle_success, committed_foul, foul_severity) =
                    self.attempt_tackle(ctx, &opponent);

                if committed_foul {
                    let mut result = StateChangeResult::with_forward_state_and_event(
                        ForwardState::Standing,
                        Event::PlayerEvent(PlayerEvent::CommitFoul(ctx.player.id, foul_severity)),
                    );
                    result.start_tackle_cooldown = true;
                    return Some(result);
                }

                if tackle_success {
                    // Double-check ball is not in flight before claiming
                    if !ctx.ball().is_in_flight() {
                        #[cfg(feature = "match-logs")]
                        crate::tackle_stats::FWD_SUCCESSES
                            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                        let mut result = StateChangeResult::with_forward_state_and_event(
                            ForwardState::Running,
                            Event::PlayerEvent(PlayerEvent::TacklingBall(ctx.player.id)),
                        );
                        result.start_tackle_cooldown = true;
                        return Some(result);
                    }
                }

                // Missed tackle — cooldown
                let mut result = StateChangeResult::with_forward_state(ForwardState::Pressing);
                result.start_tackle_cooldown = true;
                return Some(result);
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
                return Some(StateChangeResult::with_forward_state(
                    ForwardState::Returning,
                ));
            }

            return Some(StateChangeResult::with_forward_state(
                ForwardState::Assisting,
            ));
        } else if ball_distance <= PRESSURE_DISTANCE {
            return Some(StateChangeResult::with_forward_state(
                ForwardState::Pressing,
            ));
        }

        None
    }

    fn velocity(&self, ctx: &StateProcessingContext) -> Option<Vector3<f32>> {
        let opponents = ctx.players().opponents();

        if let Some(opponent) = opponents.with_ball().next() {
            let opponent_distance = ctx.tick_context.grid.get(ctx.player.id, opponent.id);

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
    /// Attempt a tackle with improved physics and skill-based calculation
    fn attempt_tackle(
        &self,
        ctx: &StateProcessingContext,
        opponent: &MatchPlayerLite,
    ) -> (bool, bool, FoulSeverity) {
        let rng = &ctx.context.rng;

        // Aggression and composure still feed the foul-risk path
        // (they should — composure protects, aggression escalates),
        // but the duel resolution itself routes through the duel
        // composites so the tackler/carrier read consistent with the
        // rest of the engine.
        let aggression = ctx.player.skills.mental.aggression / 20.0;
        let composure = ctx.player.skills.mental.composure / 20.0;

        // Calculate relative positioning advantage
        let distance = ctx.tick_context.grid.get(ctx.player.id, opponent.id);
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

        // Duel resolution via shared composites. `defensive_duel`
        // (tackler) vs `dribble_attack` (carrier) — both are 0..1 and
        // already fatigue-folded, so the raw `base_success * 0.4`
        // mapping below stays inside its calibrated band.
        let minute = sc::minute_from_ms(ctx.context.total_match_time);
        let player_tackle_ability = sc::defensive_duel(ctx.player, minute);
        let opponent_evasion_ability = match ctx.context.players.by_id(opponent.id) {
            Some(opp) => sc::dribble_attack(opp, minute),
            None => 0.50,
        };

        // Final success calculation. Forward counter-press tackle
        // success in real football is the lowest of the three roles —
        // ~15-25% — because forwards are ahead of the play, off-balance,
        // and don't drill defensive technique. Base 0.15.
        let base_success = player_tackle_ability - opponent_evasion_ability;
        let situational_bonus = distance_factor * 0.3 + tackle_angle_factor * 0.2;
        let success_chance = (0.15 + base_success * 0.4 + situational_bonus).clamp(0.03, 0.60);

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
            (foul_base_risk * 0.3) + aggression_risk + desperation_risk + situation_risk
                - skill_protection
        } else {
            // Higher foul chance for failed tackles
            foul_base_risk + aggression_risk + desperation_risk + situation_risk + 0.05
                - skill_protection
        };

        let foul_chance = foul_chance.clamp(0.0, 0.4); // Cap maximum foul chance
        let committed_foul = rng.random::<f32>() < foul_chance;

        // Forwards rarely go studs-up; tackling-from-behind (low angle factor)
        // or desperation-late-in-match pushes severity up.
        // Violent 0.10 → 0.02, Reckless gated at 0.35 — aligned with the
        // 2026-06 discipline recalibration (see defenders/tackling):
        // the engine produced ~1.0 reds/match vs real ~0.15 because most
        // failed aggressive contact escalated straight past Normal.
        let behind_tackle = tackle_angle_factor < 0.3;
        let severity = if !committed_foul {
            FoulSeverity::Normal
        } else if behind_tackle && aggression > 0.7 && rng.random::<f32>() < 0.02 {
            FoulSeverity::Violent
        } else if !tackle_success
            && (behind_tackle || aggression > 0.55)
            && rng.random::<f32>() < 0.35
        {
            FoulSeverity::Reckless
        } else {
            FoulSeverity::Normal
        };

        (tackle_success, committed_foul, severity)
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
                let closest_opponent_distance = ctx
                    .players()
                    .opponents()
                    .all()
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
                let closest_opponent_distance = ctx
                    .players()
                    .opponents()
                    .all()
                    .map(|opp| (ball_position - opp.position).magnitude())
                    .min_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal))
                    .unwrap_or(f32::MAX);

                return ball_distance < closest_opponent_distance * 0.8;
            }
        }

        false
    }
}
