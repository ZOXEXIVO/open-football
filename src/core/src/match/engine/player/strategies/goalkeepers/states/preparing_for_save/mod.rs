use crate::r#match::goalkeepers::states::common::{
    ActivityIntensity, GoalkeeperCondition, KeeperDebug, KeeperPenaltyStance, KeeperRead,
    KeeperRestPosition, KeeperSetPieceStance, KeeperSetPosition, KeeperShotDive,
    KeeperShotReaction, KeeperSweepLimit, ReadLevel, Reading,
};
use crate::r#match::goalkeepers::states::state::GoalkeeperState;
use crate::r#match::player::strategies::players::ops::goalkeeper_skill::GoalkeeperSkillProfile;
use crate::r#match::{
    ConditionContext, StateChangeResult, StateProcessingContext, StateProcessingHandler,
    SteeringBehavior,
};
use nalgebra::Vector3;

const DIVE_DISTANCE: f32 = 40.0; // Distance to attempt diving save

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
    /// him again. The read's threat door re-enters this state with an
    /// opponent carrying inside 12.5 m (100u); 140u = 17.5 m clears that
    /// with room, so the two gates cannot both be live.
    const STAND_DOWN_DISTANCE: f32 = 140.0;
}

impl StateProcessingHandler for GoalkeeperPreparingForSaveState {
    fn process(&self, ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        // The ball his, a man in his spread or through on goal, a cross he
        // can claim, a loose ball he is favourite for. A shot at his goal is
        // his own business: the flight below.
        let shot = match KeeperRead::above(ctx, ReadLevel::Threat) {
            Reading::Go(change) => return Some(change),
            Reading::Mine => true,
            Reading::Clear => false,
        };

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

        // Shot in flight: enter Catching immediately — but only inside the
        // space he is prepared to defend (see [`KeeperSweepLimit`]). We
        // need to be moving toward the intercept line every tick, not
        // waiting for the ball to come within 35u first (by which point
        // it's already past the keeper).
        if shot && KeeperSweepLimit::is_within(ctx, &GoalkeeperSkillProfile::from_ctx(ctx)) {
            return Some(StateChangeResult::with_goalkeeper_state(
                GoalkeeperState::Catching,
            ));
        }

        // The danger has passed — stand down, back to the read below him.
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
        // ball far enough away that he would have time to reset, or the
        // ball in the other half. The release radius is deliberately wider
        // than the entry radius — `COMMIT < DISENGAGE`, the invariant this
        // engine keeps breaking.
        let settled = ctx.team().is_control_ball()
            && (KeeperDebug::calm_off()
                || (ctx.in_state_time > Self::MIN_SET_TICKS
                    && ctx.ball().distance() > Self::STAND_DOWN_DISTANCE));
        if settled || !ctx.ball().on_own_side() {
            return KeeperRead::from(ctx, ReadLevel::Position).go();
        }

        // Continue preparing - position for the save
        None
    }

    fn velocity(&self, ctx: &StateProcessingContext) -> Option<Vector3<f32>> {
        let ball_position = ctx.tick_context.positions.ball.position;
        let ball_velocity = ctx.tick_context.positions.ball.velocity;
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
                // — see [`KeeperShotReaction::crossing_y`] — brought into
                // HIS OWN PLANE, which is where he is adjudicated. The same
                // projection the dive aims down, so the two cannot disagree
                // about where the ball is going to be. See
                // [`KeeperSetPosition::set_point`].
                KeeperShotDive::crossing_at(
                    ctx.player.position.x,
                    target.struck_from,
                    goal_pos.x,
                    KeeperShotReaction::crossing_y(ctx, &prof, goal_pos, target),
                )
                .y,
                (ball_position - goal_pos).magnitude(),
                ctx.context.field_size.width as f32,
                prof.positioning,
                // He does not back-pedal at a struck ball — see
                // [`KeeperSetPosition::set_point`].
                (ctx.player.position.x - goal_pos.x).abs(),
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
        // the line at a penalty, a metre off it at a corner — and the
        // angle-narrowing point below does not apply to a ball that is not
        // moving. See [`KeeperSetPieceStance`].
        if let Some(to_mark) = KeeperSetPieceStance::steer(ctx) {
            return Some(to_mark);
        }

        // **No shot in flight: he sets on the point the flight would steer
        // him to** ([`KeeperSetPosition::guard_point`]), guarding the angle
        // at the ball where his read says it is going, so the set before a
        // strike and the set during one cannot disagree.
        let ball_distance = ctx.ball().distance();
        let goal_pos = ctx.ball().direction_to_own_goal();
        let field_width = ctx.context.field_size.width as f32;
        let set = KeeperSetPosition::guard_point(
            goal_pos,
            KeeperRestPosition::anticipated(ball_position, ball_velocity, prof.positioning),
            field_width,
            prof.positioning,
        );
        if KeeperDebug::calm_off() {
            return Some(
                SteeringBehavior::Pursuit {
                    target: set,
                    target_velocity: ball_velocity * 0.3,
                }
                .calculate(ctx.player)
                .velocity
                    * speed_boost,
            );
        }

        // **Set is a place he STANDS, not a point he follows.** With no
        // deadzone a keeper "preparing for a save" was permanently in
        // transit: measured over a recorded match it was the largest
        // consumer of his mileage, 1587 m at a jog, moving in 42% of the
        // ticks he spent here. Same anisotropic tolerance the resting model
        // uses — the ball is close, so it stays at its tight end.
        if KeeperRestPosition::is_set_with(
            ctx.player.position,
            set,
            prof.concentration,
            ball_distance,
            field_width,
        ) {
            return Some(Vector3::zeros());
        }
        let step = SteeringBehavior::Arrive {
            target: set,
            slowing_distance: 4.0,
        }
        .calculate(ctx.player)
        .velocity;
        if KeeperDebug::set_pace_off() {
            return Some(step * speed_boost);
        }
        // …and he gets there at a set keeper's shuffle. A keeper who has to
        // sprint to be set is in the wrong place.
        let shuffle = KeeperShotReaction::set_shuffle(ctx.player.skills.physical.agility);
        let pace = step.norm();
        Some(if pace > shuffle {
            step * (shuffle / pace)
        } else {
            step
        })
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
}
