use crate::Club;
use crate::context::GlobalContext;
use crate::league::awards::{
    AwardAggregator, MonthlyAwardSelector, SeasonAwardSelector, SeasonAwardsSnapshot,
    TeamOfTheWeekSelector,
};
use crate::r#match::MatchResult;
use crate::utils::DateUtils;
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

        self.statistics.update_player_rankings(self.id, clubs);
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

    fn process_season_end(&mut self, clubs: &[Club], current_date: NaiveDate) {
        debug!("🏆 Season ended for league: {}", self.name);

        let champion_id = self.table.rows.first().map(|r| r.team_id);
        if let Some(champion) = champion_id {
            debug!("🥇 Champions: Team {}", champion);
            self.milestones.record_champion(champion, current_date);
        }

        // Snapshot season awards BEFORE statistics is archived. The
        // simulator's `SeasonAwardsTick` drains this on the same
        // simulation tick to fire player events while the data is fresh.
        let snapshot = self.snapshot_season_awards(clubs, current_date);
        self.awards.pending_season_awards = Some(snapshot);

        self.final_table = Some(self.table.rows.clone());

        self.dynamics.reset_for_new_season();
        self.statistics.archive_season_stats();

        self.regulations.suspended_players.clear();
        self.regulations.yellow_card_accumulation.clear();
        self.regulations.pending_cases.clear();
    }

    /// Compute the per-league `SeasonAwardsSnapshot` from this season's
    /// matches, statistics, and clubs. Read-only; does not mutate league
    /// state. Called during `process_season_end` before stats archive.
    fn snapshot_season_awards(
        &self,
        clubs: &[Club],
        current_date: NaiveDate,
    ) -> SeasonAwardsSnapshot {
        // Aggregate every match this season.
        let scores = AwardAggregator::aggregate(self.matches.iter_in_range(
            current_date - chrono::Duration::days(366),
            current_date + chrono::Duration::days(1),
        ));

        // Min apps gate: 15 apps OR 40% of league matches per team.
        let team_count = self.table.rows.len() as u32;
        let typical_matches_per_team = team_count.saturating_sub(1) * 2;
        let pos_min_apps =
            (typical_matches_per_team as f32 * 0.4).round() as u8;
        let min_apps_player = pos_min_apps.max(15);
        let min_apps_young = 10u8;

        // Final-table outcomes — used for team finish multipliers and to
        // mark the relegated finishers cleanly.
        let mut champion_team_id: Option<u32> = None;
        let mut top_four: Vec<u32> = Vec::new();
        let mut relegated: Vec<u32> = Vec::new();
        let row_count = self.table.rows.len();
        let relegation_spots = self.settings.relegation_spots as usize;
        for (idx, row) in self.table.rows.iter().enumerate() {
            if idx == 0 {
                champion_team_id = Some(row.team_id);
            }
            if idx < 4 {
                top_four.push(row.team_id);
            }
            if idx >= row_count.saturating_sub(relegation_spots) {
                relegated.push(row.team_id);
            }
        }

        let team_finish_mul = |team_id: Option<u32>| -> f32 {
            match team_id {
                Some(t) if Some(t) == champion_team_id => 1.10,
                Some(t) if top_four.contains(&t) => 1.05,
                Some(t) if relegated.contains(&t) => 0.90,
                _ => 1.0,
            }
        };

        // Build a player-id → (age, position-group, club-team-id) lookup
        // from the league's clubs once so per-candidate filtering doesn't
        // walk the world.
        let mut player_meta: std::collections::HashMap<u32, (u8, Option<u32>)> =
            std::collections::HashMap::new();
        for club in clubs {
            for team in &club.teams.teams {
                if team.league_id != Some(self.id) {
                    continue;
                }
                for player in &team.players.players {
                    let age = DateUtils::age(player.birth_date, current_date);
                    player_meta.insert(player.id, (age, Some(team.id)));
                }
            }
        }

        let weekly_awards_for = |pid: u32| -> u8 {
            self.player_of_week
                .items()
                .iter()
                .filter(|a| a.player_id == pid)
                .count()
                .min(u8::MAX as usize) as u8
        };

        let pick_best = |min_apps: u8, eligibility: &dyn Fn(u32) -> bool| -> Option<u32> {
            scores
                .iter()
                .filter(|(id, agg)| agg.matches_played >= min_apps && eligibility(**id))
                .map(|(id, agg)| {
                    let team_finish = team_finish_mul(player_meta.get(id).and_then(|(_, t)| *t));
                    let weekly = weekly_awards_for(*id);
                    let s = SeasonAwardSelector::score(agg, self.reputation, team_finish, weekly);
                    (*id, s, *agg)
                })
                .filter(|(_, s, _)| *s > 0.0)
                .max_by(|(la, sa, aa), (lb, sb, ab)| {
                    sa.partial_cmp(sb)
                        .unwrap_or(std::cmp::Ordering::Equal)
                        .then(
                            aa.best_rating
                                .partial_cmp(&ab.best_rating)
                                .unwrap_or(std::cmp::Ordering::Equal),
                        )
                        .then(aa.matches_played.cmp(&ab.matches_played))
                        .then(lb.cmp(la))
                })
                .map(|(id, _, _)| id)
        };

        let player_of_season = pick_best(min_apps_player, &|_| true);
        let young_player_of_season = pick_best(min_apps_young, &|id| {
            player_meta
                .get(&id)
                .map(|(age, _)| *age <= 21)
                .unwrap_or(false)
        });

        // Team of the season — positional quotas reused, but candidates
        // must clear a season-long appearance floor so a one-match elite
        // performance can't outrank a regular starter. Floor scales with
        // the league size: max(10, 30% of typical matches per team).
        let tots_min_apps = ((typical_matches_per_team as f32 * 0.30).round() as u8).max(10);
        let team_of_season: Vec<u32> =
            TeamOfTheWeekSelector::pick_with_min_apps(&scores, tots_min_apps)
                .into_iter()
                .map(|(id, _, _, _)| id)
                .collect();

        // League-wide top scorer / assister / golden glove. Use stats
        // directly: aggregator is per-match and may miss season totals
        // for players who racked up cup goals (already excluded by friendly
        // filter; matches storage holds league only). Fallback to walking
        // clubs when statistics doesn't have a winner — this also catches
        // the case where stats has decayed.
        let top_scorer = self.statistics.top_scorer.map(|(id, _)| id);
        let top_assists = self.statistics.top_assists.map(|(id, _)| id);

        // Golden glove — most clean sheets among GKs with ≥10 starts.
        let mut golden_glove: Option<u32> = None;
        let mut best_cs: u16 = 0;
        for club in clubs {
            for team in &club.teams.teams {
                if team.league_id != Some(self.id) {
                    continue;
                }
                for player in &team.players.players {
                    if !player.positions.is_goalkeeper() {
                        continue;
                    }
                    if player.statistics.played < 10 {
                        continue;
                    }
                    let cs = player.statistics.clean_sheets;
                    if cs > best_cs
                        || (cs == best_cs
                            && golden_glove.map(|gg| player.id < gg).unwrap_or(false))
                    {
                        best_cs = cs;
                        golden_glove = Some(player.id);
                    }
                }
            }
        }

        let _ = MonthlyAwardSelector::score; // silence unused-in-some-builds
        SeasonAwardsSnapshot {
            season_end_date: current_date,
            player_of_season,
            young_player_of_season,
            team_of_season,
            top_scorer,
            top_assists,
            golden_glove,
            champion_team_id,
            top_four_team_ids: top_four,
            relegated_team_ids: relegated,
        }
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
