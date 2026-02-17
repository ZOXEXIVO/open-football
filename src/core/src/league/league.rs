use crate::context::{GlobalContext, SimulationContext};
use crate::league::{LeagueMatch, LeagueMatchResultResult, LeagueResult, LeagueTable, MatchStorage, Schedule, ScheduleItem};
use crate::r#match::{Match, MatchResult};
use crate::utils::Logging;
use crate::{Club, Team};
use chrono::{Datelike, NaiveDate};
use log::{debug, info, warn};
use rayon::iter::IntoParallelRefMutIterator;
use std::collections::HashMap;

#[derive(Debug)]
pub struct League {
    pub id: u32,
    pub name: String,
    pub slug: String,
    pub country_id: u32,
    pub schedule: Schedule,
    pub table: LeagueTable,
    pub settings: LeagueSettings,
    pub matches: MatchStorage,
    pub reputation: u16,

    // New fields for enhanced simulation
    pub dynamics: LeagueDynamics,
    pub regulations: LeagueRegulations,
    pub statistics: LeagueStatistics,
    pub milestones: LeagueMilestones,
}

impl League {
    pub fn new(
        id: u32,
        name: String,
        slug: String,
        country_id: u32,
        reputation: u16,
        settings: LeagueSettings,
    ) -> Self {
        League {
            id,
            name,
            slug,
            country_id,
            schedule: Schedule::default(),
            table: LeagueTable::default(),
            matches: MatchStorage::new(),
            settings,
            reputation,
            dynamics: LeagueDynamics::new(),
            regulations: LeagueRegulations::new(),
            statistics: LeagueStatistics::new(),
            milestones: LeagueMilestones::new(),
        }
    }

    pub fn simulate(&mut self, clubs: &[Club], ctx: GlobalContext<'_>) -> LeagueResult {
        let league_name = self.name.clone();
        let current_date = ctx.simulation.date.date();

        info!("⚽ Simulating league: {} (Reputation: {})", league_name, self.reputation);

        // Phase 1: Pre-match preparations
        self.prepare_matchday(&ctx, clubs);

        // Phase 2: Table simulation
        let table_result = self.table.simulate(&ctx);

        let league_teams: Vec<u32> = clubs
            .iter()
            .flat_map(|c| c.teams.with_league(self.id))
            .collect();

        // Phase 3: Schedule management
        let mut schedule_result = self.schedule.simulate(
            &self.settings,
            ctx.with_league(self.id, String::from(&self.slug), &league_teams),
        );

        // Phase 4: Match execution with enhanced dynamics
        if schedule_result.is_match_scheduled() {
            let match_results = self.play_scheduled_matches(
                &mut schedule_result.scheduled_matches,
                clubs,
                &ctx,
            );

            self.process_match_day_results(&match_results, clubs, &ctx, current_date);

            return LeagueResult::with_match_result(self.id, table_result, match_results);
        }

        // Phase 5: Off-season or mid-season processing
        self.process_non_matchday(clubs, &ctx);

        LeagueResult::new(self.id, table_result)
    }

    // ========== MATCHDAY PREPARATION ==========

    fn prepare_matchday(&mut self, ctx: &GlobalContext<'_>, clubs: &[Club]) {
        debug!("Preparing matchday for {}", self.name);

        let current_date = ctx.simulation.date.date();
        let day_of_week = current_date.weekday();

        // Update attendance predictions
        self.dynamics.update_attendance_predictions(
            &self.table,
            day_of_week,
            current_date.month(),
        );

        // Check for fixture congestion
        for club in clubs {
            for team in &club.teams.teams {
                if team.league_id == self.id {
                    self.check_fixture_congestion(team, current_date);
                }
            }
        }

        // Update referee assignments (simulated)
        self.dynamics.assign_referees();
    }

    fn check_fixture_congestion(&self, team: &Team, current_date: NaiveDate) {
        let upcoming_matches = self.schedule.get_matches_for_team_in_days(team.id, current_date, 7);
        if upcoming_matches.len() > 2 {
            debug!("⚠️ Fixture congestion for team {}: {} matches in 7 days",
                   team.name, upcoming_matches.len());
        }
    }

    // ========== MATCH EXECUTION ==========

    fn play_scheduled_matches(
        &mut self,
        scheduled_matches: &mut Vec<LeagueMatch>,
        clubs: &[Club],
        ctx: &GlobalContext<'_>,
    ) -> Vec<MatchResult> {
        use rayon::iter::ParallelIterator;

        // Play all matches in parallel
        let match_results: Vec<MatchResult> = scheduled_matches
            .par_iter_mut()
            .map(|scheduled_match| {
                Self::play_single_match_static(
                    scheduled_match,
                    clubs,
                    ctx,
                    &self.dynamics,
                    &self.table,
                )
            })
            .collect();

        // Update momentum sequentially after all matches are played
        for result in &match_results {
            self.dynamics.update_team_momentum_after_match(
                result.home_team_id,
                result.away_team_id,
                result,
            );
        }

        match_results
    }

    fn play_single_match_static(
        scheduled_match: &mut LeagueMatch,
        clubs: &[Club],
        ctx: &GlobalContext<'_>,
        dynamics: &LeagueDynamics,
        table: &LeagueTable,
    ) -> MatchResult {
        let home_team = clubs
            .iter()
            .flat_map(|c| &c.teams.teams)
            .find(|team| team.id == scheduled_match.home_team_id)
            .expect("Home team not found");

        let away_team = clubs
            .iter()
            .flat_map(|c| &c.teams.teams)
            .find(|team| team.id == scheduled_match.away_team_id)
            .expect("Away team not found");

        // Get psychological factors
        let home_momentum = dynamics.get_team_momentum(scheduled_match.home_team_id);
        let away_momentum = dynamics.get_team_momentum(scheduled_match.away_team_id);

        let (home_pressure, away_pressure) = Self::calculate_match_pressures_static(
            home_team,
            away_team,
            ctx.simulation.date.date(),
            dynamics,
            table,
        );

        // Prepare squads with psychological modifiers
        let mut home_squad = home_team.get_enhanced_match_squad();
        let mut away_squad = away_team.get_enhanced_match_squad();

        Self::apply_psychological_factors_static(&mut home_squad, home_momentum, home_pressure);
        Self::apply_psychological_factors_static(&mut away_squad, away_momentum, away_pressure);

        // Create and play match
        let match_to_play = Match::make(
            scheduled_match.id.clone(),
            scheduled_match.league_id,
            &scheduled_match.league_slug,
            home_squad,
            away_squad,
        );

        let message = &format!(
            "play match: {} vs {} (Momentum: {:.1} vs {:.1})",
            &match_to_play.home_squad.team_name,
            &match_to_play.away_squad.team_name,
            home_momentum,
            away_momentum
        );

        let match_result = Logging::estimate_result(|| match_to_play.play(), message);

        // Update match result in schedule
        scheduled_match.result = Some(LeagueMatchResultResult::from_score(&match_result.score));

        match_result
    }

    #[allow(dead_code)]
    fn get_team_momentums(&self, home_id: u32, away_id: u32) -> (f32, f32) {
        let home = self.dynamics.get_team_momentum(home_id);
        let away = self.dynamics.get_team_momentum(away_id);
        (home, away)
    }

    #[allow(dead_code)]
    fn calculate_match_pressures(
        &self,
        home_team: &Team,
        away_team: &Team,
        current_date: NaiveDate,
    ) -> (f32, f32) {
        let home = self.calculate_match_pressure(home_team, &self.table, current_date);
        let away = self.calculate_match_pressure(away_team, &self.table, current_date);
        (home, away)
    }

    #[allow(dead_code)]
    fn calculate_match_pressure(
        &self,
        team: &Team,
        table: &LeagueTable,
        _current_date: NaiveDate,
    ) -> f32 {
        let position = table.rows.iter().position(|r| r.team_id == team.id).unwrap_or(0);
        let total_teams = table.rows.len();
        let matches_remaining = self.calculate_matches_remaining(team.id);

        let mut pressure: f32 = 0.5; // Base pressure

        // Title race pressure
        if position < 3 && matches_remaining < 10 {
            pressure += 0.3;
        }

        // Relegation battle pressure
        if position >= total_teams - 3 && matches_remaining < 10 {
            pressure += 0.4;
        }

        // Manager under pressure
        let losing_streak = self.dynamics.get_team_losing_streak(team.id);
        if losing_streak > 3 {
            pressure += 0.2;
        }

        pressure.min(1.0)
    }

    #[allow(dead_code)]
    fn apply_psychological_factors(
        &self,
        squad: &mut crate::r#match::MatchSquad,
        momentum: f32,
        pressure: f32,
    ) {
        debug!("Team {} - Momentum: {:.2}, Pressure: {:.2}",
               squad.team_name, momentum, pressure);
    }

    fn apply_psychological_factors_static(
        squad: &mut crate::r#match::MatchSquad,
        momentum: f32,
        pressure: f32,
    ) {
        debug!("Team {} - Momentum: {:.2}, Pressure: {:.2}",
               squad.team_name, momentum, pressure);
    }

    fn calculate_match_pressures_static(
        home_team: &Team,
        away_team: &Team,
        current_date: NaiveDate,
        dynamics: &LeagueDynamics,
        table: &LeagueTable,
    ) -> (f32, f32) {
        let home = Self::calculate_match_pressure_static(home_team, table, current_date, dynamics);
        let away = Self::calculate_match_pressure_static(away_team, table, current_date, dynamics);
        (home, away)
    }

    fn calculate_match_pressure_static(
        team: &Team,
        table: &LeagueTable,
        _current_date: NaiveDate,
        dynamics: &LeagueDynamics,
    ) -> f32 {
        let position = table.rows.iter().position(|r| r.team_id == team.id).unwrap_or(0);
        let total_teams = table.rows.len();

        let mut pressure: f32 = 0.5; // Base pressure

        // Title race pressure
        if position < 3 {
            pressure += 0.3;
        }

        // Relegation battle pressure
        if position >= total_teams - 3 {
            pressure += 0.4;
        }

        // Manager under pressure
        let losing_streak = dynamics.get_team_losing_streak(team.id);
        if losing_streak > 3 {
            pressure += 0.2;
        }

        pressure.min(1.0)
    }

    #[allow(dead_code)]
    fn calculate_matches_remaining(&self, team_id: u32) -> usize {
        self.schedule.tours.iter()
            .flat_map(|t| &t.items)
            .filter(|item| item.result.is_none() &&
                (item.home_team_id == team_id || item.away_team_id == team_id))
            .count()
    }

    // ========== POST-MATCH PROCESSING ==========

    fn process_match_day_results(
        &mut self,
        match_results: &[MatchResult],
        clubs: &[Club],
        ctx: &GlobalContext<'_>,
        current_date: NaiveDate,
    ) {
        // Update match results and statistics
        self.process_match_results(match_results, clubs, ctx);

        // Update table standings
        self.table.update_from_results(match_results);

        // Store match results
        match_results.iter().for_each(|mr| {
            self.matches.push(mr.copy_without_data_positions());
        });

        // Update league dynamics
        self.update_league_dynamics(match_results, clubs, current_date);

        // Check for milestones
        self.check_milestones_and_events(clubs, current_date);

        // Apply regulatory actions
        self.apply_regulatory_actions(clubs, ctx);
    }

    fn process_match_results(
        &mut self,
        results: &[MatchResult],
        clubs: &[Club],
        ctx: &GlobalContext<'_>,
    ) {
        for result in results {
            // Update statistics
            self.statistics.process_match_result(result);

            // Process disciplinary actions
            self.regulations.process_disciplinary_actions(result);

            // Update team streaks
            self.dynamics.update_team_streaks(
                result.score.home_team.team_id,
                result.score.away_team.team_id,
                &result.score,
            );

            // Check manager pressure
            self.check_manager_pressure(result, clubs, ctx.simulation.date.date());
        }

        // Update player rankings
        self.statistics.update_player_rankings(clubs);
    }

    fn check_manager_pressure(
        &self,
        result: &MatchResult,
        _clubs: &[Club],
        _current_date: NaiveDate,
    ) {
        // Check home team
        let home_losing_streak = self.dynamics.get_team_losing_streak(result.score.home_team.team_id);
        if home_losing_streak > 5 {
            warn!("🔴 Manager under severe pressure at team {}", result.score.home_team.team_id);
        }

        // Check away team
        let away_losing_streak = self.dynamics.get_team_losing_streak(result.score.away_team.team_id);
        if away_losing_streak > 5 {
            warn!("🔴 Manager under severe pressure at team {}", result.score.away_team.team_id);
        }
    }

    // ========== LEAGUE DYNAMICS ==========

    fn update_league_dynamics(
        &mut self,
        _results: &[MatchResult],
        clubs: &[Club],
        _current_date: NaiveDate,
    ) {
        let total_teams = self.table.rows.len();
        let matches_played = self.table.rows.first().map(|r| r.played).unwrap_or(0);
        let total_matches = (total_teams - 1) * 2;
        let season_progress = matches_played as f32 / total_matches as f32;

        // Update title race dynamics
        if season_progress > 0.6 {
            self.dynamics.update_title_race(&self.table);
        }

        // Update relegation battle
        if season_progress > 0.5 {
            self.dynamics.update_relegation_battle(&self.table, total_teams);
        }

        // Update European qualification race
        if season_progress > 0.7 {
            self.dynamics.update_european_race(&self.table);
        }

        // Calculate competitive balance
        self.statistics.update_competitive_balance(&self.table);

        // Update league reputation
        self.update_league_reputation(clubs, season_progress);
    }

    fn update_league_reputation(&mut self, _clubs: &[Club], season_progress: f32) {
        if season_progress < 0.1 {
            return; // Too early in season
        }

        let competitive_balance = self.statistics.competitive_balance_index;
        let avg_goals_per_game = self.statistics.total_goals as f32 /
            self.statistics.total_matches.max(1) as f32;

        let mut reputation_change: i16 = 0;

        if competitive_balance > 0.7 {
            reputation_change += 2; // High competition
        }

        if avg_goals_per_game > 2.8 {
            reputation_change += 1; // Entertaining matches
        }

        self.reputation = (self.reputation as i16 + reputation_change).clamp(0, 1000) as u16;
    }

    // ========== MILESTONES & EVENTS ==========

    fn check_milestones_and_events(&mut self, _clubs: &[Club], current_date: NaiveDate) {
        // Check for record-breaking performances
        self.milestones.check_records(&self.statistics, &self.table);

        // Check for special derbies or rivalry matches
        let upcoming_matches = self.schedule.get_matches_in_next_days(current_date, 7);
        for match_item in upcoming_matches {
            if self.dynamics.is_derby(match_item.home_team_id, match_item.away_team_id) {
                info!("🔥 Derby coming up: Team {} vs Team {}",
                      match_item.home_team_id, match_item.away_team_id);
            }
        }

        // Season milestones
        let matches_played = self.table.rows.first().map(|r| r.played).unwrap_or(0);
        self.milestones.check_season_milestones(matches_played, &self.table);
    }

    // ========== REGULATORY ACTIONS ==========

    fn apply_regulatory_actions(&mut self, clubs: &[Club], ctx: &GlobalContext<'_>) {
        // Check Financial Fair Play violations
        for club in clubs {
            if self.regulations.check_ffp_violation(club) {
                warn!("⚠️ FFP violation detected for club: {}", club.name);
                self.regulations.apply_ffp_sanctions(club.id, &mut self.table);
            }
        }

        // Process pending disciplinary cases
        self.regulations.process_pending_cases(ctx.simulation.date.date());
    }

    // ========== NON-MATCHDAY PROCESSING ==========

    fn process_non_matchday(&mut self, clubs: &[Club], ctx: &GlobalContext<'_>) {
        let current_date = ctx.simulation.date.date();

        // End of season processing
        if self.is_season_end(current_date) {
            self.process_season_end(clubs);
        }

        // Mid-season break
        if self.is_winter_break(current_date) {
            self.process_winter_break(clubs);
        }

        // International break
        if self.is_international_break(current_date) {
            debug!("International break - no league matches");
        }
    }

    fn is_season_end(&self, date: NaiveDate) -> bool {
        date.month() == 5 && date.day() >= 25
    }

    fn is_winter_break(&self, date: NaiveDate) -> bool {
        date.month() == 12 && date.day() >= 20 && date.day() <= 31
    }

    fn is_international_break(&self, date: NaiveDate) -> bool {
        (date.month() == 9 && date.day() >= 4 && date.day() <= 12) ||
            (date.month() == 10 && date.day() >= 9 && date.day() <= 17) ||
            (date.month() == 11 && date.day() >= 13 && date.day() <= 21) ||
            (date.month() == 3 && date.day() >= 20 && date.day() <= 28)
    }

    fn process_season_end(&mut self, _clubs: &[Club]) {
        info!("🏆 Season ended for league: {}", self.name);

        let champion_id = self.table.rows.first().map(|r| r.team_id);
        if let Some(champion) = champion_id {
            info!("🥇 Champions: Team {}", champion);
            self.milestones.record_champion(champion);
        }

        self.dynamics.reset_for_new_season();
        self.statistics.archive_season_stats();
    }

    fn process_winter_break(&mut self, _clubs: &[Club]) {
        debug!("❄️ Winter break for league: {}", self.name);
    }

    // ========== UTILITY METHODS ==========

    #[allow(dead_code)]
    fn get_team<'c>(&self, clubs: &'c [Club], id: u32) -> &'c Team {
        clubs
            .iter()
            .flat_map(|c| &c.teams.teams)
            .find(|team| team.id == id)
            .unwrap()
    }
}

// Supporting structures for enhanced simulation

#[derive(Debug)]
pub struct LeagueDynamics {
    pub team_momentum: HashMap<u32, f32>,
    pub team_streaks: HashMap<u32, TeamStreak>,
    pub title_race: TitleRace,
    pub relegation_battle: RelegationBattle,
    pub european_race: EuropeanRace,
    pub rivalries: Vec<(u32, u32)>,
    pub attendance_multiplier: f32,
}

impl LeagueDynamics {
    pub fn new() -> Self {
        LeagueDynamics {
            team_momentum: HashMap::new(),
            team_streaks: HashMap::new(),
            title_race: TitleRace::default(),
            relegation_battle: RelegationBattle::default(),
            european_race: EuropeanRace::default(),
            rivalries: Vec::new(),
            attendance_multiplier: 1.0,
        }
    }

    pub fn get_team_momentum(&self, team_id: u32) -> f32 {
        *self.team_momentum.get(&team_id).unwrap_or(&0.5)
    }

    pub fn update_team_momentum_after_match(
        &mut self,
        home_id: u32,
        away_id: u32,
        result: &MatchResult,
    ) {
        let home_won = result.score.home_team.get() > result.score.away_team.get();
        let draw = result.score.home_team.get() == result.score.away_team.get();

        // Read current values (or defaults)
        let home_val = *self.team_momentum.entry(home_id).or_insert(0.5);
        let away_val = *self.team_momentum.entry(away_id).or_insert(0.5);

        // Compute new values
        let (new_home, new_away) = if home_won {
            (
                (home_val * 0.8 + 0.3).min(1.0),
                (away_val * 0.8 - 0.1).max(0.0),
            )
        } else if draw {
            (
                (home_val * 0.9 + 0.05).min(1.0),
                (away_val * 0.9 + 0.05).min(1.0),
            )
        } else {
            (
                (home_val * 0.8 - 0.1).max(0.0),
                (away_val * 0.8 + 0.3).min(1.0),
            )
        };

        // Write back
        self.team_momentum.insert(home_id, new_home);
        self.team_momentum.insert(away_id, new_away);
    }

    pub fn update_team_streaks(&mut self, home_id: u32, away_id: u32, score: &crate::r#match::Score) {
        let home_won = score.home_team.get() > score.away_team.get();
        let draw = score.home_team.get() == score.away_team.get();

        // Update home team streak
        let home_streak = self.team_streaks.entry(home_id).or_insert(TeamStreak::default());
        if home_won {
            home_streak.winning_streak += 1;
            home_streak.unbeaten_streak += 1;
            home_streak.losing_streak = 0;
        } else if draw {
            home_streak.unbeaten_streak += 1;
            home_streak.winning_streak = 0;
            home_streak.losing_streak = 0;
        } else {
            home_streak.losing_streak += 1;
            home_streak.winning_streak = 0;
            home_streak.unbeaten_streak = 0;
        }

        // Update away team streak
        let away_streak = self.team_streaks.entry(away_id).or_insert(TeamStreak::default());
        if !home_won && !draw {
            away_streak.winning_streak += 1;
            away_streak.unbeaten_streak += 1;
            away_streak.losing_streak = 0;
        } else if draw {
            away_streak.unbeaten_streak += 1;
            away_streak.winning_streak = 0;
            away_streak.losing_streak = 0;
        } else {
            away_streak.losing_streak += 1;
            away_streak.winning_streak = 0;
            away_streak.unbeaten_streak = 0;
        }
    }

    pub fn get_team_losing_streak(&self, team_id: u32) -> u8 {
        self.team_streaks.get(&team_id).map(|s| s.losing_streak).unwrap_or(0)
    }

    pub fn update_title_race(&mut self, table: &LeagueTable) {
        if table.rows.len() < 2 { return; }

        let leader_points = table.rows[0].points;
        let second_points = table.rows[1].points;

        self.title_race.leader_id = table.rows[0].team_id;
        self.title_race.gap_to_second = (leader_points - second_points) as i8;
        self.title_race.contenders = table.rows.iter()
            .take(5)
            .filter(|r| (leader_points - r.points) <= 9)
            .map(|r| r.team_id)
            .collect();
    }

    pub fn update_relegation_battle(&mut self, table: &LeagueTable, total_teams: usize) {
        if total_teams < 4 { return; }

        let relegation_zone_start = total_teams - 3;
        self.relegation_battle.teams_in_danger = table.rows.iter()
            .skip(relegation_zone_start - 2)
            .map(|r| r.team_id)
            .collect();
    }

    pub fn update_european_race(&mut self, table: &LeagueTable) {
        // Top 4-6 teams typically qualify for European competitions
        self.european_race.teams_in_contention = table.rows.iter()
            .take(8)
            .map(|r| r.team_id)
            .collect();
    }

    pub fn is_derby(&self, team1: u32, team2: u32) -> bool {
        self.rivalries.iter().any(|(a, b)|
            (*a == team1 && *b == team2) || (*a == team2 && *b == team1)
        )
    }

    pub fn update_attendance_predictions(
        &mut self,
        table: &LeagueTable,
        day_of_week: chrono::Weekday,
        month: u32,
    ) {
        self.attendance_multiplier = 1.0;

        // Weekend matches have higher attendance
        if day_of_week == chrono::Weekday::Sat || day_of_week == chrono::Weekday::Sun {
            self.attendance_multiplier *= 1.2;
        }

        // Summer months lower attendance
        if month >= 6 && month <= 8 {
            self.attendance_multiplier *= 0.9;
        }

        // End of season crucial matches
        if table.rows.first().map(|r| r.played).unwrap_or(0) > 30 {
            self.attendance_multiplier *= 1.3;
        }
    }

    pub fn assign_referees(&mut self) {
        // Simulated referee assignment
        debug!("Referees assigned for upcoming matches");
    }

    pub fn reset_for_new_season(&mut self) {
        self.team_momentum.clear();
        self.team_streaks.clear();
        self.title_race = TitleRace::default();
        self.relegation_battle = RelegationBattle::default();
        self.european_race = EuropeanRace::default();
    }
}

#[derive(Debug, Default)]
pub struct TeamStreak {
    pub winning_streak: u8,
    pub losing_streak: u8,
    pub unbeaten_streak: u8,
}

#[derive(Debug, Default)]
pub struct TitleRace {
    pub leader_id: u32,
    pub gap_to_second: i8,
    pub contenders: Vec<u32>,
}

#[derive(Debug, Default)]
pub struct RelegationBattle {
    pub teams_in_danger: Vec<u32>,
}

#[derive(Debug, Default)]
pub struct EuropeanRace {
    pub teams_in_contention: Vec<u32>,
}

#[derive(Debug)]
pub struct LeagueRegulations {
    pub suspended_players: HashMap<u32, u8>, // player_id -> matches remaining
    pub yellow_card_accumulation: HashMap<u32, u8>, // player_id -> yellow cards
    pub ffp_violations: Vec<FFPViolation>,
    pub pending_cases: Vec<DisciplinaryCase>,
}

impl LeagueRegulations {
    pub fn new() -> Self {
        LeagueRegulations {
            suspended_players: HashMap::new(),
            yellow_card_accumulation: HashMap::new(),
            ffp_violations: Vec::new(),
            pending_cases: Vec::new(),
        }
    }

    pub fn process_disciplinary_actions(&mut self, _result: &MatchResult) {
        // Process cards and suspensions from match
        // This would need match details with card information
    }

    pub fn check_ffp_violation(&self, club: &Club) -> bool {
        // Check if club violates Financial Fair Play
        let deficit = club.finance.balance.outcome - club.finance.balance.income;
        deficit > 30_000_000 // Simplified FFP check
    }

    pub fn apply_ffp_sanctions(&mut self, club_id: u32, table: &mut LeagueTable) {
        self.ffp_violations.push(FFPViolation {
            club_id,
            violation_type: FFPViolationType::ExcessiveDeficit,
            sanction: FFPSanction::PointDeduction(6),
        });

        // Apply point deduction
        if let Some(row) = table.rows.iter_mut().find(|r| r.team_id == club_id) {
            row.points = row.points.saturating_sub(6);
        }
    }

    pub fn process_pending_cases(&mut self, current_date: NaiveDate) {
        self.pending_cases.retain(|case| case.hearing_date > current_date);
    }
}

#[derive(Debug)]
pub struct FFPViolation {
    pub club_id: u32,
    pub violation_type: FFPViolationType,
    pub sanction: FFPSanction,
}

#[derive(Debug)]
pub enum FFPViolationType {
    ExcessiveDeficit,
    UnpaidDebts,
    FalseAccounting,
}

#[derive(Debug)]
pub enum FFPSanction {
    Warning,
    Fine(u32),
    PointDeduction(u8),
    TransferBan,
}

#[derive(Debug)]
pub struct DisciplinaryCase {
    pub player_id: u32,
    pub incident_type: String,
    pub hearing_date: NaiveDate,
}

#[derive(Debug)]
pub struct LeagueStatistics {
    pub total_goals: u32,
    pub total_matches: u32,
    pub top_scorer: Option<(u32, u16)>, // player_id, goals
    pub top_assists: Option<(u32, u16)>, // player_id, assists
    pub clean_sheets: HashMap<u32, u16>, // goalkeeper_id, clean sheets
    pub competitive_balance_index: f32,
    pub average_attendance: u32,
    pub highest_scoring_match: Option<(u32, u32, u8, u8)>, // home_id, away_id, home_score, away_score
    pub biggest_win: Option<(u32, u32, u8)>, // winner_id, loser_id, goal_difference
    pub longest_unbeaten_run: Option<(u32, u8)>, // team_id, matches
}

impl LeagueStatistics {
    pub fn new() -> Self {
        LeagueStatistics {
            total_goals: 0,
            total_matches: 0,
            top_scorer: None,
            top_assists: None,
            clean_sheets: HashMap::new(),
            competitive_balance_index: 1.0,
            average_attendance: 0,
            highest_scoring_match: None,
            biggest_win: None,
            longest_unbeaten_run: None,
        }
    }

    pub fn process_match_result(&mut self, result: &MatchResult) {
        let home_goals = result.score.home_team.get();
        let away_goals = result.score.away_team.get();

        self.total_goals += (home_goals + away_goals) as u32;
        self.total_matches += 1;

        // Update highest scoring match
        let total_in_match = home_goals + away_goals;
        if let Some((_, _, _, current_high)) = self.highest_scoring_match {
            if total_in_match > current_high {
                self.highest_scoring_match = Some((
                    result.score.home_team.team_id,
                    result.score.away_team.team_id,
                    home_goals,
                    away_goals
                ));
            }
        } else {
            self.highest_scoring_match = Some((
                result.score.home_team.team_id,
                result.score.away_team.team_id,
                home_goals,
                away_goals
            ));
        }

        // Update biggest win
        let goal_diff = (home_goals as i8 - away_goals as i8).abs() as u8;
        if goal_diff > 0 {
            if let Some((_, _, current_biggest)) = self.biggest_win {
                if goal_diff > current_biggest {
                    let (winner, loser) = if home_goals > away_goals {
                        (result.score.home_team.team_id, result.score.away_team.team_id)
                    } else {
                        (result.score.away_team.team_id, result.score.home_team.team_id)
                    };
                    self.biggest_win = Some((winner, loser, goal_diff));
                }
            } else {
                let (winner, loser) = if home_goals > away_goals {
                    (result.score.home_team.team_id, result.score.away_team.team_id)
                } else {
                    (result.score.away_team.team_id, result.score.home_team.team_id)
                };
                self.biggest_win = Some((winner, loser, goal_diff));
            }
        }
    }

    pub fn update_player_rankings(&mut self, clubs: &[Club]) {
        let mut scorer_stats: HashMap<u32, u16> = HashMap::new();
        let mut assist_stats: HashMap<u32, u16> = HashMap::new();

        // Collect all player statistics
        for club in clubs {
            for team in &club.teams.teams {
                for player in &team.players.players {
                    if player.statistics.goals > 0 {
                        scorer_stats.insert(player.id, player.statistics.goals);
                    }
                    if player.statistics.assists > 0 {
                        assist_stats.insert(player.id, player.statistics.assists);
                    }

                    // Track clean sheets for goalkeepers
                    if player.positions.is_goalkeeper() && player.statistics.played > 0 {
                        // Simplified clean sheet tracking
                        self.clean_sheets.insert(player.id, 0);
                    }
                }
            }
        }

        // Find top scorer
        self.top_scorer = scorer_stats.iter()
            .max_by_key(|(_, goals)| *goals)
            .map(|(id, goals)| (*id, *goals));

        // Find top assists
        self.top_assists = assist_stats.iter()
            .max_by_key(|(_, assists)| *assists)
            .map(|(id, assists)| (*id, *assists));
    }

    pub fn update_competitive_balance(&mut self, table: &LeagueTable) {
        if table.rows.len() < 2 {
            self.competitive_balance_index = 1.0;
            return;
        }

        // Calculate standard deviation of points
        let mean_points = table.rows.iter().map(|r| r.points as f32).sum::<f32>()
            / table.rows.len() as f32;

        let variance = table.rows.iter()
            .map(|r| {
                let diff = r.points as f32 - mean_points;
                diff * diff
            })
            .sum::<f32>() / table.rows.len() as f32;

        let std_dev = variance.sqrt();

        // Lower standard deviation means better competitive balance
        // Normalize to 0-1 scale where 1 is perfect balance
        self.competitive_balance_index = 1.0 / (1.0 + std_dev / 10.0);
    }

    pub fn archive_season_stats(&mut self) {
        // Archive current season statistics
        info!("📊 Season Statistics Archived:");
        info!("  Total Goals: {}", self.total_goals);
        info!("  Total Matches: {}", self.total_matches);
        info!("  Goals per Match: {:.2}",
              self.total_goals as f32 / self.total_matches.max(1) as f32);
        info!("  Competitive Balance: {:.2}", self.competitive_balance_index);

        if let Some((player_id, goals)) = self.top_scorer {
            info!("  Top Scorer: Player {} with {} goals", player_id, goals);
        }

        // Reset for new season
        self.total_goals = 0;
        self.total_matches = 0;
        self.top_scorer = None;
        self.top_assists = None;
        self.clean_sheets.clear();
        self.highest_scoring_match = None;
        self.biggest_win = None;
        self.longest_unbeaten_run = None;
    }
}

#[derive(Debug)]
pub struct LeagueMilestones {
    pub all_time_records: AllTimeRecords,
    pub season_milestones: Vec<Milestone>,
    pub historic_champions: Vec<(u16, u32)>, // year, team_id
}

impl LeagueMilestones {
    pub fn new() -> Self {
        LeagueMilestones {
            all_time_records: AllTimeRecords::default(),
            season_milestones: Vec::new(),
            historic_champions: Vec::new(),
        }
    }

    pub fn check_records(&mut self, stats: &LeagueStatistics, table: &LeagueTable) {
        // Check for points records
        if let Some(leader) = table.rows.first() {
            if leader.points > self.all_time_records.most_points_in_season.1 {
                info!("📊 NEW RECORD! Team {} has {} points!", leader.team_id, leader.points);
                self.all_time_records.most_points_in_season = (leader.team_id, leader.points);
            }

            // Goals scored record
            if leader.goal_scored > self.all_time_records.most_goals_in_season.1 {
                info!("⚽ NEW RECORD! Team {} has scored {} goals!",
                      leader.team_id, leader.goal_scored);
                self.all_time_records.most_goals_in_season =
                    (leader.team_id, leader.goal_scored);
            }
        }

        // Check for individual records
        if let Some((player_id, goals)) = stats.top_scorer {
            if goals > self.all_time_records.most_goals_by_player.1 {
                info!("🎯 NEW RECORD! Player {} has scored {} goals!", player_id, goals);
                self.all_time_records.most_goals_by_player = (player_id, goals);
            }
        }
    }

    pub fn check_season_milestones(&mut self, matches_played: u8, table: &LeagueTable) {
        // Check for early title win
        let total_matches = 38; // Standard league
        let matches_remaining = total_matches - matches_played;

        if table.rows.len() >= 2 {
            let leader = &table.rows[0];
            let second = &table.rows[1];
            let max_possible_points_second = second.points + (matches_remaining * 3);

            if leader.points > max_possible_points_second {
                let milestone = Milestone {
                    milestone_type: MilestoneType::TitleWon,
                    team_id: leader.team_id,
                    description: format!("Title won with {} matches to spare!", matches_remaining),
                    matches_played,
                };
                self.season_milestones.push(milestone);
                info!("🏆 {} wins the title with {} matches remaining!",
                      leader.team_id, matches_remaining);
            }
        }

        // Check for unbeaten runs
        for row in &table.rows {
            if row.lost == 0 && matches_played >= 10 {
                let milestone = Milestone {
                    milestone_type: MilestoneType::UnbeatenRun,
                    team_id: row.team_id,
                    description: format!("Unbeaten in {} matches", matches_played),
                    matches_played,
                };

                if !self.season_milestones.iter().any(|m|
                    m.milestone_type == MilestoneType::UnbeatenRun &&
                        m.team_id == row.team_id
                ) {
                    self.season_milestones.push(milestone);
                    info!("💪 Team {} is unbeaten after {} matches!", row.team_id, matches_played);
                }
            }
        }
    }

    pub fn record_champion(&mut self, team_id: u32) {
        let year = chrono::Local::now().year() as u16;
        self.historic_champions.push((year, team_id));

        // Check for consecutive titles
        let consecutive_titles = self.historic_champions.iter()
            .rev()
            .take_while(|(_, id)| *id == team_id)
            .count();

        if consecutive_titles >= 3 {
            info!("👑 Dynasty! Team {} wins {} consecutive titles!",
                  team_id, consecutive_titles);
        }
    }
}

#[derive(Debug, Default)]
pub struct AllTimeRecords {
    pub most_points_in_season: (u32, u8), // team_id, points
    pub most_goals_in_season: (u32, i32), // team_id, goals
    pub fewest_goals_conceded: (u32, i32), // team_id, goals
    pub most_goals_by_player: (u32, u16), // player_id, goals
    pub longest_winning_streak: (u32, u8), // team_id, matches
    pub longest_unbeaten_streak: (u32, u8), // team_id, matches
}

#[derive(Debug)]
pub struct Milestone {
    pub milestone_type: MilestoneType,
    pub team_id: u32,
    pub description: String,
    pub matches_played: u8,
}

#[derive(Debug, PartialEq)]
pub enum MilestoneType {
    TitleWon,
    RelegationConfirmed,
    UnbeatenRun,
    WinningStreak,
    GoalRecord,
    PointsRecord,
}

// Extension to Schedule for enhanced functionality
impl Schedule {
    pub fn get_matches_in_next_days(&self, from_date: NaiveDate, days: i64) -> Vec<&ScheduleItem> {
        let end_date = from_date + chrono::Duration::days(days);

        self.tours.iter()
            .flat_map(|t| &t.items)
            .filter(|item| {
                let item_date = item.date.date();
                item_date >= from_date && item_date <= end_date && item.result.is_none()
            })
            .collect()
    }

    pub fn get_matches_for_team_in_days(&self, team_id: u32, from_date: NaiveDate, days: i64) -> Vec<&ScheduleItem> {
        let end_date = from_date + chrono::Duration::days(days);

        self.tours.iter()
            .flat_map(|t| &t.items)
            .filter(|item| {
                let item_date = item.date.date();
                (item.home_team_id == team_id || item.away_team_id == team_id) &&
                    item_date >= from_date &&
                    item_date <= end_date &&
                    item.result.is_none()
            })
            .collect()
    }
}

#[derive(Debug)]
pub struct DayMonthPeriod {
    pub from_day: u8,
    pub from_month: u8,

    pub to_day: u8,
    pub to_month: u8,
}

impl DayMonthPeriod {
    pub fn new(from_day: u8, from_month: u8, to_day: u8, to_month: u8) -> Self {
        DayMonthPeriod {
            from_day,
            from_month,
            to_day,
            to_month,
        }
    }
}

#[derive(Debug)]
pub struct LeagueSettings {
    pub season_starting_half: DayMonthPeriod,
    pub season_ending_half: DayMonthPeriod,
}

impl LeagueSettings {
    pub fn is_time_for_new_schedule(&self, context: &SimulationContext) -> bool {
        let season_starting_date = &self.season_starting_half;

        let date = context.date.date();

        (NaiveDate::day(&date) as u8) == season_starting_date.from_day
            && (date.month() as u8) == season_starting_date.from_month
    }
}