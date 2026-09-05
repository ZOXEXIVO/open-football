//! **One reach rule, on three axes.**
//!
//! A ball above a man's reach is nobody's until it comes down. That
//! sentence is the whole of this module, and the reason it exists is
//! that the engine used to answer it in five different places with five
//! different numbers — and on the event path, with no height at all.
//!
//! # The defect
//!
//! `Ball::within_possession_reach` measured `sqrt(dx² + dy²)` and said
//! so in its own doc comment: *"a ball directly overhead is within reach
//! whatever its height."* Every event-driven grant went through it —
//! `ClaimBall`, `GainBall`, `TacklingBall`, `MoveBall`,
//! `BallOwnerChange` — so a player in a Running state was handed balls
//! that were six metres over his head. Decoded from one recorded match
//! (2026-09-04): `ClaimBall(207)` at **6.6 m**, `ClaimBall(206)` at
//! **5.9 m**, and 22 grants a match above head height that then dragged
//! the ball down on a string.
//!
//! The scan paths did have ceilings, and no two agreed: 3.5 m in
//! `check_ball_ownership`, 2.8 m for a pass receiver, 2.5 m in the
//! notified claim, [`AerialReach::ceiling`] in `try_intercept`. The
//! replay viewer had copied the 2.8 across into its own possession rule
//! (`Soundtrack::OVERHEAD`), so the picture was already refusing balls
//! the engine was granting.
//!
//! # The rule
//!
//! Height is a property of the PLAYER, not of the engine — see
//! [`AerialReach`], which prices it. So there is one predicate here and
//! it takes the man:
//!
//! * [`PlayerReach::can_possess`] — may he be GIVEN the ball: within
//!   [`MAX_OWNER_TRACK_DISTANCE`] across the grass and under his own
//!   jumping ceiling.
//! * [`PlayerReach::under_ceiling`] — the height half on its own, for
//!   the grants whose horizontal reach is a separate, calibrated
//!   question (`secure_ball_for`: a tackle writes the ball to the
//!   tackler's feet rather than refusing, and the tackle rate is a
//!   calibrated number).
//! * [`PlayerReach::can_strike`] — may he KICK it: within
//!   [`KICKABLE_DISTANCE`] and no higher than what he is striking it
//!   with. A boot reaches [`AerialReach::VOLLEY`]; a state that has
//!   declared itself aerial — the heading states, the winner of a
//!   decided aerial contest, a goalkeeper's punch — reaches his full
//!   jumping ceiling.
//!
//! Nothing here rolls a die or moves anything. It answers one question
//! for every caller, so the answer cannot drift.

use crate::r#match::MatchPlayer;
use crate::r#match::engine::ball::ball::{
    AerialReach, Ball, KICKABLE_DISTANCE, MAX_OWNER_TRACK_DISTANCE,
};
use nalgebra::Vector3;

/// What a player can reach, and what he can do to it when he gets there.
///
/// A unit struct rather than loose helpers because the three questions
/// below are one rule read three ways, and the engine's history here is
/// entirely of call sites forming their own opinions.
pub struct PlayerReach;

impl PlayerReach {
    /// The highest ball this player can play at all, in metres — a
    /// standing reach for a poor leaper, the top of a jump for a good
    /// one. See [`AerialReach::ceiling`].
    #[inline]
    pub fn ceiling(player: &MatchPlayer) -> f32 {
        AerialReach::ceiling(player.skills.physical.jumping)
    }

    /// The highest ball this player can STRIKE, in metres.
    ///
    /// A boot stops at [`AerialReach::VOLLEY`]. Above it the contact has
    /// to be a header or a punch, and both of those are states — so a
    /// player whose state has committed to an aerial strike is measured
    /// against his jumping ceiling instead, exactly as a grant is.
    #[inline]
    pub fn strike_ceiling(player: &MatchPlayer, aerial: bool) -> f32 {
        if aerial {
            Self::ceiling(player)
        } else {
            AerialReach::VOLLEY
        }
    }

    /// Height alone: is the ball inside this player's vertical reach?
    ///
    /// Split out from [`Self::can_possess`] because the two axes have
    /// different histories. The horizontal cap exists to stop a grant
    /// stranding the ball ([`Ball::within_possession_reach`]) and
    /// `secure_ball_for` is deliberately exempt from it — it writes the
    /// ball to the winner's feet instead, and refusing there would move
    /// the calibrated tackle and interception rates. The VERTICAL cap is
    /// not negotiable on any path: nobody wins a tackle on a ball six
    /// metres over his head.
    #[inline]
    pub fn under_ceiling(ball: &Ball, player: &MatchPlayer) -> bool {
        ball.position.z <= Self::ceiling(player)
    }

    /// May this player be handed the ball right now — across the grass
    /// AND up the vertical axis?
    #[inline]
    pub fn can_possess(ball: &Ball, player: &MatchPlayer) -> bool {
        Self::under_ceiling(ball, player)
            && Self::within(ball.position, player.position, MAX_OWNER_TRACK_DISTANCE)
    }

    /// May this player KICK the ball right now?
    ///
    /// `aerial` is what the striker's STATE says he is doing with it —
    /// see [`PlayerState::strikes_in_the_air`](crate::r#match::PlayerState::strikes_in_the_air)
    /// and `Ball::pending_aerial_strike`. A Running-state midfielder
    /// passing a ball off the top of his head is exactly what this
    /// refuses; the same ball half a second later, on the deck, is his.
    #[inline]
    pub fn can_strike(ball: &Ball, player: &MatchPlayer, aerial: bool) -> bool {
        ball.position.z <= Self::strike_ceiling(player, aerial)
            && Self::within(ball.position, player.position, KICKABLE_DISTANCE)
    }

    /// Across the grass only, in game units. Both axes carry the same
    /// unit (1u = 0.125 m); the vertical one does not, which is the
    /// reason every height test in this module is written out separately
    /// rather than folded into a 3-D norm.
    #[inline]
    fn within(ball: Vector3<f32>, player: Vector3<f32>, radius: f32) -> bool {
        let dx = player.x - ball.x;
        let dy = player.y - ball.y;
        dx * dx + dy * dy <= radius * radius
    }
}
