use crate::awards::{
    MondayAwardCache, MonthlyAwardsTick, SeasonAwardsTick, TeamOfTheWeekTick, TeamOfTheYearTick,
    WeeklyAwardsTick, WorldPlayerOfYearTick, YoungTeamOfTheWeekTick, YoungWeeklyAwardsTick,
};
use crate::news::{ClubNewsroomTick, LeagueNewsroomTick};
use crate::utils::PerformanceProfiler;
use crate::world::SimulatorData;
use chrono::{Datelike, Duration, Weekday};

/// Who the week and the month belonged to, and the papers that say so.
///
/// Award order is load-bearing. Largest weekly award first, so the
/// centralised award-reputation pipeline can dampen the smaller one when
/// both go to the same player; Young Player of the Week fires before the
/// senior award because its breakthrough-amplified base is larger; both
/// Team selections are dampened against either weekly winner. The press
/// always goes last, so the morning's winners can make their own front
/// pages.
pub struct Honours;

impl Honours {
    pub fn run(data: &mut SimulatorData) {
        let today = data.date.date();

        let phase = PerformanceProfiler::phase_scope("F1_monday_awards", 0);
        if today.weekday() == Weekday::Mon {
            Self::monday(data, today);
        }
        drop(phase);

        let _phase = PerformanceProfiler::phase_scope("F2_periodic_awards", 0);
        Self::periodic(data);
    }

    /// The four Monday tickers all need per-league weekly aggregates, so
    /// the cache is built once (in parallel across leagues) and shared —
    /// each tick used to re-aggregate the same week's matches on its own.
    /// They run after the matchday pipeline has flushed last week's
    /// results into each league's `MatchStorage`.
    fn monday(data: &mut SimulatorData, today: chrono::NaiveDate) {
        let week_start = today - Duration::days(7);
        let cache = MondayAwardCache::build(data, week_start, today);

        YoungWeeklyAwardsTick::run(data, &cache);
        WeeklyAwardsTick::run(data, &cache);
        YoungTeamOfTheWeekTick::run(data, &cache);
        TeamOfTheWeekTick::run(data, &cache);

        ClubNewsroomTick::run(data, week_start, today);
    }

    /// Monthly, season-end and calendar-year honours. The league papers
    /// read the scoring charts `MonthlyAwardsTick` has just frozen, so
    /// they come strictly after it, never beside it.
    fn periodic(data: &mut SimulatorData) {
        MonthlyAwardsTick::run(data);
        LeagueNewsroomTick::run(data);
        // Drain any league-side pending season-awards snapshots and emit
        // the player events while stats are still meaningful.
        SeasonAwardsTick::run(data);
        TeamOfTheYearTick::run(data);
        // Built from the per-continent rankings, so a top performer in
        // any league can win.
        WorldPlayerOfYearTick::run(data);
    }
}
