//! **The wide channel** — what the man on the touchline actually does
//! once the plan has put him there.
//!
//! [`WidePlan`](crate::r#match::WidePlan) says *who* holds each flank and
//! who runs beyond him. This says what that looks like from tick to tick,
//! and it is deliberately one piece of code shared by the midfielder's
//! and the forward's version of the job: a 4-4-2's wide midfielder and a
//! 4-3-3's wide forward are the same footballer with a different label,
//! and the engine has a long history of the two drifting apart because
//! each role kept its own copy.
//!
//! # The three things a wide player does with the ball elsewhere
//!
//! * **Hold.** The ball is on the other side, or still in our own half.
//!   He stands on the paint and waits, which is not idleness — it is the
//!   job. Every metre he stands from the nearest defender is a metre
//!   that defender has to leave somewhere else, and that is what opens
//!   the middle for everybody else.
//! * **Advance.** The ball is coming to his side. He pushes up the line
//!   ahead of the carrier so there is a forward option on the flank
//!   rather than only the square one infield.
//! * **Byline.** The ball is in the final third on his side and there is
//!   grass behind the full-back. He runs it. This is the run that ends
//!   in a cutback — the highest-value pass in football and, before the
//!   width plan existed, one this engine produced **five times in two
//!   hundred matches**.
//!
//! The three are ordered by how much of the pitch they ask him to cover,
//! and they are separated by where the BALL is rather than by a timer, so
//! a wide player cannot oscillate between two of them while the ball
//! stays put.

use crate::r#match::engine::ball::ball::OffsideLine;
use crate::r#match::player::strategies::common::states::ActivityIntensity;
use crate::r#match::{Flank, StateProcessingContext, WidePlan};
use nalgebra::Vector3;

/// What the wide channel wants from this player right now.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WideIntent {
    /// Stand on the touchline at the block's depth and stretch the
    /// defence.
    Hold,
    /// Push up the line to offer ahead of the ball.
    Advance,
    /// Run in behind the full-back, toward the byline.
    Byline,
}

impl WideIntent {
    /// How hard he is working. Holding width is a jog — the value is in
    /// the position, not the effort — and the run to the byline is the
    /// one moment a winger genuinely sprints.
    pub fn intensity(self) -> ActivityIntensity {
        match self {
            WideIntent::Hold => ActivityIntensity::Moderate,
            WideIntent::Advance => ActivityIntensity::High,
            WideIntent::Byline => ActivityIntensity::VeryHigh,
        }
    }
}

/// The wide channel, resolved for one player on one tick.
pub struct WideChannel;

impl WideChannel {
    /// Attacking progress at which the flank becomes a place to attack
    /// rather than a place to stand. 0.55 is a little inside the
    /// opponent half — a winger starts his run when the ball is being
    /// carried at the defence, not when it reaches the corner flag.
    const RUN_PROGRESS: f32 = 0.55;

    /// How far ahead of the ball he may be while merely advancing, in
    /// game units. 90u ≈ 11 m: he offers in front of the carrier, he
    /// does not run away from him.
    const ADVANCE_LEAD: f32 = 90.0;

    /// How near the goal line the byline target sits (~5.6 m), and how
    /// far off the goal's centre line (~20.6 m — just outside the corner
    /// of the penalty area, which is where a winger who has beaten his
    /// man actually arrives).
    const BYLINE_DEPTH: f32 = 45.0;
    const BYLINE_LATERAL: f32 = 165.0;

    /// How wide of his own touchline the ball still counts as being "on
    /// his side" for the purpose of starting a run (~26 m). Wide enough
    /// to include the half-space, because the ball played from the
    /// inside-left channel is exactly what a left winger runs onto.
    const SAME_SIDE_BAND: f32 = 210.0;

    /// The flank this player is working, whether he is holding it or
    /// running beyond the man who is. `None` for everybody else.
    pub fn assignment(ctx: &StateProcessingContext) -> Option<Flank> {
        let team = ctx.team();
        let plan = team.wide_plan();
        plan.flank_of(ctx.player.id).or_else(|| {
            plan.is_overlap_runner(ctx.player.id)
                .then_some(plan.ball_flank)
        })
    }

    /// Does this player still have a wide job? False the moment his side
    /// loses the ball, the plan gives the flank to somebody else, or the
    /// ball arrives at his own feet — all of which are the same
    /// condition read from three directions, so entry and exit can never
    /// both be true and the state cannot two-cycle.
    pub fn still_mine(ctx: &StateProcessingContext) -> bool {
        !ctx.player.has_ball(ctx)
            && ctx.team().is_control_ball()
            && Self::assignment(ctx).is_some()
            // A box slot outranks the touchline: the far-side holder
            // arriving at the back post is the same man one phase later,
            // and `my_anchor` already resolves it that way.
            && ctx.team().my_box_slot().is_none()
    }

    /// What the channel wants from him this tick.
    pub fn intent(ctx: &StateProcessingContext) -> WideIntent {
        let Some(flank) = Self::assignment(ctx) else {
            return WideIntent::Hold;
        };
        let Some(side) = ctx.player.side else {
            return WideIntent::Hold;
        };
        let field_width = ctx.context.field_size.width as f32;
        let field_height = ctx.context.field_size.height as f32;
        let ball = ctx.tick_context.positions.ball.position;
        let ball_progress = side.attacking_progress_x(ball.x, field_width);

        // Is the ball on my side of the pitch at all? Measured against
        // MY touchline rather than the halfway line, so the half-space
        // next to me counts — a ball in the inside channel is a ball I
        // run off.
        let my_touchline = flank.touchline_y(field_height, 0.0);
        let on_my_side = (ball.y - my_touchline).abs() < Self::SAME_SIDE_BAND;
        if !on_my_side {
            return WideIntent::Hold;
        }

        if ball_progress < Self::RUN_PROGRESS {
            return WideIntent::Advance;
        }

        // A run in behind that starts offside is a run that ends in a
        // flag. He holds his position on the shoulder instead and lets
        // the ball come to feet — which is also what a real winger does
        // when the line has stepped up.
        // Every opponent, keeper included — he is normally the last man,
        // and `second_last` is defined over the whole defending side.
        // Same call shape as the pass evaluator's, deliberately: an
        // extra phantom defender on the goal line would shift the line
        // one man upfield and read an offside winger as onside.
        let line =
            OffsideLine::second_last(ctx.players().opponents().all().map(|o| o.position.x), side);
        let onside = line.is_none_or(|line_x| {
            !OffsideLine::is_beyond(side, ctx.player.position.x, ball.x, line_x)
        });
        if onside {
            WideIntent::Byline
        } else {
            WideIntent::Advance
        }
    }

    /// One tick of somebody actually doing the job — logged from the
    /// three states rather than from the plan, because the plan naming a
    /// man and the man reaching the touchline are different claims and
    /// only the second one is football.
    #[cfg(feature = "match-logs")]
    pub fn note_tick(ctx: &StateProcessingContext) {
        use crate::mid_run_diag::WideDiag;
        let wide =
            crate::r#match::player::strategies::common::passing::CrossModel::is_in_wide_position(
                ctx,
            );
        let overlap = ctx.team().is_overlap_runner();
        let at_byline = ctx
            .player
            .side
            .map(|s| {
                let goal_x = ctx.player().opponent_goal_position().x;
                s.forward_delta(ctx.player.position.x, goal_x) < 96.0
            })
            .unwrap_or(false);
        WideDiag::note_width_tick(wide, overlap, at_byline);
    }

    #[cfg(not(feature = "match-logs"))]
    #[inline(always)]
    pub fn note_tick(_ctx: &StateProcessingContext) {}

    /// Where he is going, for the intent he has been given.
    ///
    /// `Hold` and `Advance` are both expressed against the anchor the
    /// team plan already computed for him ([`my_anchor`]), so the wide
    /// channel never contradicts the block — it moves him along it.
    ///
    /// [`my_anchor`]: crate::r#match::player::strategies::common::TeamOperationsImpl::my_anchor
    pub fn target(ctx: &StateProcessingContext, intent: WideIntent) -> Vector3<f32> {
        let anchor = ctx.team().my_anchor();
        let Some(flank) = Self::assignment(ctx) else {
            return anchor;
        };
        let Some(side) = ctx.player.side else {
            return anchor;
        };
        let field_width = ctx.context.field_size.width as f32;
        let field_height = ctx.context.field_size.height as f32;
        let forward = side.forward_dir_x();
        let ball = ctx.tick_context.positions.ball.position;

        match intent {
            WideIntent::Hold => anchor,
            WideIntent::Advance => {
                // Level with the ball plus a stride, on his own line.
                // Taking the anchor's `y` keeps `team_width_target`
                // honest: a narrow side advances up a narrow channel.
                let x = (ball.x + forward * Self::ADVANCE_LEAD).clamp(14.0, field_width - 14.0);
                let ahead_of_anchor = side.forward_delta(anchor.x, x) > 0.0;
                Vector3::new(if ahead_of_anchor { x } else { anchor.x }, anchor.y, 0.0)
            }
            WideIntent::Byline => {
                let goal = ctx.player().opponent_goal_position();
                Vector3::new(
                    (goal.x - forward * Self::BYLINE_DEPTH).clamp(14.0, field_width - 14.0),
                    (goal.y + flank.sign() * Self::BYLINE_LATERAL).clamp(
                        WidePlan::TOUCHLINE_INSET,
                        field_height - WidePlan::TOUCHLINE_INSET,
                    ),
                    0.0,
                )
            }
        }
    }
}
