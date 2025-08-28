use crate::IntegerUtils;
use crate::r#match::midfielders::states::MidfielderState;
use crate::r#match::{
    ConditionContext, MatchPlayerLite, PlayerSide, StateChangeResult, StateProcessingContext,
    StateProcessingHandler, SteeringBehavior,
};
use nalgebra::Vector3;

const MAX_SHOOTING_DISTANCE: f32 = 300.0; // Maximum distance to attempt a shot
const MIN_SHOOTING_DISTANCE: f32 = 10.0; // Minimum distance to attempt a shot (e.g., edge of penalty area)

#[derive(Default)]
pub struct MidfielderRunningState {}

impl StateProcessingHandler for MidfielderRunningState {
    fn try_fast(&self, ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        if ctx.player.has_ball(ctx) {
            if self.has_clear_shot(ctx) {
                return Some(StateChangeResult::with_midfielder_state(
                    MidfielderState::Shooting,
                ));
            }

            if self.in_long_distance_shooting_range(ctx) {
                return Some(StateChangeResult::with_midfielder_state(
                    MidfielderState::DistanceShooting,
                ));
            }

            if self.in_shooting_range(ctx) {
                return Some(StateChangeResult::with_midfielder_state(
                    MidfielderState::Shooting,
                ));
            }

            if self.should_pass(ctx) {
                return Some(StateChangeResult::with_midfielder_state(
                    MidfielderState::Passing,
                ));
            }

            if self.should_dribble(ctx) {
                return Some(StateChangeResult::with_midfielder_state(
                    MidfielderState::Dribbling,
                ));
            }

            if self.is_under_pressure(ctx) {
                return Some(StateChangeResult::with_midfielder_state(
                    MidfielderState::Passing,
                ));
            }
        } else {
            if self.should_intercept(ctx) {
                return Some(StateChangeResult::with_midfielder_state(
                    MidfielderState::Intercepting,
                ));
            }

            if self.should_press(ctx) {
                return Some(StateChangeResult::with_midfielder_state(
                    MidfielderState::Pressing,
                ));
            }

            if self.should_support_attack(ctx) {
                return Some(StateChangeResult::with_midfielder_state(
                    MidfielderState::AttackSupporting,
                ));
            }

            if self.should_return_to_position(ctx) {
                return Some(StateChangeResult::with_midfielder_state(
                    MidfielderState::Returning,
                ));
            }

            if self.is_under_pressure(ctx) {
                return Some(StateChangeResult::with_midfielder_state(
                    MidfielderState::Passing,
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
                    .velocity,
                );
            }
        }

        if let Some(target_position) = self.find_space_between_opponents(ctx) {
            Some(
                SteeringBehavior::Arrive {
                    target: target_position + ctx.player().separation_velocity(),
                    slowing_distance: 10.0,
                }
                .calculate(ctx.player)
                .velocity,
            )
        } else if ctx.player.has_ball(ctx) {
            Some(
                SteeringBehavior::Arrive {
                    target: ctx.player().opponent_goal_position()
                        + ctx.player().separation_velocity(),
                    slowing_distance: 100.0,
                }
                .calculate(ctx.player)
                .velocity,
            )
        } else if ctx.team().is_control_ball() {
            Some(
                SteeringBehavior::Arrive {
                    target: ctx.player().opponent_goal_position()
                        + ctx.player().separation_velocity(),
                    slowing_distance: 100.0,
                }
                .calculate(ctx.player)
                .velocity,
            )
        } else {
            Some(
                SteeringBehavior::Wander {
                    target: ctx.player.start_position + ctx.player().separation_velocity(),
                    radius: IntegerUtils::random(5, 150) as f32,
                    jitter: IntegerUtils::random(0, 2) as f32,
                    distance: IntegerUtils::random(10, 150) as f32,
                    angle: IntegerUtils::random(0, 360) as f32,
                }
                .calculate(ctx.player)
                .velocity,
            )
        }
    }

    fn process_conditions(&self, _ctx: ConditionContext) {}
}

impl MidfielderRunningState {
    fn in_long_distance_shooting_range(&self, ctx: &StateProcessingContext) -> bool {
        (MIN_SHOOTING_DISTANCE..=MAX_SHOOTING_DISTANCE)
            .contains(&ctx.ball().distance_to_opponent_goal())
    }

    fn in_shooting_range(&self, ctx: &StateProcessingContext) -> bool {
        (MIN_SHOOTING_DISTANCE..=MAX_SHOOTING_DISTANCE)
            .contains(&ctx.ball().distance_to_opponent_goal())
    }

    fn has_clear_shot(&self, ctx: &StateProcessingContext) -> bool {
        if ctx.ball().distance_to_opponent_goal() < MAX_SHOOTING_DISTANCE {
            return ctx.player().has_clear_shot();
        }

        false
    }

    fn should_intercept(&self, ctx: &StateProcessingContext) -> bool {
        if ctx.ball().is_owned() {
            return false;
        }

        if ctx.ball().distance() < 200.0 && ctx.ball().is_towards_player_with_angle(0.8) {
            return true;
        }

        if ctx.ball().distance() < 100.0 {
            return true;
        }

        false
    }

    fn should_press(&self, ctx: &StateProcessingContext) -> bool {
        let pressing_distance = 100.0;

        !ctx.team().is_control_ball()
            && ctx.ball().distance() < pressing_distance
            && ctx.ball().is_towards_player_with_angle(0.8)
    }

    pub fn should_pass(&self, ctx: &StateProcessingContext) -> bool {
        if ctx.players().opponents().exists(15.0) {
            return true;
        }

        let game_vision_threshold = 14.0;

        if ctx.player.skills.mental.vision >= game_vision_threshold {
            return self.find_open_teammate_on_opposite_side(ctx).is_some();
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

    fn find_space_between_opponents(&self, ctx: &StateProcessingContext) -> Option<Vector3<f32>> {
        let players = ctx.players();
        let opponents = players.opponents();

        let mut nearest_opponents = opponents.nearby_raw(200.0);

        if let Some((first_id, _)) = nearest_opponents.next() {
            while let Some((second_id, _)) = nearest_opponents.next() {
                if first_id == second_id {
                    continue;
                }
                let distance_between_opponents =
                    ctx.tick_context.distances.get(first_id, second_id);
                if distance_between_opponents > 10.0 {
                    let first_position = ctx.tick_context.positions.players.position(first_id);
                    let second_position = ctx.tick_context.positions.players.position(second_id);

                    let midpoint = (first_position + second_position) * 0.5;

                    return Some(midpoint);
                }
            }
        }

        None
    }

    fn should_dribble(&self, ctx: &StateProcessingContext) -> bool {
        self.is_under_pressure(ctx)
    }

    fn should_support_attack(&self, ctx: &StateProcessingContext) -> bool {
        // Basic requirement: team must be in possession
        if !ctx.team().is_control_ball() {
            return false;
        }

        // Get player's mental attributes
        let vision = ctx.player.skills.mental.vision;
        let positioning = ctx.player.skills.mental.positioning;
        let teamwork = ctx.player.skills.mental.teamwork;
        let decisions = ctx.player.skills.mental.decisions;

        // Get physical attributes that affect ability to support
        let pace = ctx.player.skills.physical.pace;
        let stamina = ctx.player.skills.physical.stamina;
        let current_stamina = ctx.player.player_attributes.condition_percentage() as f32;

        // Calculate tactical intelligence - combination of mental attributes
        let tactical_intelligence = (vision + positioning + teamwork + decisions) / 40.0;

        // Players with lower tactical intelligence have stricter requirements
        let intelligence_threshold = if tactical_intelligence < 10.0 {
            // Low intelligence players only support when ball is very close
            50.0
        } else if tactical_intelligence < 14.0 {
            // Average intelligence players support when ball is moderately close
            120.0
        } else {
            // High intelligence players can read the game and support from further
            200.0
        };

        // Check if ball is within the player's tactical range
        let ball_distance = ctx.ball().distance();
        if ball_distance > intelligence_threshold {
            return false;
        }

        // Vision affects ability to see attacking opportunities
        let vision_range = vision * 15.0; // Better vision = see opportunities from further

        // Check if there are attacking teammates within vision range
        let attacking_teammates_nearby = ctx.players()
            .teammates()
            .nearby(vision_range)
            .filter(|teammate| {
                // Only consider forwards and attacking midfielders
                teammate.tactical_positions.is_forward() ||
                    (teammate.tactical_positions.is_midfielder() &&
                        self.is_in_attacking_position(ctx, teammate))
            })
            .count();

        // Players with good vision can spot opportunities even with fewer attacking players
        let min_attacking_players = if vision >= 16.0 {
            1 // Excellent vision - can create something from nothing
        } else if vision >= 12.0 {
            2 // Good vision - needs some support
        } else {
            3 // Poor vision - needs obvious attacking situation
        };

        if attacking_teammates_nearby < min_attacking_players {
            return false;
        }

        // Check stamina - tired players are less likely to make attacking runs
        let stamina_factor = (current_stamina / 100.0) * (stamina / 20.0);
        if stamina_factor < 0.6 {
            return false; // Too tired to support attack effectively
        }

        // Positioning skill affects understanding of when to support
        let positional_awareness = positioning / 20.0;

        // Check if player is in a good position to support (not too defensive)
        let field_length = ctx.context.field_size.width as f32;
        let player_field_position = match ctx.player.side {
            Some(PlayerSide::Left) => ctx.player.position.x / field_length,
            Some(PlayerSide::Right) => (field_length - ctx.player.position.x) / field_length,
            None => 0.5,
        };

        // Players with good positioning understand when they're too far back
        let min_field_position = if positional_awareness >= 0.8 {
            0.3 // Excellent positioning - can support from deeper
        } else if positional_awareness >= 0.6 {
            0.4 // Good positioning - needs to be in middle third
        } else {
            0.5 // Poor positioning - needs to be in attacking half
        };

        if player_field_position < min_field_position {
            return false;
        }

        // Check pace - slower players need to be closer to be effective
        let pace_factor = pace / 20.0;
        let effective_distance = if pace_factor >= 0.8 {
            200.0 // Fast players can support from further
        } else if pace_factor >= 0.6 {
            150.0 // Average pace players need to be closer
        } else {
            100.0 // Slow players need to be quite close
        };

        if ball_distance > effective_distance {
            return false;
        }

        // Teamwork affects willingness to make selfless runs
        let teamwork_factor = teamwork / 20.0;

        // Players with poor teamwork are more selfish and less likely to support
        if teamwork_factor < 0.5 {
            // Selfish players only support when they might get glory (very close to goal)
            return ctx.ball().distance_to_opponent_goal() < 150.0;
        }

        // Decision making affects timing of support runs
        let decision_quality = decisions / 20.0;

        // Poor decision makers might support at wrong times
        if decision_quality < 0.5 {
            // Check if this is actually a good time to support (not when defending)
            let opponents_in_defensive_third = ctx.players()
                .opponents()
                .all()
                .filter(|opponent| self.is_in_defensive_third(ctx, opponent))
                .count();

            // If many opponents in defensive third, poor decision makers might still go forward
            if opponents_in_defensive_third >= 3 {
                return false;
            }
        }

        // All checks passed - this player should support the attack
        true
    }

    fn should_return_to_position(&self, ctx: &StateProcessingContext) -> bool {
        // Check if the player is far from their starting position and the team is not in possession
        let distance_from_start = ctx.player().distance_from_start_position();
        let team_in_possession = ctx.team().is_control_ball();

        distance_from_start > 20.0 && !team_in_possession
    }

    fn is_under_pressure(&self, ctx: &StateProcessingContext) -> bool {
        ctx.players().opponents().exists(25.0)
    }

    fn is_in_attacking_position(&self, ctx: &StateProcessingContext, teammate: &MatchPlayerLite) -> bool {
        let field_length = ctx.context.field_size.width as f32;
        let attacking_third_start = match ctx.player.side {
            Some(PlayerSide::Left) => field_length * (2.0 / 3.0),
            Some(PlayerSide::Right) => field_length / 3.0,
            None => field_length * 0.5,
        };

        match ctx.player.side {
            Some(PlayerSide::Left) => teammate.position.x > attacking_third_start,
            Some(PlayerSide::Right) => teammate.position.x < attacking_third_start,
            None => false,
        }
    }

    fn is_in_defensive_third(&self, ctx: &StateProcessingContext, opponent: &MatchPlayerLite) -> bool {
        let field_length = ctx.context.field_size.width as f32;
        let defensive_third_end = match ctx.player.side {
            Some(PlayerSide::Left) => field_length / 3.0,
            Some(PlayerSide::Right) => field_length * (2.0 / 3.0),
            None => field_length * 0.5,
        };

        match ctx.player.side {
            Some(PlayerSide::Left) => opponent.position.x < defensive_third_end,
            Some(PlayerSide::Right) => opponent.position.x > defensive_third_end,
            None => false,
        }
    }
}
