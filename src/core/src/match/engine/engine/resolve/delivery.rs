//! **Delivering a decided aerial contest** — the shared arm both the
//! corner and the cross contest finish through, and the trajectory
//! constants that price it.
//!
//! Both used to end with a straight write of `b.position`, which is the
//! "ball teleports on corners" report. The duel still happens where it
//! did; what changed is that its result now flies to the winner on a
//! solved arc and is applied on arrival. The two contests pass different
//! apex and drop-short values, and the difference is load-bearing — a
//! midfielder's heading reach is 2.0u, which the corner's own 2.0u drop
//! sat exactly on the boundary of.

use crate::r#match::engine::ball::ball::Ball;
#[cfg(feature = "match-logs")]
use crate::r#match::engine::ball::ball::teleport as tc;
use crate::r#match::engine::ball::ball::{
    AerialDelivery, AerialOutcome, AerialReach, DeliveryIntent, FlightProtection,
};
use crate::r#match::engine::engine::*;
use nalgebra::Vector3;
#[cfg(feature = "match-logs")]
use std::sync::atomic::Ordering;

impl<const W: usize, const H: usize> FootballEngine<W, H> {
    /// Apex of a corner delivery, in metres. A normal in-swinger: 5 m up
    /// puts about 1.7 s between the strike and the header, which is what
    /// a real one takes and comfortably inside
    /// [`CornerDeadline`](crate::r#match::engine::corner_shape::CornerDeadline) so the
    /// set-piece shape holds for the whole flight.
    pub(in crate::r#match::engine::engine) const CORNER_APEX: f32 = 5.0;

    /// Apex of an open-play cross, in metres. Shorter than a corner
    /// because it is played from further forward and has to beat a moving
    /// line rather than a set one.
    pub(in crate::r#match::engine::engine) const CROSS_APEX: f32 = 4.0;

    /// How far short of the winner a corner is aimed, in units.
    pub(in crate::r#match::engine::engine) const CORNER_DROP_BEHIND: f32 = 2.0;

    /// The same for an open-play cross. 1.2u (15 cm) sits inside every
    /// role's heading reach, including the midfielder's 2.0u, which the
    /// corner's own 2.0u sits exactly on the boundary of.
    pub(in crate::r#match::engine::engine) const CROSS_DROP_BEHIND: f32 = 1.2;

    /// Does this defensive header go BEHIND for a corner rather than
    /// upfield? See the call site in [`resolve_cross_contest`].
    ///
    /// Depth decides it, because depth is what removes the option: a
    /// header met on the edge of the area can be sent anywhere, one met
    /// on the six-yard line with the ball travelling across you can only
    /// go one way. The share rises steeply as the goal line approaches
    /// and is zero outside the area, so ordinary defensive headers in and
    /// around the box still play the ball out as they always did.
    /// Put the ball over the defender's own byline, wide of the post.
    ///
    /// The other half of [`heads_it_behind`](Self::heads_it_behind) and of
    /// the corner contest's cleared branch: once the decision is taken,
    /// both need the same hooked, high, short trajectory, and both need it
    /// to finish OUTSIDE the posts — a clearance across the face of goal
    /// is an own goal, not a clearance.
    /// Send a decided aerial contest's ball to the man who won it — by
    /// flying it there, not by writing it onto his head.
    ///
    /// # The teleport this replaces
    ///
    /// Both contests used to finish with `b.position = winner_pos - dir *
    /// n`. Measured over 40 matches at level 14 with the whole-tick
    /// relocation census, `resolve_corner_contest` alone was **1.9
    /// relocations a match at a mean of 25 m, every one of them large
    /// enough for a replay to show** — the largest thing left in the
    /// engine moving the ball with no flight under it that is not a
    /// restart placing a dead ball on its spot. That is the "the ball
    /// teleports on corners" report, exactly.
    ///
    /// The duel stays where it was. What changes is that its result is
    /// now delivered by [`Ball::ballistic_launch_arriving_at`], which
    /// solves the arc that puts the ball on the winner's head at
    /// `arrival_height` **on the way down**, and the outcome is applied
    /// when the ball gets there. See [`AerialDelivery`].
    ///
    /// `behind` is how far short of the winner the ball is aimed, in
    /// units — the two contests use different values and the difference
    /// is load-bearing (a midfielder's heading reach is 2.0u, which the
    /// corner's own 2.0u drop sat exactly on the boundary of).
    pub(in crate::r#match::engine::engine) fn deliver_to_winner(
        field: &mut MatchField,
        winner_idx: usize,
        attacked_goal: Vector3<f32>,
        previous_owner: Option<u32>,
        behind: f32,
        apex: f32,
        intent: DeliveryIntent,
        force_heading: bool,
        source: usize,
    ) {
        // Read only by the arming census.
        #[cfg(not(feature = "match-logs"))]
        let _ = source;
        /// Head height, in metres. One tick above the intercept window,
        /// which is what the corner path's own comment sized it at.
        const HEADING_HEIGHT: f32 = 2.5;
        /// Ticks of slack past the solved flight before the delivery is
        /// abandoned. Half a second: the winner is running while the ball
        /// is in the air, so the arrival test has to tolerate him being a
        /// stride from where the arc was solved to.
        const GRACE_TICKS: u64 = 50;

        let winner_pos = field.players[winner_idx].position;
        let winner_id = field.players[winner_idx].id;
        let to_goal = attacked_goal - winner_pos;
        let dir = if to_goal.magnitude() > 0.01 {
            to_goal.normalize()
        } else {
            Vector3::new(1.0, 0.0, 0.0)
        };
        let target = Vector3::new(
            winner_pos.x - dir.x * behind,
            winner_pos.y - dir.y * behind,
            HEADING_HEIGHT,
        );
        // The calibrated hang, unchanged: −0.02 m/tick walks the ball down
        // through the [1.4, 2.5] heading band over ~40 ticks, and 0.12
        // u/tick of goalward drift keeps it inside the 6u header reach for
        // all of them, so ANY winner's state machine gets a valid tick.
        //
        // Each intent becomes the outcome that will be struck WHERE THE
        // BALL ARRIVES — the geometry of a hook and of a clearance both
        // depend on where he actually meets it, which is not where the
        // contest was decided. See [`DeliveryIntent`].
        let outcome_is_header = matches!(intent, DeliveryIntent::Header);
        let outcome = match intent {
            DeliveryIntent::Header => AerialOutcome::Header {
                drift: Vector3::new(dir.x * 0.12, dir.y * 0.12, -0.02),
            },
            DeliveryIntent::HookedBehind => AerialOutcome::HookedBehind {
                attacked_goal,
                field_height: field.size.height as f32,
            },
            // `clear_apex` and not `apex`: the outer one is the height of
            // the DELIVERY to him, this is the height of the clearance he
            // strikes when it gets there.
            DeliveryIntent::Cleared {
                range,
                apex: clear_apex,
            } => AerialOutcome::Cleared {
                attacked_goal,
                range,
                apex: clear_apex,
            },
        };

        let b = &mut field.ball;
        b.current_owner = None;
        b.previous_owner = previous_owner;
        if outcome_is_header {
            // Every heading state reads this to take a clean-contact roll
            // instead of re-deciding the duel. Set at the strike rather
            // than on arrival because the winner's own state machine uses
            // it to decide to go and attack the ball in the first place.
            b.aerial_contest_winner = Some(winner_id);
        }

        // ⚠ **A ball already in the heading band is not launched again.**
        //
        // `ballistic_launch_arriving_at` solves the arc from where the ball
        // is NOW, and its apex is measured over the BALL rather than over
        // the ground. That is right for a contest resolved AT THE STRIKE —
        // a corner, armed a tick after the taker hit it, climbing through
        // 2 m with the winner twenty-seven metres away, where the solved
        // arc IS the corner and turns the ball 6°. It is wrong for one
        // resolved MID-FLIGHT. `resolve_cross_contest` fires on a ball
        // already descending through [1.5, 2.9] m with the winner inside
        // 4.3 m, and there the same call is a second launch on a ball
        // nobody touched.
        //
        // Measured over 200 matches before this existed, per arming:
        //
        // | source | armed at | to travel | turned | peak |
        // |---|---|---|---|---|
        // | corner won | 2.07 m | 27.3 m | 6° | 7.07 m |
        // | corner behind | 2.07 m | 23.2 m | 3° | 7.07 m |
        // | **open cross** | **2.82 m** | **3.0 m** | **41°, 19% past 90°** | **6.82 m** |
        //
        // Four metres straight up to travel three, 4.45 times a match,
        // with nobody within a stride of it. That is the reported *"the
        // ball bounces off an invisible object above the player"*.
        //
        // The ball is already where the delivery was going to put it, so
        // it keeps its own flight and the delivery is aimed at where that
        // flight actually arrives. `PlayerReach::can_strike` at the
        // arrival is still what makes the winner reach it, and the
        // deadline is still what ends it if he never does.
        //
        // `OF_DELIVERY_RELAUNCH=1` restores the unconditional launch.
        let kept = if MatchContext::delivery_relaunch_flat() {
            None
        } else {
            // Descending, and no higher than a man can head it: this
            // contest was resolved in mid-flight, not at a strike.
            b.natural_drop(HEADING_HEIGHT)
                .filter(|_| b.velocity.z <= 0.0 && b.position.z <= AerialReach::HIGHEST)
        };
        let launch = Ball::ballistic_launch_arriving_at(b.position, target, apex);

        #[cfg(feature = "match-logs")]
        {
            /// Metres per game unit on the horizontal axes — the census
            /// reports distances in metres and these two are in units.
            const M_PER_U: f32 = 0.125;
            /// Under this the ball is not meaningfully travelling, so the
            /// arc it is given cannot be said to have TURNED it.
            /// 0.05 u/tick is 0.6 m/s.
            const ARMED_IN_FLIGHT: f32 = 0.05;

            // What the arming ACTUALLY did, not what the unused arc would
            // have done: a flight that is kept turns the ball by nothing
            // and peaked before it ever got here.
            let flat = Vector3::new(b.velocity.x, b.velocity.y, 0.0);
            let moving = flat.norm() > ARMED_IN_FLIGHT;
            let turn = launch
                .filter(|_| kept.is_none())
                .map(|(v, _)| Vector3::new(v.x, v.y, 0.0))
                .filter(|to| moving && to.norm() > 1.0e-4)
                .map(|to| flat.angle(&to).to_degrees())
                .unwrap_or(0.0);
            let peak = if kept.is_some() {
                b.position.z
            } else {
                b.position.z + apex
            };
            tc::TeleportCensus::note_delivery_arming(
                source,
                moving,
                turn,
                b.position.z,
                peak,
                (target.x - b.position.x).hypot(target.y - b.position.y) * M_PER_U,
                b.natural_drop(HEADING_HEIGHT).map(|(drop, _)| {
                    (drop.x - winner_pos.x).hypot(drop.y - winner_pos.y) * M_PER_U
                }),
                kept.is_some(),
            );
        }

        // The ball's own flight first, the solved arc second. Both arms
        // arm the SAME delivery — what differs is whether the velocity is
        // rewritten and where the aim point is.
        match kept
            .map(|(drop, ticks)| (b.velocity, drop, ticks))
            .or_else(|| launch.map(|(velocity, ticks)| (velocity, target, ticks)))
        {
            Some((velocity, aim, ticks)) => {
                #[cfg(feature = "match-logs")]
                tc::TeleportCensus::note_delivery_armed(ticks);
                b.velocity = velocity;
                // Hold the loose-ball machinery off for the whole flight:
                // `in_flight_state > 0` is what keeps `check_ball_ownership`
                // from handing a travelling delivery to whoever is nearest.
                b.flags.in_flight_state = ticks as usize + GRACE_TICKS as usize;
                b.aerial_delivery = Some(AerialDelivery {
                    winner_id,
                    target: aim,
                    outcome,
                    arrival_height: HEADING_HEIGHT,
                    deadline_tick: b.current_tick_cached + ticks as u64 + GRACE_TICKS,
                    force_heading,
                });
            }
            None => {
                // Neither arm has anything to fly: the ball is not in the
                // band, and it is already standing on the target so there
                // is no arc to solve either. Apply the outcome now; the
                // "relocation" is under a unit.
                // A `Header` outcome is a HOLD in the heading band, over
                // in a tick — its own guard (`aerial_contest_winner`) is
                // what protects it. A `HookedBehind` is a real 5 m arc,
                // and the window has to cover it or the states are sent
                // at a ball still climbing. See
                // [`FlightProtection::for_launch`].
                b.flags.in_flight_state = match outcome {
                    AerialOutcome::Header { drift } => {
                        b.velocity = drift;
                        1
                    }
                    AerialOutcome::HookedBehind {
                        attacked_goal,
                        field_height,
                    } => {
                        b.velocity =
                            Ball::hook_behind_velocity(b.position, attacked_goal, field_height);
                        FlightProtection::for_launch(b.velocity, b.position.z)
                    }
                    // …and so is a headed clear: a 6 m arc that has to
                    // travel twenty metres, which nothing may claim off
                    // him while it is still going up.
                    AerialOutcome::Cleared {
                        attacked_goal,
                        range,
                        apex: clear_apex,
                    } => {
                        b.velocity = Ball::headed_clear_velocity(
                            b.position,
                            attacked_goal,
                            range,
                            clear_apex,
                        );
                        FlightProtection::for_launch(b.velocity, b.position.z)
                    }
                };
                // There is no delivery to carry the heading transition, so
                // it is stashed straight away — the arrival is now.
                if force_heading {
                    b.pending_aerial_strike = Some(winner_id);
                }
            }
        }
    }

    pub(in crate::r#match::engine::engine) fn hook_it_behind(
        field: &mut MatchField,
        from: Vector3<f32>,
        attacked_goal: Vector3<f32>,
    ) {
        #[cfg(feature = "match-logs")]
        crate::mid_run_diag::HEADED_BEHIND_FIRED.fetch_add(1, Ordering::Relaxed);
        let field_height = field.size.height as f32;
        // The geometry lives on `Ball` so this and the arrival of an
        // `AerialDelivery` that resolved to `HookedBehind` strike the
        // same clearance. See `Ball::hook_behind_velocity`.
        let velocity = Ball::hook_behind_velocity(from, attacked_goal, field_height);

        let b = &mut field.ball;
        // ⚠ NO POSITION WRITE. The only caller left passes the BALL's own
        // position as `from` (`resolve_cross_contest`'s cleared branch),
        // so the header happens where the ball is. The corner contest used
        // to pass the CLEARER's position with the ball still at the flag,
        // which wrote it the width of the box in one tick; that path now
        // flies the delivery to him first.
        b.velocity = velocity;
        b.current_owner = None;
        // A hooked header goes up 5 m and hangs. `1` said it was
        // back in play on the next tick, so the states asking
        // `!is_in_flight()` were sent at a ball still climbing —
        // see [`FlightProtection::for_launch`].
        b.flags.in_flight_state = FlightProtection::for_launch(velocity, b.position.z);
        b.pass_target_player_id = None;
        b.clear_pending_pass_metadata();
    }

    pub(in crate::r#match::engine::engine) fn heads_it_behind(
        ball_pos: Vector3<f32>,
        attacked_goal: Vector3<f32>,
        field_width: f32,
        context: &mut MatchContext,
    ) -> bool {
        /// Outside this there is always a way out. 150u ≈ 18.75 m.
        const BEHIND_DEPTH: f32 = 150.0;

        // ⚠ The curve was re-shaped 2026-08-31 against the measured
        // resolve-depth distribution, exactly as the old warning here
        // demanded. The previous form (`0.55 · urgency²` over a 130u
        // window) was quadratic in a variable that is ~0.0-0.26 where
        // contests actually resolve (12-16 m out — the drop zones of
        // floated and whipped deliveries), so it returned **~1%** and
        // the corner-source census had "delivery HOOKED behind" at
        // 0.45/match against the real ~3.5-4 of a ~10.4-corner match.
        // The defender who wins that header is facing his own goal
        // with the ball whipped across him — heading it behind is the
        // NORMAL outcome under pressure at that depth, not a goal-line
        // desperation. The gentler exponent puts ~14% at 12 m and ~6%
        // at 16 m, which at the post-ordering-fix ~25-30 headed clears
        // a match prices the hooked family at its real share.
        // `OF_BEHIND_LINE` overrides the at-line share for titration.
        let depth = (ball_pos.x - attacked_goal.x).abs();
        if depth > BEHIND_DEPTH || field_width <= 0.0 {
            return false;
        }
        // 1.0 on the goal line, 0 at the edge of the window.
        let urgency = 1.0 - depth / BEHIND_DEPTH;
        context
            .rng
            .bernoulli(Self::behind_at_line() * urgency.powf(1.2))
    }

    /// Share of headed clears that go behind when the header is right
    /// on the goal line. `OF_BEHIND_LINE` overrides for titration.
    fn behind_at_line() -> f32 {
        use std::sync::OnceLock;
        static V: OnceLock<f32> = OnceLock::new();
        *V.get_or_init(|| {
            std::env::var("OF_BEHIND_LINE")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(0.50)
        })
    }
}
