use crate::context::GlobalContext;
use crate::continent::national_competitions::NationalTeamCompetitions;
use crate::continent::ContinentResult;
use crate::country::CountryResult;
use crate::r#match::MatchSquad;
use crate::utils::Logging;
use crate::{Club, Country, NationalTeam};
use chrono::NaiveDate;
use log::{debug, info};
use std::collections::HashMap;

pub struct Continent {
    pub id: u32,
    pub name: String,
    pub countries: Vec<Country>,

    pub continental_competitions: ContinentalCompetitions,
    pub continental_rankings: ContinentalRankings,
    pub regulations: ContinentalRegulations,
    pub economic_zone: EconomicZone,
    pub national_team_competitions: NationalTeamCompetitions,
}

impl Continent {
    pub fn new(id: u32, name: String, countries: Vec<Country>) -> Self {
        Continent {
            id,
            name,
            countries,
            continental_competitions: ContinentalCompetitions::new(),
            continental_rankings: ContinentalRankings::new(),
            regulations: ContinentalRegulations::new(),
            economic_zone: EconomicZone::new(),
            national_team_competitions: NationalTeamCompetitions::new(),
        }
    }

    pub fn simulate(&mut self, ctx: GlobalContext<'_>) -> ContinentResult {
        let continent_name = self.name.clone();
        let date = ctx.simulation.date.date();

        info!(
            "🌏 Simulating continent: {} ({} countries)",
            continent_name,
            self.countries.len()
        );

        // Phase 0: National team competition matches (parallel engine runs)
        self.simulate_national_competitions(date);

        // Phase 0.5: International friendly matches (parallel engine runs)
        self.simulate_international_friendlies(date);

        // Phase 1+: Simulate all child entities and accumulate results
        let country_results = self.simulate_countries(&ctx);

        info!("✅ Continent {} simulation complete", continent_name);

        ContinentResult::new(country_results)
    }

    /// Simulate national team competitions: check cycles, play matches in parallel, progress phases
    fn simulate_national_competitions(&mut self, date: NaiveDate) {
        use rayon::iter::{IntoParallelIterator, ParallelIterator};

        let is_europe = self.id == 1; // continent_id 1 = Europe

        // Check if we need to start new competition cycles
        let mut country_ids_by_rep: Vec<(u32, u16)> = self
            .countries
            .iter()
            .map(|c| (c.id, c.reputation))
            .collect();
        country_ids_by_rep.sort_by(|a, b| b.1.cmp(&a.1));
        let sorted_ids: Vec<u32> = country_ids_by_rep.iter().map(|(id, _)| *id).collect();

        self.national_team_competitions
            .check_new_cycles(date, &sorted_ids, is_europe);

        // Get today's matches from competitions
        let todays_matches = self.national_team_competitions.get_todays_matches(date);

        if todays_matches.is_empty() {
            return;
        }

        // Step 1: Build squads for all matches (sequential — needs &mut self)
        let prepared: Vec<(usize, MatchSquad, MatchSquad)> = todays_matches
            .iter()
            .enumerate()
            .filter_map(|(idx, fixture)| {
                let home = self.build_country_match_squad(fixture.home_country_id, date)?;
                let away = self.build_country_match_squad(fixture.away_country_id, date)?;
                Some((idx, home, away))
            })
            .collect();

        // Step 2: Run all match engines in parallel
        let engine_results: Vec<(usize, u8, u8, HashMap<u32, u16>)> = prepared
            .into_par_iter()
            .map(|(idx, home_squad, away_squad)| {
                let (home_score, away_score, player_goals) =
                    NationalTeam::play_competition_match(home_squad, away_squad);
                (idx, home_score, away_score, player_goals)
            })
            .collect();

        // Step 3: Apply results sequentially
        for (fixture_idx, home_score, away_score, player_goals) in engine_results {
            let fixture = &todays_matches[fixture_idx];
            let home_country_id = fixture.home_country_id;
            let away_country_id = fixture.away_country_id;

            // Determine penalty winner for knockout matches (if draw)
            let penalty_winner = if fixture.competition.is_knockout() && home_score == away_score {
                let home_rep = self.get_country_reputation(home_country_id);
                let away_rep = self.get_country_reputation(away_country_id);
                if home_rep >= away_rep {
                    Some(home_country_id)
                } else {
                    Some(away_country_id)
                }
            } else {
                None
            };

            // Record result in the competition
            self.national_team_competitions.record_result(
                fixture,
                home_score,
                away_score,
                penalty_winner,
            );

            // Update player stats in clubs
            self.update_player_international_stats(&player_goals);

            // Update Elo ratings for both national teams
            let away_elo = self.get_country_elo(away_country_id);
            let home_elo = self.get_country_elo(home_country_id);

            if let Some(home_country) = self.countries.iter_mut().find(|c| c.id == home_country_id) {
                home_country.national_team.update_elo(home_score, away_score, away_elo);
            }
            if let Some(away_country) = self.countries.iter_mut().find(|c| c.id == away_country_id) {
                away_country.national_team.update_elo(away_score, home_score, home_elo);
            }

            let home_name = self.get_country_name(home_country_id);
            let away_name = self.get_country_name(away_country_id);

            info!(
                "International competition ({}): {} vs {} - {}:{}",
                fixture.competition.label(),
                home_name,
                away_name,
                home_score,
                away_score
            );
        }

        // Check phase transitions after all matches
        self.national_team_competitions.check_phase_transitions();
    }

    /// Play all international friendly matches scheduled for today in parallel.
    fn simulate_international_friendlies(&mut self, date: NaiveDate) {
        use crate::r#match::engine::engine::FootballEngine;
        use rayon::iter::{IntoParallelIterator, ParallelIterator};

        // Step 1: Prepare squads for all countries with a pending friendly (sequential)
        let mut prepared: Vec<(usize, usize, MatchSquad, MatchSquad)> = Vec::new();

        for (country_idx, country) in self.countries.iter().enumerate() {
            if let Some(fixture_idx) = country.national_team.pending_friendly(date) {
                let fixture = &country.national_team.schedule[fixture_idx];
                let our_squad = country.national_team.build_match_squad(&country.clubs);
                let opponent_squad =
                    NationalTeam::build_synthetic_opponent_squad(fixture.opponent_country_id);

                let (home_squad, away_squad) = if fixture.is_home {
                    (our_squad, opponent_squad)
                } else {
                    (opponent_squad, our_squad)
                };

                prepared.push((country_idx, fixture_idx, home_squad, away_squad));
            }
        }

        if prepared.is_empty() {
            return;
        }

        // Step 2: Run all match engines in parallel
        let engine_results: Vec<(usize, usize, crate::r#match::MatchResultRaw)> = prepared
            .into_par_iter()
            .map(|(country_idx, fixture_idx, home_squad, away_squad)| {
                let result = FootballEngine::<840, 545>::play(home_squad, away_squad, crate::is_match_recordings_mode());
                (country_idx, fixture_idx, result)
            })
            .collect();

        // Step 3: Apply results sequentially
        for (country_idx, fixture_idx, match_result) in &engine_results {
            let country = &mut self.countries[*country_idx];
            country.national_team.apply_friendly_result(
                &mut country.clubs,
                *fixture_idx,
                match_result,
                date,
            );
        }
    }

    /// Build a MatchSquad for a country, ensuring national team has called up players
    fn build_country_match_squad(&mut self, country_id: u32, date: NaiveDate) -> Option<MatchSquad> {
        let country_ids: Vec<u32> = self.countries.iter().map(|c| c.id).collect();

        // Find the country and ensure it has a squad
        let country_idx = self.countries.iter().position(|c| c.id == country_id)?;

        // Ensure the national team has a squad called up
        if self.countries[country_idx].national_team.squad.is_empty()
            && self.countries[country_idx].national_team.generated_squad.is_empty()
        {
            // Collect candidates from ALL clubs across ALL countries
            let mut all_candidates = NationalTeam::collect_all_candidates_by_country(&self.countries, date);
            let candidates = all_candidates.remove(&country_id).unwrap_or_default();

            let country = &mut self.countries[country_idx];
            country.national_team.country_name = country.name.clone();
            country.national_team.reputation = country.reputation;

            let cid = country.id;
            country.national_team.call_up_squad(&mut country.clubs, candidates, date, cid, &country_ids);
        }

        // Build the match squad
        let country = &self.countries[country_idx];
        let squad = country.national_team.build_match_squad(&country.clubs);
        Some(squad)
    }

    /// Update player international stats after a competition match
    fn update_player_international_stats(&mut self, player_goals: &HashMap<u32, u16>) {
        for country in &mut self.countries {
            for club in &mut country.clubs {
                for team in &mut club.teams.teams {
                    for player in &mut team.players.players {
                        // Check if player was in any national squad
                        if country.national_team.squad.iter().any(|s| s.player_id == player.id) {
                            player.player_attributes.international_apps += 1;

                            if let Some(&goals) = player_goals.get(&player.id) {
                                player.player_attributes.international_goals += goals;
                            }
                        }
                    }
                }
            }
        }
    }

    fn get_country_reputation(&self, country_id: u32) -> u16 {
        self.countries
            .iter()
            .find(|c| c.id == country_id)
            .map(|c| c.reputation)
            .unwrap_or(0)
    }

    fn get_country_elo(&self, country_id: u32) -> u16 {
        self.countries
            .iter()
            .find(|c| c.id == country_id)
            .map(|c| c.national_team.elo_rating)
            .unwrap_or(1500)
    }

    fn get_country_name(&self, country_id: u32) -> String {
        self.countries
            .iter()
            .find(|c| c.id == country_id)
            .map(|c| c.name.clone())
            .unwrap_or_else(|| format!("Country {}", country_id))
    }

    fn simulate_countries(&mut self, ctx: &GlobalContext<'_>) -> Vec<CountryResult> {
        let country_ids: Vec<u32> = self.countries.iter().map(|c| c.id).collect();
        let date = ctx.simulation.date.date();

        // Pre-collect national team candidates from ALL clubs across ALL countries
        let need_callups = NationalTeam::is_break_start(date) || NationalTeam::is_tournament_start(date);

        let mut candidates_by_country = if need_callups {
            NationalTeam::collect_all_candidates_by_country(&self.countries, date)
        } else {
            HashMap::new()
        };

        self.countries
            .iter_mut()
            .map(|country| {
                let candidates = candidates_by_country.remove(&country.id);
                let message = &format!("simulate country: {} (Continental)", &country.name);
                Logging::estimate_result(
                    || country.simulate(ctx.with_country(country.id), &country_ids, candidates),
                    message,
                )
            })
            .collect()
    }
}

// Supporting structures for continental simulation

#[derive(Debug, Clone)]
pub struct ContinentalCompetitions {
    pub champions_league: ChampionsLeague,
    pub europa_league: EuropaLeague,
    pub conference_league: ConferenceLeague,
    pub super_cup: SuperCup,
}

impl Default for ContinentalCompetitions {
    fn default() -> Self {
        Self::new()
    }
}

impl ContinentalCompetitions {
    pub fn new() -> Self {
        ContinentalCompetitions {
            champions_league: ChampionsLeague::new(),
            europa_league: EuropaLeague::new(),
            conference_league: ConferenceLeague::new(),
            super_cup: SuperCup::new(),
        }
    }

    pub fn get_club_points(&self, club_id: u32) -> f32 {
        let mut points = 0.0;

        points += self.champions_league.get_club_points(club_id);
        points += self.europa_league.get_club_points(club_id);
        points += self.conference_league.get_club_points(club_id);

        points
    }

    pub fn get_total_prize_pool(&self) -> f64 {
        self.champions_league.prize_pool
            + self.europa_league.prize_pool
            + self.conference_league.prize_pool
            + self.super_cup.prize_pool
    }
}

#[derive(Debug, Clone)]
pub struct ChampionsLeague {
    pub participating_clubs: Vec<u32>,
    pub current_stage: CompetitionStage,
    pub matches: Vec<ContinentalMatch>,
    pub prize_pool: f64,
}

impl Default for ChampionsLeague {
    fn default() -> Self {
        Self::new()
    }
}

impl ChampionsLeague {
    pub fn new() -> Self {
        ChampionsLeague {
            participating_clubs: Vec::new(),
            current_stage: CompetitionStage::NotStarted,
            matches: Vec::new(),
            prize_pool: 2_000_000_000.0, // 2 billion euros
        }
    }

    pub fn conduct_draw(&mut self, clubs: &[u32], _rankings: &ContinentalRankings, _date: NaiveDate) {
        // Implement draw logic with seeding based on rankings
        debug!("Champions League draw conducted with {} clubs", clubs.len());
    }

    pub fn has_matches_today(&self, date: NaiveDate) -> bool {
        self.matches.iter().any(|m| m.date == date)
    }

    pub fn simulate_round(
        &mut self,
        _clubs: &HashMap<u32, &Club>,
        date: NaiveDate,
    ) -> Vec<ContinentalMatchResult> {
        let mut results = Vec::new();

        for match_to_play in self.matches.iter_mut().filter(|m| m.date == date) {
            // Simulate match (simplified)
            let result = ContinentalMatchResult {
                home_team: match_to_play.home_team,
                away_team: match_to_play.away_team,
                home_score: 0,
                away_score: 0,
                competition: CompetitionTier::ChampionsLeague,
            };

            results.push(result);
        }

        results
    }

    pub fn get_club_points(&self, club_id: u32) -> f32 {
        // Points based on performance
        if !self.participating_clubs.contains(&club_id) {
            return 0.0;
        }

        // Simplified: base points for participation
        10.0
    }
}

#[derive(Debug, Clone)]
pub struct EuropaLeague {
    pub participating_clubs: Vec<u32>,
    pub current_stage: CompetitionStage,
    pub matches: Vec<ContinentalMatch>,
    pub prize_pool: f64,
}

impl EuropaLeague {
    pub fn new() -> Self {
        EuropaLeague {
            participating_clubs: Vec::new(),
            current_stage: CompetitionStage::NotStarted,
            matches: Vec::new(),
            prize_pool: 500_000_000.0, // 500 million euros
        }
    }

    pub fn conduct_draw(&mut self, clubs: &[u32], _rankings: &ContinentalRankings, _date: NaiveDate) {
        debug!("Europa League draw conducted with {} clubs", clubs.len());
    }

    pub fn has_matches_today(&self, date: NaiveDate) -> bool {
        self.matches.iter().any(|m| m.date == date)
    }

    pub fn simulate_round(
        &mut self,
        _clubs: &HashMap<u32, &Club>,
        _date: NaiveDate,
    ) -> Vec<ContinentalMatchResult> {
        Vec::new() // Simplified
    }

    pub fn get_club_points(&self, club_id: u32) -> f32 {
        if !self.participating_clubs.contains(&club_id) {
            return 0.0;
        }
        5.0
    }
}

#[derive(Debug, Clone)]
pub struct ConferenceLeague {
    pub participating_clubs: Vec<u32>,
    pub current_stage: CompetitionStage,
    pub matches: Vec<ContinentalMatch>,
    pub prize_pool: f64,
}

impl ConferenceLeague {
    pub fn new() -> Self {
        ConferenceLeague {
            participating_clubs: Vec::new(),
            current_stage: CompetitionStage::NotStarted,
            matches: Vec::new(),
            prize_pool: 250_000_000.0, // 250 million euros
        }
    }

    pub fn conduct_draw(&mut self, clubs: &[u32], _rankings: &ContinentalRankings, _date: NaiveDate) {
        debug!(
            "Conference League draw conducted with {} clubs",
            clubs.len()
        );
    }

    pub fn has_matches_today(&self, _date: NaiveDate) -> bool {
        false // Simplified
    }

    pub fn simulate_round(
        &mut self,
        _clubs: &HashMap<u32, &Club>,
        _date: NaiveDate,
    ) -> Vec<ContinentalMatchResult> {
        Vec::new() // Simplified
    }

    pub fn get_club_points(&self, club_id: u32) -> f32 {
        if !self.participating_clubs.contains(&club_id) {
            return 0.0;
        }
        3.0
    }
}

#[derive(Debug, Clone)]
pub struct ContinentalMatchResult {
    pub home_team: u32,               // ID of the home team
    pub away_team: u32,               // ID of the away team
    pub home_score: u8,               // Goals scored by home team
    pub away_score: u8,               // Goals scored by away team
    pub competition: CompetitionTier, // Which competition (CL/EL/Conference)
}

#[derive(Debug, Clone)]
pub struct SuperCup {
    pub prize_pool: f64,
}

impl SuperCup {
    pub fn new() -> Self {
        SuperCup {
            prize_pool: 10_000_000.0,
        }
    }
}

#[derive(Debug, Clone)]
pub enum CompetitionStage {
    NotStarted,
    Qualifying,
    GroupStage,
    RoundOf32,
    RoundOf16,
    QuarterFinals,
    SemiFinals,
    Final,
}

#[derive(Debug, Clone)]
pub struct ContinentalMatch {
    pub home_team: u32,
    pub away_team: u32,
    pub date: NaiveDate,
    pub stage: CompetitionStage,
}

#[derive(Debug, Clone)]
pub struct ContinentalRankings {
    pub country_rankings: Vec<(u32, f32)>, // country_id, coefficient
    pub club_rankings: Vec<(u32, f32)>,    // club_id, points
    pub qualification_spots: HashMap<u32, QualificationSpots>,
}

impl ContinentalRankings {
    pub fn new() -> Self {
        ContinentalRankings {
            country_rankings: Vec::new(),
            club_rankings: Vec::new(),
            qualification_spots: HashMap::new(),
        }
    }

    pub fn update_country_ranking(&mut self, country_id: u32, coefficient: f32) {
        if let Some(entry) = self
            .country_rankings
            .iter_mut()
            .find(|(id, _)| *id == country_id)
        {
            entry.1 = coefficient;
        } else {
            self.country_rankings.push((country_id, coefficient));
        }

        // Sort by coefficient descending
        self.country_rankings
            .sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap());
    }

    pub fn update_club_ranking(&mut self, club_id: u32, points: f32) {
        if let Some(entry) = self.club_rankings.iter_mut().find(|(id, _)| *id == club_id) {
            entry.1 = points;
        } else {
            self.club_rankings.push((club_id, points));
        }

        self.club_rankings
            .sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap());
    }

    pub fn get_top_country(&self) -> Option<u32> {
        self.country_rankings.first().map(|(id, _)| *id)
    }

    pub fn get_country_rankings(&self) -> &[(u32, f32)] {
        &self.country_rankings
    }

    pub fn get_qualified_clubs(&self) -> HashMap<CompetitionTier, Vec<u32>> {
        // Logic to determine which clubs qualify for each competition
        HashMap::new() // Simplified
    }

    pub fn set_qualification_spots(&mut self, country_id: u32, cl_spots: u8, el_spots: u8) {
        self.qualification_spots.insert(
            country_id,
            QualificationSpots {
                champions_league: cl_spots,
                europa_league: el_spots,
                conference_league: 1, // Default
            },
        );
    }
}

#[derive(Debug, Clone)]
pub struct QualificationSpots {
    pub champions_league: u8,
    pub europa_league: u8,
    pub conference_league: u8,
}

#[derive(Debug, Clone)]
pub struct ContinentalRegulations {
    pub ffp_rules: FinancialFairPlayRules,
    pub foreign_player_limits: ForeignPlayerLimits,
    pub youth_requirements: YouthRequirements,
}

impl ContinentalRegulations {
    pub fn new() -> Self {
        ContinentalRegulations {
            ffp_rules: FinancialFairPlayRules::new(),
            foreign_player_limits: ForeignPlayerLimits::new(),
            youth_requirements: YouthRequirements::new(),
        }
    }

    pub fn update_ffp_thresholds(&mut self, economic_zone: &EconomicZone) {
        // Adjust FFP based on economic conditions
        self.ffp_rules
            .update_thresholds(economic_zone.get_overall_health());
    }

    pub fn review_foreign_player_rules(&mut self, _rankings: &ContinentalRankings) {
        // Potentially adjust foreign player rules
    }

    pub fn update_youth_requirements(&mut self) {
        // Update youth development requirements
    }
}

#[derive(Debug, Clone)]
pub struct FinancialFairPlayRules {
    pub max_deficit: f64,
    pub monitoring_period_years: u8,
    pub squad_cost_ratio_limit: f32,
}

impl FinancialFairPlayRules {
    pub fn new() -> Self {
        FinancialFairPlayRules {
            max_deficit: 30_000_000.0,
            monitoring_period_years: 3,
            squad_cost_ratio_limit: 0.7,
        }
    }

    pub fn update_thresholds(&mut self, economic_health: f32) {
        // Adjust based on economic conditions
        if economic_health < 0.5 {
            self.max_deficit *= 0.8;
        } else if economic_health > 0.8 {
            self.max_deficit *= 1.1;
        }
    }
}

#[derive(Debug, Clone)]
pub struct ForeignPlayerLimits {
    pub max_non_eu_players: Option<u8>,
    pub homegrown_minimum: u8,
}

impl ForeignPlayerLimits {
    pub fn new() -> Self {
        ForeignPlayerLimits {
            max_non_eu_players: Some(3),
            homegrown_minimum: 8,
        }
    }
}

#[derive(Debug, Clone)]
pub struct YouthRequirements {
    pub minimum_academy_investment: f64,
    pub minimum_youth_squad_size: u8,
}

impl YouthRequirements {
    pub fn new() -> Self {
        YouthRequirements {
            minimum_academy_investment: 1_000_000.0,
            minimum_youth_squad_size: 20,
        }
    }
}

#[derive(Debug, Clone)]
pub struct EconomicZone {
    pub tv_rights_pool: f64,
    pub sponsorship_value: f64,
    pub economic_health_indicator: f32,
}

impl EconomicZone {
    pub fn new() -> Self {
        EconomicZone {
            tv_rights_pool: 5_000_000_000.0,
            sponsorship_value: 2_000_000_000.0,
            economic_health_indicator: 0.7,
        }
    }

    pub fn get_overall_health(&self) -> f32 {
        self.economic_health_indicator
    }

    pub fn update_indicators(&mut self, total_revenue: f64, total_expenses: f64) {
        let profit_margin = (total_revenue - total_expenses) / total_revenue;

        // Update health indicator based on profit margin
        self.economic_health_indicator =
            (self.economic_health_indicator * 0.8 + profit_margin as f32 * 0.2).clamp(0.0, 1.0);
    }

    pub fn recalculate_tv_rights(&mut self, rankings: &ContinentalRankings) {
        // Adjust TV rights based on competitive balance
        let competitive_balance = self.calculate_competitive_balance(rankings);
        self.tv_rights_pool *= 1.0 + competitive_balance as f64 * 0.1;
    }

    pub fn update_sponsorship_market(&mut self, _rankings: &ContinentalRankings) {
        // Update based on top clubs' performance
        self.sponsorship_value *= 1.02; // Simplified growth
    }

    fn calculate_competitive_balance(&self, _rankings: &ContinentalRankings) -> f32 {
        // Measure how competitive the continent is
        0.5 // Simplified
    }
}

#[derive(Debug, Clone)]
pub struct TransferInterest {
    pub player_id: u32,
    pub source_country: u32,
    pub interest_level: f32,
}

#[derive(Debug, Clone)]
pub struct TransferNegotiation {
    pub player_id: u32,
    pub selling_club: u32,
    pub buying_club: u32,
    pub current_offer: f64,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum CompetitionTier {
    ChampionsLeague,
    EuropaLeague,
    ConferenceLeague,
}
