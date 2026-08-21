use crate::club::player::skills::GoalkeeperSpeedContext;
use crate::r#match::goalkeepers::states::common::{
    ActivityIntensity, GoalkeeperCondition, KeeperFeetDecision, KeeperSmother, KeeperSweepLimit,
};
use crate::r#match::goalkeepers::states::state::GoalkeeperState;
use crate::r#match::player::strategies::common::states::LooseBallChase;
use crate::r#match::player::strategies::players::ops::goalkeeper_skill::GoalkeeperSkillProfile;
use crate::r#match::{
    ConditionContext, StateChangeResult, StateProcessingContext, StateProcessingHandler,
    SteeringBehavior,
};
use nalgebra::Vector3;

#[derive(Default, Clone)]
pub struct GoalkeeperTakeBallState {}

impl StateProcessingHandler for GoalkeeperTakeBallState {
    fn process(&self, ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        // He has won the race. What happens to the ball is decided here,
        // not by jogging home with it at his feet — `ReturningToGoal` used
        // to be handed a keeper in possession and pass him on to
        // `Distributing` a tick later, which is two states of dithering
        // over a ball he could simply have picked up. See
        // [`KeeperFeetDecision`].
        if ctx.player.has_ball(ctx) {
            return Some(StateChangeResult::with_goalkeeper_state(
                KeeperFeetDecision::state_for(ctx),
            ));
        }

        // **A shot has been struck at his goal — stop chasing and go and
        // defend it.**
        //
        // Every sibling state that can leave him off his line carries this
        // branch — `Standing`, `Walking`, `ComingOut`, `ReturningToGoal` —
        // and this one did not, so a keeper who had committed to a loose
        // ball kept sprinting at the ball's current position for the whole
        // flight: no set, no read, no dive. Measured on recordings,
        // long-range goals whose entire keeper state track reads
        // `Take Ball`.
        //
        // He got here far more often than the state name suggests, too. The
        // universal loose-ball override in
        // `PlayerFieldPositionGroup::process` forced a keeper into `TakeBall`
        // for any unowned ball inside 60 u, and a struck shot is unowned —
        // so this branch on its own used to be a two-cycle against it, in
        // and straight back out. That override now declines live shots at his
        // own goal, which is what makes the hand-off here stick.
        //
        // `PreparingForSave` rather than `Catching` or `Diving`: he may be a
        // long way from his line, and that state owns the whole question —
        // the smother, `KeeperShotDive`, the aerial claim, and the hand-off
        // to `Catching` once he is inside the space he defends. One owner.
        if let Some(target) = &ctx.tick_context.ball.cached_shot_target {
            if Some(target.defending_side) == ctx.player.side {
                return Some(StateChangeResult::with_goalkeeper_state(
                    GoalkeeperState::PreparingForSave,
                ));
            }
        }

        // He has chased it down and a man has got there first, with the
        // ball inside his own spread: that is a smother, not a lost race.
        // See [`KeeperSmother`] — the gates are all in `assess`.
        if let Some(attempt) = KeeperSmother::assess(ctx) {
            return Some(KeeperSmother::commit(ctx, &attempt));
        }

        // Transition to Catching when ball is very close and not owned
        if ctx.ball().distance() < 3.0 && !ctx.ball().is_owned() {
            return Some(StateChangeResult::with_goalkeeper_state(
                GoalkeeperState::Catching,
            ));
        }

        if ctx.ball().is_owned() {
            return Some(StateChangeResult::with_goalkeeper_state(
                GoalkeeperState::ReturningToGoal,
            ));
        }

        // **THE BALL HAS LEFT THE GROUND HE DEFENDS — it is somebody
        // else's now.**
        //
        // This state had no bound of any kind. It is a bare `Seek` at the
        // ball's live position, entered by the dispatcher's loose-ball
        // override for anything unowned within 60 u of him, and it held
        // him for up to 200 AI ticks — four seconds of following a ball
        // wherever it rolled. Every sibling state that can take him off
        // his line carries an excursion test; this one, the only one that
        // is a pure chase, did not, and it is the state most able to take
        // him to the corner flag.
        //
        // ⚠ NOT for the taker of a restart. A goal kick may be lying four
        // metres behind his own goal line, which is outside every
        // territory there is, and he is the only man on the pitch allowed
        // to touch it — the same exemption the two timeouts below carry,
        // and for the same reason.
        if ctx.tick_context.ball.restart_taker != Some(ctx.player.id) {
            let prof = GoalkeeperSkillProfile::from_ctx(ctx);
            if !KeeperSweepLimit::covers(ctx, ctx.tick_context.positions.ball.position, &prof) {
                return Some(StateChangeResult::with_goalkeeper_state(
                    GoalkeeperState::ReturningToGoal,
                ));
            }
        }

        // ⚠ **Neither timeout applies to the taker of a restart.**
        //
        // Both were written for a keeper who has come off his line after a
        // loose ball and lost the race: giving up after a couple of
        // seconds and going back to his goal is right, because somebody
        // else has the ball. A goal kick is the opposite situation. He is
        // the only man on the pitch allowed to touch it, nobody is racing
        // him, and it may be lying four metres behind his own goal after
        // running out — a walk that comfortably exceeds 200 ticks.
        //
        // Time him out and `clamp_sweep_range` in `Standing` steers him
        // straight back in front of his goal, the 40-tick `TakeMe` nudge
        // drags him out again, and the pair runs as a two-cycle until the
        // patience bound teleports the ball to him — which is the artefact
        // the walk exists to remove. Same shape as the `RestartHold`
        // exemption from the loose-ball election, and for the same reason:
        // a dead ball is not a loose ball.
        if ctx.tick_context.ball.restart_taker != Some(ctx.player.id) {
            // Timeout after 120 ticks — but only if ball isn't very close
            // If ball is close, keep trying instead of giving up
            if ctx.in_state_time > 120 && ctx.ball().distance() > 10.0 {
                return Some(StateChangeResult::with_goalkeeper_state(
                    GoalkeeperState::Standing,
                ));
            }

            // Hard timeout after 200 ticks regardless
            if ctx.in_state_time > 200 {
                return Some(StateChangeResult::with_goalkeeper_state(
                    GoalkeeperState::Standing,
                ));
            }
        }

        None
    }

    fn velocity(&self, ctx: &StateProcessingContext) -> Option<Vector3<f32>> {
        // Use Seek for full-speed approach - no slowing when chasing a loose ball
        // …but never past the edge of his own ground. `process` decides
        // when the chase is over; this is what stops him overrunning it
        // and then having to sprint back, which is the version of the same
        // bug that shows from the stands as a keeper with his back to the
        // play. The restart taker is exempt for the reason given there.
        let target = ctx.tick_context.positions.ball.position;
        let target = if ctx.tick_context.ball.restart_taker == Some(ctx.player.id) {
            target
        } else {
            KeeperSweepLimit::contain(ctx, target, &GoalkeeperSkillProfile::from_ctx(ctx))
        };

        // A keeper sprinting off their line to claim a loose ball moves
        // like one rushing out, not like one shuffling along the six-yard
        // box. `Seek` caps at the OUTFIELD base speed, and the movement
        // integrator then applies the goalkeeper `Active` ceiling — so
        // without scaling here the ceiling can never bind and the keeper
        // arrives at walking pace. Scale by the same agility/acceleration
        // profile the ceiling is derived from, capped so this stays a
        // ceiling-filling multiplier and never outruns it.
        let gk_ceiling = ctx.player.skills.goalkeeper_max_speed(
            ctx.player.player_attributes.condition,
            GoalkeeperSpeedContext::Active,
        );
        let base_speed = ctx.player.max_speed_with_condition_cached();
        let urgency = if base_speed > 0.0 {
            (gk_ceiling / base_speed).clamp(1.0, 1.5)
        } else {
            1.0
        };
        let max_speed = base_speed * urgency;

        let mut arrive_velocity = SteeringBehavior::Seek { target }
            .calculate(ctx.player)
            .velocity
            * urgency;

        // Add separation force to prevent player stacking
        // BUT reduce separation when very close to ball to allow claiming
        const SEPARATION_RADIUS: f32 = 25.0;
        const SEPARATION_WEIGHT: f32 = 0.4;
        const BALL_CLAIM_DISTANCE: f32 = 10.0;
        const NO_SEPARATION_DISTANCE: f32 = 5.0;

        let distance_to_ball = (ctx.player.position - target).magnitude();
        let separation_factor = if distance_to_ball < NO_SEPARATION_DISTANCE {
            0.0 // No separation at all — let the keeper reach the ball
        } else if distance_to_ball < BALL_CLAIM_DISTANCE {
            let linear_factor = (distance_to_ball - NO_SEPARATION_DISTANCE)
                / (BALL_CLAIM_DISTANCE - NO_SEPARATION_DISTANCE);
            linear_factor * 0.3
        } else {
            1.0
        };

        let mut separation_force = Vector3::zeros();
        let mut neighbor_count = 0;

        // Check all nearby players (teammates and opponents)
        let players_view = ctx.players();
        let teammates_view = players_view.teammates();
        let opponents_view = players_view.opponents();
        let all_players = teammates_view
            .all()
            .chain(opponents_view.all())
            .filter(|p| p.id != ctx.player.id);

        for other_player in all_players {
            let to_player = ctx.player.position - other_player.position;
            let distance = to_player.magnitude();

            if distance > 0.0 && distance < SEPARATION_RADIUS {
                // Repulsive force inversely proportional to distance
                let repulsion_strength = (SEPARATION_RADIUS - distance) / SEPARATION_RADIUS;
                separation_force += to_player.normalize() * repulsion_strength;
                neighbor_count += 1;
            }
        }

        if neighbor_count > 0 {
            // Average and scale the separation force
            separation_force = separation_force / (neighbor_count as f32);
            separation_force = separation_force * max_speed * SEPARATION_WEIGHT * separation_factor;

            // ⚠ SEPARATION MUST NEVER SLOW THE RACE — see `LooseBallChase`.
            // A keeper coming for a loose ball is the last man; being
            // repelled off it by the striker he is racing is the worst
            // version of this bug on the pitch.
            separation_force =
                LooseBallChase::keep_non_opposing(separation_force, target - ctx.player.position);

            // Blend arrive and separation velocities
            arrive_velocity = arrive_velocity + separation_force;

            // Limit to the keeper's own chase ceiling — NOT the outfield
            // base, which would undo the urgency scaling above the moment
            // any other player came within the separation radius.
            let magnitude = arrive_velocity.magnitude();
            if magnitude > max_speed {
                arrive_velocity *= max_speed / magnitude;
            }
        }

        Some(arrive_velocity)
    }

    fn process_conditions(&self, ctx: ConditionContext) {
        // Taking ball requires high intensity as goalkeeper moves to claim the ball
        GoalkeeperCondition::with_velocity(ActivityIntensity::High).process(ctx);
    }
}
