use super::{ContinentResult, ContinentalCompetitionResults};
use crate::continent::{
    CompetitionTier, ContinentalCompetitions, ContinentalMatchResult, ContinentalMatchday,
    ContinentalQualification,
};
use crate::league::League;
use crate::league::LeagueResult;
use crate::league::simulation::matchday::MatchdayCommitments;
use crate::r#match::{Match, MatchResult};
use crate::utils::PerformanceProfiler;
use crate::world::SimulatorData;
use crate::{Club, Country, MatchRuntime, SimulationResult};
use chrono::{Datelike, NaiveDate};
use log::debug;
use rustc_hash::{FxHashMap, FxHashSet};

/// One draw day's qualified fields: which competition is drawing and the
/// clubs it takes.
struct DrawPlan {
    cl_clubs: Vec<u32>,
    el_clubs: Vec<u32>,
    conf_clubs: Vec<u32>,
    copa_clubs: Vec<u32>,
    which: u8,
}

impl DrawPlan {
    fn for_date(continent: &crate::continent::Continent, date: NaiveDate) -> Option<Self> {
        let is_europe = continent.is_europe();
        let is_south_america = continent.is_south_america();

        let countries = ContinentalQualification::ranked_countries(continent);
        let already_drawn = continent
            .continental_competitions
            .drawn_this_season(date.year() as u16);

        match (date.month(), date.day()) {
            (8, 15) if is_europe => {
                let cl = ContinentResult::collect_qualified_clubs(
                    &countries,
                    &CompetitionTier::ChampionsLeague,
                    &already_drawn,
                );
                if cl.is_empty() {
                    return None;
                }
                Some(DrawPlan {
                    cl_clubs: cl,
                    el_clubs: Vec::new(),
                    conf_clubs: Vec::new(),
                    copa_clubs: Vec::new(),
                    which: 0,
                })
            }
            (8, 16) if is_south_america => {
                let copa = ContinentResult::collect_qualified_clubs(
                    &countries,
                    &CompetitionTier::CopaLibertadores,
                    &already_drawn,
                );
                if copa.is_empty() {
                    return None;
                }
                Some(DrawPlan {
                    cl_clubs: Vec::new(),
                    el_clubs: Vec::new(),
                    conf_clubs: Vec::new(),
                    copa_clubs: copa,
                    which: 3,
                })
            }
            (8, 20) if is_europe => {
                let el = ContinentResult::collect_qualified_clubs(
                    &countries,
                    &CompetitionTier::EuropaLeague,
                    &already_drawn,
                );
                if el.is_empty() {
                    return None;
                }
                Some(DrawPlan {
                    cl_clubs: Vec::new(),
                    el_clubs: el,
                    conf_clubs: Vec::new(),
                    copa_clubs: Vec::new(),
                    which: 1,
                })
            }
            (8, 25) if is_europe => {
                let conf = ContinentResult::collect_qualified_clubs(
                    &countries,
                    &CompetitionTier::ConferenceLeague,
                    &already_drawn,
                );
                if conf.is_empty() {
                    return None;
                }
                Some(DrawPlan {
                    cl_clubs: Vec::new(),
                    el_clubs: Vec::new(),
                    conf_clubs: conf,
                    copa_clubs: Vec::new(),
                    which: 2,
                })
            }
            _ => None,
        }
    }

    fn conduct(&self, continent: &mut crate::continent::Continent, date: NaiveDate) {
        match self.which {
            0 => {
                debug!(
                    "Champions League draw: {} qualified clubs",
                    self.cl_clubs.len()
                );
                continent
                    .continental_competitions
                    .champions_league
                    .conduct_draw(&self.cl_clubs, &continent.continental_rankings, date);
            }
            1 => {
                debug!(
                    "Europa League draw: {} qualified clubs",
                    self.el_clubs.len()
                );
                continent
                    .continental_competitions
                    .europa_league
                    .conduct_draw(&self.el_clubs, &continent.continental_rankings, date);
            }
            2 => {
                debug!(
                    "Conference League draw: {} qualified clubs",
                    self.conf_clubs.len()
                );
                continent
                    .continental_competitions
                    .conference_league
                    .conduct_draw(&self.conf_clubs, &continent.continental_rankings, date);
            }
            3 => {
                debug!(
                    "Copa Libertadores draw: {} qualified clubs",
                    self.copa_clubs.len()
                );
                continent
                    .continental_competitions
                    .copa_libertadores
                    .conduct_draw(&self.copa_clubs, &continent.continental_rankings, date);
            }
            _ => {}
        }
    }

    fn distribute_bonuses(&self, result: &ContinentResult, data: &mut SimulatorData) {
        let clubs_for_bonus: Vec<(Vec<u32>, CompetitionTier)> = match self.which {
            0 => vec![(self.cl_clubs.clone(), CompetitionTier::ChampionsLeague)],
            1 => vec![(self.el_clubs.clone(), CompetitionTier::EuropaLeague)],
            2 => vec![(self.conf_clubs.clone(), CompetitionTier::ConferenceLeague)],
            3 => vec![(self.copa_clubs.clone(), CompetitionTier::CopaLibertadores)],
            _ => vec![],
        };
        for (club_ids, tier) in clubs_for_bonus {
            result.distribute_competition_tier_rewards(&club_ids, tier, data);
        }
    }
}

impl ContinentResult {
    pub(crate) fn is_competition_draw_period(&self, date: NaiveDate) -> bool {
        // Champions League draw: August 15 (Europe)
        (date.month() == 8 && date.day() == 15) ||
            // Copa Libertadores draw: August 16 (South America)
            (date.month() == 8 && date.day() == 16) ||
            // Europa League draw: August 20 (Europe)
            (date.month() == 8 && date.day() == 20) ||
            // Conference League draw: August 25 (Europe)
            (date.month() == 8 && date.day() == 25) ||
            // Knockout stage draws in December
            (date.month() == 12 && date.day() == 15)
    }

    pub(crate) fn conduct_competition_draws(&self, data: &mut SimulatorData, date: NaiveDate) {
        let continent_id = self.get_continent_id();

        // Collect qualified clubs while holding immutable borrow, then drop it.
        // Draws are continent-scoped: UEFA competitions only run in Europe and
        // Copa Libertadores only runs in South America, so a shared draw date
        // never fires the wrong competition on the wrong continent.
        let Some(plan) = data
            .continent(continent_id)
            .and_then(|continent| DrawPlan::for_date(continent, date))
        else {
            return;
        };

        if let Some(continent) = data.continent_mut(continent_id) {
            plan.conduct(continent, date);
        }

        // Distribute one-time participation bonus at draw time (not every match day)
        plan.distribute_bonuses(self, data);
    }

    /// Check if a country's top league actually played matches (not just an empty table).
    /// Countries with disabled leagues have clubs but no league activity.
    fn league_has_played(league: &League) -> bool {
        league.table.rows.iter().any(|r| r.played > 0)
    }

    /// The clubs a league table sends to `tier`. A club already drawn into
    /// another of the season's competitions is passed over and its place
    /// goes to the next club down, so a side that moved between two August
    /// draws is never in two competitions.
    fn collect_qualified_clubs(
        countries: &[&Country],
        tier: &CompetitionTier,
        already_drawn: &FxHashSet<u32>,
    ) -> Vec<u32> {
        let cap = ContinentalQualification::field_cap(tier).unwrap_or(usize::MAX);
        let mut qualified = Vec::new();

        for (rank, country) in countries.iter().enumerate() {
            let Some(league) = ContinentalQualification::qualifying_league(country) else {
                continue;
            };
            if !Self::league_has_played(league) {
                continue;
            }
            let Some((skip, take)) = ContinentalQualification::band(tier, rank) else {
                continue;
            };

            let band = league
                .table
                .rows
                .iter()
                .skip(skip)
                .filter(|row| row.team_id != 0)
                .filter_map(|row| country.clubs.iter().find(|c| c.teams.contains(row.team_id)))
                .filter(|club| !already_drawn.contains(&club.id))
                .take(take);
            for club in band {
                if !qualified.contains(&club.id) {
                    qualified.push(club.id);
                }
            }
        }

        qualified.truncate(cap);

        debug!(
            "{:?} qualification: {} clubs from {} countries",
            tier,
            qualified.len(),
            countries.len()
        );

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
        // Domestic fixtures dated today were played in the morning's batch;
        // a squad playing one lends nobody to a continental tie.
        let commitments = MatchdayCommitments::gather(
            continent.countries.iter().flat_map(|c| c.schedules()),
            date,
        );

        // Every competition builds today's ties from the same club state,
        // then the continent plays all of them in one dispatch: a Thursday
        // with two competitions costs one match's length, not two.
        let competitions = &mut continent.continental_competitions;
        let mut days =
            Self::prepare_continental_matchdays(competitions, &clubs_map, date, &commitments);

        let (batch, sizes) = Self::drain_continental_batch(&mut days);
        let mut played = if batch.is_empty() {
            Vec::new()
        } else {
            PerformanceProfiler::stage("drain_continental_dispatch", 2, || {
                MatchRuntime::engine_pool().play(batch)
            })
        }
        .into_iter();

        let [champions, europa, conference, libertadores] = days;
        Self::record_champions_day(competitions, champions, sizes[0], &mut played, &mut results);
        Self::record_europa_day(competitions, europa, sizes[1], &mut played, &mut results);
        Self::record_conference_day(
            competitions,
            conference,
            sizes[2],
            &mut played,
            &mut results,
        );
        Self::record_libertadores_day(
            competitions,
            libertadores,
            sizes[3],
            &mut played,
            &mut results,
        );

        // Only return Some when there are actual match results to process
        if results.match_results.is_empty() {
            return None;
        }

        Some(results)
    }

    /// Build every competition's matchday for today from one shared club
    /// snapshot.
    fn prepare_continental_matchdays(
        competitions: &ContinentalCompetitions,
        clubs_map: &FxHashMap<u32, &Club>,
        date: NaiveDate,
        commitments: &MatchdayCommitments,
    ) -> [Option<ContinentalMatchday>; 4] {
        [
            competitions
                .champions_league
                .has_matches_today(date)
                .then(|| {
                    competitions
                        .champions_league
                        .prepare_matchday(clubs_map, date, commitments)
                }),
            competitions.europa_league.has_matches_today(date).then(|| {
                competitions
                    .europa_league
                    .prepare_matchday(clubs_map, date, commitments)
            }),
            competitions
                .conference_league
                .has_matches_today(date)
                .then(|| {
                    competitions
                        .conference_league
                        .prepare_matchday(clubs_map, date, commitments)
                }),
            competitions
                .copa_libertadores
                .has_matches_today(date)
                .then(|| {
                    competitions
                        .copa_libertadores
                        .prepare_matchday(clubs_map, date, commitments)
                }),
        ]
    }

    /// Concatenate every competition's matches into one engine batch and
    /// record the per-competition sizes for the result split afterwards.
    fn drain_continental_batch(
        days: &mut [Option<ContinentalMatchday>; 4],
    ) -> (Vec<Match>, [usize; 4]) {
        let mut sizes = [0usize; 4];
        let mut batch: Vec<Match> = Vec::new();
        for (day, size) in days.iter_mut().zip(sizes.iter_mut()) {
            if let Some(day) = day {
                *size = day.matches.len();
                batch.append(&mut day.matches);
            }
        }
        (batch, sizes)
    }

    fn record_champions_day(
        competitions: &mut ContinentalCompetitions,
        day: Option<ContinentalMatchday>,
        size: usize,
        played: &mut impl Iterator<Item = MatchResult>,
        results: &mut ContinentalCompetitionResults,
    ) {
        if let Some(day) = day {
            let real_results = match size {
                0 => Vec::new(),
                n => competitions
                    .champions_league
                    .record_matchday(day.todays, played.by_ref().take(n).collect()),
            };
            results.champions_league_results = Some(Self::tier_results(
                &real_results,
                CompetitionTier::ChampionsLeague,
            ));
            results.match_results.extend(real_results);
        }
    }

    fn record_europa_day(
        competitions: &mut ContinentalCompetitions,
        day: Option<ContinentalMatchday>,
        size: usize,
        played: &mut impl Iterator<Item = MatchResult>,
        results: &mut ContinentalCompetitionResults,
    ) {
        if let Some(day) = day {
            let real_results = match size {
                0 => Vec::new(),
                n => competitions
                    .europa_league
                    .record_matchday(day.todays, played.by_ref().take(n).collect()),
            };
            results.europa_league_results = Some(Self::tier_results(
                &real_results,
                CompetitionTier::EuropaLeague,
            ));
            results.match_results.extend(real_results);
        }
    }

    fn record_conference_day(
        competitions: &mut ContinentalCompetitions,
        day: Option<ContinentalMatchday>,
        size: usize,
        played: &mut impl Iterator<Item = MatchResult>,
        results: &mut ContinentalCompetitionResults,
    ) {
        if let Some(day) = day {
            let real_results = match size {
                0 => Vec::new(),
                n => competitions
                    .conference_league
                    .record_matchday(day.todays, played.by_ref().take(n).collect()),
            };
            results.conference_league_results = Some(Self::tier_results(
                &real_results,
                CompetitionTier::ConferenceLeague,
            ));
            results.match_results.extend(real_results);
        }
    }

    fn record_libertadores_day(
        competitions: &mut ContinentalCompetitions,
        day: Option<ContinentalMatchday>,
        size: usize,
        played: &mut impl Iterator<Item = MatchResult>,
        results: &mut ContinentalCompetitionResults,
    ) {
        if let Some(day) = day {
            let real_results = match size {
                0 => Vec::new(),
                n => competitions
                    .copa_libertadores
                    .record_matchday(day.todays, played.by_ref().take(n).collect()),
            };
            results.copa_libertadores_results = Some(Self::tier_results(
                &real_results,
                CompetitionTier::CopaLibertadores,
            ));
            results.match_results.extend(real_results);
        }
    }

    /// Write every first team's continental run into its fixture window,
    /// beside the domestic run its country keeps there.
    pub(crate) fn refresh_continental_fixture_windows(
        &self,
        data: &mut SimulatorData,
        date: NaiveDate,
    ) {
        let Some(continent) = data.continent_mut(self.get_continent_id()) else {
            return;
        };
        let mut runs = {
            let clubs = Self::get_clubs_map(&continent.countries);
            continent
                .continental_competitions
                .fixture_runs(date, &clubs)
        };
        for club in continent
            .countries
            .iter_mut()
            .flat_map(|country| country.clubs.iter_mut())
        {
            let run = runs.remove(&club.id).unwrap_or_default();
            let Some(team) = club.teams.main_mut() else {
                continue;
            };
            if team.fixture_window.continental != run {
                team.fixture_window.continental = run;
            }
            // Both runs of the window are current now, so the squad can
            // be told which competitions its next fixture belongs to.
            let family = team.fixture_window.next_family(date);
            for player in team.players.iter_mut() {
                if player.player_attributes.discipline.awaiting != family {
                    player.on_next_fixture_family(family);
                }
            }
        }
    }

    fn tier_results(
        results: &[MatchResult],
        competition: CompetitionTier,
    ) -> Vec<ContinentalMatchResult> {
        results
            .iter()
            .map(|r| ContinentalMatchResult {
                home_team: r.home_team_id,
                away_team: r.away_team_id,
                home_score: r.score.home_team.get(),
                away_score: r.score.away_team.get(),
                competition: competition.clone(),
            })
            .collect()
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

        // Process Copa Libertadores results
        if let Some(copa_results) = comp_results.copa_libertadores_results {
            for match_result in copa_results {
                self.process_single_match(match_result, data, result);
            }
        }

        // Process real match results through the stat pipeline (player stats routing)
        // and store in global match store so match detail pages can find them
        let today = data.date.date();
        for mut match_result in comp_results.match_results {
            LeagueResult::process_cup_match(&mut match_result, data);
            data.match_store
                .push(match_result.copy_without_data_positions(), today);
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

            // Update finances with match revenue — categorised as
            // continental prize money so the finance page and audits can
            // attribute it (plain `push_income` showed up as "other").
            let match_revenue = self.calculate_match_revenue(match_result);
            club.finance
                .balance
                .push_income_continental_prize(match_revenue as i64);

            // Win bonus
            if won {
                let win_bonus = self.calculate_win_bonus(match_result);
                club.finance
                    .balance
                    .push_income_continental_prize(win_bonus as i64);
            }

            // Update club reputation based on result
            self.update_club_reputation(club, match_result, won, drawn);

            // Update player morale and form based on result
            self.update_players_after_match(club, won, drawn);

            debug!("Club {} stats updated: revenue +{}", club_id, match_revenue);
        }
    }

    fn calculate_match_revenue(&self, match_result: &ContinentalMatchResult) -> f64 {
        let base_revenue = match match_result.competition {
            CompetitionTier::ChampionsLeague => 3_000_000.0,
            CompetitionTier::EuropaLeague => 1_000_000.0,
            CompetitionTier::ConferenceLeague => 500_000.0,
            CompetitionTier::CopaLibertadores => 1_250_000.0,
        };

        let ticket_revenue = match match_result.competition {
            CompetitionTier::CopaLibertadores => 180_000.0,
            _ => 200_000.0,
        };
        base_revenue + ticket_revenue
    }

    fn calculate_win_bonus(&self, match_result: &ContinentalMatchResult) -> f64 {
        match match_result.competition {
            CompetitionTier::ChampionsLeague => 2_800_000.0,
            CompetitionTier::EuropaLeague => 570_000.0,
            CompetitionTier::ConferenceLeague => 500_000.0,
            CompetitionTier::CopaLibertadores => 650_000.0,
        }
    }

    fn update_club_reputation(
        &self,
        club: &mut Club,
        match_result: &ContinentalMatchResult,
        won: bool,
        drawn: bool,
    ) {
        let reputation_change = if won {
            match match_result.competition {
                CompetitionTier::ChampionsLeague => 5,
                CompetitionTier::EuropaLeague => 3,
                CompetitionTier::ConferenceLeague => 2,
                CompetitionTier::CopaLibertadores => 4,
            }
        } else if drawn {
            1
        } else {
            -2
        };

        debug!(
            "Club {} reputation change: {:+}",
            club.id, reputation_change
        );
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
            CompetitionTier::CopaLibertadores => 3_000_000.0,
        };

        for &club_id in participating_clubs {
            if let Some(club) = data.club_mut(club_id) {
                club.finance
                    .balance
                    .push_income_continental_prize(participation_bonus as i64);

                debug!(
                    "Club {} received participation bonus: {:.2}M",
                    club_id,
                    participation_bonus / 1_000_000.0
                );
            }
        }
    }

    fn get_clubs_map(countries: &[Country]) -> FxHashMap<u32, &Club> {
        countries
            .iter()
            .flat_map(|c| &c.clubs)
            .map(|club| (club.id, club))
            .collect()
    }
}

#[cfg(test)]
mod qualification_tests {
    use super::*;
    use crate::country::country::fixture_calendar_tests::country_with_first_teams;
    use crate::league::LeagueTableRow;

    #[test]
    fn a_club_already_in_europe_passes_its_conference_place_down_the_table() {
        // The continent's top-ranked country: its Conference League place
        // is eighth. Clubs finish in id order, club n*100 in place n.
        let mut country = country_with_first_teams(10);
        country.leagues.leagues[0].table.rows = (1..=10u32)
            .map(|n| LeagueTableRow {
                team_id: n * 10,
                played: 3,
                win: 0,
                draft: 0,
                lost: 0,
                goal_scored: 0,
                goal_concerned: 0,
                points: (30 - n) as u8,
                points_deduction: 0,
            })
            .collect();
        let countries = vec![&country];

        let fresh = ContinentResult::collect_qualified_clubs(
            &countries,
            &CompetitionTier::ConferenceLeague,
            &FxHashSet::default(),
        );
        assert_eq!(
            fresh,
            vec![800],
            "eighth place goes to the Conference League"
        );

        // Club 800 was seventh on Europa League draw day and is already in.
        let in_europe: FxHashSet<u32> = [800].into_iter().collect();
        let passed_down = ContinentResult::collect_qualified_clubs(
            &countries,
            &CompetitionTier::ConferenceLeague,
            &in_europe,
        );
        assert_eq!(passed_down, vec![900], "the place passes to ninth");
    }
}
