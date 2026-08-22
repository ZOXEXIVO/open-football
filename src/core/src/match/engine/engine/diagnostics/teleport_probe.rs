//! **The whole-tick ball relocation probe.** Carries the ball's position
//! from one census checkpoint to the next through a tick, so the call
//! sites in the tick driver read as one line each.

use crate::r#match::engine::ball::ball::teleport as tc;
use crate::r#match::engine::engine::*;
use nalgebra::Vector3;

/// Carries the ball's position from one census checkpoint to the next
/// through a tick.
///
/// It exists so the call sites in the tick loop read as one line each.
/// Nothing between two checkpoints integrates the ball — see
/// [`teleport`](crate::r#match::engine::ball::ball::teleport) — so the
/// probe holds the position and the velocity either side of `Ball::update`
/// and needs nothing else.
#[cfg(feature = "match-logs")]
pub(in crate::r#match::engine::engine) struct TeleportProbe {
    pos: Vector3<f32>,
    entry_velocity: Vector3<f32>,
    dead: bool,
}

#[cfg(feature = "match-logs")]
impl TeleportProbe {
    pub(in crate::r#match::engine::engine) fn open(field: &MatchField) -> Self {
        tc::TeleportCensus::note_tick();
        Self {
            pos: field.ball.position,
            entry_velocity: field.ball.velocity,
            // Sampled at the top of the tick: a relocation on a ball that
            // was already dead when the tick began is a dead-ball leak,
            // whereas a restart AWARDED during the tick legitimately
            // places one. Reading the flag afterwards would confuse them.
            dead: field.ball.awaiting_restart.is_some(),
        }
    }

    /// A checkpoint after `Ball::update`, where no travel is explained.
    pub(in crate::r#match::engine::engine) fn at(&mut self, field: &MatchField, stage: usize) {
        self.pos = tc::TeleportCensus::checkpoint(stage, self.pos, field.ball.position, self.dead);
    }

    /// The ball's own pass, whose travel its velocity does explain.
    pub(in crate::r#match::engine::engine) fn ball_update(
        &mut self,
        field: &MatchField,
        stage: usize,
    ) {
        self.pos = tc::TeleportCensus::note_ball_update(
            stage,
            self.pos,
            field.ball.position,
            self.entry_velocity,
            field.ball.velocity,
            self.dead,
        );
    }
}
