use crate::r#match::events::Event;
use crate::r#match::midfielders::states::MidfielderState;
use crate::r#match::midfielders::states::common::{
    ActivityIntensity, MidfielderCondition, U_PER_M,
};
use crate::r#match::player::events::{PassingEventContext, PlayerEvent};
use crate::r#match::{
    ConditionContext, MatchPlayerLite, StateChangeResult, StateProcessingContext,
    StateProcessingHandler,
};
use nalgebra::Vector3;
use std::cmp::Ordering;

#[derive(Default, Clone)]
pub struct MidfielderDistributingState {}

impl StateProcessingHandler for MidfielderDistributingState {
    fn process(&self, ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        // Lost possession — transition out
        if !ctx.player.has_ball(ctx) {
            return Some(StateChangeResult::with_midfielder_state(
                MidfielderState::Running,
            ));
        }

        // Find the best passing option
        if let Some(teammate) = self.find_best_pass_option(ctx) {
            return Some(StateChangeResult::with_midfielder_state_and_event(
                MidfielderState::Running,
                Event::PlayerEvent(PlayerEvent::PassTo(
                    PassingEventContext::new()
                        .with_from_player_id(ctx.player.id)
                        .with_to_player_id(teammate.id)
                        .with_reason("MID_DISTRIBUTING")
                        .build(ctx),
                )),
            ));
        }

        // Timeout: if no pass option found, transition to running with ball
        if ctx.in_state_time > 30 {
            return Some(StateChangeResult::with_midfielder_state(
                MidfielderState::Running,
            ));
        }

        None
    }

    fn velocity(&self, _ctx: &StateProcessingContext) -> Option<Vector3<f32>> {
        Some(Vector3::new(0.0, 0.0, 0.0))
    }

    fn process_conditions(&self, ctx: ConditionContext) {
        // Distributing is moderate intensity
        MidfielderCondition::new(ActivityIntensity::Moderate).process(ctx);
    }
}

impl MidfielderDistributingState {
    fn find_best_pass_option<'a>(
        &self,
        ctx: &StateProcessingContext<'a>,
    ) -> Option<MatchPlayerLite> {
        // How far he can pick a man out. `vision * 10` units is 1.25 m
        // per point of vision — a 15/20 playmaker could see 19 m, which
        // is a five-yard ball in real money. Written in metres: from
        // ~14 m for a player who sees nothing to ~40 m for one who sees
        // everything.
        let vision01 = (ctx.player.skills.mental.vision / 20.0).clamp(0.0, 1.0);
        let vision_range = (14.0 + vision01 * 26.0) * U_PER_M;

        ctx.players()
            .teammates()
            .nearby(vision_range)
            .filter(|t| !t.tactical_positions.is_goalkeeper())
            .filter(|t| self.is_teammate_open(ctx, t) && ctx.player().has_clear_pass(t.id))
            .max_by(|a, b| {
                let recency_a = ctx.ball().passer_recency_penalty(a.id);
                let recency_b = ctx.ball().passer_recency_penalty(b.id);
                let space_a = self.calculate_space_around_player(ctx, a) * recency_a;
                let space_b = self.calculate_space_around_player(ctx, b) * recency_b;
                space_a.partial_cmp(&space_b).unwrap_or(Ordering::Equal)
            })
    }

    fn is_teammate_open(&self, ctx: &StateProcessingContext, teammate: &MatchPlayerLite) -> bool {
        // "Open" was no opponent within 5u — 62 cm. Every team-mate on
        // the pitch qualified, so this state passed to whoever scored
        // best on a space metric that was equally blind. 2.5 m is a
        // man you can play into.
        let opponent_distance_threshold = 2.5 * U_PER_M;

        // Use distance closure: from teammate's perspective, opponents are nearby
        ctx.tick_context
            .grid
            .opponents(teammate.id, opponent_distance_threshold)
            .next()
            .is_none()
    }

    /// Room around the receiver, 0..1. The old form subtracted a head
    /// count from a radius — a length minus a cardinal — and scanned
    /// 10u (1.25 m), so it read 10.0 for practically everybody and the
    /// `max_by` over it was picking arbitrarily.
    fn calculate_space_around_player(
        &self,
        ctx: &StateProcessingContext,
        player: &MatchPlayerLite,
    ) -> f32 {
        const SPACE_RADIUS: f32 = 6.0 * U_PER_M;
        let mut crowding = 0.0f32;
        for (_id, dist) in ctx.tick_context.grid.opponents(player.id, SPACE_RADIUS) {
            let proximity = 1.0 - (dist / SPACE_RADIUS).clamp(0.0, 1.0);
            crowding += proximity * proximity;
        }
        1.0 / (1.0 + crowding)
    }
}
