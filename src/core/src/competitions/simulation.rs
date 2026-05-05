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

        let prepared: Vec<(usize, MatchSquad, MatchSquad, bool)> = todays_matches
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
                Some((idx, home, away, fixture.phase.is_knockout()))
            })
            .collect();

        let engine_results = crate::match_engine_pool().play_squads_with_knockout(prepared);

        for (fixture_idx, raw_result) in engine_results {
            let fixture = &todays_matches[fixture_idx];
            let score = raw_result.score.as_ref().expect("match should have score");
            let home_score = score.home_team.get();
            let away_score = score.away_team.get();

            // Knockout draws use the engine's penalty shootout result.
            // Falls back to a reputation-weighted coin flip only if a
            // knockout fixture somehow finishes level without a
            // shootout being played (engine bug / data anomaly).
            let penalty_winner = Self::resolve_knockout_winner(
                &data.continents,
                fixture,
                score,
                home_score,
                away_score,
            );

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

    /// Resolve who advances when a knockout fixture finishes level in
    /// regulation. Prefers the engine's penalty shootout result; falls
    /// back to a reputation-weighted draw only when the engine didn't
    /// run a shootout (e.g. group-stage games that allow draws should
    /// never enter this branch — `phase.is_knockout()` filters them).
    fn resolve_knockout_winner(
        continents: &[Continent],
        fixture: &GlobalCompetitionFixture,
        score: &crate::r#match::Score,
        home_score: u8,
        away_score: u8,
    ) -> Option<u32> {
        if !fixture.phase.is_knockout() || home_score != away_score {
            return None;
        }
        // Engine-played shootout — read the winner directly.
        if score.had_shootout() {
            return Some(if score.home_shootout > score.away_shootout {
                fixture.home_country_id
            } else if score.away_shootout > score.home_shootout {
                fixture.away_country_id
            } else {
                // Defensive: shootouts cannot end tied, but if they do
                // (data anomaly), prefer the home side rather than
                // panicking.
                fixture.home_country_id
            });
        }
        // Last-resort reputation fallback. Should not fire in practice
        // because the play pipeline routes `is_knockout = true` to the
        // engine's shootout resolver, but kept so an engine quirk
        // doesn't stall tournament progression.
        Self::reputation_weighted_winner(continents, fixture)
    }

    fn reputation_weighted_winner(
        continents: &[Continent],
        fixture: &GlobalCompetitionFixture,
    ) -> Option<u32> {
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
