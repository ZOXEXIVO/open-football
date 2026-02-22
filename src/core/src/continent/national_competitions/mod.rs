pub mod competition;
pub mod european_championship;
pub mod schedule;
pub mod world_cup;

pub use competition::*;
pub use european_championship::*;
pub use world_cup::*;

use chrono::{Datelike, NaiveDate};

/// Manages all national team competitions at the continent level
#[derive(Debug, Clone)]
pub struct NationalTeamCompetitions {
    pub world_cup: Option<WorldCupCompetition>,
    pub european_championship: Option<EuropeanChampionship>,
}

impl NationalTeamCompetitions {
    pub fn new() -> Self {
        NationalTeamCompetitions {
            world_cup: None,
            european_championship: None,
        }
    }

    /// Check and start new competition cycles if needed.
    /// Called with the current simulation date and European country IDs sorted by reputation.
    pub fn check_new_cycles(&mut self, date: NaiveDate, country_ids_by_reputation: &[u32], is_europe: bool) {
        let year = date.year();
        let month = date.month();
        let day = date.day();

        // Only initiate draws in September (start of qualifying)
        if month != 9 || day != 1 {
            return;
        }

        // World Cup qualifying cycle check
        if WorldCupCompetition::should_start_cycle(year) {
            if self.world_cup.is_none() || self.world_cup.as_ref().map_or(true, |wc| wc.phase == CompetitionPhase::Completed) {
                let tournament_year = WorldCupCompetition::tournament_year_for(year);
                let mut wc = WorldCupCompetition::new(tournament_year);
                wc.draw_qualifying_groups(country_ids_by_reputation, year);
                self.world_cup = Some(wc);
            }
        }

        // European Championship qualifying cycle check (Europe only)
        if is_europe && EuropeanChampionship::should_start_cycle(year) {
            if self.european_championship.is_none() || self.european_championship.as_ref().map_or(true, |ec| ec.phase == CompetitionPhase::Completed) {
                let tournament_year = EuropeanChampionship::tournament_year_for(year);
                let mut ec = EuropeanChampionship::new(tournament_year);
                ec.draw_qualifying_groups(country_ids_by_reputation, year);
                self.european_championship = Some(ec);
            }
        }
    }

    /// Get all match pairings scheduled for today across all competitions
    /// Returns (home_country_id, away_country_id, competition_label) tuples
    pub fn get_todays_matches(&self, date: NaiveDate) -> Vec<NationalCompetitionFixture> {
        let mut matches = Vec::new();

        // World Cup fixtures
        if let Some(wc) = &self.world_cup {
            match wc.phase {
                CompetitionPhase::Qualifying => {
                    for (group_idx, fix_idx) in wc.get_todays_qualifying_fixtures(date) {
                        if let Some(group) = wc.qualifying_groups.get(group_idx as usize) {
                            if let Some(fixture) = group.fixtures.get(fix_idx) {
                                matches.push(NationalCompetitionFixture {
                                    home_country_id: fixture.home_country_id,
                                    away_country_id: fixture.away_country_id,
                                    competition: NationalCompetitionType::WorldCupQualifying,
                                    group_idx: group_idx as usize,
                                    fixture_idx: fix_idx,
                                });
                            }
                        }
                    }
                }
                CompetitionPhase::GroupStage => {
                    for (group_idx, fix_idx) in wc.get_todays_tournament_group_fixtures(date) {
                        if let Some(group) = wc.tournament_groups.get(group_idx as usize) {
                            if let Some(fixture) = group.fixtures.get(fix_idx) {
                                matches.push(NationalCompetitionFixture {
                                    home_country_id: fixture.home_country_id,
                                    away_country_id: fixture.away_country_id,
                                    competition: NationalCompetitionType::WorldCupGroupStage,
                                    group_idx: group_idx as usize,
                                    fixture_idx: fix_idx,
                                });
                            }
                        }
                    }
                }
                CompetitionPhase::Knockout => {
                    for (bracket_idx, fix_idx) in wc.get_todays_knockout_fixtures(date) {
                        if let Some(bracket) = wc.knockout.get(bracket_idx) {
                            if let Some(fixture) = bracket.fixtures.get(fix_idx) {
                                matches.push(NationalCompetitionFixture {
                                    home_country_id: fixture.home_country_id,
                                    away_country_id: fixture.away_country_id,
                                    competition: NationalCompetitionType::WorldCupKnockout,
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

        // European Championship fixtures
        if let Some(ec) = &self.european_championship {
            match ec.phase {
                CompetitionPhase::Qualifying => {
                    for (group_idx, fix_idx) in ec.get_todays_qualifying_fixtures(date) {
                        if let Some(group) = ec.qualifying_groups.get(group_idx as usize) {
                            if let Some(fixture) = group.fixtures.get(fix_idx) {
                                matches.push(NationalCompetitionFixture {
                                    home_country_id: fixture.home_country_id,
                                    away_country_id: fixture.away_country_id,
                                    competition: NationalCompetitionType::EuroQualifying,
                                    group_idx: group_idx as usize,
                                    fixture_idx: fix_idx,
                                });
                            }
                        }
                    }
                }
                CompetitionPhase::GroupStage => {
                    for (group_idx, fix_idx) in ec.get_todays_tournament_group_fixtures(date) {
                        if let Some(group) = ec.tournament_groups.get(group_idx as usize) {
                            if let Some(fixture) = group.fixtures.get(fix_idx) {
                                matches.push(NationalCompetitionFixture {
                                    home_country_id: fixture.home_country_id,
                                    away_country_id: fixture.away_country_id,
                                    competition: NationalCompetitionType::EuroGroupStage,
                                    group_idx: group_idx as usize,
                                    fixture_idx: fix_idx,
                                });
                            }
                        }
                    }
                }
                CompetitionPhase::Knockout => {
                    for (bracket_idx, fix_idx) in ec.get_todays_knockout_fixtures(date) {
                        if let Some(bracket) = ec.knockout.get(bracket_idx) {
                            if let Some(fixture) = bracket.fixtures.get(fix_idx) {
                                matches.push(NationalCompetitionFixture {
                                    home_country_id: fixture.home_country_id,
                                    away_country_id: fixture.away_country_id,
                                    competition: NationalCompetitionType::EuroKnockout,
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
    pub fn record_result(&mut self, fixture: &NationalCompetitionFixture, home_score: u8, away_score: u8, penalty_winner: Option<u32>) {
        match fixture.competition {
            NationalCompetitionType::WorldCupQualifying => {
                if let Some(wc) = &mut self.world_cup {
                    wc.record_qualifying_result(fixture.group_idx, fixture.fixture_idx, home_score, away_score);
                }
            }
            NationalCompetitionType::WorldCupGroupStage => {
                if let Some(wc) = &mut self.world_cup {
                    wc.record_tournament_group_result(fixture.group_idx, fixture.fixture_idx, home_score, away_score);
                }
            }
            NationalCompetitionType::WorldCupKnockout => {
                if let Some(wc) = &mut self.world_cup {
                    wc.record_knockout_result(fixture.group_idx, fixture.fixture_idx, home_score, away_score, penalty_winner);
                }
            }
            NationalCompetitionType::EuroQualifying => {
                if let Some(ec) = &mut self.european_championship {
                    ec.record_qualifying_result(fixture.group_idx, fixture.fixture_idx, home_score, away_score);
                }
            }
            NationalCompetitionType::EuroGroupStage => {
                if let Some(ec) = &mut self.european_championship {
                    ec.record_tournament_group_result(fixture.group_idx, fixture.fixture_idx, home_score, away_score);
                }
            }
            NationalCompetitionType::EuroKnockout => {
                if let Some(ec) = &mut self.european_championship {
                    ec.record_knockout_result(fixture.group_idx, fixture.fixture_idx, home_score, away_score, penalty_winner);
                }
            }
        }
    }

    /// Check phase transitions (qualifying complete, group stage complete, knockout progression)
    pub fn check_phase_transitions(&mut self) {
        if let Some(wc) = &mut self.world_cup {
            let tournament_year = wc.cycle_year as i32;

            match wc.phase {
                CompetitionPhase::Qualifying => {
                    wc.check_qualifying_complete();
                    if wc.phase == CompetitionPhase::GroupStage {
                        wc.draw_tournament_groups(tournament_year);
                    }
                }
                CompetitionPhase::GroupStage => {
                    wc.check_tournament_groups_complete(tournament_year);
                }
                CompetitionPhase::Knockout => {
                    wc.progress_knockout(tournament_year);
                }
                _ => {}
            }
        }

        if let Some(ec) = &mut self.european_championship {
            let tournament_year = ec.cycle_year as i32;

            match ec.phase {
                CompetitionPhase::Qualifying => {
                    ec.check_qualifying_complete();
                    if ec.phase == CompetitionPhase::GroupStage {
                        ec.draw_tournament_groups(tournament_year);
                    }
                }
                CompetitionPhase::GroupStage => {
                    ec.check_tournament_groups_complete(tournament_year);
                }
                CompetitionPhase::Knockout => {
                    ec.progress_knockout(tournament_year);
                }
                _ => {}
            }
        }
    }
}

/// A fixture from a national team competition, with enough info to record the result back
#[derive(Debug, Clone)]
pub struct NationalCompetitionFixture {
    pub home_country_id: u32,
    pub away_country_id: u32,
    pub competition: NationalCompetitionType,
    pub group_idx: usize,
    pub fixture_idx: usize,
}

/// Type of national team competition for fixture identification
#[derive(Debug, Clone, PartialEq)]
pub enum NationalCompetitionType {
    WorldCupQualifying,
    WorldCupGroupStage,
    WorldCupKnockout,
    EuroQualifying,
    EuroGroupStage,
    EuroKnockout,
}

impl NationalCompetitionType {
    pub fn label(&self) -> &'static str {
        match self {
            NationalCompetitionType::WorldCupQualifying => "WCQ",
            NationalCompetitionType::WorldCupGroupStage => "WC",
            NationalCompetitionType::WorldCupKnockout => "WC",
            NationalCompetitionType::EuroQualifying => "ECQ",
            NationalCompetitionType::EuroGroupStage => "EC",
            NationalCompetitionType::EuroKnockout => "EC",
        }
    }

    pub fn is_knockout(&self) -> bool {
        matches!(self, NationalCompetitionType::WorldCupKnockout | NationalCompetitionType::EuroKnockout)
    }
}
