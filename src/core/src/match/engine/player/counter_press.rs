//! The second after the ball is taken off a player: he goes straight back
//! at whoever has it, or he lets the team's shape deal with it.
//!
//! The pressing state does the work once he is in it: it closes the
//! carrier, takes a loose ball he is first to, and tackles, which is where
//! the tactical foul is decided. This only decides whether he goes.

use crate::PlayerFieldPositionGroup;
use crate::r#match::defenders::states::DefenderState;
use crate::r#match::forwarders::states::ForwardState;
use crate::r#match::midfielders::states::MidfielderState;
use crate::r#match::player::state::PlayerState;
use crate::r#match::{GameTickContext, MatchContext, MatchPlayer};

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum CounterPressCall {
    /// Nothing to decide yet: the ball is loose or he is mid-action.
    Wait,
    /// He lets it go, or there is nothing left to go after.
    LetGo,
    /// After the carrier, in this state.
    Press(PlayerState),
}

pub struct CounterPress;

impl CounterPress {
    /// A second. A man who has not gone after it by then is not going to.
    pub const WINDOW_TICKS: u64 = 100;
    /// 12 m: beyond it the carrier is somebody else's to press.
    pub const RANGE: f32 = 96.0;
    /// The urge it takes to go after a carrier standing next to him.
    const MIN_URGE: f32 = 0.25;

    /// 0..1, how badly he wants it back: his own work rate, aggression and
    /// determination, the press the team is playing, and the legs left.
    pub fn urge(player: &MatchPlayer, press_intensity: f32) -> f32 {
        let mental = &player.skills.mental;
        let attitude =
            (mental.work_rate * 0.45 + mental.aggression * 0.25 + mental.determination * 0.30)
                / 20.0;
        let legs = player.player_attributes.condition_percentage() as f32 / 100.0;
        (attitude * (0.5 + press_intensity) * legs * player.injury_handicap()).clamp(0.0, 1.0)
    }

    /// What a player who lost the ball at `lost_at` does now. The further
    /// away the carrier, the more urge it takes to go.
    pub fn call(
        player: &MatchPlayer,
        lost_at: u64,
        context: &MatchContext,
        tick_context: &GameTickContext,
    ) -> CounterPressCall {
        if context.current_tick().saturating_sub(lost_at) > Self::WINDOW_TICKS
            || player.state == PlayerState::Injured
            || player.off_pitch
            || tick_context.ball.restart_taker.is_some()
            || tick_context.ball.held_in_hands
        {
            return CounterPressCall::LetGo;
        }
        let Some(carrier) = tick_context.ball.current_owner else {
            return CounterPressCall::Wait;
        };
        if tick_context.positions.players.side(carrier) == player.side {
            return CounterPressCall::LetGo;
        }
        if player.state.is_committed_action() {
            return CounterPressCall::Wait;
        }
        let Some(pressing) =
            Self::pressing_state(player.tactical_position.current_position.position_group())
        else {
            return CounterPressCall::LetGo;
        };
        let gap = (tick_context.positions.players.position(carrier) - player.position).norm();
        let press = context.tactical_for_team(player.team_id).press_intensity;
        let need = Self::MIN_URGE + (1.0 - Self::MIN_URGE) * gap / Self::RANGE;
        if Self::urge(player, press) >= need {
            CounterPressCall::Press(pressing)
        } else {
            CounterPressCall::LetGo
        }
    }

    fn pressing_state(group: PlayerFieldPositionGroup) -> Option<PlayerState> {
        match group {
            PlayerFieldPositionGroup::Goalkeeper => None,
            PlayerFieldPositionGroup::Defender => {
                Some(PlayerState::Defender(DefenderState::Pressing))
            }
            PlayerFieldPositionGroup::Midfielder => {
                Some(PlayerState::Midfielder(MidfielderState::Pressing))
            }
            PlayerFieldPositionGroup::Forward => Some(PlayerState::Forward(ForwardState::Pressing)),
        }
    }
}
