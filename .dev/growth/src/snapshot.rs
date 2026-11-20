//! Every player on the opening day and on each anniversary of it.

use crate::tracker::{Track, WeeklyTracker};
use crate::world::{Kind, Place, WorldWalk};
use chrono::Duration;
use core::club::player::development::GrowthSource;
use core::{Person, SimulatorData};
use std::collections::HashMap;

pub const SKILLS: usize = 50;

#[derive(Clone)]
pub struct Snap {
    pub age: u8,
    pub kind: Kind,
    pub ca: u8,
    pub pa: u8,
    pub place: Place,
    pub rank: u32,
    pub league_rep: u16,
    /// Football banked for his career, mean over the families, 0..1.
    pub banked: f32,
    /// The ledger's last season (July to June): minutes, minutes at a
    /// fitting standard counted by their fit, and attribute change by
    /// source (sessions, matches, maturation, ageing, lay-off).
    pub minutes: f32,
    pub fitting_minutes: f32,
    pub sources: [f32; 5],
    pub skills: [f32; SKILLS],
    pub track: Track,
}

impl Snap {
    pub fn ratio(&self) -> f32 {
        self.ca as f32 / self.pa.max(1) as f32
    }
}

#[derive(Default)]
pub struct YearlySnapshots {
    pub years: Vec<HashMap<u32, Snap>>,
    pub names: Vec<String>,
}

impl YearlySnapshots {
    /// Read the world as it stands. The tick that just ran has moved the
    /// calendar on, so ages are read on the day it simulated.
    pub fn take(&mut self, data: &SimulatorData, tracker: &mut WeeklyTracker) {
        let date = data.date.date() - Duration::days(1);
        let mut snaps: HashMap<u32, Snap> = HashMap::new();
        let names = &mut self.names;
        WorldWalk::visit(data, |seen| {
            let p = seen.player;
            let season = p.development.ledger.last_season();
            let mut skills = [0.0f32; SKILLS];
            for (i, (id, value)) in p.skills.iter_all().enumerate() {
                skills[i] = value;
                if names.len() <= i {
                    names.push(format!("{id:?}"));
                }
            }
            snaps.insert(
                p.id,
                Snap {
                    age: p.age(date),
                    kind: Kind::of(p.position()),
                    ca: p.player_attributes.current_ability,
                    pa: p.player_attributes.potential_ability,
                    place: seen.place,
                    rank: seen.rank,
                    league_rep: seen.league_rep,
                    banked: p.development.football.share(),
                    minutes: season.minutes(),
                    fitting_minutes: season.fitting_minutes(),
                    sources: GrowthSource::ALL.map(|source| season.by_source(source)),
                    skills,
                    track: tracker.track(p.id),
                },
            );
        });
        tracker.next_year();
        self.years.push(snaps);
    }

    pub fn last(&self) -> usize {
        self.years.len() - 1
    }

    pub fn skill(&self, name: &str) -> usize {
        self.names.iter().position(|n| n == name).unwrap_or(0)
    }
}
