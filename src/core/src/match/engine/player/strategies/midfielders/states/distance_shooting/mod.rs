use crate::r#match::events::Event;
use crate::r#match::midfielders::states::common::{ActivityIntensity, MidfielderCondition};
use crate::r#match::midfielders::states::MidfielderState;
use crate::r#match::player::events::{PlayerEvent, ShootingEventContext};
use crate::r#match::{
    ConditionContext, MatchPlayerLite, PlayerSide, StateChangeResult, StateProcessingContext,
    StateProcessingHandler, SteeringBehavior,
};
use nalgebra::Vector3;

#[derive(Default)]
pub struct MidfielderDistanceShootingState {}

impl StateProcessingHandler for MidfielderDistanceShootingState {
    fn try_fast(&self, ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        // Check if the midfielder still has the ball
        if !ctx.player.has_ball(ctx) {
            // Lost possession, transition to Pressing
            return Some(StateChangeResult::with_midfielder_state(
                MidfielderState::Running,
            ));
        }

        if ctx.player().goal_distance() > 250.0 {
            // Too far from the goal, consider other options
            if self.should_pass(ctx) {
                return Some(StateChangeResult::with_midfielder_state(
                    MidfielderState::Passing,
                ));
            } else if self.should_dribble(ctx) {
                return Some(StateChangeResult::with_midfielder_state(
                    MidfielderState::Dribbling,
                ));
            }
        }

        // Evaluate shooting opportunity
        if self.is_favorable_shooting_opportunity(ctx) {
            // Transition to shooting state
            return Some(StateChangeResult::with_midfielder_state_and_event(
                MidfielderState::Shooting,
                Event::PlayerEvent(PlayerEvent::Shoot(
                    ShootingEventContext::new()
                        .with_player_id(ctx.player.id)
                        .with_target(ctx.player().shooting_direction())
                        .with_reason("MID_DISTANCE_SHOOTING")
                        .build(ctx),
                )),
            ));
        }

        None
    }

    fn process_slow(&self, _ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        None
    }

    fn velocity(&self, ctx: &StateProcessingContext) -> Option<Vector3<f32>> {
        Some(
            SteeringBehavior::Arrive {
                target: ctx.player().opponent_goal_position(),
                slowing_distance: 150.0,
            }
            .calculate(ctx.player)
            .velocity,
        )
    }

    fn process_conditions(&self, ctx: ConditionContext) {
        // Distance shooting is very high intensity - explosive action
        MidfielderCondition::new(ActivityIntensity::VeryHigh).process(ctx);
    }
}

impl MidfielderDistanceShootingState {
    fn is_favorable_shooting_opportunity(&self, ctx: &StateProcessingContext) -> bool {
        // Evaluate the shooting opportunity based on various factors
        let distance_to_goal = ctx.player().goal_distance();
        let angle_to_goal = ctx.player().goal_angle();
        let has_clear_shot = self.has_clear_shot(ctx);
        let long_shots = ctx.player.skills.technical.long_shots / 20.0;

        // Distance shooting only for skilled players from reasonable distance
        let distance_threshold = 100.0; // Maximum ~50m for long shots
        let angle_threshold = std::f32::consts::PI / 6.0; // 30 degrees

        distance_to_goal <= distance_threshold
            && angle_to_goal <= angle_threshold
            && has_clear_shot
            && long_shots > 0.55 // Reduced from 0.7 to 0.55 - more players can take long shots
    }

    fn should_pass(&self, ctx: &StateProcessingContext) -> bool {
        // Determine if the player should pass based on the game state

        let teammates = ctx.players().teammates();
        let mut open_teammates = teammates.all()
            .filter(|teammate| self.is_teammate_open(ctx, teammate));

        let has_open_teammate = open_teammates.next().is_some();
        let under_pressure = self.is_under_pressure(ctx);

        has_open_teammate && under_pressure
    }

    fn should_dribble(&self, ctx: &StateProcessingContext) -> bool {
        // Determine if the player should dribble based on the game state
        let has_space = self.has_space_to_dribble(ctx);
        let under_pressure = self.is_under_pressure(ctx);

        has_space && !under_pressure
    }

    // Additional helper functions

    fn get_opponent_goal_position(&self, ctx: &StateProcessingContext) -> Vector3<f32> {
        // Get the position of the opponent's goal based on the player's side
        let field_width = ctx.context.field_size.width as f32;
        let field_length = ctx.context.field_size.width as f32;

        if ctx.player.side == Some(PlayerSide::Left) {
            Vector3::new(field_width, field_length / 2.0, 0.0)
        } else {
            Vector3::new(0.0, field_length / 2.0, 0.0)
        }
    }

    fn has_clear_shot(&self, ctx: &StateProcessingContext) -> bool {
        // Check if the player has a clear shot to the goal without any obstructing opponents
        let player_position = ctx.player.position;
        let goal_position = self.get_opponent_goal_position(ctx);
        let shot_direction = (goal_position - player_position).normalize();

        let ray_cast_result = ctx.tick_context.space.cast_ray(
            player_position,
            shot_direction,
            ctx.player().goal_distance(),
            false,
        );

        ray_cast_result.is_none() // No collisions with opponents
    }

    fn is_teammate_open(&self, ctx: &StateProcessingContext, teammate: &MatchPlayerLite) -> bool {
        // Check if a teammate is open to receive a pass
        let is_in_passing_range = (teammate.position - ctx.player.position).magnitude() <= 30.0;
        let has_clear_passing_lane = self.has_clear_passing_lane(ctx, teammate);

        is_in_passing_range && has_clear_passing_lane
    }

    fn has_clear_passing_lane(
        &self,
        ctx: &StateProcessingContext,
        teammate: &MatchPlayerLite,
    ) -> bool {
        // Check if there is a clear passing lane to a teammate without any obstructing opponents
        let player_position = ctx.player.position;
        let teammate_position = teammate.position;
        let passing_direction = (teammate_position - player_position).normalize();

        let ray_cast_result = ctx.tick_context.space.cast_ray(
            player_position,
            passing_direction,
            (teammate_position - player_position).magnitude(),
            false,
        );

        ray_cast_result.is_none() // No collisions with opponents
    }

    fn is_under_pressure(&self, ctx: &StateProcessingContext) -> bool {
        ctx.player().pressure().is_under_immediate_pressure()
    }

    fn has_space_to_dribble(&self, ctx: &StateProcessingContext) -> bool {
        let dribble_distance = 10.0;
        !ctx.players().opponents().exists(dribble_distance)
    }
}
