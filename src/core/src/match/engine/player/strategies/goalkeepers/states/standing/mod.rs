use crate::r#match::goalkeepers::states::common::{
    ActivityIntensity, GoalkeeperCondition, KeeperRead, KeeperRestPosition, KeeperSetPieceStance,
    ReadLevel,
};
use crate::r#match::player::strategies::players::ops::goalkeeper_skill::GoalkeeperSkillProfile;
use crate::r#match::{
    ConditionContext, PlayerSide, StateChangeResult, StateProcessingContext,
    StateProcessingHandler, SteeringBehavior,
};
use nalgebra::Vector3;

/// **Alert**: the opposition is coming at him, or the ball is near his goal.
/// He holds his rest point on his toes, and every situation is the read's
/// (`KeeperRead`); the alert and relaxed phases differ in how he holds
/// himself and how fast he moves, never in where he stands.
#[derive(Default, Clone)]
pub struct GoalkeeperStandingState {}

impl StateProcessingHandler for GoalkeeperStandingState {
    fn process(&self, ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        KeeperRead::from(ctx, ReadLevel::Possession).go()
    }

    fn velocity(&self, ctx: &StateProcessingContext) -> Option<Vector3<f32>> {
        // A dead ball at his goal: to his mark, and stand on it. Ahead of
        // the rest model because that model's tolerances open with the
        // ball's distance, and a corner is thirty-four metres away — read
        // through them he was "set" anywhere within three metres of the
        // line and never went to it. See [`KeeperSetPieceStance`].
        if let Some(to_mark) = KeeperSetPieceStance::steer(ctx) {
            return Some(to_mark);
        }

        let optimal_position = self.calculate_optimal_position(ctx);

        // A keeper is not always repositioning. Standing still, set, is a
        // real and common thing for him to be doing, and the target moves
        // continuously with the ball — so without a deadzone he chases it
        // every tick and ends up covering more ground than a midfielder.
        //
        // But the tolerance is ANISOTROPIC, and that is the whole point:
        // slack in depth, where a metre is nothing, and tight across the
        // goal, where a metre is a goal. See
        // `KeeperRestPosition::LATERAL_DEADZONE`.
        if KeeperRestPosition::is_set_with(
            ctx.player.position,
            optimal_position,
            GoalkeeperSkillProfile::from_ctx(ctx).concentration,
            ctx.ball().distance(),
            ctx.context.field_size.width as f32,
        ) {
            return Some(Vector3::zeros());
        }

        // URGENCY. How fast he travels is set by how near the ball is, not
        // by how far he has to go — which is the difference between a
        // keeper strolling out to the edge of his box while play is at the
        // other end, and the same keeper sprinting to the same spot with
        // a striker running through. Without this he moves at repositioning
        // pace always, and covers 12.8 km a match against a real ~5 km.
        let ball_distance = ctx.ball().distance();
        let field_width = ctx.context.field_size.width as f32;
        let urgency = (1.0 - ball_distance / (field_width * 0.55)).clamp(0.0, 1.0);
        // Amble ↔ sprint.
        let pace = 0.18 + urgency * urgency * 1.12;

        Some(
            SteeringBehavior::Arrive {
                target: optimal_position,
                slowing_distance: 8.0,
            }
            .calculate(ctx.player)
            .velocity
                * pace,
        )
    }

    fn process_conditions(&self, ctx: ConditionContext) {
        // Standing goalkeepers recover condition well due to low activity
        GoalkeeperCondition::new(ActivityIntensity::Recovery).process(ctx);
    }
}

impl GoalkeeperStandingState {
    /// Calculate optimal goalkeeper position based on ball and goal
    /// Where the keeper should be standing.
    ///
    /// # Why this was rebuilt
    ///
    /// A keeper in this engine stood on his line and never left it. He
    /// spent **88% of the match motionless** and sat 11.4 m deeper than
    /// even the modest anchor the team shape gave him, which is the "only
    /// the GK stays in the goal" report.
    ///
    /// The old model had two faults, and the first is the interesting one:
    /// **its depth was inverted.** With the ball in the opponent's half it
    /// put him `12 + command * 8` units — **1.5 to 2.5 metres** — off his
    /// line. That is the moment a real keeper is FURTHEST out, 20-30 m up,
    /// sweeping behind his defence. With the ball near him it brought him
    /// *further* out. Across every input the whole range came to about
    /// 1-4 m, so nothing he did was visible.
    ///
    /// Second, every result was clamped inside the penalty area, so
    /// sweeping up to meet a ball played in behind was impossible by
    /// construction.
    ///
    /// # The model
    ///
    /// A keeper's depth is set by **where the ball is** and **how high his
    /// own defence is**, and his lateral position by **the angle** — he
    /// stands on the line from the centre of his goal to the ball, which
    /// is what "narrowing the angle" physically means.
    ///
    /// Real reference points, which the constants below reproduce:
    ///   * ball in the opponent's half behind a high line — 20-28 m out,
    ///     effectively a sweeper;
    ///   * ball in midfield — 12-18 m;
    ///   * ball entering our final third — 6-10 m;
    ///   * ball in the box — 2-5 m, on the angle, ready to spread.
    fn calculate_optimal_position(&self, ctx: &StateProcessingContext) -> Vector3<f32> {
        self.clamp_sweep_range(ctx, KeeperRestPosition::for_keeper(ctx))
    }

    /// Bound the keeper's standing position.
    ///
    /// Replaces a hard clamp into the penalty area. That clamp made
    /// sweeping impossible by construction — a keeper cannot come and
    /// meet a ball played in behind if he is not allowed out of his box —
    /// and it is not what the laws or the football say: a keeper may go
    /// anywhere, he just cannot handle the ball outside the area, which
    /// the handling code enforces separately.
    ///
    /// What bounds him is [`KeeperSweepLimit`], and
    /// `KeeperRestPosition::for_keeper` has already applied it — to the
    /// same territory the sweep gates use, and for `Walking` and
    /// `ReturningToGoal` as well as for this state. What is left here is
    /// the backstop that is a property of the PITCH rather than of the
    /// keeper: he is never behind his own goal line and never past
    /// halfway.
    ///
    /// ⚠ The lateral half of this clamp — into the width of the penalty
    /// area — used to be the ONLY thing in the engine that bounded a
    /// keeper across the pitch, and it bounded a target rather than the
    /// keeper: the moment he left this state for `ComingOut`, `TakeBall`
    /// or `Catching` there was nothing at all. See the note on
    /// `KeeperSweepLimit` for what that measured.
    fn clamp_sweep_range(
        &self,
        ctx: &StateProcessingContext,
        position: Vector3<f32>,
    ) -> Vector3<f32> {
        let goal_center = ctx.ball().direction_to_own_goal();
        let side = ctx.player.side.unwrap_or(PlayerSide::Left);
        let field_width = ctx.context.field_size.width as f32;

        let far_x = goal_center.x + side.forward_dir_x() * (field_width * 0.5 - 20.0);
        let (lo_x, hi_x) = if side.forward_dir_x() > 0.0 {
            (goal_center.x, far_x)
        } else {
            (far_x, goal_center.x)
        };

        Vector3::new(position.x.clamp(lo_x, hi_x), position.y, 0.0)
    }
}
