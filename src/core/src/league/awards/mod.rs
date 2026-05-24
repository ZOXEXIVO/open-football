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

/// Minimum `TeamOfTheWeekSelector::candidate_score` to qualify for a
/// Young Team of the Week slot. Without this gate, low-reputation
/// leagues with a thin U-20 pool fill all 11 slots every week with
/// whoever has the highest score above zero — so a routine 6.5-avg
/// kid in a tier-5 league racks up 6+ "young XI" appearances in a
/// couple of months. A 3.0 floor roughly maps to "either a 7.4+ avg
/// rating or a real contribution (goal / assist / clean sheet)" — a
/// real Young-XI bar instead of "best of what was available". Senior
/// Team of the Week intentionally keeps no floor (its pool is deep
/// enough that the issue is invisible).
pub const YOUNG_WEEKLY_TOTW_MIN_SCORE: f32 = 3.0;

/// Minimum `PlayerOfTheWeekSelector` weekly score to qualify as the
/// Young Player of the Week winner. Same rationale as the TOTW floor
/// but on the POW scoring scale, which has half the rating-term weight
/// (rating contributes `(avg-6.0).max(0).min(4)`, not `* 2.0`). A 2.0
/// floor maps to "a ≥ 7.5 avg with at least one real contribution
/// (goal / assist / MOTM / clean sheet)" — under that bar the league
/// simply doesn't crown a Young POW that week.
pub const YOUNG_WEEKLY_POW_MIN_SCORE: f32 = 2.0;
