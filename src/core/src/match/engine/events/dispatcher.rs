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
    /// Create empty collection without heap allocation.
    /// Use `with_capacity` only for the reusable per-tick collection.
    pub fn new() -> Self {
        EventCollection { events: Vec::new() }
    }

    pub fn with_capacity(cap: usize) -> Self {
        EventCollection {
            events: Vec::with_capacity(cap),
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

    #[inline]
    pub fn has_events(&self) -> bool {
        !self.events.is_empty()
    }

    pub fn clear(&mut self) {
        self.events.clear();
    }

    pub fn drain(&mut self) -> std::vec::Drain<'_, Event> {
        self.events.drain(..)
    }

    pub fn to_vec(self) -> Vec<Event> {
        self.events
    }
}

pub struct EventDispatcher;

impl EventDispatcher {
    pub fn dispatch(
        events: &mut EventCollection,
        field: &mut MatchField,
        context: &mut MatchContext,
        match_data: &mut ResultMatchPositionData,
        process_remaining_events: bool,
    ) {
        Self::dispatch_iter(
            events.drain(),
            field,
            context,
            match_data,
            process_remaining_events,
        );
    }

    fn dispatch_iter(
        events: impl Iterator<Item = Event>,
        field: &mut MatchField,
        context: &mut MatchContext,
        match_data: &mut ResultMatchPositionData,
        process_remaining_events: bool,
    ) {
        let mut remaining_events: Vec<Event> = Vec::new();

        for event in events {
            match event {
                Event::BallEvent(ball_event) => {
                    if context.logging_enabled {
                        match ball_event {
                            BallEvent::TakeMe(_) => {}
                            _ => match_data.add_match_event(
                                context.total_match_time,
                                "ball",
                                format!("{:?}", ball_event),
                            ),
                        }
                    }

                    let mut ball_remaining_events =
                        BallEventDispatcher::dispatch(ball_event, field, context);

                    if process_remaining_events && !ball_remaining_events.is_empty() {
                        remaining_events.append(&mut ball_remaining_events);
                    }
                }
                Event::PlayerEvent(player_event) => {
                    if context.logging_enabled {
                        match &player_event {
                            PlayerEvent::TakeBall(_) => {}
                            _ => match_data.add_match_event(
                                context.total_match_time,
                                "player",
                                format!("{:?}", player_event),
                            ),
                        }
                    }

                    let mut player_remaining_events =
                        PlayerEventDispatcher::dispatch(player_event, field, context, match_data);

                    if process_remaining_events && !player_remaining_events.is_empty() {
                        remaining_events.append(&mut player_remaining_events);
                    }
                }
            }
        }

        if process_remaining_events && !remaining_events.is_empty() {
            Self::dispatch_iter(
                remaining_events.into_iter(),
                field,
                context,
                match_data,
                false,
            )
        }
    }
}
