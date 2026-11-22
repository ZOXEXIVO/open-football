mod media;
mod preseason;
mod registration;
mod reputation;
mod statistics;

use crate::club::ClubResult;
use crate::context::GlobalContext;
use crate::country::country::Country;
use crate::league::LeagueResult;
use crate::utils::PerformanceProfiler;
use chrono::NaiveDate;

/// The country-local half of a tick: fixture windows, club simulation, and
/// the country-owned processors that read those results.
pub struct CountryTicks;

impl CountryTicks {
    pub fn run(
        country: &mut Country,
        ctx: GlobalContext<'_>,
        current_date: NaiveDate,
        league_results: &[LeagueResult],
    ) -> Vec<ClubResult> {
        let country_name = country.name.clone();
        PerformanceProfiler::stage_labelled(
            "country_fixture_windows",
            2,
            || country_name.clone(),
            || country.refresh_team_fixture_windows(current_date),
        );

        let ctx = Self::with_country_economics(ctx, country);
        let clubs_results = PerformanceProfiler::stage_labelled(
            "country_club_simulation",
            2,
            || country_name.clone(),
            || country.simulate_clubs(&ctx),
        );

        let stage = PerformanceProfiler::stage_scope("country_local_results", 2)
            .labelled(|| country_name.clone());
        country.on_media_coverage(league_results);
        country.on_end_of_period(current_date, &clubs_results);
        country.on_country_reputation();
        country.on_international_competitions(current_date);
        country.on_economic_factors(current_date);
        if country.season_dates().is_off_season(current_date) {
            country.on_preseason_activities();
        }
        drop(stage);

        clubs_results
    }

    fn with_country_economics<'gc>(
        mut ctx: GlobalContext<'gc>,
        country: &Country,
    ) -> GlobalContext<'gc> {
        if let Some(ref mut country_ctx) = ctx.country {
            country_ctx.tv_revenue_multiplier = country.economic_factors.tv_revenue_multiplier;
            country_ctx.sponsorship_market_strength =
                country.economic_factors.sponsorship_market_strength;
            country_ctx.stadium_attendance_factor =
                country.economic_factors.stadium_attendance_factor;
            country_ctx.price_level = country.settings.pricing.price_level;
            country_ctx.reputation = country.reputation;
        }
        ctx
    }
}
