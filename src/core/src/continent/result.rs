use crate::continent::{
    CompetitionTier, Continent, ContinentalCompetitions, ContinentalMatchResult,
    ContinentalRankings,
};
use crate::country::CountryResult;
use crate::simulator::SimulatorData;
use crate::transfers::CompletedTransfer;
use crate::{Club, Country, SimulationResult};
use chrono::Datelike;
use chrono::NaiveDate;
use log::{debug, info};
use std::collections::HashMap;

pub struct ContinentResult {
    pub countries: Vec<CountryResult>,

    // New fields for continental-level results
    pub competition_results: Option<ContinentalCompetitionResults>,
    pub rankings_update: Option<ContinentalRankingsUpdate>,
    pub transfer_summary: Option<CrossBorderTransferSummary>,
    pub economic_impact: Option<EconomicZoneImpact>,
}

impl ContinentResult {
    pub fn new(countries: Vec<CountryResult>) -> Self {
        ContinentResult {
            countries,
            competition_results: None,
            rankings_update: None,
            transfer_summary: None,
            economic_impact: None,
        }
    }

    pub fn process(self, data: &mut SimulatorData, result: &mut SimulationResult) {
        let current_date = data.date.date(); // Assuming SimulationResult has date

        // Phase 2: Update Continental Rankings (monthly)
        if current_date.day() == 1 {
            self.update_continental_rankings(data, result);
        }

        // Phase 3: Continental Competition Processing
        if self.is_competition_draw_period(current_date) {
            self.conduct_competition_draws(data, current_date);
        }

        let competition_results = self.simulate_continental_competitions(data, current_date);
        if let Some(comp_results) = competition_results {
            self.process_competition_results(comp_results, data, result);
        }

        // Phase 4: Continental Economic Updates (quarterly)
        if current_date.month() % 3 == 0 && current_date.day() == 1 {
            self.update_economic_zone(data, &self.countries);
        }

        // Phase 5: Continental Regulatory Updates (yearly)
        if current_date.month() == 1 && current_date.day() == 1 {
            self.update_continental_regulations(data, current_date);
        }

        // Phase 6: Continental Awards & Recognition (yearly)
        if current_date.month() == 12 && current_date.day() == 31 {
            self.process_continental_awards(data, &self.countries);
        }

        for country_result in self.countries {
            country_result.process(data, result);
        }
    }

    fn update_continental_rankings(&self, data: &mut SimulatorData, result: &mut SimulationResult) {
        info!("📊 Updating continental rankings");

        // Get continent from data
        let continent_id = self.get_continent_id(data);

        if let Some(continent) = data.continent_mut(continent_id) {
            // Update country coefficients based on club performances
            for country in &mut continent.countries {
                let coefficient = Self::calculate_country_coefficient(country, &continent.continental_competitions);
                continent.continental_rankings.update_country_ranking(country.id, coefficient);
            }

            // Update club rankings
            let all_clubs = Self::get_all_clubs(&continent.countries);
            for club in all_clubs {
                let club_points = Self::calculate_club_continental_points(club, &continent.continental_competitions);
                continent.continental_rankings.update_club_ranking(club.id, club_points);
            }

            // Determine continental competition qualifications
            Self::determine_competition_qualifications(&mut continent.continental_rankings);

            debug!(
                "Continental rankings updated - Top country: {:?}",
                continent.continental_rankings.get_top_country()
            );
        }
    }

    fn is_competition_draw_period(&self, date: NaiveDate) -> bool {
        // Champions League draw typically in August
        (date.month() == 8 && date.day() == 15) ||
            // Europa League draw
            (date.month() == 8 && date.day() == 20) ||
            // Knockout stage draws in December
            (date.month() == 12 && date.day() == 15)
    }

    fn conduct_competition_draws(&self, data: &mut SimulatorData, date: NaiveDate) {
        info!("🎲 Conducting continental competition draws");

        let continent_id = self.get_continent_id(data);

        if let Some(continent) = data.continent_mut(continent_id) {
            let qualified_clubs = continent.continental_rankings.get_qualified_clubs();

            // Champions League draw
            if let Some(cl_clubs) = qualified_clubs.get(&CompetitionTier::ChampionsLeague) {
                continent.continental_competitions.champions_league.conduct_draw(
                    cl_clubs,
                    &continent.continental_rankings,
                    date,
                );
            }

            // Europa League draw
            if let Some(el_clubs) = qualified_clubs.get(&CompetitionTier::EuropaLeague) {
                continent.continental_competitions.europa_league.conduct_draw(
                    el_clubs,
                    &continent.continental_rankings,
                    date,
                );
            }

            // Conference League draw
            if let Some(conf_clubs) = qualified_clubs.get(&CompetitionTier::ConferenceLeague) {
                continent.continental_competitions.conference_league.conduct_draw(
                    conf_clubs,
                    &continent.continental_rankings,
                    date,
                );
            }
        }
    }

    fn simulate_continental_competitions(
        &self,
        data: &mut SimulatorData,
        date: NaiveDate,
    ) -> Option<ContinentalCompetitionResults> {
        let continent_id = self.get_continent_id(data);

        let continent = data.continent_mut(continent_id)?;
        let mut results = ContinentalCompetitionResults::new();

        let clubs_map = Self::get_clubs_map(&continent.countries);

        // Simulate Champions League matches
        if continent.continental_competitions.champions_league.has_matches_today(date) {
            let cl_results = continent.continental_competitions.champions_league.simulate_round(
                &clubs_map,
                date,
            );
            results.champions_league_results = Some(cl_results);
        }

        // Simulate Europa League matches
        if continent.continental_competitions.europa_league.has_matches_today(date) {
            let el_results = continent.continental_competitions.europa_league.simulate_round(
                &clubs_map,
                date,
            );
            results.europa_league_results = Some(el_results);
        }

        // Simulate Conference League matches
        if continent.continental_competitions.conference_league.has_matches_today(date) {
            let conf_results = continent.continental_competitions.conference_league.simulate_round(
                &clubs_map,
                date,
            );
            results.conference_league_results = Some(conf_results);
        }

        Some(results)
    }

    fn process_competition_results(
        &self,
        comp_results: ContinentalCompetitionResults,
        data: &mut SimulatorData,
        result: &mut SimulationResult,
    ) {
        info!("🏆 Processing continental competition results");

        // Process Champions League results
        if let Some(cl_results) = comp_results.champions_league_results {
            for match_result in cl_results {
                self.process_single_match(match_result, data, result);
            }
        }

        // Process Europa League results
        if let Some(el_results) = comp_results.europa_league_results {
            for match_result in el_results {
                self.process_single_match(match_result, data, result);
            }
        }

        // Process Conference League results
        if let Some(conf_results) = comp_results.conference_league_results {
            for match_result in conf_results {
                self.process_single_match(match_result, data, result);
            }
        }

        // Distribute competition rewards after all matches processed
        self.distribute_competition_rewards(data);
    }

    fn process_single_match(
        &self,
        match_result: ContinentalMatchResult,
        data: &mut SimulatorData,
        result: &mut SimulationResult,
    ) {
        debug!(
            "Processing match: {} vs {} ({}-{})",
            match_result.home_team,
            match_result.away_team,
            match_result.home_score,
            match_result.away_score
        );

        // Update statistics for home team
        self.update_club_continental_stats(match_result.home_team, &match_result, true, data);

        // Update statistics for away team
        self.update_club_continental_stats(match_result.away_team, &match_result, false, data);

        // Store match in simulation result for output/history
        // result.continental_matches.push(match_result);
    }

    fn update_club_continental_stats(
        &self,
        club_id: u32,
        match_result: &ContinentalMatchResult,
        is_home: bool,
        data: &mut SimulatorData,
    ) {
        if let Some(club) = data.club_mut(club_id) {
            // Determine match outcome for this club
            let (goals_for, goals_against) = if is_home {
                (match_result.home_score, match_result.away_score)
            } else {
                (match_result.away_score, match_result.home_score)
            };

            let won = goals_for > goals_against;
            let drawn = goals_for == goals_against;
            let lost = goals_for < goals_against;

            // Update continental record (would need to add this to Club struct)
            // club.continental_record.matches_played += 1;
            // if won {
            //     club.continental_record.wins += 1;
            // } else if drawn {
            //     club.continental_record.draws += 1;
            // } else {
            //     club.continental_record.losses += 1;
            // }
            // club.continental_record.goals_for += goals_for as u32;
            // club.continental_record.goals_against += goals_against as u32;

            // Update finances with match revenue
            let match_revenue = self.calculate_match_revenue(match_result);
            club.finance.balance.push_income(match_revenue as i32);

            // Win bonus
            if won {
                let win_bonus = self.calculate_win_bonus(match_result);
                club.finance.balance.push_income(win_bonus as i32);
            }

            // Update club reputation based on result
            self.update_club_reputation(club, match_result, won, drawn);

            // Update player morale and form based on result
            self.update_players_after_match(club, won, drawn);

            debug!(
                "Club {} stats updated: revenue +€{:.0}",
                club_id,
                match_revenue
            );
        }
    }

    fn calculate_match_revenue(&self, match_result: &ContinentalMatchResult) -> f64 {
        // Base revenue by competition tier
        let base_revenue = match match_result.competition {
            CompetitionTier::ChampionsLeague => 3_000_000.0,   // €3M per match
            CompetitionTier::EuropaLeague => 1_000_000.0,      // €1M per match
            CompetitionTier::ConferenceLeague => 500_000.0,    // €500K per match
        };

        // Add ticket revenue (simplified - would depend on stadium capacity)
        let ticket_revenue = 200_000.0;

        base_revenue + ticket_revenue
    }

    fn calculate_win_bonus(&self, match_result: &ContinentalMatchResult) -> f64 {
        match match_result.competition {
            CompetitionTier::ChampionsLeague => 2_800_000.0,   // €2.8M win bonus
            CompetitionTier::EuropaLeague => 570_000.0,        // €570K win bonus
            CompetitionTier::ConferenceLeague => 500_000.0,    // €500K win bonus
        }
    }

    fn update_club_reputation(&self, club: &mut Club, match_result: &ContinentalMatchResult, won: bool, drawn: bool) {
        // Reputation changes based on continental performance
        let reputation_change = if won {
            match match_result.competition {
                CompetitionTier::ChampionsLeague => 5,
                CompetitionTier::EuropaLeague => 3,
                CompetitionTier::ConferenceLeague => 2,
            }
        } else if drawn {
            1
        } else {
            -2
        };

        // Apply reputation change (would need reputation field in Club)
        // club.reputation.continental = (club.reputation.continental as i32 + reputation_change)
        //     .clamp(0, 1000) as u16;

        debug!("Club {} reputation change: {:+}", club.id, reputation_change);
    }

    fn update_players_after_match(&self, club: &mut Club, won: bool, _drawn: bool) {
        // Update player morale and form after continental match
        let morale_change = if won { 5 } else { -3 };

        for team in &mut club.teams.teams {
            for player in &mut team.players.players {
                // Morale change (would need morale field in Player)
                // player.morale = (player.morale as i32 + morale_change).clamp(0, 100) as u8;

                // Form change based on performance (simplified)
                // if won {
                //     player.form = (player.form + 2).min(100);
                // }
            }
        }
    }

    fn distribute_competition_rewards(&self, data: &mut SimulatorData) {
        info!("💰 Distributing continental competition rewards");

        let continent_id = self.get_continent_id(data);

        // Collect participating clubs data first to avoid borrow conflicts
        let (cl_clubs, el_clubs, conf_clubs) = if let Some(continent) = data.continent(continent_id) {
            (
                continent.continental_competitions.champions_league.participating_clubs.clone(),
                continent.continental_competitions.europa_league.participating_clubs.clone(),
                continent.continental_competitions.conference_league.participating_clubs.clone(),
            )
        } else {
            return;
        };

        // Now we can mutably borrow data without conflicts
        // Distribute Champions League rewards
        self.distribute_competition_tier_rewards(
            &cl_clubs,
            CompetitionTier::ChampionsLeague,
            data,
        );

        // Distribute Europa League rewards
        self.distribute_competition_tier_rewards(
            &el_clubs,
            CompetitionTier::EuropaLeague,
            data,
        );

        // Distribute Conference League rewards
        self.distribute_competition_tier_rewards(
            &conf_clubs,
            CompetitionTier::ConferenceLeague,
            data,
        );
    }

    fn distribute_competition_tier_rewards(
        &self,
        participating_clubs: &[u32],
        tier: CompetitionTier,
        data: &mut SimulatorData,
    ) {
        // Participation bonus
        let participation_bonus = match tier {
            CompetitionTier::ChampionsLeague => 15_640_000.0,   // €15.64M base
            CompetitionTier::EuropaLeague => 3_630_000.0,       // €3.63M base
            CompetitionTier::ConferenceLeague => 2_940_000.0,   // €2.94M base
        };

        for &club_id in participating_clubs {
            if let Some(club) = data.club_mut(club_id) {
                club.finance.balance.push_income(participation_bonus as i32);

                debug!(
                    "Club {} received participation bonus: €{:.2}M",
                    club_id,
                    participation_bonus / 1_000_000.0
                );
            }
        }

        // Additional stage progression bonuses would be calculated here
        // based on how far each team progressed in the competition
    }

    fn update_economic_zone(&self, data: &mut SimulatorData, country_results: &[CountryResult]) {
        info!("💰 Updating continental economic zone");

        let continent_id = self.get_continent_id(data);

        if let Some(continent) = data.continent_mut(continent_id) {
            // Calculate overall economic health
            let mut total_revenue = 0.0;
            let mut total_expenses = 0.0;

            for country in &continent.countries {
                for club in &country.clubs {
                    total_revenue += club.finance.balance.income as f64;
                    total_expenses += club.finance.balance.outcome as f64;
                }
            }

            continent.economic_zone.update_indicators(total_revenue, total_expenses);

            // Update TV rights distribution
            continent.economic_zone.recalculate_tv_rights(&continent.continental_rankings);

            // Update sponsorship market
            continent.economic_zone.update_sponsorship_market(&continent.continental_rankings);
        }
    }

    fn update_continental_regulations(&self, data: &mut SimulatorData, date: NaiveDate) {
        info!("📋 Updating continental regulations");

        let continent_id = self.get_continent_id(data);

        if let Some(continent) = data.continent_mut(continent_id) {
            // Financial Fair Play adjustments
            continent.regulations.update_ffp_thresholds(&continent.economic_zone);

            // Foreign player regulations
            continent.regulations.review_foreign_player_rules(&continent.continental_rankings);

            // Youth development requirements
            continent.regulations.update_youth_requirements();

            debug!("Continental regulations updated for year {}", date.year());
        }
    }

    fn process_continental_awards(&self, data: &mut SimulatorData, country_results: &[CountryResult]) {
        info!("🏆 Processing continental awards");

        let continent_id = self.get_continent_id(data);

        if let Some(continent) = data.continent(continent_id) {
            // Player of the Year
            let _player_of_year = Self::determine_player_of_year(continent);

            // Team of the Year
            let _team_of_year = Self::determine_team_of_year(continent);

            // Coach of the Year
            let _coach_of_year = Self::determine_coach_of_year(continent);

            // Young Player Award
            let _young_player = Self::determine_young_player_award(continent);

            debug!("Continental awards distributed");
        }
    }

    // Helper methods

    fn get_continent_id(&self, data: &SimulatorData) -> u32 {
        // Assuming we can get continent ID from the first country
        // You might want to store this in ContinentResult
        if let Some(first_country) = self.countries.first() {
            // Get country from data and return its continent_id
            // This is a placeholder - adjust based on your actual data structure
            0 // Replace with actual logic
        } else {
            0
        }
    }

    fn calculate_country_coefficient(country: &Country, competitions: &ContinentalCompetitions) -> f32 {
        let mut coefficient = 0.0;

        for club in &country.clubs {
            coefficient += competitions.get_club_points(club.id);
        }

        if !country.clubs.is_empty() {
            coefficient /= country.clubs.len() as f32;
        }

        coefficient
    }

    fn calculate_club_continental_points(club: &Club, competitions: &ContinentalCompetitions) -> f32 {
        let competition_points = competitions.get_club_points(club.id);
        let domestic_bonus = 0.0; // Would need league standings
        competition_points + domestic_bonus
    }

    fn determine_competition_qualifications(rankings: &mut ContinentalRankings) {
        // Collect country rankings data first to avoid borrow conflicts
        let country_rankings: Vec<(u32, f32)> = rankings.get_country_rankings().to_vec();

        // Now we can mutably borrow rankings without conflicts
        for (rank, (country_id, _coefficient)) in country_rankings.iter().enumerate() {
            let cl_spots = match rank {
                0..=3 => 4,
                4..=5 => 3,
                6..=14 => 2,
                _ => 1,
            };

            let el_spots = match rank {
                0..=5 => 2,
                _ => 1,
            };

            rankings.set_qualification_spots(*country_id, cl_spots, el_spots);
        }
    }

    fn get_all_clubs(countries: &[Country]) -> Vec<&Club> {
        countries.iter().flat_map(|c| &c.clubs).collect()
    }

    fn get_clubs_map(countries: &[Country]) -> HashMap<u32, &Club> {
        countries
            .iter()
            .flat_map(|c| &c.clubs)
            .map(|club| (club.id, club))
            .collect()
    }

    fn determine_player_of_year(continent: &Continent) -> Option<u32> {
        None
    }

    fn determine_team_of_year(continent: &Continent) -> Option<Vec<u32>> {
        None
    }

    fn determine_coach_of_year(continent: &Continent) -> Option<u32> {
        None
    }

    fn determine_young_player_award(continent: &Continent) -> Option<u32> {
        None
    }
}

// Supporting structures for the result

#[derive(Debug, Clone)]
pub struct ContinentalRankingsUpdate {
    pub country_updates: Vec<(u32, f32)>, // country_id, new coefficient
    pub club_updates: Vec<(u32, f32)>,    // club_id, new points
    pub qualification_changes: Vec<QualificationChange>,
}

impl ContinentalRankingsUpdate {
    pub fn from_rankings(rankings: ContinentalRankings) -> Self {
        ContinentalRankingsUpdate {
            country_updates: rankings.country_rankings,
            club_updates: rankings.club_rankings,
            qualification_changes: Vec::new(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct QualificationChange {
    pub country_id: u32,
    pub competition: CompetitionTier,
    pub old_spots: u8,
    pub new_spots: u8,
}

#[derive(Debug, Clone)]
pub struct CrossBorderTransferSummary {
    pub completed_transfers: Vec<CompletedTransfer>,
    pub total_value: f64,
    pub most_expensive: Option<CompletedTransfer>,
    pub by_country_flow: HashMap<u32, TransferFlow>, // country_id -> flow stats
}

#[derive(Debug, Clone)]
pub struct TransferFlow {
    pub incoming_transfers: u32,
    pub outgoing_transfers: u32,
    pub net_spend: f64,
}

#[derive(Debug, Clone)]
pub struct EconomicZoneImpact {
    pub economic_multiplier: f32,
    pub tv_rights_change: f64,
    pub sponsorship_change: f64,
    pub overall_health_change: f32,
}

// Extension to SimulationResult to include continental matches
impl SimulationResult {
    // Note: This would need to be added to the actual SimulationResult struct
    // pub continental_matches: Vec<ContinentalMatchResult>,
}

#[derive(Debug)]
pub struct ContinentalCompetitionResults {
    pub champions_league_results: Option<Vec<ContinentalMatchResult>>,
    pub europa_league_results: Option<Vec<ContinentalMatchResult>>,
    pub conference_league_results: Option<Vec<ContinentalMatchResult>>,
}

impl ContinentalCompetitionResults {
    pub fn new() -> Self {
        ContinentalCompetitionResults {
            champions_league_results: None,
            europa_league_results: None,
            conference_league_results: None,
        }
    }
}
