use crate::continent::continent::Continent;
use crate::NationalTeam;
use chrono::NaiveDate;

impl Continent {
    /// Play all international friendly matches scheduled for today in parallel.
    pub(crate) fn simulate_international_friendlies(&mut self, date: NaiveDate) {
        use crate::r#match::engine::engine::FootballEngine;
        use crate::r#match::MatchSquad;
        use rayon::iter::{IntoParallelIterator, ParallelIterator};

        // Step 1: Prepare squads for all countries with a pending friendly (sequential)
        let mut prepared: Vec<(usize, usize, MatchSquad, MatchSquad)> = Vec::new();

        for (country_idx, country) in self.countries.iter().enumerate() {
            if let Some(fixture_idx) = country.national_team.pending_friendly(date) {
                let fixture = &country.national_team.schedule[fixture_idx];
                let our_squad = country.national_team.build_match_squad(&country.clubs);
                let opponent_squad =
                    NationalTeam::build_synthetic_opponent_squad(fixture.opponent_country_id, &fixture.opponent_country_name);

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

        // Step 2: Run all match engines in parallel (limited to 4 threads)
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(4)
            .build()
            .unwrap();
        let engine_results: Vec<(usize, usize, crate::r#match::MatchResultRaw)> = pool.install(|| {
            prepared
                .into_par_iter()
                .map(|(country_idx, fixture_idx, home_squad, away_squad)| {
                    let result = FootballEngine::<840, 545>::play(home_squad, away_squad, crate::is_match_recordings_mode());
                    (country_idx, fixture_idx, result)
                })
                .collect()
        });

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
}
