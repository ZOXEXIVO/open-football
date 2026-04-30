pub mod competition;
pub mod config;
pub mod national_team_competition;
pub mod schedule;
pub mod world;

pub use competition::*;
pub use config::*;
pub use national_team_competition::*;

use chrono::{Datelike, NaiveDate};

/// Phase of a national competition fixture
#[derive(Debug, Clone, PartialEq)]
pub enum NationalCompetitionPhase {
    Qualifying,
    GroupStage,
    Knockout,
}

impl NationalCompetitionPhase {
    pub fn is_knockout(&self) -> bool {
        matches!(self, NationalCompetitionPhase::Knockout)
    }
}

/// Manages all national team competitions at the continent level
#[derive(Debug, Clone)]
pub struct NationalTeamCompetitions {
    pub competition_configs: Vec<NationalCompetitionConfig>,
    pub competitions: Vec<NationalTeamCompetition>,
}

impl NationalTeamCompetitions {
    pub fn new(configs: Vec<NationalCompetitionConfig>) -> Self {
        NationalTeamCompetitions {
            competition_configs: configs,
            competitions: Vec::new(),
        }
    }

    /// Check and start new competition cycles if needed.
    /// Called with the current simulation date and country IDs sorted by reputation.
    pub fn check_new_cycles(
        &mut self,
        date: NaiveDate,
        country_ids_by_reputation: &[u32],
        continent_id: u32,
    ) {
        let year = date.year();
        let month = date.month();
        let day = date.day();

        // Only initiate draws in September (start of qualifying)
        if month != 9 || day != 1 {
            return;
        }

        for config_idx in 0..self.competition_configs.len() {
            let config = &self.competition_configs[config_idx];

            if !config.should_start_cycle(year) {
                continue;
            }

            // Find the qualifying zone for this continent
            let zone = match config.qualifying_zone_for(continent_id) {
                Some(z) => z.clone(),
                None => continue,
            };

            // Check if there's already an active competition for this config
            let already_active = self
                .competitions
                .iter()
                .any(|c| c.config.id == config.id && c.phase != CompetitionPhase::Completed);

            if already_active {
                continue;
            }

            let tournament_year = config.tournament_year_for(year);
            let config_clone = config.clone();
            let mut comp = NationalTeamCompetition::new(config_clone, tournament_year);
            comp.draw_qualifying_groups(country_ids_by_reputation, year, &zone);
            self.competitions.push(comp);
        }
    }

    /// Get all match pairings scheduled for today across all competitions
    pub fn get_todays_matches(&self, date: NaiveDate) -> Vec<NationalCompetitionFixture> {
        let mut matches = Vec::new();

        for (comp_idx, comp) in self.competitions.iter().enumerate() {
            match comp.phase {
                CompetitionPhase::Qualifying => {
                    for (group_idx, fix_idx) in comp.get_todays_qualifying_fixtures(date) {
                        if let Some(group) = comp.qualifying_groups.get(group_idx as usize) {
                            if let Some(fixture) = group.fixtures.get(fix_idx) {
                                matches.push(NationalCompetitionFixture {
                                    home_country_id: fixture.home_country_id,
                                    away_country_id: fixture.away_country_id,
                                    competition_idx: comp_idx,
                                    phase: NationalCompetitionPhase::Qualifying,
                                    group_idx: group_idx as usize,
                                    fixture_idx: fix_idx,
                                });
                            }
                        }
                    }
                }
                CompetitionPhase::GroupStage => {
                    for (group_idx, fix_idx) in comp.get_todays_tournament_group_fixtures(date) {
                        if let Some(group) = comp.tournament_groups.get(group_idx as usize) {
                            if let Some(fixture) = group.fixtures.get(fix_idx) {
                                matches.push(NationalCompetitionFixture {
                                    home_country_id: fixture.home_country_id,
                                    away_country_id: fixture.away_country_id,
                                    competition_idx: comp_idx,
                                    phase: NationalCompetitionPhase::GroupStage,
                                    group_idx: group_idx as usize,
                                    fixture_idx: fix_idx,
                                });
                            }
                        }
                    }
                }
                CompetitionPhase::Knockout => {
                    for (bracket_idx, fix_idx) in comp.get_todays_knockout_fixtures(date) {
                        if let Some(bracket) = comp.knockout.get(bracket_idx) {
                            if let Some(fixture) = bracket.fixtures.get(fix_idx) {
                                matches.push(NationalCompetitionFixture {
                                    home_country_id: fixture.home_country_id,
                                    away_country_id: fixture.away_country_id,
                                    competition_idx: comp_idx,
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

    /// Record a match result for the appropriate competition
    pub fn record_result(
        &mut self,
        fixture: &NationalCompetitionFixture,
        home_score: u8,
        away_score: u8,
        penalty_winner: Option<u32>,
    ) {
        if let Some(comp) = self.competitions.get_mut(fixture.competition_idx) {
            match fixture.phase {
                NationalCompetitionPhase::Qualifying => {
                    comp.record_qualifying_result(
                        fixture.group_idx,
                        fixture.fixture_idx,
                        home_score,
                        away_score,
                    );
                }
                NationalCompetitionPhase::GroupStage => {
                    comp.record_tournament_group_result(
                        fixture.group_idx,
                        fixture.fixture_idx,
                        home_score,
                        away_score,
                    );
                }
                NationalCompetitionPhase::Knockout => {
                    comp.record_knockout_result(
                        fixture.group_idx,
                        fixture.fixture_idx,
                        home_score,
                        away_score,
                        penalty_winner,
                    );
                }
            }
        }
    }

    /// Check phase transitions (qualifying complete, group stage complete, knockout progression)
    pub fn check_phase_transitions(&mut self, continent_id: u32) {
        for comp in &mut self.competitions {
            let tournament_year = comp.cycle_year as i32;

            match comp.phase {
                CompetitionPhase::Qualifying => {
                    if let Some(zone) = comp.config.qualifying_zone_for(continent_id) {
                        let zone = zone.clone();
                        comp.check_qualifying_complete(&zone);
                        if comp.phase == CompetitionPhase::GroupStage {
                            comp.draw_tournament_groups(tournament_year);
                        }
                    }
                }
                CompetitionPhase::GroupStage => {
                    comp.check_tournament_groups_complete(tournament_year);
                }
                CompetitionPhase::Knockout => {
                    comp.progress_knockout(tournament_year);
                }
                _ => {}
            }
        }
    }

    /// Get qualified teams for a specific competition (by config id), for global tournament assembly
    pub fn get_qualified_teams_for(&self, competition_id: u32) -> Vec<u32> {
        self.competitions
            .iter()
            .filter(|c| c.config.id == competition_id && c.phase == CompetitionPhase::Completed)
            .flat_map(|c| c.qualified_teams.iter().copied())
            .collect()
    }
}

/// A fixture from a national team competition, with enough info to record the result back
#[derive(Debug, Clone)]
pub struct NationalCompetitionFixture {
    pub home_country_id: u32,
    pub away_country_id: u32,
    pub competition_idx: usize,
    pub phase: NationalCompetitionPhase,
    pub group_idx: usize,
    pub fixture_idx: usize,
}
