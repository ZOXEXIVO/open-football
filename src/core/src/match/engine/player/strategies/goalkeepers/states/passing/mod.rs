use crate::r#match::events::Event;
use crate::r#match::goalkeepers::states::state::GoalkeeperState;
use crate::r#match::player::events::{PassingEventContext, PlayerEvent};
use crate::r#match::{ConditionContext, MatchPlayerLite, PassEvaluator, StateChangeResult, StateProcessingContext, StateProcessingHandler};
use nalgebra::Vector3;

#[derive(Default)]
pub struct GoalkeeperPassingState {}

impl StateProcessingHandler for GoalkeeperPassingState {
    fn try_fast(&self, ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        if !ctx.player.has_ball(ctx) {
            return Some(StateChangeResult::with_goalkeeper_state(
                GoalkeeperState::Standing,
            ));
        }

        if let Some(teammate) = self.find_best_pass_option(ctx) {
            return Some(StateChangeResult::with_goalkeeper_state_and_event(
                GoalkeeperState::Standing,
                Event::PlayerEvent(PlayerEvent::PassTo(
                    PassingEventContext::new()
                        .with_from_player_id(ctx.player.id)
                        .with_to_player_id(teammate.id)
                        .build(ctx)
                )),
            ));
        }

        if ctx.in_state_time > 10 {
            return Some(StateChangeResult::with_goalkeeper_state(
                GoalkeeperState::Running,
            ));
        }

        None
    }

    fn process_slow(&self, _ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        None
    }

    fn velocity(&self, _ctx: &StateProcessingContext) -> Option<Vector3<f32>> {
        Some(Vector3::new(0.0, 0.0, 0.0))
    }

    fn process_conditions(&self, _ctx: ConditionContext) {}
}

impl GoalkeeperPassingState {
    fn find_best_pass_option(&self, ctx: &StateProcessingContext) -> Option<MatchPlayerLite> {
        // Goalkeeper passing - search extended range for ultra-long passes
        let max_distance = ctx.context.field_size.width as f32 * 2.0;

        // Get goalkeeper's vision and kicking skills
        let vision_skill = ctx.player.skills.mental.vision / 20.0;
        let kicking_skill = ctx.player.skills.technical.long_throws / 20.0;
        let _pass_skill = ctx.player.skills.technical.passing / 20.0;
        let technique_skill = ctx.player.skills.technical.technique / 20.0;

        // Calculate ultra-long pass capability
        let ultra_long_capability = (vision_skill * 0.5) + (kicking_skill * 0.3) + (technique_skill * 0.2);

        // Find best option using standard evaluator
        let mut best_option = PassEvaluator::find_best_pass_option(ctx, max_distance);

        // If goalkeeper has elite skills, bias towards longer passes
        if ultra_long_capability > 0.75 {
            // Look specifically for ultra-long opportunities to forwards
            let mut best_ultra_score = 0.0;

            for teammate in ctx.players().teammates().nearby(max_distance) {
                let distance = (teammate.position - ctx.player.position).norm();

                // Only consider ultra-long passes
                if distance < 200.0 {
                    continue;
                }

                // Prefer forwards for ultra-long passes
                let is_forward = matches!(
                    teammate.tactical_positions.position_group(),
                    crate::PlayerFieldPositionGroup::Forward
                );

                if !is_forward {
                    continue;
                }

                // Calculate forward progress
                let forward_progress = teammate.position.x - ctx.player.position.x;
                if forward_progress < 0.0 {
                    continue; // Don't pass backwards on ultra-long
                }

                // Check space around receiver
                let nearby_opponents = ctx.tick_context.distances.opponents(teammate.id, 10.0).count();

                let space_factor = if nearby_opponents == 0 {
                    2.0
                } else if nearby_opponents == 1 {
                    1.2
                } else {
                    0.6
                };

                // Score based on distance, vision, and space
                let distance_score = if distance > 300.0 {
                    ultra_long_capability * 2.5
                } else if distance > 200.0 {
                    ultra_long_capability * 2.0
                } else {
                    1.0
                };

                let score = distance_score * space_factor * (forward_progress / ctx.context.field_size.width as f32);

                if score > best_ultra_score {
                    best_ultra_score = score;
                    best_option = Some(teammate);
                }
            }
        }

        best_option
    }
}
