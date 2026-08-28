use crate::r#match::forwarders::states::ForwardState;
use crate::r#match::forwarders::states::common::{ActivityIntensity, ForwardCondition};
use crate::r#match::player::strategies::common::players::ops::box_movement::BoxMovement;
use crate::r#match::player::strategies::players::skills::SkillCurve;
use crate::r#match::{
    ConditionContext, StateChangeResult, StateProcessingContext, StateProcessingHandler,
    SteeringBehavior,
};
use nalgebra::Vector3;
use std::cmp::Ordering;

#[derive(Default, Clone)]
pub struct ForwardAssistingState {}

impl StateProcessingHandler for ForwardAssistingState {
    fn process(&self, ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        // Take ball only if best positioned — prevents swarming
        if ctx.ball().should_take_ball_immediately() && ctx.team().is_best_player_to_chase_ball() {
            return Some(StateChangeResult::with_forward_state(
                ForwardState::TakeBall,
            ));
        }

        if !ctx.team().is_control_ball() {
            return Some(StateChangeResult::with_forward_state(ForwardState::Running));
        }

        // A branch routing to `Intercepting` used to sit here, and it
        // could only ever fire while OUR side had the ball — the check
        // above returns `Running` otherwise. `Intercepting`'s first act
        // is to reject exactly that (`is_control_ball` => `Returning`),
        // so the net effect was that a forward with a team-mate's pass
        // travelling toward him turned and ran back. Measured:
        // `Forward: Intercepting` exited after a mean of **0.0 AI ticks**
        // with 99.7% of visits lasting a single tick.
        //
        // You do not intercept your own team's pass, you receive it, and
        // the dispatcher already designates the intended receiver as the
        // chaser (see `should_force_takeball`'s `pass_target` rule). So
        // there is nothing to do here but keep making the run.

        // Check if the player is on the opponent's side of the field
        if ctx.team().is_control_ball()
            && !ctx.player().on_own_side()
            && ctx.players().opponents().exists(100.0)
        {
            // If not on the opponent's side, focus on creating space and moving forward
            return Some(StateChangeResult::with_forward_state(
                ForwardState::CreatingSpace,
            ));
        }

        // Ball-required actions only while actually carrying it. This
        // state is usually entered OFF the ball (support play after a
        // press/tackle win, or from CreatingSpace) — and Passing /
        // Dribbling both bounce straight back to Running when entered
        // without the ball, so routing there off-ball just churned
        // transitions instead of assisting.
        if ctx.player.has_ball(ctx) {
            // Under immediate pressure: quick release to a runner if the
            // passer's skills allow it, otherwise carry out of pressure.
            if self.is_under_pressure(ctx) {
                if self.should_make_quick_pass(ctx)
                    && self.find_best_teammate_to_assist(ctx).is_some()
                {
                    return Some(StateChangeResult::with_forward_state(ForwardState::Passing));
                }
                return Some(StateChangeResult::with_forward_state(
                    ForwardState::Dribbling,
                ));
            }

            // If not under immediate pressure, look for assist opportunities
            if self.find_best_teammate_to_assist(ctx).is_some() {
                return Some(StateChangeResult::with_forward_state(ForwardState::Passing));
            }

            if self.is_in_shooting_range(ctx) {
                return Some(StateChangeResult::with_forward_state(
                    ForwardState::Shooting,
                ));
            }
        }

        // Off the ball, hand movement back to Running immediately —
        // Running owns the off-ball attacking repertoire (runs in
        // behind, box entries, shot logic). Letting Assisting persist
        // off-ball parks a forward on an Arrive-at-goal vector where he
        // stops generating runs AND reads as a "much better positioned
        // teammate" to every carrier's defer gate — the pre-fix code
        // reached the same Running hand-off accidentally, by bouncing
        // through Passing/Dribbling's !has_ball guards.
        if !ctx.player.has_ball(ctx) {
            if self.should_create_space(ctx) {
                return Some(StateChangeResult::with_forward_state(
                    ForwardState::CreatingSpace,
                ));
            }
            return Some(StateChangeResult::with_forward_state(ForwardState::Running));
        }

        // Carrier with nothing better to do: keep supporting movement.
        if self.should_create_space(ctx) {
            return Some(StateChangeResult::with_forward_state(
                ForwardState::CreatingSpace,
            ));
        }

        None
    }

    fn velocity(&self, ctx: &StateProcessingContext) -> Option<Vector3<f32>> {
        // An assigned patch of the box outranks the generic support
        // position, and it has to be honoured HERE as well as in
        // `CreatingSpace` — the two bounce off each other roughly every
        // sixteen ticks (see the note in `process`), so a slot honoured
        // in one and ignored in the other means the occupant spends half
        // his ticks being staged toward the box and the other half being
        // pulled to a fixed point 11 m off the goal line. See
        // `BoxMovement`.
        if let Some(slot) = ctx.team().my_box_slot() {
            return Some(BoxMovement::steer(ctx, slot));
        }

        // Take up a supporting position in the box, not the goal itself.
        //
        // This used to `Arrive` at `opponent_goal_position()` — a fixed
        // point ON the goal line. This state is entered mostly OFF the
        // ball (see `process`), so every assisting forward converged into
        // the net, where they stacked on one spot and ground against the
        // pitch boundary. It was the highest-RATE flicker state left in
        // the engine at ~2.7 velocity reversals per second held
        // (`dev_match trace`), and standing in the goal is not a
        // supporting position in the first place.
        //
        // Depth comes back off the line toward the ball, so the forward
        // occupies the area a cutback or cross is played into. Laterally
        // he mostly holds his own channel, drawn gently central — that
        // keeps forwards in separate lanes instead of piling onto a
        // single aim point, and it is continuous in his own position, so
        // it cannot introduce a jump.
        const SUPPORT_DEPTH: f32 = 90.0; // ~11 m off the line
        let goal = ctx.player().opponent_goal_position();
        let ball = ctx.tick_context.positions.ball.position;
        let to_ball = ball - goal;
        let dir = if to_ball.magnitude() > 0.01 {
            to_ball.normalize()
        } else {
            Vector3::new(1.0, 0.0, 0.0)
        };
        let depth_point = goal + dir * SUPPORT_DEPTH;
        let target = Vector3::new(
            depth_point.x,
            ctx.player.position.y * 0.7 + goal.y * 0.3,
            0.0,
        );

        Some(
            SteeringBehavior::Arrive {
                target,
                slowing_distance: 20.0,
            }
            .calculate(ctx.player)
            .velocity,
        )
    }

    fn process_conditions(&self, ctx: ConditionContext) {
        // Assisting is moderate intensity - supporting movement
        ForwardCondition::with_velocity(ActivityIntensity::Moderate).process(ctx);
    }
}

impl ForwardAssistingState {
    fn is_under_pressure(&self, ctx: &StateProcessingContext) -> bool {
        ctx.player().pressure().is_under_immediate_pressure()
    }

    fn should_make_quick_pass(&self, ctx: &StateProcessingContext) -> bool {
        // Quick-pass willingness blends passing and decision-making
        // smoothly across 1-20. Sigmoid pivots (13/12) match the
        // "competent passer" / "decent decision-maker" intent of the
        // original `> 70/65` gate (which was bugged for the 1-20 scale
        // and never fired). Product of two curves so both skills matter.
        let pass_p = SkillCurve::new(ctx.player.skills.technical.passing, 13.0, 0.6).probability();
        let dec_p = SkillCurve::new(ctx.player.skills.mental.decisions, 12.0, 0.6).probability();
        ctx.context.rng.unit_f32() < pass_p * dec_p
    }

    fn find_best_teammate_to_assist(&self, ctx: &StateProcessingContext) -> Option<u32> {
        ctx.players()
            .teammates()
            .nearby_ids(200.0)
            .filter(|(id, _)| self.is_in_good_scoring_position(ctx, *id))
            .min_by(|(_, dist_a), (_, dist_b)| {
                dist_a.partial_cmp(dist_b).unwrap_or(Ordering::Equal)
            })
            .map(|(id, _)| id)
    }

    fn is_in_good_scoring_position(&self, ctx: &StateProcessingContext, player_id: u32) -> bool {
        // Find the teammate's actual position
        if let Some(teammate) = ctx.players().teammates().all().find(|p| p.id == player_id) {
            let goal_pos = ctx.player().opponent_goal_position();
            let distance_to_goal = (teammate.position - goal_pos).magnitude();

            // Good scoring position: within 35m of goal
            if distance_to_goal > 350.0 {
                return false;
            }

            // Check if teammate has space (not heavily marked)
            let close_defenders = ctx.tick_context.grid.opponents(teammate.id, 10.0).count();

            // Good if close to goal with some space or is another forward
            distance_to_goal < 350.0
                && (close_defenders < 2 || teammate.tactical_positions.is_forward())
        } else {
            false
        }
    }

    fn is_in_shooting_range(&self, ctx: &StateProcessingContext) -> bool {
        let distance_to_goal = ctx.ball().distance_to_opponent_goal();
        distance_to_goal < 25.0
    }

    fn should_create_space(&self, ctx: &StateProcessingContext) -> bool {
        if !ctx.players().teammates().exists(100.0) {
            return false;
        }
        // Off-the-ball skill scales the urge to create space smoothly —
        // elite movers (15+) almost always do it; ordinary 10s do it
        // sometimes; very poor (sub-5) almost never.
        let p = SkillCurve::new(ctx.player.skills.mental.off_the_ball, 15.0, 0.6).probability();
        ctx.context.rng.unit_f32() < p
    }
}
