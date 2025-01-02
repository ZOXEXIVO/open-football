use crate::common::loader::DefaultNeuralNetworkLoader;
use crate::common::NeuralNetwork;
use crate::r#match::defenders::states::DefenderState;
use crate::r#match::events::Event;
use crate::r#match::player::events::{PassingEventContext, PlayerEvent};
use crate::r#match::{ConditionContext, MatchPlayerLite, StateChangeResult, StateProcessingContext, StateProcessingHandler};
use nalgebra::Vector3;
use std::sync::LazyLock;

static _DEFENDER_PASSING_STATE_NETWORK: LazyLock<NeuralNetwork> =
    LazyLock::new(|| DefaultNeuralNetworkLoader::load(include_str!("nn_passing_data.json")));

#[derive(Default)]
pub struct DefenderPassingState {}

impl StateProcessingHandler for DefenderPassingState {
    fn try_fast(&self, ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        if !ctx.player.has_ball(ctx) {
            return Some(StateChangeResult::with_defender_state(
                DefenderState::Standing,
            ));
        }

        if let Some(teammate) = self.find_best_pass_option(ctx) {
            return Some(StateChangeResult::with_defender_state_and_event(
                DefenderState::Standing,
                Event::PlayerEvent(PlayerEvent::PassTo(
                    PassingEventContext::build()
                        .with_from_player_id(ctx.player.id)
                        .with_to_player_id(teammate.id)
                        .with_target(teammate.position)
                        .with_force(ctx.player().pass_teammate_power(teammate.id))
                        .build()
                )),
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

impl DefenderPassingState {
    fn find_best_pass_option<'a>(
        &self,
        ctx: &StateProcessingContext<'a>,
    ) -> Option<MatchPlayerLite> {
        let vision_range = ctx.player.skills.mental.vision * 10.0;
        let open_teammates: Vec<MatchPlayerLite> = ctx.players().teammates()
            .nearby(vision_range)
            .filter(|t| !t.tactical_positions.is_goalkeeper())
            .filter(|t| self.is_teammate_open(ctx, t))
            .collect();

        if !open_teammates.is_empty() {
            open_teammates.iter()
                .min_by(|a, b| {
                    let risk_a = self.estimate_interception_risk(ctx, a);
                    let risk_b = self.estimate_interception_risk(ctx, b);
                    risk_a.partial_cmp(&risk_b).unwrap_or(std::cmp::Ordering::Equal)
                })
                .cloned()
        } else {
            None
        }
    }

    fn is_teammate_open(&self, ctx: &StateProcessingContext, teammate: &MatchPlayerLite) -> bool {
        let opponent_distance_threshold = 5.0;
        ctx.players().opponents().all()
            .filter(|o| (o.position - teammate.position).magnitude() <= opponent_distance_threshold)
            .count() == 0
    }

    fn estimate_interception_risk(&self, ctx: &StateProcessingContext, teammate: &MatchPlayerLite) -> f32 {
        let max_interception_distance = 10.0;
        let player_position = ctx.player.position;
        let pass_direction = (teammate.position - player_position).normalize();

        ctx.players().opponents().all()
            .filter(|o| (o.position - player_position).dot(&pass_direction) > 0.0)
            .map(|o| (o.position - player_position).magnitude())
            .filter(|d| *d <= max_interception_distance)
            .min_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal))
            .unwrap_or(max_interception_distance)
    }

    pub fn calculate_pass_power(&self, teammate_id: u32, ctx: &StateProcessingContext) -> f32 {
        let distance = ctx.tick_context.distances.get(ctx.player.id, teammate_id);
        let pass_skill = ctx.player.skills.technical.passing as f32 / 20.0;
        (distance / pass_skill).clamp(0.1, 1.0)
    }
}
