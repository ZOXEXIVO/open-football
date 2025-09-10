use crate::r#match::events::Event;
use crate::r#match::goalkeepers::states::state::GoalkeeperState;
use crate::r#match::player::events::{PassingEventContext, PlayerEvent};
use crate::r#match::{ConditionContext, MatchPlayerLite, StateChangeResult, StateProcessingContext, StateProcessingHandler};
use nalgebra::Vector3;
use rand::prelude::IteratorRandom;

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
        let players = ctx.players();
        let teammates = players.teammates();
        let vision_range = ctx.player.skills.mental.vision * 10.0;

        let open_teammates: Vec<MatchPlayerLite> = teammates
            .nearby(vision_range)
            .filter(|t| self.is_teammate_open(ctx, t) && ctx.player().has_clear_pass(t.id))
            .collect();

        if !open_teammates.is_empty() {
            open_teammates
                .iter()
                .max_by(|a, b| {
                    let space_a = self.calculate_space_around_player(ctx, a);
                    let space_b = self.calculate_space_around_player(ctx, b);
                    space_a.partial_cmp(&space_b).unwrap_or(std::cmp::Ordering::Equal)
                })
                .cloned()
        } else {
            teammates.nearby(300.0).choose(&mut rand::rng())
        }
    }

    fn is_teammate_open(&self, ctx: &StateProcessingContext, teammate: &MatchPlayerLite) -> bool {
        let opponent_distance_threshold = 5.0;

        ctx.players().opponents().all()
            .filter(|opponent| (opponent.position - teammate.position).magnitude() <= opponent_distance_threshold)
            .count() == 0
    }

    fn calculate_space_around_player(&self, ctx: &StateProcessingContext, player: &MatchPlayerLite) -> f32 {
        let space_radius = 10.0;

        space_radius - ctx.players().opponents().all()
            .filter(|opponent| (opponent.position - player.position).magnitude() <= space_radius)
            .count() as f32
    }
}
