//! **What a match is**: when and where it is played, in what competition,
//! with how much riding on it and between whom. The match's weather,
//! pitch, crowd and referee are drawn from these facts
//! (`MatchEnvironment::for_fixture`, `RefereeProfile::draw`), seeded by
//! the fixture so the same fixture always draws the same.

use chrono::NaiveDate;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CompetitionKind {
    League,
    DomesticCup,
    Continental,
    International,
    Friendly,
}

impl CompetitionKind {
    /// How much an ordinary fixture of this kind matters, before its own
    /// stage or table says more.
    pub fn importance(self) -> f32 {
        match self {
            CompetitionKind::Friendly => 0.1,
            CompetitionKind::League => 0.45,
            CompetitionKind::DomesticCup => 0.5,
            CompetitionKind::Continental | CompetitionKind::International => 0.6,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct FixtureContext {
    pub date: NaiveDate,
    pub competition: CompetitionKind,
    /// Level after normal time, it goes to extra time and penalties.
    pub knockout: bool,
    /// 0..1, how much rides on it.
    pub importance: f32,
    /// The ground is south of the equator, where the seasons run the
    /// other way.
    pub southern: bool,
    /// Share of the ground expected to be full, 0..1.
    pub gate: f32,
    /// 0..1: a derby.
    pub rivalry: f32,
    /// Seeds everything drawn from the fixture.
    pub seed: u64,
}

impl FixtureContext {
    pub const EUROPE: u32 = 1;
    pub const SOUTH_AMERICA: u32 = 3;
    const OCEANIA: u32 = 5;
    /// A ground three-quarters full: what is assumed when nobody says.
    const TYPICAL_GATE: f32 = 0.75;

    /// A fixture of `competition` identified by `id` on `date`, at an
    /// ordinary northern ground, between two sides who are not rivals.
    pub fn new(id: &str, date: NaiveDate, competition: CompetitionKind, knockout: bool) -> Self {
        FixtureContext {
            date,
            competition,
            knockout,
            importance: competition.importance(),
            southern: false,
            gate: Self::TYPICAL_GATE,
            rivalry: 0.0,
            seed: Self::seed_of(id, date),
        }
    }

    pub fn with_importance(mut self, importance: f32) -> Self {
        self.importance = importance.clamp(0.0, 1.0);
        self
    }

    /// The continent the ground is on.
    pub fn on_continent(mut self, continent_id: u32) -> Self {
        self.southern = matches!(continent_id, Self::SOUTH_AMERICA | Self::OCEANIA);
        self
    }

    /// How full the ground is expected to be.
    pub fn with_gate(mut self, gate: f32) -> Self {
        self.gate = gate.clamp(0.0, 1.0);
        self
    }

    pub fn with_rivalry(mut self, rivalry: f32) -> Self {
        self.rivalry = rivalry.clamp(0.0, 1.0);
        self
    }

    pub fn is_friendly(&self) -> bool {
        self.competition == CompetitionKind::Friendly
    }

    /// FNV-1a over the fixture's id and date: stable across runs and
    /// machines, different for every fixture.
    fn seed_of(id: &str, date: NaiveDate) -> u64 {
        let date = date.format("%Y%m%d").to_string();
        id.bytes()
            .chain(date.bytes())
            .fold(0xcbf2_9ce4_8422_2325u64, |hash, byte| {
                (hash ^ byte as u64).wrapping_mul(0x0000_0100_0000_01b3)
            })
    }
}
