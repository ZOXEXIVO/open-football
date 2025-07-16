use crate::r#match::events::Event;
use crate::r#match::player::events::PlayerEvent;
use crate::r#match::{MatchContext, MatchField};
use log::debug;

#[derive(Copy, Clone, Debug)]
pub enum BallEvent {
    Goal(BallGoalEventMetadata),
    Claimed(u32),
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

        debug!("Ball event: {:?}", event);

        match event {
            BallEvent::Goal(metadata) => {
                match metadata.side {
                    GoalSide::Home => context.score.increment_away_goals(),
                    GoalSide::Away => context.score.increment_home_goals(),
                }

                remaining_events.push(Event::PlayerEvent(PlayerEvent::Goal(
                    metadata.goalscorer_player_id,
                    metadata.auto_goal,
                )));

                field.reset_players_positions();
            }
            BallEvent::Claimed(player_id) => {
                remaining_events.push(Event::PlayerEvent(PlayerEvent::ClaimBall(player_id)));
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
