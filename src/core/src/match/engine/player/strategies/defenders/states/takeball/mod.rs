use crate::r#match::defenders::states::DefenderState;
use crate::r#match::defenders::states::common::{ActivityIntensity, DefenderCondition};
use crate::r#match::player::strategies::common::states::LooseBallChase;
use crate::r#match::{
    ConditionContext, StateChangeResult, StateProcessingContext, StateProcessingHandler,
};
use nalgebra::Vector3;

#[derive(Default, Clone)]
pub struct DefenderTakeBallState {}

impl StateProcessingHandler for DefenderTakeBallState {
    fn process(&self, ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        // WE own the ball → TakeBall is the wrong state. Drop to
        // Running so the ball-on-foot paths can clear / pass / dribble.
        if ctx.player.has_ball(ctx) {
            return Some(StateChangeResult::with_defender_state(
                DefenderState::Running,
            ));
        }
        // Ball got claimed. Running handles teammate/opponent ownership —
        // hand off there instead of duplicating the dispatch here.
        if ctx.ball().is_owned() {
            return Some(StateChangeResult::with_defender_state(
                DefenderState::Running,
            ));
        }

        // Ball is loose: commit. No distance cap, no teammate yield, no
        // "opponent is closer" bailout. The Running state's
        // `is_best_player_to_chase_ball` already committed this player.
        // Spatial-proximity checks against stationary rivals created
        // stalemates where nobody actually went for the ball.
        None
    }
    fn velocity(&self, ctx: &StateProcessingContext) -> Option<Vector3<f32>> {
        // Run to where the ball is GOING. The aim point crosses smoothly
        // from the meeting point on the ground to the landing spot as the
        // ball rises — see `LooseBallChase::aim`.
        //
        // ⚠ THIS STATE AIMED AT WHERE THE BALL WAS, FOR ITS WHOLE LIFE.
        //
        // `Ball::calculate_landing_position` returns the ball's own
        // position for anything on the turf, and `Pursuit` — the one
        // thing meant to lead a moving target — clamps its lead to five
        // TICKS. So a defender chasing a ground pass was steered at a
        // point inside half a metre of the ball on every tick, which is a
        // tail chase: his heading converges on the ball's and he trails
        // it at a fixed gap. Reported from the viewer as *"defenders with
        // TakeBall don't intercept, they run parallel with the ball"*,
        // and measured at 44% of a defender's samples in that state
        // (`mid_run_diag::CHASE_SAMPLES`). `SteeringBehavior::Intercept`
        // carries the numbers and `OF_TAIL_CHASE` restores the old model.
        //
        // ── on the flicker note this comment used to carry ────────────
        //
        // It listed five ruled-out hypotheses for `Defender: Take Ball`
        // being the engine's largest source of velocity reversals at
        // 5.8-6.5 per second held, and said the next attempt should
        // instrument rather than inspect. The instrument built for the
        // chase report is not that instrument — it measures aim, not
        // stability — but `dev_match trace` no longer ranks this state in
        // the top twelve at all, in EITHER arm of `OF_TAIL_CHASE`. So the
        // flicker was fixed by something else, some time between that
        // note and now, and the five hypotheses are left recorded below
        // because they are still worth not repeating:
        //
        //   1. the aerial/ground aim point snapping at `z > 2.3`
        //      (blended — kept, in `LooseBallChase::aim`);
        //   2. the separation weight's 0.3 -> 1.0 step at 10u (smoothed);
        //   3. the separation force entirely (disabled outright);
        //   4. `Pursuit`'s discontinuous interception solver (rewritten
        //      continuous — kept, in `steering.rs`);
        //   5. `Pursuit` swapped for `Arrive` across the whole range.
        //
        // Also ruled OUT: the chaser faithfully tracking a jittery ball.
        // The ball's own direction was measured reversing **6 times in
        // 1.38M ticks** (the `(ball direction changes)` control row), so
        // the target is essentially perfectly stable and any instability
        // is generated on the player side.
        let (target, mut arrive_velocity) = LooseBallChase::aim(ctx);
        let (target, mut arrive_velocity) = LooseBallChase::aim(ctx);

        // Add separation force to prevent player stacking
        // Reduce separation when approaching ball, but keep minimum to prevent clustering
        const SEPARATION_RADIUS: f32 = 25.0;
        const SEPARATION_WEIGHT: f32 = 0.4;
        const NO_SEPARATION_DISTANCE: f32 = 5.0; // Completely disable separation within this distance

        // NOTE — this ramp has a genuine discontinuity: it runs 0 -> 0.3
        // across 5..10u and then JUMPS to 1.0 beyond 10u, so a chaser
        // hovering either side of 10u sees the force opposing his pursuit
        // change by more than 3x between ticks. Smoothing it into one
        // continuous ramp was tried and did NOT reduce this state's
        // reversal rate, so it is not the cause of the `Defender: Take
        // Ball` flicker, and the smoothed version changes how tightly
        // chasers converge on a loose ball. Left as-is pending a real
        // diagnosis; the step is documented so the next attempt starts
        // from what is already known.
        const BALL_CLAIM_DISTANCE: f32 = 10.0;
        let distance_to_ball = (ctx.player.position - target).magnitude();
        let separation_factor = if distance_to_ball < NO_SEPARATION_DISTANCE {
            0.0 // No separation at all — let the player reach the ball
        } else if distance_to_ball < BALL_CLAIM_DISTANCE {
            let linear_factor = (distance_to_ball - NO_SEPARATION_DISTANCE)
                / (BALL_CLAIM_DISTANCE - NO_SEPARATION_DISTANCE);
            linear_factor * 0.3 // Gentle ramp from 0 to 0.3
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
            separation_force = separation_force
                * ctx.player.max_speed_with_condition_cached()
                * SEPARATION_WEIGHT
                * separation_factor;

            // ⚠ SEPARATION MUST NEVER SLOW THE RACE — see `LooseBallChase`.
            // Without this an opponent standing between the defender and
            // the ball repels him **away from the ball** at up to 0.4 of
            // top speed, and a rival who started further away wins it.
            separation_force =
                LooseBallChase::keep_non_opposing(separation_force, target - ctx.player.position);

            // Blend arrive and separation velocities
            arrive_velocity = arrive_velocity + separation_force;

            // Limit to max speed
            let magnitude = arrive_velocity.magnitude();
            if magnitude > ctx.player.max_speed_with_condition_cached() {
                arrive_velocity =
                    arrive_velocity * (ctx.player.max_speed_with_condition_cached() / magnitude);
            }
        }

        Some(arrive_velocity)
    }

    fn process_conditions(&self, ctx: ConditionContext) {
        // ⚠ THIS IS A SPEED CAP, NOT JUST FATIGUE ACCOUNTING.
        //
        // `MovementEffort::speed_fraction` turns the intensity into a hard
        // ceiling on how fast the player may move, and this line used to
        // read `Moderate` — **0.52 of his top speed** — with the comment
        // "taking ball involves movement towards ball - moderate
        // intensity". Midfielders and forwards chase the same loose ball
        // at `VeryHigh` (0.95).
        //
        // So a defender racing a forward for a loose ball ran at 55% of
        // the forward's speed. He lost races he started far closer to,
        // which is exactly how it was reported from the viewer: "defenders
        // with Take Ball not running to the ball, and the opponent is
        // first even though he had the bigger distance".
        //
        // `MovementEffort`'s own tier list has always said where this
        // belongs — "Explosive: runs in behind, shooting, tackling,
        // **chasing loose balls**". A defender sprinting for a fifty-fifty
        // is the same action as anyone else sprinting for it; nothing
        // about his shirt number makes it a jog.
        DefenderCondition::with_velocity(ActivityIntensity::VeryHigh).process(ctx);
    }
}
