use crate::r#match::goalkeepers::states::common::{
    ActivityIntensity, GoalkeeperCondition, KeeperAerialClaim, KeeperBallClaim, KeeperDebug,
    KeeperOneOnOne, KeeperPenaltyStance, KeeperRestPosition, KeeperSetPieceStance,
    KeeperSetPosition, KeeperShotDive, KeeperShotReaction, KeeperSmother, KeeperSweepLimit,
};
use crate::r#match::goalkeepers::states::state::GoalkeeperState;
use crate::r#match::player::strategies::players::ops::goalkeeper_skill::GoalkeeperSkillProfile;
use crate::r#match::{
    ConditionContext, StateChangeResult, StateProcessingContext, StateProcessingHandler,
    SteeringBehavior,
};
use nalgebra::Vector3;

const DIVE_DISTANCE: f32 = 40.0; // Distance to attempt diving save
const CATCH_DISTANCE: f32 = 35.0; // Distance to attempt catching
const PUNCH_DISTANCE: f32 = 18.0; // Distance to attempt punching

#[derive(Default, Clone)]
pub struct GoalkeeperPreparingForSaveState {}

impl GoalkeeperPreparingForSaveState {
    /// He does not un-set himself the instant a team-mate gets a toe on
    /// it. 25 AI ticks = half a second — long enough that a cleared ball
    /// and a scramble look different, short enough to be invisible when
    /// the danger really has gone. (AI ticks are 20 ms: the engine runs
    /// at 10 ms and the AI every second tick.)
    const MIN_SET_TICKS: u64 = 25;

    /// …and the ball has to be further away than anything that would set
    /// him again. `Standing` re-enters this state with an opponent
    /// carrying inside 12.5 m (100u); 140u = 17.5 m clears that with room,
    /// so the two gates cannot both be live.
    const STAND_DOWN_DISTANCE: f32 = 140.0;
}

impl StateProcessingHandler for GoalkeeperPreparingForSaveState {
    fn process(&self, ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        // If goalkeeper has the ball, transition to passing
        if ctx.player.has_ball(ctx) {
            return Some(StateChangeResult::with_goalkeeper_state(
                GoalkeeperState::Passing,
            ));
        }

        // A man has arrived on top of him with the ball at his feet. Set
        // is no longer a position — go and take it. Above everything else
        // here because it is the most immediate thing on the pitch, and
        // because the alternative is what the engine used to do: stand
        // still and wait to be dribbled round. See [`KeeperSmother`].
        if let Some(attempt) = KeeperSmother::assess(ctx) {
            return Some(KeeperSmother::commit(ctx, &attempt));
        }

        // A penalty: he goes at the strike, to the side he has guessed,
        // and there is nothing to read. Above the ordinary launch because
        // that one waits out a reaction the penalty does not have time
        // for. See [`KeeperPenaltyStance`].
        if let Some(guess) = KeeperPenaltyStance::commit(ctx) {
            return Some(guess);
        }

        // A shot he cannot get to on his feet — leave them, now, so the
        // dive and the ball arrive together. `should_dive` below is a
        // proximity test (`DIVE_DISTANCE` is 40u, about a seventh of a
        // second of flight) and cannot see a shot into the corner coming;
        // this reads the projected crossing point and the time left. See
        // [`KeeperShotDive`].
        if KeeperShotDive::should_launch(ctx) {
            return Some(StateChangeResult::with_goalkeeper_state(
                GoalkeeperState::Diving,
            ));
        }

        // Check if we need to dive
        if self.should_dive(ctx) {
            return Some(StateChangeResult::with_goalkeeper_state(
                GoalkeeperState::Diving,
            ));
        }

        // A cross or a chip hanging over the box is his to take — see
        // [`KeeperAerialClaim`]. Below the dive so a shot he can still get
        // a hand to always wins, above everything else because a keeper
        // set for a shot that never comes should be attacking the
        // delivery instead of watching it drop onto a forehead.
        if let Some(claim) = KeeperAerialClaim::assess(ctx) {
            KeeperAerialClaim::note_start(ctx, &claim);
            return Some(StateChangeResult::with_goalkeeper_state(
                if claim.at_contact(ctx.player.position) {
                    if claim.standing {
                        GoalkeeperState::Catching
                    } else {
                        GoalkeeperState::Jumping
                    }
                } else {
                    GoalkeeperState::ComingOut
                },
            ));
        }

        let ball_distance = ctx.ball().distance();
        let ball_velocity = ctx.tick_context.positions.ball.velocity;
        let ball_speed = ball_velocity.norm();

        // Check if we should attempt a save — but only inside the space
        // he is prepared to defend. That bound used to be
        // `distance_from_start_position() < 50.0`: six metres from his
        // kickoff dot, measured as a radius, so a keeper who had swept out
        // to the edge of his area could not enter `Catching` for a shot at
        // all. See [`KeeperSweepLimit`].
        let within_his_space =
            KeeperSweepLimit::is_within(ctx, &GoalkeeperSkillProfile::from_ctx(ctx));

        // Shot in flight: enter Catching immediately — we need to be
        // moving toward the intercept line every tick, not waiting for
        // the ball to come within 35u first (by which point it's
        // already past the keeper).
        if ctx.tick_context.ball.cached_shot_target.is_some() && within_his_space {
            return Some(StateChangeResult::with_goalkeeper_state(
                GoalkeeperState::Catching,
            ));
        }

        // Claim a LOOSE ball near the goal.
        //
        // This used to fire on ball distance alone: anything inside 35u
        // (4.4 m) sent the keeper into `Catching` whether the ball was
        // loose or at his own defender's feet, whether it was coming
        // towards him or going away, and whether or not he had just put
        // it there himself. `Standing`'s equivalent gate has always
        // carried those conditions; this one carried none of them, at
        // 3.5x the radius — so the keeper hoovered up everything that
        // came within four metres of him.
        //
        // Measured: 154 gathers a match against a real 8-12, and the
        // ball in a keeper's gloves for 11-13% of the match against a
        // real 3-6%. The visible symptom is the ball forever ending up
        // on one spot in front of the goal, because that spot is where
        // the keeper who just collected it is standing.
        //
        // A shot in flight is handled by the branch above and is
        // deliberately untouched — this is about everything that is NOT
        // a shot.
        // …and it has to be HIS ball. See [`KeeperBallClaim`]: "loose" in
        // a busy box means "unowned for a tick", which is not the same as
        // "nobody is on it", and claiming those is what produced the
        // keeper/attacker ping-pong on one spot in front of goal.
        let loose_ball_claimable = !ctx.ball().is_owned()
            && !ctx.ball().blocked_from_recollecting()
            && ctx.ball().on_own_side()
            && KeeperBallClaim::is_favourite(ctx);
        if loose_ball_claimable && ball_distance < CATCH_DISTANCE && within_his_space {
            return Some(StateChangeResult::with_goalkeeper_state(
                GoalkeeperState::Catching,
            ));
        }

        // If ball is on opponent's half, return to goal
        if !ctx.ball().on_own_side() {
            return Some(StateChangeResult::with_goalkeeper_state(
                GoalkeeperState::ReturningToGoal,
            ));
        }

        // The danger has passed — stand down.
        //
        // ⚠ THIS WAS A ONE-TICK POSSESSION FLAG (`team().is_control_ball()`
        // alone), against a `Standing` entry condition that fires on an
        // opponent carrying the ball within 12.5 m or any ball moving
        // goalward inside 37.5 m. Those are not complements: through every
        // scramble, every tackle and every loose touch in his own third
        // BOTH were true in alternation, at tick resolution. Measured over
        // a recorded match: **1174 `Standing` → `PreparingForSave` and 1161
        // back, a mean dwell of 827 ms, and 667 of the returns inside
        // 300 ms** — thirteen changes of mind a minute, each one re-aiming
        // his steering at a different point. That oscillation IS the
        // reported behaviour: a keeper visibly jinking around his area
        // instead of setting himself.
        //
        // Being set is a POSTURE, held while the ball is somewhere it can
        // hurt him, not a reaction to who touched it last. He comes out of
        // it when the danger is actually gone: possession settled AND the
        // ball far enough away that he would have time to reset. The
        // release radius is deliberately wider than the entry radius —
        // `COMMIT < DISENGAGE`, the invariant this engine keeps breaking.
        if ctx.team().is_control_ball()
            && (KeeperDebug::calm_off()
                || (ctx.in_state_time > Self::MIN_SET_TICKS
                    && ctx.ball().distance() > Self::STAND_DOWN_DISTANCE))
        {
            return Some(StateChangeResult::with_goalkeeper_state(
                GoalkeeperState::Standing,
            ));
        }

        // Check if we should punch (for dangerous high balls)
        if self.should_punch(ctx) {
            return Some(StateChangeResult::with_goalkeeper_state(
                GoalkeeperState::Punching,
            ));
        }

        // Check if ball is moving away and we should come out
        let ball_toward_goal = self.is_ball_toward_goal(ctx);
        if !ball_toward_goal && ball_distance < 30.0 && ball_speed < 2.0 {
            // Loose ball not heading to goal - come out to claim
            return Some(StateChangeResult::with_goalkeeper_state(
                GoalkeeperState::ComingOut,
            ));
        }

        // Continue preparing - position for the save
        None
    }

    fn velocity(&self, ctx: &StateProcessingContext) -> Option<Vector3<f32>> {
        let ball_position = ctx.tick_context.positions.ball.position;
        let ball_velocity = ctx.tick_context.positions.ball.velocity;
        let ball_speed = ball_velocity.norm();
        let prof = GoalkeeperSkillProfile::from_ctx(ctx);

        // Sprint speed boost — gated by explosive multiplier.
        let speed_boost =
            (1.6 + prof.shot_stopping * 0.6 + prof.dive_reach * 0.5) * prof.explosive_mult;

        // If a shot has been fired, the projected goal-line crossing is
        // cached on the ball. Commit to that line instead of chasing
        // the ball's current position — a real keeper picks a spot on
        // the line and dives there. Without this the keeper lost ground
        // tick-by-tick to the 5.6 u/tick shot and never saved anything.
        if let Some(target) = &ctx.tick_context.ball.cached_shot_target {
            let goal_pos = ctx.ball().direction_to_own_goal();
            // Guard `target.goal_line_y`, but from a set position OFF the
            // line rather than on it — see `KeeperSetPosition` for why
            // standing on the line put the ball inside the goal frame
            // after every catch. Z ignored: we move on the ground.
            let intercept_point = KeeperSetPosition::set_point(
                goal_pos,
                // His read of it, which lags the truth and converges on it
                // — see [`KeeperShotReaction::crossing_y`].
                KeeperShotReaction::crossing_y(ctx, &prof, goal_pos, target),
                (ball_position - goal_pos).magnitude(),
                ctx.context.field_size.width as f32,
                prof.positioning,
            );
            // …but only as fast as a set keeper moves. Everything past a
            // side-step has to come out of the dive; see
            // [`KeeperShotReaction`].
            return Some(KeeperShotReaction::on_foot(
                ctx,
                &prof,
                SteeringBehavior::Arrive {
                    target: intercept_point,
                    slowing_distance: 3.0,
                }
                .calculate(ctx.player)
                .velocity
                    * speed_boost,
            ));
        }

        // **A dead ball at his goal.** He is set for it on his mark — on
        // the line at a penalty, a metre off it at a corner — and nothing
        // below (the duel, the angle-narrowing point) applies to a ball
        // that is not moving. See [`KeeperSetPieceStance`].
        if let Some(to_mark) = KeeperSetPieceStance::steer(ctx) {
            return Some(to_mark);
        }

        // **A man is running at him with the ball.** Then the point to
        // stand on is measured off the BALL, not off his own line — see
        // [`KeeperOneOnOne`]. The branch below holds a fixed 18-32u of
        // depth whatever the carrier does, which measured as a keeper
        // watching a striker dribble from 8.7 m out to 2.8 m from goal
        // while he held station 5.22 m away: the smother wants the ball
        // inside his own spread and it was never going to see it.
        if KeeperOneOnOne::duel(ctx).is_some() {
            return Some(
                SteeringBehavior::Arrive {
                    target: KeeperOneOnOne::point(ctx, &prof),
                    slowing_distance: 8.0,
                }
                .calculate(ctx.player)
                .velocity
                    * speed_boost,
            );
        }

        // No shot cached — slow ball / through ball / loose ball: fall
        // back to the angle-narrowing behaviour.
        let ball_distance = ctx.ball().distance();
        let goal_pos = ctx.ball().direction_to_own_goal();
        let prediction_time = 0.2 + prof.shot_stopping * 0.4;
        let predicted_ball = ball_position + ball_velocity * prediction_time;
        let goal_to_predicted = predicted_ball - goal_pos;
        let intercept_distance = if ball_speed > 1.2 {
            10.0 + prof.shot_stopping * 8.0 + prof.dive_reach * 3.0
        } else {
            18.0 + prof.shot_stopping * 10.0 + prof.dive_reach * 4.0
        };
        let target = if goal_to_predicted.norm() > 1.0 {
            goal_pos + goal_to_predicted.normalize() * intercept_distance.min(ball_distance * 0.5)
        } else {
            goal_pos
        };

        // **Set is a place he STANDS, not a point he follows.**
        //
        // This branch had no deadzone of any kind: `Pursuit` re-aimed at a
        // lead point every tick and `speed_boost` is above 2, so a keeper
        // "preparing for a save" was permanently in transit. Measured over
        // a recorded match it was the single largest consumer of his
        // mileage — **1587 m at 132 m/min**, a sustained jog — while the
        // state census had him moving in 42% of the ticks he spent here.
        // That is the opposite of what the state is for, and it is the
        // half of the reported behaviour that the two-cycle fixes do not
        // reach: a keeper who is set should be planted, weight forward,
        // adjusting in short steps.
        //
        // Same anisotropic tolerance the resting model uses, and the ball
        // is by definition close here, so `distance_slack` is near 1 and
        // the tolerance stays at its tight end — three quarters of a metre
        // across the goal. `Arrive` rather than `Pursuit` for the same
        // reason: he is going to a spot, not chasing a moving one.
        if !KeeperDebug::calm_off()
            && KeeperRestPosition::is_set_with(
                ctx.player.position,
                target,
                prof.concentration,
                ball_distance,
                ctx.context.field_size.width as f32,
            )
        {
            return Some(Vector3::zeros());
        }
        if KeeperDebug::calm_off() {
            return Some(
                SteeringBehavior::Pursuit {
                    target,
                    target_velocity: ball_velocity * 0.3,
                }
                .calculate(ctx.player)
                .velocity
                    * speed_boost,
            );
        }

        Some(
            SteeringBehavior::Arrive {
                target,
                slowing_distance: 4.0,
            }
            .calculate(ctx.player)
            .velocity
                * speed_boost,
        )
    }

    fn process_conditions(&self, ctx: ConditionContext) {
        // Preparing for save requires high intensity as goalkeeper moves into position
        GoalkeeperCondition::with_velocity(ActivityIntensity::High).process(ctx);
    }
}

impl GoalkeeperPreparingForSaveState {
    /// Determine if goalkeeper should dive for a ball that is NOT a shot.
    ///
    /// ⚠ **A live shot is [`KeeperShotDive`]'s, and only its.** This gate is
    /// a proximity test — `DIVE_DISTANCE` is 40u, five metres, a seventh of
    /// a second of flight — so for a shot it can only ever fire once the
    /// ball is already on top of him. Measured on a recording, that is
    /// exactly what it did: dives beginning with the ball 0.7 to 2.7 m away
    /// at 35 m/s, which the viewer draws as the ball stopping dead at a
    /// standing man who then falls over. Two gates for one decision also
    /// means the worse one wins whenever it is cheaper to satisfy.
    ///
    /// What is left to it is everything that is not a shot and has no
    /// projected crossing point to reason about: a deflection, a rebound
    /// off a defender, a cross that dips.
    fn should_dive(&self, ctx: &StateProcessingContext) -> bool {
        if ctx.tick_context.ball.cached_shot_target.is_some() {
            return false;
        }

        let ball_distance = ctx.ball().distance();
        let ball_velocity = ctx.tick_context.positions.ball.velocity;
        let ball_speed = ball_velocity.norm();

        // Don't dive if ball is too far
        if ball_distance > DIVE_DISTANCE {
            return false;
        }

        // Check if ball is heading toward goal
        let toward_goal = self.is_ball_toward_goal(ctx);
        if !toward_goal {
            return false;
        }

        // Ball must be moving (shots have velocity ~1.0-2.0 per tick)
        if ball_speed < 0.3 {
            return false;
        }

        let prof = GoalkeeperSkillProfile::from_ctx(ctx);
        let time_to_ball = ball_distance / ball_speed.max(0.5);

        // Skill-driven distances. Effective dive distance already
        // bakes in dive_reach + positioning so it can scale with
        // condition; ball-speed branches differ in reaction window.
        let effective = prof.effective_dive_distance;
        if ball_speed > 1.5 {
            ball_distance < effective && time_to_ball < (18.0 + prof.shot_stopping * 22.0)
        } else if ball_speed > 0.8 {
            ball_distance < (effective * 0.85)
        } else {
            ball_distance < (effective * 0.65)
        }
    }

    /// Determine if goalkeeper should punch the ball
    fn should_punch(&self, ctx: &StateProcessingContext) -> bool {
        let ball_distance = ctx.ball().distance();
        let ball_velocity = ctx.tick_context.positions.ball.velocity;
        let ball_speed = ball_velocity.norm();
        let ball_position = ctx.tick_context.positions.ball.position;

        if ball_distance > PUNCH_DISTANCE {
            return false;
        }

        let prof = GoalkeeperSkillProfile::from_ctx(ctx);
        let ball_height = ball_position.z;
        let is_high_ball = ball_height > 2.0;

        let crowd = if ball_distance < 10.0 {
            (ctx.players().opponents().nearby(8.0).count() as f32 / 4.0).clamp(0.0, 1.0)
        } else {
            0.0
        };

        // ⚠ UNITS. Every speed bar in this function was written against a
        // 5.6 u/tick shot cap and a friction model that has since been
        // corrected; the engine now caps a shot at `MAX_SHOT_VELOCITY`
        // 3.2 and a struck pass leaves the foot at 0.5-2.2. So the `> 8.0`
        // and `> 6.0` branches below could never fire and `power_factor`
        // was pinned at ZERO for every ball in the game — the punch
        // decision could not see how hard the ball was hit at all, which
        // is the one thing that decides whether a keeper catches it or
        // pushes it away. Re-anchored on the live scale: `DRIVEN` is a
        // firmly-struck delivery, and power runs 0..1 from there to the
        // hardest strike the engine can produce.
        const DRIVEN: f32 = 1.2;
        let power_factor = ((ball_speed - DRIVEN) / 2.0).clamp(0.0, 1.0);
        // Build a synthetic catch_prob: aerial command + handling
        // discounted by crowd + power.
        let synthetic_catch = (prof.handling_profile * 0.55 + prof.aerial_command * 0.45
            - power_factor * 0.20
            - crowd * 0.20)
            .clamp(0.0, 1.0);

        if is_high_ball && ball_speed > 2.2 {
            return true;
        }
        if crowd >= 0.5 && ball_distance < 10.0 {
            return prof.should_punch(synthetic_catch, crowd, power_factor);
        }
        if prof.handling_profile < 0.5 && ball_speed > 1.8 && is_high_ball {
            return true;
        }
        false
    }

    /// Check if ball is moving toward goal
    fn is_ball_toward_goal(&self, ctx: &StateProcessingContext) -> bool {
        let ball_velocity = ctx.tick_context.positions.ball.velocity;
        let ball_speed = ball_velocity.norm();

        // Stationary ball is not moving toward goal
        if ball_speed < 0.5 {
            return false;
        }

        // Direction from the ball to the own goal mouth.
        // `direction_to_own_goal()` returns the goal POSITION — it must
        // be anchored at the ball before normalizing, otherwise the dot
        // below compares against the field-origin→goal axis and a real
        // shot at the left goal (travelling −x) never reads as "toward
        // goal".
        let ball_position = ctx.tick_context.positions.ball.position;
        let goal_position = ctx.ball().direction_to_own_goal();
        let to_goal = match (goal_position - ball_position).try_normalize(1e-4) {
            Some(dir) => dir,
            // Ball already on the goal line — that is "toward goal".
            None => return true,
        };

        // Check if ball velocity is pointing toward goal
        // Use dot product: > 0 means moving in same general direction
        let toward_goal_dot = ball_velocity.normalize().dot(&to_goal);

        // Consider it "toward goal" if angle is less than 90 degrees (dot > 0)
        // More strict for positioning: require at least 30 degree alignment
        toward_goal_dot > 0.5
    }

    /// Calculate the optimal position for making a save
    #[allow(dead_code)]
    fn calculate_optimal_save_position(&self, ctx: &StateProcessingContext) -> Vector3<f32> {
        let ball_position = ctx.tick_context.positions.ball.position;
        let ball_velocity = ctx.tick_context.positions.ball.velocity;
        let ball_speed = ball_velocity.norm();
        let goal_position = ctx.ball().direction_to_own_goal();
        let prof = GoalkeeperSkillProfile::from_ctx(ctx);

        // If ball is moving, predict where it will be
        let predicted_ball_position = if ball_speed > 1.0 {
            let prediction_time = 0.3 + prof.positioning * 0.3;
            ball_position + ball_velocity * prediction_time
        } else {
            ball_position
        };

        let goal_line_position = goal_position;
        let positioning_ratio = 0.15 + prof.positioning * 0.15;
        let optimal_position =
            goal_line_position + (predicted_ball_position - goal_line_position) * positioning_ratio;
        let max_distance_from_goal = 8.0 + prof.positioning * 4.0;
        let distance_from_goal = (optimal_position - goal_line_position).magnitude();

        if distance_from_goal > max_distance_from_goal {
            // Clamp to max distance
            goal_line_position
                + (optimal_position - goal_line_position).normalize() * max_distance_from_goal
        } else {
            optimal_position
        }
    }
}
