use crate::r#match::events::Event;
use crate::r#match::player::events::PlayerEvent;
use crate::r#match::{MatchContext, MatchField};
use log::debug;

#[derive(Copy, Clone, Debug)]
pub enum BallEvent {
    Goal(BallGoalEventMetadata),
    Claimed(u32),
    /// Pass reached its intended target: (receiver_id, passer_id).
    /// Emitted by `try_pass_target_claim` so pass-completion stats
    /// can be credited exactly once per successful pass.
    PassCompleted(u32, u32),
    /// Pass intercepted by opponent: (interceptor_id, passer_id)
    Intercepted(u32, Option<u32>),
    Gained(u32),
    TakeMe(u32),
}

#[derive(Copy, Clone, Debug, PartialOrd, PartialEq)]
pub enum GoalSide {
    Home,
    Away,
}

#[derive(Copy, Clone, Debug)]
pub struct BallGoalEventMetadata {
    pub side: GoalSide,
    pub goalscorer_player_id: u32,
    pub assist_player_id: Option<u32>,
    pub auto_goal: bool,
}

pub struct BallEventDispatcher;

impl BallEventDispatcher {
    pub fn dispatch(
        event: BallEvent,
        field: &mut MatchField,
        context: &MatchContext,
    ) -> Vec<Event> {
        let mut remaining_events = Vec::new();

        if context.logging_enabled {
            match event {
                BallEvent::TakeMe(_) | BallEvent::Claimed(_) => {}
                BallEvent::Intercepted(pid, _) => {
                    debug!("Ball event: Intercepted by player {}", pid);
                }
                _ => debug!("Ball event: {:?}", event),
            }
        }

        match event {
            BallEvent::Goal(metadata) => {
                // Determine which team scored based on the goalscorer's team, not goal position.
                // Goal position (GoalSide) is unreliable after halftime side swap.
                if let Some(scorer) = field
                    .players
                    .iter()
                    .find(|p| p.id == metadata.goalscorer_player_id)
                {
                    let is_home_scorer = scorer.team_id == context.score.home_team.team_id;

                    if metadata.auto_goal {
                        // Own goal — credit the opposing team
                        if is_home_scorer {
                            context.score.increment_away_goals();
                        } else {
                            context.score.increment_home_goals();
                        }
                    } else {
                        // Normal goal — credit the scorer's team
                        if is_home_scorer {
                            context.score.increment_home_goals();
                        } else {
                            context.score.increment_away_goals();
                        }
                    }
                }

                remaining_events.push(Event::PlayerEvent(PlayerEvent::Goal(
                    metadata.goalscorer_player_id,
                    metadata.auto_goal,
                )));

                if let Some(assist_id) = metadata.assist_player_id {
                    remaining_events.push(Event::PlayerEvent(PlayerEvent::Assist(assist_id)));
                }

                field.reset_players_positions();
            }
            BallEvent::Claimed(player_id) => {
                remaining_events.push(Event::PlayerEvent(PlayerEvent::ClaimBall(player_id)));
            }
            BallEvent::PassCompleted(receiver_id, passer_id) => {
                if let Some(passer) = field.get_player_mut(passer_id) {
                    passer.statistics.passes_completed += 1;
                }
                // Clear the pending-pass tag so the downstream ClaimBall
                // handler doesn't also credit this pass (double-count).
                field.ball.pending_pass_passer = None;
                remaining_events.push(Event::PlayerEvent(PlayerEvent::ClaimBall(receiver_id)));
            }
            BallEvent::Intercepted(interceptor_id, passer_id) => {
                // Credit the interceptor. Opponent touch ends the pass
                // window — accuracy was NOT earned.
                let _ = passer_id;
                field.ball.pending_pass_passer = None;
                if let Some(player) = field.get_player_mut(interceptor_id) {
                    player.statistics.interceptions += 1;
                }
                remaining_events.push(Event::PlayerEvent(PlayerEvent::ClaimBall(interceptor_id)));
            }
            BallEvent::Gained(player_id) => {
                remaining_events.push(Event::PlayerEvent(PlayerEvent::GainBall(player_id)));
            }
            BallEvent::TakeMe(player_id) => {
                remaining_events.push(Event::PlayerEvent(PlayerEvent::TakeBall(player_id)));
            }
        }

        remaining_events
    }
}
