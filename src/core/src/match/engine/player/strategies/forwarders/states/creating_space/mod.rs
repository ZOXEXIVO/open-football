use crate::r#match::forwarders::states::ForwardState;
use crate::r#match::forwarders::states::common::{
    ActivityIntensity, ForwardCondition, ForwardRead,
};
use crate::r#match::player::strategies::common::players::ops::support::SupportOffer;
use crate::r#match::{
    ConditionContext, StateChangeResult, StateProcessingContext, StateProcessingHandler,
    SteeringBehavior,
};
use nalgebra::Vector3;

/// **The short outlet.** Showing for the man on the ball at an angle and a
/// distance (`SupportOffer`), for as long as the attack names him for it.
#[derive(Default, Clone)]
pub struct ForwardCreatingSpaceState {}

impl StateProcessingHandler for ForwardCreatingSpaceState {
    fn process(&self, ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        ForwardRead::hold(ctx, ForwardState::CreatingSpace)
    }

    fn velocity(&self, ctx: &StateProcessingContext) -> Option<Vector3<f32>> {
        Some(Self::outlet(ctx))
    }

    fn process_conditions(&self, ctx: ConditionContext) {
        ForwardCondition::new(ActivityIntensity::Moderate).process(ctx);
    }
}

impl ForwardCreatingSpaceState {
    /// At an angle and a distance off the man on the ball, on his own side
    /// of him; with nobody on it, his place.
    pub(crate) fn outlet(ctx: &StateProcessingContext) -> Vector3<f32> {
        let target = ctx
            .players()
            .teammates()
            .all()
            .find(|t| ctx.ball().owner_id() == Some(t.id))
            .and_then(|carrier| SupportOffer::target(ctx, carrier.position))
            .unwrap_or_else(|| ctx.team().my_anchor());
        SteeringBehavior::Arrive {
            target,
            slowing_distance: 24.0,
        }
        .calculate(ctx.player)
        .velocity
    }
}
