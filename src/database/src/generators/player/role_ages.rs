//! How old real players are in the seasons each squad role stands for.

use super::SquadRole;
use crate::generators::rng::HydrationRng;
use crate::loaders::OdbPlayer;
use chrono::Datelike;
use std::collections::HashMap;

/// The seed records' real seasons, by position kind and appearance band:
/// the ages a generated player of each squad role is drawn from. A keeper
/// earns a regular's season years later than an outfielder does, and a
/// role drawn from the whole age range would make a nineteen-year-old a
/// first-choice keeper as often as a twenty-six-year-old.
#[derive(Default)]
pub struct RoleAgeTable {
    /// `[keeper][band]`, ages sorted.
    ages: [[Vec<u8>; 3]; 2],
}

impl RoleAgeTable {
    /// Seasons of 30 or more appearances, of 10 to 29, of 1 to 9.
    const REGULAR: usize = 0;
    const ROTATION: usize = 1;
    const OCCASIONAL: usize = 2;

    /// Years a prospect's age is drawn from, at the young end of his team's
    /// range.
    const PROSPECT_YEARS: i32 = 5;

    /// One row per recorded season, every spell of the season summed; a
    /// player's age in a season is its start year less his birth year.
    pub fn from_records<'a>(records: impl Iterator<Item = &'a OdbPlayer>) -> Self {
        let mut table = RoleAgeTable::default();
        for record in records {
            let best = record.positions.iter().map(|p| p.level).max();
            let keeper = record
                .positions
                .iter()
                .find(|p| Some(p.level) == best)
                .is_some_and(|p| p.code == "GK");
            let mut seasons: HashMap<u16, u32> = HashMap::new();
            for item in &record.history {
                *seasons.entry(item.season).or_default() += item.played as u32;
            }
            let born = record.birth_date.year();
            for (season, apps) in seasons {
                let band = match apps {
                    0 => continue,
                    1..=9 => Self::OCCASIONAL,
                    10..=29 => Self::ROTATION,
                    _ => Self::REGULAR,
                };
                let age = (season as i32 - born).clamp(0, u8::MAX as i32) as u8;
                table.ages[keeper as usize][band].push(age);
            }
        }
        for kind in &mut table.ages {
            for band in kind {
                band.sort_unstable();
            }
        }
        table
    }

    /// A generated player's age in a team whose players are `min..=max`:
    /// drawn from the seasons his role stands for, within the team's range.
    /// A prospect is drawn evenly from the range's youngest years; a band
    /// with no recorded season in range falls back to the whole range.
    pub fn draw(
        &self,
        keeper: bool,
        role: SquadRole,
        min: i32,
        max: i32,
        rng: &mut HydrationRng,
    ) -> i32 {
        let band = match role {
            SquadRole::Star | SquadRole::Starter => Self::REGULAR,
            SquadRole::Rotation => Self::ROTATION,
            SquadRole::Backup | SquadRole::Fringe => Self::OCCASIONAL,
            SquadRole::Prospect => {
                return rng.int_range(min, (min + Self::PROSPECT_YEARS).min(max + 1));
            }
        };
        let ages = &self.ages[keeper as usize][band];
        let lo = ages.partition_point(|&a| (a as i32) < min);
        let hi = ages.partition_point(|&a| (a as i32) <= max);
        if lo == hi {
            return rng.int_range(min, max + 1);
        }
        let at = lo + ((rng.f32() * (hi - lo) as f32) as usize).min(hi - lo - 1);
        ages[at] as i32
    }
}
