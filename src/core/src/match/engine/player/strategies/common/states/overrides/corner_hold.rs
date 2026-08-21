//! The rule that keeps a corner looking like a corner: **stand where you
//! were put until the ball comes near you.**
//!
//! # Why this is a cross-state rule and not a state
//!
//! `CornerShape` places all twenty non-takers when a corner is awarded,
//! and every one of them is somewhere his own state machine did not
//! choose to be. Left alone they walk straight back out: a midfielder
//! dropped into his own six-yard box reads the next tick as ordinary open
//! play and heads for the space he was making before the ball went
//! behind, and a forward stationed on the penalty spot is already
//! running the channel again. At ~0.6 u per tick that is eight metres a
//! second — enough to empty the box inside the corner's own lifetime.
//!
//! Making it a state instead would mean a `DefendingCorner` variant in
//! three separate role enums, three handler modules, and a decision about
//! what each one does when the ball arrives — re-implementing marking,
//! clearing, heading and interception inside the new state, because those
//! are exactly the behaviours a defender in his own box needs. The states
//! the players are already in do all of that correctly. What they get
//! wrong is only *where they stand*, so that is the only thing overridden
//! here. Same shape as [`DefensiveRecovery::depth_override`], and for the
//! same reason.
//!
//! # Releasing
//!
//! The hold is not a cage. It fades continuously as the ball approaches,
//! reaching zero well outside heading range, so the man the delivery is
//! dropping onto goes and attacks it while the rest of the box holds its
//! shape. A player on the ball, or mid-way through an action a real
//! footballer cannot abort, is exempt outright.
//!
//! [`DefensiveRecovery::depth_override`]: crate::r#match::defenders::states::common::DefensiveRecovery

use crate::r#match::defenders::states::DefenderState;
use crate::r#match::forwarders::states::ForwardState;
use crate::r#match::goalkeepers::states::state::GoalkeeperState;
use crate::r#match::midfielders::states::MidfielderState;
use crate::r#match::player::state::PlayerState;
use crate::r#match::{
    GameTickContext, MatchPlayer, PassOriginRestart, StateProcessingResult, SteeringBehavior,
};
use nalgebra::Vector3;

/// Distance from the ball (field units, 1 u = 0.125 m) at which a player
/// is fully released from his station — 4 m, comfortably outside the
/// 6 u header reach, so nobody is still being pulled backwards while the
/// ball drops on his head.
const RELEASE_FULL: f32 = 32.0;
/// Distance at which the hold is at full strength again. Between the two
/// it eases, so there is no radius at which a player's velocity flips.
const RELEASE_NONE: f32 = 96.0;
/// How hard a player brakes onto his station. Generous: the point is to
/// stop drifting, not to snap to a coordinate.
const STATION_SLOWING: f32 = 10.0;

/// The corner set-piece position hold.
pub struct CornerHold;

impl CornerHold {
    /// Blend the player's own velocity toward his corner station.
    ///
    /// No-op — one `Option` read — whenever no corner is being taken,
    /// which is all but a couple of seconds in twenty of a match.
    pub fn apply(
        player: &MatchPlayer,
        tick_context: &GameTickContext,
        result: &mut StateProcessingResult,
    ) {
        let Some(station) = player.set_piece_station else {
            return;
        };
        // Belt and braces. `clear_expired_corner_stations` takes the
        // stations off at first contact, so this rarely decides anything —
        // but it is a one-byte read and it means the hold cannot outlive a
        // corner even if the clear is ever missed on some path.
        if tick_context.ball.pass_origin_restart != PassOriginRestart::Corner {
            return;
        }
        let weight = Self::hold_weight(player, tick_context);
        if weight <= 0.0 {
            return;
        }

        let station_velocity = SteeringBehavior::Arrive {
            target: station,
            slowing_distance: STATION_SLOWING,
        }
        .calculate(player)
        .velocity;

        let own = result.velocity.unwrap_or_else(Vector3::zeros);
        result.velocity = Some(own * (1.0 - weight) + station_velocity * weight);
        // ⚠ **A velocity imposed from outside the state machine has to
        // carry its own effort, or it is served at the pace of a state
        // nobody is following any more.**
        //
        // Same mechanism and the same reason as `ShapeDiscipline`'s
        // `shape_recall_pull` and `KeeperReleaseSpace`'s floor: the speed
        // cap in `state.rs` is keyed to the `ActivityIntensity` the
        // player's CURRENT state declared, and the states a man drifts in
        // while a corner is being set up are the low ones — `Standing` is
        // `Recovery`, 0.12 of top speed. So the twenty were steered to
        // their stations at a twelfth of walking pace.
        //
        // That is why the walked corner's box would not fill. Measured,
        // `OF_CORNER_WALK=on` over 60 matches at level 14 with the taker
        // already waiting on the arc: **3.3 attackers in the box at the
        // delivery against a placed corner's 5.4** and a real 5-7, with
        // over half the set-ups released by the ceiling rather than by
        // the box arriving. They were not short of time; they were
        // forbidden to run.
        //
        // Floored at `weight` exactly, which is the share of the final
        // velocity the station owns — the same "permit this component and
        // nothing more" rule the other two overrides use. `Arrive` still
        // decelerates them into the station, so nobody overshoots it.
        result.effort_floor = result.effort_floor.max(weight);
    }

    /// How much of the player's movement the station owns this tick,
    /// 0 (his own state decides) to 1 (hold position).
    fn hold_weight(player: &MatchPlayer, tick_context: &GameTickContext) -> f32 {
        // ── the corner has not been TAKEN yet ─────────────────────────
        //
        // The ball is lying on the byline where it went out, or moving up
        // it under the taker's feet, and everybody else is walking into
        // the shape. Nothing below applies to that: every release in this
        // function is about a DELIVERY — the man it is dropping on plays
        // it, the rest hold — and there is no delivery until the ball is
        // on the arc.
        //
        // Both releases would otherwise fire on exactly the wrong people.
        // The distance fade is measured from the ball, which spends the
        // set-up on the byline a few metres from the near-post and short
        // stations, so the players nearest the goal — the ones the shape
        // exists to place — would be the ones let go. And the carrier is
        // in `TakeBall` with the ball at his feet, so the chaser exemption
        // AND the fade both zero him: his station is the only thing that
        // can walk him to the flag, and it is refused unless this comes
        // first. See `AwaitedRestart::carrying`.
        if tick_context.ball.restart_taker.is_some() {
            // …with one exception, and it is the deadlock if it is missed:
            // the taker on his way to FETCH the ball has no business
            // standing anywhere. He gets no station until he picks it up.
            let fetching = tick_context.ball.restart_taker == Some(player.id)
                && tick_context.ball.restart_carrier != Some(player.id);
            return if fetching { 0.0 } else { 1.0 };
        }

        // On the ball, or committed to something that cannot be aborted
        // — a shot, a tackle, a header already begun. Never overridden.
        if tick_context.ball.current_owner == Some(player.id) || player.state.is_committed_action()
        {
            return 0.0;
        }
        // ⚠ THE MAN GOING FOR THE BALL IS NEVER HELD, however far away it
        // is. He is either the delivery's intended receiver — the short
        // corner is played to a man standing thirty metres from the box —
        // or the loose-ball override has designated him his side's chaser
        // and put him in `TakeBall`. Holding either of them is a deadlock:
        // the corner's restart origin only decays when somebody *touches*
        // the ball, so pinning the only two players allowed to go to it
        // means the pin never lifts. That is a real measurement, not a
        // hypothetical — see `clear_expired_corner_stations`.
        if tick_context.ball.pass_target == Some(player.id) || Self::is_chasing(player) {
            return 0.0;
        }
        // Anyone the delivery is actually coming to plays it. Smoothstep
        // rather than a radius test: a hard boundary here is a velocity
        // inversion (the station is usually on the far side of the player
        // from the ball), which is the exact failure the corner
        // centre-back's own attack/hold blend was rewritten to avoid.
        let ball_distance = (tick_context.positions.ball.position - player.position).magnitude();
        let t = ((ball_distance - RELEASE_FULL) / (RELEASE_NONE - RELEASE_FULL)).clamp(0.0, 1.0);
        t * t * (3.0 - 2.0 * t)
    }

    /// Has the engine designated this player the man who goes and gets it?
    /// `TakeBall` is where the universal loose-ball override puts him, in
    /// all four role machines.
    fn is_chasing(player: &MatchPlayer) -> bool {
        matches!(
            player.state,
            PlayerState::Defender(DefenderState::TakeBall)
                | PlayerState::Midfielder(MidfielderState::TakeBall)
                | PlayerState::Forward(ForwardState::TakeBall)
                | PlayerState::Goalkeeper(GoalkeeperState::TakeBall)
        )
    }
}
