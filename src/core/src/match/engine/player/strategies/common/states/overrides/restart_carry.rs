//! **The man with the ball under his arm, walking back to the line.**
//!
//! # Why it needs its own rule
//!
//! A restart now has two legs. The taker goes out into the run-off and
//! fetches the ball from wherever the hoardings stopped it, and then he
//! brings it back to the point the throw or the kick is legally taken
//! from ([`AwaitedRestart::take_from`]). The engine has a signal for the
//! first leg — `TakeMe` puts him in `TakeBall` and `run_for_ball` steers
//! him — and it has **nothing at all** for the second, because the ball is
//! already at his feet: every chase behaviour in the engine reads a ball
//! it is standing on as reached, so he stops dead the moment he picks it
//! up and the restart sits there until the patience bound teleports it.
//!
//! [`CornerHold`](super::CornerHold) solved this once, for the corner, and
//! its solution is the right one: the engine writes the taker a
//! `set_piece_station` and something outside the four state machines walks
//! him to it. But that module is bounded to `PassOriginRestart::Corner` on
//! purpose — the twenty-man shape it also holds only exists for a corner —
//! so a throw-in's or a goal kick's carrier fell through it.
//!
//! This is the carrier's half, lifted out and made origin-blind. It is the
//! only piece of the corner's walk that a throw-in needs, and it is
//! exactly the piece it was missing.
//!
//! # Why it overrides outright
//!
//! There is no blend and no fade. A man carrying a dead ball to the spot
//! is not doing anything else — he is not marking, not making a run, not
//! contesting — and every one of those behaviours would pull the ball
//! along with him, because the ball rides on his position while
//! [`AwaitedRestart::carrying`] is up. The whole point of the leg is that
//! the ball travels *at his pace, in his hands*, to one specific point.
//!
//! [`AwaitedRestart::take_from`]: crate::r#match::engine::ball::ball::AwaitedRestart::take_from
//! [`AwaitedRestart::carrying`]: crate::r#match::engine::ball::ball::AwaitedRestart::carrying

use crate::r#match::{GameTickContext, MatchPlayer, StateProcessingResult, SteeringBehavior};

/// Steers the taker of a restart while he is carrying the ball to the spot.
pub struct RestartCarry;

impl RestartCarry {
    /// How hard he brakes onto the spot. Same value `CornerHold` uses for
    /// the same job, and generous for the same reason: he has to stop
    /// beside a point, not land on a coordinate.
    const SLOWING: f32 = 10.0;

    /// Walk the carrier to his station.
    ///
    /// No-op — two `Option` reads — for everybody else, on every tick of
    /// the match. Applied at dispatch rather than inside a state because
    /// none of the four state machines has a "carrying a dead ball"
    /// concept and giving all four one would be four copies of this.
    pub fn apply(
        player: &MatchPlayer,
        tick_context: &GameTickContext,
        result: &mut StateProcessingResult,
    ) {
        if tick_context.ball.restart_carrier != Some(player.id) {
            return;
        }
        let Some(station) = player.set_piece_station else {
            return;
        };
        result.velocity = Some(
            SteeringBehavior::Arrive {
                target: station,
                slowing_distance: Self::SLOWING,
            }
            .calculate(player)
            .velocity,
        );
    }
}
