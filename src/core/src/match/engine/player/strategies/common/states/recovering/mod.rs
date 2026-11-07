use crate::PlayerFieldPositionGroup;
use crate::r#match::defenders::states::common::DefenderCondition;
use crate::r#match::forwarders::states::common::ForwardCondition;
use crate::r#match::goalkeepers::states::common::GoalkeeperCondition;
use crate::r#match::midfielders::states::common::MidfielderCondition;
use crate::r#match::player::strategies::common::ActivityIntensity;
use crate::r#match::player::strategies::common::states::CommonInjuredState;
use crate::r#match::player::strategies::processor::{
    ConditionContext, StateChangeResult, StateProcessingContext, StateProcessingHandler,
};
use nalgebra::Vector3;

/// What kind of challenge a player has just gone in with.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChallengeKind {
    /// On his feet, in front of the man.
    Block,
    /// A stretch or a slide, with his weight gone.
    Slide,
}

impl ChallengeKind {
    /// AI ticks to get back up after missing with this challenge, for a man
    /// of no agility or balance, and how many of them the best in the game
    /// save. A block costs 0.3-0.5 s and a slide 0.8-1.2 s.
    fn recovery_range(self) -> (f32, f32) {
        match self {
            ChallengeKind::Block => (25.0, 10.0),
            ChallengeKind::Slide => (60.0, 20.0),
        }
    }

    /// How long a man with these attributes takes to get back up.
    pub fn recovery_ticks(self, agility: f32, balance: f32) -> u64 {
        let (longest, saved) = self.recovery_range();
        let nimble = (((agility + balance) / 2.0 - 1.0) / 19.0).clamp(0.0, 1.0);
        (longest - saved * nimble).round() as u64
    }
}

/// A player recovering from a challenge he missed.
///
/// One state for every role, like [`CommonInjuredState`]: what his body does
/// here has nothing to do with his position. Before it existed `Missed` went
/// straight to `Pressing` and a man ran faster in the half second after a
/// failed challenge (4.63 m/s) than the outfield did on average (3.73 m/s).
#[derive(Default, Clone)]
pub struct CommonRecoveringState {}

impl CommonRecoveringState {
    /// Share of his speed kept each AI tick: he stops rather than runs on, and
    /// does not stop dead.
    const SPEED_KEPT_PER_TICK: f32 = 0.85;
}

impl StateProcessingHandler for CommonRecoveringState {
    fn process(&self, ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        if ctx.in_state_time < ctx.player.recovery_ticks {
            return None;
        }
        Some(StateChangeResult::with(
            CommonInjuredState::default_state_for(
                ctx.player
                    .tactical_position
                    .current_position
                    .position_group(),
            ),
        ))
    }

    fn velocity(&self, ctx: &StateProcessingContext) -> Option<Vector3<f32>> {
        Some(ctx.player.velocity * Self::SPEED_KEPT_PER_TICK)
    }

    fn process_conditions(&self, ctx: ConditionContext) {
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
