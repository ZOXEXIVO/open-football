//! The team as ONE body — a live positional block that slides with the
//! ball, and the anchor each player holds inside it.
//!
//! # The problem this exists to solve
//!
//! Everything in this engine that is *about the ball* was already a team
//! decision: [`TeamTacticalState`](super::super::tactical) says how we are
//! playing, [`AttackPlan`](super::attack) says who the attack is for and
//! who takes which patch of the box, [`DefensivePlan`](super::defence)
//! says who picks up whom. Between them they name at most six or seven
//! players, and only while the ball is live near them.
//!
//! Everyone else — which at any instant is most of the side — had no team
//! reference at all. Their off-ball destination was
//! [`MatchPlayer::start_position`](crate::r#match::MatchPlayer::start_position):
//! a kickoff formation dot, fixed for the whole match, only ever rewritten
//! by a red card, a substitution or the half-time swap. Every resting
//! state steered at it — `Returning` arrived at it, `Walking` *wandered
//! around it*, `Standing` returned `Vector3::zeros()` and did nothing at
//! all.
//!
//! So the shape never moved. Measured over three matches at level 14
//! (`dev_match paths`):
//!
//! | | engine | real |
//! |---|---|---|
//! | outfield block length | **84.7 m** | 35-45 m |
//! | forwards standing still | **63.2%** | 15-25% |
//! | midfielder distance | **23.6 km** | ~11 km |
//! | defender distance | **17.9 km** | ~10 km |
//! | nearest team-mate | **8.6 m** | 15-20 m |
//!
//! That is one number saying the whole thing: an 85 m block is not a team,
//! it is two groups of five with a hole between them. Forwards camped on
//! the last line doing nothing because the ball was 60 m away and their
//! dot was here; defenders camped on their own dots; and the midfield ran
//! 23.6 km trying to be in both places, which is also why team-mates
//! averaged 8.6 m apart while the block was twice its real length — the
//! ball-chasing logic pulled bodies into clumps that the static dots then
//! dragged back apart.
//!
//! # The rule
//!
//! A real defensive block is a rigid-ish rectangle roughly 35-45 m long
//! and 40-55 m wide that *slides*: goal-side of the ball by a standoff
//! distance, shifted laterally toward the ball's flank, compressed or
//! stretched by the phase. Players hold a position **within that
//! rectangle**, not on the pitch.
//!
//! So the formation stops being a set of pitch coordinates and becomes a
//! *pattern*: each player's kickoff dot is normalised into
//! (depth, lateral) fractions of the kickoff shape's own bounding box,
//! and those fractions are projected into the live block every refresh.
//! A 4-4-2's two banks stay two banks; where those banks *are* is now a
//! function of the ball and the phase.
//!
//! Refreshed on the same cadence as the other three plans, from the
//! tactical state that was just computed, so all four always describe the
//! same instant.

use crate::r#match::engine::teamplay::plans::block::{
    BLOCK_SPEED, LENGTH_COMPACT, ShapeBuilder, WIDTH_NARROW,
};
use crate::r#match::engine::teamplay::tactical::TeamTacticalState;
use crate::r#match::{MatchContext, MatchField};
use nalgebra::Vector3;

/// Most players one side can have on the pitch.
pub(in crate::r#match::engine::teamplay::plans) const MAX_ON_PITCH: usize = 11;

/// One side's live positional block plus the anchor each player holds
/// inside it. Cheap to copy (plain POD), like the other three plans.
#[derive(Debug, Clone, Copy)]
pub struct TeamShape {
    pub(in crate::r#match::engine::teamplay::plans) anchors: [(u32, Vector3<f32>); MAX_ON_PITCH],
    pub(in crate::r#match::engine::teamplay::plans) len: usize,
    /// Attacking progress (0 = own goal line, 1 = theirs) of the block's
    /// deepest line.
    pub rear_progress: f32,
    /// Block length along the goal-to-goal axis, in units.
    pub length: f32,
    /// Lateral centre of the block, in pitch y.
    pub centre_y: f32,
    /// Block width, in units.
    pub width: f32,
    /// False before the first refresh, or for a side with no players —
    /// consumers then fall back to the kickoff dot.
    pub active: bool,
}

impl TeamShape {
    pub const fn idle() -> Self {
        TeamShape {
            anchors: [(0, Vector3::new(0.0, 0.0, 0.0)); MAX_ON_PITCH],
            len: 0,
            rear_progress: 0.0,
            length: LENGTH_COMPACT,
            centre_y: 0.0,
            width: WIDTH_NARROW,
            active: false,
        }
    }

    /// Where this player should be standing right now in the absence of a
    /// ball-related job. `None` before the first refresh or for a player
    /// who is not on this side's pitch list.
    pub fn anchor_of(&self, player_id: u32) -> Option<Vector3<f32>> {
        if !self.active {
            return None;
        }
        self.anchors[..self.len]
            .iter()
            .find(|(id, _)| *id == player_id)
            .map(|(_, pos)| *pos)
    }

    /// The block's live lateral centre, or `fallback` before the first
    /// refresh.
    ///
    /// The counterpart of `DefensiveLine::position_x` for the other axis:
    /// a unit-level answer to "where is the middle of us right now", so a
    /// state that positions a player sideways has something to hang off
    /// besides the pitch's own midpoint. Without it every lateral
    /// constraint in the engine measured from the halfway stripe, which
    /// is a fixed landmark, and the whole side was laterally static
    /// however far the block was told to slide.
    pub fn centre_or(&self, fallback: f32) -> f32 {
        if self.active { self.centre_y } else { fallback }
    }

    /// Recompute both sides' blocks in place, from the tactical state
    /// produced by the same refresh pass.
    pub fn refresh(home: &mut Self, away: &mut Self, inputs: &ShapeRefreshInputs<'_>) {
        // A/B control for the whole positional layer — see
        // `MatchContext::shape_off`. Leaving both plans inert makes
        // `anchor_of` return `None`, and every consumer then falls back
        // to the kickoff dot.
        if MatchContext::shape_off() {
            *home = Self::idle();
            *away = Self::idle();
            return;
        }
        let max_step = BLOCK_SPEED * inputs.tick_interval.max(1) as f32;
        for (shape, team_id, tactical) in [
            (&mut *home, inputs.home_team_id, inputs.home_tactical),
            (&mut *away, inputs.away_team_id, inputs.away_tactical),
        ] {
            ShapeBuilder {
                field: inputs.field,
                team_id,
                tactical,
                max_step,
            }
            .build(shape);
        }
    }
}

/// Inputs to [`TeamShape::refresh`], bundled so the call site stays
/// readable — the same shape as the other three plans' input structs.
pub struct ShapeRefreshInputs<'a> {
    pub field: &'a MatchField,
    pub home_team_id: u32,
    pub away_team_id: u32,
    pub home_tactical: &'a TeamTacticalState,
    pub away_tactical: &'a TeamTacticalState,
    /// Ticks elapsed since the last refresh — the block's movement
    /// allowance scales with it.
    pub tick_interval: u32,
}
