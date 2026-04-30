use crate::SimulatorData;
use crate::competitions::global::GlobalCompetitionFixture;
use crate::continent::Continent;
use crate::continent::national::world as national_world;
use crate::r#match::MatchSquad;

pub struct GlobalCompetitionSimulator;

impl GlobalCompetitionSimulator {
    pub fn simulate(data: &mut SimulatorData) {
        let date = data.date.date();
        data.global_competitions
            .check_tournament_assembly(date, &data.continents);
        Self::simulate_matches(data, date);
        data.global_competitions.check_phase_transitions();
    }

    fn simulate_matches(data: &mut SimulatorData, date: chrono::NaiveDate) {
        let todays_matches = data.global_competitions.get_todays_matches(date);
        if todays_matches.is_empty() {
            return;
        }

        let prepared: Vec<(usize, MatchSquad, MatchSquad)> = todays_matches
            .iter()
            .enumerate()
            .filter_map(|(idx, fixture)| {
                let home = national_world::build_world_match_squad(
                    &mut data.continents,
                    fixture.home_country_id,
                    date,
                )?;
                let away = national_world::build_world_match_squad(
                    &mut data.continents,
                    fixture.away_country_id,
                    date,
                )?;
                Some((idx, home, away))
            })
            .collect();

        let engine_results = crate::match_engine_pool().play_squads(prepared);

        for (fixture_idx, raw_result) in engine_results {
            let fixture = &todays_matches[fixture_idx];
            let score = raw_result.score.as_ref().expect("match should have score");
            let home_score = score.home_team.get();
            let away_score = score.away_team.get();

            let penalty_winner =
                Self::penalty_winner(&data.continents, fixture, home_score, away_score);

            data.global_competitions
                .record_result(fixture, home_score, away_score, penalty_winner);

            let (label, full_name) = data
                .global_competitions
                .tournaments
                .get(fixture.tournament_idx)
                .map(|t| (t.short_name().to_string(), t.config.name.clone()))
                .unwrap_or_else(|| ("INT".to_string(), "International".to_string()));

            let match_result = national_world::apply_global_tournament_result(
                &mut data.continents,
                fixture,
                &raw_result,
                date,
                &label,
                &full_name,
            );

            data.match_store.push(match_result, date);
        }
    }

    /// Penalty shootout winner for knockout draws. Reputation nudges the
    /// baseline a little (capped at ±0.15 — i.e. 35/65 in the most
    /// lopsided matchup) and is then rolled — shootouts are coin-flippy
    /// in reality.
    fn penalty_winner(
        continents: &[Continent],
        fixture: &GlobalCompetitionFixture,
        home_score: u8,
        away_score: u8,
    ) -> Option<u32> {
        if !fixture.phase.is_knockout() || home_score != away_score {
            return None;
        }
        let home_rep =
            national_world::world_country_reputation(continents, fixture.home_country_id) as f32;
        let away_rep =
            national_world::world_country_reputation(continents, fixture.away_country_id) as f32;
        let total_rep = (home_rep + away_rep).max(1.0);
        let rep_share = home_rep / total_rep;
        let home_chance = (0.5 + (rep_share - 0.5) * 0.30).clamp(0.35, 0.65);
        let roll = crate::utils::FloatUtils::random(0.0, 1.0);
        Some(if roll < home_chance {
            fixture.home_country_id
        } else {
            fixture.away_country_id
        })
    }
}
