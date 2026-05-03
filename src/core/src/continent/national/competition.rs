use chrono::NaiveDate;
use log::info;

use super::config::*;
use super::schedule;

/// Phase of a national team competition cycle
#[derive(Debug, Clone, PartialEq)]
pub enum CompetitionPhase {
    NotStarted,
    Qualifying,
    QualifyingPlayoff,
    GroupStage,
    Knockout,
    Completed,
}

/// A qualifying group for World Cup or European Championship qualifying
#[derive(Debug, Clone)]
pub struct QualifyingGroup {
    pub id: u8,
    pub team_country_ids: Vec<u32>,
    pub standings: Vec<GroupStanding>,
    pub fixtures: Vec<GroupFixture>,
}

impl QualifyingGroup {
    pub fn new(id: u8, team_country_ids: Vec<u32>) -> Self {
        let standings = team_country_ids
            .iter()
            .map(|&country_id| GroupStanding::new(country_id))
            .collect();

        QualifyingGroup {
            id,
            team_country_ids,
            standings,
            fixtures: Vec::new(),
        }
    }

    /// Update standings after a match result
    pub fn update_standings(
        &mut self,
        home_country_id: u32,
        away_country_id: u32,
        home_score: u8,
        away_score: u8,
    ) {
        // Update home team
        if let Some(home) = self
            .standings
            .iter_mut()
            .find(|s| s.country_id == home_country_id)
        {
            home.played += 1;
            home.goals_for += home_score as u16;
            home.goals_against += away_score as u16;
            if home_score > away_score {
                home.won += 1;
                home.points += 3;
            } else if home_score == away_score {
                home.drawn += 1;
                home.points += 1;
            } else {
                home.lost += 1;
            }
        }

        // Update away team
        if let Some(away) = self
            .standings
            .iter_mut()
            .find(|s| s.country_id == away_country_id)
        {
            away.played += 1;
            away.goals_for += away_score as u16;
            away.goals_against += home_score as u16;
            if away_score > home_score {
                away.won += 1;
                away.points += 3;
            } else if away_score == home_score {
                away.drawn += 1;
                away.points += 1;
            } else {
                away.lost += 1;
            }
        }

        // Sort standings: points desc, goal difference desc, goals for desc
        self.standings.sort_by(|a, b| {
            b.points
                .cmp(&a.points)
                .then_with(|| b.goal_difference().cmp(&a.goal_difference()))
                .then_with(|| b.goals_for.cmp(&a.goals_for))
        });
    }

    /// Get the group winner (first place)
    pub fn winner(&self) -> Option<u32> {
        self.standings.first().map(|s| s.country_id)
    }

    /// Get the runner-up (second place)
    pub fn runner_up(&self) -> Option<u32> {
        self.standings.get(1).map(|s| s.country_id)
    }

    /// Check if all fixtures in the group have been played
    pub fn is_complete(&self) -> bool {
        self.fixtures.iter().all(|f| f.result.is_some())
    }
}

/// Standing of a team within a qualifying group
#[derive(Debug, Clone)]
pub struct GroupStanding {
    pub country_id: u32,
    pub played: u8,
    pub won: u8,
    pub drawn: u8,
    pub lost: u8,
    pub goals_for: u16,
    pub goals_against: u16,
    pub points: u8,
}

impl GroupStanding {
    pub fn new(country_id: u32) -> Self {
        GroupStanding {
            country_id,
            played: 0,
            won: 0,
            drawn: 0,
            lost: 0,
            goals_for: 0,
            goals_against: 0,
            points: 0,
        }
    }

    pub fn goal_difference(&self) -> i16 {
        self.goals_for as i16 - self.goals_against as i16
    }
}

/// A single fixture in a qualifying group
#[derive(Debug, Clone)]
pub struct GroupFixture {
    pub matchday: u8,
    pub date: NaiveDate,
    pub home_country_id: u32,
    pub away_country_id: u32,
    pub result: Option<FixtureResult>,
}

/// Result of a group stage or qualifying fixture
#[derive(Debug, Clone)]
pub struct FixtureResult {
    pub home_score: u8,
    pub away_score: u8,
}

/// A knockout bracket round
#[derive(Debug, Clone)]
pub struct KnockoutBracket {
    pub round: KnockoutRound,
    pub fixtures: Vec<KnockoutFixture>,
}

impl KnockoutBracket {
    pub fn new(round: KnockoutRound) -> Self {
        KnockoutBracket {
            round,
            fixtures: Vec::new(),
        }
    }

    pub fn is_complete(&self) -> bool {
        self.fixtures.iter().all(|f| f.result.is_some())
    }

    /// Get the winners of all fixtures in this bracket
    pub fn winners(&self) -> Vec<u32> {
        self.fixtures
            .iter()
            .filter_map(|f| {
                f.result
                    .as_ref()
                    .map(|r| r.winner(f.home_country_id, f.away_country_id))
            })
            .collect()
    }
}

/// Knockout round type
#[derive(Debug, Clone, PartialEq)]
pub enum KnockoutRound {
    RoundOf16,
    QuarterFinals,
    SemiFinals,
    ThirdPlace,
    Final,
}

/// A single knockout fixture
#[derive(Debug, Clone)]
pub struct KnockoutFixture {
    pub date: NaiveDate,
    pub home_country_id: u32,
    pub away_country_id: u32,
    pub result: Option<KnockoutResult>,
}

/// Result of a knockout match, including potential penalty winner
#[derive(Debug, Clone)]
pub struct KnockoutResult {
    pub home_score: u8,
    pub away_score: u8,
    pub penalty_winner: Option<u32>,
}

impl KnockoutResult {
    /// Determine the winner of a knockout match
    pub fn winner(&self, home_country_id: u32, away_country_id: u32) -> u32 {
        if let Some(pw) = self.penalty_winner {
            return pw;
        }
        if self.home_score > self.away_score {
            home_country_id
        } else {
            away_country_id
        }
    }
}

/// Generic national team competition replacing both WorldCupCompetition and EuropeanChampionship.
/// Driven entirely by NationalCompetitionConfig from the database.
#[derive(Debug, Clone)]
pub struct NationalTeamCompetition {
    pub config: NationalCompetitionConfig,
    pub cycle_year: u16,
    pub phase: CompetitionPhase,
    pub qualifying_groups: Vec<QualifyingGroup>,
    pub qualified_teams: Vec<u32>,
    pub tournament_groups: Vec<QualifyingGroup>,
    pub knockout: Vec<KnockoutBracket>,
    pub champion: Option<u32>,
}

impl NationalTeamCompetition {
    pub fn new(config: NationalCompetitionConfig, cycle_year: u16) -> Self {
        NationalTeamCompetition {
            config,
            cycle_year,
            phase: CompetitionPhase::NotStarted,
            qualifying_groups: Vec::new(),
            qualified_teams: Vec::new(),
            tournament_groups: Vec::new(),
            knockout: Vec::new(),
            champion: None,
        }
    }

    pub fn short_name(&self) -> &str {
        &self.config.short_name
    }

    /// Draw qualifying groups from country IDs sorted by reputation (descending)
    pub fn draw_qualifying_groups(
        &mut self,
        country_ids_by_reputation: &[u32],
        start_year: i32,
        zone_config: &QualifyingZoneConfig,
    ) {
        if country_ids_by_reputation.is_empty() {
            return;
        }

        let team_count = country_ids_by_reputation.len();
        let target = zone_config.teams_per_group_target as usize;
        let group_count = (team_count / target)
            .max(1)
            .min(zone_config.max_groups as usize);

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
                group.fixtures = schedule::generate_group_qualifying_fixtures_from_config(
                    &team_ids,
                    start_year,
                    &self.config.schedule,
                );
                group
            })
            .collect();

        self.phase = CompetitionPhase::Qualifying;

        info!(
            "{} {} qualifying draw: {} groups",
            self.config.name,
            self.cycle_year,
            self.qualifying_groups.len()
        );
    }

    /// Check if qualifying is complete and determine qualified teams
    pub fn check_qualifying_complete(&mut self, zone_config: &QualifyingZoneConfig) {
        if self.phase != CompetitionPhase::Qualifying {
            return;
        }

        let all_complete = self.qualifying_groups.iter().all(|g| g.is_complete());
        if !all_complete {
            return;
        }

        self.qualified_teams.clear();

        // Qualify teams based on qualifiers_per_group config
        for group in &self.qualifying_groups {
            for position in &zone_config.qualifiers_per_group {
                match position {
                    QualifyingPosition::Winner => {
                        if let Some(winner) = group.winner() {
                            self.qualified_teams.push(winner);
                        }
                    }
                    QualifyingPosition::RunnerUp => {
                        if let Some(runner_up) = group.runner_up() {
                            self.qualified_teams.push(runner_up);
                        }
                    }
                }
            }
        }

        // Best runners-up fill remaining spots
        if zone_config.best_runners_up > 0 {
            let mut runners_up: Vec<(u32, &GroupStanding)> = self
                .qualifying_groups
                .iter()
                .filter_map(|g| g.standings.get(1).map(|s| (s.country_id, s)))
                .filter(|(id, _)| !self.qualified_teams.contains(id))
                .collect();

            runners_up.sort_by(|a, b| {
                b.1.points
                    .cmp(&a.1.points)
                    .then_with(|| b.1.goal_difference().cmp(&a.1.goal_difference()))
            });

            for (country_id, _) in runners_up
                .into_iter()
                .take(zone_config.best_runners_up as usize)
            {
                self.qualified_teams.push(country_id);
            }
        }

        // Best 3rd-placed teams fill remaining spots
        if zone_config.best_third_placed > 0 {
            let mut third_placed: Vec<(u32, &GroupStanding)> = self
                .qualifying_groups
                .iter()
                .filter_map(|g| g.standings.get(2).map(|s| (s.country_id, s)))
                .collect();

            third_placed.sort_by(|a, b| {
                b.1.points
                    .cmp(&a.1.points)
                    .then_with(|| b.1.goal_difference().cmp(&a.1.goal_difference()))
            });

            for (country_id, _) in third_placed
                .into_iter()
                .take(zone_config.best_third_placed as usize)
            {
                self.qualified_teams.push(country_id);
            }
        }

        // Cap at total tournament spots
        self.qualified_teams.truncate(zone_config.spots as usize);

        // For global scope: qualifying complete, tournament assembled elsewhere
        // For continental scope: transition directly to GroupStage
        if self.config.scope == CompetitionScope::Global {
            self.phase = CompetitionPhase::Completed;
        } else {
            self.phase = CompetitionPhase::GroupStage;
        }

        info!(
            "{} {} qualifying complete: {} teams qualified",
            self.config.name,
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

        // Too few qualifiers to form any group fixtures (e.g. tiny simulation
        // with a single league). Crown the lone qualifier, if any, and finish.
        if teams.len() < 2 {
            self.champion = teams.first().copied();
            self.phase = CompetitionPhase::Completed;
            info!(
                "{} {} tournament cancelled: only {} team qualified",
                self.config.name,
                self.cycle_year,
                teams.len()
            );
            return;
        }

        let group_count = (self.config.tournament.group_count as usize).min(teams.len() / 2);
        let mut groups: Vec<Vec<u32>> = (0..group_count).map(|_| Vec::new()).collect();

        for (idx, &country_id) in teams.iter().enumerate() {
            let group_idx = idx % group_count;
            groups[group_idx].push(country_id);
        }

        // For tournament dates, the start_year is qualifying_start_year, but
        // tournament_group_dates already have year_offset=2, so we need the qualifying start year
        let qualifying_start_year = tournament_year - 2;
        let group_dates = self
            .config
            .schedule
            .generate_tournament_group_dates(qualifying_start_year);

        self.tournament_groups = groups
            .into_iter()
            .enumerate()
            .map(|(idx, team_ids)| {
                let mut group = QualifyingGroup::new(idx as u8, team_ids.clone());

                for i in 0..team_ids.len() {
                    for j in (i + 1)..team_ids.len() {
                        let date_idx = group
                            .fixtures
                            .len()
                            .min(group_dates.len().saturating_sub(1));
                        let date = group_dates
                            .get(date_idx)
                            .copied()
                            .unwrap_or(NaiveDate::from_ymd_opt(tournament_year, 6, 14).unwrap());

                        group.fixtures.push(GroupFixture {
                            matchday: (group.fixtures.len() + 1) as u8,
                            date,
                            home_country_id: team_ids[i],
                            away_country_id: team_ids[j],
                            result: None,
                        });
                    }
                }

                group
            })
            .collect();

        info!(
            "{} {} tournament draw: {} groups",
            self.config.name,
            self.cycle_year,
            self.tournament_groups.len()
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

        // Advance top N from each group
        let mut r16_teams: Vec<u32> = Vec::new();

        for group in &self.tournament_groups {
            for i in 0..(self.config.tournament.advance_per_group as usize) {
                if let Some(standing) = group.standings.get(i) {
                    r16_teams.push(standing.country_id);
                }
            }
        }

        // Best 3rd-placed teams
        if self.config.tournament.best_third_placed > 0 {
            let mut third_placed: Vec<(u32, &GroupStanding)> = self
                .tournament_groups
                .iter()
                .filter_map(|g| g.standings.get(2).map(|s| (s.country_id, s)))
                .collect();

            third_placed.sort_by(|a, b| {
                b.1.points
                    .cmp(&a.1.points)
                    .then_with(|| b.1.goal_difference().cmp(&a.1.goal_difference()))
            });

            for (country_id, _) in third_placed
                .into_iter()
                .take(self.config.tournament.best_third_placed as usize)
            {
                r16_teams.push(country_id);
            }
        }

        let qualifying_start_year = tournament_year - 2;
        let knockout_dates = self
            .config
            .schedule
            .generate_tournament_knockout_dates(qualifying_start_year);

        let mut r16 = KnockoutBracket::new(KnockoutRound::RoundOf16);
        let pair_count = r16_teams.len() / 2;
        for i in 0..pair_count {
            let home = r16_teams[i];
            let away = r16_teams[r16_teams.len() - 1 - i];
            let date = knockout_dates
                .first()
                .copied()
                .unwrap_or(NaiveDate::from_ymd_opt(tournament_year, 6, 28).unwrap());
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
            "{} {} knockout stage: {} teams",
            self.config.name,
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
        let qualifying_start_year = tournament_year - 2;
        let knockout_dates = self
            .config
            .schedule
            .generate_tournament_knockout_dates(qualifying_start_year);

        if let Some(current_bracket) = self.knockout.last() {
            if !current_bracket.is_complete() {
                return;
            }

            let winners = current_bracket.winners();
            let current_round = current_bracket.round.clone();

            let next_round = match current_round {
                KnockoutRound::RoundOf16 if winners.len() >= 2 => {
                    Some(KnockoutRound::QuarterFinals)
                }
                KnockoutRound::QuarterFinals if winners.len() >= 2 => {
                    Some(KnockoutRound::SemiFinals)
                }
                KnockoutRound::SemiFinals if winners.len() >= 2 => Some(KnockoutRound::Final),
                KnockoutRound::Final => {
                    if let Some(&champion) = winners.first() {
                        self.champion = Some(champion);
                        self.phase = CompetitionPhase::Completed;
                        info!(
                            "{} {} champion: country_id {}",
                            self.config.name, self.cycle_year, champion
                        );
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
                let date = knockout_dates
                    .get(date_idx)
                    .copied()
                    .unwrap_or(NaiveDate::from_ymd_opt(tournament_year, 7, 10).unwrap());

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
            CompetitionPhase::GroupStage => {
                !self.get_todays_tournament_group_fixtures(date).is_empty()
            }
            CompetitionPhase::Knockout => !self.get_todays_knockout_fixtures(date).is_empty(),
            _ => false,
        }
    }
}
