//! **The man standing over the ball on the centre mark.**
//!
//! # What was there before
//!
//! Nothing. A kick-off resolved to `current_owner = the taker` and from
//! that tick on he was an ordinary carrier under his ordinary state
//! machine — so the kick-off was whatever a forward alone on the halfway
//! line does with a ball at his feet. Traced through
//! [`ForwardPassingState`] at the first whistle of a 4-2-3-1, with the
//! lone striker on the ball:
//!
//! ```text
//!   t= 0   Standing -> Passing
//!   t= 8   Passing  -> (no pass found)
//!   t=15   Passing  -> (no pass found)
//!   t=31   Passing  -> Running          … and off he goes with it
//! ```
//!
//! He never had anybody to give it to. That is the reported *"he plays
//! the ball to himself"*, and it is the throw-in's defect at the other
//! restart — see [`ThrowInDelivery`](super::ThrowInDelivery).
//!
//! # What a kick-off is
//!
//! Law 8, in the parts that reach the pitch: the ball is stationary on
//! the centre mark, the opponents are outside the centre circle, and it
//! is **in play once it is kicked and clearly moves**. There is no other
//! thing the taker may do with it — he may not carry it, and standing
//! over it is a stalled match.
//!
//! So this is not a decision, and it is deliberately not scored like
//! one: the pair agreed the kick-off before the whistle, and
//! [`KickoffShape`] is what stood the receiver beside him. The taker
//! looks up for the length of a real kick-off and rolls it to him.
//!
//! # Why an override rather than a state
//!
//! For the reason [`ThrowInDelivery`](super::ThrowInDelivery) is one.
//! Four state machines would each need a "standing over a dead ball on
//! the centre mark" concept, and all four would need it to beat
//! everything else they do — the taker is not marking, not pressing, not
//! making a run, and any of those walks the ball away from the mark with
//! him.
//!
//! [`ForwardPassingState`]: crate::r#match::forwarders::states::ForwardPassingState
//! [`KickoffShape`]: crate::r#match::engine::kickoff_shape::KickoffShape

use crate::r#match::StateProcessingContext;
use crate::r#match::events::Event;
use crate::r#match::player::events::PlayerEvent;
use crate::r#match::player::events::models::PassingEventContext;

/// The kick-off delivery: the beat he holds it for, and the man he rolls
/// it to.
pub struct KickoffDelivery;

impl KickoffDelivery {
    /// How long he stands over it before he plays it, in engine ticks.
    /// 40 = 0.4 s.
    ///
    /// The same beat [`ThrowInDelivery`](super::ThrowInDelivery) gives the
    /// throw, and not decoration in either place: it is the pause between
    /// the whistle and the touch that every kick-off has, and it is what
    /// lets the retreat the set-up wrote settle before the ball moves.
    const SCAN_TICKS: u64 = 40;

    /// True while this player is the one taking a kick-off and still has
    /// the ball.
    ///
    /// Two reads and no scan for everybody else, on every tick of the
    /// match — the same shape as [`ThrowInDelivery::taking`], and for the
    /// same reason: it is asked once per player per tick at dispatch.
    ///
    /// [`ThrowInDelivery::taking`]: super::ThrowInDelivery::taking
    #[inline]
    pub fn taking(ctx: &StateProcessingContext) -> bool {
        ctx.tick_context.ball.kickoff_taker == Some(ctx.player.id)
            && ctx.tick_context.ball.current_owner == Some(ctx.player.id)
    }

    /// The kick-off, once he has looked up.
    ///
    /// `None` means he is still standing over it, which is what a
    /// kick-off looks like for the half-second before it is taken.
    pub fn deliver(ctx: &StateProcessingContext) -> Option<Event> {
        if (ctx.tick_context.ball.ownership_duration as u64) < Self::SCAN_TICKS {
            return None;
        }
        let partner = ctx.tick_context.ball.kickoff_partner?;
        Some(Event::PlayerEvent(PlayerEvent::PassTo(
            PassingEventContext::new()
                .with_from_player_id(ctx.player.id)
                .with_to_player_id(partner)
                .with_reason("KICK_OFF")
                .build(ctx),
        )))
    }
}
