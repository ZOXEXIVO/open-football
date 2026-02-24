use chrono::{Datelike, NaiveDate};
use log::info;

use crate::continent::national_competitions::{
    CompetitionPhase, NationalCompetitionConfig,
    NationalCompetitionPhase, NationalTeamCompetition,
};
use crate::continent::Continent;

/// Manages global-scope competitions (e.g. World Cup) at the SimulatorData level.
/// Qualifying runs per-continent; the tournament is assembled here from all zones.
#[derive(Debug, Clone)]
pub struct GlobalCompetitions {
    pub configs: Vec<NationalCompetitionConfig>,
    pub tournaments: Vec<NationalTeamCompetition>,
}

impl GlobalCompetitions {
    pub fn new(configs: Vec<NationalCompetitionConfig>) -> Self {
        GlobalCompetitions {
            configs,
            tournaments: Vec::new(),
        }
    }

    /// On June 1 of the tournament year, aggregate qualified teams from all continents
    /// and create the tournament phase.
    pub fn check_tournament_assembly(&mut self, date: NaiveDate, continents: &[Continent]) {
        let month = date.month();
        let day = date.day();

        // Assembly happens on June 1 of the tournament year
        if month != 6 || day != 1 {
            return;
        }

        let year = date.year();

        for config in &self.configs {
            // Check if this is a tournament year for this competition
            // Tournament year: qualifying started 2 years ago
            let qualifying_start_year = year - 2;
            if !config.should_start_cycle(qualifying_start_year) {
                continue;
            }

            // Check if we already have an active tournament for this config
            let already_active = self.tournaments.iter().any(|t| {
                t.config.id == config.id && t.cycle_year == year as u16
            });

            if already_active {
                continue;
            }

            // Aggregate qualified teams from all continents
            let mut all_qualified: Vec<u32> = Vec::new();
            for continent in continents {
                let qualified = continent
                    .national_team_competitions
                    .get_qualified_teams_for(config.id);
                all_qualified.extend(qualified);
            }

            if all_qualified.is_empty() {
                continue;
            }

            // Cap at tournament total_teams
            all_qualified.truncate(config.tournament.total_teams as usize);

            // Create tournament competition
            let mut tournament = NationalTeamCompetition::new(config.clone(), year as u16);
            tournament.qualified_teams = all_qualified;
            tournament.phase = CompetitionPhase::GroupStage;
            tournament.draw_tournament_groups(year);

            info!(
                "Global {} {} tournament assembled: {} teams",
                config.name,
                year,
                tournament.qualified_teams.len()
            );

            self.tournaments.push(tournament);
        }
    }

    /// Get all tournament fixtures for today
    pub fn get_todays_matches(&self, date: NaiveDate) -> Vec<GlobalCompetitionFixture> {
        let mut matches = Vec::new();

        for (tournament_idx, tournament) in self.tournaments.iter().enumerate() {
            match tournament.phase {
                CompetitionPhase::GroupStage => {
                    for (group_idx, fix_idx) in
                        tournament.get_todays_tournament_group_fixtures(date)
                    {
                        if let Some(group) =
                            tournament.tournament_groups.get(group_idx as usize)
                        {
                            if let Some(fixture) = group.fixtures.get(fix_idx) {
                                matches.push(GlobalCompetitionFixture {
                                    home_country_id: fixture.home_country_id,
                                    away_country_id: fixture.away_country_id,
                                    tournament_idx,
                                    phase: NationalCompetitionPhase::GroupStage,
                                    group_idx: group_idx as usize,
                                    fixture_idx: fix_idx,
                                });
                            }
                        }
                    }
                }
                CompetitionPhase::Knockout => {
                    for (bracket_idx, fix_idx) in tournament.get_todays_knockout_fixtures(date) {
                        if let Some(bracket) = tournament.knockout.get(bracket_idx) {
                            if let Some(fixture) = bracket.fixtures.get(fix_idx) {
                                matches.push(GlobalCompetitionFixture {
                                    home_country_id: fixture.home_country_id,
                                    away_country_id: fixture.away_country_id,
                                    tournament_idx,
                                    phase: NationalCompetitionPhase::Knockout,
                                    group_idx: bracket_idx,
                                    fixture_idx: fix_idx,
                                });
                            }
                        }
                    }
                }
                _ => {}
            }
        }

        matches
    }

    /// Record a match result for a global tournament fixture
    pub fn record_result(
        &mut self,
        fixture: &GlobalCompetitionFixture,
        home_score: u8,
        away_score: u8,
        penalty_winner: Option<u32>,
    ) {
        if let Some(tournament) = self.tournaments.get_mut(fixture.tournament_idx) {
            match fixture.phase {
                NationalCompetitionPhase::GroupStage => {
                    tournament.record_tournament_group_result(
                        fixture.group_idx,
                        fixture.fixture_idx,
                        home_score,
                        away_score,
                    );
                }
                NationalCompetitionPhase::Knockout => {
                    tournament.record_knockout_result(
                        fixture.group_idx,
                        fixture.fixture_idx,
                        home_score,
                        away_score,
                        penalty_winner,
                    );
                }
                _ => {}
            }
        }
    }

    /// Check phase transitions for all active tournaments
    pub fn check_phase_transitions(&mut self) {
        for tournament in &mut self.tournaments {
            let tournament_year = tournament.cycle_year as i32;

            match tournament.phase {
                CompetitionPhase::GroupStage => {
                    tournament.check_tournament_groups_complete(tournament_year);
                }
                CompetitionPhase::Knockout => {
                    tournament.progress_knockout(tournament_year);
                }
                _ => {}
            }
        }
    }
}

/// A fixture from a global competition tournament phase
#[derive(Debug, Clone)]
pub struct GlobalCompetitionFixture {
    pub home_country_id: u32,
    pub away_country_id: u32,
    pub tournament_idx: usize,
    pub phase: NationalCompetitionPhase,
    pub group_idx: usize,
    pub fixture_idx: usize,
}
