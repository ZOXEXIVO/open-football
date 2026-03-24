use super::{ContinentResult, ContinentalCompetitionResults};
use crate::continent::{CompetitionTier, ContinentalMatchResult};
use crate::simulator::SimulatorData;
use crate::{Club, Country, SimulationResult};
use chrono::{Datelike, NaiveDate};
use log::debug;
use std::collections::HashMap;

impl ContinentResult {
    pub(crate) fn is_competition_draw_period(&self, date: NaiveDate) -> bool {
        // Champions League draw: August 15
        (date.month() == 8 && date.day() == 15) ||
            // Europa League draw: August 20
            (date.month() == 8 && date.day() == 20) ||
            // Conference League draw: August 25
            (date.month() == 8 && date.day() == 25) ||
            // Knockout stage draws in December
            (date.month() == 12 && date.day() == 15)
    }

    pub(crate) fn conduct_competition_draws(&self, data: &mut SimulatorData, date: NaiveDate) {
        let continent_id = self.get_continent_id();

        // Collect qualified clubs while holding immutable borrow, then drop it
        let clubs = {
            let continent = match data.continent(continent_id) {
                Some(c) => c,
                None => return,
            };

            let mut countries: Vec<&Country> = continent.countries.iter().collect();
            countries.sort_by(|a, b| b.reputation.cmp(&a.reputation));

            match (date.month(), date.day()) {
                (8, 15) => {
                    let cl = Self::collect_cl_qualified_clubs(&countries);
                    if cl.is_empty() { return; }
                    (cl, Vec::new(), Vec::new(), 0u8)
                }
                (8, 20) => {
                    let el = Self::collect_el_qualified_clubs(&countries);
                    if el.is_empty() { return; }
                    (Vec::new(), el, Vec::new(), 1u8)
                }
                (8, 25) => {
                    let conf = Self::collect_conference_qualified_clubs(&countries);
                    if conf.is_empty() { return; }
                    (Vec::new(), Vec::new(), conf, 2u8)
                }
                _ => return,
            }
        };

        let (cl_clubs, el_clubs, conf_clubs, which) = clubs;

        if let Some(continent) = data.continent_mut(continent_id) {
            match which {
                0 => {
                    debug!("Champions League draw: {} qualified clubs", cl_clubs.len());
                    continent.continental_competitions.champions_league.conduct_draw(
                        &cl_clubs,
                        &continent.continental_rankings,
                        date,
                    );
                }
                1 => {
                    debug!("Europa League draw: {} qualified clubs", el_clubs.len());
                    continent.continental_competitions.europa_league.conduct_draw(
                        &el_clubs,
                        &continent.continental_rankings,
                        date,
                    );
                }
                2 => {
                    debug!("Conference League draw: {} qualified clubs", conf_clubs.len());
                    continent.continental_competitions.conference_league.conduct_draw(
                        &conf_clubs,
                        &continent.continental_rankings,
                        date,
                    );
                }
                _ => {}
            }
        }

        // Distribute one-time participation bonus at draw time (not every match day)
        let clubs_for_bonus: Vec<(Vec<u32>, CompetitionTier)> = match which {
            0 => vec![(cl_clubs, CompetitionTier::ChampionsLeague)],
            1 => vec![(el_clubs, CompetitionTier::EuropaLeague)],
            2 => vec![(conf_clubs, CompetitionTier::ConferenceLeague)],
            _ => vec![],
        };
        for (club_ids, tier) in clubs_for_bonus {
            self.distribute_competition_tier_rewards(&club_ids, tier, data);
        }
    }

    /// Champions League: top clubs from each country.
    /// Top 4 countries get 4 spots, next 2 get 2 spots, rest get 1.
    /// Check if a country's top league actually played matches (not just an empty table).
    /// Countries with disabled leagues have clubs but no league activity.
    fn league_has_played(league: &crate::league::League) -> bool {
        league.table.rows.iter().any(|r| r.played > 0)
    }

    fn collect_cl_qualified_clubs(countries: &[&Country]) -> Vec<u32> {
        let mut qualified = Vec::new();

        for (rank, country) in countries.iter().enumerate() {
            let top_league = country.leagues.leagues.iter()
                .find(|l| l.settings.tier == 1 && !l.friendly);

            let league = match top_league {
                Some(l) => l,
                None => continue,
            };

            if !Self::league_has_played(league) {
                continue;
            }

            let spots: usize = if rank < 4 { 4 } else if rank < 6 { 2 } else { 1 };

            let table = &league.table;
            for row in table.rows.iter().take(spots) {
                if row.team_id > 0 {
                    if let Some(club) = country.clubs.iter().find(|c|
                        c.teams.teams.iter().any(|t| t.id == row.team_id)
                    ) {
                        qualified.push(club.id);
                    }
                }
            }
        }

        debug!("Champions League qualification: {} clubs from {} countries",
            qualified.len(), countries.len());

        qualified
    }

    /// Europa League: next tier of clubs after CL qualification.
    /// Top 4 countries: positions 5-7 (3 spots), next 4: positions 3-4 (2 spots),
    /// next 12: position 2 (1 spot).
    fn collect_el_qualified_clubs(countries: &[&Country]) -> Vec<u32> {
        let mut qualified = Vec::new();

        for (rank, country) in countries.iter().enumerate() {
            let top_league = country.leagues.leagues.iter()
                .find(|l| l.settings.tier == 1 && !l.friendly);

            let league = match top_league {
                Some(l) => l,
                None => continue,
            };

            if !Self::league_has_played(league) {
                continue;
            }

            // Determine which table positions to take (after CL spots)
            let (skip, take) = if rank < 4 {
                (4, 3usize) // positions 5-7
            } else if rank < 8 {
                (2, 2) // positions 3-4
            } else if rank < 20 {
                (1, 1) // position 2
            } else {
                continue;
            };

            let table = &league.table;
            for row in table.rows.iter().skip(skip).take(take) {
                if row.team_id > 0 {
                    if let Some(club) = country.clubs.iter().find(|c|
                        c.teams.teams.iter().any(|t| t.id == row.team_id)
                    ) {
                        qualified.push(club.id);
                    }
                }
            }
        }

        debug!("Europa League qualification: {} clubs from {} countries",
            qualified.len(), countries.len());

        qualified
    }

    /// Conference League: third tier of clubs after CL and EL.
    /// Top 4 countries: position 8 (1 spot), next 4: positions 5-6 (2 spots),
    /// next 12: position 3 (1 spot), rest: position 2 (1 spot).
    fn collect_conference_qualified_clubs(countries: &[&Country]) -> Vec<u32> {
        let mut qualified = Vec::new();

        for (rank, country) in countries.iter().enumerate() {
            let top_league = country.leagues.leagues.iter()
                .find(|l| l.settings.tier == 1 && !l.friendly);

            let league = match top_league {
                Some(l) => l,
                None => continue,
            };

            if !Self::league_has_played(league) {
                continue;
            }

            // Determine which table positions to take (after CL + EL spots)
            let (skip, take) = if rank < 4 {
                (7, 1usize) // position 8
            } else if rank < 8 {
                (4, 2) // positions 5-6
            } else if rank < 20 {
                (2, 1) // position 3
            } else {
                (1, 1) // position 2
            };

            let table = &league.table;
            for row in table.rows.iter().skip(skip).take(take) {
                if row.team_id > 0 {
                    if let Some(club) = country.clubs.iter().find(|c|
                        c.teams.teams.iter().any(|t| t.id == row.team_id)
                    ) {
                        qualified.push(club.id);
                    }
                }
            }
        }

        debug!("Conference League qualification: {} clubs from {} countries",
            qualified.len(), countries.len());

        qualified
    }

    pub(crate) fn simulate_continental_competitions(
        &self,
        data: &mut SimulatorData,
        date: NaiveDate,
    ) -> Option<ContinentalCompetitionResults> {
        let continent_id = self.get_continent_id();

        let continent = data.continent_mut(continent_id)?;
        let mut results = ContinentalCompetitionResults::new();

        let clubs_map = Self::get_clubs_map(&continent.countries);

        // Simulate Champions League matches with real engine
        if continent.continental_competitions.champions_league.has_matches_today(date) {
            let real_results = continent.continental_competitions.champions_league.play_matches(
                &clubs_map,
                date,
            );
            let cl_results: Vec<ContinentalMatchResult> = real_results.iter().map(|r| {
                ContinentalMatchResult {
                    home_team: r.home_team_id,
                    away_team: r.away_team_id,
                    home_score: r.score.home_team.get(),
                    away_score: r.score.away_team.get(),
                    competition: CompetitionTier::ChampionsLeague,
                }
            }).collect();
            results.champions_league_results = Some(cl_results);
            results.match_results.extend(real_results);
        }

        // Simulate Europa League matches with real engine
        if continent.continental_competitions.europa_league.has_matches_today(date) {
            let real_results = continent.continental_competitions.europa_league.play_matches(
                &clubs_map,
                date,
            );
            let el_results: Vec<ContinentalMatchResult> = real_results.iter().map(|r| {
                ContinentalMatchResult {
                    home_team: r.home_team_id,
                    away_team: r.away_team_id,
                    home_score: r.score.home_team.get(),
                    away_score: r.score.away_team.get(),
                    competition: CompetitionTier::EuropaLeague,
                }
            }).collect();
            results.europa_league_results = Some(el_results);
            results.match_results.extend(real_results);
        }

        // Simulate Conference League matches with real engine
        if continent.continental_competitions.conference_league.has_matches_today(date) {
            let real_results = continent.continental_competitions.conference_league.play_matches(
                &clubs_map,
                date,
            );
            let conf_results: Vec<ContinentalMatchResult> = real_results.iter().map(|r| {
                ContinentalMatchResult {
                    home_team: r.home_team_id,
                    away_team: r.away_team_id,
                    home_score: r.score.home_team.get(),
                    away_score: r.score.away_team.get(),
                    competition: CompetitionTier::ConferenceLeague,
                }
            }).collect();
            results.conference_league_results = Some(conf_results);
            results.match_results.extend(real_results);
        }

        // Only return Some when there are actual match results to process
        if results.match_results.is_empty() {
            return None;
        }

        Some(results)
    }

    pub(crate) fn process_competition_results(
        &self,
        comp_results: ContinentalCompetitionResults,
        data: &mut SimulatorData,
        result: &mut SimulationResult,
    ) {
        debug!("Processing continental competition results");

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

        // Process real match results through the stat pipeline (player stats routing)
        // and store in global match store so match detail pages can find them
        for mut match_result in comp_results.match_results {
            crate::league::LeagueResult::process_cup_match(&mut match_result, data);
            data.match_store.push(match_result.clone());
            result.match_results.push(match_result);
        }
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
    }

    fn update_club_continental_stats(
        &self,
        club_id: u32,
        match_result: &ContinentalMatchResult,
        is_home: bool,
        data: &mut SimulatorData,
    ) {
        if let Some(club) = data.club_mut(club_id) {
            let (goals_for, goals_against) = if is_home {
                (match_result.home_score, match_result.away_score)
            } else {
                (match_result.away_score, match_result.home_score)
            };

            let won = goals_for > goals_against;
            let drawn = goals_for == goals_against;

            // Update finances with match revenue
            let match_revenue = self.calculate_match_revenue(match_result);
            club.finance.balance.push_income(match_revenue as i64);

            // Win bonus
            if won {
                let win_bonus = self.calculate_win_bonus(match_result);
                club.finance.balance.push_income(win_bonus as i64);
            }

            // Update club reputation based on result
            self.update_club_reputation(club, match_result, won, drawn);

            // Update player morale and form based on result
            self.update_players_after_match(club, won, drawn);

            debug!(
                "Club {} stats updated: revenue +{}",
                club_id,
                match_revenue
            );
        }
    }

    fn calculate_match_revenue(&self, match_result: &ContinentalMatchResult) -> f64 {
        let base_revenue = match match_result.competition {
            CompetitionTier::ChampionsLeague => 3_000_000.0,
            CompetitionTier::EuropaLeague => 1_000_000.0,
            CompetitionTier::ConferenceLeague => 500_000.0,
        };

        let ticket_revenue = 200_000.0;
        base_revenue + ticket_revenue
    }

    fn calculate_win_bonus(&self, match_result: &ContinentalMatchResult) -> f64 {
        match match_result.competition {
            CompetitionTier::ChampionsLeague => 2_800_000.0,
            CompetitionTier::EuropaLeague => 570_000.0,
            CompetitionTier::ConferenceLeague => 500_000.0,
        }
    }

    fn update_club_reputation(&self, club: &mut Club, match_result: &ContinentalMatchResult, won: bool, drawn: bool) {
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
        let _morale_change = if won { 5 } else { -3 };

        for team in &mut club.teams.teams {
            for _player in &mut team.players.players {
                // Morale change (would need morale field in Player)
            }
        }
    }

    fn distribute_competition_tier_rewards(
        &self,
        participating_clubs: &[u32],
        tier: CompetitionTier,
        data: &mut SimulatorData,
    ) {
        let participation_bonus = match tier {
            CompetitionTier::ChampionsLeague => 15_640_000.0,
            CompetitionTier::EuropaLeague => 3_630_000.0,
            CompetitionTier::ConferenceLeague => 2_940_000.0,
        };

        for &club_id in participating_clubs {
            if let Some(club) = data.club_mut(club_id) {
                club.finance.balance.push_income(participation_bonus as i64);

                debug!(
                    "Club {} received participation bonus: {:.2}M",
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
