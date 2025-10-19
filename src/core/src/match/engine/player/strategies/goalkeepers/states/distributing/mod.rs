use crate::r#match::events::Event;
use crate::r#match::goalkeepers::states::state::GoalkeeperState;
use crate::r#match::player::events::{PassingEventContext, PlayerEvent};
use crate::r#match::{ConditionContext, MatchPlayerLite, StateChangeResult, StateProcessingContext, StateProcessingHandler};
use nalgebra::Vector3;

#[derive(Default)]
pub struct GoalkeeperDistributingState {}

impl StateProcessingHandler for GoalkeeperDistributingState {
    fn try_fast(&self, ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        if !ctx.player.has_ball(ctx) {
            return Some(StateChangeResult::with_goalkeeper_state(
                GoalkeeperState::Standing,
            ));
        }

        if let Some(teammate) = self.find_best_pass_option(ctx) {
            return Some(StateChangeResult::with_goalkeeper_state_and_event(
                GoalkeeperState::ReturningToGoal,
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

impl GoalkeeperDistributingState {
    fn find_best_pass_option<'a>(&'a self, ctx: &'a StateProcessingContext<'a>) -> Option<MatchPlayerLite> {
        // Goalkeepers should look for long passes to start attacks
        // Search the entire field for passing options
        let max_distance = ctx.context.field_size.width as f32 * 1.5;

        // Get goalkeeper's skills to determine passing style
        let pass_skill = ctx.player.skills.technical.passing / 20.0;
        let vision_skill = ctx.player.skills.mental.vision / 20.0;
        let kicking_skill = ctx.player.skills.technical.long_throws / 20.0;
        let decision_skill = ctx.player.skills.mental.decisions / 20.0;
        let composure_skill = ctx.player.skills.mental.composure / 20.0;
        let anticipation_skill = ctx.player.skills.mental.anticipation / 20.0;

        // Determine goalkeeper passing style based on skills
        let is_technical_keeper = pass_skill > 0.7 && vision_skill > 0.7; // Likes build-up play
        let is_long_ball_keeper = kicking_skill > 0.7 && pass_skill < 0.6; // Prefers long kicks
        let is_cautious_keeper = composure_skill < 0.5 || decision_skill < 0.5; // Safe, short passes
        let is_visionary_keeper = vision_skill > 0.8 && anticipation_skill > 0.7; // Sees through balls

        let mut best_option: Option<MatchPlayerLite> = None;
        let mut best_score = 0.0;

        for teammate in ctx.players().teammates().nearby(max_distance) {
            let distance = (teammate.position - ctx.player.position).norm();

            // Skill-based distance preference
            let distance_bonus = if is_long_ball_keeper {
                // Long ball keeper: heavily prefers long passes
                if distance > 100.0 {
                    3.0 // Very long pass - excellent
                } else if distance > 60.0 {
                    2.0 // Long pass - good
                } else if distance > 30.0 {
                    0.8 // Medium pass - less preferred
                } else {
                    0.3 // Short pass - avoid
                }
            } else if is_technical_keeper {
                // Technical keeper: balanced approach, builds from back
                if distance > 100.0 {
                    1.5 // Very long pass - occasional
                } else if distance > 60.0 {
                    1.8 // Long pass - good option
                } else if distance > 30.0 {
                    2.0 // Medium pass - preferred for build-up
                } else {
                    1.5 // Short pass - safe option
                }
            } else if is_cautious_keeper {
                // Cautious keeper: prefers safe, short-medium passes
                if distance > 100.0 {
                    0.5 // Very long pass - risky, avoid
                } else if distance > 60.0 {
                    0.8 // Long pass - risky
                } else if distance > 30.0 {
                    1.5 // Medium pass - acceptable
                } else {
                    2.5 // Short pass - safe choice
                }
            } else {
                // Average keeper: standard preference
                if distance > 100.0 {
                    2.0
                } else if distance > 60.0 {
                    1.5
                } else if distance > 30.0 {
                    1.0
                } else {
                    0.5
                }
            };

            // Skill-based position preference
            let position_bonus = match teammate.tactical_positions.position_group() {
                crate::PlayerFieldPositionGroup::Forward => {
                    if is_visionary_keeper {
                        3.0 // Visionary keepers love finding forwards
                    } else if is_long_ball_keeper {
                        2.8 // Long ball keepers target forwards
                    } else if is_technical_keeper {
                        1.5 // Technical keepers less direct
                    } else {
                        2.0
                    }
                }
                crate::PlayerFieldPositionGroup::Midfielder => {
                    if is_technical_keeper {
                        2.5 // Technical keepers love midfield build-up
                    } else if is_cautious_keeper {
                        2.0 // Safe option for cautious keepers
                    } else {
                        1.5
                    }
                }
                crate::PlayerFieldPositionGroup::Defender => {
                    if is_cautious_keeper {
                        2.2 // Cautious keepers prefer defenders
                    } else if is_technical_keeper {
                        1.8 // Part of build-up play
                    } else {
                        0.6 // Others avoid defenders
                    }
                }
                crate::PlayerFieldPositionGroup::Goalkeeper => 0.1,
            };

            // Check if receiver is in space
            let nearby_opponents = ctx.tick_context
                .distances
                .opponents(teammate.id, 10.0)
                .count();

            let space_bonus = if nearby_opponents == 0 {
                2.0 // Completely free
            } else if nearby_opponents == 1 {
                if is_cautious_keeper {
                    0.8 // Cautious keepers avoid any pressure
                } else if is_technical_keeper {
                    1.4 // Technical keepers trust receiver's control
                } else {
                    1.2
                }
            } else {
                if is_cautious_keeper {
                    0.3 // Heavily avoid for cautious keepers
                } else {
                    0.6
                }
            };

            // Forward progress preference (skill-based)
            let forward_progress = teammate.position.x - ctx.player.position.x;
            let forward_bonus = if forward_progress > 0.0 {
                let base_forward = 1.0 + (forward_progress / ctx.context.field_size.width as f32) * 0.5;
                if is_visionary_keeper || is_long_ball_keeper {
                    base_forward * 1.3 // Aggressive forward passing
                } else if is_cautious_keeper {
                    base_forward * 0.8 // Less emphasis on forward progress
                } else {
                    base_forward
                }
            } else {
                if is_cautious_keeper {
                    0.7 // More willing to pass back
                } else if is_technical_keeper {
                    0.5 // Build-up allows some backward passes
                } else {
                    0.2 // Others avoid backward passes
                }
            };

            // Skill multipliers based on keeper abilities
            let skill_factor = if is_technical_keeper {
                (pass_skill * 0.5) + (vision_skill * 0.3) + (decision_skill * 0.2)
            } else if is_long_ball_keeper {
                (kicking_skill * 0.6) + (pass_skill * 0.2) + (vision_skill * 0.2)
            } else if is_visionary_keeper {
                (vision_skill * 0.5) + (anticipation_skill * 0.3) + (pass_skill * 0.2)
            } else if is_cautious_keeper {
                (composure_skill * 0.4) + (decision_skill * 0.4) + (pass_skill * 0.2)
            } else {
                (pass_skill * 0.4) + (vision_skill * 0.4) + (kicking_skill * 0.2)
            };

            // Calculate final score with skill-based weighting
            let score = distance_bonus * position_bonus * space_bonus * forward_bonus * skill_factor;

            if score > best_score {
                best_score = score;
                best_option = Some(teammate);
            }
        }

        best_option
    }

    pub fn calculate_pass_power(&self, teammate_id: u32, ctx: &StateProcessingContext) -> f64 {
        let distance = ctx.tick_context.distances.get(ctx.player.id, teammate_id);

        let pass_skill = ctx.player.skills.technical.passing;

        (distance / pass_skill as f32 * 10.0) as f64
    }
}
