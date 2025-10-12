use crate::context::GlobalContext;
use crate::country::CountryResult;
use crate::league::LeagueCollection;
use crate::shared::{Currency, CurrencyValue};
use crate::transfers::market::{TransferListingType, TransferMarket};
use crate::utils::Logging;
use crate::{Club, ClubResult};
use chrono::{Datelike, NaiveDate};
use log::{debug, info};
use rayon::iter::{IntoParallelRefMutIterator, ParallelIterator};
use std::collections::HashMap;
use crate::country::builder::CountryBuilder;

pub struct Country {
    pub id: u32,
    pub code: String,
    pub slug: String,
    pub name: String,
    pub continent_id: u32,
    pub leagues: LeagueCollection,
    pub clubs: Vec<Club>,
    pub reputation: u16,
    pub generator_data: CountryGeneratorData,

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

    pub fn simulate(&mut self, ctx: GlobalContext<'_>) -> CountryResult {
        let country_name = self.name.clone();

        info!("🌍 Simulating country: {} (Reputation: {})", country_name, self.reputation);

        // Phase 1: League Competitions
        let league_results = self.simulate_leagues(&ctx);

        // Phase 2: Club Operations
        let clubs_results = self.simulate_clubs(&ctx);

        info!("✅ Country {} simulation complete", country_name);

        CountryResult::new(league_results, clubs_results)
    }

    fn simulate_leagues(&mut self, ctx: &GlobalContext<'_>) -> Vec<crate::league::LeagueResult> {
        self.leagues.simulate(&self.clubs, ctx)
    }

    fn simulate_clubs(&mut self, ctx: &GlobalContext<'_>) -> Vec<ClubResult> {
        self.clubs
            .iter_mut()
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

// Supporting structures

#[derive(Debug, Clone)]
pub struct CountryEconomicFactors {
    pub gdp_growth: f32,
    pub inflation_rate: f32,
    pub tv_revenue_multiplier: f32,
    pub sponsorship_market_strength: f32,
    pub stadium_attendance_factor: f32,
}

impl CountryEconomicFactors {
    pub fn new() -> Self {
        CountryEconomicFactors {
            gdp_growth: 0.02,
            inflation_rate: 0.03,
            tv_revenue_multiplier: 1.0,
            sponsorship_market_strength: 1.0,
            stadium_attendance_factor: 1.0,
        }
    }

    pub fn get_financial_multiplier(&self) -> f32 {
        1.0 + self.gdp_growth - self.inflation_rate
    }

    pub fn monthly_update(&mut self) {
        // Simulate economic fluctuations
        use crate::utils::FloatUtils;

        self.gdp_growth += FloatUtils::random(-0.005, 0.005);
        self.gdp_growth = self.gdp_growth.clamp(-0.05, 0.10);

        self.inflation_rate += FloatUtils::random(-0.003, 0.003);
        self.inflation_rate = self.inflation_rate.clamp(0.0, 0.10);

        self.tv_revenue_multiplier += FloatUtils::random(-0.02, 0.02);
        self.tv_revenue_multiplier = self.tv_revenue_multiplier.clamp(0.8, 1.5);
    }
}

#[derive(Debug)]
pub struct InternationalCompetition {
    pub name: String,
    pub competition_type: CompetitionType,
    pub participating_clubs: Vec<u32>,
    pub current_round: String,
}

impl InternationalCompetition {
    pub fn simulate_round(&mut self, date: NaiveDate) {
        // Simulate competition rounds
        debug!("Simulating {} round: {}", self.name, self.current_round);
    }
}

#[derive(Debug)]
pub enum CompetitionType {
    ChampionsLeague,
    EuropaLeague,
    ConferenceLeague,
    SuperCup,
}

#[derive(Debug)]
pub struct MediaCoverage {
    pub intensity: f32,
    pub trending_stories: Vec<MediaStory>,
    pub pressure_targets: HashMap<u32, f32>, // club_id -> pressure level
}

impl MediaCoverage {
    pub fn new() -> Self {
        MediaCoverage {
            intensity: 0.5,
            trending_stories: Vec::new(),
            pressure_targets: HashMap::new(),
        }
    }

    pub fn get_pressure_level(&self) -> f32 {
        self.intensity
    }

    pub fn update_from_results(&mut self, results: &[crate::league::LeagueResult]) {
        // Update media intensity based on exciting results
        self.intensity = (self.intensity * 0.9 + 0.1).min(1.0);
    }

    pub fn generate_weekly_stories(&mut self, clubs: &[Club]) {
        self.trending_stories.clear();

        // Generate stories based on club performance, transfers, etc.
        use crate::utils::IntegerUtils;

        for club in clubs {
            if IntegerUtils::random(0, 100) > 80 {
                self.trending_stories.push(MediaStory {
                    club_id: club.id,
                    story_type: StoryType::TransferRumor,
                    intensity: 0.5,
                });
            }
        }
    }
}

#[derive(Debug)]
pub struct MediaStory {
    pub club_id: u32,
    pub story_type: StoryType,
    pub intensity: f32,
}

#[derive(Debug)]
pub enum StoryType {
    TransferRumor,
    ManagerPressure,
    PlayerControversy,
    SuccessStory,
    CrisisStory,
}

#[derive(Debug, Clone)]
pub struct CountryRegulations {
    pub foreign_player_limit: Option<u8>,
    pub salary_cap: Option<f64>,
    pub homegrown_requirements: Option<u8>,
    pub ffp_enabled: bool, // Financial Fair Play
}

impl CountryRegulations {
    pub fn new() -> Self {
        CountryRegulations {
            foreign_player_limit: None,
            salary_cap: None,
            homegrown_requirements: None,
            ffp_enabled: false,
        }
    }
}

#[derive(Debug)]
struct TransferActivitySummary {
    total_listings: u32,
    active_negotiations: u32,
    completed_transfers: u32,
    total_fees_exchanged: f64,
}

impl TransferActivitySummary {
    fn new() -> Self {
        TransferActivitySummary {
            total_listings: 0,
            active_negotiations: 0,
            completed_transfers: 0,
            total_fees_exchanged: 0.0,
        }
    }

    fn get_market_heat_index(&self) -> f32 {
        // Calculate how "hot" the transfer market is
        let activity = (self.active_negotiations as f32 + self.completed_transfers as f32) / 100.0;
        activity.min(1.0)
    }
}

#[derive(Debug)]
struct SquadAnalysis {
    surplus_positions: Vec<crate::PlayerPositionType>,
    needed_positions: Vec<crate::PlayerPositionType>,
    average_age: f32,
    quality_level: u8,
}

struct CountrySimulationContext {
    economic_multiplier: f32,
    transfer_market_heat: f32,
    media_pressure: f32,
    regulatory_constraints: CountryRegulations,
}

// Update CountryGeneratorData and PeopleNameGeneratorData as per original
pub struct CountryGeneratorData {
    pub people_names: PeopleNameGeneratorData,
}

impl CountryGeneratorData {
    pub fn new(first_names: Vec<String>, last_names: Vec<String>) -> Self {
        CountryGeneratorData {
            people_names: PeopleNameGeneratorData {
                first_names,
                last_names,
            },
        }
    }

    pub fn empty() -> Self {
        CountryGeneratorData {
            people_names: PeopleNameGeneratorData {
                first_names: Vec::new(),
                last_names: Vec::new(),
            },
        }
    }
}

pub struct PeopleNameGeneratorData {
    pub first_names: Vec<String>,
    pub last_names: Vec<String>,
}
