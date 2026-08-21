use crate::r#match::MatchPlayer;
use crate::r#match::engine::ball::ball::Ball;
use nalgebra::Vector3;

/// How high a footballer can play the ball, and what it costs him to do
/// it. All heights in metres, matching the ball's vertical axis (see
/// [`GRAVITY_PER_TICK`]).
///
/// # Why this is one model rather than a constant per call site
///
/// Every aerial decision in the engine used to carry its own literal —
/// `2.5` in the intercept gate, `3.5` in the claim loop, `2.8` for a pass
/// receiver, `1.5` to enter a header — and none of them agreed with any
/// other or with a human being. Worse, all of them were BINARY: below the
/// number the ball was as easy to play as one rolling along the floor,
/// above it the ball did not exist. A binary gate is what produces the
/// two symptoms that look opposite and share a cause — a defender picking
/// a ball out of the air at shoulder height without moving, and nobody at
/// all going for one a few centimetres higher.
///
/// Height is a difficulty, not a door. [`Self::reach_difficulty`] is the
/// curve; [`Self::ceiling`] is the only genuine door, and it is a
/// property of the player rather than of the engine.
pub struct AerialReach;

impl AerialReach {
    /// Head height of an average player.
    pub const HEAD: f32 = 1.8;

    /// The highest a player can play the ball with both feet on the
    /// floor: a raised boot, a stretched neck, a chest-high volley.
    /// Above this he has to leave the ground, and if he does not, he
    /// should not be getting the ball.
    pub const STANDING: f32 = 2.2;

    /// Ball height a poor leaper reaches at the top of a jump.
    const JUMP_MIN: f32 = 2.5;
    /// Ball height an elite leaper reaches at the top of a jump. Real
    /// aerial specialists head the ball around 2.9-3.0 m.
    const JUMP_MAX: f32 = 3.1;

    /// The highest ball this player can play, given his `jumping`
    /// attribute on the raw 1-20 scale.
    #[inline]
    pub fn ceiling(jumping: f32) -> f32 {
        let spring = ((jumping - 1.0) / 19.0).clamp(0.0, 1.0);
        Self::JUMP_MIN + spring * (Self::JUMP_MAX - Self::JUMP_MIN)
    }

    /// True when the ball is high enough that playing it means leaving
    /// the ground.
    #[inline]
    pub fn needs_leap(ball_z: f32) -> bool {
        ball_z > Self::STANDING
    }

    /// How much of his usual chance a player keeps at this ball height,
    /// 1.0 on the deck falling to 0 at his own ceiling.
    ///
    /// Squared rather than linear because the hard part of an aerial ball
    /// is the last few centimetres: a ball at knee height and one at
    /// chest height are both simply *there*, while one at the very top of
    /// the jump is a fingertip touch that mostly does not come off.
    #[inline]
    pub fn reach_difficulty(ball_z: f32, jumping: f32) -> f32 {
        if ball_z <= Self::HEAD {
            return 1.0;
        }
        let ceiling = Self::ceiling(jumping);
        if ball_z >= ceiling {
            return 0.0;
        }
        let over = (ball_z - Self::HEAD) / (ceiling - Self::HEAD);
        (1.0 - over * over).clamp(0.0, 1.0)
    }

    /// Apex, in metres, of the jump this player must make to meet a ball
    /// at `ball_z` with whatever he plays it with — a boot, a knee, a
    /// shoulder. Zero when he can reach it standing.
    ///
    /// He jumps to bring his own reach up to the ball and no further —
    /// an aerial challenge is timed, not maximal, and a player who
    /// launched himself to his ceiling for every ball above his head
    /// would spend the match in orbit.
    #[inline]
    pub fn leap_for(ball_z: f32, jumping: f32) -> f32 {
        Self::leap_from(ball_z, jumping, Self::STANDING)
    }

    /// The same, for a ball he is going to HEAD.
    ///
    /// A header is played off the forehead, not off a raised boot, so it
    /// is measured from [`Self::HEAD`] — 40 cm lower than
    /// [`Self::STANDING`]. Using the standing reach here is what would
    /// keep a player flat-footed for every header between 1.8 m and
    /// 2.2 m, which is most of them.
    #[inline]
    pub fn header_leap_for(ball_z: f32, jumping: f32) -> f32 {
        Self::leap_from(ball_z, jumping, Self::HEAD)
    }

    #[inline]
    fn leap_from(ball_z: f32, jumping: f32, reach: f32) -> f32 {
        if ball_z <= reach {
            return 0.0;
        }
        let ceiling = Self::ceiling(jumping);
        (ball_z - reach).min((ceiling - reach).max(0.0)).max(0.0)
    }
}

/// What a decided aerial contest does to the ball once it gets there.
///
/// The contest picks the outcome at the strike, because that is where the
/// skill comparison belongs; the outcome is *applied* on arrival, because
/// that is where the contact is. See [`AerialDelivery`].
#[derive(Clone, Copy, Debug)]
pub enum AerialOutcome {
    /// The attacker won it. Hold the ball in the heading band, drifting
    /// goalward, so his heading state gets valid ticks to strike it —
    /// the calibrated hang the corner path documents at length.
    Header { drift: Vector3<f32> },
    /// The defender won it and puts it behind for another corner.
    /// `attacked_goal` is the goal being attacked, i.e. the one he is
    /// clearing over his own byline.
    HookedBehind {
        attacked_goal: Vector3<f32>,
        field_height: f32,
    },
}

/// A delivery whose aerial contest is already decided, in the air on its
/// way to the man who won it.
///
/// # The defect this exists to remove
///
/// `resolve_corner_contest` and `resolve_cross_contest` elect a winner the
/// instant the delivery is struck, and used to finish the job by writing
/// the ball onto his head. Measured over 40 matches at level 14 that was
/// **1.9 relocations a match at a mean of 25 m**, every one of them large
/// enough for a replay to show — comfortably the largest thing in the
/// engine still moving the ball without a flight, and exactly the "the
/// ball teleports on corners" report.
///
/// The duel is not the problem: resolving one skill-weighted contest at
/// the strike is what stops twenty-two state machines settling a crowded
/// box by whoever's `process` runs first, and its win rate carries the
/// corner's whole calibration. What was wrong is that the OUTCOME was
/// applied by moving the ball. So the contest now solves a real arc to the
/// winner ([`Ball::ballistic_launch_arriving_at`]) and parks its result
/// here; the ball flies the twenty-five metres, and the outcome is applied
/// when it arrives.
///
/// # Why the flight is exempt from the loose-ball machinery
///
/// The contest has *already* priced every defender in the box and the
/// keeper's command of his area. Letting `try_intercept` roll again on the
/// way is the same double jeopardy the heading states carve out for
/// `aerial_contest_winner`, and it would quietly re-tune corner conversion
/// as a side effect of a rendering fix. So while this is armed the
/// delivery is nobody's but the winner's — which leaves the arm
/// behaviour-identical to the teleport it replaces, with a flight in the
/// middle.
#[derive(Clone, Copy, Debug)]
pub struct AerialDelivery {
    /// Who the contest awarded it to.
    pub winner_id: u32,
    /// Where the arc was solved to arrive, at heading height.
    pub target: Vector3<f32>,
    /// What happens when it gets there.
    pub outcome: AerialOutcome,
    /// Height the ball is being delivered to, in metres.
    pub arrival_height: f32,
    /// Tick past which the delivery is abandoned and the ball becomes an
    /// ordinary loose one. A solved flight plus a margin: without it a
    /// delivery whose winner is tackled, substituted or sent off would
    /// hold the ball out of play indefinitely.
    pub deadline_tick: u64,
    /// Put the winner into his role's heading state when the ball gets to
    /// him.
    ///
    /// ⚠ **On arrival, not at the strike.** `resolve_cross_contest` used
    /// to force the transition the instant it elected him, which was
    /// right when the ball was written onto his head on the same tick and
    /// is wrong now that it flies for 1.5 s first: the heading state has
    /// its own exit conditions and does not survive a second and a half
    /// of the ball being nowhere near. Measured, the cross contest went
    /// `attacker-won 21 → 28` and `headers on goal 10 → 0` — it kept
    /// winning duels and stopped producing headers, which is the exact
    /// failure its own doc-comment records ("the contest decided a duel
    /// nobody then took").
    pub force_heading: bool,
}

impl Ball {
    /// Carry a decided aerial contest through its flight, and apply its
    /// outcome the tick the ball actually gets there.
    ///
    /// See [`AerialDelivery`] for why the outcome is applied here rather
    /// than at the strike. Three things end a delivery:
    ///
    /// * **it arrives** — the ball is inside the winner's heading reach
    ///   and has come down into the band, so the hold that the old code
    ///   wrote along with the position is applied to the VELOCITY alone
    ///   and the ball is handed to his heading state exactly as before;
    /// * **the deadline passes** — the winner never got there (tackled,
    ///   substituted, sent off, or simply beaten to the spot), and the
    ///   delivery becomes an ordinary loose ball;
    /// * **somebody touches it** — handled by `record_touch`.
    ///
    /// Nothing here writes `position`. That is the whole point.
    pub(in crate::r#match::engine::ball::ball) fn tick_aerial_delivery(
        &mut self,
        players: &[MatchPlayer],
    ) {
        let Some(delivery) = self.aerial_delivery else {
            return;
        };
        if self.current_tick_cached >= delivery.deadline_tick {
            self.aerial_delivery = None;
            #[cfg(feature = "match-logs")]
            crate::r#match::engine::ball::ball::teleport::TeleportCensus::note_delivery_lost();
            // The grant goes with it: a contest whose ball never arrived
            // did not award anybody anything, and leaving the flag up
            // would let the winner head a ball he had to chase down.
            self.aerial_contest_winner = None;
            self.flags.in_flight_state = 0;
            return;
        }
        if players.iter().all(|p| p.id != delivery.winner_id) {
            self.aerial_delivery = None;
            self.aerial_contest_winner = None;
            #[cfg(feature = "match-logs")]
            crate::r#match::engine::ball::ball::teleport::TeleportCensus::note_delivery_lost();
            return;
        }
        // Still climbing, or still above head height: not there yet.
        if self.velocity.z > 0.0 || self.position.z > delivery.arrival_height {
            return;
        }
        /// How far off its aim point a delivery may be and still count as
        /// having arrived, in game units. 24 u is 3 m.
        ///
        /// # ⚠ Measured against the TARGET, not against the winner
        ///
        /// It used to be a 6 u radius around the winner himself, on the
        /// reasoning that the outcome should be applied where the contact
        /// happens. That reasoning is right and the test was wrong, for a
        /// reason the delivery census made obvious the moment it existed:
        /// **26% of deliveries reached the winner and 64% timed out.** A
        /// man attacking a corner is running while the ball is in the air
        /// — that is what attacking a corner is — so an arc solved to
        /// where he stood 1.85 s ago does not land on him, and a duel the
        /// contest had already awarded was quietly being thrown away
        /// along with `aerial_contest_winner`. `CB header chances` fell
        /// 9 → 1 per 60 matches on exactly this.
        ///
        /// A cross does not home. It is aimed at a spot and the attacker
        /// runs onto it, which is what the aim point is: his position at
        /// the strike. So the delivery arrives when it reaches the SPOT,
        /// the hold then keeps it in the heading band for ~40 ticks
        /// (`AerialOutcome::Header`'s −0.02 m/tick), and the winner —
        /// whose own state is steering him at the ball throughout — has
        /// that long to meet it. The radius is a sanity guard against
        /// applying the outcome to a ball something deflected on the way,
        /// not a gate the honest case has to squeeze through.
        const ARRIVAL_RADIUS: f32 = 24.0;
        let gap = (delivery.target.x - self.position.x).hypot(delivery.target.y - self.position.y);
        if gap > ARRIVAL_RADIUS {
            return;
        }
        // Arrived. Apply the outcome on the VELOCITY only — the position
        // is wherever the flight put it, which is the whole difference
        // between this and the write it replaces.
        #[cfg(feature = "match-logs")]
        crate::r#match::engine::ball::ball::teleport::TeleportCensus::note_delivery_arrived(gap);
        if delivery.force_heading {
            self.pending_aerial_strike = Some(delivery.winner_id);
        }
        self.velocity = match delivery.outcome {
            AerialOutcome::Header { drift } => drift,
            AerialOutcome::HookedBehind {
                attacked_goal,
                field_height,
            } => {
                // He heads it over his own byline. The grant belongs to
                // nobody now — this is a clearance, not a chance.
                self.aerial_contest_winner = None;
                self.pass_target_player_id = None;
                self.clear_pending_pass_metadata();
                Self::hook_behind_velocity(self.position, attacked_goal, field_height)
            }
        };
        self.aerial_delivery = None;
    }
}

#[cfg(test)]
mod aerial_reach_tests {
    use super::*;

    /// Height must be a difficulty, not a door. The engine's aerial gates
    /// were all binary: below the number the ball was as easy to play as
    /// one on the floor, above it the ball did not exist. That single
    /// shape produced both reported symptoms — defenders picking balls
    /// out of the air without moving, and nobody at all going for one a
    /// few centimetres higher.
    #[test]
    fn reach_difficulty_falls_away_smoothly_instead_of_switching_off() {
        let jumping = 12.0;
        let ceiling = AerialReach::ceiling(jumping);
        assert_eq!(
            AerialReach::reach_difficulty(0.0, jumping),
            1.0,
            "a ball on the deck is no harder than a ball on the deck"
        );
        assert_eq!(
            AerialReach::reach_difficulty(AerialReach::HEAD, jumping),
            1.0,
            "up to head height costs nothing"
        );
        assert_eq!(
            AerialReach::reach_difficulty(ceiling + 0.01, jumping),
            0.0,
            "past his own ceiling he cannot play it at all"
        );

        // Strictly decreasing in between — no plateau a player could sit
        // on, and no cliff.
        let mut previous = 1.0;
        let mut z = AerialReach::HEAD;
        while z < ceiling {
            let d = AerialReach::reach_difficulty(z, jumping);
            assert!(
                d <= previous,
                "difficulty must not rise as the ball climbs (at {z} m)"
            );
            assert!((0.0..=1.0).contains(&d), "difficulty stays a fraction");
            previous = d;
            z += 0.05;
        }
        assert!(
            previous < 0.25,
            "a ball at the very top of the jump must be a fingertip touch, got {previous}"
        );
    }

    /// The ceiling belongs to the PLAYER. The old flat `2.5` gate meant
    /// the best header of the ball in the division and the worst had
    /// exactly the same aerial range.
    #[test]
    fn a_better_leaper_reaches_a_higher_ball() {
        let poor = AerialReach::ceiling(1.0);
        let elite = AerialReach::ceiling(20.0);
        assert!(
            elite > poor + 0.4,
            "jumping must be worth real height: {poor} vs {elite}"
        );
        // A ball an elite leaper can just about reach is out of a poor
        // one's range entirely.
        let z = poor + 0.1;
        assert_eq!(AerialReach::reach_difficulty(z, 1.0), 0.0);
        assert!(AerialReach::reach_difficulty(z, 20.0) > 0.0);
    }

    /// A jump is timed to the ball, not maximal — otherwise a player
    /// would launch himself to his ceiling for every ball above his head.
    #[test]
    fn a_leap_reaches_the_ball_and_no_further() {
        let jumping = 14.0;
        assert_eq!(
            AerialReach::leap_for(AerialReach::STANDING - 0.1, jumping),
            0.0,
            "a ball he can reach standing needs no jump"
        );
        let low = AerialReach::leap_for(AerialReach::STANDING + 0.2, jumping);
        let high = AerialReach::leap_for(AerialReach::STANDING + 0.6, jumping);
        assert!(low > 0.0 && high > low, "higher ball, bigger jump");
        // Never asked to jump past his own ceiling.
        let ceiling = AerialReach::ceiling(jumping);
        let beyond = AerialReach::leap_for(ceiling + 5.0, jumping);
        assert!(
            beyond <= ceiling - AerialReach::STANDING + 1.0e-4,
            "the leap is bounded by what he can actually jump"
        );
    }

    /// A header is played off the forehead, not off a raised boot, so it
    /// starts 40 cm lower. Measuring it from the standing reach is what
    /// would keep a player flat-footed for most real headers.
    #[test]
    fn a_header_leaves_the_ground_earlier_than_a_boot_does() {
        let jumping = 12.0;
        let z = AerialReach::HEAD + 0.15; // 1.95 m — a normal header
        assert_eq!(
            AerialReach::leap_for(z, jumping),
            0.0,
            "a boot can still reach this standing"
        );
        assert!(
            AerialReach::header_leap_for(z, jumping) > 0.0,
            "but heading it means jumping"
        );
    }
}
