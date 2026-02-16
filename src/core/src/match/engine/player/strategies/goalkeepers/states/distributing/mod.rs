use crate::r#match::events::Event;
use crate::r#match::goalkeepers::states::common::{ActivityIntensity, GoalkeeperCondition};
use crate::r#match::goalkeepers::states::state::GoalkeeperState;
use crate::r#match::player::events::{PassingEventContext, PlayerEvent};
use crate::r#match::{ConditionContext, MatchPlayerLite, StateChangeResult, StateProcessingContext, StateProcessingHandler};
use nalgebra::Vector3;

#[derive(Default)]
pub struct GoalkeeperDistributingState {}

impl StateProcessingHandler for GoalkeeperDistributingState {
    fn try_fast(&self, ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        // If we no longer have the ball, we must have passed or lost it
        if !ctx.player.has_ball(ctx) {
            return Some(StateChangeResult::with_goalkeeper_state(
                GoalkeeperState::Standing,
            ));
        }

        // Try to find the best pass option
        if let Some(teammate) = self.find_best_pass_option(ctx) {
            // Execute the pass and transition to returning to goal
            return Some(StateChangeResult::with_goalkeeper_state_and_event(
                GoalkeeperState::ReturningToGoal,
                Event::PlayerEvent(PlayerEvent::PassTo(
                    PassingEventContext::new()
                        .with_from_player_id(ctx.player.id)
                        .with_to_player_id(teammate.id)
                        .with_reason("GK_DISTRIBUTING")
                        .build(ctx)
                )),
            ));
        }

        // Timeout after a short time if no pass is made
        // This prevents the goalkeeper from being stuck trying to pass forever
        if ctx.in_state_time > 20 {
            // If we still have the ball after timeout, try running to find space
            return Some(StateChangeResult::with_goalkeeper_state(
                GoalkeeperState::Running,
            ));
        }

        // If we have the ball but no good passing option yet, wait
        // The goalkeeper should not be trying to catch the ball since they already have it
        None
    }

    fn process_slow(&self, _ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        None
    }

    fn velocity(&self, _ctx: &StateProcessingContext) -> Option<Vector3<f32>> {
        Some(Vector3::new(0.0, 0.0, 0.0))
    }

    fn process_conditions(&self, ctx: ConditionContext) {
        // Distributing requires moderate intensity with focused effort
        GoalkeeperCondition::new(ActivityIntensity::Moderate).process(ctx);
    }
}

impl GoalkeeperDistributingState {
    fn find_best_pass_option<'a>(&'a self, ctx: &'a StateProcessingContext<'a>) -> Option<MatchPlayerLite> {
        // Goalkeepers should look for long passes to start attacks
        // Search the entire field including ultra-long distances for goal kicks
        let max_distance = ctx.context.field_size.width as f32 * 2.5; // Extended for 300m+ passes

        // Get goalkeeper's skills to determine passing style
        let pass_skill = ctx.player.skills.technical.passing / 20.0;
        let vision_skill = ctx.player.skills.mental.vision / 20.0;
        let kicking_skill = ctx.player.skills.technical.long_throws / 20.0;
        let decision_skill = ctx.player.skills.mental.decisions / 20.0;
        let composure_skill = ctx.player.skills.mental.composure / 20.0;
        let anticipation_skill = ctx.player.skills.mental.anticipation / 20.0;
        let technique_skill = ctx.player.skills.technical.technique / 20.0;

        // Determine goalkeeper passing style based on skills
        let is_technical_keeper = pass_skill > 0.7 && vision_skill > 0.7; // Likes build-up play
        let is_long_ball_keeper = kicking_skill > 0.7 && pass_skill < 0.6; // Prefers long kicks
        let is_cautious_keeper = composure_skill < 0.5 || decision_skill < 0.5; // Safe, short passes
        let is_visionary_keeper = vision_skill > 0.8 && anticipation_skill > 0.7; // Sees through balls
        let is_elite_distributor = vision_skill > 0.85 && technique_skill > 0.8 && kicking_skill > 0.75; // Can attempt extreme passes

        let mut best_option: Option<MatchPlayerLite> = None;
        let mut best_score = 0.0;

        for teammate in ctx.players().teammates().nearby(max_distance) {
            // GRADUATED RECENCY PENALTY: Penalize recent passers instead of hard-skipping
            let recency_penalty = ctx.ball().passer_recency_penalty(teammate.id);

            let distance = (teammate.position - ctx.player.position).norm();

            // PREVENT PASSING TO PLAYERS TOO CLOSE TO GOALKEEPER (creates ping-pong)
            if distance < 50.0 {
                continue; // Skip players too close to goalkeeper
            }

            // Skill-based distance preference with ultra-long pass support
            let distance_bonus = if is_elite_distributor {
                // Elite distributor: can attempt any distance with vision-based weighting
                if distance > 300.0 {
                    // Extreme passes - only elite keepers should attempt
                    let extreme_confidence = (vision_skill * 0.5) + (kicking_skill * 0.3) + (technique_skill * 0.2);
                    2.5 + extreme_confidence * 2.0 // Up to 4.5 for world-class keepers
                } else if distance > 200.0 {
                    // Ultra-long passes - elite specialty
                    let ultra_confidence = (vision_skill * 0.6) + (kicking_skill * 0.4);
                    3.0 + ultra_confidence * 1.5 // Up to 4.5
                } else if distance > 100.0 {
                    3.5 // Very long - excellent
                } else if distance > 60.0 {
                    2.8 // Long - good
                } else if distance > 30.0 {
                    2.0 // Medium - acceptable
                } else {
                    1.5 // Short - for build-up
                }
            } else if is_long_ball_keeper {
                // Long ball keeper: heavily prefers long passes, vision limits ultra-long
                if distance > 300.0 {
                    // Extreme passes - limited by vision
                    if vision_skill > 0.7 {
                        2.5 + (vision_skill - 0.7) * 3.0 // Up to 3.4
                    } else {
                        0.8 // Avoid without vision
                    }
                } else if distance > 200.0 {
                    // Ultra-long - good kicking but needs some vision
                    if vision_skill > 0.6 {
                        3.0 + (vision_skill - 0.6) * 2.0 // Up to 3.8
                    } else {
                        1.5
                    }
                } else if distance > 100.0 {
                    3.5 // Very long pass - excellent
                } else if distance > 60.0 {
                    2.5 // Long pass - good
                } else if distance > 30.0 {
                    0.8 // Medium pass - less preferred
                } else {
                    0.3 // Short pass - avoid
                }
            } else if is_visionary_keeper {
                // Visionary keeper: sees opportunities at all ranges
                if distance > 300.0 {
                    // Extreme passes - vision-driven
                    let vision_multiplier = (vision_skill - 0.8) * 5.0; // 0.0 to 1.0
                    2.0 + vision_multiplier + (kicking_skill * 1.5)
                } else if distance > 200.0 {
                    // Ultra-long - perfect for visionary
                    2.8 + (vision_skill * 1.5)
                } else if distance > 100.0 {
                    3.2 // Very long - sees through balls
                } else if distance > 60.0 {
                    2.5 // Long - good vision
                } else if distance > 30.0 {
                    2.0 // Medium - builds play
                } else {
                    1.8 // Short - safe
                }
            } else if is_technical_keeper {
                // Technical keeper: balanced approach, builds from back
                if distance > 300.0 {
                    // Extreme passes - rare for technical keepers
                    if vision_skill > 0.75 && kicking_skill > 0.7 {
                        1.5
                    } else {
                        0.5 // Avoid
                    }
                } else if distance > 200.0 {
                    // Ultra-long - occasional if skilled
                    if vision_skill > 0.7 {
                        1.8
                    } else {
                        0.8
                    }
                } else if distance > 100.0 {
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
                if distance > 300.0 || distance > 200.0 {
                    0.2 // Ultra/extreme passes - too risky, avoid
                } else if distance > 100.0 {
                    0.5 // Very long pass - risky, avoid
                } else if distance > 60.0 {
                    0.8 // Long pass - risky
                } else if distance > 30.0 {
                    1.5 // Medium pass - acceptable
                } else {
                    2.5 // Short pass - safe choice
                }
            } else {
                // Average keeper: standard preference with limited ultra-long ability
                if distance > 300.0 {
                    // Extreme passes - very limited
                    if vision_skill > 0.7 && kicking_skill > 0.7 {
                        1.2
                    } else {
                        0.4
                    }
                } else if distance > 200.0 {
                    // Ultra-long - needs good skills
                    if vision_skill > 0.65 {
                        1.5
                    } else {
                        0.7
                    }
                } else if distance > 100.0 {
                    2.0
                } else if distance > 60.0 {
                    1.5
                } else if distance > 30.0 {
                    1.0
                } else {
                    0.5
                }
            };

            // Skill-based position preference with distance consideration
            let position_bonus = match teammate.tactical_positions.position_group() {
                crate::PlayerFieldPositionGroup::Forward => {
                    // Ultra-long passes to forwards are more valuable
                    let ultra_long_multiplier = if distance > 300.0 {
                        1.5 // Extreme distance to striker - game-changing
                    } else if distance > 200.0 {
                        1.3 // Ultra-long to striker - counter-attack
                    } else {
                        1.0
                    };

                    if is_elite_distributor {
                        3.5 * ultra_long_multiplier // Elite keepers excel at finding forwards
                    } else if is_visionary_keeper {
                        3.0 * ultra_long_multiplier // Visionary keepers love finding forwards
                    } else if is_long_ball_keeper {
                        2.8 * ultra_long_multiplier // Long ball keepers target forwards
                    } else if is_technical_keeper {
                        1.5 // Technical keepers less direct
                    } else {
                        2.0
                    }
                }
                crate::PlayerFieldPositionGroup::Midfielder => {
                    // Medium to long passes to midfield
                    let long_pass_multiplier = if distance > 200.0 {
                        0.8 // Less ideal for ultra-long to midfield
                    } else if distance > 100.0 {
                        1.2 // Good for switching play
                    } else {
                        1.0
                    };

                    if is_technical_keeper {
                        2.5 * long_pass_multiplier // Technical keepers love midfield build-up
                    } else if is_cautious_keeper {
                        2.0 * long_pass_multiplier // Safe option for cautious keepers
                    } else if is_elite_distributor && distance > 150.0 {
                        2.2 * long_pass_multiplier // Elite can switch play through midfield
                    } else {
                        1.5
                    }
                }
                crate::PlayerFieldPositionGroup::Defender => {
                    // Short passes to defenders, avoid long ones
                    if distance > 200.0 {
                        0.3 // Never ultra-long pass to defender
                    } else if distance > 100.0 {
                        0.5 // Rarely long pass to defender
                    } else if is_cautious_keeper {
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
                .opponents(teammate.id, 15.0)
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

            // Calculate final score with skill-based weighting and recency penalty
            let score = distance_bonus * position_bonus * space_bonus * forward_bonus * skill_factor * recency_penalty;

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

        (distance / pass_skill * 10.0) as f64
    }
}
