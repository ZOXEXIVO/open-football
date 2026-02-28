use crate::continent::continent::Continent;
use crate::r#match::MatchSquad;
use crate::NationalTeam;
use chrono::NaiveDate;
use log::info;
use std::collections::HashMap;

impl Continent {
    /// Simulate national team competitions: check cycles, play matches in parallel, progress phases
    pub(crate) fn simulate_national_competitions(&mut self, date: NaiveDate) {
        use rayon::iter::{IntoParallelIterator, ParallelIterator};

        let continent_id = self.id;

        // Check if we need to start new competition cycles
        let mut country_ids_by_rep: Vec<(u32, u16)> = self
            .countries
            .iter()
            .map(|c| (c.id, c.reputation))
            .collect();
        country_ids_by_rep.sort_by(|a, b| b.1.cmp(&a.1));
        let sorted_ids: Vec<u32> = country_ids_by_rep.iter().map(|(id, _)| *id).collect();

        self.national_team_competitions
            .check_new_cycles(date, &sorted_ids, continent_id);

        // Get today's matches from competitions
        let todays_matches = self.national_team_competitions.get_todays_matches(date);

        if todays_matches.is_empty() {
            return;
        }

        // Step 1: Build squads for all matches (sequential - needs &mut self)
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
            let penalty_winner = if fixture.phase.is_knockout() && home_score == away_score {
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
            self.update_player_international_stats(home_country_id, away_country_id, &player_goals);

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

            // Get label from competition config
            let label = self.national_team_competitions
                .competitions
                .get(fixture.competition_idx)
                .map(|c| c.short_name())
                .unwrap_or("INT");

            info!(
                "International competition ({}): {} vs {} - {}:{}",
                label,
                home_name,
                away_name,
                home_score,
                away_score
            );
        }

        // Check phase transitions after all matches
        self.national_team_competitions.check_phase_transitions(continent_id);
    }

    /// Build a MatchSquad for a country, ensuring national team has called up players
    pub(crate) fn build_country_match_squad(&mut self, country_id: u32, date: NaiveDate) -> Option<MatchSquad> {
        let country_ids: Vec<(u32, String)> = self.countries.iter().map(|c| (c.id, c.name.clone())).collect();

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
    fn update_player_international_stats(&mut self, home_country_id: u32, away_country_id: u32, player_goals: &HashMap<u32, u16>) {
        for country in &mut self.countries {
            if country.id != home_country_id && country.id != away_country_id {
                continue;
            }

            for club in &mut country.clubs {
                for team in &mut club.teams.teams {
                    for player in &mut team.players.players {
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

    pub(crate) fn get_country_reputation(&self, country_id: u32) -> u16 {
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
            .unwrap_or_default()
    }
}
