use crate::r#match::MatchContext;
use crate::r#match::MatchPlayer;
use crate::r#match::engine::ball::ball::{Ball, PlayerReach};
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

    /// **The highest ball a BOOT can strike**, in metres — the volley
    /// ceiling.
    ///
    /// Below [`Self::HEAD`] because that is what a footballer's leg
    /// does: a shin volley, a knee, a hooked clearance off the thigh all
    /// happen under this, and above it the contact is a head or a
    /// shoulder — which is a DECISION a heading state takes, not
    /// something a Running state does by accident.
    ///
    /// It is the same 1.45 m the replay viewer draws a boot below
    /// (`Actors::HEADED`), and it lives beside [`Self::HEAD`] and
    /// [`Self::STANDING`] so the two cannot drift: the picture and the
    /// engine have to agree about what a header is, or a man is drawn
    /// hooking his boot up past his own ear. Measured off one recorded
    /// match before this number reached the strike handlers, the engine
    /// struck 18 passes and 5 clearances a match out of the head band
    /// with nobody heading anything.
    pub const VOLLEY: f32 = 1.45;

    /// Ball height a poor leaper reaches at the top of a jump.
    const JUMP_MIN: f32 = 2.5;
    /// Ball height an elite leaper reaches at the top of a jump. Real
    /// aerial specialists head the ball around 2.9-3.0 m.
    const JUMP_MAX: f32 = 3.1;

    /// The highest ball ANYBODY on the pitch can play — the ceiling of
    /// the best leaper there could be.
    ///
    /// The whole-pitch early-out: above this the ball is nobody's,
    /// whoever is standing under it. `check_ball_ownership` used to
    /// carry a hand-rolled `MAX_BALL_HEIGHT` of 4.0 m for the same
    /// question, most of a metre above the tallest jump this model
    /// admits.
    pub const HIGHEST: f32 = Self::JUMP_MAX;

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
    /// The defender won it and heads it away upfield.
    ///
    /// Carried as an INTENT rather than a solved vector for the same
    /// reason [`Self::HookedBehind`] is: the clearance is struck from
    /// wherever the ball actually reaches him, and a velocity frozen at
    /// the contest would be aimed from where the ball was a second
    /// earlier. See [`Ball::headed_clear_velocity`].
    Cleared {
        attacked_goal: Vector3<f32>,
        /// How far the clearance travels, in units.
        range: f32,
        /// How high it goes over the strike, in metres.
        apex: f32,
    },
}

/// **What the man who won an aerial contest is going to do with the ball**
/// — the caller's half of [`AerialOutcome`], stated before the geometry
/// that depends on where the ball actually arrives is known.
///
/// The two are separate because a contest is decided at one place and
/// applied at another. `resolve_corner_contest` elects a winner while the
/// ball is still at the flag; the header, the hook or the clearance is
/// struck a second and a half later, twenty-five metres away, and every
/// one of those is aimed from where the ball IS when he meets it.
#[derive(Clone, Copy, Debug)]
pub enum DeliveryIntent {
    /// He attacks it. The ball is held in the heading band for him and his
    /// own heading state strikes it through the normal shot pipeline.
    Header,
    /// He hooks it over his own byline and concedes the corner.
    HookedBehind,
    /// He heads it away upfield — `range` units at `apex` metres.
    Cleared { range: f32, apex: f32 },
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
    /// **Where this ball's own flight next comes down through
    /// `arrival_height`** — the point a delivery would reach if nothing
    /// re-aimed it, plus how many ticks it takes to get there.
    ///
    /// This is the question
    /// [`deliver_to_winner`](crate::r#match::engine::engine::FootballEngine::deliver_to_winner)
    /// has to ask before it solves an arc. A contest resolved at the
    /// STRIKE — a corner, still at the flag with the winner twenty-five
    /// metres away — genuinely has to launch the ball. A contest resolved
    /// MID-FLIGHT — an open-play cross, already descending through head
    /// height a stride from the man — does not: the ball is already going
    /// to him, and writing a fresh launch onto it turns a ball nobody has
    /// touched.
    ///
    /// `None` when the ball is not on a flight that gets there — it
    /// reaches the turf first, or is rolling along it already.
    pub fn natural_drop(&self, arrival_height: f32) -> Option<(Vector3<f32>, u32)> {
        // Already under the band on the way down: it is there now.
        if self.velocity.z <= 0.0 && self.position.z <= arrival_height {
            return Some((self.position, 0));
        }
        let horizontal = self.velocity.x.hypot(self.velocity.y);
        // Straight down is a legitimate flight and has no heading — there
        // is nowhere for it to land but under itself.
        let heading = Vector3::new(self.velocity.x, self.velocity.y, 0.0)
            .try_normalize(1.0e-4)
            .unwrap_or_else(Vector3::zeros);
        let (range, ticks) =
            Self::ballistic_arrival(horizontal, self.velocity.z, self.position.z, arrival_height);
        // `ballistic_arrival` reports the turf, not the band, for a ball
        // that never comes down through it — a flight that ends on the
        // deck has no drop point at heading height.
        if self.position.z + Self::apex_for_launch(self.velocity.z) < arrival_height {
            return None;
        }
        Some((
            Vector3::new(
                self.position.x + heading.x * range,
                self.position.y + heading.y * range,
                arrival_height,
            ),
            ticks,
        ))
    }

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
        // ⚠ **…AND THE HEADER HAPPENS AT THE MAN.**
        //
        // The radius above says the ball got where it was AIMED. The
        // outcome is a header — a velocity written onto the ball at heading
        // height — and writing it because the ball reached a SPOT turns the
        // ball round in mid-air whether or not the winner is under it.
        // Measured off a recorded match: **a dozen a match**, reversing
        // 160-180° at 2.75-3.0 m with the nearest man three metres away, in
        // the band 11-17 m out from goal where a cross is attacked. That is
        // the reported *"the ball bounces off something invisible above the
        // player"*, and it is the same defect the block channels carried.
        //
        // The note above is right that a radius around the WINNER is not
        // how a delivery arrives — a cross is aimed at a spot and the man
        // runs onto it. So both halves are asked: the ball has to reach the
        // spot, and he has to be able to head it when it does. If he is not
        // there yet the hold keeps the ball in the heading band for ~40
        // ticks and this is asked again on the next one; the deadline is
        // what ends it if he never arrives, exactly as before.
        //
        // `PlayerReach::can_strike(.., aerial = true)` and nothing of this
        // module's own: it is `KICKABLE_DISTANCE` across the grass and the
        // man's own jumping ceiling up it, which is the engine's single
        // answer to "may he play this ball", and a second opinion here is
        // how the two would drift.
        let winner = players.iter().find(|p| p.id == delivery.winner_id);
        #[cfg(feature = "match-logs")]
        let to_winner = winner
            .map(|p| (p.position.x - self.position.x).hypot(p.position.y - self.position.y))
            .unwrap_or(f32::MAX);
        let in_reach = winner.is_some_and(|p| PlayerReach::can_strike(self, p, true));
        if !in_reach && !MatchContext::aerial_arrival_flat() {
            // He is not there yet. The hold keeps the ball in the heading
            // band and this is asked again next tick; the deadline is what
            // ends it if he never comes.
            #[cfg(feature = "match-logs")]
            crate::r#match::engine::ball::ball::teleport::TeleportCensus::note_delivery_waiting();
            return;
        }
        // Booked HERE and not above the gate: the delivery arrives once,
        // and a census that counts every tick the ball spends over its aim
        // point waiting for him is counting ticks, not arrivals.
        #[cfg(feature = "match-logs")]
        crate::r#match::engine::ball::ball::teleport::TeleportCensus::note_delivery_arrived(
            gap,
            to_winner,
            self.position.z,
            !in_reach,
        );
        // Arrived. Apply the outcome on the VELOCITY only — the position
        // is wherever the flight put it, which is the whole difference
        // between this and the write it replaces.
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
            AerialOutcome::Cleared {
                attacked_goal,
                range,
                apex,
            } => {
                // His to clear, not his to attack — the same disarming the
                // hooked branch does, and for the same reason: this is a
                // clearance, so nothing here is anybody's chance.
                self.aerial_contest_winner = None;
                self.pass_target_player_id = None;
                self.clear_pending_pass_metadata();
                Self::headed_clear_velocity(self.position, attacked_goal, range, apex)
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

#[cfg(test)]
mod natural_drop_tests {
    use super::*;

    /// A ball, put where the test wants it and sent where the test says.
    struct Flight;

    impl Flight {
        fn at(position: Vector3<f32>, velocity: Vector3<f32>) -> Ball {
            let mut ball = Ball::with_coord(840.0, 545.0);
            ball.position = position;
            ball.velocity = velocity;
            ball
        }
    }

    /// The case the whole fix turns on: a cross descending through the
    /// heading band is ALREADY where the delivery was going to put it, so
    /// its drop point is under itself and no arc has to be solved.
    #[test]
    fn a_ball_already_in_the_band_drops_where_it_is() {
        let ball = Flight::at(
            Vector3::new(400.0, 250.0, 2.3),
            Vector3::new(0.6, -0.2, -0.05),
        );
        let (drop, ticks) = ball.natural_drop(2.5).expect("it is in the band now");
        assert_eq!(ticks, 0, "it has already arrived — there is nothing to fly");
        assert_eq!(drop, ball.position);
    }

    /// …and one still coming down through it lands AHEAD of itself, along
    /// its own heading. This is the aim point a kept delivery is given.
    #[test]
    fn a_ball_still_coming_down_lands_ahead_of_itself() {
        let ball = Flight::at(
            Vector3::new(400.0, 250.0, 3.0),
            Vector3::new(0.8, 0.0, -0.04),
        );
        let (drop, ticks) = ball.natural_drop(2.5).expect("it comes down through it");
        assert!(ticks > 0, "it has not got there yet");
        assert!(
            drop.x > ball.position.x,
            "it drops down the line it is travelling, not behind itself: {drop:?}"
        );
        assert_eq!(drop.z, 2.5, "the drop is BY DEFINITION at the band");
    }

    /// A ball whose whole flight stays under the band never reaches it —
    /// and answering "under itself" there would let a delivery ride a
    /// flight that is really a ball rolling along the floor.
    #[test]
    fn a_flight_that_never_climbs_to_the_band_has_no_drop() {
        let ball = Flight::at(
            Vector3::new(400.0, 250.0, 0.3),
            Vector3::new(1.2, 0.0, 0.02),
        );
        assert!(
            ball.natural_drop(2.5).is_none(),
            "a ball peaking well under head height never comes down through it"
        );
    }

    /// A corner, one tick off the taker's boot: climbing, and below the
    /// band. This is the arm that MUST still solve an arc — the winner is
    /// twenty-five metres away and the ball's own flight is nowhere near
    /// him — so it must not read as "already there".
    #[test]
    fn a_corner_leaving_the_flag_is_not_already_in_the_band() {
        let ball = Flight::at(Vector3::new(419.0, 5.0, 2.1), Vector3::new(1.4, 1.0, 0.18));
        assert!(
            ball.velocity.z > 0.0,
            "the discriminant is CLIMBING, and this is the case it protects"
        );
    }
}
