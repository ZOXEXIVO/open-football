use crate::context::GlobalContext;
use crate::country::CountryResult;
use crate::league::LeagueCollection;
use crate::transfers::market::{TransferMarket};
use crate::utils::Logging;
use crate::{Club, ClubResult, PlayerStatusType, StaffPosition};
use crate::club::staff::result::ScoutRecommendation;
use chrono::NaiveDate;
use log::{debug, info};
use std::collections::HashMap;
use crate::country::builder::CountryBuilder;

pub struct ScoutingInterest {
    pub player_id: u32,
    pub interested_club_id: u32,
    pub recommendation: ScoutRecommendation,
    pub date: NaiveDate,
}

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
    pub scouting_interests: Vec<ScoutingInterest>,
}

impl Country {
    pub fn builder() -> CountryBuilder {
        CountryBuilder::default()
    }

    pub fn simulate(&mut self, ctx: GlobalContext<'_>) -> CountryResult {
        let country_name = self.name.clone();
        let date = ctx.simulation.date.date();

        info!("🌍 Simulating country: {} (Reputation: {})", country_name, self.reputation);

        // Phase 1: League Competitions
        let league_results = self.simulate_leagues(&ctx);

        // Phase 2: Club Operations
        let clubs_results = self.simulate_clubs(&ctx);

        // Phase 3: Country-level scouting
        self.process_scouting(date);

        info!("✅ Country {} simulation complete", country_name);

        CountryResult::new(self.id, league_results, clubs_results)
    }

    fn process_scouting(&mut self, date: NaiveDate) {
        use crate::utils::IntegerUtils;

        // Pass 1: Collect scout info and player summaries (immutable reads)
        struct ScoutInfo {
            club_id: u32,
            judging_ability: u8,
            judging_potential: u8,
        }

        struct PlayerSummary {
            player_id: u32,
            club_id: u32,
            current_ability: u8,
            potential_ability: u8,
        }

        let mut scouts = Vec::new();
        let mut all_players = Vec::new();

        for club in &self.clubs {
            for team in &club.teams.teams {
                // Collect scouts from team staff
                for staff in &team.staffs.staffs {
                    let is_scout = matches!(
                        staff.contract.as_ref().map(|c| &c.position),
                        Some(StaffPosition::Scout) | Some(StaffPosition::ChiefScout)
                    );
                    if is_scout {
                        scouts.push(ScoutInfo {
                            club_id: club.id,
                            judging_ability: staff.staff_attributes.knowledge.judging_player_ability,
                            judging_potential: staff.staff_attributes.knowledge.judging_player_potential,
                        });
                    }
                }

                // Collect players
                for player in &team.players.players {
                    all_players.push(PlayerSummary {
                        player_id: player.id,
                        club_id: club.id,
                        current_ability: player.player_attributes.current_ability,
                        potential_ability: player.player_attributes.potential_ability,
                    });
                }
            }
        }

        if scouts.is_empty() || all_players.is_empty() {
            return;
        }

        // For each scout, with 30% daily chance, evaluate a random player from another club
        let mut interests: Vec<ScoutingInterest> = Vec::new();

        for scout in &scouts {
            // 30% daily chance of scouting
            if IntegerUtils::random(0, 100) > 30 {
                continue;
            }

            // Pick a random player from another club
            let other_players: Vec<&PlayerSummary> = all_players
                .iter()
                .filter(|p| p.club_id != scout.club_id)
                .collect();

            if other_players.is_empty() {
                continue;
            }

            let idx = IntegerUtils::random(0, other_players.len() as i32) as usize;
            let idx = idx.min(other_players.len() - 1);
            let target = other_players[idx];

            // Evaluate with error margin based on scout skill
            let ability_error = (20i16 - scout.judging_ability as i16).max(1) as i32;
            let potential_error = (20i16 - scout.judging_potential as i16).max(1) as i32;

            let assessed_ability = (target.current_ability as i32
                + IntegerUtils::random(-ability_error, ability_error))
                .clamp(1, 100) as u8;
            let assessed_potential = (target.potential_ability as i32
                + IntegerUtils::random(-potential_error, potential_error))
                .clamp(1, 100) as u8;

            let recommendation = if assessed_ability > 75 || assessed_potential > 85 {
                ScoutRecommendation::Sign
            } else if assessed_ability > 60 || assessed_potential > 70 {
                ScoutRecommendation::Monitor
            } else {
                ScoutRecommendation::Pass
            };

            if recommendation == ScoutRecommendation::Pass {
                continue;
            }

            debug!(
                "Scout from club {} evaluated player {} (ability:{}, potential:{}) -> {:?}",
                scout.club_id, target.player_id, assessed_ability, assessed_potential, recommendation
            );

            interests.push(ScoutingInterest {
                player_id: target.player_id,
                interested_club_id: scout.club_id,
                recommendation,
                date,
            });
        }

        // Pass 2: Apply statuses to players (mutable writes)
        for interest in &interests {
            for club in &mut self.clubs {
                for team in &mut club.teams.teams {
                    if let Some(player) = team.players.players.iter_mut().find(|p| p.id == interest.player_id) {
                        match interest.recommendation {
                            ScoutRecommendation::Sign => {
                                if !player.statuses.get().contains(&PlayerStatusType::Wnt) {
                                    player.statuses.add(date, PlayerStatusType::Wnt);
                                }
                            }
                            ScoutRecommendation::Monitor => {
                                if !player.statuses.get().contains(&PlayerStatusType::Sct)
                                    && !player.statuses.get().contains(&PlayerStatusType::Wnt)
                                {
                                    player.statuses.add(date, PlayerStatusType::Sct);
                                }
                            }
                            ScoutRecommendation::Pass => {}
                        }
                    }
                }
            }
        }

        // Store interests for transfer negotiation use
        self.scouting_interests.extend(interests);

        // Prune old interests (older than 30 days)
        let cutoff = date - chrono::Duration::days(30);
        self.scouting_interests.retain(|i| i.date >= cutoff);
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

impl Default for CountryEconomicFactors {
    fn default() -> Self {
        Self::new()
    }
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
    pub fn simulate_round(&mut self, _date: NaiveDate) {
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

impl Default for MediaCoverage {
    fn default() -> Self {
        Self::new()
    }
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

    pub fn update_from_results(&mut self, _results: &[crate::league::LeagueResult]) {
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

#[allow(dead_code)]
#[derive(Debug)]
struct TransferActivitySummary {
    total_listings: u32,
    active_negotiations: u32,
    completed_transfers: u32,
    total_fees_exchanged: f64,
}

#[allow(dead_code)]
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

#[allow(dead_code)]
#[derive(Debug)]
struct SquadAnalysis {
    surplus_positions: Vec<crate::PlayerPositionType>,
    needed_positions: Vec<crate::PlayerPositionType>,
    average_age: f32,
    quality_level: u8,
}

#[allow(dead_code)]
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
