use crate::r#match::events::Event;
use crate::r#match::forwarders::states::ForwardState;
use crate::r#match::player::events::{PassingEventContext, PlayerEvent};
use crate::r#match::{
    ConditionContext, MatchPlayerLite, PlayerSide, StateChangeResult, StateProcessingContext,
    StateProcessingHandler,
};
use nalgebra::Vector3;
use rand::prelude::IteratorRandom;

#[derive(Default)]
pub struct ForwardPassingState {}

impl StateProcessingHandler for ForwardPassingState {
    fn try_fast(&self, ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        // Check if the player has the ball
        if !ctx.player.has_ball(ctx) {
            // Transition to Running state if the player doesn't have the ball
            return Some(StateChangeResult::with_forward_state(ForwardState::Running));
        }

        if self.can_shoot(ctx) {
            // Transition to Shooting state if there's an opportunity to shoot
            return Some(StateChangeResult::with_forward_state(
                ForwardState::Shooting,
            ));
        }

        // Find the best passing option
        if let Some(teammate) = self.find_best_pass_option(ctx) {
            return Some(StateChangeResult::with_forward_state_and_event(
                ForwardState::Running,
                Event::PlayerEvent(PlayerEvent::PassTo(
                    PassingEventContext::build()
                        .with_from_player_id(ctx.player.id)
                        .with_to_player_id(teammate.id)
                        .with_target(ctx.tick_context.positions.players.position(teammate.id))
                        .with_force(ctx.player().pass_teammate_power(teammate.id))
                        .build(),
                )),
            ));
        }

        // Check if there's space to dribble forward
        if self.space_to_dribble(ctx) {
            // Transition to Dribbling state if there's space to dribble
            return Some(StateChangeResult::with_forward_state(
                ForwardState::Dribbling,
            ));
        }

        None
    }

    fn process_slow(&self, _ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        None
    }

    fn velocity(&self, _ctx: &StateProcessingContext) -> Option<Vector3<f32>> {
        None
    }

    fn process_conditions(&self, _ctx: ConditionContext) {}
}

impl ForwardPassingState {
    pub fn calculate_pass_power(&self, teammate_id: u32, ctx: &StateProcessingContext) -> f64 {
        let distance = ctx.tick_context.distances.get(ctx.player.id, teammate_id);

        let pass_skill = ctx.player.skills.technical.passing;

        (distance / pass_skill as f32 * 10.0) as f64
    }

    fn find_best_pass_option<'a>(
        &self,
        ctx: &StateProcessingContext<'a>,
    ) -> Option<MatchPlayerLite> {
        let player_position = ctx.player.position;
        let field_width = ctx.context.field_size.width as f32;

        let attacking_third_start = if ctx.player.side == Some(PlayerSide::Left) {
            field_width * (2.0 / 3.0)
        } else {
            field_width / 3.0
        };

        if player_position.x >= attacking_third_start {
            // Player is in the attacking third, prioritize teammates near the opponent's goal
            self.find_best_pass_option_attacking_third(ctx)
        } else if player_position.x >= field_width / 3.0
            && player_position.x <= field_width * (2.0 / 3.0)
        {
            // Player is in the middle third, prioritize teammates in advanced positions
            self.find_best_pass_option_middle_third(ctx)
        } else {
            // Player is in the defensive third, prioritize safe passes to nearby teammates
            self.find_best_pass_option_defensive_third(ctx)
        }
    }

    fn find_best_pass_option_attacking_third(
        &self,
        ctx: &StateProcessingContext<'_>,
    ) -> Option<MatchPlayerLite> {
        let players = ctx.players();
        let teammates = players.teammates();

        let nearest_to_goal = teammates
            .all()
            .filter(|p| ctx.player().has_clear_pass(p.id))
            .filter(|teammate| {
                // Check if the teammate is in a dangerous position near the opponent's goal
                let goal_distance_threshold = ctx.context.field_size.width as f32 * 0.2;
                (teammate.position - ctx.player().opponent_goal_position()).magnitude()
                    < goal_distance_threshold
            })
            .min_by(|a, b| {
                let dist_a = (a.position - ctx.player().opponent_goal_position()).magnitude();
                let dist_b = (b.position - ctx.player().opponent_goal_position()).magnitude();
                dist_a.partial_cmp(&dist_b).unwrap()
            });

        nearest_to_goal
    }

    fn find_best_pass_option_middle_third(
        &self,
        ctx: &StateProcessingContext<'_>,
    ) -> Option<MatchPlayerLite> {
        let players = ctx.players();
        let teammates = players.teammates();

        teammates
            .nearby(300.0)
            .filter(|p| !p.tactical_positions.is_forward() && !p.tactical_positions.is_goalkeeper())
            .choose(&mut rand::thread_rng())
    }

    fn find_best_pass_option_defensive_third(
        &self,
        ctx: &StateProcessingContext<'_>,
    ) -> Option<MatchPlayerLite> {
        let players = ctx.players();
        let teammates = players.teammates();

        let nearest_teammate = teammates.nearby(300.0).filter(|p| ctx.player().has_clear_pass(p.id))
            .max_by(|a, b| {
            let dist_a = (a.position - ctx.player.position).magnitude();
            let dist_b = (b.position - ctx.player.position).magnitude();
            dist_a.partial_cmp(&dist_b).unwrap()
        });

        nearest_teammate
    }

    fn space_to_dribble(&self, ctx: &StateProcessingContext) -> bool {
        if !ctx.player.has_ball(ctx) {
            return false;
        }

        let dribble_distance = 10.0; // Adjust based on your game's scale
        let players = ctx.players();
        let opponents = players.opponents();

        !opponents.exists(dribble_distance)
    }

    fn can_shoot(&self, ctx: &StateProcessingContext) -> bool {
        ctx.ball().distance_to_opponent_goal() < 250.0 && ctx.player().has_clear_shot()
    }
}
