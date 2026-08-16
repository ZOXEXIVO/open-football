use crate::r#match::midfielders::states::MidfielderState;
use crate::r#match::midfielders::states::common::{
    ActivityIntensity, Interception, MidfielderCondition,
};
use crate::r#match::player::strategies::common::players::MatchPlayerIteratorExt;
use crate::r#match::player::strategies::common::players::ops::midfielder_skill::MidfielderSkillProfile;
use crate::r#match::player::strategies::common::states::TackleEngagement;
use crate::r#match::{
    ConditionContext, StateChangeResult, StateProcessingContext, StateProcessingHandler,
    SteeringBehavior,
};
use nalgebra::Vector3;

#[derive(Default, Clone)]
pub struct MidfielderPressingState {}

impl StateProcessingHandler for MidfielderPressingState {
    fn process(&self, ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        if ctx.player.has_ball(ctx) {
            return Some(StateChangeResult::with_midfielder_state(
                MidfielderState::Running,
            ));
        }

        // Ball coming toward this player (pass to us) — intercept it
        if Interception::is_available(ctx)
            && ctx.ball().is_towards_player_with_angle(0.8)
            && ctx.ball().distance() < 150.0
        {
            return Some(StateChangeResult::with_midfielder_state(
                MidfielderState::Intercepting,
            ));
        }

        // Loose ball very close — take it regardless of speed
        if Interception::is_available(ctx) && ctx.ball().distance() < 30.0 {
            return Some(StateChangeResult::with_midfielder_state(
                MidfielderState::TakeBall,
            ));
        }

        // If team has possession, stop pressing and contribute to attack
        if ctx.team().is_control_ball() {
            return Some(StateChangeResult::with_midfielder_state(
                MidfielderState::AttackSupporting,
            ));
        }

        // Opponent GK holds ball in hands — can't challenge, back off
        if ctx.ball().is_held_by_opponent_goalkeeper() {
            return Some(StateChangeResult::with_midfielder_state(
                MidfielderState::Running,
            ));
        }

        // Press time budget: combines the team-shared press intensity
        // with the midfielder's own pressing_profile (work_rate/stamina/
        // anticipation/positioning blend, dampened by condition). Poor
        // pressers and tired midfielders give up earlier; elite pressers
        // sustain longer hunts.
        let intensity = ctx.team().press_intensity();
        let mid_profile = MidfielderSkillProfile::from_ctx(ctx);
        let max_press_time = mid_profile.press_time_ticks(intensity);
        if ctx.in_state_time > max_press_time {
            return Some(StateChangeResult::with_midfielder_state(
                MidfielderState::Running,
            ));
        }

        // Back off during foul protection — don't crowd the free kick
        if ctx.ball().is_in_flight() && ctx.ball().is_owned() {
            return Some(StateChangeResult::with_midfielder_state(
                MidfielderState::Running,
            ));
        }

        // Hand the press over when somebody else is genuinely taking it.
        //
        // ⚠ This used to break off whenever ANY team-mate was 10 units —
        // **1.25 metres** — closer to the ball. In a defending block that
        // is true of everybody except the single closest man, on almost
        // every tick, so the state emptied itself: one player chased and
        // the other nine dropped straight back into `Running`. It is the
        // mechanical half of the report "no opposing player is pressing,
        // or only one player is pressing".
        //
        // Two things replace it. The plan already nominates ONE engager
        // (`DefensiveDuty::Press`) and the whole defensive model treats
        // that as authoritative, so a nominated presser does not defer to
        // anybody. Everyone else hands over only when a team-mate is
        // clearly on it — `HANDOVER_MARGIN` is a hand-over distance, not
        // a tie-break — and what he does instead is his own duty, which
        // is now a MARK on one of the carrier's outlets rather than
        // nothing at all (see `teamplay::defence::BALL_THREAT_RADIUS`).
        const HANDOVER_MARGIN: f32 = 60.0;
        let ball_distance = ctx.ball().distance();
        let ball_position = ctx.tick_context.positions.ball.position;

        if !TackleEngagement::is_nominated_presser(ctx) {
            if let Some(closest_teammate) = ctx
                .players()
                .teammates()
                .all()
                .filter(|t| t.id != ctx.player.id)
                .min_by(|a, b| {
                    let dist_a = (a.position - ball_position).magnitude();
                    let dist_b = (b.position - ball_position).magnitude();
                    dist_a.total_cmp(&dist_b)
                })
            {
                let teammate_distance = (closest_teammate.position - ball_position).magnitude();

                if teammate_distance < ball_distance - HANDOVER_MARGIN {
                    return Some(StateChangeResult::with_midfielder_state(
                        MidfielderState::Running,
                    ));
                }
            }
        }

        // CRITICAL: Tackle if an opponent has the ball nearby
        if let Some(opponent) = ctx.players().opponents().nearby(60.0).with_ball(ctx).next() {
            let opponent_distance = (opponent.position - ctx.player.position).magnitude();

            // Engage only at genuine challenge range. This was 50u
            // (~6 m) — a distance at which no one can tackle anyone —
            // while `MidfielderTacklingState` breaks off far closer, so
            // the pair oscillated for every carrier in between (~35k
            // round trips a match). `should_commit` also refuses while
            // the tackle cooldown is running, which is what stops a
            // midfielder who has just lunged from being fed back into a
            // state that can only hand him out again.
            if TackleEngagement::should_commit(ctx, opponent_distance) {
                return Some(StateChangeResult::with_midfielder_state(
                    MidfielderState::Tackling,
                ));
            }
        }

        // Loose ball nearby — go claim it directly instead of pressing thin air
        // But only if no teammate is closer (avoid chasing ball during teammate's pass)
        if !ctx.ball().is_owned() && ball_distance < 50.0 && ctx.ball().speed() < 3.0 {
            let ball_pos = ctx.tick_context.positions.ball.position;
            let cutoff = ball_distance - 5.0;
            let closer_teammate = cutoff > 0.0
                && ctx
                    .players()
                    .teammates()
                    .nearby_at(ball_pos, cutoff)
                    .next()
                    .is_some();

            if !closer_teammate {
                return Some(StateChangeResult::with_midfielder_state(
                    MidfielderState::TakeBall,
                ));
            }
        }

        // Only give up pressing if ball is truly far away
        if ctx.ball().distance() > 300.0 {
            return Some(StateChangeResult::with_midfielder_state(
                MidfielderState::Returning,
            ));
        }

        // Check if the pressing is ineffective (opponent still has ball after some time)
        if ctx.in_state_time > 30 && !self.is_making_progress(ctx) {
            return Some(StateChangeResult::with_midfielder_state(
                MidfielderState::Running,
            ));
        }

        None
    }

    fn velocity(&self, ctx: &StateProcessingContext) -> Option<Vector3<f32>> {
        // Only pursue if opponent has the ball
        if let Some(opponent) = ctx
            .players()
            .opponents()
            .nearby(500.0)
            .with_ball(ctx)
            .next()
        {
            let distance_to_opponent = (opponent.position - ctx.player.position).magnitude();

            // Predictive pursuit — lead the carrier based on their
            // current velocity. Aiming at their present position means
            // arriving several ticks behind a running attacker. Same
            // fix applied to defender pressing: estimate how long we'd
            // take to close the gap at our own pace, project the
            // carrier along their velocity by that many ticks, then bias
            // the aim point slightly toward our own goal to close the
            // shooting lane rather than just touch their back.
            let opp_velocity = ctx.tick_context.positions.players.velocity(opponent.id);
            let opp_speed = opp_velocity.magnitude();

            // Goal-side bias: close the shooting lane rather than just
            // touch the carrier's back. The interception itself is left to
            // `SteeringBehavior::Pursuit`.
            //
            // The lead time used to be hand-rolled here as
            // `distance / pace`, mixing units: `pace` is a 1-20 SKILL,
            // while distance is in field units and the answer was used as
            // ticks. At pace 15 that put a carrier 50u away only ~3 ticks
            // ahead when closing him actually takes ~100 — the prediction
            // was ~30x short and moved erratically as the distance
            // changed. `Pursuit` computes the same thing from the
            // player's real u/tick speed.
            // Goal-side bias only; the interception maths belongs to
            // `SteeringBehavior::Pursuit` below, which derives the lead
            // from the player's real u/tick speed.
            //
            // The lead time used to be hand-rolled here as
            // `distance / pace`, mixing units: `pace` is a 1-20 SKILL,
            // while distance is in field units and the result was used as
            // a tick count. At pace 15 that put a carrier 50u away only
            // ~3 ticks ahead when closing him actually takes ~100 — the
            // prediction was ~30x short and lurched as the gap changed,
            // which moved the aim point (and the presser's heading) tick
            // to tick.
            let predicted = opponent.position;

            let own_goal = ctx.ball().direction_to_own_goal();
            let to_own_goal = (own_goal - predicted).normalize();
            let goalside_bias = if opp_speed > 0.1 { 2.0 } else { 0.0 };
            let intercept_target = predicted + to_own_goal * goalside_bias;

            // Steer, don't teleport. `direction * pace` assigned an
            // absolute velocity of up to 20 u/tick against a ~0.63 u/tick
            // top speed, with no reference to the velocity the player
            // already had — so the engine-wide clamp kept whatever heading
            // this tick's aim point produced, and the press could invert
            // between consecutive ticks. `Pursuit` integrates from the
            // current velocity under a force limit, as every other
            // well-behaved state does. This took `Midfielder: Pressing`
            // from 8.6 velocity reversals per second held to below the
            // reporting threshold (`dev_match trace`).
            let pressing_velocity = SteeringBehavior::Pursuit {
                target: intercept_target,
                target_velocity: opp_velocity,
            }
            .calculate(ctx.player)
            .velocity;

            // Reduce separation when actively pressing to allow close approach
            let separation = if distance_to_opponent < 25.0 {
                ctx.player().separation_velocity() * 0.05
            } else {
                ctx.player().separation_velocity() * 0.15
            };

            Some(pressing_velocity + separation)
        } else if Interception::is_available(ctx) && ctx.ball().distance() < 100.0 {
            // Loose ball — pursue it directly
            let direction =
                (ctx.tick_context.positions.ball.position - ctx.player.position).normalize();
            let speed = ctx.player.skills.physical.pace;
            Some(direction * speed)
        } else {
            // Teammate has ball — maintain position
            Some(Vector3::zeros())
        }
    }

    fn process_conditions(&self, ctx: ConditionContext) {
        // Pressing is high intensity - sustained running and pressure
        MidfielderCondition::with_velocity(ActivityIntensity::High).process(ctx);
    }
}

impl MidfielderPressingState {
    // New helper function to determine if pressing is making progress
    fn is_making_progress(&self, ctx: &StateProcessingContext) -> bool {
        let player_velocity = ctx.player.velocity;

        // Calculate dot product between player velocity and direction to ball
        let to_ball = ctx.tick_context.positions.ball.position - ctx.player.position;
        let to_ball_normalized = if to_ball.magnitude() > 0.0 {
            to_ball / to_ball.magnitude()
        } else {
            Vector3::new(0.0, 0.0, 0.0)
        };

        let moving_toward_ball = player_velocity.dot(&to_ball_normalized) > 0.0;

        // Check if other teammates are better positioned to press
        let other_pressing_teammates = ctx
            .players()
            .teammates()
            .all()
            .filter(|t| {
                let dist = (t.position - ctx.tick_context.positions.ball.position).magnitude();
                dist < ctx.ball().distance() * 0.8 // 20% closer than current player
            })
            .count();

        // Continue pressing if moving toward ball and not many better-positioned teammates
        moving_toward_ball && other_pressing_teammates < 2
    }
}
