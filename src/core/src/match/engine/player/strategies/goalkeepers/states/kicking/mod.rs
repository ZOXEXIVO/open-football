use crate::r#match::events::Event;
use crate::r#match::goalkeepers::states::common::{ActivityIntensity, GoalkeeperCondition};
use crate::r#match::goalkeepers::states::state::GoalkeeperState;
use crate::r#match::player::events::{PassingEventContext, PlayerEvent};
use crate::r#match::{ConditionContext, MatchPlayerLite, PassEvaluator, StateChangeResult, StateProcessingContext, StateProcessingHandler};
use nalgebra::Vector3;

#[derive(Default)]
pub struct GoalkeeperKickingState {}

impl StateProcessingHandler for GoalkeeperKickingState {
    fn try_fast(&self, ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        // 1. Check if the goalkeeper has the ball
        if !ctx.player.has_ball(ctx) {
            return Some(StateChangeResult::with_goalkeeper_state(
                GoalkeeperState::Standing,
            ));
        }

        // 2. Find the best teammate to kick the ball to
        if let Some(teammate) = self.find_best_pass_option(ctx) {
            return Some(StateChangeResult::with_goalkeeper_state_and_event(
                GoalkeeperState::Standing,
                Event::PlayerEvent(PlayerEvent::PassTo(
                    PassingEventContext::new()
                        .with_from_player_id(ctx.player.id)
                        .with_to_player_id(teammate.id)
                        .build(ctx),
                )),
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

    fn process_conditions(&self, ctx: ConditionContext) {
        // Kicking requires moderate intensity with focused effort
        GoalkeeperCondition::new(ActivityIntensity::Moderate).process(ctx);
    }
}

impl GoalkeeperKickingState {
    fn find_best_pass_option<'a>(
        &self,
        ctx: &StateProcessingContext<'a>,
    ) -> Option<MatchPlayerLite> {
        // Kicking allows for extreme long passes - search maximum range including 300m+
        let max_distance = ctx.context.field_size.width as f32 * 3.0;

        // Get goalkeeper's kicking and vision skills
        let vision_skill = ctx.player.skills.mental.vision / 20.0;
        let kicking_skill = ctx.player.skills.technical.long_throws / 20.0;
        let technique_skill = ctx.player.skills.technical.technique / 20.0;
        let anticipation_skill = ctx.player.skills.mental.anticipation / 20.0;

        // Calculate extreme pass capability (kicking emphasized)
        let extreme_capability = (kicking_skill * 0.5) + (vision_skill * 0.3) + (technique_skill * 0.15) + (anticipation_skill * 0.05);

        // Determine if goalkeeper should attempt extreme clearances
        let can_attempt_extreme = extreme_capability > 0.7;
        let prefers_extreme = extreme_capability > 0.8;

        let mut best_option: Option<MatchPlayerLite> = None;
        let mut best_score = 0.0;

        for teammate in ctx.players().teammates().nearby(max_distance) {
            let distance = (teammate.position - ctx.player.position).norm();

            // Calculate base score using vision-weighted evaluation
            let forward_progress = (teammate.position.x - ctx.player.position.x).max(0.0);
            let field_progress = forward_progress / ctx.context.field_size.width as f32;

            // Check if receiver is a forward
            let is_forward = matches!(
                teammate.tactical_positions.position_group(),
                crate::PlayerFieldPositionGroup::Forward
            );

            // Check space around receiver
            let nearby_opponents = ctx.tick_context.distances.opponents(teammate.id, 15.0).count();
            let space_factor = match nearby_opponents {
                0 => 3.0, // Completely free
                1 => 1.8,
                2 => 1.0,
                _ => 0.4,
            };

            // Distance-based scoring with vision weighting
            let distance_score = if distance > 300.0 {
                // Extreme long kicks (300m+)
                if !can_attempt_extreme {
                    0.2 // Very poor option without skills
                } else if prefers_extreme && is_forward {
                    // Elite kicker targeting forward - spectacular play
                    let extreme_bonus = (extreme_capability - 0.8) * 10.0; // 0.0 to 2.0
                    3.5 + extreme_bonus
                } else if is_forward {
                    2.5 + (extreme_capability * 1.5)
                } else {
                    1.0 // Avoid extreme passes to non-forwards
                }
            } else if distance > 200.0 {
                // Ultra-long kicks (200-300m)
                if extreme_capability > 0.75 {
                    if is_forward {
                        3.0 + (vision_skill * 2.0) // Vision helps spot forwards
                    } else {
                        1.8
                    }
                } else if extreme_capability > 0.6 {
                    2.0 + (kicking_skill * 1.5)
                } else {
                    1.2
                }
            } else if distance > 100.0 {
                // Very long kicks (100-200m)
                if is_forward {
                    2.5 + (vision_skill * 1.0)
                } else {
                    1.8
                }
            } else if distance > 60.0 {
                // Long kicks (60-100m)
                2.0
            } else {
                // Short kicks - less ideal for kicking state
                0.8
            };

            // Position bonus
            let position_bonus = match teammate.tactical_positions.position_group() {
                crate::PlayerFieldPositionGroup::Forward => {
                    if distance > 300.0 && prefers_extreme {
                        2.5 // Extreme clearance to striker
                    } else if distance > 200.0 {
                        2.0 // Ultra-long to striker
                    } else {
                        1.5
                    }
                }
                crate::PlayerFieldPositionGroup::Midfielder => {
                    if distance > 200.0 {
                        0.7 // Avoid ultra-long to midfield
                    } else {
                        1.2
                    }
                }
                crate::PlayerFieldPositionGroup::Defender => 0.3, // Avoid kicking to defenders
                crate::PlayerFieldPositionGroup::Goalkeeper => 0.1,
            };

            // Combine all factors with vision-based weighting
            let score = distance_score * space_factor * position_bonus * (1.0 + field_progress) * (0.5 + vision_skill * 0.5);

            if score > best_score {
                best_score = score;
                best_option = Some(teammate);
            }
        }

        // Fallback to standard evaluator if no good option found
        if best_option.is_none() || best_score < 1.0 {
            PassEvaluator::find_best_pass_option(ctx, max_distance)
        } else {
            best_option
        }
    }
}

