use crate::r#match::defenders::states::DefenderState;
use crate::r#match::defenders::states::common::{ActivityIntensity, DefenderCondition};
use crate::r#match::{
    ConditionContext, StateChangeResult, StateProcessingContext, StateProcessingHandler,
};
use nalgebra::Vector3;

// Lowered from 90% — a condition this high is rarely reached mid-match
// (condition drifts through 40-80% band under active play), which kept
// defenders stuck in Resting forever. 65% is a natural "second wind"
// point for exiting a recovery jog back into active defending.
const STAMINA_RECOVERY_THRESHOLD: f32 = 65.0;
/// Minimum stamina required to abandon rest and engage a crisis.
/// Hysteresis against Pressing's 25% exit threshold — the 25%–45%
/// band is a "stay in Resting but walk toward the ball" zone, so a
/// defender at 30% stamina doesn't flicker into Pressing and back.
const CRISIS_ENGAGE_STAMINA: f32 = 45.0;
const BALL_PROXIMITY_THRESHOLD: f32 = 10.0;
const MARKING_DISTANCE_THRESHOLD: f32 = 10.0;

#[derive(Default, Clone)]
pub struct DefenderRestingState {}

impl StateProcessingHandler for DefenderRestingState {
    fn process(&self, ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        let stamina = ctx.player.player_attributes.condition_percentage() as f32;

        // Crisis engage — only exit to Pressing if we have enough
        // stamina to actually press (≥45%). Between Pressing's 25%
        // exit and this 45% re-entry is a hysteresis band: during
        // crisis we walk toward the ball (see `velocity`) but don't
        // sprint into a press we'll immediately exit due to fatigue.
        // Stops the Resting ↔ Pressing flicker the user was seeing.
        if ctx.player().defensive().is_defensive_crisis() && stamina >= CRISIS_ENGAGE_STAMINA {
            return Some(StateChangeResult::with_defender_state(
                DefenderState::Pressing,
            ));
        }

        // 1. Stamina recovered enough — back to full defensive duties
        if stamina >= STAMINA_RECOVERY_THRESHOLD {
            return Some(StateChangeResult::with_defender_state(
                DefenderState::HoldingLine,
            ));
        }

        // 2. Check if the ball is close
        let ball_distance =
            (ctx.tick_context.positions.ball.position - ctx.player.position).magnitude();
        if ball_distance < BALL_PROXIMITY_THRESHOLD {
            // If the ball is close, check for nearby opponents
            let opponent_nearby = self.is_opponent_nearby(ctx);
            return Some(StateChangeResult::with_defender_state(if opponent_nearby {
                DefenderState::Marking
            } else {
                DefenderState::Intercepting
            }));
        }

        // Previous "team under threat" exit (fires if 2+ opponents in
        // our defensive third) was causing Resting ↔ Pressing flicker:
        // Pressing drains stamina to <30% → Resting → threat still
        // present → Pressing again → drain → Resting. Hysteresis is
        // now handled via `is_defensive_crisis` (ball in our third
        // with an opposing carrier — a real emergency) and the
        // ball-proximity check above. Everything else waits for
        // stamina to recover.

        // Remain in Resting state
        None
    }

    fn velocity(&self, ctx: &StateProcessingContext) -> Option<Vector3<f32>> {
        // Crisis is active but we're too tired to press — walk toward
        // the ball rather than standing still. Real football: a
        // winded defender doesn't stop in place while the opposition
        // attacks, they jog back into shape. This is the "not running
        // to the player with ball" fix: the defender still moves
        // toward the threat at ~35% pace, even while recovering.
        if ctx.player().defensive().is_defensive_crisis() {
            let to_ball = ctx.tick_context.positions.ball.position - ctx.player.position;
            let dist = to_ball.magnitude();
            if dist > 5.0 {
                let direction = to_ball / dist;
                let walk_speed = ctx.player.skills.physical.pace * 0.35;
                return Some(direction * walk_speed);
            }
        }

        // No crisis — full stop for maximum recovery.
        Some(Vector3::new(0.0, 0.0, 0.0))
    }

    fn process_conditions(&self, ctx: ConditionContext) {
        // Resting provides maximum recovery - dedicated recovery state
        DefenderCondition::new(ActivityIntensity::Recovery).process(ctx);
    }
}

impl DefenderRestingState {
    /// Checks if an opponent player is nearby within the MARKING_DISTANCE_THRESHOLD.
    fn is_opponent_nearby(&self, ctx: &StateProcessingContext) -> bool {
        ctx.players().opponents().exists(MARKING_DISTANCE_THRESHOLD)
    }
}
