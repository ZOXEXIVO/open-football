use crate::context::GlobalContext;
use crate::country::CountryResult;
use crate::country::national_team::{NationalTeam, CallUpCandidate};
use crate::league::LeagueCollection;
use crate::transfers::market::TransferMarket;
use crate::utils::Logging;
use crate::{Club, ClubResult};
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
}

impl Country {
    pub fn builder() -> CountryBuilder {
        CountryBuilder::default()
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
        let league_results = self.simulate_leagues(&ctx);

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

    fn simulate_leagues(&mut self, ctx: &GlobalContext<'_>) -> Vec<crate::league::LeagueResult> {
        self.leagues.simulate(&self.clubs, ctx)
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
