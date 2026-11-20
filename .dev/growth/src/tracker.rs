//! What a player did in a year, read once a week: official minutes,
//! friendly-flagged appearances, where he spent his weeks and how many of
//! them on loan.

use crate::world::{Place, WorldWalk};
use core::SimulatorData;
use std::collections::HashMap;

/// How much football a year held, by official minutes.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Role {
    Regular = 0,
    Rotation = 1,
    Fringe = 2,
    YouthFootball = 3,
    None = 4,
}

impl Role {
    pub const NAMES: [&str; 5] = ["regular", "rotation", "fringe", "youthfb", "none"];

    pub fn name(self) -> &'static str {
        Self::NAMES[self as usize]
    }
}

#[derive(Clone, Copy, Default)]
pub struct Track {
    last_starts: u16,
    last_subs: u16,
    last_friendly: u16,
    pub minutes: f32,
    pub friendly_apps: u16,
    loan_weeks: u16,
    weeks: u16,
    place_weeks: [u16; 5],
}

impl Track {
    /// Official starts count 90 minutes and substitute appearances 20: the
    /// statistics hold appearances, not minutes.
    const START_MINUTES: f32 = 90.0;
    const SUB_MINUTES: f32 = 20.0;

    pub fn role(&self) -> Role {
        if self.minutes >= 1800.0 {
            Role::Regular
        } else if self.minutes >= 700.0 {
            Role::Rotation
        } else if self.friendly_apps >= 15 {
            Role::YouthFootball
        } else if self.minutes > 0.0 {
            Role::Fringe
        } else {
            Role::None
        }
    }

    /// Where he spent most of the year's weeks.
    pub fn place(&self) -> Place {
        let mut best = 0;
        for i in 1..Place::ALL.len() {
            if self.place_weeks[i] > self.place_weeks[best] {
                best = i;
            }
        }
        Place::ALL[best]
    }

    pub fn on_loan(&self) -> bool {
        self.weeks > 0 && self.loan_weeks * 2 >= self.weeks
    }

    /// A new year: the counters start again, the appearance totals they
    /// are read against do not.
    fn next_year(&self) -> Track {
        Track {
            last_starts: self.last_starts,
            last_subs: self.last_subs,
            last_friendly: self.last_friendly,
            ..Track::default()
        }
    }
}

#[derive(Default)]
pub struct WeeklyTracker {
    tracks: HashMap<u32, Track>,
}

impl WeeklyTracker {
    pub fn observe(&mut self, data: &SimulatorData) {
        let tracks = &mut self.tracks;
        WorldWalk::visit(data, |seen| {
            let p = seen.player;
            let t = tracks.entry(p.id).or_default();
            let starts = p.statistics.played + p.cup_statistics.played;
            let subs = p.statistics.played_subs + p.cup_statistics.played_subs;
            let friendly = p.friendly_statistics.played + p.friendly_statistics.played_subs;
            // A season's statistics are folded away at its start: a total
            // below last week's is a new season counted from zero.
            let delta = |now: u16, last: u16| if now >= last { now - last } else { now };
            t.minutes += delta(starts, t.last_starts) as f32 * Track::START_MINUTES
                + delta(subs, t.last_subs) as f32 * Track::SUB_MINUTES;
            t.friendly_apps += delta(friendly, t.last_friendly);
            t.last_starts = starts;
            t.last_subs = subs;
            t.last_friendly = friendly;
            t.weeks += 1;
            t.place_weeks[seen.place as usize] += 1;
            if p.is_on_loan() {
                t.loan_weeks += 1;
            }
        });
    }

    pub fn track(&self, id: u32) -> Track {
        self.tracks.get(&id).copied().unwrap_or_default()
    }

    pub fn next_year(&mut self) {
        for t in self.tracks.values_mut() {
            *t = t.next_year();
        }
    }
}
