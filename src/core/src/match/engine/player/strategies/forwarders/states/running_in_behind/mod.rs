use crate::r#match::forwarders::states::ForwardState;
use crate::r#match::forwarders::states::common::{
    ActivityIntensity, ForwardCondition, ForwardRead, RunLine,
};
#[cfg(feature = "match-logs")]
use crate::r#match::player::diagnostics::forward_census::ForwardCensus;
use crate::r#match::common_states::StationKeeping;
use crate::r#match::player::strategies::common::players::ops::marker_evasion::MarkerEvasion;
use crate::r#match::{
    ConditionContext, MatchContext, PlayerSide, StateChangeResult, StateProcessingContext,
    StateProcessingHandler, SteeringBehavior,
};
use nalgebra::Vector3;

impl ForwardRunningInBehindState {
    /// How far past the last defender a forward with no sense of timing
    /// drifts while he waits, in game units. 20u = 2.5 m — comfortably
    /// offside, which is the point: the real four-to-six flags a match are
    /// mistimed runs, and a rig where nobody ever mistimes one produces
    /// none at all.
    const STRAY: f32 = 20.0;
    /// …and how far BEHIND it a forward who does time it holds. 8u = 1 m.
    /// Level with the last man is both offside on any tolerance and the
    /// most dangerous standing position in football; a yard behind him is
    /// where a runner actually waits.
    const LINE_MARGIN: f32 = 8.0;
    /// How far ahead on the run he aims, 7.5 m.
    const RUN_LOOKAHEAD: f32 = 60.0;
    /// Within this (0.25 m) of his station he is on it, and keeps it as the
    /// line moves; further off he arrives at it, easing in over `ARRIVE`.
    const ON_STATION: f32 = 2.0;
    const ARRIVE: f32 = 24.0;

    /// The run the plan gave him: at the goal, across his marker by
    /// `MarkerEvasion`. Until the ball is played to him — he is its intended
    /// receiver — he keeps station on the hold line, moving as the line
    /// moves, so a runner already there walks with it; then he goes at full
    /// effort.
    fn given_run(ctx: &StateProcessingContext) -> Vector3<f32> {
        let here = ctx.player.position;
        let to_goal = (ctx.ball().direction_to_opponent_goal() - here).normalize();
        let aim = MarkerEvasion::evade(ctx, here + to_goal * Self::RUN_LOOKAHEAD);
        let played_to_him = ctx.tick_context.ball.pass_target == Some(ctx.player.id);
        if MatchContext::forward_run_pace_off() {
            let aim = if played_to_him {
                aim
            } else {
                Self::held(ctx, aim)
            };
            let acceleration = ctx.player.skills.physical.acceleration / 20.0;
            return SteeringBehavior::Seek { target: aim }
                .calculate(ctx.player)
                .velocity
                * (1.0 + acceleration * 0.15)
                * MarkerEvasion::burst(ctx);
        }
        if played_to_him {
            return SteeringBehavior::Seek { target: aim }
                .calculate(ctx.player)
                .velocity;
        }
        let Some(station) = Self::station(ctx) else {
            return SteeringBehavior::Seek { target: aim }
                .calculate(ctx.player)
                .velocity;
        };
        #[cfg(feature = "match-logs")]
        ForwardCensus::note_run_held(ctx);
        let gap = station - here;
        let line = Self::line_velocity(ctx);
        if gap.xy().norm() > Self::ON_STATION {
            // The line goes on moving while he closes on his station: off
            // it, he runs with the line as well as at the point.
            return line
                + SteeringBehavior::Arrive {
                    target: station,
                    slowing_distance: Self::ARRIVE,
                }
                .calculate(ctx.player)
                .velocity;
        }
        StationKeeping::hold(line, gap)
    }

    /// Where a waiting runner stands: on his hold line, in the lane of his
    /// place, across his marker by `MarkerEvasion`.
    pub(crate) fn station(ctx: &StateProcessingContext) -> Option<Vector3<f32>> {
        let hold = Self::hold_x(ctx)?;
        let lane = ctx.team().my_anchor().y;
        let point = MarkerEvasion::evade(ctx, Vector3::new(hold, lane, 0.0));
        Some(Vector3::new(hold, point.y, 0.0))
    }

    /// How fast their last line moves up or down the pitch: the run of the
    /// opponent who makes it, and none when the halfway line is the line.
    fn line_velocity(ctx: &StateProcessingContext) -> Vector3<f32> {
        let Some(line) = RunLine::of(ctx) else {
            return Vector3::zeros();
        };
        ctx.players()
            .opponents()
            .all()
            .filter(|o| (o.position.x - line).abs() < 1.0)
            .map(|o| ctx.tick_context.positions.players.velocity(o.id).x)
            .next()
            .map_or_else(Vector3::zeros, |x| Vector3::new(x, 0.0, 0.0))
    }

    /// **HOLD THE LINE UNTIL THE BALL IS PLAYED.** The run's aim, kept
    /// inside the line the referee reads.
    ///
    /// The run was always meant to stay level with the last defender and go
    /// past him when the ball is played, and for a long time none of the
    /// code did it: a forward simply ran beyond the last man and stayed
    /// there, and every ball played to him was offside by definition —
    /// measured at **25.4 offsides a match against a real 4-6**, which no
    /// amount of restraint on the PASSER can fix, because he still has to
    /// pass to somebody.
    ///
    /// Reading the line through [`RunLine`] is the point: a forward holding
    /// a line one unit away from the one the referee uses is still offside,
    /// and would look like he was holding nothing. In his own half there is
    /// no line to hold.
    pub(crate) fn held(ctx: &StateProcessingContext, mut aim: Vector3<f32>) -> Vector3<f32> {
        let (Some(side), Some(hold)) = (ctx.player.side, Self::hold_x(ctx)) else {
            return aim;
        };
        let beyond = match side {
            PlayerSide::Left => aim.x > hold,
            PlayerSide::Right => aim.x < hold,
        };
        if beyond {
            aim.x = hold;
            #[cfg(feature = "match-logs")]
            ForwardCensus::note_run_held(ctx);
        }
        aim
    }

    /// The line he holds: inside the referee's line by a stride, or past it
    /// by as much as his timing lets him stray.
    fn hold_x(ctx: &StateProcessingContext) -> Option<f32> {
        let side = ctx.player.side?;
        let line = RunLine::of(ctx)?;
        // …but he is not a laser. Timing a run is `off_the_ball` and
        // `anticipation`, and a forward who holds the last line PERFECTLY is
        // both unrealistic and the most dangerous position on the pitch:
        // clamped exactly, offsides fell to 9 a match and goals rose
        // 5.1 → 6.0, because every flagged run became a clean one instead.
        // A good runner sits on the shoulder; a poor one strays past it and
        // gets flagged, which is where the real four-to-six a match come
        // from.
        let timing = ((ctx.player.skills.mental.off_the_ball
            + ctx.player.skills.mental.anticipation)
            / 40.0)
            .clamp(0.0, 1.0);
        let stray = Self::STRAY * (1.0 - timing);
        // Held a stride BEHIND the line rather than level with it, for the
        // same reason: a man permanently on the shoulder is played in every
        // time the ball is worked wide.
        Some(line + side.forward_dir_x() * (stray - Self::LINE_MARGIN))
    }
}

#[derive(Default, Clone)]
pub struct ForwardRunningInBehindState {}

impl StateProcessingHandler for ForwardRunningInBehindState {
    /// The run is the plan's: it ends when the read says it does — the ball
    /// is his, his side has lost it, or the run is somebody else's.
    fn process(&self, ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        ForwardRead::hold(ctx, ForwardState::RunningInBehind)
    }

    fn velocity(&self, ctx: &StateProcessingContext) -> Option<Vector3<f32>> {
        Some(Self::given_run(ctx))
    }

    fn process_conditions(&self, ctx: ConditionContext) {
        // Running in behind is very high intensity - explosive sprinting
        ForwardCondition::new(ActivityIntensity::VeryHigh).process(ctx);
    }
}
