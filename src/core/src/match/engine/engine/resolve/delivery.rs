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
use crate::r#match::engine::ball::ball::{AerialDelivery, AerialOutcome};
use crate::r#match::engine::engine::*;
use nalgebra::Vector3;
#[cfg(feature = "match-logs")]
use std::sync::atomic::Ordering;

impl<const W: usize, const H: usize> FootballEngine<W, H> {
    /// Apex of a corner delivery, in metres. A normal in-swinger: 5 m up
    /// puts about 1.7 s between the strike and the header, which is what
    /// a real one takes and comfortably inside
    /// [`CORNER_SHAPE_MAX_TICKS`](Self::CORNER_SHAPE_MAX_TICKS) so the
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
        outcome_is_header: bool,
        force_heading: bool,
    ) {
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
        // through the [1.4, 2.5] heading band over ~40 ticks and 0.12
        // u/tick of goalward drift keeps it inside the 6u header reach for
        // all of them, so ANY winner's state machine gets a valid tick.
        let outcome = if outcome_is_header {
            AerialOutcome::Header {
                drift: Vector3::new(dir.x * 0.12, dir.y * 0.12, -0.02),
            }
        } else {
            AerialOutcome::HookedBehind {
                attacked_goal,
                field_height: field.size.height as f32,
            }
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

        match Ball::ballistic_launch_arriving_at(b.position, target, apex) {
            Some((velocity, ticks)) => {
                #[cfg(feature = "match-logs")]
                tc::TeleportCensus::note_delivery_armed(ticks);
                b.velocity = velocity;
                // Hold the loose-ball machinery off for the whole flight:
                // `in_flight_state > 0` is what keeps `check_ball_ownership`
                // from handing a travelling delivery to whoever is nearest.
                b.flags.in_flight_state = ticks as usize + GRACE_TICKS as usize;
                b.aerial_delivery = Some(AerialDelivery {
                    winner_id,
                    target,
                    outcome,
                    arrival_height: HEADING_HEIGHT,
                    deadline_tick: b.current_tick_cached + ticks as u64 + GRACE_TICKS,
                    force_heading,
                });
            }
            None => {
                // The ball is already standing on the target — there is no
                // arc to solve and nothing to fly. Apply the outcome now;
                // the "relocation" is under a unit.
                b.velocity = match outcome {
                    AerialOutcome::Header { drift } => drift,
                    AerialOutcome::HookedBehind {
                        attacked_goal,
                        field_height,
                    } => Ball::hook_behind_velocity(b.position, attacked_goal, field_height),
                };
                b.flags.in_flight_state = 1;
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
        b.flags.in_flight_state = 1;
        b.pass_target_player_id = None;
        b.clear_pending_pass_metadata();
    }

    pub(in crate::r#match::engine::engine) fn heads_it_behind(
        ball_pos: Vector3<f32>,
        attacked_goal: Vector3<f32>,
        field_width: f32,
        context: &mut MatchContext,
    ) -> bool {
        /// Outside this there is always a way out. 130u ≈ 16 m.
        const BEHIND_DEPTH: f32 = 130.0;
        /// Share that goes behind when the header is right on the line.
        const BEHIND_AT_LINE: f32 = 0.55;

        let depth = (ball_pos.x - attacked_goal.x).abs();
        if depth > BEHIND_DEPTH || field_width <= 0.0 {
            return false;
        }
        // 1.0 on the goal line, 0 at the edge of the window.
        let urgency = 1.0 - depth / BEHIND_DEPTH;
        context.rng.bernoulli(BEHIND_AT_LINE * urgency * urgency)
    }
}
