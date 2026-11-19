use crate::r#match::engine::ball::ball::{AerialReach, PlayerReach};
use crate::r#match::events::Event;
use crate::r#match::forwarders::states::ForwardState;
use crate::r#match::forwarders::states::common::{
    ActivityIntensity, ForwardCondition, ForwardRead,
};
#[cfg(feature = "match-logs")]
use crate::r#match::player::diagnostics::defender_census::HeaderBooking;
#[cfg(feature = "match-logs")]
use crate::r#match::player::diagnostics::forward_census::{ForwardCensus, FwdHeadingExit};
use crate::r#match::player::events::{PlayerEvent, ShootingEventContext};
#[cfg(feature = "match-logs")]
use crate::r#match::player::strategies::common::players::ops::forward_shot_decision::mid_run_diag::CROSS_HEADER_ON_GOAL;
use crate::r#match::player::strategies::players::{ShotType, WonHeader};
use crate::r#match::{
    ConditionContext, StateChangeResult, StateProcessingContext, StateProcessingHandler,
    SteeringBehavior,
};
use nalgebra::Vector3;
#[cfg(feature = "match-logs")]
use std::sync::atomic::Ordering as AtomicOrdering;

/// **Up for a high ball a contest has given him.** He goes to meet it, takes
/// off as it comes down to him, and heads it once it is in reach by the one
/// reach rule: at goal from inside headed range on the odds of who he is and
/// where it came from, glanced away otherwise. A ball the contest carries an
/// outcome for comes off his head through the delivery that brings it
/// (`Ball::tick_aerial_delivery`). Nothing here rolls for the ball again.
#[derive(Default, Clone)]
pub struct ForwardHeadingState {}

impl StateProcessingHandler for ForwardHeadingState {
    fn process(&self, ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        let me = ctx.player.id;
        let ball = &ctx.tick_context.ball;
        if ctx.ball().is_owned() || !ctx.player.aerial_award || !ball.awaits_header_from(me) {
            #[cfg(feature = "match-logs")]
            ForwardCensus::note_heading_exit(
                ctx,
                if ball.aerial_contest_winner.is_some_and(|id| id != me) {
                    FwdHeadingExit::OtherAward
                } else {
                    FwdHeadingExit::AwardDropped
                },
            );
            return Some(ForwardRead::change(ctx));
        }
        let position = ctx.tick_context.positions.ball.position;
        let falling = ctx.tick_context.positions.ball.velocity.z <= 0.0;
        // A header he plays himself is struck within his head's reach, so he
        // takes off under the ball; one the delivery plays off his head only
        // needs him up within its own reach.
        let his_to_strike = ball.aerial_contest_winner == Some(me);
        let take_off = if his_to_strike {
            PlayerReach::HEAD_REACH
        } else {
            Self::TAKE_OFF
        };
        let across = (position - ctx.player.position).xy().norm();
        if falling && !ctx.player.is_airborne() && across <= take_off {
            let apex = AerialReach::header_leap_for(position.z, ctx.player.skills.physical.jumping)
                .max(Self::HOP);
            return Some(StateChangeResult::with_forward_state_and_event(
                ForwardState::Heading,
                Event::PlayerEvent(PlayerEvent::Leap(me, apex)),
            ));
        }
        if !his_to_strike || !PlayerReach::can_head(position, ctx.player) {
            return None;
        }
        Some(self.strike(ctx))
    }

    fn velocity(&self, ctx: &StateProcessingContext) -> Option<Vector3<f32>> {
        Some(
            SteeringBehavior::Arrive {
                target: WonHeader::meeting_point(ctx),
                slowing_distance: 3.0,
            }
            .calculate(ctx.player)
            .velocity,
        )
    }

    fn process_conditions(&self, ctx: ConditionContext) {
        // Heading is very high intensity - explosive jumping action
        ForwardCondition::new(ActivityIntensity::VeryHigh).process(ctx);
    }
}

impl ForwardHeadingState {
    /// He takes off as a ball the delivery plays off his head comes within
    /// 2 m of him on its way down.
    const TAKE_OFF: f32 = 16.0;
    /// He attacks a header on the jump even when it comes at his head: a
    /// hop of 0.2 m.
    const HOP: f32 = 0.2;
    /// The header itself: at goal from inside headed range, on the odds of
    /// his heading and jumping — set higher from a corner, which is a jump
    /// at a ball aimed at his head — and glanced away otherwise.
    fn strike(&self, ctx: &StateProcessingContext) -> StateChangeResult {
        let next = ForwardRead::released(ctx);
        if WonHeader::at_goal(ctx) {
            #[cfg(feature = "match-logs")]
            {
                CROSS_HEADER_ON_GOAL.fetch_add(1, AtomicOrdering::Relaxed);
                ForwardCensus::note_heading_exit(ctx, FwdHeadingExit::AwardShot);
                ForwardCensus::note_heading_header(
                    ctx,
                    ForwardCensus::award_rule(ctx),
                    HeaderBooking::Shot,
                );
            }
            return StateChangeResult::with_forward_state_and_event(
                next,
                Event::PlayerEvent(PlayerEvent::Shoot(
                    ShootingEventContext::builder()
                        .with_player_id(ctx.player.id)
                        .with_target(ctx.player().shooting_direction())
                        .with_reason("FWD_HEADING_ON_GOAL")
                        .with_shot_type(ShotType::Header)
                        .build(ctx),
                )),
            );
        }
        #[cfg(feature = "match-logs")]
        {
            ForwardCensus::note_heading_exit(ctx, FwdHeadingExit::AwardGlance);
            ForwardCensus::note_heading_header(
                ctx,
                ForwardCensus::award_rule(ctx),
                Self::glance_booking(ctx),
            );
        }
        StateChangeResult::with_forward_state_and_event(next, WonHeader::glance(ctx))
    }

    #[cfg(feature = "match-logs")]
    fn glance_booking(ctx: &StateProcessingContext) -> HeaderBooking {
        if WonHeader::in_own_third(ctx) {
            HeaderBooking::Clearance
        } else {
            HeaderBooking::Nothing
        }
    }
}
