use crate::continent::Continent;
use crate::SimulatorData;
use log::info;

pub struct GlobalCompetitionSimulator;

impl GlobalCompetitionSimulator {
    pub fn simulate(data: &mut SimulatorData) {
        let date = data.date.date();
        data.global_competitions.check_tournament_assembly(date, &data.continents);
        Self::simulate_matches(data, date);
        data.global_competitions.check_phase_transitions();
    }

    fn simulate_matches(data: &mut SimulatorData, date: chrono::NaiveDate) {
        let todays_matches = data.global_competitions.get_todays_matches(date);
        if todays_matches.is_empty() {
            return;
        }

        // Build squads - need to search across all continents
        let prepared: Vec<(usize, crate::r#match::MatchSquad, crate::r#match::MatchSquad)> = todays_matches
            .iter()
            .enumerate()
            .filter_map(|(idx, fixture)| {
                let home = Self::build_match_squad(&mut data.continents, fixture.home_country_id, date)?;
                let away = Self::build_match_squad(&mut data.continents, fixture.away_country_id, date)?;
                Some((idx, home, away))
            })
            .collect();

        // Run matches through the bounded engine thread pool
        let engine_results = crate::match_engine_pool().play_squads(prepared);

        // Apply results
        for (fixture_idx, match_result) in engine_results {
            let score = match_result.score.as_ref().expect("match should have score");
            let home_score = score.home_team.get();
            let away_score = score.away_team.get();
            let fixture = &todays_matches[fixture_idx];
            let home_country_id = fixture.home_country_id;
            let away_country_id = fixture.away_country_id;

            let penalty_winner = if fixture.phase.is_knockout() && home_score == away_score {
                // Weighted random: reputation nudges the baseline a little, but
                // shootouts are coin-flippy in reality. Rep bias capped at ±0.15
                // (so 35/65 in the most lopsided matchup) and then rolled.
                let home_rep = Self::country_reputation(&data.continents, home_country_id) as f32;
                let away_rep = Self::country_reputation(&data.continents, away_country_id) as f32;
                let total_rep = (home_rep + away_rep).max(1.0);
                let rep_share = home_rep / total_rep; // 0..1
                let home_chance = (0.5 + (rep_share - 0.5) * 0.30).clamp(0.35, 0.65);
                let roll = crate::utils::FloatUtils::random(0.0, 1.0);
                if roll < home_chance {
                    Some(home_country_id)
                } else {
                    Some(away_country_id)
                }
            } else {
                None
            };

            data.global_competitions.record_result(
                fixture,
                home_score,
                away_score,
                penalty_winner,
            );

            // Update Elo ratings across continents
            let away_elo = Self::country_elo(&data.continents, away_country_id);
            let home_elo = Self::country_elo(&data.continents, home_country_id);

            for continent in &mut data.continents {
                if let Some(country) = continent.countries.iter_mut().find(|c| c.id == home_country_id) {
                    country.national_team.update_elo(home_score, away_score, away_elo);
                }
                if let Some(country) = continent.countries.iter_mut().find(|c| c.id == away_country_id) {
                    country.national_team.update_elo(away_score, home_score, home_elo);
                }
            }

            let label = data.global_competitions
                .tournaments
                .get(fixture.tournament_idx)
                .map(|t| t.short_name())
                .unwrap_or("INT");

            let home_name = Self::country_name(&data.continents, home_country_id);
            let away_name = Self::country_name(&data.continents, away_country_id);

            info!(
                "Global competition ({}): {} vs {} - {}:{}",
                label, home_name, away_name, home_score, away_score
            );
        }
    }

    fn build_match_squad(
        continents: &mut [Continent],
        country_id: u32,
        date: chrono::NaiveDate,
    ) -> Option<crate::r#match::MatchSquad> {
        for continent in continents.iter_mut() {
            if continent.countries.iter().any(|c| c.id == country_id) {
                return continent.build_country_match_squad(country_id, date);
            }
        }
        None
    }

    fn country_reputation(continents: &[Continent], country_id: u32) -> u16 {
        continents
            .iter()
            .flat_map(|c| &c.countries)
            .find(|c| c.id == country_id)
            .map(|c| c.reputation)
            .unwrap_or(0)
    }

    fn country_elo(continents: &[Continent], country_id: u32) -> u16 {
        continents
            .iter()
            .flat_map(|c| &c.countries)
            .find(|c| c.id == country_id)
            .map(|c| c.national_team.elo_rating)
            .unwrap_or(1500)
    }

    fn country_name(continents: &[Continent], country_id: u32) -> String {
        continents
            .iter()
            .flat_map(|c| &c.countries)
            .find(|c| c.id == country_id)
            .map(|c| c.name.clone())
            .unwrap_or_default()
    }
}
