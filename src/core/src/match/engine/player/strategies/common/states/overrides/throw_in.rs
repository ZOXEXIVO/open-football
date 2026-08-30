//! **The man on the touchline with the ball in his hands.**
//!
//! # What was there before
//!
//! Nothing. A throw-in resolved to `current_owner = the taker` and
//! `pass_origin_restart = ThrowIn`, and from that tick on he was an
//! ordinary carrier under his ordinary state machine — so the throw was
//! whatever a defender standing on the touchline happens to do with a
//! ball at his feet. Measured over 60 matches at level 14, before this
//! module existed:
//!
//! ```text
//!   163.9 throw-ins taken per match
//!     of those, CARRIED IN — never thrown at all      99.7%
//!     actually thrown                                  0.3%
//!   of the throws that were thrown:
//!     the THROWER was the first to touch it again     100%
//! ```
//!
//! Which is the report, twice over: *"a player throws the ball in to
//! himself"*. He does — and on the other 99.7% he does not throw it at
//! all, he picks it up on the line, walks in and dribbles away, and the
//! ball goes back out of play often enough that the match ran at 164
//! throw-ins against a real 40-50.
//!
//! # What a throw-in is
//!
//! Law 15, in the three parts that reach the pitch:
//!
//! * it is **thrown**, not carried — the ball leaves his hands, and it
//!   leaves them within a few seconds of him picking it up;
//! * it goes to a **team-mate who is free**, because a throw is the one
//!   delivery in football with no pace behind it and no angle on it, and
//!   a marked man receiving one is a turnover;
//! * and **he may not touch it again** until somebody else has. That
//!   half lives on the ball ([`Ball::throw_in_taker`]), because it has to
//!   bind the claim paths as well as the chase.
//!
//! # Why an override rather than a state
//!
//! For the same reason [`RestartCarry`](super::RestartCarry) is one. Four
//! state machines would each need a "standing on the line with the ball
//! in my hands" concept, and all four would need it to beat everything
//! else they do — a thrower is not marking, not making a run, not
//! dribbling, and any of those behaviours would walk the ball away from
//! the line with him, because the ball rides on his position while he
//! holds it.
//!
//! So this runs at dispatch, ahead of the handler, and when it fires it
//! is the whole of that player's tick.
//!
//! [`Ball::throw_in_taker`]: crate::r#match::engine::ball::ball::Ball::throw_in_taker

use crate::r#match::engine::ball::ball::ThrowIn;
use crate::r#match::events::Event;
use crate::r#match::player::events::PlayerEvent;
use crate::r#match::player::events::models::PassingEventContext;
use crate::r#match::{MatchPlayerLite, StateProcessingContext};
use nalgebra::Vector3;

/// The throw-in delivery: who he throws it to, and when he lets go.
pub struct ThrowInDelivery;

impl ThrowInDelivery {
    /// How long he looks up before letting go, in engine ticks. 40 =
    /// 0.4 s.
    ///
    /// Not decoration. The ball is already dead for the walk to the line
    /// ([`AwaitedRestart`](crate::r#match::engine::ball::ball::AwaitedRestart)),
    /// and this is the pause between arriving and throwing that every
    /// real throw-in has: it is what gives the team-mates one beat to
    /// come short or spin off, which is the only way "throw it to
    /// somebody free" can ever have anybody free to throw it to.
    const SCAN_TICKS: u64 = 40;

    /// …and the longest he holds on to it waiting for one, in ticks.
    /// 350 = 3.5 s.
    ///
    /// Past this he throws it to the best man he has, marked or not.
    /// Football does the same thing — the referee's patience is finite
    /// and so is the crowd's — and it is also what stops a throw taken
    /// against an eleven that has the whole line covered from stalling
    /// the match. There is no third option: he may not carry it in.
    const PATIENCE_TICKS: u64 = 350;

    /// A team-mate with an opponent closer to him than this is marked,
    /// and is not thrown to while there is any alternative. 24 u = 3 m —
    /// close enough that the defender gets to the ball at the same time
    /// it does.
    const MARKED_RADIUS: f32 = 24.0;

    /// …and the distance past which the openness score stops improving.
    /// 72 u = 9 m: a man with nine metres on his marker is as free as
    /// the throw can use.
    const CLEAR_RADIUS: f32 = 72.0;

    /// How close an opponent has to get to the throwing lane before the
    /// throw is treated as blocked, in units. 20 u = 2.5 m — an arm and
    /// a step, which is what it takes to cut out a ball at throwing pace.
    const LANE_WIDTH: f32 = 20.0;

    /// The share of his maximum range a comfortable throw is. A thrower
    /// aims at somebody he can reach easily, not at the far end of what
    /// he is physically capable of.
    const COMFORTABLE: f32 = 0.55;

    /// False when `OF_THROW_IN=off` — the throw-in reverts to what it
    /// was: an ordinary carrier with an ordinary state machine, which is
    /// the arm the measurement above was taken on.
    pub fn armed() -> bool {
        static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
        *ON.get_or_init(|| {
            std::env::var("OF_THROW_IN")
                .map(|v| v != "off" && v != "0")
                .unwrap_or(true)
        })
    }

    /// True while this player is the one taking a throw-in and still has
    /// the ball in his hands.
    ///
    /// Two reads and no scan for everybody else, on every tick — the same
    /// shape as [`RestartCarry`](super::RestartCarry), and for the same
    /// reason: it is checked once per player per tick at dispatch.
    #[inline]
    pub fn taking(ctx: &StateProcessingContext) -> bool {
        Self::armed()
            && ctx.tick_context.ball.throw_taker == Some(ctx.player.id)
            && ctx.tick_context.ball.current_owner == Some(ctx.player.id)
    }

    /// The throw, if he is ready to let go of it.
    ///
    /// `None` means he is still holding it — either the scan has not run
    /// yet, or nobody is free and his patience has not expired. Both are
    /// a man standing over the ball on the touchline, which is what a
    /// throw-in looks like for the second before it is taken.
    pub fn deliver(ctx: &StateProcessingContext) -> Option<Event> {
        let held = ctx.tick_context.ball.ownership_duration as u64;
        if held < Self::SCAN_TICKS {
            return None;
        }
        let out_of_patience = held >= Self::PATIENCE_TICKS;
        let picked = Self::target(ctx, out_of_patience);
        #[cfg(feature = "match-logs")]
        crate::mid_run_diag::RestartCensus::note_throw_scan(picked.is_some());
        let target = picked?;
        Some(Event::PlayerEvent(PlayerEvent::PassTo(
            PassingEventContext::new()
                .with_from_player_id(ctx.player.id)
                .with_to_player_id(target.id)
                .with_reason("THROW_IN")
                .build(ctx),
        )))
    }

    /// **Who he throws it to.**
    ///
    /// Every team-mate his arms can actually reach ([`ThrowIn::range`],
    /// which is where the `long_throws` attribute buys its ground), scored
    /// on the four things a man holding a ball over his head can see:
    /// how much room the receiver has, whether there is anybody standing
    /// in the way of the ball, whether it is a comfortable throw or a
    /// maximal one, and whether it goes forward.
    ///
    /// `desperate` drops the marked-man gate, and nothing else — a
    /// thrower out of patience still throws to the best of what he has
    /// rather than to a random man.
    fn target(ctx: &StateProcessingContext, desperate: bool) -> Option<MatchPlayerLite> {
        let (min_range, max_range) = ThrowIn::range(ctx.player.skills.technical.long_throws);
        let from = ctx.player.position;
        let goal = ctx.player().opponent_goal_position();
        let forward = (goal.x - from.x).signum();

        let mut best: Option<(MatchPlayerLite, f32)> = None;
        for mate in ctx.players().teammates().nearby(max_range) {
            let to = mate.position - from;
            let distance = to.norm();
            if distance < min_range {
                continue;
            }

            // How much room he has. Read off the nearest opponent to HIM,
            // not to the ball: a throw-in is contested where it lands.
            let marker = ctx
                .tick_context
                .grid
                .opponents(mate.id, Self::CLEAR_RADIUS)
                .map(|(_, d)| d)
                .fold(f32::INFINITY, f32::min);
            if marker < Self::MARKED_RADIUS && !desperate {
                continue;
            }
            let openness = ((marker - Self::MARKED_RADIUS)
                / (Self::CLEAR_RADIUS - Self::MARKED_RADIUS))
                .clamp(0.0, 1.0);

            // Is anybody standing in it? The nearest opponent to the
            // segment between the two of them, as a fraction of the
            // width it takes to cut the ball out.
            let lane = Self::lane_clearance(ctx, from, mate.position);

            // A comfortable throw rather than the longest one he can
            // manage: 1.0 at `COMFORTABLE` of his range, tapering both
            // ways.
            let reach = (distance / max_range).clamp(0.0, 1.0);
            let comfort = 1.0 - ((reach - Self::COMFORTABLE) / Self::COMFORTABLE).abs().min(1.0);

            // …and up the pitch, gently. Most throw-ins go square or
            // back, and weighting this any harder turns every one of them
            // into a hopeful ball down the line.
            let progression = (0.5 + 0.5 * (to.x * forward) / max_range).clamp(0.0, 1.0);

            let score =
                openness * 0.50 + lane * 0.22 + comfort * 0.18 + progression * 0.10;
            if best.map_or(true, |(_, b)| score > b) {
                best = Some((mate, score));
            }
        }
        best.map(|(m, _)| m)
    }

    /// 1.0 when the throwing lane is empty, falling to 0.0 when somebody
    /// is standing in it.
    ///
    /// Measured as the closest approach of any opponent to the segment
    /// `from → to`, against [`Self::LANE_WIDTH`]. Only opponents BESIDE
    /// the line count: one standing behind the thrower, or beyond the
    /// receiver, is not in the way of anything.
    fn lane_clearance(ctx: &StateProcessingContext, from: Vector3<f32>, to: Vector3<f32>) -> f32 {
        let lane = to - from;
        let length = lane.norm();
        if length < 1.0 {
            return 1.0;
        }
        let dir = lane / length;
        let mut nearest = f32::INFINITY;
        for (id, _) in ctx.tick_context.grid.opponents(ctx.player.id, length + 40.0) {
            let rel = ctx.tick_context.grid.position_of(id) - from;
            let along = rel.dot(&dir);
            if along <= 0.0 || along >= length {
                continue;
            }
            let across = (rel - dir * along).norm();
            nearest = nearest.min(across);
        }
        if nearest.is_infinite() {
            return 1.0;
        }
        (nearest / Self::LANE_WIDTH).clamp(0.0, 1.0)
    }
}
