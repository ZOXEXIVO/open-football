//! A player's development, read back: this season's and last season's change
//! by source and family, and the football behind it.

use super::ceilings::PositionalSkillCeilings;
use super::skills_array::SKILL_COUNT;
use crate::club::player::maturation::MaturationGroup;
use chrono::{Datelike, NaiveDate};

/// Where a change of attributes came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GrowthSource {
    Sessions = 0,
    Matches = 1,
    Maturation = 2,
    Ageing = 3,
    Layoff = 4,
}

impl GrowthSource {
    pub const COUNT: usize = 5;
    pub const ALL: [GrowthSource; Self::COUNT] = [
        GrowthSource::Sessions,
        GrowthSource::Matches,
        GrowthSource::Maturation,
        GrowthSource::Ageing,
        GrowthSource::Layoff,
    ];
}

/// One season of a player's development.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct SeasonGrowth {
    /// Attribute points changed, by source and maturation family.
    change: [[f32; MaturationGroup::COUNT]; GrowthSource::COUNT],
    /// Minutes of football played, and the same minutes weighted by how
    /// well each match's standard fitted him.
    minutes: f32,
    fitting_minutes: f32,
    /// Minutes at a standard below his own, each counted by the share of
    /// it his fit loses.
    outgrown_minutes: f32,
    /// Football banked this season, as a share of a career's.
    banked: f32,
}

impl SeasonGrowth {
    pub fn change(&self, source: GrowthSource, family: MaturationGroup) -> f32 {
        self.change[source as usize][family as usize]
    }

    /// Attribute points changed by `source`, all families together.
    pub fn by_source(&self, source: GrowthSource) -> f32 {
        self.change[source as usize].iter().sum()
    }

    /// Every recorded change, all sources and families together.
    pub fn total(&self) -> f32 {
        self.change.iter().flatten().sum()
    }

    pub fn minutes(&self) -> f32 {
        self.minutes
    }

    /// Minutes at a standard that fits him: each minute counted by its fit.
    pub fn fitting_minutes(&self) -> f32 {
        self.fitting_minutes
    }

    /// How well, on average over his minutes, his football fitted him.
    pub fn mean_fit(&self) -> f32 {
        if self.minutes > 0.0 {
            self.fitting_minutes / self.minutes
        } else {
            0.0
        }
    }

    pub fn banked(&self) -> f32 {
        self.banked
    }

    pub fn outgrown_minutes(&self) -> f32 {
        self.outgrown_minutes
    }
}

/// What a player can know of his own development: whether he is still
/// gaining, how much football at a standard that fits him he has had, and
/// whether he has outgrown the football he is given, over the year behind
/// him. It carries no potential.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct DevelopmentRead {
    /// A full season of his record lies behind the read; without one it
    /// is no view.
    pub known: bool,
    /// Attribute points the year gave him, net of what it took.
    pub gain: f32,
    pub fitting_minutes: f32,
    /// Share of his minutes lost to a standard he has outgrown, 0..1.
    pub outgrown: f32,
}

impl DevelopmentRead {
    /// Ten full matches at a fitting standard in a year.
    const FITTING_FOOTBALL: f32 = 900.0;
    const OUTGROWN: f32 = 0.5;
    /// Attribute points in a year below which he is not gaining.
    const GAINING: f32 = 3.0;

    pub fn short_of_fitting_football(&self) -> bool {
        self.known
            && (self.fitting_minutes < Self::FITTING_FOOTBALL || self.outgrown > Self::OUTGROWN)
    }

    pub fn still_gaining(&self) -> bool {
        self.known && self.gain > Self::GAINING
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct GrowthLedger {
    season: u16,
    /// Whether `last` is a season he was followed through.
    followed: bool,
    this: SeasonGrowth,
    last: SeasonGrowth,
}

impl GrowthLedger {
    /// The season a date belongs to, by the year it starts in July.
    fn season_of(date: NaiveDate) -> u16 {
        let year = if date.month() >= 7 {
            date.year()
        } else {
            date.year() - 1
        };
        year.max(0) as u16
    }

    pub fn this_season(&self) -> &SeasonGrowth {
        &self.this
    }

    pub fn last_season(&self) -> &SeasonGrowth {
        &self.last
    }

    /// Close the season if `date` has moved into a new one.
    pub(super) fn roll(&mut self, date: NaiveDate) {
        let season = Self::season_of(date);
        if self.season == 0 {
            self.season = season;
        } else if season != self.season {
            self.followed = season == self.season + 1;
            self.last = if self.followed {
                self.this
            } else {
                SeasonGrowth::default()
            };
            self.this = SeasonGrowth::default();
            self.season = season;
        }
    }

    pub(super) fn record(
        &mut self,
        source: GrowthSource,
        before: &[f32; SKILL_COUNT],
        after: &[f32; SKILL_COUNT],
    ) {
        for i in 0..SKILL_COUNT {
            let d = after[i] - before[i];
            if d != 0.0 && !PositionalSkillCeilings::is_condition(i) {
                let family = PositionalSkillCeilings::maturation_group(i) as usize;
                self.this.change[source as usize][family] += d;
            }
        }
    }

    pub(super) fn record_football(&mut self, minutes: f32, fit: f32, below: bool, banked: f32) {
        self.this.minutes += minutes;
        self.this.fitting_minutes += minutes * fit;
        if below {
            self.this.outgrown_minutes += minutes * (1.0 - fit);
        }
        self.this.banked += banked;
    }

    /// The year behind `today`: this season so far and the part of last
    /// season still inside it.
    pub fn read(&self, today: NaiveDate) -> DevelopmentRead {
        let opened = NaiveDate::from_ymd_opt(self.season as i32, 7, 1).unwrap_or(today);
        let elapsed = ((today - opened).num_days() as f32 / 365.0).clamp(0.0, 1.0);
        let behind = if self.followed { 1.0 - elapsed } else { 0.0 };
        let year = |f: fn(&SeasonGrowth) -> f32| f(&self.this) + f(&self.last) * behind;
        let minutes = year(SeasonGrowth::minutes);
        DevelopmentRead {
            known: self.followed,
            gain: year(SeasonGrowth::total),
            fitting_minutes: year(SeasonGrowth::fitting_minutes),
            outgrown: if minutes > 0.0 {
                year(SeasonGrowth::outgrown_minutes) / minutes
            } else {
                0.0
            },
        }
    }
}
