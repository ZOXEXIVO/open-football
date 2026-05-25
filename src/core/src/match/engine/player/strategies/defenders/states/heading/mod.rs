use crate::r#match::defenders::states::DefenderState;
use crate::r#match::defenders::states::common::{ActivityIntensity, DefenderCondition};
use crate::r#match::player::events::{PlayerEvent, ShootingEventContext};
use crate::r#match::player::strategies::players::ShotType;
use crate::r#match::{
    ConditionContext, StateChangeResult, StateProcessingContext, StateProcessingHandler,
    SteeringBehavior,
};
use nalgebra::Vector3;

use crate::r#match::events::Event;

/// Within this distance of the opponent goal a header is an attacking
/// attempt (a centre-back up for a corner), so it's directed ON GOAL
/// rather than cleared. Anywhere else (the usual case — defending a cross
/// in our own box) the header is a clearance away from our own goal.
const ATTACKING_HEADER_RANGE: f32 = 90.0;

const HEADING_HEIGHT_THRESHOLD: f32 = 1.5; // Minimum height to consider heading (meters)
const HEADING_DISTANCE_THRESHOLD: f32 = 1.5; // Maximum distance to the ball for heading (meters)
#[allow(dead_code)]
const HEADING_SUCCESS_THRESHOLD: f32 = 0.5; // Threshold for heading success based on skills

#[derive(Default, Clone)]
pub struct DefenderHeadingState {}

impl StateProcessingHandler for DefenderHeadingState {
    fn process(&self, ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        let ball_position = ctx.tick_context.positions.ball.position;

        // During an attacking corner, keep contesting the delivery rather
        // than dropping out of the box — return to AttackingCorner so the
        // CB pursues the ball / second ball instead of holding the line.
        let attacking_corner = ctx.ball().is_team_attacking_corner();

        if ctx.ball().distance() > HEADING_DISTANCE_THRESHOLD {
            return Some(StateChangeResult::with_defender_state(if attacking_corner {
                DefenderState::AttackingCorner
            } else {
                DefenderState::HoldingLine
            }));
        }

        // Check if the ball is at a height suitable for heading
        if ball_position.z < HEADING_HEIGHT_THRESHOLD {
            // Ball is too low to head
            return Some(StateChangeResult::with_defender_state(if attacking_corner {
                DefenderState::AttackingCorner
            } else {
                DefenderState::Standing
            }));
        }

        // 2. Attempt to head the ball
        if self.attempt_heading(ctx) {
            // Attacking header: a centre-back up for a corner near the
            // opponent goal heads ON GOAL (marked as a Header for xG),
            // returning to AttackingCorner to keep contesting the
            // second ball. Everywhere else this is a defensive clearance
            // away from our own goal.
            if ctx.ball().distance_to_opponent_goal() < ATTACKING_HEADER_RANGE {
                #[cfg(feature = "match-logs")]
                {
                    use std::sync::atomic::Ordering;
                    crate::r#match::player::strategies::common::players::ops::forward_shot_decision::mid_run_diag::DEF_CORNER_HEADER.fetch_add(1, Ordering::Relaxed);
                }
                Some(StateChangeResult::with_defender_state_and_event(
                    DefenderState::AttackingCorner,
                    Event::PlayerEvent(PlayerEvent::Shoot(
                        ShootingEventContext::new()
                            .with_player_id(ctx.player.id)
                            .with_target(ctx.player().shooting_direction())
                            .with_reason("DEF_HEADER_ON_GOAL")
                            .with_shot_type(ShotType::Header)
                            .build(ctx),
                    )),
                ))
            } else {
                // Defenders clear the ball AWAY from own goal, not toward opponent goal
                Some(StateChangeResult::with_defender_state_and_event(
                    DefenderState::HoldingLine,
                    Event::PlayerEvent(PlayerEvent::Shoot(
                        ShootingEventContext::new()
                            .with_player_id(ctx.player.id)
                            .with_target(ctx.player().clearing_direction())
                            .with_reason("DEF_HEADING")
                            .build(ctx),
                    )),
                ))
            }
        } else {
            // Heading failed; transition to appropriate state (e.g., Standing)
            Some(StateChangeResult::with_defender_state(
                DefenderState::Standing,
            ))
        }
    }

    fn velocity(&self, ctx: &StateProcessingContext) -> Option<Vector3<f32>> {
        let ball_position = ctx.tick_context.positions.ball.position;
        Some(
            SteeringBehavior::Arrive {
                target: ball_position,
                slowing_distance: 3.0,
            }
            .calculate(ctx.player)
            .velocity,
        )
    }

    fn process_conditions(&self, ctx: ConditionContext) {
        // Heading involves jumping and explosive neck/body movement
        DefenderCondition::new(ActivityIntensity::VeryHigh).process(ctx);
    }
}

impl DefenderHeadingState {
    /// Determines if the defender successfully heads the ball based on skills and random chance.
    fn attempt_heading(&self, ctx: &StateProcessingContext) -> bool {
        let heading_skill = ctx.player.skills.technical.heading / 20.0; // Normalize skill to [0,1]
        let jumping_skill = ctx.player.skills.physical.jumping / 20.0;
        let overall_skill = (heading_skill + jumping_skill) / 2.0;

        // Simulate chance of success
        let random_value: f32 = rand::random(); // Generates a random float between 0.0 and 1.0

        random_value < overall_skill
    }
}
