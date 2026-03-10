use chrono::{Datelike, NaiveDate};
use crate::context::GlobalContext;
use crate::country::CountryResult;
use crate::country::national_team::{NationalTeam, CallUpCandidate};
use crate::league::LeagueCollection;
use crate::transfers::market::TransferMarket;
use crate::utils::Logging;
use crate::{Club, ClubResult, Player};
use log::{debug};
use rayon::prelude::IntoParallelRefMutIterator;
use crate::country::builder::CountryBuilder;
use rayon::iter::ParallelIterator;

use super::{
    CountrySettings, CountryGeneratorData, CountryEconomicFactors,
    InternationalCompetition, MediaCoverage, CountryRegulations,
};

#[derive(Clone)]
pub struct Country {
    pub id: u32,
    pub code: String,
    pub slug: String,
    pub name: String,
    pub background_color: String,
    pub foreground_color: String,
    pub continent_id: u32,
    pub leagues: LeagueCollection,
    pub clubs: Vec<Club>,
    pub reputation: u16,
    pub settings: CountrySettings,
    pub generator_data: CountryGeneratorData,

    pub national_team: NationalTeam,

    pub transfer_market: TransferMarket,
    pub economic_factors: CountryEconomicFactors,
    pub international_competitions: Vec<InternationalCompetition>,
    pub media_coverage: MediaCoverage,
    pub regulations: CountryRegulations,

    pub retired_players: Vec<Player>,
}

/// Season boundary dates derived from a country's primary league settings.
#[derive(Debug, Clone, Copy)]
pub struct SeasonDates {
    /// Day/month when the season ends (from season_ending_half.to_day/to_month)
    pub end_day: u8,
    pub end_month: u8,
    /// Day/month when the new season starts (from season_starting_half.from_day/from_month)
    pub start_day: u8,
    pub start_month: u8,
}

impl Default for SeasonDates {
    fn default() -> Self {
        SeasonDates { end_day: 31, end_month: 5, start_day: 20, start_month: 8 }
    }
}

impl SeasonDates {
    /// Check if the given date is the season end day.
    pub fn is_season_end(&self, date: NaiveDate) -> bool {
        date.day() as u8 == self.end_day && date.month() as u8 == self.end_month
    }

    /// Check if the given date falls in the off-season (between season end and season start).
    pub fn is_off_season(&self, date: NaiveDate) -> bool {
        let m = date.month() as u8;
        let d = date.day() as u8;
        let after_end = m > self.end_month
            || (m == self.end_month && d > self.end_day);
        let before_start = m < self.start_month
            || (m == self.start_month && d < self.start_day);
        after_end && before_start
    }
}

impl Country {
    pub fn builder() -> CountryBuilder {
        CountryBuilder::default()
    }

    /// Get season dates from the country's primary (tier-1, non-friendly) league.
    /// Falls back to May 31 / Aug 20 if no league is found.
    pub fn season_dates(&self) -> SeasonDates {
        self.leagues.leagues.iter()
            .find(|l| !l.friendly && l.settings.tier == 1)
            .or_else(|| self.leagues.leagues.iter().find(|l| !l.friendly))
            .map(|l| SeasonDates {
                end_day: l.settings.season_ending_half.to_day,
                end_month: l.settings.season_ending_half.to_month,
                start_day: l.settings.season_starting_half.from_day,
                start_month: l.settings.season_starting_half.from_month,
            })
            .unwrap_or_default()
    }

    pub(crate) fn simulate(
        &mut self,
        ctx: GlobalContext<'_>,
        country_ids: &[(u32, String)],
        candidates: Option<Vec<CallUpCandidate>>,
    ) -> CountryResult {
        let country_name = self.name.clone();
        let _date = ctx.simulation.date.date();

        debug!("🌍 Simulating country: {} (Reputation: {})", country_name, self.reputation);

        // Phase 1: League Competitions
        let league_results = self.leagues.simulate(&self.clubs, &ctx);

        // Phase 2: National Team (international breaks)
        let date = ctx.simulation.date.date();
        let country_id = self.id;
        
        // Pass country context to national team
        self.national_team.country_name = self.name.clone();
        self.national_team.reputation = self.reputation;
        
        self.national_team.simulate_state(&mut self.clubs, date, country_id, country_ids, candidates);

        // Phase 3: Club Operations
        let clubs_results = self.simulate_clubs(&ctx);

        debug!("✅ Country {} simulation complete", country_name);

        CountryResult::new(self.id, league_results, clubs_results)
    }

    fn simulate_clubs(&mut self, ctx: &GlobalContext<'_>) -> Vec<ClubResult> {
        self.clubs
            .par_iter_mut()
            .map(|club| {
                let message = &format!("simulate club: {}", &club.name);
                Logging::estimate_result(
                    || club.simulate(ctx.with_club(club.id, &club.name.clone())),
                    message,
                )
            })
            .collect()
    }
}
