use crate::r#match::goalkeepers::states::common::{ActivityIntensity, GoalkeeperCondition};
use crate::r#match::goalkeepers::states::state::GoalkeeperState;
use crate::r#match::player::events::PlayerEvent;
use crate::r#match::player::strategies::players::ops::goalkeeper_skill::GoalkeeperSkillProfile;
use crate::r#match::{
    ConditionContext, StateChangeResult, StateProcessingContext, StateProcessingHandler,
};
use nalgebra::Vector3;

const PUNCHING_DISTANCE_THRESHOLD: f32 = 2.0; // Maximum distance to attempt punching

#[derive(Default, Clone)]
pub struct GoalkeeperPunchingState {}

impl StateProcessingHandler for GoalkeeperPunchingState {
    fn process(&self, ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        let prof = GoalkeeperSkillProfile::from_ctx(ctx);
        // Punch reach now scales with aerial command + parry control:
        // weak ~1.6u, elite ~3.0u against the previous flat 2.0u.
        let punch_threshold = PUNCHING_DISTANCE_THRESHOLD
            * (0.85 + prof.aerial_command * 0.40 + prof.parry_control * 0.25);
        if ctx.ball().distance() > punch_threshold {
            return Some(StateChangeResult::with_goalkeeper_state(
                GoalkeeperState::Jumping,
            ));
        }

        // Crowd / pressure from nearby opponents — high crowd punishes
        // weak claim quality more.
        let crowd = (ctx.players().opponents().nearby(8.0).count() as f32 / 4.0).clamp(0.0, 1.0);
        let claim_quality = (prof.aerial_command * 0.46
            + prof.positioning * 0.18
            + prof.rushing_out_profile * 0.12
            + prof.communication * 0.10
            + prof.condition_mult * 0.06)
            .clamp(0.0, 1.0);
        let claim_difficulty =
            (crowd * 0.32 + (1.0 - prof.parry_control) * 0.20 + prof.poor_skill_penalty * 0.18)
                .clamp(0.0, 1.0);
        // Punch success rolls between ~0.40 (weak in heavy traffic) and
        // ~0.92 (elite in space).
        let punch_success_prob =
            (claim_quality * 0.85 - claim_difficulty * 0.30 + prof.elite_lift).clamp(0.20, 0.95);
        let punch_success = rand::random::<f32>() < punch_success_prob;

        if punch_success {
            // Punch is successful
            let mut state_change =
                StateChangeResult::with_goalkeeper_state(GoalkeeperState::Standing);

            // Determine the direction to punch the ball (e.g., towards the sidelines)
            let punch_direction = ctx.ball().direction_to_own_goal().normalize() * -1.0;

            // Generate a punch event
            state_change
                .events
                .add_player_event(PlayerEvent::ClearBall(punch_direction));

            Some(state_change)
        } else {
            // Punch failed, transition to appropriate state (e.g., Diving)
            Some(StateChangeResult::with_goalkeeper_state(
                GoalkeeperState::Diving,
            ))
        }
    }

    fn velocity(&self, _ctx: &StateProcessingContext) -> Option<Vector3<f32>> {
        // Remain stationary while punching
        Some(Vector3::new(0.0, 0.0, 0.0))
    }

    fn process_conditions(&self, ctx: ConditionContext) {
        // Punching is a very high intensity activity requiring explosive effort
        GoalkeeperCondition::new(ActivityIntensity::VeryHigh).process(ctx);
    }
}
