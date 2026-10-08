//! **The man standing over a free kick, a penalty or a corner.**
//!
//! Handed the ball, he was an ordinary carrier under his ordinary state
//! machine, and every free kick he was not going to shoot was walked away
//! from the mark: traced at a level-14 free kick 22 m out, the taker
//! picked the ball up and dribbled it sixteen metres into the area. A
//! corner fared no better: the routine chosen for it never reached the
//! delivery, a full-back taker had no corner in his machine at all, and
//! the rest crossed for whoever was the biggest centre-back. A set piece
//! is in play once it is kicked and clearly moves, and the taker may not
//! touch it again until somebody else has. There is no third thing to do
//! with it: he shoots, he delivers it, or he plays it to a team-mate.
//!
//! An override for the reason [`KickoffDelivery`](super::KickoffDelivery)
//! is one: four state machines would each need the concept, and it has to
//! beat everything else they do.

use crate::r#match::PassOriginRestart;
use crate::r#match::engine::officiating::restart_shape::RestartShape;
use crate::r#match::events::Event;
use crate::r#match::player::events::PlayerEvent;
use crate::r#match::player::events::models::{PassingEventContext, ShootingEventContext};
use crate::r#match::player::strategies::passing::{CrossDecision, CrossModel};
use crate::r#match::player::strategies::players::ops::forward_shot_decision::FreeKickResolver;
use crate::r#match::{MatchPlayerLite, StateProcessingContext};

pub struct SetPieceKick;

impl SetPieceKick {
    /// How long he stands over it before he plays it, in engine ticks.
    /// 60 = 0.6 s: the look up between the whistle and the kick, with the
    /// wall already in place.
    const SCAN_TICKS: u64 = 60;
    /// Nobody to give it to by now, and he plays it to the nearest man.
    /// 3 s.
    const PATIENCE_TICKS: u64 = 300;
    /// Team-mates within this of goal (~21 m) count as in the area for a
    /// delivery, and it takes this many to be worth one.
    const BOX_RANGE: f32 = 170.0;
    const BOX_BODIES: usize = 2;

    /// True while this player has a set piece at his feet that he has not
    /// yet kicked.
    #[inline]
    pub fn taking(ctx: &StateProcessingContext) -> bool {
        ctx.tick_context.ball.set_piece_kicker == Some(ctx.player.id)
            && ctx.tick_context.ball.current_owner == Some(ctx.player.id)
    }

    /// The kick, once he has looked up. `None` while he is still standing
    /// over it.
    pub fn deliver(ctx: &StateProcessingContext) -> Option<Event> {
        // Timed from the hand-over rather than off `ownership_duration`,
        // which does not run while the claim window the hand-over opens is.
        let held = ctx
            .current_tick()
            .saturating_sub(ctx.tick_context.ball.set_piece_handed_tick);
        if held < Self::SCAN_TICKS {
            return None;
        }
        if let Some(reason) = Self::shot(ctx) {
            return Some(Event::PlayerEvent(PlayerEvent::Shoot(
                ShootingEventContext::builder()
                    .with_player_id(ctx.player.id)
                    .with_target(ctx.player().shooting_direction())
                    .with_reason(reason)
                    .build(ctx),
            )));
        }
        let corner = ctx.tick_context.ball.pass_origin_restart == PassOriginRestart::Corner;
        if let Some(delivery) = ctx
            .tick_context
            .ball
            .corner_routine
            .filter(|_| corner)
            .and_then(|routine| CrossModel::corner(ctx, routine))
        {
            #[cfg(feature = "match-logs")]
            Self::note_corner_delivery(ctx, delivery.target_id);
            return Some(Event::PlayerEvent(PlayerEvent::PassTo(
                PassingEventContext::builder()
                    .with_from_player_id(ctx.player.id)
                    .with_to_player_id(delivery.target_id)
                    .with_cross_type(delivery.cross_type)
                    .with_target_point(delivery.aim_point)
                    .with_reason("CORNER")
                    .build(ctx),
            )));
        }
        if let Some(delivery) = Self::free_kick_delivery(ctx) {
            return Some(Event::PlayerEvent(PlayerEvent::PassTo(
                PassingEventContext::builder()
                    .with_from_player_id(ctx.player.id)
                    .with_to_player_id(delivery.target_id)
                    .with_cross_type(delivery.cross_type)
                    .with_target_point(delivery.aim_point)
                    .with_reason("FREE_KICK_DELIVERY")
                    .build(ctx),
            )));
        }
        // A short corner goes to the man who came for it; anything else to
        // whoever the passing model finds.
        let target = if corner {
            Self::nearest(ctx)
        } else {
            ctx.player()
                .passing()
                .find_best_pass_option()
                .map(|(mate, _)| mate)
                .or_else(|| {
                    (held >= Self::PATIENCE_TICKS)
                        .then(|| Self::nearest(ctx))
                        .flatten()
                })
        }?;
        Some(Event::PlayerEvent(PlayerEvent::PassTo(
            PassingEventContext::builder()
                .with_from_player_id(ctx.player.id)
                .with_to_player_id(target.id)
                .with_reason("SET_PIECE")
                .build(ctx),
        )))
    }

    /// A free kick within range of the area that he is not shooting goes
    /// into it, at the man the cross model picks out, once there are two
    /// to aim at.
    fn free_kick_delivery(ctx: &StateProcessingContext) -> Option<CrossDecision> {
        let free_kick = matches!(
            ctx.tick_context.ball.pass_origin_restart,
            PassOriginRestart::DirectFreeKick | PassOriginRestart::IndirectFreeKick
        );
        if !free_kick || ctx.ball().distance_to_opponent_goal() > RestartShape::DELIVERY_RANGE {
            return None;
        }
        let goal = ctx.player().opponent_goal_position();
        let bodies = ctx
            .players()
            .teammates()
            .nearby_at(goal, Self::BOX_RANGE)
            .filter(|t| t.id != ctx.player.id)
            .count();
        if bodies < Self::BOX_BODIES {
            return None;
        }
        CrossModel::pick(ctx)
    }

    /// Goes for goal, and why: a penalty always, a direct free kick when
    /// he chooses to, an indirect one never.
    fn shot(ctx: &StateProcessingContext) -> Option<&'static str> {
        match ctx.tick_context.ball.pass_origin_restart {
            PassOriginRestart::Penalty => Some("PENALTY_KICK"),
            PassOriginRestart::DirectFreeKick if FreeKickResolver::shoots(ctx) => {
                Some("DIRECT_FREE_KICK")
            }
            _ => None,
        }
    }

    fn nearest(ctx: &StateProcessingContext) -> Option<MatchPlayerLite> {
        ctx.players()
            .teammates()
            .all()
            .filter(|m| !m.tactical_positions.is_goalkeeper())
            .map(|m| (m, (m.position - ctx.player.position).norm()))
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .map(|(m, _)| m)
    }

    #[cfg(feature = "match-logs")]
    fn note_corner_delivery(ctx: &StateProcessingContext, target_id: u32) {
        use crate::r#match::player::strategies::common::players::ops::forward_shot_decision::mid_run_diag::{
            CORNER_CROSS_SENT, CORNER_CROSS_TO_CB,
        };
        use std::sync::atomic::Ordering;
        CORNER_CROSS_SENT.fetch_add(1, Ordering::Relaxed);
        if ctx
            .context
            .players
            .by_id(target_id)
            .is_some_and(|t| t.tactical_position.current_position.is_central_defender())
        {
            CORNER_CROSS_TO_CB.fetch_add(1, Ordering::Relaxed);
        }
    }
}
