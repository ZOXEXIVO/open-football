use crate::r#match::defenders::states::DefenderState;
use crate::r#match::defenders::states::shared::{
    ActivityIntensity, DefenderCondition, DefenderRead,
};
use crate::r#match::engine::ball::ball::{AerialReach, PlayerReach};
use crate::r#match::events::Event;
#[cfg(feature = "match-logs")]
use crate::r#match::player::diagnostics::defender_census::{
    DefenderCensus, HeaderBooking, HeaderRule,
};
use crate::r#match::player::events::{PlayerEvent, ShootingEventContext};
use crate::r#match::player::strategies::players::{ShotType, WonHeader};
use crate::r#match::{
    ConditionContext, MatchContext, StateChangeResult, StateProcessingContext,
    StateProcessingHandler, SteeringBehavior,
};
use nalgebra::Vector3;

/// **Up for a high ball a contest has given him.** He goes to meet it and
/// takes off as it comes down to him. The ball comes off his head through the
/// delivery that brings it, by the one reach rule (`PlayerReach`), booked as
/// the clearance the contest decided; nothing here rolls for it again. A
/// header he has won near the opponent's goal is his attempt on it.
#[derive(Default, Clone)]
pub struct DefenderHeadingState {}

impl DefenderHeadingState {
    /// Within this distance of the opponent's goal a header he has won is
    /// an attempt on goal (a defender up for a set piece).
    const ATTACKING_RANGE: f32 = 90.0;
    /// Where he meets a header he has won near their goal: at heading height
    /// (1.5 m), within a contact radius of 0.75 m.
    const CONTACT_HEIGHT: f32 = 1.5;
    const CONTACT_RADIUS: f32 = 6.0;
    /// He takes off as the ball comes within 2 m of him on its way down.
    const TAKE_OFF: f32 = 16.0;
    /// He attacks a header on the jump even when it comes at his head: a
    /// hop of 0.2 m.
    const HOP: f32 = 0.2;
}

impl StateProcessingHandler for DefenderHeadingState {
    fn process(&self, ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        let me = ctx.player.id;
        let ball = &ctx.tick_context.ball;
        if !ctx.player.aerial_award || !ball.awaits_header_from(me) {
            return Some(DefenderRead::change(ctx));
        }
        let position = ctx.tick_context.positions.ball.position;
        if !MatchContext::cross_head_arrival_off()
            && ball.aerial_contest_winner == Some(me)
            && PlayerReach::can_head(position, ctx.player)
        {
            return Some(self.strike(ctx));
        }
        if ball.aerial_contest_winner == Some(me)
            && ctx.ball().distance_to_opponent_goal() < Self::ATTACKING_RANGE
            && ctx.ball().distance() <= Self::CONTACT_RADIUS
            && position.z >= Self::CONTACT_HEIGHT
        {
            #[cfg(feature = "match-logs")]
            DefenderCensus::note_header(
                HeaderRule::HeadingShot,
                ctx.player,
                ctx.player.is_airborne(),
                HeaderBooking::Shot,
            );
            return Some(StateChangeResult::with_defender_state_and_event(
                DefenderRead::released(ctx),
                Event::PlayerEvent(PlayerEvent::Shoot(
                    ShootingEventContext::builder()
                        .with_player_id(me)
                        .with_target(ctx.player().shooting_direction())
                        .with_reason("DEF_HEADER_ON_GOAL")
                        .with_shot_type(ShotType::Header)
                        .build(ctx),
                )),
            ));
        }
        let falling = ctx.tick_context.positions.ball.velocity.z <= 0.0;
        if falling && !ctx.player.is_airborne() && ctx.ball().distance() <= Self::TAKE_OFF {
            let apex = AerialReach::header_leap_for(position.z, ctx.player.skills.physical.jumping)
                .max(Self::HOP);
            return Some(StateChangeResult::with_defender_state_and_event(
                DefenderState::Heading,
                Event::PlayerEvent(PlayerEvent::Leap(me, apex)),
            ));
        }
        None
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
        DefenderCondition::new(ActivityIntensity::VeryHigh).process(ctx);
    }
}

impl DefenderHeadingState {
    /// The header the contest left him, by the rule every role plays a won
    /// header by ([`WonHeader`]): at goal from a chance, glanced away
    /// otherwise.
    fn strike(&self, ctx: &StateProcessingContext) -> StateChangeResult {
        if WonHeader::at_goal(ctx) {
            #[cfg(feature = "match-logs")]
            DefenderCensus::note_header(
                HeaderRule::HeadingShot,
                ctx.player,
                ctx.player.is_airborne(),
                HeaderBooking::Shot,
            );
            return StateChangeResult::with_defender_state_and_event(
                DefenderRead::released(ctx),
                Event::PlayerEvent(PlayerEvent::Shoot(
                    ShootingEventContext::builder()
                        .with_player_id(ctx.player.id)
                        .with_target(ctx.player().shooting_direction())
                        .with_reason("DEF_HEADER_ON_GOAL")
                        .with_shot_type(ShotType::Header)
                        .build(ctx),
                )),
            );
        }
        #[cfg(feature = "match-logs")]
        DefenderCensus::note_header(
            HeaderRule::HeadingShot,
            ctx.player,
            ctx.player.is_airborne(),
            if WonHeader::in_own_third(ctx) {
                HeaderBooking::Clearance
            } else {
                HeaderBooking::Nothing
            },
        );
        StateChangeResult::with_defender_state_and_event(
            DefenderRead::released(ctx),
            WonHeader::glance(ctx),
        )
    }
}
