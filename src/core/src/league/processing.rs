use crate::Club;
use crate::context::GlobalContext;
use crate::r#match::MatchResult;
use chrono::{Datelike, NaiveDate};
use log::debug;

use super::League;

impl League {
    pub(super) fn process_match_day_results(
        &mut self,
        match_results: &[MatchResult],
        clubs: &[Club],
        ctx: &GlobalContext<'_>,
        current_date: NaiveDate,
    ) {
        self.process_match_results(match_results, clubs, ctx);
        self.table.update_from_results(match_results);

        match_results.iter().for_each(|mr| {
            self.matches
                .push(mr.copy_without_data_positions(), current_date);
        });

        self.update_league_dynamics(match_results, clubs, current_date);
        self.check_milestones_and_events(clubs, current_date);
        self.apply_regulatory_actions(clubs, ctx);
    }

    fn process_match_results(
        &mut self,
        results: &[MatchResult],
        clubs: &[Club],
        ctx: &GlobalContext<'_>,
    ) {
        for result in results {
            self.statistics.process_match_result(result);
            self.regulations.process_disciplinary_actions(result);

            self.dynamics.update_team_streaks(
                result.score.home_team.team_id,
                result.score.away_team.team_id,
                &result.score,
            );

            self.check_manager_pressure(result, clubs, ctx.simulation.date.date());
        }

        self.statistics.update_player_rankings(clubs);
    }

    fn check_manager_pressure(
        &self,
        result: &MatchResult,
        _clubs: &[Club],
        _current_date: NaiveDate,
    ) {
        let home_losing_streak = self
            .dynamics
            .get_team_losing_streak(result.score.home_team.team_id);
        if home_losing_streak > 5 {
            debug!(
                "🔴 Manager under severe pressure at team {}",
                result.score.home_team.team_id
            );
        }

        let away_losing_streak = self
            .dynamics
            .get_team_losing_streak(result.score.away_team.team_id);
        if away_losing_streak > 5 {
            debug!(
                "🔴 Manager under severe pressure at team {}",
                result.score.away_team.team_id
            );
        }
    }

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

        if season_progress > 0.6 {
            self.dynamics.update_title_race(&self.table);
        }

        if season_progress > 0.5 {
            self.dynamics
                .update_relegation_battle(&self.table, total_teams);
        }

        if season_progress > 0.7 {
            self.dynamics.update_european_race(&self.table);
        }

        self.statistics.update_competitive_balance(&self.table);
        self.update_league_reputation(clubs, season_progress);
    }

    fn update_league_reputation(&mut self, _clubs: &[Club], season_progress: f32) {
        if season_progress < 0.1 {
            return;
        }

        let competitive_balance = self.statistics.competitive_balance_index;
        let avg_goals_per_game =
            self.statistics.total_goals as f32 / self.statistics.total_matches.max(1) as f32;

        let mut reputation_change: i16 = 0;

        if competitive_balance > 0.7 {
            reputation_change += 2;
        }

        if avg_goals_per_game > 2.8 {
            reputation_change += 1;
        }

        self.reputation =
            (self.reputation as i32 + reputation_change as i32).clamp(0, 10000) as u16;
    }

    fn check_milestones_and_events(&mut self, _clubs: &[Club], current_date: NaiveDate) {
        self.milestones.check_records(&self.statistics, &self.table);

        let upcoming_matches = self.schedule.get_matches_in_next_days(current_date, 7);
        for match_item in upcoming_matches {
            if self
                .dynamics
                .is_derby(match_item.home_team_id, match_item.away_team_id)
            {
                debug!(
                    "🔥 Derby coming up: Team {} vs Team {}",
                    match_item.home_team_id, match_item.away_team_id
                );
            }
        }

        let matches_played = self.table.rows.first().map(|r| r.played).unwrap_or(0);
        self.milestones
            .check_season_milestones(matches_played, &self.table);
    }

    fn apply_regulatory_actions(&mut self, clubs: &[Club], ctx: &GlobalContext<'_>) {
        for club in clubs {
            if self.regulations.check_ffp_violation(club) {
                debug!("⚠️ FFP violation detected for club: {}", club.name);
                self.regulations
                    .apply_ffp_sanctions(club.id, &mut self.table);
            }
        }

        self.regulations
            .process_pending_cases(ctx.simulation.date.date());
    }

    pub(super) fn process_non_matchday(&mut self, clubs: &[Club], ctx: &GlobalContext<'_>) {
        let current_date = ctx.simulation.date.date();

        if self.is_season_end(current_date) {
            self.process_season_end(clubs, current_date);
        }

        if self.is_winter_break(current_date) {
            self.process_winter_break(clubs);
        }

        if self.is_international_break(current_date) {
            debug!("International break - no league matches");
        }
    }

    fn is_season_end(&self, date: NaiveDate) -> bool {
        let end = &self.settings.season_ending_half;
        date.day() as u8 == end.to_day && date.month() as u8 == end.to_month
    }

    fn is_winter_break(&self, date: NaiveDate) -> bool {
        date.month() == 12 && date.day() >= 20 && date.day() <= 31
    }

    fn is_international_break(&self, date: NaiveDate) -> bool {
        (date.month() == 9 && date.day() >= 4 && date.day() <= 12)
            || (date.month() == 10 && date.day() >= 9 && date.day() <= 17)
            || (date.month() == 11 && date.day() >= 13 && date.day() <= 21)
            || (date.month() == 3 && date.day() >= 20 && date.day() <= 28)
    }

    fn process_season_end(&mut self, _clubs: &[Club], current_date: NaiveDate) {
        debug!("🏆 Season ended for league: {}", self.name);

        let champion_id = self.table.rows.first().map(|r| r.team_id);
        if let Some(champion) = champion_id {
            debug!("🥇 Champions: Team {}", champion);
            self.milestones.record_champion(champion, current_date);
        }

        self.final_table = Some(self.table.rows.clone());

        self.dynamics.reset_for_new_season();
        self.statistics.archive_season_stats();

        self.regulations.suspended_players.clear();
        self.regulations.yellow_card_accumulation.clear();
        self.regulations.pending_cases.clear();
    }

    fn process_winter_break(&mut self, _clubs: &[Club]) {
        debug!("❄️ Winter break for league: {}", self.name);
    }

    #[allow(dead_code)]
    pub(super) fn calculate_matches_remaining(&self, team_id: u32) -> usize {
        self.schedule
            .tours
            .iter()
            .flat_map(|t| &t.items)
            .filter(|item| {
                item.result.is_none()
                    && (item.home_team_id == team_id || item.away_team_id == team_id)
            })
            .count()
    }
}
