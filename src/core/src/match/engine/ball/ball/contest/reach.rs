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

/// Where a committed block stands this tick — see
/// [`PlayerReach::block_contact`].
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum BlockContact {
    /// The ball has not reached him yet. Hold the commitment.
    Coming,
    /// It is at him, inside his reach on all three axes. This is where
    /// the deflection happens.
    AtTheBody,
    /// It has gone by — over his head, or past him and out of reach.
    /// He did not block it, and the commitment is spent.
    Missed,
}

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

    /// **May he get a body to this ball, right now?** — the block
    /// channels' contact test.
    ///
    /// Both block models decide the RATE where the read happens and
    /// defer the CONTACT until the ball reaches the man who won it
    /// (`ShotTarget::blocked_by`, `Ball::pass_blocked_by`), for the
    /// reason `try_block_shot` gives: the candidate window reaches
    /// eleven metres up the shot line, so deflecting on the tick of the
    /// roll turns the ball round in mid-flight with the defender still
    /// eleven metres away.
    ///
    /// ⚠ **That deferral used to test `hypot(dx, dy)` and nothing
    /// else.** The height gate sits at the roll, which is the wrong end
    /// of the flight — a shot rolled for at knee height was deflected
    /// wherever the ball had climbed to by the time it arrived, and a
    /// ball three metres up came off a man standing underneath it.
    /// Nothing is drawn there: the replay rig attributes a contact only
    /// within a stride and below its own claim ceiling, so the picture
    /// was a ball turning in mid-air over an idle defender. Reported
    /// 2026-09-06 as *"the ball bounces off an invisible object above
    /// the player"*, which is a literal description of it.
    ///
    /// `direction` is the ball's own heading across the grass, and it
    /// is what tells the two out-of-reach cases apart — a defender the
    /// ball is still travelling toward is waiting for it, one it has
    /// gone past has been beaten.
    ///
    /// ⚠ **And the contact is at his CLOSEST APPROACH, not at the edge
    /// of the radius.** A deferral that fires on the first tick the ball
    /// is within reach fires at the farthest point of it by
    /// construction, and measured off a recorded match the blocks piled
    /// up exactly there — 1.79, 1.84, 1.86, 1.90, 2.02 m from the man
    /// they came off, against a two-metre reach. The replay rig
    /// attributes a contact to a man within a stride, so seven of the
    /// eighteen blocks in that match were drawn coming off nobody.
    /// Waiting until the ball is level with him costs a handful of ticks
    /// and puts the deflection on the body, which is also when a
    /// defender actually throws a leg at one.
    ///
    /// The height is likewise only allowed to end the commitment once
    /// the ball is level with him: a dipping shot that is over his head
    /// at the edge of his reach may be on his shin at the middle of it,
    /// and refusing early would throw those away.
    pub fn block_contact(
        ball: &Ball,
        player: &MatchPlayer,
        direction: Vector3<f32>,
        reach: f32,
        ceiling: f32,
    ) -> BlockContact {
        let dx = player.position.x - ball.position.x;
        let dy = player.position.y - ball.position.y;
        // Positive while the ball is still travelling toward him, and zero
        // at the closest it will ever come.
        let along = dx * direction.x + dy * direction.y;
        if along > 0.0 {
            return BlockContact::Coming;
        }
        if dx * dx + dy * dy > reach * reach || ball.position.z > ceiling {
            BlockContact::Missed
        } else {
            BlockContact::AtTheBody
        }
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

/// The block contact rule — the one that decides whether a deflection
/// happens at a defender or in mid-air above him.
#[cfg(test)]
mod block_contact_tests {
    use super::*;
    use crate::shared::fullname::FullName;
    use crate::{
        PersonAttributes, PlayerAttributes, PlayerBuilder, PlayerPosition, PlayerPositionType,
        PlayerPositions, PlayerSkills,
    };
    use chrono::NaiveDate;

    /// A defender standing where the test puts him.
    struct Blocker;

    impl Blocker {
        fn at(x: f32, y: f32) -> MatchPlayer {
            let player = PlayerBuilder::new()
                .id(1)
                .full_name(FullName::new("T".to_string(), "P".to_string()))
                .birth_date(NaiveDate::from_ymd_opt(2000, 1, 1).unwrap())
                .country_id(1)
                .attributes(PersonAttributes::default())
                .skills(PlayerSkills::default())
                .positions(PlayerPositions {
                    positions: vec![PlayerPosition {
                        position: PlayerPositionType::DefenderCenter,
                        level: 18,
                    }],
                })
                .player_attributes(PlayerAttributes::default())
                .build()
                .unwrap();
            let mut man = MatchPlayer::from_player(
                1,
                &player,
                PlayerPositionType::DefenderCenter,
                false,
                None,
            );
            man.position = Vector3::new(x, y, 0.0);
            man
        }
    }

    /// A ball at `(x, y)` and `height` metres up, travelling along +x.
    struct Flight;

    impl Flight {
        fn at(x: f32, y: f32, height: f32) -> Ball {
            // `with_coord` takes the FIELD, not a position — it puts the
            // ball on the centre spot of a pitch that size.
            let mut ball = Ball::with_coord(840.0, 545.0);
            ball.position = Vector3::new(x, y, height);
            ball.velocity = Vector3::new(4.0, 0.0, 0.0);
            ball
        }
    }

    /// The reach and ceiling the shot block uses, in its own units.
    const REACH: f32 = 16.0;
    const CEILING: f32 = 2.2;
    const DIR: Vector3<f32> = Vector3::new(1.0, 0.0, 0.0);

    /// The whole report in one test: a ball three metres over a man's
    /// head is not a block, however close he is on the grass.
    #[test]
    fn a_ball_over_his_head_is_not_a_block() {
        let ball = Flight::at(100.0, 200.0, 3.0);
        let man = Blocker::at(100.0, 200.0);
        assert_eq!(
            PlayerReach::block_contact(&ball, &man, DIR, REACH, CEILING),
            BlockContact::Missed
        );
    }

    /// …and the same ball at shin height is.
    #[test]
    fn a_ball_at_his_shin_is_a_block() {
        let ball = Flight::at(100.0, 200.0, 0.3);
        let man = Blocker::at(100.0, 200.0);
        assert_eq!(
            PlayerReach::block_contact(&ball, &man, DIR, REACH, CEILING),
            BlockContact::AtTheBody
        );
    }

    /// A dipping shot still over his head at the edge of his reach has
    /// not beaten him yet — he is ahead of it, and it may be on his shin
    /// by the time it is level with him. Refusing here would throw away
    /// every block on a falling ball.
    #[test]
    fn a_high_ball_still_short_of_him_is_only_coming() {
        let ball = Flight::at(100.0, 200.0, 3.0);
        // Ten units (1.25 m) further up the pitch, so the ball is still
        // travelling toward him.
        let man = Blocker::at(110.0, 200.0);
        assert_eq!(
            PlayerReach::block_contact(&ball, &man, DIR, REACH, CEILING),
            BlockContact::Coming
        );
    }

    /// ⚠ **And a ball ALREADY inside his reach is still only coming**, so
    /// long as it is closing on him. Firing at the first tick inside a two
    /// metre radius fires at the far edge of it, which is where the blocks
    /// in a recorded match piled up — and a deflection two metres from the
    /// man it came off is drawn coming off nobody.
    #[test]
    fn the_contact_waits_for_his_closest_approach() {
        let man = Blocker::at(100.0, 200.0);
        // Inside the reach and still a metre up the pitch from him.
        let closing = Flight::at(92.0, 200.0, 0.3);
        assert_eq!(
            PlayerReach::block_contact(&closing, &man, DIR, REACH, CEILING),
            BlockContact::Coming
        );
        // Level with him: this is the tick the leg goes in.
        let level = Flight::at(100.0, 200.0, 0.3);
        assert_eq!(
            PlayerReach::block_contact(&level, &man, DIR, REACH, CEILING),
            BlockContact::AtTheBody
        );
    }

    /// And a ball that has gone past him is gone, whatever its height —
    /// this is what stops a commitment being carried to the other end of
    /// the pitch and spent on a man who was beaten forty metres ago.
    #[test]
    fn a_ball_past_him_is_missed_not_pending() {
        let man = Blocker::at(100.0, 200.0);
        for height in [0.2_f32, 3.0] {
            // Twenty units (2.5 m) beyond him and still going away.
            let ball = Flight::at(120.0, 200.0, height);
            assert_eq!(
                PlayerReach::block_contact(&ball, &man, DIR, REACH, CEILING),
                BlockContact::Missed,
                "at {height} m up"
            );
        }
    }

    /// Out of reach across the grass but still in front of him: the
    /// deferral this whole rule exists to keep working.
    #[test]
    fn a_ball_short_of_his_reach_is_coming() {
        let ball = Flight::at(100.0, 200.0, 0.3);
        let man = Blocker::at(160.0, 200.0);
        assert_eq!(
            PlayerReach::block_contact(&ball, &man, DIR, REACH, CEILING),
            BlockContact::Coming
        );
    }
}
