use crate::r#match::defenders::states::DefenderState;
use crate::r#match::defenders::states::common::{DefenderCondition, ActivityIntensity};
use crate::r#match::{
    ConditionContext, MatchPlayerLite, PlayerDistanceFromStartPosition, PlayerSide,
    StateChangeResult, StateProcessingContext, StateProcessingHandler, SteeringBehavior,
};
use crate::IntegerUtils;
use nalgebra::Vector3;

const MAX_SHOOTING_DISTANCE: f32 = 80.0; // Defenders rarely shoot, only from close range

#[derive(Default)]
pub struct DefenderRunningState {}

impl StateProcessingHandler for DefenderRunningState {
    fn try_fast(&self, ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        if ctx.player.has_ball(ctx) {
            if self.is_in_shooting_range(ctx) {
                return Some(StateChangeResult::with_defender_state(
                    DefenderState::Shooting,
                ));
            }

            if self.has_clear_shot(ctx) {
                return Some(StateChangeResult::with_defender_state(
                    DefenderState::Shooting,
                ));
            }

            if self.should_pass(ctx) {
                return Some(StateChangeResult::with_defender_state(
                    DefenderState::Passing,
                ));
            }

            if self.should_clear(ctx) {
                return Some(StateChangeResult::with_defender_state(
                    DefenderState::Clearing,
                ));
            }
        } else {
            if ctx.player().position_to_distance() == PlayerDistanceFromStartPosition::Big {
                return Some(StateChangeResult::with_defender_state(
                    DefenderState::Returning,
                ));
            }

            // Only tackle if an opponent has the ball
            if let Some(_opponent) = ctx.players().opponents().with_ball().next() {
                if ctx.ball().distance() < 200.0 {
                    return Some(StateChangeResult::with_defender_state(
                        DefenderState::Tackling,
                    ));
                }
            }

            // Loose ball nearby — go claim it directly
            if !ctx.ball().is_owned() && ctx.ball().distance() < 50.0 && ctx.ball().speed() < 3.0 {
                return Some(StateChangeResult::with_defender_state(
                    DefenderState::TakeBall,
                ));
            }

            // Notification system: if ball system notified us to take the ball, act immediately
            if ctx.ball().should_take_ball_immediately() {
                return Some(StateChangeResult::with_defender_state(
                    DefenderState::TakeBall,
                ));
            }

            if !ctx.ball().is_owned() && self.should_intercept(ctx) {
                return Some(StateChangeResult::with_defender_state(
                    DefenderState::Intercepting,
                ));
            }
        }

        None
    }

    fn process_slow(&self, _ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        None
    }

    fn velocity(&self, ctx: &StateProcessingContext) -> Option<Vector3<f32>> {
        if ctx.player.should_follow_waypoints(ctx) {
            let waypoints = ctx.player.get_waypoints_as_vectors();

            if !waypoints.is_empty() {
                return Some(
                    SteeringBehavior::FollowPath {
                        waypoints,
                        current_waypoint: ctx.player.waypoint_manager.current_index,
                        path_offset: IntegerUtils::random(1, 10) as f32,
                    }
                    .calculate(ctx.player)
                    .velocity
                        + ctx.player().separation_velocity(),
                );
            }
        }

        Some(
            SteeringBehavior::Arrive {
                target: ctx.player().opponent_goal_position(),
                slowing_distance: if ctx.player.has_ball(ctx) {
                    150.0
                } else {
                    100.0
                },
            }
            .calculate(ctx.player)
            .velocity
                + ctx.player().separation_velocity(),
        )
    }

    fn process_conditions(&self, ctx: ConditionContext) {
        // Running is physically demanding - reduce condition based on intensity and player's stamina
        // Use velocity-based calculation to account for sprinting vs jogging
        DefenderCondition::with_velocity(ActivityIntensity::High).process(ctx);
    }
}

impl DefenderRunningState {
    pub fn should_clear(&self, ctx: &StateProcessingContext) -> bool {
        // Original: Clear if in own penalty area with nearby opponents
        if ctx.ball().in_own_penalty_area() && ctx.players().opponents().exists(100.0) {
            return true;
        }

        // Clear if congested anywhere (not just boundaries)
        if self.is_congested_near_boundary(ctx) || ctx.player().movement().is_congested() {
            return true;
        }

        false
    }

    /// Check if player is stuck in a corner/boundary with multiple players around
    fn is_congested_near_boundary(&self, ctx: &StateProcessingContext) -> bool {
        // Check if near any boundary (within 20 units)
        let field_width = ctx.context.field_size.width as f32;
        let field_height = ctx.context.field_size.height as f32;
        let pos = ctx.player.position;

        let near_boundary = pos.x < 20.0
            || pos.x > field_width - 20.0
            || pos.y < 20.0
            || pos.y > field_height - 20.0;

        if !near_boundary {
            return false;
        }

        // Count all nearby players (teammates + opponents) within 15 units
        let nearby_teammates = ctx.players().teammates().nearby(15.0).count();
        let nearby_opponents = ctx.players().opponents().nearby(15.0).count();
        let total_nearby = nearby_teammates + nearby_opponents;

        // If 3 or more players nearby (congestion), need to clear
        total_nearby >= 3
    }

    pub fn should_pass(&self, ctx: &StateProcessingContext) -> bool {
        if ctx.players().opponents().exists(20.0) {
            return true;
        }

        let game_vision_skill = ctx.player.skills.mental.vision;
        let game_vision_threshold = 14.0; // Adjust this value based on your game balance

        if game_vision_skill >= game_vision_threshold {
            if let Some(_) = self.find_open_teammate_on_opposite_side(ctx) {
                return true;
            }
        }

        false
    }

    fn find_open_teammate_on_opposite_side(
        &self,
        ctx: &StateProcessingContext,
    ) -> Option<MatchPlayerLite> {
        let player_position = ctx.player.position;
        let field_width = ctx.context.field_size.width as f32;
        let opposite_side_x = match ctx.player.side {
            Some(PlayerSide::Left) => field_width * 0.75,
            Some(PlayerSide::Right) => field_width * 0.25,
            None => return None,
        };

        let mut open_teammates: Vec<MatchPlayerLite> = ctx
            .players()
            .teammates()
            .nearby(200.0)
            .filter(|teammate| {
                if teammate.tactical_positions.is_goalkeeper() {
                    return false;
                }

                let is_on_opposite_side = match ctx.player.side {
                    Some(PlayerSide::Left) => teammate.position.x > opposite_side_x,
                    Some(PlayerSide::Right) => teammate.position.x < opposite_side_x,
                    None => false,
                };
                let is_open = !ctx
                    .players()
                    .opponents()
                    .nearby(20.0)
                    .any(|opponent| opponent.id == teammate.id);
                is_on_opposite_side && is_open
            })
            .collect();

        if open_teammates.is_empty() {
            None
        } else {
            open_teammates.sort_by(|a, b| {
                let dist_a = (a.position - player_position).magnitude();
                let dist_b = (b.position - player_position).magnitude();
                dist_a.partial_cmp(&dist_b).unwrap()
            });
            Some(open_teammates[0])
        }
    }

    fn is_in_shooting_range(&self, ctx: &StateProcessingContext) -> bool {
        let distance_to_goal = ctx.ball().distance_to_opponent_goal();

        distance_to_goal <= MAX_SHOOTING_DISTANCE && ctx.player().has_clear_shot()
    }

    fn has_clear_shot(&self, ctx: &StateProcessingContext) -> bool {
        if ctx.ball().distance_to_opponent_goal() < MAX_SHOOTING_DISTANCE {
            return ctx.player().has_clear_shot();
        }

        false
    }

    fn should_intercept(&self, ctx: &StateProcessingContext) -> bool {
        // Don't intercept if a teammate has the ball
        if let Some(owner_id) = ctx.ball().owner_id() {
            if let Some(owner) = ctx.context.players.by_id(owner_id) {
                if owner.team_id == ctx.player.team_id {
                    // A teammate has the ball, don't try to intercept
                    return false;
                }
            }
        }

        // Only intercept if you're the best player to chase the ball
        if !ctx.team().is_best_player_to_chase_ball() {
            return false;
        }

        // Check if the ball is moving toward this player and is close enough
        if ctx.ball().distance() < 200.0 && ctx.ball().is_towards_player_with_angle(0.8) {
            return true;
        }

        // Check if the ball is very close and no teammate is clearly going for it
        if ctx.ball().distance() < 50.0 && !ctx.team().is_teammate_chasing_ball() {
            return true;
        }

        false
    }
}
