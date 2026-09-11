use crate::club::player::development::CoachingEffect;
use crate::context::SimulationContext;
use crate::continent::{ContinentAwardOutcome, ContinentResult};
use crate::country::CountryResult;
use crate::simulator::phase::MatchdayOutcome;
use crate::simulator::{SimulationResult, WorldMatchdayResult};
use crate::transfers::pipeline::approach::ApproachPass;
use crate::utils::{DateUtils, PerformanceProfiler};
use crate::world::SimulatorData;
use rayon::prelude::*;
use std::collections::HashSet;

/// Everything the matchday deferred, plus the calendar's own work.
///
/// The passes here are ordered by what they must see: the batched
/// cross-country sweeps read the deferred ops *before* the drain commits
/// them, and the season snapshot freezes a borrowing club's loanees
/// *before* the drain's loan returns move them home.
pub struct PeriodicPasses;

impl PeriodicPasses {
    pub fn run(
        data: &mut SimulatorData,
        outcome: MatchdayOutcome<'_>,
        result: &mut SimulationResult,
    ) {
        let today = data.date.date();
        let MatchdayOutcome {
            matchday,
            world_pool,
            free_agents,
            ..
        } = outcome;

        // The matchday's parallel pass read these snapshots directly; we
        // republish the same view here so any caller that reaches for the
        // `daily_*` caches (test harnesses, continental-cup paths) finds
        // it. Cleared at the end of the phase so the next tick rebuilds.
        data.daily_world_player_pool = Some(world_pool);
        data.daily_global_free_agents = Some(free_agents);

        Self::continent_periodic(data, today);
        Self::batched_market_sweeps(data, &matchday, today);
        Self::free_agent_development(data);
        Self::season_snapshot(data, &matchday, today);

        let _phase = PerformanceProfiler::phase_scope("C7_drain", 0);
        matchday.drain_into(data, result);

        data.daily_world_player_pool = None;
        data.daily_global_free_agents = None;
    }

    /// Monthly rankings, quarterly economic zone, yearly regulations and
    /// the year-end continental award. Each closure mutates only its own
    /// continent, so they run in parallel — these are the four heaviest
    /// periodic walks (rankings and economics aggregate every club; the
    /// award walks every player in every team in every league), which is
    /// why they are not left inside the serial drain.
    ///
    /// Applying the award is serial: `data.player_mut` resolves against
    /// every continent. Small N — three nominees and a winner per
    /// continent per year.
    fn continent_periodic(data: &mut SimulatorData, today: chrono::NaiveDate) {
        let phase = PerformanceProfiler::phase_scope("C1_continent_periodic", 0);
        let award_outcomes: Vec<ContinentAwardOutcome> = data
            .continents
            .par_iter_mut()
            .filter_map(|continent| {
                if DateUtils::is_month_beginning(today) {
                    ContinentResult::update_continental_rankings(continent);
                }
                if DateUtils::is_quarter_start(today) {
                    ContinentResult::update_economic_zone(continent);
                }
                if DateUtils::is_year_start(today) {
                    ContinentResult::update_continental_regulations(continent, today);
                }
                DateUtils::is_year_end(today)
                    .then(|| ContinentResult::build_continental_award_outcome(continent, today))
            })
            .collect();
        drop(phase);

        let _phase = PerformanceProfiler::phase_scope("C2_award_apply", 0);
        for outcome in award_outcomes {
            ContinentResult::apply_continental_award_outcome(data, outcome, today);
        }
    }

    /// The two world-wide sweeps the matchday staged rather than ran.
    ///
    /// Both used to fire per country and re-walk the world once per
    /// country: the interest cleanup re-walked every other country's
    /// shortlists once per domestic signing, and the free-agent bumps
    /// walked the whole pool per country — `O(countries × pool)`.
    /// Aggregating first turns each into one pass.
    fn batched_market_sweeps(
        data: &mut SimulatorData,
        matchday: &WorldMatchdayResult<'_>,
        today: chrono::NaiveDate,
    ) {
        let phase = PerformanceProfiler::phase_scope("C3_interest_cleanup", 0);
        let signed_ids = matchday.collect_domestic_signed_ids();
        ApproachPass::cleanup_player_transfer_interest_batch(data, &signed_ids);
        drop(phase);

        let _phase = PerformanceProfiler::phase_scope("C4_free_agent_bumps", 0);
        let bumps = matchday.collect_free_agent_bumps();
        ApproachPass::apply_free_agent_market_bumps_batch(data, &bumps, today);
    }

    /// Unattached players still age. A light weekly development tick with
    /// no club environment (neutral coach, league rep 0) keeps pool
    /// veterans declining and pool youngsters ticking over, instead of
    /// every free agent being frozen in time until someone signs them.
    fn free_agent_development(data: &mut SimulatorData) {
        let _phase = PerformanceProfiler::phase_scope("C5_free_agent_development", 0);
        if !SimulationContext::new(data.date).is_week_beginning() {
            return;
        }
        let neutral_coach = CoachingEffect::neutral();
        let today = data.date.date();
        data.free_agents
            .par_iter_mut()
            .filter(|p| !p.retired)
            .for_each(|p| p.process_development(today, 0, &neutral_coach, 0.0));
    }

    /// Season-start career-history snapshot, fanned out across countries
    /// × clubs in one pass. Runs BEFORE the drain so borrowing clubs
    /// freeze their loanees' stats before the cross-country loan returns
    /// inside the drain move them home. Country-local mutation only ⇒
    /// safe in `countries.par_iter_mut`.
    fn season_snapshot(
        data: &mut SimulatorData,
        matchday: &WorldMatchdayResult<'_>,
        today: chrono::NaiveDate,
    ) {
        let _phase = PerformanceProfiler::phase_scope("C6_season_snapshot", 0);
        let new_season: HashSet<u32> = matchday
            .collect_new_season_country_ids()
            .into_iter()
            .collect();
        if new_season.is_empty() {
            return;
        }
        data.continents
            .par_iter_mut()
            .flat_map(|c| c.countries.par_iter_mut())
            .for_each(|country| {
                if new_season.contains(&country.id) {
                    CountryResult::snapshot_country(country, today);
                }
            });
    }
}
