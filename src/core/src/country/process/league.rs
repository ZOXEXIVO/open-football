use crate::country::country::Country;
use crate::league::LeagueResult;
use crate::league::LeagueTableResult;
use crate::league::result::{CountryProcessCtx, DeferredGlobalOps, WorldSnapshot};
use crate::r#match::MatchResult;
use crate::shared::CountryLookupIndex;
use crate::utils::PerformanceProfiler;

/// The per-league result fan-out: `process_local` for every result that
/// carries matches, then the daily cup and playoff winner awards.
pub struct CountryLeagueFanout;

impl CountryLeagueFanout {
    pub fn run(
        country: &mut Country,
        world: WorldSnapshot<'_>,
        league_results: Vec<LeagueResult>,
        lookup: &CountryLookupIndex,
        deferred: &mut DeferredGlobalOps,
    ) -> (Vec<LeagueResult>, Vec<(u32, Vec<MatchResult>)>) {
        let country_name = country.name.clone();
        let stage = PerformanceProfiler::stage_scope("country_match_fanout", 2)
            .labelled(|| country_name.clone());
        let processed = Self::process_results(country, world, league_results, lookup, deferred);
        drop(stage);

        country.on_domestic_cup_winner_awards(world.date.date());
        country.on_playoff_winner_awards(world.date.date());
        processed
    }

    fn process_results(
        country: &mut Country,
        world: WorldSnapshot<'_>,
        league_results: Vec<LeagueResult>,
        lookup: &CountryLookupIndex,
        deferred: &mut DeferredGlobalOps,
    ) -> (Vec<LeagueResult>, Vec<(u32, Vec<MatchResult>)>) {
        let mut processed: Vec<(u32, Vec<MatchResult>)> = Vec::new();
        let mut league_results_after = Vec::with_capacity(league_results.len());

        for result in league_results {
            let league_id = result.league_id;
            if result.match_results.is_some() {
                let mut ctx_local = CountryProcessCtx {
                    country,
                    date: world.date,
                    country_info_ref: world.country_info,
                    indexes_ref: world.indexes,
                    deferred,
                    lookup: Some(lookup),
                };
                let mut out: Vec<MatchResult> = Vec::new();
                let new_season_started = result.new_season_started;
                result.process_local(&mut ctx_local, &mut out);
                processed.push((league_id, out));

                let mut placeholder = LeagueResult::new(league_id, LeagueTableResult {});
                placeholder.new_season_started = new_season_started;
                league_results_after.push(placeholder);
            } else {
                league_results_after.push(result);
            }
        }

        (league_results_after, processed)
    }
}
