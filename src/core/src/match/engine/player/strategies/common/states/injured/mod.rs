use crate::PlayerFieldPositionGroup;
use crate::r#match::StateChangeResult;
use crate::r#match::defenders::states::DefenderState;
use crate::r#match::defenders::states::common::DefenderCondition;
use crate::r#match::forwarders::states::ForwardState;
use crate::r#match::forwarders::states::common::ForwardCondition;
use crate::r#match::goalkeepers::states::common::GoalkeeperCondition;
use crate::r#match::goalkeepers::states::state::GoalkeeperState;
use crate::r#match::midfielders::states::MidfielderState;
use crate::r#match::midfielders::states::common::MidfielderCondition;
use crate::r#match::player::state::PlayerState;
use crate::r#match::player::strategies::common::ActivityIntensity;
use crate::r#match::player::strategies::processor::{
    ConditionContext, StateProcessingContext, StateProcessingHandler,
};
use nalgebra::Vector3;

/// A player who has gone down injured.
///
/// He stops, recovers nothing (a hurt player is not resting), and is out of
/// the loose-ball redirects and the chase table via
/// [`PlayerState::is_committed_action`], so play carries on around him.
/// He gets up when his treatment is over — a knock quickly, a hurt after
/// the physio has seen him — and a serious injury does not get up at all:
/// the medical pass replaces him or he is carried off.
#[derive(Default, Clone)]
pub struct CommonInjuredState {}

impl StateProcessingHandler for CommonInjuredState {
    fn process(&self, ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        if !ctx.player.is_treated(ctx.context.total_match_time) {
            return None;
        }
        Some(StateChangeResult::with(Self::default_state_for(
            ctx.player
                .tactical_position
                .current_position
                .position_group(),
        )))
    }

    fn velocity(&self, _ctx: &StateProcessingContext) -> Option<Vector3<f32>> {
        Some(Vector3::zeros())
    }

    fn process_conditions(&self, ctx: ConditionContext) {
        // Down injured is NOT a rest. Routing through the role's condition
        // processor keeps the fatigue model's single entry point (so
        // `last_activity_intensity` stays truthful and the movement
        // integrator never reads a stale sprint), while `Low` denies the
        // deep recovery a fully stationary player would otherwise bank —
        // the whole point is that the player comes back diminished.
        let group = ctx
            .player
            .tactical_position
            .current_position
            .position_group();
        match group {
            PlayerFieldPositionGroup::Goalkeeper => {
                GoalkeeperCondition::new(ActivityIntensity::Low).process(ctx)
            }
            PlayerFieldPositionGroup::Defender => {
                DefenderCondition::new(ActivityIntensity::Low).process(ctx)
            }
            PlayerFieldPositionGroup::Midfielder => {
                MidfielderCondition::new(ActivityIntensity::Low).process(ctx)
            }
            PlayerFieldPositionGroup::Forward => {
                ForwardCondition::new(ActivityIntensity::Low).process(ctx)
            }
        }
    }
}

impl CommonInjuredState {
    /// Role default to rejoin in — mirrors `MatchPlayer::default_state`.
    fn default_state_for(group: PlayerFieldPositionGroup) -> PlayerState {
        match group {
            PlayerFieldPositionGroup::Goalkeeper => {
                PlayerState::Goalkeeper(GoalkeeperState::Standing)
            }
            PlayerFieldPositionGroup::Defender => PlayerState::Defender(DefenderState::Standing),
            PlayerFieldPositionGroup::Midfielder => {
                PlayerState::Midfielder(MidfielderState::Standing)
            }
            PlayerFieldPositionGroup::Forward => PlayerState::Forward(ForwardState::Standing),
        }
    }
}
