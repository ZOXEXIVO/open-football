use crate::context::GlobalContext;
use crate::country::CountryResult;
use crate::league::LeagueCollection;
use crate::shared::{Currency, CurrencyValue};
use crate::transfers::market::{TransferListing, TransferListingType, TransferMarket};
use crate::transfers::window::TransferWindowManager;
use crate::utils::Logging;
use crate::{Club, ClubResult, ClubTransferStrategy};
use chrono::{Datelike, NaiveDate};
use log::{debug, info};
use std::collections::HashMap;

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
    pub fn new(id: u32,
               code: String,
               slug: String,
               name: String,
               continent_id: u32,
               leagues: LeagueCollection,
               clubs: Vec<Club>,
               reputation: u16,
               generator_data: CountryGeneratorData) -> Self {
        Country {
            id,
            code,
            slug,
            name,
            continent_id,
            leagues,
            clubs,
            reputation,
            generator_data,

            transfer_market: TransferMarket::new(),
            economic_factors: CountryEconomicFactors::new(),
            international_competitions: vec![],
            media_coverage: MediaCoverage::new(),
            regulations: CountryRegulations::new(),
        }
    }

    pub fn simulate(&mut self, ctx: GlobalContext<'_>) -> CountryResult {
        let country_name = self.name.clone();

        info!("🌍 Simulating country: {} (Reputation: {})", country_name, self.reputation);

        // Phase 1: Pre-season activities (if applicable)
        if self.is_preseason(&ctx) {
            self.simulate_preseason_activities(&ctx);
        }

        // Phase 2: Transfer Market Activities
        let transfer_activities = self.simulate_transfer_market(&ctx);

        // Phase 3: League Competitions
        let league_results = self.simulate_leagues(&ctx);

        // Phase 4: Club Operations (with awareness of league standings)
        let clubs_results = self.simulate_clubs_with_context(&ctx, &transfer_activities);

        // Phase 5: International Competitions
        self.simulate_international_competitions(&ctx);

        // Phase 6: Economic Updates
        self.update_economic_factors(&ctx);

        // Phase 7: Media and Public Interest
        self.simulate_media_coverage(&ctx, &league_results);

        // Phase 8: End of Period Processing
        self.process_end_of_period(&ctx, &clubs_results);

        // Phase 9: Country Reputation Update
        self.update_country_reputation(&league_results, &clubs_results);

        info!("✅ Country {} simulation complete. New reputation: {}",
              country_name, self.reputation);

        CountryResult::new(league_results, clubs_results)
    }

    fn is_preseason(&self, ctx: &GlobalContext<'_>) -> bool {
        let date = ctx.simulation.date.date();
        let month = date.month();
        // Preseason typically June-July in Europe
        month == 6 || month == 7
    }

    fn simulate_preseason_activities(&mut self, ctx: &GlobalContext<'_>) {
        info!("⚽ Running preseason activities...");

        // Schedule friendlies between clubs
        self.schedule_friendly_matches(ctx);

        // Training camps and tours
        self.organize_training_camps();

        // Preseason tournaments
        self.organize_preseason_tournaments();
    }

    fn simulate_transfer_market(&mut self, ctx: &GlobalContext<'_>) -> TransferActivitySummary {
        let current_date = ctx.simulation.date.date();
        let mut summary = TransferActivitySummary::new();

        // Check if transfer window is open
        let window_manager = TransferWindowManager::new();
        if !window_manager.is_window_open(self.id, current_date) {
            return summary;
        }

        info!("💰 Transfer window is OPEN - simulating market activity");

        // Phase 1: Clubs list players for transfer
        self.list_players_for_transfer(current_date, &mut summary);

        // Phase 2: Generate interest and negotiate transfers
        self.negotiate_transfers(current_date, &mut summary);

        // Phase 3: Process loan deals
        self.process_loan_deals(current_date, &mut summary);

        // Phase 4: Free agents and contract expirations
        self.handle_free_agents(current_date, &mut summary);

        // Phase 5: Update market based on completed deals
        self.transfer_market.update(current_date);

        debug!("Transfer Activity - Listings: {}, Negotiations: {}, Completed: {}",
               summary.total_listings, summary.active_negotiations, summary.completed_transfers);

        summary
    }

    fn list_players_for_transfer(&mut self, date: NaiveDate, summary: &mut TransferActivitySummary) {
        for club in &self.clubs {
            // Analyze squad and determine transfer needs
            let squad_analysis = self.analyze_squad_needs(club);

            // List surplus players
            for player in &club.teams.teams[0].players.players {
                if self.should_list_player(player, &squad_analysis, club) {
                    let asking_price = self.calculate_asking_price(player, club, date);

                    let listing = TransferListing::new(
                        player.id,
                        club.id,
                        club.teams.teams[0].id,
                        asking_price,
                        date,
                        TransferListingType::Transfer,
                    );

                    self.transfer_market.add_listing(listing);
                    summary.total_listings += 1;
                }
            }
        }
    }

    fn negotiate_transfers(&mut self, date: NaiveDate, summary: &mut TransferActivitySummary) {
        // Collect negotiations to process (avoid borrow conflicts)
        let mut negotiations_to_process = Vec::new();

        // Clubs look at available listings and make offers
        for buying_club in &self.clubs {
            let budget = CurrencyValue {
                amount: 1_000_000.0,
                currency: Currency::Usd,
            };

            // Get club's transfer strategy
            let strategy = ClubTransferStrategy {
                club_id: buying_club.id,
                budget: Some(budget),
                selling_willingness: 0.5,
                buying_aggressiveness: self.calculate_buying_aggressiveness(buying_club),
                target_positions: self.identify_target_positions(buying_club),
                reputation_level: 0, //buying_club.reputation.world,
            };

            // Collect available listings first to avoid borrow conflicts
            let available_listings: Vec<_> = self.transfer_market.get_available_listings()
                .into_iter()
                .filter(|listing| listing.club_id != buying_club.id)
                .cloned()
                .collect();

            // Look through available listings
            for listing in available_listings {
                // Find the player
                if let Some(player) = self.find_player(listing.player_id) && strategy.decide_player_interest(player) {
                    let offer = strategy.calculate_initial_offer(
                        player,
                        &listing.asking_price,
                        date,
                    );

                    negotiations_to_process.push((
                        listing.player_id,
                        buying_club.id,
                        listing.club_id,
                        offer,
                    ));
                }
            }
        }

        // Process all negotiations
        for (player_id, buying_club_id, selling_club_id, offer) in negotiations_to_process {
            if let Some(neg_id) = self.transfer_market.start_negotiation(
                player_id,
                buying_club_id,
                offer,
                date,
            ) {
                summary.active_negotiations += 1;

                // Simulate negotiation outcome
                if self.simulate_negotiation_outcome(neg_id, selling_club_id, buying_club_id) {
                    if let Some(completed) = self.transfer_market.complete_transfer(neg_id, date) {
                        summary.completed_transfers += 1;
                        summary.total_fees_exchanged += completed.fee.amount;
                    }
                }
            }
        }
    }

    fn simulate_leagues(&mut self, ctx: &GlobalContext<'_>) -> Vec<crate::league::LeagueResult> {
        self.leagues.simulate(&self.clubs, ctx)
    }

    fn simulate_clubs_with_context(
        &mut self,
        ctx: &GlobalContext<'_>,
        transfer_summary: &TransferActivitySummary,
    ) -> Vec<ClubResult> {
        // Add country-specific context for club simulation
        let country_context = CountrySimulationContext {
            economic_multiplier: self.economic_factors.get_financial_multiplier(),
            transfer_market_heat: transfer_summary.get_market_heat_index(),
            media_pressure: self.media_coverage.get_pressure_level(),
            regulatory_constraints: self.regulations.clone(),
        };

        // Apply context to all clubs first (avoiding the borrow conflict)
        for club in &mut self.clubs {
            Self::apply_country_context_to_club_static(club, &country_context);
        }

        // Then simulate clubs
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

    fn apply_country_context_to_club_static(club: &mut Club, context: &CountrySimulationContext) {
        // Apply economic multiplier to finances
        if let Some(budget) = &mut club.finance.transfer_budget {
            budget.amount *= context.economic_multiplier as f64;
        }

        // Apply regulatory constraints
        // This would affect things like foreign player limits, salary caps, etc.
    }

    fn simulate_international_competitions(&mut self, ctx: &GlobalContext<'_>) {
        // Simulate continental competitions (e.g., Champions League, Europa League)
        for competition in &mut self.international_competitions {
            competition.simulate_round(ctx.simulation.date.date());
        }
    }

    fn update_economic_factors(&mut self, ctx: &GlobalContext<'_>) {
        // Update country's economic indicators monthly
        if ctx.simulation.date.day() == 1 {
            self.economic_factors.monthly_update();
        }
    }

    fn simulate_media_coverage(&mut self, ctx: &GlobalContext<'_>, league_results: &[crate::league::LeagueResult]) {
        // Update media coverage based on recent results
        self.media_coverage.update_from_results(league_results);

        // Generate media stories and pressure
        if ctx.simulation.is_week_beginning() {
            self.media_coverage.generate_weekly_stories(&self.clubs);
        }
    }

    fn process_end_of_period(&mut self, ctx: &GlobalContext<'_>, club_results: &[ClubResult]) {
        let date = ctx.simulation.date.date();

        // End of season processing
        if date.month() == 5 && date.day() == 31 {
            info!("📅 End of season processing for {}", self.name);

            // Award ceremonies
            self.process_season_awards(club_results);

            // Contract expirations
            self.process_contract_expirations();

            // Retirements
            self.process_player_retirements(date);
        }

        // End of year processing
        if date.month() == 12 && date.day() == 31 {
            self.process_year_end_finances();
        }
    }

    fn update_country_reputation(&mut self, league_results: &[crate::league::LeagueResult], club_results: &[ClubResult]) {
        // Base reputation change
        let mut reputation_change: i16 = 0;

        // Factor 1: League competitiveness
        for league in &self.leagues.leagues {
            let competitiveness = self.calculate_league_competitiveness(league);
            reputation_change += (competitiveness * 5.0) as i16;
        }

        // Factor 2: International competition performance
        let international_success = self.calculate_international_success();
        reputation_change += international_success as i16;

        // Factor 4: Transfer market activity
        let transfer_reputation = self.calculate_transfer_market_reputation();
        reputation_change += transfer_reputation as i16;

        // Apply change with bounds
        let new_reputation = (self.reputation as i16 + reputation_change).clamp(0, 1000) as u16;

        if new_reputation != self.reputation {
            debug!("Country {} reputation changed: {} -> {} ({})",
                   self.name, self.reputation, new_reputation,
                   if reputation_change > 0 { format!("+{}", reputation_change) } else { reputation_change.to_string() });
            self.reputation = new_reputation;
        }
    }

    // Helper methods

    fn analyze_squad_needs(&self, club: &Club) -> SquadAnalysis {
        SquadAnalysis {
            surplus_positions: vec![],
            needed_positions: vec![],
            average_age: 25.0,
            quality_level: 50,
        }
    }

    fn should_list_player(&self, player: &crate::Player, analysis: &SquadAnalysis, club: &Club) -> bool {
        // Complex logic to determine if a player should be listed
        // Consider: performance, age, contract, squad role, etc.
        false // Simplified for now
    }

    fn calculate_asking_price(&self, player: &crate::Player, club: &Club, date: NaiveDate) -> CurrencyValue {
        use crate::transfers::window::PlayerValuationCalculator;

        let base_value = PlayerValuationCalculator::calculate_value(player, date);

        // Adjust based on club's willingness to sell
        let multiplier = if club.finance.balance.balance < 0 {
            0.9 // Financial pressure, willing to sell cheaper
        } else {
            1.1 // No pressure, demand premium
        };

        CurrencyValue {
            amount: base_value.amount * multiplier,
            currency: base_value.currency,
        }
    }

    fn calculate_buying_aggressiveness(&self, club: &Club) -> f32 {
        // Based on club's ambition, financial health, and league position
        0.6 // Simplified
    }

    fn identify_target_positions(&self, club: &Club) -> Vec<crate::PlayerPositionType> {
        // Analyze squad and identify weak positions
        vec![] // Simplified
    }

    fn find_player(&self, player_id: u32) -> Option<&crate::Player> {
        for club in &self.clubs {
            for team in &club.teams.teams {
                if let Some(player) = team.players.players.iter().find(|p| p.id == player_id) {
                    return Some(player);
                }
            }
        }
        None
    }

    fn simulate_negotiation_outcome(&self, negotiation_id: u32, selling_club: u32, buying_club: u32) -> bool {
        // Simulate whether negotiation succeeds
        // Consider: offer vs asking price, club relationships, player wishes, etc.
        use crate::utils::IntegerUtils;
        IntegerUtils::random(0, 100) > 60 // 40% success rate (simplified)
    }

    fn schedule_friendly_matches(&mut self, ctx: &GlobalContext<'_>) {
        // Schedule preseason friendlies between clubs
        debug!("Scheduling preseason friendlies");
    }

    fn organize_training_camps(&mut self) {
        // Some clubs go on training camps
        debug!("Organizing training camps");
    }

    fn organize_preseason_tournaments(&mut self) {
        // Cup competitions before season starts
        debug!("Organizing preseason tournaments");
    }

    fn process_loan_deals(&mut self, date: NaiveDate, summary: &mut TransferActivitySummary) {
        // Process loan transfers
        debug!("Processing loan deals");
    }

    fn handle_free_agents(&mut self, date: NaiveDate, summary: &mut TransferActivitySummary) {
        // Handle players without contracts
        debug!("Handling free agents");
    }

    fn calculate_league_competitiveness(&self, league: &crate::league::League) -> f32 {
        // Calculate based on point spread, goal differences, etc.
        0.5 // Simplified
    }

    fn calculate_international_success(&self) -> i16 {
        // Based on clubs' performance in continental competitions
        0 // Simplified
    }

    fn calculate_transfer_market_reputation(&self) -> i16 {
        // Based on high-profile transfers in/out
        0 // Simplified
    }

    fn process_season_awards(&self, club_results: &[ClubResult]) {
        debug!("Processing season awards");
    }

    fn process_contract_expirations(&mut self) {
        debug!("Processing contract expirations");
    }

    fn process_player_retirements(&mut self, date: NaiveDate) {
        debug!("Processing player retirements");
    }

    fn process_year_end_finances(&mut self) {
        debug!("Processing year-end finances");
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