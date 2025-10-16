use chrono::Datelike;
use chrono::NaiveDate;
use log::{debug, info};
use crate::league::LeagueResult;
use crate::simulator::SimulatorData;
use crate::{Club, ClubResult, ClubTransferStrategy, Country, SimulationResult};
use crate::shared::{Currency, CurrencyValue};
use crate::transfers::{TransferListing, TransferListingType, TransferWindowManager};

pub struct CountryResult {
    pub leagues: Vec<LeagueResult>,
    pub clubs: Vec<ClubResult>,
}

impl CountryResult {
    pub fn new(leagues: Vec<LeagueResult>, clubs: Vec<ClubResult>) -> Self {
        CountryResult { leagues, clubs }
    }

    pub fn process(self, data: &mut SimulatorData, result: &mut SimulationResult) {
        let current_date = data.date.date();
        let country_id = self.get_country_id(data);

        // Phase 3: Pre-season activities (if applicable)
        if Self::is_preseason(current_date) {
            self.simulate_preseason_activities(data, country_id, current_date);
        }

        // Phase 4: Transfer Market Activities
        let transfer_activities = self.simulate_transfer_market(data, country_id, current_date);

        // Phase 5: International Competitions
        self.simulate_international_competitions(data, country_id, current_date);

        // Phase 6: Economic Updates
        self.update_economic_factors(data, country_id, current_date);

        // Phase 7: Media and Public Interest
        self.simulate_media_coverage(data, country_id, &self.leagues);

        // Phase 8: End of Period Processing
        self.process_end_of_period(data, country_id, current_date, &self.clubs);

        // Phase 9: Country Reputation Update
        self.update_country_reputation(data, country_id, &self.leagues, &self.clubs);

        // Phase 1: Process league results
        for league_result in self.leagues {
            league_result.process(data, result);
        }

        // Phase 2: Process club results
        for club_result in self.clubs {
            club_result.process(data, result);
        }
    }

    // Helper methods

    fn get_country_id(&self, data: &SimulatorData) -> u32 {
        // Get country ID from first club or league result
        // Adjust based on your actual data structure
        if let Some(club_result) = self.clubs.first() {
            // Get club from data and return its country_id
            // This is a placeholder
            0
        } else {
            0
        }
    }

    fn is_preseason(date: NaiveDate) -> bool {
        let month = date.month();
        // Preseason typically June-July in Europe
        month == 6 || month == 7
    }

    fn simulate_preseason_activities(&self, data: &mut SimulatorData, country_id: u32, date: NaiveDate) {
        info!("⚽ Running preseason activities...");

        if let Some(country) = data.country_mut(country_id) {
            // Schedule friendlies between clubs
            Self::schedule_friendly_matches(country, date);

            // Training camps and tours
            Self::organize_training_camps(country);

            // Preseason tournaments
            Self::organize_preseason_tournaments(country);
        }
    }

    fn simulate_transfer_market(
        &self,
        data: &mut SimulatorData,
        country_id: u32,
        current_date: NaiveDate,
    ) -> TransferActivitySummary {
        let mut summary = TransferActivitySummary::new();

        // Check if transfer window is open
        let window_manager = TransferWindowManager::new();
        if !window_manager.is_window_open(country_id, current_date) {
            return summary;
        }

        info!("💰 Transfer window is OPEN - simulating market activity");

        // Get country to access its data
        if let Some(country) = data.country_mut(country_id) {
            // Phase 1: Clubs list players for transfer
            Self::list_players_for_transfer(country, current_date, &mut summary);

            // Phase 2: Generate interest and negotiate transfers
            Self::negotiate_transfers(country, current_date, &mut summary);

            // Phase 3: Process loan deals
            Self::process_loan_deals(country, current_date, &mut summary);

            // Phase 4: Free agents and contract expirations
            Self::handle_free_agents(country, current_date, &mut summary);

            // Phase 5: Update market based on completed deals
            country.transfer_market.update(current_date);

            debug!(
                "Transfer Activity - Listings: {}, Negotiations: {}, Completed: {}",
                summary.total_listings, summary.active_negotiations, summary.completed_transfers
            );
        }

        summary
    }

    fn list_players_for_transfer(
        country: &mut Country,
        date: NaiveDate,
        summary: &mut TransferActivitySummary,
    ) {
        // Collect player listing data to avoid borrow conflicts
        let mut listings_to_add = Vec::new();

        for club in &country.clubs {
            // Analyze squad and determine transfer needs
            let squad_analysis = Self::analyze_squad_needs(club);

            // List surplus players
            for player in &club.teams.teams[0].players.players {
                if Self::should_list_player(player, &squad_analysis, club) {
                    let asking_price = Self::calculate_asking_price(player, club, date);

                    listings_to_add.push((
                        player.id,
                        club.id,
                        club.teams.teams[0].id,
                        asking_price,
                    ));
                }
            }
        }

        // Now add all listings
        for (player_id, club_id, team_id, asking_price) in listings_to_add {
            let listing = TransferListing::new(
                player_id,
                club_id,
                team_id,
                asking_price,
                date,
                TransferListingType::Transfer,
            );

            country.transfer_market.add_listing(listing);
            summary.total_listings += 1;
        }
    }

    fn negotiate_transfers(
        country: &mut Country,
        date: NaiveDate,
        summary: &mut TransferActivitySummary,
    ) {
        // Collect negotiations to process (avoid borrow conflicts)
        let mut negotiations_to_process = Vec::new();

        // First pass: collect all potential negotiations
        for buying_club in &country.clubs {
            let budget = CurrencyValue {
                amount: 1_000_000.0,
                currency: Currency::Usd,
            };

            // Get club's transfer strategy
            let strategy = ClubTransferStrategy {
                club_id: buying_club.id,
                budget: Some(budget),
                selling_willingness: 0.5,
                buying_aggressiveness: Self::calculate_buying_aggressiveness(buying_club),
                target_positions: Self::identify_target_positions(buying_club),
                reputation_level: 0,
            };

            // Collect available listings
            let available_listings: Vec<_> = country
                .transfer_market
                .get_available_listings()
                .into_iter()
                .filter(|listing| listing.club_id != buying_club.id)
                .cloned()
                .collect();

            // Look through available listings
            for listing in available_listings {
                // Find the player
                if let Some(player) = Self::find_player_in_country(country, listing.player_id) {
                    if strategy.decide_player_interest(player) {
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
        }

        // Second pass: process all negotiations
        for (player_id, buying_club_id, selling_club_id, offer) in negotiations_to_process {
            if let Some(neg_id) = country.transfer_market.start_negotiation(
                player_id,
                buying_club_id,
                offer,
                date,
            ) {
                summary.active_negotiations += 1;

                // Simulate negotiation outcome
                if Self::simulate_negotiation_outcome(neg_id, selling_club_id, buying_club_id) {
                    if let Some(completed) = country.transfer_market.complete_transfer(neg_id, date)
                    {
                        summary.completed_transfers += 1;
                        summary.total_fees_exchanged += completed.fee.amount;
                    }
                }
            }
        }
    }

    fn simulate_international_competitions(
        &self,
        data: &mut SimulatorData,
        country_id: u32,
        date: NaiveDate,
    ) {
        if let Some(country) = data.country_mut(country_id) {
            // Simulate continental competitions (e.g., Champions League, Europa League)
            for competition in &mut country.international_competitions {
                competition.simulate_round(date);
            }
        }
    }

    fn update_economic_factors(
        &self,
        data: &mut SimulatorData,
        country_id: u32,
        date: NaiveDate,
    ) {
        // Update country's economic indicators monthly
        if date.day() == 1 {
            if let Some(country) = data.country_mut(country_id) {
                country.economic_factors.monthly_update();
            }
        }
    }

    fn simulate_media_coverage(
        &self,
        data: &mut SimulatorData,
        country_id: u32,
        league_results: &[LeagueResult],
    ) {
        if let Some(country) = data.country_mut(country_id) {
            country.media_coverage.update_from_results(league_results);
            country.media_coverage.generate_weekly_stories(&country.clubs);
        }
    }

    fn process_end_of_period(
        &self,
        data: &mut SimulatorData,
        country_id: u32,
        date: NaiveDate,
        club_results: &[ClubResult],
    ) {
        // End of season processing
        if date.month() == 5 && date.day() == 31 {
            info!("📅 End of season processing");

            if let Some(country) = data.country_mut(country_id) {
                // Award ceremonies
                Self::process_season_awards(country, club_results);

                // Contract expirations
                Self::process_contract_expirations(country);

                // Retirements
                Self::process_player_retirements(country, date);
            }
        }

        // End of year processing
        if date.month() == 12 && date.day() == 31 {
            if let Some(country) = data.country_mut(country_id) {
                Self::process_year_end_finances(country);
            }
        }
    }

    fn update_country_reputation(
        &self,
        data: &mut SimulatorData,
        country_id: u32,
        league_results: &[crate::league::LeagueResult],
        club_results: &[ClubResult],
    ) {
        if let Some(country) = data.country_mut(country_id) {
            // Base reputation change
            let mut reputation_change: i16 = 0;

            // Factor 1: League competitiveness
            for league in &country.leagues.leagues {
                let competitiveness = Self::calculate_league_competitiveness(league);
                reputation_change += (competitiveness * 5.0) as i16;
            }

            // Factor 2: International competition performance
            let international_success = Self::calculate_international_success(country);
            reputation_change += international_success as i16;

            // Factor 3: Transfer market activity
            let transfer_reputation = Self::calculate_transfer_market_reputation(country);
            reputation_change += transfer_reputation as i16;

            // Apply change with bounds
            let new_reputation = (country.reputation as i16 + reputation_change).clamp(0, 1000) as u16;

            if new_reputation != country.reputation {
                debug!(
                    "Country {} reputation changed: {} -> {} ({})",
                    country.name,
                    country.reputation,
                    new_reputation,
                    if reputation_change > 0 {
                        format!("+{}", reputation_change)
                    } else {
                        reputation_change.to_string()
                    }
                );
                country.reputation = new_reputation;
            }
        }
    }

    // Helper methods

    fn analyze_squad_needs(club: &Club) -> SquadAnalysis {
        SquadAnalysis {
            surplus_positions: vec![],
            needed_positions: vec![],
            average_age: 25.0,
            quality_level: 50,
        }
    }

    fn should_list_player(
        player: &crate::Player,
        analysis: &SquadAnalysis,
        club: &Club,
    ) -> bool {
        false // Simplified for now
    }

    fn calculate_asking_price(
        player: &crate::Player,
        club: &Club,
        date: NaiveDate,
    ) -> CurrencyValue {
        use crate::transfers::window::PlayerValuationCalculator;

        let base_value = PlayerValuationCalculator::calculate_value(player, date);

        let multiplier = if club.finance.balance.balance < 0 {
            0.9 // Financial pressure
        } else {
            1.1 // No pressure
        };

        CurrencyValue {
            amount: base_value.amount * multiplier,
            currency: base_value.currency,
        }
    }

    fn calculate_buying_aggressiveness(club: &Club) -> f32 {
        0.6 // Simplified
    }

    fn identify_target_positions(club: &Club) -> Vec<crate::PlayerPositionType> {
        vec![] // Simplified
    }

    fn find_player_in_country(country: &Country, player_id: u32) -> Option<&crate::Player> {
        for club in &country.clubs {
            for team in &club.teams.teams {
                if let Some(player) = team.players.players.iter().find(|p| p.id == player_id) {
                    return Some(player);
                }
            }
        }
        None
    }

    fn simulate_negotiation_outcome(
        negotiation_id: u32,
        selling_club: u32,
        buying_club: u32,
    ) -> bool {
        use crate::utils::IntegerUtils;
        IntegerUtils::random(0, 100) > 60 // 40% success rate
    }

    fn schedule_friendly_matches(country: &mut Country, date: NaiveDate) {
        debug!("Scheduling preseason friendlies");
    }

    fn organize_training_camps(country: &mut Country) {
        debug!("Organizing training camps");
    }

    fn organize_preseason_tournaments(country: &mut Country) {
        debug!("Organizing preseason tournaments");
    }

    fn process_loan_deals(country: &mut Country, date: NaiveDate, summary: &mut TransferActivitySummary) {
        debug!("Processing loan deals");
    }

    fn handle_free_agents(country: &mut Country, date: NaiveDate, summary: &mut TransferActivitySummary) {
        debug!("Handling free agents");
    }

    fn calculate_league_competitiveness(league: &crate::league::League) -> f32 {
        0.5 // Simplified
    }

    fn calculate_international_success(country: &Country) -> i16 {
        0 // Simplified
    }

    fn calculate_transfer_market_reputation(country: &Country) -> i16 {
        0 // Simplified
    }

    fn process_season_awards(country: &mut Country, club_results: &[ClubResult]) {
        debug!("Processing season awards");
    }

    fn process_contract_expirations(country: &mut Country) {
        debug!("Processing contract expirations");
    }

    fn process_player_retirements(country: &mut Country, date: NaiveDate) {
        debug!("Processing player retirements");
    }

    fn process_year_end_finances(country: &mut Country) {
        debug!("Processing year-end finances");
    }
}

// Supporting structures (keep these in country.rs)

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