//! Catching: the keeper gathering a ball that has reached him.
//!
//! A shot in flight is not his to roll for: the save model settles it once,
//! on the ball (`Ball::try_save_shot`), and a ball it puts in his hands is
//! gathered here before he does anything with it. Nor is a high ball he
//! came for: the aerial contest decides that claim as it reaches him.

use crate::r#match::goalkeepers::states::GoalkeeperPunchingState;
use crate::r#match::goalkeepers::states::common::{
    ActivityIntensity, GoalkeeperCondition, KeeperBallClaim, KeeperDelivery, KeeperFeetDecision,
    KeeperPenaltyStance, KeeperRead, KeeperSetPosition, KeeperShotDive, KeeperShotReaction,
    KeeperSmother, ReadLevel,
};
use crate::r#match::goalkeepers::states::state::GoalkeeperState;
#[cfg(feature = "match-logs")]
use crate::r#match::player::diagnostics::keeper_census::{ClaimSite, KeeperCensus};
use crate::r#match::player::events::PlayerEvent;
use crate::r#match::player::strategies::players::ops::goalkeeper_skill::GoalkeeperSkillProfile;
use crate::r#match::{
    ClaimOutcome, ConditionContext, StateChangeResult, StateProcessingContext,
    StateProcessingHandler, SteeringBehavior,
};
use nalgebra::Vector3;

#[derive(Default, Clone)]
pub struct GoalkeeperCatchingState {}

impl GoalkeeperCatchingState {
    /// The gather: from the ball going into his gloves to his having it
    /// held into his body, in engine ticks (0.3 s).
    const GATHER_TICKS: u64 = 30;
}

impl StateProcessingHandler for GoalkeeperCatchingState {
    fn process(&self, ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        // In his gloves already — a save held on his feet: he takes it into
        // his body, then holds it.
        if ctx.player.has_ball(ctx) && ctx.tick_context.ball.held_in_hands {
            let held = ctx
                .context
                .current_tick()
                .saturating_sub(ctx.tick_context.ball.hands_since_tick);
            return (held >= Self::GATHER_TICKS)
                .then(|| StateChangeResult::with_goalkeeper_state(GoalkeeperState::HoldingBall));
        }

        // **He has just played this ball: it is not his to catch.**
        //
        // The give-up half of [`KeeperDelivery`]. Closing the doors into
        // this state is not enough on its own — he can be standing in it
        // when he releases (a smother he gathered, a parry he then
        // cleared). Above everything, because there is nothing here to
        // decide. A live shot is never his own delivery, so the save path
        // cannot reach this.
        if KeeperDelivery::is_his(ctx) && ctx.tick_context.ball.cached_shot_target.is_none() {
            return KeeperRead::from(ctx, ReadLevel::Possession).go();
        }

        // A claim is the aerial contest's: it takes a caught ball into his
        // gloves (above), and his part of the rest is what it decided.
        match ctx.player.pending_claim {
            Some(ClaimOutcome::Punched) => return Some(GoalkeeperPunchingState::strike(ctx)),
            Some(ClaimOutcome::Flapped) => {
                return KeeperRead::from(ctx, ReadLevel::Possession).go();
            }
            _ => {}
        }

        if ctx.player.claim_intent.is_none() && self.is_catch_successful(ctx) {
            // Are hands legal on THIS ball? Asked at the moment the gloves
            // close, NOT while he is moving to it. The Laws bite on the act
            // of handling, and a keeper crossing his own box to reach a
            // shot spends most of that journey with the ball still outside
            // the area — judging it per-tick made him abandon virtually
            // every save from range (goals went 2.3 → 4.0 a match).
            //
            // Illegal means he plays it with his feet, which is what a
            // keeper receiving a back-pass actually does, and a goal kick
            // he has just been handed is kicked. `KeeperFeetDecision` owns
            // which: the same answer every other door gets.
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
                // takes. A ball already his needs no taking.
                let mut result = StateChangeResult::with_goalkeeper_state(
                    KeeperFeetDecision::without_hands(ctx),
                );
                if !ctx.player.has_ball(ctx) {
                    result
                        .events
                        .add_player_event(PlayerEvent::GainBall(ctx.player.id));
                }
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

        // A penalty is a guess made at the strike, not a read. See
        // [`KeeperPenaltyStance`] — `PreparingForSave` asks the same
        // question, so it is answered in whichever of the two he is in
        // when it is struck.
        if let Some(guess) = KeeperPenaltyStance::commit(ctx) {
            return Some(guess);
        }

        // He cannot get to this one on his feet. LEAVE THEM — now, while
        // the ball is still in the air, so the dive travels to the corner
        // instead of being drawn after the ball has already stopped. See
        // [`KeeperShotDive`].
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

        // **Not a shot: he catches what arrives at him and goes after
        // nothing.** A ball he has to go and get is a race (`ComingOut`) or
        // the chase election (`TakeBall`). This state used to chase it
        // itself, at 1.6-2.6x with a 0.6 s timeout and a 5.6 m release to
        // stop it, which made it a third chase with its own bounds; once the
        // ball is past his reach and not coming to him, or somebody has it,
        // it is the read's again.
        let reach = GoalkeeperSkillProfile::from_ctx(ctx).effective_catch_distance;
        if ctx.ball().is_owned()
            || (ctx.ball().distance() > reach && !ctx.ball().is_towards_player_with_angle(0.6))
        {
            return KeeperRead::from(ctx, ReadLevel::Possession).go();
        }

        None
    }

    fn velocity(&self, ctx: &StateProcessingContext) -> Option<Vector3<f32>> {
        let prof = GoalkeeperSkillProfile::from_ctx(ctx);
        // Sprint reaction speed: 1.6..2.6x band, gated by explosive_mult.
        let speed_boost =
            (1.6 + prof.shot_stopping * 0.5 + prof.dive_reach * 0.5) * prof.explosive_mult;

        // Shot in flight → commit to the intercept line, don't chase
        // the current ball position (it's moving at 5.6 u/tick and
        // outrunning the keeper's pursuit steering).
        if let Some(target) = &ctx.tick_context.ball.cached_shot_target {
            let goal_pos = ctx.ball().direction_to_own_goal();
            // Off the line, not on it — see `KeeperSetPosition`. This is
            // the site that decides where a CAUGHT ball ends up, because
            // the physics save snaps the ball onto the keeper.
            let intercept = KeeperSetPosition::set_point(
                goal_pos,
                // Where he THINKS it is going, not where it is going. See
                // [`KeeperShotReaction::crossing_y`] — steering at the true
                // crossing point from the tick of the strike is a tracking
                // servo, and a servo never has to dive. Brought into HIS
                // OWN PLANE, which is where the catch is adjudicated; see
                // [`KeeperSetPosition::set_point`].
                KeeperShotDive::crossing_at(
                    ctx.player.position.x,
                    target.struck_from,
                    goal_pos.x,
                    KeeperShotReaction::crossing_y(ctx, &prof, goal_pos, target),
                )
                .y,
                (ctx.tick_context.positions.ball.position - goal_pos).magnitude(),
                ctx.context.field_size.width as f32,
                prof.positioning,
                // He does not back-pedal at a struck ball — see
                // [`KeeperSetPosition::set_point`].
                (ctx.player.position.x - goal_pos.x).abs(),
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

        // Not a shot: he takes it where he stands.
        Some(Vector3::zeros())
    }

    fn process_conditions(&self, ctx: ConditionContext) {
        // Catching is a moderate intensity activity requiring focused effort
        GoalkeeperCondition::new(ActivityIntensity::Moderate).process(ctx);
    }
}

impl GoalkeeperCatchingState {
    fn is_catch_successful(&self, ctx: &StateProcessingContext) -> bool {
        let prof = GoalkeeperSkillProfile::from_ctx(ctx);

        // A shot, live or settled, is the save model's: one adjudication,
        // on the ball, which also cannot hand him later in its flight a
        // shot it has already let past him.
        if ctx.tick_context.ball.cached_shot_target.is_some() {
            return false;
        }

        // Past a shot in flight, the gloves only close on a ball that is
        // genuinely FREE and genuinely his.
        //
        // This roll runs every tick the keeper spends in `Catching` and
        // asked nothing at all about who the ball belonged to — only how
        // far away it was, how fast, and whether it was coming towards
        // him. A forward carrying the ball into the area satisfies all
        // three, so the keeper rolled `catch_prob` against him on every
        // tick until the dice came up and then took it off his foot.
        // That is the ball ping-ponging between the keeper and the
        // players in front of him, and it is why catches (44 a match)
        // outnumbered shots (24) — most of them were not saves of
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

        // The same gather as any other ball he gets his hands to.
        #[cfg(feature = "match-logs")]
        KeeperCensus::claim_roll_in(ctx, ClaimSite::Catching);
        KeeperBallClaim::gathers_this_tick(ctx)
    }
}
