use crate::continent::continent::Continent;
use crate::r#match::{MatchResult, MatchResultRaw, Score};
use chrono::NaiveDate;
use log::info;

impl Continent {
    /// Simulate national team competitions: check cycles, play matches via engine pool, progress phases.
    /// Returns collected MatchResults for storage in the global match_store.
    pub(crate) fn simulate_national_competitions(&mut self, date: NaiveDate) -> Vec<MatchResult> {
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
            return Vec::new();
        }

        // Step 1: Build squads for all matches (sequential — needs &mut self)
        let prepared = self.prepare_match_squads(&todays_matches, date);

        // Step 2: Run all matches through the bounded engine thread pool
        let engine_results = crate::match_engine_pool().play_squads(prepared);

        // Step 3: Apply results sequentially, collect MatchResults
        let mut collected_results: Vec<MatchResult> = Vec::new();

        for (fixture_idx, match_result) in engine_results {
            let fixture = &todays_matches[fixture_idx];
            let home_country_id = fixture.home_country_id;
            let away_country_id = fixture.away_country_id;

            let score = match_result.score.as_ref().expect("match should have score");
            let home_score = score.home_team.get();
            let away_score = score.away_team.get();

            // Generate a unique match ID
            let match_id = format!("int-{}-{}-{}", date.format("%Y%m%d"), home_country_id, away_country_id);

            let penalty_winner = self.determine_penalty_winner(fixture, home_score, away_score);

            // Record result in the competition standings
            self.national_team_competitions.record_result(
                fixture,
                home_score,
                away_score,
                penalty_winner,
            );

            // Collect goals from match player stats
            let player_goals: std::collections::HashMap<u32, u16> = match_result
                .player_stats
                .iter()
                .filter(|(_, stats)| stats.goals > 0)
                .map(|(&id, stats)| (id, stats.goals))
                .collect();

            // Update player international stats, reputation, and Elo
            self.update_player_international_stats(home_country_id, away_country_id, &player_goals);
            self.update_elo_ratings(home_country_id, away_country_id, home_score, away_score);

            // Record fixtures in each country's national team schedule (for web display)
            let home_name = self.get_country_name(home_country_id);
            let away_name = self.get_country_name(away_country_id);

            let (label, comp_full_name) = self.national_team_competitions
                .competitions
                .get(fixture.competition_idx)
                .map(|c| (c.short_name().to_string(), c.config.name.clone()))
                .unwrap_or_else(|| ("INT".to_string(), "International".to_string()));

            self.record_country_schedule(date, home_country_id, away_country_id, &home_name, &away_name, home_score, away_score, &comp_full_name, &match_id);

            // Build MatchResult for global storage (used by match detail page)
            collected_results.push(MatchResult {
                id: match_id,
                league_id: 0,
                league_slug: "international".to_string(),
                home_team_id: home_country_id,
                away_team_id: away_country_id,
                score: score.clone(),
                details: Some(match_result),
                friendly: false,
            });

            info!(
                "International match ({}): {} {} - {} {}",
                label, home_name, home_score, away_score, away_name
            );
        }

        // Check phase transitions after all matches
        self.national_team_competitions.check_phase_transitions(continent_id);

        collected_results
    }
}
