pub mod competition;
pub mod config;
pub mod schedule;
pub mod world;

pub use competition::*;
pub use config::*;

use crate::NationalTeamLevel;
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

    /// Months until the next major tournament these countries are
    /// heading toward. `u8::MAX` when there is none in view.
    ///
    /// The tournament calendar is the only clock in football that puts a
    /// hard date on a player's *club* situation: a fringe international
    /// who is not playing in the January before a World Cup will move,
    /// and will drop a level to do it. `MindSituation::tournament_pressure`
    /// is what reads this, and it is why every January of a tournament
    /// year looks different from every other January.
    ///
    /// Derived from the cycle arithmetic rather than from a fixture,
    /// because the fixtures for a tournament two years out do not exist
    /// yet and the players thinking about it do. Assumes a June
    /// tournament, which every competition in the catalogue is.
    pub fn months_to_next_tournament(&self, date: NaiveDate) -> u8 {
        const TOURNAMENT_MONTH: u32 = 6;

        let mut soonest = u32::MAX;
        for config in &self.competition_configs {
            let cycle = config.cycle_years.max(1) as i32;
            // The window has to reach *backwards* as well as forwards.
            // A tournament two years out had its qualifying draw two
            // years ago, so a forward-only walk skips the whole current
            // cycle and reports the one after it — three years late,
            // every time, which is exactly the case this is for.
            for offset in -cycle..=cycle {
                let year = date.year() + offset;
                if !config.should_start_cycle(year) {
                    continue;
                }
                let tournament_year = config.tournament_year_for(year) as i32;
                let months = (tournament_year - date.year()) * 12 + TOURNAMENT_MONTH as i32
                    - date.month() as i32;
                if months >= 0 {
                    soonest = soonest.min(months as u32);
                }
            }
        }

        if soonest == u32::MAX {
            u8::MAX
        } else {
            soonest.min(u8::MAX as u32 - 1) as u8
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
                                    level: comp.config.team_level,
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
                                    level: comp.config.team_level,
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
                                    level: comp.config.team_level,
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
    /// National-team level this fixture is contested at — stamped from
    /// the owning competition's `config.team_level`. Drives squad
    /// building, stats, and schedule routing in the world orchestrator.
    pub level: NationalTeamLevel,
}

#[cfg(test)]
mod tournament_clock_tests {
    use super::*;
    use crate::continent::national::config::{
        CompetitionScope, QualifyingConfig, ScheduleConfig, TournamentConfig,
    };

    fn config(id: u32, cycle_years: u32, cycle_offset: u32) -> NationalCompetitionConfig {
        NationalCompetitionConfig {
            id,
            name: format!("Competition {id}"),
            short_name: format!("C{id}"),
            scope: CompetitionScope::Global,
            continent_id: None,
            team_level: NationalTeamLevel::default(),
            cycle_years,
            cycle_offset,
            qualifying: QualifyingConfig { zones: Vec::new() },
            tournament: TournamentConfig {
                total_teams: 24,
                group_count: 6,
                teams_per_group: 4,
                advance_per_group: 2,
                best_third_placed: 4,
            },
            schedule: ScheduleConfig {
                qualifying_dates: Vec::new(),
                tournament_group_dates: Vec::new(),
                tournament_knockout_dates: Vec::new(),
            },
        }
    }

    fn day(y: i32, m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, d).expect("valid date")
    }

    #[test]
    fn a_confederation_with_no_competitions_has_no_clock() {
        let comps = NationalTeamCompetitions::new(Vec::new());
        assert_eq!(comps.months_to_next_tournament(day(2030, 1, 1)), u8::MAX);
    }

    #[test]
    fn the_clock_counts_down_to_the_next_tournament() {
        // World Cup shape: cycle 4, offset 2 → qualifying opens in years
        // divisible by four, tournament two years later (2026, 2030…).
        let comps = NationalTeamCompetitions::new(vec![config(1, 4, 2)]);

        let january_before = comps.months_to_next_tournament(day(2030, 1, 1));
        let two_years_out = comps.months_to_next_tournament(day(2028, 6, 1));

        assert_eq!(january_before, 5, "June 2030 is five months from January");
        assert!(
            two_years_out > january_before,
            "the clock runs down, not up"
        );
    }

    #[test]
    fn the_soonest_tournament_is_the_one_that_presses() {
        // A confederation running both a global and a continental cycle:
        // a player is heading for whichever comes first, which is what a
        // January window actually reacts to.
        let both = NationalTeamCompetitions::new(vec![config(1, 4, 2), config(2, 2, 1)]);
        let global_only = NationalTeamCompetitions::new(vec![config(1, 4, 2)]);

        let date = day(2028, 9, 1);
        assert!(
            both.months_to_next_tournament(date) <= global_only.months_to_next_tournament(date),
            "the nearer of the two is the one he is playing for"
        );
    }
}
