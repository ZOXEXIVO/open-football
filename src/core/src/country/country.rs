use chrono::{Datelike, NaiveDate};
use crate::context::GlobalContext;
use crate::country::CountryResult;
use crate::country::national_team::NationalTeam;
use crate::league::LeagueCollection;
use crate::transfers::market::TransferMarket;
use crate::{Club, ClubResult, Player};
use log::debug;
use crate::country::builder::CountryBuilder;

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

    pub(crate) fn simulate(&mut self, ctx: GlobalContext<'_>) -> CountryResult {
        let country_name = self.name.clone();

        debug!("Simulating country: {} (Reputation: {})", country_name, self.reputation);

        // Phase 1: League Competitions
        let league_results = self.leagues.simulate(&self.clubs, &ctx);

        // Phase 2: Club Operations (with economic factors)
        // National team call-ups are handled at the continent level (cross-country visibility)
        let ctx = {
            let mut c = ctx;
            if let Some(ref mut country_ctx) = c.country {
                country_ctx.tv_revenue_multiplier = self.economic_factors.tv_revenue_multiplier;
                country_ctx.sponsorship_market_strength = self.economic_factors.sponsorship_market_strength;
                country_ctx.stadium_attendance_factor = self.economic_factors.stadium_attendance_factor;
                country_ctx.price_level = self.settings.pricing.price_level;
            }
            c
        };
        let clubs_results = self.simulate_clubs(&ctx);

        debug!("Country {} simulation complete", country_name);

        CountryResult::new(self.id, league_results, clubs_results)
    }

    fn simulate_clubs(&mut self, ctx: &GlobalContext<'_>) -> Vec<ClubResult> {
        // Build team_id → (league_position, league_size, total_matches) from league tables
        let mut team_league_info: std::collections::HashMap<u32, (u8, u8, u8)> = std::collections::HashMap::new();
        for league in &self.leagues.leagues {
            if league.friendly {
                continue;
            }
            let league_size = league.table.rows.len() as u8;
            // Total matches in a full season: (n-1) * 2 for double round-robin
            let total_matches = if league_size > 1 { (league_size - 1) * 2 } else { 0 };
            for (pos, row) in league.table.rows.iter().enumerate() {
                team_league_info.insert(row.team_id, ((pos + 1) as u8, league_size, total_matches));
            }
        }

        self.clubs
            .iter_mut()
            .map(|club| {
                // Find main team's league position
                let league_info = club.teams.main()
                    .and_then(|t| team_league_info.get(&t.id))
                    .copied()
                    .unwrap_or((0, 0, 0));

                let name = club.name.clone();
                let club_ctx = ctx.with_club(club.id, &name);
                let club_ctx = {
                    let mut c = club_ctx;
                    if let Some(ref mut cc) = c.club {
                        *cc = cc.clone().with_league_position(league_info.0, league_info.1, league_info.2);
                    }
                    c
                };
                club.simulate(club_ctx)
            })
            .collect()
    }
}
