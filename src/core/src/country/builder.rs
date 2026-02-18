use crate::league::LeagueCollection;
use crate::transfers::market::TransferMarket;
use crate::{Club, Country, CountryEconomicFactors, CountryGeneratorData, CountryRegulations, CountrySettings, InternationalCompetition, MediaCoverage};

#[derive(Default)]
pub struct CountryBuilder {
    id: Option<u32>,
    code: Option<String>,
    slug: Option<String>,
    name: Option<String>,
    color: Option<String>,
    continent_id: Option<u32>,
    leagues: Option<LeagueCollection>,
    clubs: Option<Vec<Club>>,
    reputation: Option<u16>,
    settings: Option<CountrySettings>,
    generator_data: Option<CountryGeneratorData>,
    transfer_market: Option<TransferMarket>,
    economic_factors: Option<CountryEconomicFactors>,
    international_competitions: Option<Vec<InternationalCompetition>>,
    media_coverage: Option<MediaCoverage>,
    regulations: Option<CountryRegulations>,
}

impl CountryBuilder {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn id(mut self, id: u32) -> Self {
        self.id = Some(id);
        self
    }

    pub fn code(mut self, code: String) -> Self {
        self.code = Some(code);
        self
    }

    pub fn slug(mut self, slug: String) -> Self {
        self.slug = Some(slug);
        self
    }

    pub fn name(mut self, name: String) -> Self {
        self.name = Some(name);
        self
    }

    pub fn color(mut self, color: String) -> Self {
        self.color = Some(color);
        self
    }

    pub fn continent_id(mut self, continent_id: u32) -> Self {
        self.continent_id = Some(continent_id);
        self
    }

    pub fn leagues(mut self, leagues: LeagueCollection) -> Self {
        self.leagues = Some(leagues);
        self
    }

    pub fn clubs(mut self, clubs: Vec<Club>) -> Self {
        self.clubs = Some(clubs);
        self
    }

    pub fn reputation(mut self, reputation: u16) -> Self {
        self.reputation = Some(reputation);
        self
    }

    pub fn settings(mut self, settings: CountrySettings) -> Self {
        self.settings = Some(settings);
        self
    }

    pub fn generator_data(mut self, generator_data: CountryGeneratorData) -> Self {
        self.generator_data = Some(generator_data);
        self
    }

    pub fn transfer_market(mut self, transfer_market: TransferMarket) -> Self {
        self.transfer_market = Some(transfer_market);
        self
    }

    pub fn economic_factors(mut self, economic_factors: CountryEconomicFactors) -> Self {
        self.economic_factors = Some(economic_factors);
        self
    }

    pub fn international_competitions(mut self, competitions: Vec<InternationalCompetition>) -> Self {
        self.international_competitions = Some(competitions);
        self
    }

    pub fn media_coverage(mut self, media_coverage: MediaCoverage) -> Self {
        self.media_coverage = Some(media_coverage);
        self
    }

    pub fn regulations(mut self, regulations: CountryRegulations) -> Self {
        self.regulations = Some(regulations);
        self
    }

    pub fn build(self) -> Result<Country, String> {
        Ok(Country {
            id: self.id.ok_or("id is required")?,
            code: self.code.ok_or("code is required")?,
            slug: self.slug.ok_or("slug is required")?,
            name: self.name.ok_or("name is required")?,
            color: self.color.unwrap_or_else(|| "#1e272d".to_string()),
            continent_id: self.continent_id.ok_or("continent_id is required")?,
            leagues: self.leagues.ok_or("leagues is required")?,
            clubs: self.clubs.ok_or("clubs is required")?,
            reputation: self.reputation.unwrap_or(500), // Default reputation
            settings: self.settings.unwrap_or_default(),
            generator_data: self.generator_data.unwrap_or_else(CountryGeneratorData::empty),
            transfer_market: self.transfer_market.unwrap_or_else(TransferMarket::new),
            economic_factors: self.economic_factors.unwrap_or_else(CountryEconomicFactors::new),
            international_competitions: self.international_competitions.unwrap_or_default(),
            media_coverage: self.media_coverage.unwrap_or_else(MediaCoverage::new),
            regulations: self.regulations.unwrap_or_else(CountryRegulations::new),
            scouting_interests: Vec::new(),
        })
    }
}