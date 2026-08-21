//! Catching: the keeper gathering a ball that has reached him.
//!
//! ⚠ **THE SAVE ROLL FOR A SHOT IN FLIGHT NO LONGER LIVES HERE.** It moved
//! verbatim to `KeeperShotSave`, along with the whole derivation of
//! `EXPECTED_SAVE_TICKS` — a number that has been "corrected" four times and
//! whose history is worth more than the constant. It had to move because the
//! keeper can now spend part of a flight in `Diving` (see `KeeperShotDive`),
//! and two copies of one model would make the realised save rate depend on
//! when he left his feet.

use crate::r#match::events::Event;
use crate::r#match::goalkeepers::states::common::{
    ActivityIntensity, GoalkeeperCondition, KeeperBallClaim, KeeperSetPosition, KeeperShotDive,
    KeeperShotReaction, KeeperShotSave, KeeperSmother, KeeperSweepLimit,
};
use crate::r#match::goalkeepers::states::state::GoalkeeperState;
use crate::r#match::player::events::PlayerEvent;
use crate::r#match::player::strategies::players::ops::goalkeeper_skill::GoalkeeperSkillProfile;
use crate::r#match::{
    ConditionContext, StateChangeResult, StateProcessingContext, StateProcessingHandler,
    SteeringBehavior,
};
use nalgebra::Vector3;

#[derive(Default, Clone)]
pub struct GoalkeeperCatchingState {}

impl StateProcessingHandler for GoalkeeperCatchingState {
    fn process(&self, ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        if self.is_catch_successful(ctx) {
            // Are hands legal on THIS ball? Asked at the moment the gloves
            // close, NOT while he is moving to it. The Laws bite on the act
            // of handling, and a keeper crossing his own box to reach a
            // shot spends most of that journey with the ball still outside
            // the area â judging it per-tick made him abandon virtually
            // every save from range (goals went 2.3 â 4.0 a match).
            //
            // Illegal means he plays it with his feet, which is what a
            // keeper receiving a back-pass actually does. `Clearing` is the
            // honest default: he is on the ball, usually with a forward
            // bearing down.
            if !ctx.ball().handling_verdict().is_legal() {
                // ...and he has to actually GET it, with his feet. This
                // branch emitted no event at all, so it handed `Clearing` a
                // ball the keeper did not own -- and `Clearing`'s first
                // line is `if !has_ball { return Standing }`. The clearance
                // therefore never executed once: he entered, bailed on the
                // next tick, re-reached the same ball and entered again.
                // Measured on a recording: 99 `Catching` -> `Clearing` and
                // 100 back, mean dwell 49 and 83 ms, plus 44 more one-tick
                // exits to Standing and ReturningToGoal -- about 140 dead
                // entries a match and not one ball cleared. `GainBall` is
                // foot control, which is what a keeper playing a back-pass
                // takes.
                let mut result =
                    StateChangeResult::with_goalkeeper_state(GoalkeeperState::Clearing);
                result
                    .events
                    .add_player_event(PlayerEvent::GainBall(ctx.player.id));
                return Some(result);
            }

            let mut holding_result =
                StateChangeResult::with_goalkeeper_state(GoalkeeperState::HoldingBall);

            #[cfg(feature = "match-logs")]
            if ctx.tick_context.positions.ball.position.z > 1.35 {
                crate::mid_run_diag::KeeperActionDiag::note(5);
            }

            holding_result.events.add_player_event({
                #[cfg(feature = "match-logs")]
                crate::r#match::engine::ball::ball::ownership::reception_diag::GATHER_SOURCE[0]
                    .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                PlayerEvent::CaughtBall(ctx.player.id)
            });

            return Some(holding_result);
        }

        // A man at his feet with the ball: take it off him. Reachable
        // from here because `PreparingForSave` sends the keeper straight
        // into `Catching` for any live shot, and a rebound off the first
        // save falls to a striker inside the six-yard box more often than
        // anything else in football. See [`KeeperSmother`].
        if let Some(attempt) = KeeperSmother::assess(ctx) {
            return Some(KeeperSmother::commit(ctx, &attempt));
        }

        // He cannot get to this one on his feet. LEAVE THEM â now, while
        // the ball is still in the air, so the dive travels to the corner
        // instead of being drawn after the ball has already stopped. See
        // [`KeeperShotDive`]; the save itself is unaffected, because
        // `Diving` rolls the identical `KeeperShotSave`.
        if KeeperShotDive::should_launch(ctx) {
            return Some(StateChangeResult::with_goalkeeper_state(
                GoalkeeperState::Diving,
            ));
        }

        // Shot is live: stay in Catching and keep sprinting toward the
        // intercept line. The old logic exited to Standing / ComingOut
        // the moment the ball was >12u away, which meant a keeper
        // aiming for the far post gave up the instant the shot was
        // fired. With a cached shot target the keeper commits.
        if ctx.tick_context.ball.cached_shot_target.is_some() {
            return None;
        }

        // Ball is moving away from the keeper at speed â only credit
        // a parry when the ball was actually within reach (the keeper
        // got a hand to it). Otherwise the shot just missed past the
        // keeper and "parry" would falsely credit a save for a wide
        // shot the GK never touched.
        let ball_speed = ctx.tick_context.positions.ball.velocity.norm();
        let ball_distance = ctx.ball().distance();
        if ball_speed > 2.0 && !ctx.ball().is_towards_player_with_angle(0.6) {
            if ctx.tick_context.ball.cached_shot_target.is_some() && ball_distance < 25.0 {
                return Some(StateChangeResult::with_goalkeeper_state_and_event(
                    GoalkeeperState::Standing,
                    Event::PlayerEvent(PlayerEvent::ParriedBall(ctx.player.id)),
                ));
            }
            return Some(StateChangeResult::with_goalkeeper_state(
                GoalkeeperState::Standing,
            ));
        }

        // If ball is too far, decide based on distance from goal.
        // "Far from goal" is how far off his LINE he is, bounded by how far
        // this keeper sweeps â not five metres from his kickoff dot, which
        // is what `distance_from_start_position() > 40.0` meant and which
        // made a keeper who had come to meet a through-ball turn round
        // instead of gathering it. See [`KeeperSweepLimit`].
        //
        // THE GIVE-UP DISTANCE HAS TO BE WIDER THAN EVERY ENTRY INTO THIS
        // STATE. It was 12u -- 1.5 m -- while `ComingOut` claims at 20u and
        // `PreparingForSave` at 35u, so the band between them was a
        // two-cycle by construction: he entered `Catching` at 2.5 m, was
        // thrown straight back out at 1.6 m, re-entered, and so on.
        // Measured on a recording: 173 `Catching` -> `ComingOut` and 141
        // back, mean dwell 87 and 111 ms -- over three hundred state
        // changes a match spent oscillating across a one-metre band, at the
        // sprint pace `ComingOut` steers with. From the stands that is a
        // keeper twitching back and forth over a loose ball in a crowd.
        //
        // 45u = 5.6 m is strictly outside the widest entry, so once he goes
        // for a ball he stays going for it -- the same COMMIT < DISENGAGE
        // ordering `MAX_COMING_OUT_DISTANCE` and `KeeperSweepLimit`
        // document.
        if ctx.ball().distance() > Self::RELEASE_DISTANCE {
            // If already far from goal, return rather than chasing further
            let sweeper = GoalkeeperSkillProfile::from_ctx(ctx);
            if !KeeperSweepLimit::is_within(ctx, &sweeper) {
                return Some(StateChangeResult::with_goalkeeper_state(
                    GoalkeeperState::ReturningToGoal,
                ));
            }
            return Some(StateChangeResult::with_goalkeeper_state(
                GoalkeeperState::ComingOut,
            ));
        }

        // NB the last of the KICKOFF-DOT LEASHES lived here:
        //
        //     if position_to_distance() == Big { return ReturningToGoal }
        //
        // measured from (20, 275) as a radius, and asked with the ball
        // already inside gathering range. It told a keeper standing on a
        // loose ball to turn round and go home because of where he was
        // relative to a dot, and `ReturningToGoal` sent him straight back
        // for the same ball at 1.9 m: 68 `Catching` -> `ReturningToGoal`
        // and 65 back on one recording, mean dwell 57 and 136 ms.
        //
        // How far off his line he may be is `KeeperSweepLimit`'s question
        // and it is already asked, with the ball distance, in the branch
        // above -- which is the correct place for it, because giving up on
        // a ball is about the ball. See [`KeeperSweepLimit`] for the other
        // five sites this same leash was removed from.

        if ctx.in_state_time > 30 {
            return Some(StateChangeResult::with_goalkeeper_state(
                GoalkeeperState::Standing,
            ));
        }

        None
    }

    fn velocity(&self, ctx: &StateProcessingContext) -> Option<Vector3<f32>> {
        let prof = GoalkeeperSkillProfile::from_ctx(ctx);
        // Sprint reaction speed: 1.6..2.6x band, gated by explosive_mult.
        let speed_boost =
            (1.6 + prof.shot_stopping * 0.5 + prof.dive_reach * 0.5) * prof.explosive_mult;

        // Shot in flight â commit to the intercept line, don't chase
        // the current ball position (it's moving at 5.6 u/tick and
        // outrunning the keeper's pursuit steering).
        if let Some(target) = &ctx.tick_context.ball.cached_shot_target {
            let goal_pos = ctx.ball().direction_to_own_goal();
            // Off the line, not on it â see `KeeperSetPosition`. This is
            // the site that decides where a CAUGHT ball ends up, because
            // the physics save snaps the ball onto the keeper.
            let intercept = KeeperSetPosition::set_point(
                goal_pos,
                // Where he THINKS it is going, not where it is going. See
                // [`KeeperShotReaction::crossing_y`] — steering at the true
                // crossing point from the tick of the strike is a tracking
                // servo, and a servo never has to dive.
                KeeperShotReaction::crossing_y(ctx, &prof, goal_pos, target),
                (ctx.tick_context.positions.ball.position - goal_pos).magnitude(),
                ctx.context.field_size.width as f32,
                prof.positioning,
            );
            // ...AT A SET KEEPER'S PACE. `speed_boost` on top of the
            // `Active` band is 8-13 m/s sideways, which is how this keeper
            // came to track every shot in the game to the exact point it
            // crossed the line and gather it standing up: the answer to
            // "can he get there on his feet?" was always yes, so he never
            // had to dive. A man still running when the ball arrives has
            // not made a save. See [`KeeperShotReaction`].
            return Some(KeeperShotReaction::on_foot(
                ctx,
                &prof,
                SteeringBehavior::Arrive {
                    target: intercept,
                    slowing_distance: 2.0,
                }
                .calculate(ctx.player)
                .velocity
                    * speed_boost,
            ));
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
    /// How far the ball has to get before he gives up on gathering it.
    /// Must stay strictly WIDER than every distance at which another state
    /// hands him the ball to gather -- `ComingOut` at 20u,
    /// `PreparingForSave` at 35u -- or the overlap is a two-cycle. 45u =
    /// 5.6 m.
    const RELEASE_DISTANCE: f32 = 45.0;

    fn is_catch_successful(&self, ctx: &StateProcessingContext) -> bool {
        let prof = GoalkeeperSkillProfile::from_ctx(ctx);

        // Shot-in-flight: judge the save from the *intercept line*, not
        // from current ball distance. A ball aimed into the corner
        // passes the GK 8-15 units wide of their current position â
        // real keepers reach 3-4 m (6-8 u) diving, so the relevant
        // metric is "how far off the line am I?", not "am I touching
        // the ball right now?".
        if ctx.tick_context.ball.cached_shot_target.is_some() {
            return KeeperShotSave::roll(ctx);
        }

        // Past a shot in flight, the gloves only close on a ball that is
        // genuinely FREE and genuinely his.
        //
        // This roll runs every tick the keeper spends in `Catching` and
        // asked nothing at all about who the ball belonged to â only how
        // far away it was, how fast, and whether it was coming towards
        // him. A forward carrying the ball into the area satisfies all
        // three, so the keeper rolled `catch_prob` against him on every
        // tick until the dice came up and then took it off his foot.
        // That is the ball ping-ponging between the keeper and the
        // players in front of him, and it is why catches (44 a match)
        // outnumbered shots (24) â most of them were not saves of
        // anything.
        //
        // The keeper still comes for what is his: a loose ball he is
        // favourite for. He does not tackle with his hands.
        if ctx.ball().is_owned() && !ctx.player.has_ball(ctx) {
            return false;
        }
        // ...and it must not be the ball HE just put into play.
        //
        // `Standing` has carried this guard for a while, with a comment
        // explaining that a keeper whose own delivery stops at his feet
        // "used to collect it and distribute again on the spot, over and
        // over". `Catching` is the second door into the identical cycle
        // and never had it. Probed on a recording: of 120 consecutive
        // catches the keeper was not allowed to handle, 115 were OUTSIDE
        // his penalty area, none was a shot, and the ball was a mean 11 cm
        // from him travelling 0.02 u/tick -- a dead ball at his feet. He
        // gathered it with his feet, hoofed it, watched it stop, and
        // gathered it again: 144 `Catching` -> `Clearing` and 131 back per
        // match, and 257 clearances in a match that should see a dozen.
        if ctx.ball().blocked_from_recollecting() {
            return false;
        }
        if !KeeperBallClaim::is_favourite(ctx) {
            return false;
        }

        let distance_to_ball = ctx.ball().distance();
        let max_catch_distance = prof.effective_catch_distance;
        if distance_to_ball > max_catch_distance {
            return false;
        }

        let ball_speed = ctx.tick_context.positions.ball.velocity.norm();
        if ball_speed > 0.5 && !ctx.ball().is_towards_player_with_angle(0.6) {
            return false;
        }

        // NB this branch is a LOOSE ball, not a shot, so it keeps its own
        // (gentler) power scale â `SaveModel::strike_power` is centred on
        // a struck shot and would read every trickling ball as maximally
        // easy.
        let ball_height = ctx.tick_context.positions.ball.position.z;
        let stretch = (distance_to_ball / max_catch_distance).clamp(0.0, 1.0);
        let power = ((ball_speed - 1.5) / 6.0).clamp(0.0, 1.0);

        // Awkward-height penalty: ground or above-head balls are harder.
        let height_pen = if (0.5..=1.8).contains(&ball_height) {
            0.0
        } else if ball_height < 0.2 {
            0.18
        } else if ball_height > 2.5 {
            0.22
        } else {
            0.06
        };

        let direction_factor = if ctx.ball().is_towards_player_with_angle(0.7) {
            0.0
        } else {
            0.18
        };

        let catch_difficulty = (power * 0.28
            + stretch * 0.22
            + height_pen * 0.18
            + direction_factor * 0.12
            + (1.0 - prof.condition_mult) * 0.10
            + prof.poor_skill_penalty * 0.10)
            .clamp(0.0, 1.0);

        let catch_prob = prof.catch_probability(catch_difficulty);
        ctx.context.rng.unit_f32() < catch_prob
    }
}
