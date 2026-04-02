use crate::continent::continent::Continent;
use crate::country::national_team::{NationalTeamFixture, NationalTeamMatchResult};
use crate::NationalCompetitionFixture;
use chrono::NaiveDate;
use std::collections::HashMap;

impl Continent {
    /// Update player international stats and reputation after a competition match.
    ///
    /// Playing for the national team is a major reputation event:
    /// - Every squad member gets a base reputation boost (just being called up matters)
    /// - Goal scorers get significant additional bonus
    /// - World reputation is especially boosted (international stage = global visibility)
    pub(crate) fn update_player_international_stats(
        &mut self,
        home_country_id: u32,
        away_country_id: u32,
        player_goals: &HashMap<u32, u16>,
    ) {
        // Collect squad player IDs and country weights for both teams
        let mut squad_player_ids: Vec<u32> = Vec::new();
        let mut country_weights: HashMap<u32, f32> = HashMap::new();

        for country in &self.countries {
            if country.id != home_country_id && country.id != away_country_id {
                continue;
            }
            let country_rep = country.reputation as f32;
            let country_weight = (country_rep / 500.0).clamp(0.5, 2.0);
            for s in &country.national_team.squad {
                squad_player_ids.push(s.player_id);
                country_weights.insert(s.player_id, country_weight);
            }
        }

        // Search ALL countries' clubs — squad members may play abroad
        // (e.g. a Kazakh player at Eintracht Frankfurt)
        for country in &mut self.countries {
            for club in &mut country.clubs {
                for team in &mut club.teams.teams {
                    for player in &mut team.players.players {
                        if !squad_player_ids.contains(&player.id) {
                            continue;
                        }
                        let country_weight = country_weights.get(&player.id).copied().unwrap_or(1.0);

                        player.player_attributes.international_apps += 1;

                        let mut goal_bonus: f32 = 0.0;

                        if let Some(&goals) = player_goals.get(&player.id) {
                            player.player_attributes.international_goals += goals;
                            goal_bonus = goals.min(3) as f32 * 20.0;
                        }

                        let base = 15.0;
                        let raw = base + goal_bonus;

                        let current_delta = (raw * 0.6 * country_weight) as i16;
                        let home_delta = (raw * 0.8 * country_weight) as i16;
                        let world_delta = (raw * 1.0 * country_weight) as i16;

                        player.player_attributes.update_reputation(
                            current_delta,
                            home_delta,
                            world_delta,
                        );
                    }
                }
            }
        }
    }

    /// Update Elo ratings for both countries after a match
    pub(crate) fn update_elo_ratings(
        &mut self,
        home_country_id: u32,
        away_country_id: u32,
        home_score: u8,
        away_score: u8,
    ) {
        let away_elo = self.get_country_elo(away_country_id);
        let home_elo = self.get_country_elo(home_country_id);

        if let Some(home_country) = self.countries.iter_mut().find(|c| c.id == home_country_id) {
            home_country.national_team.update_elo(home_score, away_score, away_elo);
        }
        if let Some(away_country) = self.countries.iter_mut().find(|c| c.id == away_country_id) {
            away_country.national_team.update_elo(away_score, home_score, home_elo);
        }
    }

    /// Record match fixtures in each country's schedule for web display
    pub(crate) fn record_country_schedule(
        &mut self,
        date: NaiveDate,
        home_country_id: u32,
        away_country_id: u32,
        home_name: &str,
        away_name: &str,
        home_score: u8,
        away_score: u8,
        competition_name: &str,
        match_id: &str,
    ) {
        if let Some(home_country) = self.countries.iter_mut().find(|c| c.id == home_country_id) {
            home_country.national_team.schedule.push(NationalTeamFixture {
                date,
                opponent_country_id: away_country_id,
                opponent_country_name: away_name.to_string(),
                is_home: true,
                competition_name: competition_name.to_string(),
                match_id: match_id.to_string(),
                result: Some(NationalTeamMatchResult {
                    home_score,
                    away_score,
                    date,
                    opponent_country_id: away_country_id,
                }),
            });
        }
        if let Some(away_country) = self.countries.iter_mut().find(|c| c.id == away_country_id) {
            away_country.national_team.schedule.push(NationalTeamFixture {
                date,
                opponent_country_id: home_country_id,
                opponent_country_name: home_name.to_string(),
                is_home: false,
                competition_name: competition_name.to_string(),
                match_id: match_id.to_string(),
                result: Some(NationalTeamMatchResult {
                    home_score: away_score,
                    away_score: home_score,
                    date,
                    opponent_country_id: home_country_id,
                }),
            });
        }
    }

    /// Determine penalty winner for knockout matches
    pub(crate) fn determine_penalty_winner(
        &self,
        fixture: &NationalCompetitionFixture,
        home_score: u8,
        away_score: u8,
    ) -> Option<u32> {
        if fixture.phase.is_knockout() && home_score == away_score {
            let home_rep = self.get_country_reputation(fixture.home_country_id);
            let away_rep = self.get_country_reputation(fixture.away_country_id);
            if home_rep >= away_rep {
                Some(fixture.home_country_id)
            } else {
                Some(fixture.away_country_id)
            }
        } else {
            None
        }
    }

    /// Country lookup helpers
    pub(crate) fn get_country_reputation(&self, country_id: u32) -> u16 {
        self.countries
            .iter()
            .find(|c| c.id == country_id)
            .map(|c| c.reputation)
            .unwrap_or(0)
    }

    pub(crate) fn get_country_elo(&self, country_id: u32) -> u16 {
        self.countries
            .iter()
            .find(|c| c.id == country_id)
            .map(|c| c.national_team.elo_rating)
            .unwrap_or(1500)
    }

    pub(crate) fn get_country_name(&self, country_id: u32) -> String {
        self.countries
            .iter()
            .find(|c| c.id == country_id)
            .map(|c| c.name.clone())
            .unwrap_or_default()
    }
}
