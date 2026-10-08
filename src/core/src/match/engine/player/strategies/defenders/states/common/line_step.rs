//! A defender's half of the offside trap: whether he has read the line's
//! call yet, and where he steps to once he has. See `LineStepCall` for the
//! call itself.

use crate::r#match::StateProcessingContext;

pub struct LineStep;

impl LineStep {
    /// A metre past the runner: where the line steps to.
    const PAST: f32 = 8.0;

    /// Engine ticks a defender needs to read the call and go: a fifth of a
    /// second for a sharp reader in a side that knows its line with a
    /// keeper talking to it, most of a second for a slow one in a side
    /// that does not.
    pub fn reaction_ticks(anticipation: f32, trap_risk: f32, keeper_voice: f32) -> u64 {
        let slow_read = (1.0 - (anticipation / 20.0).clamp(0.0, 1.0)) * 60.0;
        (20.0 + slow_read + trap_risk * 1000.0 - keeper_voice.clamp(0.0, 1.0) * 20.0).max(10.0)
            as u64
    }

    /// The depth he holds once he has gone with the call: just past the
    /// runner, and never deeper than `target_x`, where he was going anyway.
    /// `None` while there is no call or he has not read it yet.
    pub fn depth(ctx: &StateProcessingContext, target_x: f32) -> Option<f32> {
        let call = ctx.team().defensive_plan().line_step?;
        let team_id = ctx.player.team_id;
        let reaction = Self::reaction_ticks(
            ctx.player.skills.mental.anticipation,
            ctx.context
                .familiarity_for_team(team_id)
                .offside_trap_risk(),
            ctx.context.skill_aggregates_for_team(team_id).keeper_voice,
        );
        if ctx.context.current_tick() < call.called_tick + reaction {
            return None;
        }
        let forward = ctx.player.side?.forward_dir_x();
        let stepped =
            ctx.tick_context.positions.players.position(call.runner).x + forward * Self::PAST;
        ((stepped - target_x) * forward > 0.0).then_some(stepped)
    }
}
