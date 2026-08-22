//! The forty-five to seventy-five seconds between the ball hitting the net
//! and the referee's whistle for the restart.
//!
//! # Why this exists at all
//!
//! The post-goal window was a hole in the simulation. `handle_goal_reset`
//! teleported all twenty-two players back into formation and the ball onto
//! the centre spot on the very tick the goal went in, then the engine loop
//! skipped every tick until
//! [`MatchContext::dead_ball_until_ms`](super::context::MatchContext::dead_ball_until_ms) — no
//! physics, no movement, and, crucially, **no position samples**. A replay
//! therefore showed the last frame before the goal held on screen for a
//! minute: the ball frozen an inch short of the line, twenty-two players
//! standing where they happened to be. The most dramatic thing that happens
//! in a football match was the one thing the recording had nothing to say
//! about.
//!
//! # What it does instead
//!
//! The window is now played out. The ball stays in the net (see
//! [`net`](crate::r#match::engine::ball::ball::net)), the scorer wheels away,
//! his team-mates chase him down, the conceding side trudges back, and
//! somebody goes and fetches the ball out of the goal. Only at the END of the
//! window does the field snap into the kickoff set-up — the same set-up, at
//! the same instant, that play used to resume from.
//!
//! # Why it is not the state machine
//!
//! Deliberately choreographed rather than expressed as player states, for the
//! same reason the corner dead-ball set-up teleports its centre-backs: this is
//! a cutscene, not a contest. No decision here can affect the match, so
//! running the AI through it would spend real CPU on twenty-two players
//! deciding how to press a ball that is in the back of the net — and would
//! risk moving numbers that took a lot of work to calibrate.
//!
//! That constraint is load-bearing and worth stating outright: **nothing in
//! this module may draw from [`MatchContext::rng`](super::context::MatchContext::rng),
//! emit an event, run a
//! state machine, or touch a statistic.** The RNG stream is shared with every
//! calibrated roll in the engine, so a single extra draw here would shift
//! every subsequent decision in the match. Variation comes from player ids
//! instead — deterministic, free, and outside the stream.

pub mod cast;
pub mod choreography;
pub mod movement;

pub use choreography::GoalCelebration;
