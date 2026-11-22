use crate::r#match::defenders::states::DefenderState;
use crate::r#match::defenders::states::shared::{
    ActivityIntensity, DefenderCondition, DefenderRead, ReadLevel,
};
use crate::r#match::player::strategies::players::ops::skill_composites as sc;
use crate::r#match::player::strategies::shared::players::ops::defender_skill::DefenderSkillProfile;
use crate::r#match::player::strategies::shared::states::{
    BlockChallenge, ChallengeKind, ClosingPoint, RecoveryChallenge, TackleEngagement,
};
use crate::r#match::{
    ConditionContext, MatchContext, StateChangeResult, StateProcessingContext,
    StateProcessingHandler, SteeringBehavior,
};
use nalgebra::Vector3;

const CLOSE_PRESSING_DISTANCE: f32 = 25.0; // Wider close pressing zone for tight approach

#[derive(Default, Clone)]
pub struct DefenderPressingState {}

impl StateProcessingHandler for DefenderPressingState {
    /// The run to the man on the ball. Inside challenge range and goal-side
    /// of him he contains (`Jockeying`), whatever his tackle cooldown says;
    /// a man the carrier has gone past has the stretched challenge left to
    /// him and nothing else. A presser going with a pass stays on it while
    /// the plan names him.
    fn process(&self, ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        if let Some(change) = DefenderRead::change_above(ctx, ReadLevel::Engage) {
            return Some(change);
        }
        let Some(carrier) = ctx.players().opponents().with_ball().next() else {
            if TackleEngagement::is_nominated_presser(ctx) && !ctx.team().is_control_ball() {
                return None;
            }
            return Some(DefenderRead::change(ctx));
        };
        if !DefenderRead::is_man_for_carrier(ctx) {
            return Some(DefenderRead::change(ctx));
        }
        if DefenderRead::duel_rung(ctx, &carrier) == DefenderState::Jockeying {
            return Some(StateChangeResult::with_defender_state(
                DefenderState::Jockeying,
            ));
        }
        let distance = carrier.distance(ctx);
        let lead = RecoveryChallenge::lead(ctx, carrier.position);
        if RecoveryChallenge::is_available(ctx, distance, lead)
            && RecoveryChallenge::is_eligible(ctx)
            && ctx
                .context
                .rng
                .bernoulli(RecoveryChallenge::commits_now(ctx, distance, lead))
        {
            return Some(
                StateChangeResult::with_defender_state(DefenderState::Tackling)
                    .with_staged_challenge(ChallengeKind::Slide),
            );
        }
        None
    }

    fn velocity(&self, ctx: &StateProcessingContext) -> Option<Vector3<f32>> {
        // Move towards the opponent with the ball

        let opponents = ctx.players().opponents();
        let mut opponent_with_ball = opponents.with_ball();

        if let Some(opponent) = opponent_with_ball.next() {
            let distance_to_opponent = opponent.distance(ctx);

            // Intercept the carrier's future position based on their
            // velocity — stops them slipping past because the defender
            // was aiming at where they WERE, not where they're going.
            // Previously this was a 5u goal-side offset which meant a
            // running defender arrived ~5u behind the carrier and kept
            // chasing without ever making contact.
            let opp_velocity = ctx.tick_context.positions.players.velocity(opponent.id);
            let opp_speed = opp_velocity.magnitude();
            // Press boost from the unified profile (press_profile +
            // mobility composite). Tired / low-stamina chasers drop
            // into a softer press; elite pressers genuinely outsprint
            // journeyman carriers. Stays bounded by the original
            // calibrated envelope.
            let minute = sc::minute_from_ms(ctx.context.total_match_time);
            let def_profile = DefenderSkillProfile::from_ctx(ctx);
            let mobility = sc::mobility(ctx.player, minute);
            let press_composite = 0.65 * def_profile.press_profile + 0.35 * mobility;
            let press_boost = if BlockChallenge::duel_off() {
                (1.40 + (press_composite - 0.50) * 0.50).clamp(1.15, 1.65)
            } else {
                1.0
            };
            // A stride off the carrier, across his line to our box
            // (`ClosingPoint`). Through the middle that is the line to goal
            // at the old shot-zone step's size, so the shot corridor is
            // still covered.
            let intercept_target = if MatchContext::cross_line_off() {
                Self::goal_side_target(ctx, opponent.position, opp_speed)
            } else {
                ClosingPoint::for_carrier(ctx, &opponent)
            };

            // Steer rather than assign — see `MidfielderPressingState` for
            // the same change. `direction * speed` set an absolute
            // velocity of up to ~33 u/tick (pace 20 x press_boost 1.65)
            // against a ~0.63 u/tick top speed, ignoring the velocity the
            // defender already had, so the engine-wide clamp kept whatever
            // heading this tick produced. `press_boost` still applies, as
            // a multiplier on the achievable result.
            let pressing_velocity = SteeringBehavior::Pursuit {
                target: intercept_target,
                // Better readers anticipate more of the run; weaker ones
                // chase nearer the carrier's current position. Physical
                // pace still limits whether either defender can get there.
                target_velocity: opp_velocity * (0.5 + 0.5 * def_profile.defensive_reading),
            }
            .calculate(ctx.player)
            .velocity
                * press_boost;

            // Reduce separation velocity when actively pressing to allow close approach
            // When very close, disable separation entirely to enable tackling
            let separation = if distance_to_opponent < CLOSE_PRESSING_DISTANCE {
                ctx.player().separation_velocity() * 0.05 // Almost no separation when actively pressing
            } else {
                ctx.player().separation_velocity() * 0.15 // Minimal separation when pressing
            };

            return Some(pressing_velocity + separation);
        }

        // Loose ball nearby — pursue it. `Intercept` onto the shared
        // meeting point rather than `direction * pace`: `pace` is a 1-20
        // attribute, so that assigned an absolute velocity of up to 20
        // u/tick against a ~0.63 ceiling and threw away the velocity the
        // defender already had.
        if !ctx.ball().is_owned() && ctx.ball().distance() < 80.0 {
            return Some(
                SteeringBehavior::Intercept {
                    target: ctx.tick_context.positions.ball.position,
                    target_velocity: ctx.tick_context.positions.ball.velocity,
                }
                .calculate(ctx.player)
                .velocity,
            );
        }

        // Nobody on the ball and it is further out than the loose-ball
        // leg above: this is the presser going with a pass in flight.
        // `my_duty_anchor` resolves his `Press` duty to the point of
        // attack — the ball, when nobody owns it — which is the same
        // point `DutyAssigner` nominated him over, so the plan and the
        // run agree about where the press is happening.
        //
        // `Pursuit` rather than `Seek`, for the same reason the carrier
        // branch above uses it: a ball in flight is going somewhere, and
        // aiming at where it IS arrives behind it every time.
        if let Some(anchor) = ctx.team().my_duty_anchor() {
            return Some(
                SteeringBehavior::Pursuit {
                    target: anchor,
                    target_velocity: ctx.tick_context.positions.ball.velocity,
                }
                .calculate(ctx.player)
                .velocity,
            );
        }

        None
    }

    fn process_conditions(&self, ctx: ConditionContext) {
        // ⚠ CLOSING DOWN THE MAN ON THE BALL IS A SPRINT, AND THIS IS A
        // SPEED CAP.
        //
        // `High` is 0.78 of top speed (`MovementEffort::speed_fraction`).
        // The man being pressed is on the ball, and a carrier's ceiling
        // is a flat-out sprint scaled only by the carry cost — so the
        // presser was forbidden, by the movement layer, from ever
        // arriving. Measured before this: the carrier's ceiling 0.525
        // u/tick against his nearest opponent's 0.450, with the chaser
        // slower on 89% of ticks (`dead_ball_diag::CHASE_SAMPLES`), and
        // defenders producing 25.2 pressures a match for 1.12 successes
        // against a real ~11 for ~3.5. He was pressing constantly and
        // winning nothing because he could not close the last two metres.
        //
        // Same defect and same fix as `DefenderMarkingState` — see the
        // note there. The tier is a CEILING, so this does not make
        // defenders sprint all match: a presser shepherding a carrier who
        // is walking still walks.
        DefenderCondition::new(ActivityIntensity::chase()).process(ctx);
    }
}

impl DefenderPressingState {
    /// The `OF_CROSS_LINE_OFF` control arm: the carrier himself, stepped
    /// goal-side by a bias that grows inside shooting range.
    fn goal_side_target(
        ctx: &StateProcessingContext,
        carrier: Vector3<f32>,
        opp_speed: f32,
    ) -> Vector3<f32> {
        let predicted = carrier;

        // Bias predicted point toward the goal-side so we close the
        // shooting lane even on chase — the defender wants to be
        // BETWEEN the carrier and our goal, not just on top of them.
        // When the carrier is inside shooting range, ramp the
        // goal-side bias hard. This puts the defender squarely in the
        // shot line, which gives him a real chance to block the strike
        // via `try_block_shot`. Real football: a defender closing down
        // shows the shooter his body and steps along the shot line —
        // he doesn't just run at the ball.
        //
        // ⚠ WIDENING THIS TO REAL SHOOTING RANGE WAS TRIED AND
        // MEASURED NULL — do not retry it without a different reason.
        //
        // 80u is TEN METRES, and shots are struck from 124u (15.5 m)
        // on average (`block_diag::SHOT_RANGE_X100`, n=2 800 over 120
        // fixtures), so for the shot that actually happens this step
        // is off and the presser runs at the ball rather than across
        // it. That reads like the whole of the blocking problem, and
        // it is not: taking `SHOT_ZONE` to 240u (the 30 m
        // `DefenderMarkingState` uses for the same question) moved the
        // mean perpendicular distance of the defenders inside the
        // block window by 72.0u → **73.3u**, and blocks by 16.6% →
        // **16.5% of shots**. It cost 1.6 shots per team per match on
        // the way past (12.5 → 10.9 against a real ~13), because a
        // presser holding a containing position over the whole final
        // third suppresses the shot instead of blocking it.
        //
        // The reason is that the presser is ONE man. 41% of the back
        // line is `Marking` when a shot is struck and 14% `Pressing`,
        // so the corridor statistic is owned by the markers, whose
        // line is goal-side of their MAN and not of the ball. The
        // block rate lives in the marking geometry (see
        // `DefensiveLine::hold_shape_on_man`), not here.
        let own_goal = ctx.ball().direction_to_own_goal();
        let to_own_goal = (own_goal - predicted)
            .try_normalize(0.01)
            .unwrap_or_default();
        let carrier_to_goal = (own_goal - predicted).magnitude();
        let shot_zone_bias = if carrier_to_goal < 80.0 {
            // In shot zone: step 8-12u goal-side so we're actually
            // in the shot corridor. Heavier bias closer to goal.
            let zone_factor = 1.0 - (carrier_to_goal / 80.0).clamp(0.0, 1.0);
            8.0 + zone_factor * 4.0
        } else if opp_speed > 0.1 {
            2.0
        } else {
            0.0
        };
        predicted + to_own_goal * shot_zone_bias
    }
}
