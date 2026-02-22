use chrono::NaiveDate;
use log::info;

use super::competition::*;
use super::schedule;

/// World Cup competition managing both qualifying and tournament phases.
/// Cycle: qualifying starts ~2 years before the tournament year.
#[derive(Debug, Clone)]
pub struct WorldCupCompetition {
    pub cycle_year: u16,
    pub phase: CompetitionPhase,
    pub qualifying_groups: Vec<QualifyingGroup>,
    pub qualified_teams: Vec<u32>,
    pub tournament_groups: Vec<QualifyingGroup>,
    pub knockout: Vec<KnockoutBracket>,
    pub champion: Option<u32>,
}

/// European zone: 13 spots, ~10 groups of 5
const EUROPEAN_WC_QUALIFYING_SPOTS: usize = 13;
const WC_TOURNAMENT_GROUP_COUNT: usize = 8;
const WC_TOURNAMENT_TEAMS_PER_GROUP: usize = 4;

impl WorldCupCompetition {
    pub fn new(cycle_year: u16) -> Self {
        WorldCupCompetition {
            cycle_year,
            phase: CompetitionPhase::NotStarted,
            qualifying_groups: Vec::new(),
            qualified_teams: Vec::new(),
            tournament_groups: Vec::new(),
            knockout: Vec::new(),
            champion: None,
        }
    }

    /// Should a new WC cycle start? WC qualifying begins 2 years before the tournament.
    /// Tournament years: 2026, 2030, 2034, ...
    pub fn should_start_cycle(year: i32) -> bool {
        // Qualifying starts in September, 2 years before the tournament
        (year + 2) % 4 == 2 // e.g., 2024 qualifying for 2026 WC
    }

    /// Get the tournament year for a qualifying start year
    pub fn tournament_year_for(qualifying_start_year: i32) -> u16 {
        (qualifying_start_year + 2) as u16
    }

    /// Draw qualifying groups from country IDs sorted by reputation (descending)
    pub fn draw_qualifying_groups(&mut self, country_ids_by_reputation: &[u32], start_year: i32) {
        if country_ids_by_reputation.is_empty() {
            return;
        }

        let team_count = country_ids_by_reputation.len();
        let group_count = (team_count / 5).max(1).min(10);

        let mut groups: Vec<Vec<u32>> = (0..group_count).map(|_| Vec::new()).collect();

        // Pot-based draw: distribute teams to groups cycling through pots
        for (idx, &country_id) in country_ids_by_reputation.iter().enumerate() {
            let group_idx = idx % group_count;
            groups[group_idx].push(country_id);
        }

        self.qualifying_groups = groups
            .into_iter()
            .enumerate()
            .map(|(idx, team_ids)| {
                let mut group = QualifyingGroup::new(idx as u8, team_ids.clone());
                group.fixtures = schedule::generate_group_qualifying_fixtures(&team_ids, start_year);
                group
            })
            .collect();

        self.phase = CompetitionPhase::Qualifying;

        info!(
            "World Cup {} qualifying draw: {} groups",
            self.cycle_year,
            self.qualifying_groups.len()
        );
    }

    /// Get today's qualifying fixtures across all groups
    pub fn get_todays_qualifying_fixtures(&self, date: NaiveDate) -> Vec<(u8, usize)> {
        let mut fixtures = Vec::new();
        for (group_idx, group) in self.qualifying_groups.iter().enumerate() {
            for (fix_idx, fixture) in group.fixtures.iter().enumerate() {
                if fixture.date == date && fixture.result.is_none() {
                    fixtures.push((group_idx as u8, fix_idx));
                }
            }
        }
        fixtures
    }

    /// Record a qualifying match result
    pub fn record_qualifying_result(
        &mut self,
        group_idx: usize,
        fixture_idx: usize,
        home_score: u8,
        away_score: u8,
    ) {
        if let Some(group) = self.qualifying_groups.get_mut(group_idx) {
            if let Some(fixture) = group.fixtures.get_mut(fixture_idx) {
                let home_id = fixture.home_country_id;
                let away_id = fixture.away_country_id;

                fixture.result = Some(FixtureResult {
                    home_score,
                    away_score,
                });

                group.update_standings(home_id, away_id, home_score, away_score);
            }
        }
    }

    /// Check if qualifying is complete and determine qualified teams
    pub fn check_qualifying_complete(&mut self) {
        if self.phase != CompetitionPhase::Qualifying {
            return;
        }

        let all_complete = self.qualifying_groups.iter().all(|g| g.is_complete());
        if !all_complete {
            return;
        }

        self.qualified_teams.clear();

        // Group winners qualify directly
        for group in &self.qualifying_groups {
            if let Some(winner) = group.winner() {
                self.qualified_teams.push(winner);
            }
        }

        // Best runners-up fill remaining spots
        if self.qualified_teams.len() < EUROPEAN_WC_QUALIFYING_SPOTS {
            let mut runners_up: Vec<(u32, &GroupStanding)> = self
                .qualifying_groups
                .iter()
                .filter_map(|g| {
                    g.standings.get(1).map(|s| (s.country_id, s))
                })
                .collect();

            runners_up.sort_by(|a, b| {
                b.1.points.cmp(&a.1.points)
                    .then_with(|| b.1.goal_difference().cmp(&a.1.goal_difference()))
            });

            let spots_remaining = EUROPEAN_WC_QUALIFYING_SPOTS - self.qualified_teams.len();
            for (country_id, _) in runners_up.into_iter().take(spots_remaining) {
                self.qualified_teams.push(country_id);
            }
        }

        // Cap at 32 teams for the tournament
        self.qualified_teams.truncate(32);

        self.phase = CompetitionPhase::GroupStage;

        info!(
            "World Cup {} qualifying complete: {} teams qualified",
            self.cycle_year,
            self.qualified_teams.len()
        );
    }

    /// Draw tournament groups from qualified teams
    pub fn draw_tournament_groups(&mut self, tournament_year: i32) {
        if self.qualified_teams.is_empty() || self.phase != CompetitionPhase::GroupStage {
            return;
        }

        let teams = &self.qualified_teams;
        let group_count = WC_TOURNAMENT_GROUP_COUNT.min(teams.len() / 2);
        let mut groups: Vec<Vec<u32>> = (0..group_count).map(|_| Vec::new()).collect();

        for (idx, &country_id) in teams.iter().enumerate() {
            let group_idx = idx % group_count;
            groups[group_idx].push(country_id);
        }

        let group_dates = schedule::generate_tournament_group_dates(tournament_year);

        self.tournament_groups = groups
            .into_iter()
            .enumerate()
            .map(|(idx, team_ids)| {
                let mut group = QualifyingGroup::new(idx as u8, team_ids.clone());
                // Generate group stage fixtures with tournament dates
                let round_robin = schedule::generate_round_robin_fixtures(team_ids.len());

                for (_, home_idx, away_idx) in round_robin {
                    // Only single-leg in tournament group stage
                    if home_idx < away_idx {
                        let date_idx = group.fixtures.len().min(group_dates.len().saturating_sub(1));
                        let date = group_dates.get(date_idx).copied().unwrap_or(
                            NaiveDate::from_ymd_opt(tournament_year, 6, 14).unwrap()
                        );

                        group.fixtures.push(GroupFixture {
                            matchday: (group.fixtures.len() + 1) as u8,
                            date,
                            home_country_id: team_ids[home_idx],
                            away_country_id: team_ids[away_idx],
                            result: None,
                        });
                    }
                }

                group
            })
            .collect();

        info!(
            "World Cup {} tournament draw: {} groups of {}",
            self.cycle_year,
            self.tournament_groups.len(),
            WC_TOURNAMENT_TEAMS_PER_GROUP
        );
    }

    /// Get today's tournament group fixtures
    pub fn get_todays_tournament_group_fixtures(&self, date: NaiveDate) -> Vec<(u8, usize)> {
        let mut fixtures = Vec::new();
        for (group_idx, group) in self.tournament_groups.iter().enumerate() {
            for (fix_idx, fixture) in group.fixtures.iter().enumerate() {
                if fixture.date == date && fixture.result.is_none() {
                    fixtures.push((group_idx as u8, fix_idx));
                }
            }
        }
        fixtures
    }

    /// Record a tournament group match result
    pub fn record_tournament_group_result(
        &mut self,
        group_idx: usize,
        fixture_idx: usize,
        home_score: u8,
        away_score: u8,
    ) {
        if let Some(group) = self.tournament_groups.get_mut(group_idx) {
            if let Some(fixture) = group.fixtures.get_mut(fixture_idx) {
                let home_id = fixture.home_country_id;
                let away_id = fixture.away_country_id;

                fixture.result = Some(FixtureResult {
                    home_score,
                    away_score,
                });

                group.update_standings(home_id, away_id, home_score, away_score);
            }
        }
    }

    /// Check if tournament group stage is complete and setup knockout
    pub fn check_tournament_groups_complete(&mut self, tournament_year: i32) {
        if self.phase != CompetitionPhase::GroupStage {
            return;
        }

        let all_complete = self.tournament_groups.iter().all(|g| g.is_complete());
        if !all_complete {
            return;
        }

        // Advance top 2 from each group to knockout
        let mut r16_teams: Vec<u32> = Vec::new();
        for group in &self.tournament_groups {
            if let Some(winner) = group.winner() {
                r16_teams.push(winner);
            }
            if let Some(runner_up) = group.runner_up() {
                r16_teams.push(runner_up);
            }
        }

        // Create R16 bracket: 1A vs 2B, 1B vs 2A, etc.
        let knockout_dates = schedule::generate_tournament_knockout_dates(tournament_year);

        let mut r16 = KnockoutBracket::new(KnockoutRound::RoundOf16);
        let pair_count = r16_teams.len() / 2;
        for i in 0..pair_count {
            let home = r16_teams[i];
            let away = r16_teams[r16_teams.len() - 1 - i];
            let date = knockout_dates.first().copied().unwrap_or(
                NaiveDate::from_ymd_opt(tournament_year, 6, 28).unwrap()
            );
            r16.fixtures.push(KnockoutFixture {
                date,
                home_country_id: home,
                away_country_id: away,
                result: None,
            });
        }

        self.knockout = vec![r16];
        self.phase = CompetitionPhase::Knockout;

        info!(
            "World Cup {} knockout stage: {} teams",
            self.cycle_year,
            r16_teams.len()
        );
    }

    /// Get today's knockout fixtures
    pub fn get_todays_knockout_fixtures(&self, date: NaiveDate) -> Vec<(usize, usize)> {
        let mut fixtures = Vec::new();
        for (bracket_idx, bracket) in self.knockout.iter().enumerate() {
            for (fix_idx, fixture) in bracket.fixtures.iter().enumerate() {
                if fixture.date == date && fixture.result.is_none() {
                    fixtures.push((bracket_idx, fix_idx));
                }
            }
        }
        fixtures
    }

    /// Record a knockout match result
    pub fn record_knockout_result(
        &mut self,
        bracket_idx: usize,
        fixture_idx: usize,
        home_score: u8,
        away_score: u8,
        penalty_winner: Option<u32>,
    ) {
        if let Some(bracket) = self.knockout.get_mut(bracket_idx) {
            if let Some(fixture) = bracket.fixtures.get_mut(fixture_idx) {
                fixture.result = Some(KnockoutResult {
                    home_score,
                    away_score,
                    penalty_winner,
                });
            }
        }
    }

    /// Progress knockout: when current round is complete, create next round
    pub fn progress_knockout(&mut self, tournament_year: i32) {
        let knockout_dates = schedule::generate_tournament_knockout_dates(tournament_year);

        if let Some(current_bracket) = self.knockout.last() {
            if !current_bracket.is_complete() {
                return;
            }

            let winners = current_bracket.winners();
            let current_round = current_bracket.round.clone();

            let next_round = match current_round {
                KnockoutRound::RoundOf16 if winners.len() >= 2 => Some(KnockoutRound::QuarterFinals),
                KnockoutRound::QuarterFinals if winners.len() >= 2 => Some(KnockoutRound::SemiFinals),
                KnockoutRound::SemiFinals if winners.len() >= 2 => Some(KnockoutRound::Final),
                KnockoutRound::Final => {
                    // Tournament complete
                    if let Some(&champion) = winners.first() {
                        self.champion = Some(champion);
                        self.phase = CompetitionPhase::Completed;
                        info!("World Cup {} champion: country_id {}", self.cycle_year, champion);
                    }
                    None
                }
                _ => None,
            };

            if let Some(round) = next_round {
                let date_idx = match round {
                    KnockoutRound::QuarterFinals => 2,
                    KnockoutRound::SemiFinals => 4,
                    KnockoutRound::Final => 6,
                    _ => 0,
                };
                let date = knockout_dates.get(date_idx).copied().unwrap_or(
                    NaiveDate::from_ymd_opt(tournament_year, 7, 10).unwrap()
                );

                let mut next_bracket = KnockoutBracket::new(round);
                let pair_count = winners.len() / 2;
                for i in 0..pair_count {
                    next_bracket.fixtures.push(KnockoutFixture {
                        date,
                        home_country_id: winners[i * 2],
                        away_country_id: winners[i * 2 + 1],
                        result: None,
                    });
                }
                self.knockout.push(next_bracket);
            }
        }
    }

    /// Check if there's activity on a given date
    pub fn has_activity_on(&self, date: NaiveDate) -> bool {
        match self.phase {
            CompetitionPhase::Qualifying => !self.get_todays_qualifying_fixtures(date).is_empty(),
            CompetitionPhase::GroupStage => !self.get_todays_tournament_group_fixtures(date).is_empty(),
            CompetitionPhase::Knockout => !self.get_todays_knockout_fixtures(date).is_empty(),
            _ => false,
        }
    }
}
