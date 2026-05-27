use crate::PlayerFieldPositionGroup;
use crate::club::player::skills::GoalkeeperSpeedContext;
use crate::r#match::defenders::states::DefenderState;
use crate::r#match::events::EventCollection;
use crate::r#match::forwarders::states::ForwardState;
use crate::r#match::goalkeepers::states::state::GoalkeeperState;
use crate::r#match::midfielders::states::MidfielderState;
use crate::r#match::player::strategies::players::ops::skill_composites as sc;
use crate::r#match::{GameTickContext, MatchContext, MatchPlayer};

use nalgebra::Vector3;
use std::fmt::Display;
use std::fmt::Formatter;
use std::fmt::Result;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum PlayerState {
    Injured,
    Goalkeeper(GoalkeeperState),
    Defender(DefenderState),
    Midfielder(MidfielderState),
    Forward(ForwardState),
}

impl PlayerState {
    /// Cheap integer ID for fast dedup — avoids `to_string()` allocation.
    /// Each (outer variant, inner variant) pair maps to a unique u16.
    #[inline]
    pub fn compact_id(&self) -> u16 {
        match self {
            PlayerState::Injured => 0,
            PlayerState::Goalkeeper(s) => 100 + (*s as u16),
            PlayerState::Defender(s) => 200 + (*s as u16),
            PlayerState::Midfielder(s) => 300 + (*s as u16),
            PlayerState::Forward(s) => 400 + (*s as u16),
        }
    }
}

impl Display for PlayerState {
    fn fmt(&self, f: &mut Formatter) -> Result {
        match self {
            PlayerState::Injured => write!(f, "Injured"),
            PlayerState::Goalkeeper(state) => write!(f, "Goalkeeper: {}", state),
            PlayerState::Defender(state) => write!(f, "Defender: {}", state),
            PlayerState::Midfielder(state) => write!(f, "Midfielder: {}", state),
            PlayerState::Forward(state) => write!(f, "Forward: {}", state),
        }
    }
}

pub struct PlayerMatchState;

impl PlayerMatchState {
    pub fn process(
        player: &mut MatchPlayer,
        context: &MatchContext,
        tick_context: &GameTickContext,
    ) -> EventCollection {
        // Decay memory every 100 ticks
        let current_tick = context.current_tick();
        if current_tick > 0 && current_tick % 100 == 0 {
            player.memory.decay(current_tick);
        }

        let player_position_group = player.tactical_position.current_position.position_group();

        let state_change_result =
            player_position_group.process(player.in_state_time, player, context, tick_context);

        if state_change_result.start_tackle_cooldown {
            player.start_tackle_cooldown();
        }

        // Stash the shot reason on the player. The Shooting state will
        // consume and clear this when it composes the Shoot event.
        if let Some(reason) = state_change_result.shot_reason {
            player.pending_shot_reason = Some(reason);
        }

        if let Some(state) = state_change_result.state {
            Self::change_state(player, state);
        } else {
            player.in_state_time += 1;
        }

        if let Some(velocity) = state_change_result.velocity {
            let mut max_speed = if player_position_group == PlayerFieldPositionGroup::Goalkeeper {
                let speed_context = match player.state {
                    PlayerState::Goalkeeper(GoalkeeperState::Diving)
                    | PlayerState::Goalkeeper(GoalkeeperState::PreparingForSave)
                    | PlayerState::Goalkeeper(GoalkeeperState::Jumping) => {
                        GoalkeeperSpeedContext::Explosive
                    }
                    PlayerState::Goalkeeper(GoalkeeperState::Catching)
                    | PlayerState::Goalkeeper(GoalkeeperState::ComingOut) => {
                        GoalkeeperSpeedContext::Active
                    }
                    PlayerState::Goalkeeper(GoalkeeperState::Standing)
                    | PlayerState::Goalkeeper(GoalkeeperState::ReturningToGoal) => {
                        GoalkeeperSpeedContext::Positioning
                    }
                    _ => GoalkeeperSpeedContext::Casual,
                };
                player
                    .skills
                    .goalkeeper_max_speed(player.player_attributes.condition, speed_context)
            } else {
                player
                    .skills
                    .max_speed_with_condition(player.player_attributes.condition)
            };

            // Ball-carrier speed multiplier. Real football: carrying
            // the ball costs ~15-25% of top sprint for an average
            // player — they keep the ball in stride, look up, protect
            // it. Elite carriers (Mbappé/Messi) lose almost nothing.
            //
            // Routes through `movement_speed_with_ball` so dribbling +
            // technique + pace + acceleration + agility + balance all
            // contribute, and so fatigue/late-game effects propagate
            // through `effective_skill`. Mapping per spec:
            //
            //   carry_mult = 0.78 + composite * 0.42
            //
            // Composite floor 0.05 → 0.80 (worst carrier under fatigue);
            // composite 1.00 → 1.20 (elite carrier — no realistic
            // penalty). Capped to existing `[0.75, 1.00]` band so the
            // model stays a CARRY COST: an elite carrier matches their
            // off-ball speed but doesn't go faster than it.
            if tick_context.ball.current_owner == Some(player.id)
                && player_position_group != PlayerFieldPositionGroup::Goalkeeper
            {
                let minute = sc::minute_from_ms(context.total_match_time);
                let composite = sc::movement_speed_with_ball(player, minute);
                let raw = 0.78 + composite * 0.42;
                max_speed *= raw.clamp(0.75, 1.00);
            }

            // NaN/Inf guard: state velocity functions compose many
            // divisions and normalizations, and any zero-magnitude vector
            // put through `.normalize()` anywhere upstream produces a
            // NaN that propagates into player.velocity → player.position
            // → the recording → the viewer renders nothing. Catch it
            // here at the single integration point so no state has to
            // remember to self-sanitize. Non-finite → zero this tick.
            let finite = velocity.x.is_finite() && velocity.y.is_finite() && velocity.z.is_finite();
            let velocity = if finite { velocity } else { Vector3::zeros() };

            let velocity_sq = velocity.norm_squared();
            let max_speed_sq = max_speed * max_speed;

            if velocity_sq > max_speed_sq && velocity_sq > 0.0 {
                let velocity_magnitude = velocity_sq.sqrt();
                player.velocity = velocity * (max_speed / velocity_magnitude);
            } else {
                player.velocity = velocity;
            }
        }

        state_change_result.events
    }

    fn change_state(player: &mut MatchPlayer, state: PlayerState) {
        player.in_state_time = 0;
        player.state = state;
    }
}
