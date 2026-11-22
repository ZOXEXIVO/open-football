use crate::club::ClubResult;
use crate::context::GlobalContext;
use crate::country::CountryResult;
use crate::country::country::{Country, CountryPendingState};
use crate::country::result::transfers::DeferredTransferOps;
use crate::league::LeagueResult;
use crate::league::result::{DeferredGlobalOps, LeaguePendingState};
use crate::r#match::MatchResult;
use crate::utils::PerformanceProfiler;
use chrono::NaiveDate;
use rustc_hash::FxHashMap;

/// The day's match results as the country routes them: league, cup and
/// playoff results plus the processed-match and deferred-op state that
/// travel to the final `CountryResult`.
pub struct CountryMatchdayResults {
    league_results: Vec<LeagueResult>,
    clubs_results: Vec<ClubResult>,
    processed_match_results: Vec<(u32, Vec<MatchResult>)>,
    deferred_global_ops: DeferredGlobalOps,
}

impl CountryMatchdayResults {
    pub fn route(
        country: &mut Country,
        ctx: &GlobalContext<'_>,
        current_date: NaiveDate,
        pending: CountryPendingState,
        match_results: Vec<MatchResult>,
    ) -> Self {
        let mut by_league: FxHashMap<u32, Vec<MatchResult>> = FxHashMap::default();
        for result in match_results {
            by_league.entry(result.league_id).or_default().push(result);
        }

        let CountryPendingState {
            leagues,
            cup,
            playoffs,
            immediate_results,
        } = pending;
        let mut league_results = immediate_results;
        let stage = PerformanceProfiler::stage_scope("country_league_results", 2)
            .labelled(|| country.name.clone());

        Self::route_leagues(
            country,
            ctx,
            current_date,
            leagues,
            &mut by_league,
            &mut league_results,
        );
        if let Some(cup) = cup {
            Self::route_cup(
                country,
                ctx,
                current_date,
                cup,
                &mut by_league,
                &mut league_results,
            );
        }
        Self::route_playoffs(
            country,
            ctx,
            current_date,
            playoffs,
            &mut by_league,
            &mut league_results,
        );

        drop(stage);
        Self {
            league_results,
            clubs_results: Vec::new(),
            processed_match_results: Vec::new(),
            deferred_global_ops: DeferredGlobalOps::new(),
        }
    }

    fn route_leagues(
        country: &mut Country,
        ctx: &GlobalContext<'_>,
        current_date: NaiveDate,
        pending: Vec<Option<LeaguePendingState>>,
        by_league: &mut FxHashMap<u32, Vec<MatchResult>>,
        league_results: &mut Vec<LeagueResult>,
    ) {
        for (league, slot) in country.leagues.leagues.iter_mut().zip(pending) {
            let Some(pending) = slot else {
                continue;
            };
            let results = by_league.remove(&league.id).unwrap_or_default();
            let league_ctx =
                ctx.with_league(league.id, league.slug.clone(), &[], league.reputation);
            league_results.push(league.simulate_process(
                results,
                pending,
                &country.clubs,
                &league_ctx,
                current_date,
            ));
        }
    }

    fn route_cup(
        country: &mut Country,
        ctx: &GlobalContext<'_>,
        current_date: NaiveDate,
        pending: LeaguePendingState,
        by_league: &mut FxHashMap<u32, Vec<MatchResult>>,
        league_results: &mut Vec<LeagueResult>,
    ) {
        let Some((cup_id, cup_slug, cup_rep)) = country.domestic_cup.as_ref().map(|cup| {
            (
                cup.league.id,
                cup.league.slug.clone(),
                cup.league.reputation,
            )
        }) else {
            return;
        };
        let cup_ctx = ctx.with_league(cup_id, cup_slug, &[], cup_rep);
        let cup = country.domestic_cup.as_mut().expect("cup checked above");
        let results = by_league.remove(&cup_id).unwrap_or_default();
        let cup_result =
            cup.simulate_process(results, pending, &country.clubs, &cup_ctx, current_date);
        Self::pay_cup_prizes(country, &cup_result);
        league_results.push(cup_result);
    }

    fn pay_cup_prizes(country: &mut Country, cup_result: &LeagueResult) {
        let payouts = country
            .domestic_cup
            .as_ref()
            .map(|cup| {
                cup.round_prize_payouts(
                    cup_result.match_results.as_deref().unwrap_or(&[]),
                    &country.clubs,
                    country.economic_factors.tv_revenue_multiplier,
                )
            })
            .unwrap_or_default();
        for (club_id, amount) in payouts {
            if let Some(club) = country.clubs.iter_mut().find(|club| club.id == club_id) {
                club.finance.balance.push_income_cup_prize(amount);
            }
        }
    }

    fn route_playoffs(
        country: &mut Country,
        ctx: &GlobalContext<'_>,
        current_date: NaiveDate,
        pending: Vec<Option<LeaguePendingState>>,
        by_league: &mut FxHashMap<u32, Vec<MatchResult>>,
        league_results: &mut Vec<LeagueResult>,
    ) {
        for (playoff, slot) in country.playoffs.iter_mut().zip(pending) {
            let Some(pending) = slot else {
                continue;
            };
            let results = by_league.remove(&playoff.league.id).unwrap_or_default();
            let playoff_ctx = ctx.with_league(
                playoff.league.id,
                playoff.league.slug.clone(),
                &[],
                playoff.league.reputation,
            );
            league_results.push(playoff.simulate_process(
                results,
                pending,
                &country.clubs,
                &playoff_ctx,
                current_date,
            ));
        }
    }

    pub fn league_results(&self) -> &[LeagueResult] {
        &self.league_results
    }

    pub fn with_clubs(mut self, clubs_results: Vec<ClubResult>) -> Self {
        self.clubs_results = clubs_results;
        self
    }

    pub fn take_clubs(&mut self) -> Vec<ClubResult> {
        std::mem::take(&mut self.clubs_results)
    }

    pub fn take_league_results(&mut self) -> Vec<LeagueResult> {
        std::mem::take(&mut self.league_results)
    }

    pub fn set_league_results(&mut self, league_results: Vec<LeagueResult>) {
        self.league_results = league_results;
    }

    pub fn record_processed(&mut self, processed: Vec<(u32, Vec<MatchResult>)>) {
        self.processed_match_results = processed;
    }

    pub fn deferred_global_ops_mut(&mut self) -> &mut DeferredGlobalOps {
        &mut self.deferred_global_ops
    }

    pub fn into_country_result(
        self,
        country_id: u32,
        deferred_transfer_ops: DeferredTransferOps,
    ) -> CountryResult {
        let mut result = CountryResult::new(country_id, self.league_results, self.clubs_results);
        result.processed_match_results = self.processed_match_results;
        result.deferred_global_ops = self.deferred_global_ops;
        result.deferred_transfer_ops = Some(deferred_transfer_ops);
        result
    }
}
