use crate::r#match::engine::ball::ball::HandlingVerdict;
use crate::r#match::goalkeepers::states::common::{
    ActivityIntensity, GoalkeeperCondition, KeeperBallClaim, KeeperDelivery, KeeperFeetDecision,
    KeeperRead, ReadLevel,
};
use crate::r#match::goalkeepers::states::state::GoalkeeperState;
use crate::r#match::player::events::PlayerEvent;
use crate::r#match::{
    ConditionContext, StateChangeResult, StateProcessingContext, StateProcessingHandler,
};
use nalgebra::Vector3;

const PICKUP_DISTANCE_THRESHOLD: f32 = 1.0; // Maximum distance to actually gather the ball
/// The state is entered from up to ~10u away (a keeper walking onto a
/// ball rolling through their box), so it needs an approach phase. Beyond
/// this the ball is someone else's problem.
const PICKUP_APPROACH_RANGE: f32 = 14.0;
/// Ticks spent closing on the ball before giving up. A keeper covering
/// 10u at walking-to-jogging pace needs well under this.
const PICKUP_APPROACH_TIMEOUT: u64 = 60;

#[derive(Default, Clone)]
pub struct GoalkeeperPickingUpState {}

impl StateProcessingHandler for GoalkeeperPickingUpState {
    fn process(&self, ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        // 0. CRITICAL: Goalkeeper can only pick up balls that are NOT flying away from them
        // If the ball is flying away, they cannot pick it up (e.g., their own pass/kick)
        // Check if ball has significant velocity (not just rolling)
        let ball_velocity = ctx.tick_context.positions.ball.velocity;
        let ball_speed = ball_velocity.norm();

        // 5.0 u/tick is above `MAX_SHOT_VELOCITY` (3.2) — this abort never
        // fired, so a keeper kept trying to scoop up a ball leaving him at
        // speed. 1.5 is a ball genuinely travelling away.
        if ball_speed > 1.5 && !ctx.ball().is_towards_player_with_angle(0.3) {
            // Ball is flying away from goalkeeper at high speed - cannot pick up
            return Some(StateChangeResult::with_goalkeeper_state(
                GoalkeeperState::Standing,
            ));
        }

        // 1. Someone beat us to it — or he has just played it himself,
        // which is the same statement about whose ball it is. See
        // [`KeeperDelivery`]: bending to scoop up his own throw is the
        // second-touch offence as well as the reported behaviour.
        if (ctx.ball().is_owned() && !ctx.player.has_ball(ctx)) || KeeperDelivery::is_his(ctx) {
            return Some(StateChangeResult::with_goalkeeper_state(
                GoalkeeperState::Standing,
            ));
        }
        // …and a shot at his goal is the save model's, live or settled.
        if KeeperRead::shot_at_his_goal(ctx) {
            return KeeperRead::from(ctx, ReadLevel::Possession).go();
        }

        // 2. Are hands legal on THIS ball? Covers the area (it may have
        // rolled out of the box while we were closing on it), the
        // back-pass, and the second-touch bar. Previously only the area
        // was checked, so a keeper scooped up a team-mate's pass without
        // anything noticing.
        //
        // Illegal means "play it with your feet", not "leave it": break off
        // to Clearing so the ball is dealt with rather than abandoned in
        // the six-yard box.
        match ctx.ball().handling_verdict() {
            HandlingVerdict::Legal => {}
            HandlingVerdict::OutsideArea => {
                return Some(StateChangeResult::with_goalkeeper_state(
                    GoalkeeperState::Standing,
                ));
            }
            HandlingVerdict::BackPass | HandlingVerdict::SecondTouch => {
                return Some(StateChangeResult::with_goalkeeper_state(
                    GoalkeeperState::Clearing,
                ));
            }
            HandlingVerdict::Restart => {
                return Some(StateChangeResult::with_goalkeeper_state(
                    KeeperFeetDecision::without_hands(ctx),
                ));
            }
        }

        // 3. Approach phase. The state is entered from up to ~10u out, so
        // walking onto the ball is part of the job; only abandon when it
        // is genuinely out of range or the approach has dragged on.
        let ball_distance = ctx.ball().distance();
        if ball_distance > PICKUP_APPROACH_RANGE || ctx.in_state_time > PICKUP_APPROACH_TIMEOUT {
            return Some(StateChangeResult::with_goalkeeper_state(
                GoalkeeperState::Standing,
            ));
        }
        if ball_distance > PICKUP_DISTANCE_THRESHOLD {
            // Still closing — `velocity()` walks us onto it.
            return None;
        }

        // 4. Attempt to pick up the ball. See [`KeeperBallClaim::gather_odds`].
        let pickup_success = ctx.context.rng.unit_f32() < KeeperBallClaim::gather_odds(ctx);
        if pickup_success {
            // Pickup is successful
            let mut state_change =
                StateChangeResult::with_goalkeeper_state(GoalkeeperState::HoldingBall);

            // Generate a pickup event
            state_change.events.add_player_event({
                #[cfg(feature = "match-logs")]
                crate::r#match::engine::ball::ball::ownership::reception_diag::GATHER_SOURCE[1]
                    .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                PlayerEvent::CaughtBall(ctx.player.id)
            });

            Some(state_change)
        } else if ctx.player.has_ball(ctx) {
            // He fumbled a ball that is still his. Diving at it would be
            // absurd — he is standing over it. Play it with his feet
            // instead, which is what a keeper who cannot get his hands to
            // it cleanly actually does.
            Some(StateChangeResult::with_goalkeeper_state(
                KeeperFeetDecision::without_hands(ctx),
            ))
        } else {
            // **A fumble is not a dive.**
            //
            // This branch sent him to `Diving` — a committed action of
            // 0.8 to 1.8 s — at a ball he was standing over, one unit
            // away and barely moving. A keeper who cannot get his hands
            // cleanly round a ball at his feet does not leave them; he
            // reaches for it again, and if a man is on him he goes
            // through it with his body. Both are the read's: a loose ball
            // still at his feet brings him straight back here, and a man
            // on it is the smother.
            //
            // The branch above already handles the same failure on a ball
            // he owns, and handles it exactly this way — on his feet. The
            // two halves of one fumble disagreeing about whether he ends
            // up on the floor is the whole of the defect.
            KeeperRead::from(ctx, ReadLevel::Possession).go()
        }
    }

    fn velocity(&self, ctx: &StateProcessingContext) -> Option<Vector3<f32>> {
        // Move towards the ball to pick it up. Guarded normalize — the
        // state is entered with the ball ~1u away, so "exactly on the
        // ball" is reachable and normalize() of zero would NaN the tick.
        let ball_position = ctx.tick_context.positions.ball.position;
        match (ball_position - ctx.player.position).try_normalize(1e-4) {
            Some(direction) => Some(direction * ctx.player.skills.physical.pace),
            None => Some(Vector3::zeros()),
        }
    }

    fn process_conditions(&self, ctx: ConditionContext) {
        // Picking up requires moderate intensity with focused effort, includes movement
        GoalkeeperCondition::with_velocity(ActivityIntensity::Moderate).process(ctx);
    }
}
