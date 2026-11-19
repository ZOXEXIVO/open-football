use crate::r#match::engine::ball::ball::OffsideLine;
use crate::r#match::midfielders::states::common::{
    ActivityIntensity, MidfielderCondition, MidfielderRead,
};
use crate::r#match::midfielders::states::{MidfielderRunningState, MidfielderState};
use crate::r#match::{
    ConditionContext, StateChangeResult, StateProcessingContext, StateProcessingHandler,
    SteeringBehavior,
};
use nalgebra::Vector3;

/// **The run the attack plan gave him**: his place in the box, or the run
/// in behind, held at the offside line. He holds it while the read gives it
/// to him.
#[derive(Default, Clone)]
pub struct MidfielderAttackSupportingState {}

impl StateProcessingHandler for MidfielderAttackSupportingState {
    fn process(&self, ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        MidfielderRead::hold(ctx, MidfielderState::AttackSupporting)
    }

    fn velocity(&self, ctx: &StateProcessingContext) -> Option<Vector3<f32>> {
        Some(self.given_run(ctx))
    }

    fn process_conditions(&self, ctx: ConditionContext) {
        // Attack supporting is high intensity - sustained running to support attacks
        //
        // NOTE — `Moderate` was tried here and REVERTED; don't repeat it
        // without a plan for the cost. The argument for it is good on
        // paper: the declared intensity is also the speed cap
        // (`MovementEffort::speed_fraction`: High 0.78 vs Moderate 0.52),
        // this is where midfielders spend most of the match (7.3M of
        // ~14M midfielder ticks, ~24 minutes each), and `Moderate` is the
        // tier documented as "jogging into space" while `High` reads
        // "pressing, marking, covering, tracking back".
        //
        // Measured over 5x60 matches it bought less than it cost: MID
        // distance 20.1 -> 18.2 km against a real ~11 (only -9%, because
        // midfielders are in SOME motion state ~75% of the time either
        // way), while goals went 4.19 -> 4.64 per match against a real
        // ~2.5 — slower support means less presence around the ball.
        //
        // The real problem is not this tier, it is that midfielders never
        // stand still: they have no low-intensity dwell state in their
        // rotation the way defenders have `HoldingLine` (`Recovery`),
        // which is why DEF measures a realistic 10.0 km on the same
        // model. Fixing the distance means giving midfielders somewhere
        // to rest, not making their running slower.
        MidfielderCondition::new(ActivityIntensity::High).process(ctx);
    }
}

impl MidfielderAttackSupportingState {
    /// How far from goal the run in behind is aimed (15 m).
    const IN_BEHIND: f32 = 120.0;

    /// The run the attack plan gave him: his place in the box, staged as
    /// the ball comes, or the run in behind, both held at the offside line.
    /// With the ball at his feet before the on-ball state takes him, he
    /// carries it as that state does.
    fn given_run(&self, ctx: &StateProcessingContext) -> Vector3<f32> {
        if ctx.player.has_ball(ctx) {
            return MidfielderRunningState::default().carry(ctx);
        }
        let team = ctx.team();
        let target = if let Some(place) = team.my_box_slot_target() {
            place
        } else if team.attack_plan().far_runner == Some(ctx.player.id) {
            let goal = ctx.player().opponent_goal_position();
            let forward = ctx.player.side.map_or(0.0, |side| side.forward_dir_x());
            Vector3::new(
                goal.x - forward * Self::IN_BEHIND,
                ctx.player.position.y + self.calculate_lateral_run_adjustment(ctx),
                0.0,
            )
        } else {
            team.my_anchor()
        };
        let field_width = ctx.context.field_size.width as f32;
        let field_height = ctx.context.field_size.height as f32;
        let target = Self::held_onside(ctx, target).clamp_to_field(field_width, field_height);
        if (target - ctx.player.position).magnitude() < 8.0 {
            return Vector3::zeros();
        }
        let urgency = self.calculate_urgency_factor(ctx);
        SteeringBehavior::Arrive {
            target,
            slowing_distance: 20.0 * (1.0 - urgency * 0.3),
        }
        .calculate(ctx.player)
        .velocity
    }

    /// `target` brought back to the offside line the referee reads, when
    /// it is beyond it.
    fn held_onside(ctx: &StateProcessingContext, target: Vector3<f32>) -> Vector3<f32> {
        let Some(side) = ctx.player.side else {
            return target;
        };
        let line =
            OffsideLine::second_last(ctx.players().opponents().all().map(|o| o.position.x), side);
        let Some(line_x) = line else {
            return target;
        };
        let ball_x = ctx.tick_context.positions.ball.position.x;
        Vector3::new(
            OffsideLine::onside_x(side, target.x, ball_x, line_x),
            target.y,
            target.z,
        )
    }

    /// Wider when the centre-backs are narrow, inside from a wide start.
    fn calculate_lateral_run_adjustment(&self, ctx: &StateProcessingContext) -> f32 {
        let field_height = ctx.context.field_size.height as f32;
        let player_y = ctx.player.position.y;

        // Check defender positioning — only nearby opponents matter
        let center_y = field_height / 2.0;
        let central_band = field_height * 0.2;
        let defenders_central = ctx
            .players()
            .opponents()
            .nearby(200.0)
            .filter(|opp| {
                opp.tactical_positions.is_defender()
                    && (opp.position.y - center_y).abs() < central_band
            })
            .count();

        // If defenders are concentrated centrally, make wider runs
        if defenders_central >= 2 {
            if player_y < field_height / 2.0 {
                -30.0 // Go wider left
            } else {
                30.0 // Go wider right
            }
        } else {
            // Make central runs if space exists
            if (player_y - field_height / 2.0).abs() > field_height * 0.25 {
                if player_y < field_height / 2.0 {
                    20.0 // Come inside from left
                } else {
                    -20.0 // Come inside from right
                }
            } else {
                0.0
            }
        }
    }

    /// Calculate urgency factor for movement
    fn calculate_urgency_factor(&self, ctx: &StateProcessingContext) -> f32 {
        let mut urgency: f32 = 0.5;

        // Increase urgency if team is losing
        if ctx.team().is_loosing() {
            urgency += 0.2;
        }

        // Increase urgency late in game
        if ctx.context.is_running_out() {
            urgency += 0.2;
        }

        // Increase urgency if good attacking opportunity
        if ctx.ball().distance_to_opponent_goal() < 200.0 {
            urgency += 0.1;
        }

        urgency.min(1.0)
    }
}

/// Extension trait for Vector3 to clamp to field
trait VectorFieldExtensions {
    fn clamp_to_field(self, field_width: f32, field_height: f32) -> Self;
}

impl VectorFieldExtensions for Vector3<f32> {
    fn clamp_to_field(self, field_width: f32, field_height: f32) -> Self {
        Vector3::new(
            self.x.clamp(10.0, field_width - 10.0),
            self.y.clamp(10.0, field_height - 10.0),
            self.z,
        )
    }
}
