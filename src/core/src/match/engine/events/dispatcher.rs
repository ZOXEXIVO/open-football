use crate::r#match::ball::events::{BallEvent, BallEventDispatcher};
use crate::r#match::player::events::{PlayerEvent, PlayerEventDispatcher};
use crate::r#match::{MatchContext, MatchField, ResultMatchPositionData};

pub enum Event {
    BallEvent(BallEvent),
    PlayerEvent(PlayerEvent),
}

pub struct EventCollection {
    events: Vec<Event>,
}

impl EventCollection {
    pub fn new() -> Self {
        EventCollection {
            events: Vec::with_capacity(10),
        }
    }

    pub fn with_event(event: Event) -> Self {
        EventCollection {
            events: vec![event],
        }
    }

    pub fn add(&mut self, event: Event) {
        self.events.push(event)
    }

    pub fn add_ball_event(&mut self, event: BallEvent) {
        self.events.push(Event::BallEvent(event))
    }

    pub fn add_player_event(&mut self, event: PlayerEvent) {
        self.events.push(Event::PlayerEvent(event))
    }

    pub fn add_range(&mut self, events: Vec<Event>) {
        for event in events {
            self.events.push(event);
        }
    }

    pub fn add_from_collection(&mut self, events: EventCollection) {
        for event in events.events {
            self.events.push(event);
        }
    }

    pub fn to_vec(self) -> Vec<Event> {
        self.events
    }
}

pub struct EventDispatcher;

impl EventDispatcher {
    pub fn dispatch(
        events: Vec<Event>,
        field: &mut MatchField,
        context: &mut MatchContext,
        match_data: &mut ResultMatchPositionData,
        process_remaining_events: bool,
    ) {
        let mut remaining_events = Vec::with_capacity(10);

        for event in events {
            match event {
                Event::BallEvent(ball_event) => {
                    let mut ball_remaining_events =
                        BallEventDispatcher::dispatch(ball_event, field, context);

                    if process_remaining_events {
                        remaining_events.append(&mut ball_remaining_events);
                    }
                }
                Event::PlayerEvent(player_event) => {
                    let mut player_remaining_events =
                        PlayerEventDispatcher::dispatch(player_event, field, context, match_data);

                    if process_remaining_events {
                        remaining_events.append(&mut player_remaining_events);
                    }
                }
            }
        }

        if process_remaining_events {
            Self::dispatch(remaining_events, field, context, match_data, false)
        }
    }
}
