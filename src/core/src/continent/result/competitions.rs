use super::{ContinentResult, ContinentalCompetitionResults};
use crate::continent::{CompetitionTier, ContinentalMatchResult};
use crate::simulator::SimulatorData;
use crate::{Club, Country, SimulationResult};
use chrono::{Datelike, NaiveDate};
use log::{debug, info};
use std::collections::HashMap;

impl ContinentResult {
    pub(crate) fn is_competition_draw_period(&self, date: NaiveDate) -> bool {
        // Champions League draw typically in August
        (date.month() == 8 && date.day() == 15) ||
            // Europa League draw
            (date.month() == 8 && date.day() == 20) ||
            // Knockout stage draws in December
            (date.month() == 12 && date.day() == 15)
    }

    pub(crate) fn conduct_competition_draws(&self, data: &mut SimulatorData, date: NaiveDate) {
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

    pub(crate) fn simulate_continental_competitions(
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

    pub(crate) fn process_competition_results(
        &self,
        comp_results: ContinentalCompetitionResults,
        data: &mut SimulatorData,
        result: &mut SimulationResult,
    ) {
        debug!("🏆 Processing continental competition results");

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
        _result: &mut SimulationResult,
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
        // _result.continental_matches.push(match_result);
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
            let _lost = goals_for < goals_against;

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

        debug!("Club {} reputation change: {:+}", club.id, reputation_change);
    }

    fn update_players_after_match(&self, club: &mut Club, won: bool, _drawn: bool) {
        // Update player morale and form after continental match
        let _morale_change = if won { 5 } else { -3 };

        for team in &mut club.teams.teams {
            for _player in &mut team.players.players {
                // Morale change (would need morale field in Player)
            }
        }
    }

    fn distribute_competition_rewards(&self, data: &mut SimulatorData) {
        debug!("💰 Distributing continental competition rewards");

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
    }

    fn get_clubs_map(countries: &[Country]) -> HashMap<u32, &Club> {
        countries
            .iter()
            .flat_map(|c| &c.clubs)
            .map(|club| (club.id, club))
            .collect()
    }
}
