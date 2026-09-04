//! The goal kick as a ceremony: place it, step back, look up, run in.
//!
//! # Why this exists
//!
//! A goal kick is the goalkeeper's most repeated set piece — eight to
//! twelve a match — and the engine took it in 20 ms. `AwaitedRestart`
//! walked him to the ball honestly and then handed him ownership the tick
//! he arrived; `TakeBall` saw the ball at his feet and went to
//! `Distributing`, whose first tick emitted the pass. Measured on a
//! recording: he reached the ball and it left at the next sample. From the
//! stands that is a keeper who walks up to a dead ball and it simply
//! departs — no placing, no stepping off it, no run.
//!
//! Two smaller faults rode along. The arrival path never asked the one
//! question the engine already knew how to answer — *short or long?* —
//! because `goes_long_from_goal_kick` lived in `Standing` and the restart
//! delivered him to `TakeBall`, so a keeper under press chose through
//! `KeeperFeetDecision` instead and could pick his own goal kick UP. And a
//! long goal kick struck from standing over the ball is a physical
//! nonsense a viewer reads instantly.
//!
//! # The model
//!
//! Short goal kicks are taken as they were: he walks to the ball and rolls
//! it to a full-back from standing, which is what a keeper playing out does.
//! A LONG one is a ceremony, owned by the restart the same way a corner's
//! wait-for-the-box is:
//!
//! 1. **Placing.** He reaches the ball. The decision to go long is taken
//!    here, once, on the same inputs `Standing` always used — his kicking
//!    against his passing, the press on his box, a free outlet, the side's
//!    build-up patience — and recorded on the ball so every later reader
//!    agrees with it.
//! 2. **Backing.** He walks to a mark [`Self::RUN_UP_DEPTH`] behind the
//!    ball and [`Self::RUN_UP_SIDE`] to the side of it: a run at an angle,
//!    which is how a ball is struck long.
//! 3. **Set.** He stands on the mark for [`Self::SCAN_TICKS`], looking up
//!    the pitch. The pause is the ceremony's whole point from the stands.
//! 4. **Running.** He runs at the ball at his chase pace, ownership is
//!    handed over as he reaches it, and `Kicking` strikes it on the next
//!    tick — the run ends in the kick.
//!
//! The ball stays pinned on the spot throughout: it is still a dead ball
//! and nothing the walk does can move it. `OF_KEEPER_RUNUP=off` restores
//! the instant kick.

use crate::PlayerFieldPositionGroup;
use crate::r#match::{MatchPlayer, PlayerSide, StateProcessingContext};
use nalgebra::Vector3;

pub struct KeeperGoalKick;

impl KeeperGoalKick {
    /// How far behind the ball he takes his run from, in units. 22u =
    /// 2.75 m: three strides, which is what a goalkeeper's run-up is.
    pub const RUN_UP_DEPTH: f32 = 22.0;
    /// …and how far to the side, toward the middle of the pitch. 14u =
    /// 1.75 m, an approach of about 30° — a ball struck long is not
    /// struck from dead behind.
    pub const RUN_UP_SIDE: f32 = 14.0;
    /// He is on his mark within this. 5u = 0.6 m — wider than
    /// `Arrive`'s own deadzone so a man settling beside the mark counts.
    pub const MARK_REACH: f32 = 5.0;
    /// How long he stands on the mark, in engine ticks. 150 = 1.5 s: long
    /// enough to be a pause, short enough that a match with ten of them
    /// does not stall behind the keeper.
    pub const SCAN_TICKS: u64 = 150;
    /// Longest the walk back may take before the restart stops waiting
    /// for it and treats him as set where he stands. 300 = 3 s for a
    /// three-metre walk.
    pub const BACKING_CEILING: u64 = 300;
    /// Walking pace, as a share of his chase steering.
    pub const BACKING_PACE: f32 = 0.38;
    /// How close he has to be to the ball for the run to end in the
    /// strike, in units. 4u = half a metre — the ball is at his boot,
    /// against the 1.5 m `AwaitedRestart::REACH` hands a walking taker.
    pub const STRIKE_REACH: f32 = 4.0;
    /// Longest the run-in may take before the ordinary arrival takes
    /// over, in engine ticks. 200 = 2 s for a three-metre run.
    pub const RUN_IN_CEILING: u64 = 200;

    /// False when `OF_KEEPER_RUNUP=off`.
    pub fn armed() -> bool {
        use std::sync::OnceLock;
        static ON: OnceLock<bool> = OnceLock::new();
        *ON.get_or_init(|| std::env::var("OF_KEEPER_RUNUP").as_deref() != Ok("off"))
    }

    /// Goal kick: go long, or play short to a defender?
    ///
    /// Continuous score, no threshold flip. Going long is driven by the
    /// keeper's kicking leg, by how high the opposition have pushed
    /// (nothing else is on when they squeeze the box), and by how direct
    /// the manager wants the side to be. Playing short is driven by
    /// having a genuinely free defender to give it to and the composure
    /// to do it under pressure — the modern build-out, available only to
    /// keepers who can actually pass.
    ///
    /// The pure form, so the ball's restart tick and the keeper's own
    /// state can ask the same question of the same inputs.
    pub fn goes_long(
        kick_skill: f32,
        short_skill: f32,
        composure: f32,
        press: f32,
        short_available: f32,
        patience: f32,
    ) -> bool {
        let long = 0.40 + kick_skill * 0.50 + press * 0.75 - patience * 0.35;
        let short =
            0.20 + short_available * 0.55 + short_skill * 0.45 + composure * 0.15 - press * 0.55;
        long >= short
    }

    /// The decision as the ball sees it at the moment he places the kick:
    /// the keeper's own skills, the press around the SPOT and the free
    /// outlets near it.
    pub fn goes_long_at_spot(
        taker: &MatchPlayer,
        spot: Vector3<f32>,
        players: &[MatchPlayer],
        patience: f32,
    ) -> bool {
        let gk = &taker.skills.goalkeeping;
        let kick_skill = (gk.kicking / 20.0).clamp(0.0, 1.0);
        let short_skill = ((gk.passing + gk.first_touch) / 40.0).clamp(0.0, 1.0);
        let composure = (taker.skills.mental.composure / 20.0).clamp(0.0, 1.0);
        let on_pitch = |p: &&MatchPlayer| p.side.is_some() && !p.is_sent_off && p.id != taker.id;
        let flat = |a: Vector3<f32>, b: Vector3<f32>| {
            Vector3::new(a.x - b.x, a.y - b.y, 0.0).norm()
        };
        let squeezing = players
            .iter()
            .filter(on_pitch)
            .filter(|p| p.side != taker.side && flat(p.position, spot) < 200.0)
            .count() as f32;
        let press = ((squeezing - 1.0) / 3.0).clamp(0.0, 1.0);
        let free_short = players
            .iter()
            .filter(on_pitch)
            .filter(|t| t.side == taker.side && flat(t.position, spot) < 120.0)
            .filter(|t| {
                !players
                    .iter()
                    .filter(on_pitch)
                    .any(|o| o.side != taker.side && flat(o.position, t.position) < 18.0)
            })
            .count();
        let short_available = (free_short as f32 / 2.0).min(1.0);
        Self::goes_long(
            kick_skill,
            short_skill,
            composure,
            press,
            short_available,
            patience.clamp(0.0, 1.0),
        )
    }

    /// The same decision from inside the keeper's own state machine.
    pub fn goes_long_from_ctx(ctx: &StateProcessingContext) -> bool {
        let gk = &ctx.player.skills.goalkeeping;
        let kick_skill = (gk.kicking / 20.0).clamp(0.0, 1.0);
        let short_skill = ((gk.passing + gk.first_touch) / 40.0).clamp(0.0, 1.0);
        let composure = (ctx.player.skills.mental.composure / 20.0).clamp(0.0, 1.0);
        let squeezing = ctx.players().opponents().nearby(200.0).count() as f32;
        let press = ((squeezing - 1.0) / 3.0).clamp(0.0, 1.0);
        let free_short = ctx
            .players()
            .teammates()
            .nearby(120.0)
            .filter(|t| ctx.tick_context.grid.opponents(t.id, 18.0).next().is_none())
            .count();
        let short_available = (free_short as f32 / 2.0).min(1.0);
        let patience = ctx.team().build_up_patience().clamp(0.0, 1.0);
        Self::goes_long(
            kick_skill,
            short_skill,
            composure,
            press,
            short_available,
            patience,
        )
    }

    /// Long or short, as DECIDED — by the ball at placement when the
    /// run-up is armed (so the man who took a run-up kicks it long, and
    /// the man who did not rolls it short), otherwise by the keeper now.
    pub fn decided_long(ctx: &StateProcessingContext) -> bool {
        if Self::armed() {
            ctx.tick_context.ball.goal_kick_long
        } else {
            Self::goes_long_from_ctx(ctx)
        }
    }

    /// Where he takes his run from: behind the ball, toward his own goal,
    /// and a little toward the middle of the pitch.
    pub fn mark(spot: Vector3<f32>, side: Option<PlayerSide>, field_height: f32) -> Vector3<f32> {
        let back = -side.map_or(1.0, |s| s.forward_dir_x());
        let inward = if spot.y > field_height * 0.5 { -1.0 } else { 1.0 };
        Vector3::new(
            spot.x + back * Self::RUN_UP_DEPTH,
            spot.y + inward * Self::RUN_UP_SIDE,
            0.0,
        )
    }

    /// Is this player a goalkeeper? The restart tick asks before it
    /// stages a run-up — an outfielder standing in for a sent-off keeper
    /// takes the kick as he would any other.
    pub fn is_keeper(player: &MatchPlayer) -> bool {
        player.tactical_position.current_position.position_group()
            == PlayerFieldPositionGroup::Goalkeeper
    }
}
