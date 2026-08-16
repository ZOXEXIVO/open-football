//! Positional discipline — the elastic tether that keeps a player inside
//! the space his team has given him.
//!
//! # Why this is a movement-layer rule and not per-state logic
//!
//! [`TeamShape`](crate::r#match::TeamShape) hands every player a live
//! anchor, and the resting states (`Walking`, `Standing`, `Returning`)
//! steer at it. Rewiring those states moved the measured block length by
//! 1.4 m, because **players are almost never in them**. The state census
//! (`dev_match stats`, level 14, twelve matches) says where the match
//! actually goes:
//!
//! | state | share of AI ticks | mean anchor lag |
//! |---|---|---|
//! | Defender: Marking | 14.2% | 15.4 m |
//! | Midfielder: Supporting Attack | 12.8% | 16.2 m |
//! | Defender: Running | 9.3% | 15.5 m |
//! | Midfielder: Running | 7.9% | 15.4 m |
//! | Midfielder: Returning | 7.4% | 23.9 m |
//! | Forward: Creating Space | 6.1% | 17.0 m |
//! | Forward: Running | 4.6% | 28.8 m |
//!
//! Six states own more than half the match, each one is thousands of
//! lines of ball-relative reasoning, and the mean player sits 16-20 m
//! from the place his team wants him **at all times** — which is the
//! 85 m block, restated per player.
//!
//! Editing twenty state machines to each remember the shape is how the
//! engine got here: every one of them re-derives a destination from the
//! same global geometry, and they cannot see each other. The rule belongs
//! where all movement already converges — one place, applied once.
//!
//! # The rule
//!
//! A player has **freedom inside his zone and a leash outside it.** Up to
//! [`SLACK`] from his anchor his own state decides entirely; past that a
//! recall component fades in, and by [`SLACK`] + [`RANGE`] the shape has
//! the casting vote. That is what a positional game actually is: the
//! coach gives you a space, you do what you like in it, and if you leave
//! it somebody has to be told to cover.
//!
//! # What is exempt
//!
//! Everything to do with playing the ball. You break shape to make a
//! tackle, to press the carrier, to receive a pass, to finish a chance —
//! that is football, and a tether that fought it would produce eleven
//! players politely holding a formation while the opposition walked
//! through them. The exemptions are listed in
//! [`ShapeDiscipline::is_exempt`] and each one is a case where the ball,
//! not the shape, is the player's job this instant.

use crate::r#match::player::state::PlayerState;
use crate::r#match::{MatchContext, StateProcessingContext};
use nalgebra::Vector3;

/// How far a player may stray from his anchor before the shape says
/// anything at all. 64u = 8 m — a real positional zone: a player covers
/// roughly that radius on his own before a team-mate is expected to
/// shuffle across for him.
///
/// Calibrated, not picked. The first draft used 120u (15 m), which is
/// what a zone *feels* like — and measured almost nothing, because the
/// mean anchor lag was 15-17 m, so the leash sat exactly at the average
/// player's existing error and engaged for nobody. A tether whose slack
/// equals the drift it is meant to correct is a no-op.
const SLACK: f32 = 48.0;

/// Over how much further the recall fades from nothing to full. At
/// `SLACK + RANGE` = 128u = 16 m the player is out of his zone by more
/// than twice its radius and the shape wins outright.
///
/// The curve has to be steep as well as early. A shallow one settles at
/// an equilibrium rather than pulling players home: the recall grows with
/// distance, the state's outward intent does not, so they balance at
/// whatever lag makes the two equal. A first pass at (64u, 120u) parked
/// the mean lag at 12 m for exactly that reason.
const RANGE: f32 = 80.0;

/// Ceiling on the recall's share of the final velocity. Never 1.0: even
/// a badly out-of-position player keeps a fraction of his own intent, so
/// a defender sprinting back still drifts toward the man he was watching
/// instead of running a dead-straight line to a coordinate.
///
/// A softer setting was measured — (72u slack, 0.55 pull) — on the theory
/// that a gentler leash would keep most of the shape while adding less
/// duel density. It does not: the observed block went 54 m → 67 m, giving
/// back nearly all the compaction, and bought only ~0.3 goals a match
/// back. The tether is close to all-or-nothing, so it is set to work.
const MAX_PULL: f32 = 0.85;

/// Within this distance of the ball a player is *in the play*, and the
/// shape stops having opinions. 80u = 10 m — the range over which a
/// player can realistically affect the ball in the next second or two.
const BALL_ENGAGEMENT: f32 = 80.0;

pub struct ShapeDiscipline;

impl ShapeDiscipline {
    /// Blend the state's own velocity with a pull toward the team anchor,
    /// according to how far outside his zone the player has drifted.
    /// Returns the velocity unchanged whenever the ball is this player's
    /// job.
    pub fn apply(ctx: &StateProcessingContext, velocity: Vector3<f32>) -> Vector3<f32> {
        Self::apply_with_pull(ctx, velocity).0
    }

    /// As [`Self::apply`], but also reports how hard the recall is pulling
    /// so the caller can stop the movement-effort cap from throttling a
    /// recovery run — see `StateProcessingResult::shape_recall_pull`.
    pub fn apply_with_pull(
        ctx: &StateProcessingContext,
        velocity: Vector3<f32>,
    ) -> (Vector3<f32>, f32) {
        // A/B control for the positional layer — see
        // `MatchContext::shape_off`.
        if MatchContext::shape_off() || Self::is_exempt(ctx) {
            return (velocity, 0.0);
        }

        let anchor = ctx.team().my_anchor();
        let to_anchor = anchor - ctx.player.position;
        let lag = to_anchor.magnitude();
        if lag <= SLACK {
            return (velocity, 0.0);
        }

        let pull = (((lag - SLACK) / RANGE).clamp(0.0, 1.0)) * MAX_PULL;

        // The recall runs at the player's own top speed, so a man 30 m out
        // of shape actually recovers rather than ambling back — the
        // recovery run is the thing being modelled here.
        //
        // ⚠ That intent was defeated downstream for as long as this layer
        // has existed. `state.rs` caps the finished velocity at
        // `max_speed * MovementEffort::speed_fraction(state_intensity)`,
        // and a player drifting out of shape is by definition NOT in a
        // high-effort state — `Standing` declares `Recovery` (0.12),
        // `Returning` and `CreatingSpace` declare `Moderate` (0.52). So
        // the recall was computed at 1.0 and then served at 0.12-0.52.
        // Reporting `pull` lets the cap floor itself at the recall's own
        // demand, which is the honest reading: being this far out of
        // position IS the effort, whatever the state was calling itself.
        let recall = (to_anchor / lag) * ctx.player.max_speed_with_condition_cached();

        (velocity * (1.0 - pull) + recall * pull, pull)
    }

    /// Is the ball this player's job right now? Then the shape is not.
    fn is_exempt(ctx: &StateProcessingContext) -> bool {
        // The keeper's position is his own state machine's business
        // entirely — he is not part of the block.
        if ctx
            .player
            .tactical_position
            .current_position
            .is_goalkeeper()
        {
            return true;
        }

        // On the ball, or mid-way through an action that cannot be
        // abandoned (a shot, a header, a tackle, a dive).
        if ctx.player.has_ball(ctx) || ctx.player.state.is_committed_action() {
            return true;
        }

        // Injured players are not doing positional play.
        if matches!(ctx.player.state, PlayerState::Injured) {
            return true;
        }

        // In the immediate vicinity of the ball. Includes every chase,
        // press, interception and reception without needing to enumerate
        // the states that do them.
        if ctx.ball().distance() < BALL_ENGAGEMENT {
            return true;
        }

        // A pass is on its way to him — he must go and meet it wherever
        // it has been played, which is by design AHEAD of where he is.
        if ctx.tick_context.ball.pass_target == Some(ctx.player.id) {
            return true;
        }

        // ⚠ NOT EXEMPT: a player holding an individual defensive duty.
        //
        // Tried 2026-08-17 and REVERTED, because the reasoning was good
        // and the measurement disagreed. The argument: every exemption
        // above says "when something else is definitely this player's
        // job, the shape stops having opinions", and an assignment from
        // `DefensivePlan` is exactly that — a marker tracking a midfielder
        // eight metres in front of the back four is by definition outside
        // `SLACK` with the ball outside `BALL_ENGAGEMENT`, so up to 85% of
        // his velocity is replaced by a run back to his formation slot.
        //
        // Measured, exempting him made marking WORSE, not better: the duel
        // gap went 5.2 m → 5.6 m and against midfielders 7.1 m → 7.8 m.
        // Freed from the tether the marker does not close on his man, he
        // drifts — his own steering already has a lag, and the recall was
        // incidentally holding him inside a zone the man kept re-entering.
        // So this layer is NOT what keeps markers off their men; see the
        // note at `DefensiveLine::hold_shape_on_man` for what was.
        false
    }
}
