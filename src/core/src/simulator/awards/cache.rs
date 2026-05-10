use crate::league::awards::{AwardAggregator, CandidateAggregate, WeeklyAggregate};
use crate::league::player_of_week::PlayerOfTheWeekSelector;
use crate::simulator::SimulatorData;
use rayon::prelude::*;
use std::collections::HashMap;

/// Per-league aggregates for a single Monday window, built once and
/// shared across all four weekly award ticks (Player of the Week, Young
/// Player of the Week, Team of the Week, Young Team of the Week). Each
/// tick used to walk every league's match storage and re-aggregate the
/// same window — four full passes per league per Monday. This caches
/// both the `WeeklyAggregate` (driver of Player of the Week) and the
/// `CandidateAggregate` (driver of Team of the Week) so the four ticks
/// reduce to a per-league lookup.
pub(crate) struct MondayAwardCache {
    weekly: HashMap<u32, HashMap<u32, WeeklyAggregate>>,
    candidate: HashMap<u32, HashMap<u32, CandidateAggregate>>,
}

impl MondayAwardCache {
    pub(crate) fn build(
        data: &SimulatorData,
        week_start: chrono::NaiveDate,
        week_end: chrono::NaiveDate,
    ) -> Self {
        let entries: Vec<(
            u32,
            HashMap<u32, WeeklyAggregate>,
            HashMap<u32, CandidateAggregate>,
        )> = data
            .continents
            .par_iter()
            .flat_map(|c| c.countries.par_iter())
            .flat_map(|country| country.leagues.leagues.par_iter())
            .filter(|league| !league.friendly)
            .map(|league| {
                let weekly = PlayerOfTheWeekSelector::aggregate(
                    league.matches.iter_in_range(week_start, week_end),
                );
                let candidate =
                    AwardAggregator::aggregate(league.matches.iter_in_range(week_start, week_end));
                (league.id, weekly, candidate)
            })
            .collect();

        let mut weekly: HashMap<u32, HashMap<u32, WeeklyAggregate>> =
            HashMap::with_capacity(entries.len());
        let mut candidate: HashMap<u32, HashMap<u32, CandidateAggregate>> =
            HashMap::with_capacity(entries.len());
        for (lid, w, c) in entries {
            weekly.insert(lid, w);
            candidate.insert(lid, c);
        }
        MondayAwardCache { weekly, candidate }
    }

    pub(super) fn weekly_for(&self, league_id: u32) -> Option<&HashMap<u32, WeeklyAggregate>> {
        self.weekly.get(&league_id)
    }

    pub(super) fn candidate_for(
        &self,
        league_id: u32,
    ) -> Option<&HashMap<u32, CandidateAggregate>> {
        self.candidate.get(&league_id)
    }
}
