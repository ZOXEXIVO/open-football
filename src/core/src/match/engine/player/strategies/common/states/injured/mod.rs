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

/// Ticks a player stays down before hobbling back into the game.
///
/// The engine's medical pass runs every 6-14 sim-minutes and substitutes
/// anyone in a critical condition, so this only has to cover the window
/// between going down and the physio's verdict. A side with no
/// substitutions left keeps a passenger on the pitch — which is exactly
/// what happens in a real match.
const TREATMENT_TICKS: u64 = 400;

/// A player who has just gone down injured.
///
/// This state was implemented but had no inbound transition anywhere in
/// the engine: an in-match injury was modelled purely by crushing the
/// player's `condition` to a critical value and waiting for the next
/// substitution pass to notice. The player kept sprinting, pressing and
/// tackling at full tilt in the meantime, and a side with no subs left
/// simply played on with eleven fit players.
///
/// Now `roll_in_match_injuries` transitions the victim here. They stop,
/// they recover nothing (a hurt player is not resting), and they are
/// excluded from the loose-ball redirects and the chase table via
/// [`PlayerState::is_committed_action`], so play carries on around them.
#[derive(Default, Clone)]
pub struct CommonInjuredState {}

impl StateProcessingHandler for CommonInjuredState {
    fn process(&self, ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        if ctx.in_state_time < TREATMENT_TICKS {
            return None;
        }

        // Treated and waved back on — or never came off, because the
        // bench was empty. Either way they rejoin in their role's default
        // state, carrying whatever condition the injury left them with.
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
