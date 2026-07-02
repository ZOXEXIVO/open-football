use crate::r#match::ball::events::{BallEvent, BallEventDispatcher};
use crate::r#match::player::events::{PlayerEvent, PlayerEventDispatcher};
use crate::r#match::{MatchContext, MatchField, ResultMatchPositionData};
use std::vec::IntoIter;

pub enum Event {
    BallEvent(BallEvent),
    PlayerEvent(PlayerEvent),
}

/// Inline capacity for the per-state EventCollection. One slot: a
/// `StateProcessingResult` + `StateChangeResult` (each embedding one of
/// these) is constructed and moved by value on the ~6M-updates/match hot
/// path, and 0-1 events is the overwhelmingly dominant case — at cap 4
/// the inline array alone was ~224 bytes of per-update construction and
/// move traffic. Multi-event bursts (tackle + foul, claim + clear) spill
/// to the overflow Vec — a handful of heap allocations per match, with
/// iteration order (inline first, then overflow) unchanged.
pub const INLINE_EVENT_CAP: usize = 1;
/// Inline capacity for the dispatcher's `remaining_events` collection
/// — slightly larger since downstream handlers can fan out before any
/// spill occurs.
pub const DISPATCH_REMAINING_INLINE_CAP: usize = 8;

pub struct EventCollection {
    inline: [Option<Event>; INLINE_EVENT_CAP],
    inline_len: u8,
    overflow: Vec<Event>,
}

impl Default for EventCollection {
    fn default() -> Self {
        Self::new()
    }
}

impl EventCollection {
    /// Allocation-free constructor: the inline buffer covers 0..=4
    /// events, the overflow Vec stays empty until it's actually needed.
    #[inline]
    pub fn new() -> Self {
        EventCollection {
            inline: std::array::from_fn(|_| None),
            inline_len: 0,
            overflow: Vec::new(),
        }
    }

    /// Pre-size the overflow Vec for callers that know they'll spill
    /// past INLINE_EVENT_CAP. Smaller capacities stay allocation-free.
    #[inline]
    pub fn with_capacity(cap: usize) -> Self {
        let extra = cap.saturating_sub(INLINE_EVENT_CAP);
        EventCollection {
            inline: std::array::from_fn(|_| None),
            inline_len: 0,
            overflow: if extra > 0 {
                Vec::with_capacity(extra)
            } else {
                Vec::new()
            },
        }
    }

    /// Single-event constructor — allocation-free.
    #[inline]
    pub fn with_event(event: Event) -> Self {
        let mut c = Self::new();
        c.add(event);
        c
    }

    #[inline]
    pub fn add(&mut self, event: Event) {
        let n = self.inline_len as usize;
        if n < INLINE_EVENT_CAP {
            self.inline[n] = Some(event);
            self.inline_len = (n + 1) as u8;
        } else {
            self.overflow.push(event);
        }
    }

    #[inline]
    pub fn add_ball_event(&mut self, event: BallEvent) {
        self.add(Event::BallEvent(event))
    }

    #[inline]
    pub fn add_player_event(&mut self, event: PlayerEvent) {
        self.add(Event::PlayerEvent(event))
    }

    /// Generic over any iterator of `Event` — accepts `Vec<Event>`,
    /// `vec::Drain`, or any other producer without forcing the caller
    /// to materialise a Vec first.
    pub fn add_range<I: IntoIterator<Item = Event>>(&mut self, events: I) {
        for e in events {
            self.add(e);
        }
    }

    pub fn add_from_collection(&mut self, mut events: EventCollection) {
        let total = events.inline_len as usize;
        for slot in events.inline.iter_mut().take(total) {
            if let Some(ev) = slot.take() {
                self.add(ev);
            }
        }
        events.inline_len = 0;
        if !events.overflow.is_empty() {
            for e in events.overflow.drain(..) {
                self.add(e);
            }
        }
    }

    #[inline]
    pub fn has_events(&self) -> bool {
        self.inline_len > 0 || !self.overflow.is_empty()
    }

    #[inline]
    pub fn clear(&mut self) {
        let n = self.inline_len as usize;
        for slot in self.inline.iter_mut().take(n) {
            *slot = None;
        }
        self.inline_len = 0;
        self.overflow.clear();
    }

    /// Move every event out of the collection, leaving it empty once
    /// the returned iterator is fully consumed (or dropped).
    pub fn drain(&mut self) -> EventDrain<'_> {
        let inline_total = self.inline_len as usize;
        // Reset length up front: drain semantically empties the
        // collection, and the iterator's own Drop guarantees any
        // un-consumed inline slots get cleared.
        self.inline_len = 0;
        let overflow = std::mem::take(&mut self.overflow);
        EventDrain {
            coll: self,
            inline_idx: 0,
            inline_total,
            overflow_iter: overflow.into_iter(),
        }
    }
}

pub struct EventDrain<'a> {
    coll: &'a mut EventCollection,
    inline_idx: usize,
    inline_total: usize,
    overflow_iter: IntoIter<Event>,
}

impl<'a> Iterator for EventDrain<'a> {
    type Item = Event;

    fn next(&mut self) -> Option<Event> {
        while self.inline_idx < self.inline_total {
            let i = self.inline_idx;
            self.inline_idx += 1;
            if let Some(e) = self.coll.inline[i].take() {
                return Some(e);
            }
        }
        self.overflow_iter.next()
    }
}

impl<'a> Drop for EventDrain<'a> {
    fn drop(&mut self) {
        while self.inline_idx < self.inline_total {
            let i = self.inline_idx;
            self.inline_idx += 1;
            self.coll.inline[i] = None;
        }
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
        // Inline storage for the recursion buffer — most chained
        // events fan out to 0-2 follow-ups; only a goal-reset cascade
        // ever fills more than the inline cap. `new()` keeps the same
        // 4-slot inline buffer but leaves the overflow Vec unallocated
        // until something actually spills (the dominant path never does),
        // avoiding a heap alloc+free on every dispatch.
        let mut remaining_events: EventCollection = EventCollection::new();

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

                    let ball_remaining_events =
                        BallEventDispatcher::dispatch(ball_event, field, context);

                    if process_remaining_events && !ball_remaining_events.is_empty() {
                        remaining_events.add_range(ball_remaining_events);
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

                    let player_remaining_events =
                        PlayerEventDispatcher::dispatch(player_event, field, context, match_data);

                    if process_remaining_events && !player_remaining_events.is_empty() {
                        remaining_events.add_range(player_remaining_events);
                    }
                }
            }
        }

        if process_remaining_events && remaining_events.has_events() {
            Self::dispatch_iter(remaining_events.drain(), field, context, match_data, false)
        }
    }
}
