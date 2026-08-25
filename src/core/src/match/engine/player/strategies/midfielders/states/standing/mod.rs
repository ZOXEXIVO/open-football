use crate::r#match::player::strategies::common::team::WideChannel;
use crate::r#match::midfielders::states::MidfielderState;
use crate::r#match::midfielders::states::common::{
    ActivityIntensity, Interception, MidfielderCondition,
};
use crate::r#match::{
    ConditionContext, PlayerSide, StateChangeResult, StateProcessingContext, StateProcessingHandler,
};
use nalgebra::Vector3;

const PRESSING_DISTANCE_THRESHOLD: f32 = 80.0; // Midfielders press from further out

#[derive(Default, Clone)]
pub struct MidfielderStandingState {}

impl StateProcessingHandler for MidfielderStandingState {
    fn process(&self, ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        // Offside discipline — attack-minded midfielders (AM, wingers)
        // can drift beyond the opposing defensive line. If our team
        // doesn't have the ball, drop back to Returning or any pass
        // upfield will catch us offside.
        if !ctx.player.has_ball(ctx) && ctx.player().defensive().is_stranded_offside() {
            return Some(StateChangeResult::with_midfielder_state(
                MidfielderState::Returning,
            ));
        }

        if ctx.player.has_ball(ctx) {
            // With the ball and no passing options, stay in Standing — the
            // top-of-function idle logic will refresh next tick.
            //
            // With options, the choice is between two different jobs.
            // `Passing` runs the full progressive evaluation — it is
            // looking to advance the ball. `Distributing` picks purely on
            // space around the receiver: it is the deep-lying playmaker
            // recycling possession, keeping the ball moving while the
            // shape reforms. That second state had no inbound transition
            // at all, so a midfielder in settled possession with nothing
            // on always went looking for a forward pass.
            return if !self.has_passing_options(ctx) {
                // Nobody in range to pass to — carry it.
                //
                // This used to return `None`: stay in `Standing`, whose
                // `velocity` is a hard zero. The same evaluation runs
                // next tick and gives the same answer, so the player
                // stood motionless over the ball until the ball's own
                // stall detector took it off him — a fixed point, not a
                // decision. Measured from a replay dump: **11 episodes of
                // exactly 19.5 s a match**, three and a half minutes of a
                // midfielder standing on the ball while the game waited.
                // (19.5 s because `STALL_TICKS = 1000` is counted in full
                // ticks, not the 10 s its comment claims.)
                //
                // It fires far more often than "no options" suggests:
                // `has_passing_options` only looks 30u — 3.75 m — so any
                // time the nearest team-mate is more than four metres
                // away, which in open play is most of the time, the
                // midfielder had nothing to do.
                //
                // A midfielder with the ball and no short option drives
                // into the space in front of him. That is what a real one
                // does, and it is what breaks the fixed point.
                Some(StateChangeResult::with_midfielder_state(
                    MidfielderState::Dribbling,
                ))
            } else if self.should_recycle_possession(ctx) {
                Some(StateChangeResult::with_midfielder_state(
                    MidfielderState::Distributing,
                ))
            } else {
                Some(StateChangeResult::with_midfielder_state(
                    MidfielderState::Passing,
                ))
            };
        } else {
            // Loose-ball claim lives in the dispatcher.

            if ctx.team().is_control_ball() {
                // If teammates are clustered nearby, create space instead of running
                let nearby_teammates = ctx.players().teammates().nearby(25.0).count();
                if nearby_teammates >= 2 && ctx.ball().distance() > 30.0 {
                    return Some(StateChangeResult::with_midfielder_state(
                        MidfielderState::CreatingSpace,
                    ));
                }
                // With our team on the ball at the far end of the pitch
                // and nothing to chase, a midfielder walks — they do not
                // break into a run every time possession settles. This is
                // the ONLY entry into `MidfielderState::Walking`, and it
                // has to live inside this branch: everything below returns
                // before the state's timeout, so a walk gate placed there
                // was unreachable by construction (the state stayed at
                // 0 observed entries across 200 matches).
                return Some(StateChangeResult::with_midfielder_state(
                    if self.should_walk(ctx) {
                        MidfielderState::Walking
                    } else {
                        MidfielderState::Running
                    },
                ));
            } else {
                // Only press/tackle if an OPPONENT has the ball AND we're the best chaser
                if let Some(_opponent) = ctx.players().opponents().with_ball().next() {
                    if ctx.ball().distance() < PRESSING_DISTANCE_THRESHOLD
                        && ctx.team().is_best_player_to_chase_ball()
                    {
                        return Some(StateChangeResult::with_midfielder_state(
                            MidfielderState::Pressing,
                        ));
                    }

                    // Second closest can press from very short range only
                    if ctx.ball().distance() < 20.0 {
                        return Some(StateChangeResult::with_midfielder_state(
                            MidfielderState::Pressing,
                        ));
                    }
                }

                // Ball in flight (clearance, long pass) — go contest the
                // landing zone. Without this, clearances to midfield always
                // end up at the opposing team's feet because we only
                // intercept when the ball is already headed directly at
                // us. Midfielders are the default contester of loose
                // balls in the middle third; the predicted landing
                // position gives them a runway to reach it.
                if Interception::is_available(ctx) && ctx.ball().is_in_flight() {
                    let landing = ctx.tick_context.positions.ball.landing_position;
                    let dist_to_landing = (landing - ctx.player.position).magnitude();
                    if dist_to_landing < 100.0 {
                        return Some(StateChangeResult::with_midfielder_state(
                            MidfielderState::Intercepting,
                        ));
                    }
                }

                // Loose + heading toward us — stay with the original tight
                // trigger (angle gate filters passes that aren't coming
                // our way).
                if Interception::is_available(ctx)
                    && ctx.ball().distance() < 250.0
                    && ctx.ball().is_towards_player_with_angle(0.8)
                {
                    return Some(StateChangeResult::with_midfielder_state(
                        MidfielderState::Intercepting,
                    ));
                }

                // Guard unmarked attackers on our side
                if ctx.ball().on_own_side() {
                    return Some(StateChangeResult::with_midfielder_state(
                        MidfielderState::Guarding,
                    ));
                }
            }
        }

        // Only press if opponent is nearby AND has the ball AND we're closest
        if let Some(opponent) = ctx.players().opponents().with_ball().next() {
            if opponent.distance(ctx) < PRESSING_DISTANCE_THRESHOLD
                && ctx.team().is_best_player_to_chase_ball()
            {
                return Some(StateChangeResult::with_midfielder_state(
                    MidfielderState::Pressing,
                ));
            }
        }

        // Check if a teammate is making a run and needs support
        if self.should_support_attack(ctx) {
            return Some(StateChangeResult::with_midfielder_state(
                MidfielderState::AttackSupporting,
            ));
        }

        // Midfielders should not stand still for long — get moving quickly.
        // (The walk-or-run fork lives in the possession branch above; by
        // the time control has been lost this player has somewhere to be.)
        if ctx.in_state_time > 8 {
            return Some(StateChangeResult::with_midfielder_state(
                MidfielderState::Running,
            ));
        }

        None
    }

    fn velocity(&self, _ctx: &StateProcessingContext) -> Option<Vector3<f32>> {
        // Standing = completely still. No separation, no drift.
        // Midfielders transition out of Standing within 8 ticks anyway.
        Some(Vector3::zeros())
    }

    fn process_conditions(&self, ctx: ConditionContext) {
        // Standing is recovery - minimal movement
        MidfielderCondition::new(ActivityIntensity::Recovery).process(ctx);
    }
}

impl MidfielderStandingState {
    /// Should this midfielder drop to a walk rather than break into a run?
    ///
    /// Mirrors the defender's walking gate: the ball is far away or we're
    /// already where we want to be, nothing is threatening, and either
    /// we're tired or there is simply nothing to do. Called only from the
    /// possession branch, so "our team has the ball" is already true.
    ///
    /// Real midfielders spend a large share of a match walking — this is
    /// where that comes from, and it feeds the fatigue model's `Low`
    /// intensity band, which midfielders previously never touched.
    fn should_walk(&self, ctx: &StateProcessingContext) -> bool {
        const WALK_BALL_DISTANCE: f32 = 250.0;
        const IN_POSITION_DISTANCE: f32 = 40.0;
        const THREAT_RADIUS: f32 = 100.0;

        let no_immediate_threat = ctx
            .players()
            .opponents()
            .nearby(THREAT_RADIUS)
            .next()
            .is_none();
        if !no_immediate_threat {
            return false;
        }

        let ball_far = ctx.ball().distance() > WALK_BALL_DISTANCE;
        let in_position = ctx.player().distance_from_start_position() < IN_POSITION_DISTANCE;

        // Tired players take any excuse to walk; fresh ones only walk when
        // there is genuinely nothing to run toward.
        if ctx.player().is_tired() {
            ball_far || in_position
        } else {
            ball_far && in_position
        }
    }

    /// Is this a "keep the ball moving" moment rather than a "play
    /// forward" one?
    ///
    /// True for the settled deep-lying picture: we're not yet in the
    /// final third, nobody is closing us down, and the manager wants
    /// possession held. That is when a midfielder recycles — sideways and
    /// backwards into space — instead of forcing the ball forward.
    fn should_recycle_possession(&self, ctx: &StateProcessingContext) -> bool {
        // 20u is 2.5 m. A man closing from six metres is pressure a
        // midfielder reacts to, and this test exists precisely to say
        // "no time to pick out space" — so it has to see him.
        const PRESSURE_RADIUS: f32 = 48.0;
        const FINAL_THIRD_PROGRESS: f32 = 0.66;

        // Under pressure there is no time to pick out space — the passing
        // state's evaluation is the right tool.
        if ctx.players().opponents().exists(PRESSURE_RADIUS) {
            return false;
        }

        // In the final third the job is to create, not to recycle.
        let Some(side) = ctx.player.side else {
            return false;
        };
        let field_width = ctx.context.field_size.width as f32;
        if side.attacking_progress_x(ctx.player.position.x, field_width) > FINAL_THIRD_PROGRESS {
            return false;
        }

        // Recycling is what a possession side does; a direct one plays
        // forward. `build_up_patience` carries both the manager's
        // instruction and the team's tactical shape.
        ctx.team().build_up_patience() > 0.5
    }

    /// Determines if the midfielder has passing options.
    /// Is there anyone at all to give it to?
    ///
    /// A cheap pre-filter, not a decision — `Passing` runs the real
    /// evaluation and picks the target. It was set at 30u, which is
    /// **3.75 m**: a midfielder whose nearest team-mate was four metres
    /// away counted as having nobody to pass to. In open play that is
    /// most of the time, and the caller's answer to "no options" was to
    /// stand still (see the `has_ball` branch above), so this constant is
    /// what made the freeze routine rather than rare.
    ///
    /// 200u = 25 m — a normal midfield pass. Below that he is genuinely
    /// isolated and carries it instead.
    fn has_passing_options(&self, ctx: &StateProcessingContext) -> bool {
        const PASSING_DISTANCE_THRESHOLD: f32 = 200.0;
        ctx.players().teammates().exists(PASSING_DISTANCE_THRESHOLD)
    }

    /// Checks if an opponent player is nearby within the pressing threshold.
    #[allow(dead_code)]
    fn is_opponent_nearby(&self, ctx: &StateProcessingContext) -> bool {
        ctx.players()
            .opponents()
            .exists(PRESSING_DISTANCE_THRESHOLD)
    }

    /// Determines if the midfielder should support an attacking play.
    fn should_support_attack(&self, ctx: &StateProcessingContext) -> bool {
        // For simplicity, assume the midfielder supports the attack if the ball is in the attacking third
        let field_length = ctx.context.field_size.width as f32;
        let attacking_third_start = if ctx.player.side == Some(PlayerSide::Left) {
            field_length * (2.0 / 3.0)
        } else {
            field_length / 3.0
        };

        let ball_position_x = ctx.tick_context.positions.ball.position.x;

        if ctx.player.side == Some(PlayerSide::Left) {
            ball_position_x > attacking_third_start
        } else {
            ball_position_x < attacking_third_start
        }
    }
}
