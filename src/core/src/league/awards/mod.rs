pub mod player_of_week;
mod season_awards;

pub use player_of_week::{
    PlayerOfTheWeekAward, PlayerOfTheWeekHistory, PlayerOfTheWeekSelector, WeeklyAggregate,
};
pub use season_awards::*;

/// Maximum age (inclusive) for the Young Player / Team of the Week
/// awards. Distinct from the monthly / season Young awards (which use
/// `<= 21`) — the weekly version has a tighter age window so it
/// rewards genuinely emerging talent rather than late-developing
/// twenty-ones.
pub const YOUNG_WEEKLY_MAX_AGE: u8 = 20;
