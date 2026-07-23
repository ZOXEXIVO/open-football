use crate::MatchRuntime;
use crate::context::{GlobalContext, SimulationContext};
use crate::league::{
    LeagueAwards, LeagueBuildOutput, LeagueDynamics, LeagueMilestones, LeaguePendingState,
    LeagueRegulations, LeagueResult, LeagueStatistics, LeagueTable, LeagueTableRow, MatchStorage,
    PlayerOfTheWeekHistory, Schedule, ScheduleItem,
};
use crate::r#match::MatchResult;
use crate::{Club, PlayerFieldPositionGroup, PlayerStatistics, Team};
use chrono::Duration;
use chrono::{Datelike, NaiveDate};
use log::debug;

#[derive(Debug, Clone)]
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
    pub final_table: Option<Vec<LeagueTableRow>>,
    /// Split-season leagues (Argentine Apertura/Clausura): the first
    /// tournament's final table, frozen when the second tournament kicks
    /// off. `None` outside split leagues and during the first tournament.
    pub split_first_table: Option<Vec<LeagueTableRow>>,
    pub dynamics: LeagueDynamics,
    pub regulations: LeagueRegulations,
    pub statistics: LeagueStatistics,
    pub milestones: LeagueMilestones,
    pub friendly: bool,
    pub is_cup: bool,
    pub financials: LeagueFinancials,
    pub player_of_week: PlayerOfTheWeekHistory,
    pub awards: LeagueAwards,
}

#[derive(Debug, Clone, Default)]
pub struct LeagueFinancials {
    pub prize_pool: i64,
    pub tv_deal_total: i64,
}

impl LeagueFinancials {
    pub fn from_reputation_and_tier(reputation: u16, tier: u8, country_reputation: u16) -> Self {
        let rep_factor = (reputation as f64 / 10000.0).clamp(0.0, 1.0);
        let country_factor = (country_reputation as f64 / 10000.0).clamp(0.0, 1.0);

        let tier_factor = match tier {
            1 => 1.0,
            2 => 0.30,
            _ => 0.10,
        };

        let base_prize = 200_000_000.0;
        let base_tv = 500_000_000.0;

        let country_market = country_factor * country_factor;
        let scale = rep_factor * rep_factor * rep_factor * tier_factor * country_market;

        LeagueFinancials {
            prize_pool: (base_prize * scale) as i64,
            tv_deal_total: (base_tv * scale) as i64,
        }
    }
}

impl League {
    pub fn new(
        id: u32,
        name: String,
        slug: String,
        country_id: u32,
        reputation: u16,
        settings: LeagueSettings,
        friendly: bool,
    ) -> Self {
        let financials = LeagueFinancials::default();

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
            final_table: None,
            split_first_table: None,
            dynamics: LeagueDynamics::new(),
            regulations: LeagueRegulations::new(),
            statistics: LeagueStatistics::new(),
            milestones: LeagueMilestones::new(),
            friendly,
            is_cup: false,
            financials,
            player_of_week: PlayerOfTheWeekHistory::new(),
            awards: LeagueAwards::default(),
        }
    }

    /// Prepare today's matchday but do not play it. Mutates the league's
    /// dynamics / table / schedule up to (and including) schedule
    /// regeneration, then either:
    /// - returns a [`LeagueBuildOutput`] with `pending = Some(...)` and
    ///   the `Match` objects ready for a batched engine dispatch, or
    /// - runs the non-matchday work and returns `immediate = Some(LeagueResult)`.
    ///
    /// The matched second half is [`simulate_process`], which takes the
    /// played [`MatchResult`]s and `LeaguePendingState` back and runs
    /// `process_match_day_results`.
    pub fn simulate_build(&mut self, clubs: &[Club], ctx: &GlobalContext<'_>) -> LeagueBuildOutput {
        let league_name = self.name.clone();
        debug!(
            "⚽ Building matchday for league: {} (Reputation: {})",
            league_name, self.reputation
        );

        self.prepare_matchday(ctx, clubs);

        self.maybe_flip_split_stage(&ctx.simulation);

        let table_result = self.table.simulate(ctx);

        let league_teams: Vec<u32> = clubs
            .iter()
            .flat_map(|c| c.teams.with_league(self.id))
            .collect();

        let schedule_result = self.schedule.simulate(
            &self.settings,
            ctx.with_league(
                self.id,
                String::from(&self.slug),
                &league_teams,
                self.reputation,
            ),
        );

        let new_season_started =
            schedule_result.generated && self.table.rows.iter().any(|r| r.played > 0);

        if schedule_result.generated {
            self.table = LeagueTable::new(&league_teams);
            self.matches = MatchStorage::new();
            self.split_first_table = None;
            debug!("📊 League table reset for new season: {}", self.name);
        }

        if schedule_result.is_match_scheduled() {
            let matches = self.build_matchday_matches(
                &schedule_result.scheduled_matches,
                clubs,
                ctx,
                self.friendly,
                false,
            );
            return LeagueBuildOutput {
                matches,
                pending: Some(LeaguePendingState {
                    scheduled_matches: schedule_result.scheduled_matches,
                    table_result,
                    new_season_started,
                }),
                immediate: None,
            };
        }

        self.process_non_matchday(clubs, ctx);
        let mut result = LeagueResult::new(self.id, table_result);
        result.new_season_started = new_season_started;
        LeagueBuildOutput {
            matches: Vec::new(),
            pending: None,
            immediate: Some(result),
        }
    }

    /// Apply played match results onto a league's [`LeaguePendingState`]
    /// returned from [`simulate_build`]. Stamps results onto the
    /// scheduled fixtures, runs the per-match table / dynamics /
    /// statistics / discipline fan-out, and yields the final
    /// [`LeagueResult`] for the day.
    pub fn simulate_process(
        &mut self,
        match_results: Vec<MatchResult>,
        pending: LeaguePendingState,
        clubs: &[Club],
        ctx: &GlobalContext<'_>,
        current_date: NaiveDate,
    ) -> LeagueResult {
        let LeaguePendingState {
            mut scheduled_matches,
            table_result,
            new_season_started,
        } = pending;

        self.apply_matchday_results(&mut scheduled_matches, &match_results);
        self.process_match_day_results(&match_results, clubs, ctx, current_date);

        let mut result = LeagueResult::with_match_result(self.id, table_result, match_results);
        result.new_season_started = new_season_started;
        result
    }

    /// Backwards-compatible wrapper that runs build → engine → process
    /// in one call. Production paths (`Country::simulate_build` +
    /// `Continent::simulate` + `WorldMatchdayResult::process`) call
    /// the halves directly so the world's matches dispatch in ONE
    /// global batch per tick instead of one per league.
    pub fn simulate(&mut self, clubs: &[Club], ctx: GlobalContext<'_>) -> LeagueResult {
        let current_date = ctx.simulation.date.date();
        let output = self.simulate_build(clubs, &ctx);
        if let Some(immediate) = output.immediate {
            return immediate;
        }
        let match_results = MatchRuntime::engine_pool().play(output.matches);
        let pending = output
            .pending
            .expect("simulate_build with matches must produce a pending state");
        self.simulate_process(match_results, pending, clubs, &ctx, current_date)
    }

    #[allow(dead_code)]
    fn get_team<'c>(&self, clubs: &'c [Club], id: u32) -> &'c Team {
        clubs
            .iter()
            .flat_map(|c| &c.teams.teams)
            .find(|team| team.id == id)
            .unwrap()
    }

    /// Aggregate one player's statistics across this league's stored match
    /// records, replicating the in-match stat fan-out (`record_match_*`)
    /// over each match's recorded `player_stats`. The web layer uses this
    /// to render a player's cup row straight from the authoritative match
    /// records — the same source the cup pages read — rather than the live
    /// per-spell counter, which can be incomplete for a player who changed
    /// clubs mid-season. Because it consumes the same per-match stats the
    /// live counter was fed, the totals match for players who never moved.
    /// Returns `None` when the player never featured in a stored match.
    pub fn aggregate_player_statistics(&self, player_id: u32) -> Option<PlayerStatistics> {
        let mut s = PlayerStatistics::default();
        let mut featured = false;

        for mr in self.matches.iter() {
            let Some(details) = mr.details.as_ref() else {
                continue;
            };
            let Some(ps) = details.player_stats.get(&player_id) else {
                continue;
            };

            let in_left_main = details.left_team_players.main.contains(&player_id);
            let in_right_main = details.right_team_players.main.contains(&player_id);
            let is_starter = in_left_main || in_right_main;
            let is_sub = details
                .left_team_players
                .substitutes_used
                .contains(&player_id)
                || details
                    .right_team_players
                    .substitutes_used
                    .contains(&player_id);
            if !is_starter && !is_sub {
                continue;
            }
            featured = true;

            if is_starter {
                s.played += 1;
            } else {
                s.played_subs += 1;
            }

            s.goals += ps.goals;
            s.assists += ps.assists;
            s.shots_on_target += ps.shots_on_target as f32;
            s.tackling += ps.tackles as f32;
            s.yellow_cards = s.yellow_cards.saturating_add(ps.yellow_cards as u8);
            s.red_cards = s.red_cards.saturating_add(ps.red_cards as u8);

            if ps.passes_attempted > 0 {
                let match_pct =
                    (ps.passes_completed as f32 / ps.passes_attempted as f32 * 100.0) as u8;
                let games = s.played + s.played_subs;
                s.passes = if games <= 1 {
                    match_pct
                } else {
                    let prev = s.passes as f32;
                    ((prev * (games - 1) as f32 + match_pct as f32) / games as f32) as u8
                };
            }

            s.record_match_rating(ps.match_rating, ps.minutes_played, is_starter);

            if details.player_of_the_match_id == Some(player_id) {
                s.player_of_the_match = s.player_of_the_match.saturating_add(1);
            }

            // GK conceded / clean sheets — starting keepers only, mirroring
            // `record_match_stats`. The player's side is resolved from the
            // squad's `team_id`; goals against come from the other side.
            if is_starter && ps.position_group == PlayerFieldPositionGroup::Goalkeeper {
                let side_team_id = if in_left_main {
                    details.left_team_players.team_id
                } else {
                    details.right_team_players.team_id
                };
                let conceded = if mr.score.home_team.team_id == side_team_id {
                    mr.score.away_team.get()
                } else {
                    mr.score.home_team.get()
                };
                s.conceded += conceded as u16;
                if conceded == 0 {
                    s.clean_sheets += 1;
                }
            }
        }

        if featured { Some(s) } else { None }
    }

    /// Split-season leagues: the number of schedule tours that belong to the
    /// first tournament (Apertura). The generator always emits two mirrored
    /// single round-robins for split leagues, so the boundary is the halfway
    /// point. `None` for regular leagues.
    pub fn split_stage_tour_boundary(&self) -> Option<usize> {
        if !self.settings.split_season || self.schedule.tours.is_empty() {
            return None;
        }
        Some(self.schedule.tours.len() / 2)
    }

    /// True once every fixture of the first (Apertura) tournament has a
    /// result. Meaningless (always false) for non-split leagues.
    pub fn first_stage_played(&self) -> bool {
        match self.split_stage_tour_boundary() {
            Some(boundary) if boundary > 0 => self.schedule.tours[..boundary]
                .iter()
                .all(|t| !t.items.is_empty() && t.items.iter().all(|i| i.result.is_some())),
            _ => false,
        }
    }

    /// On the second tournament's opening day of a split season, freeze the
    /// first tournament's table into `split_first_table` and restart the
    /// live table from zero. Idempotent per season — the frozen table is
    /// only taken once and clears at the next season's schedule reset.
    fn maybe_flip_split_stage(&mut self, sim: &SimulationContext) {
        if !self.settings.split_season || self.split_first_table.is_some() {
            return;
        }
        let date = sim.date.date();
        let second = &self.settings.season_ending_half;
        if date.day() as u8 != second.from_day || date.month() as u8 != second.from_month {
            return;
        }
        if self.table.rows.iter().all(|r| r.played == 0) {
            return; // nothing to freeze — first tournament never ran
        }
        let team_ids: Vec<u32> = self.table.rows.iter().map(|r| r.team_id).collect();
        self.split_first_table = Some(self.table.rows.clone());
        self.table = LeagueTable::new(&team_ids);
        debug!(
            "📊 Split season: first-stage table frozen and reset for {}",
            self.name
        );
    }

    /// Annual aggregate rows for this league across both tournaments of a
    /// split season (first-stage frozen table + live/second table, summed
    /// per team and re-sorted). For regular leagues this is just the live
    /// table order. The relegation pipeline and the web's "Tabla Anual"
    /// both read this.
    pub fn annual_table_rows(&self) -> Vec<LeagueTableRow> {
        let mut rows: Vec<LeagueTableRow> = self.table.rows.clone();
        if let Some(first) = &self.split_first_table {
            for f in first {
                if let Some(row) = rows.iter_mut().find(|r| r.team_id == f.team_id) {
                    row.played = row.played.saturating_add(f.played);
                    row.win = row.win.saturating_add(f.win);
                    row.draft = row.draft.saturating_add(f.draft);
                    row.lost = row.lost.saturating_add(f.lost);
                    row.goal_scored += f.goal_scored;
                    row.goal_concerned += f.goal_concerned;
                    row.points = row.points.saturating_add(f.points);
                    row.points_deduction = row.points_deduction.saturating_add(f.points_deduction);
                } else {
                    rows.push(f.clone());
                }
            }
        }
        let policy = self.table.tie_break.clone();
        rows.sort_by(|a, b| policy.compare(a, b));
        rows
    }
}

#[derive(Debug, Clone, Copy)]
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

#[derive(Debug, Clone)]
pub struct LeagueSettings {
    pub season_starting_half: DayMonthPeriod,
    pub season_ending_half: DayMonthPeriod,
    pub tier: u8,
    pub promotion_spots: u8,
    pub relegation_spots: u8,
    pub league_group: Option<LeagueGroup>,
    /// Argentine-style split season: the two season halves are separate
    /// tournaments (Apertura in the starting half, Clausura in the ending
    /// half), each a single round-robin with its own table, playoff and
    /// champion. Relegation reads the annual aggregate across both.
    pub split_season: bool,
}

/// Identifies a league as one group within a larger competition.
#[derive(Debug, Clone)]
pub struct LeagueGroup {
    pub name: String,
    pub competition: String,
    pub total_groups: u8,
    /// When set, the competition crowns a single champion via an
    /// end-of-season knockout playoff seeded from every group's final
    /// standings (MLS Cup, Serie C promotion playoff, …). Drives
    /// [`crate::league::LeaguePlayoff`]. Carried on each member group's
    /// config; the builder reads it from whichever member declares it.
    pub playoff: Option<LeaguePlayoffConfig>,
}

/// Configuration for a grouped competition's end-of-season playoff. See
/// [`crate::league::LeaguePlayoff`].
#[derive(Debug, Clone)]
pub struct LeaguePlayoffConfig {
    /// Top N of each group's table that enter the knockout bracket.
    pub qualifiers_per_group: u8,
    /// Bracket shape — see [`PlayoffFormat`].
    pub format: PlayoffFormat,
    /// Display name for the playoff competition (e.g. "MLS Cup Playoffs").
    /// Falls back to "{competition} Playoff" when unset.
    pub name: Option<String>,
    /// Split-season tournaments' display names, first then second (e.g.
    /// ["Torneo Apertura", "Torneo Clausura"]). Only read when the member
    /// groups run a split season; each stage gets its own playoff.
    pub stage_names: Vec<String>,
}

/// Bracket shape for a grouped competition's playoff.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlayoffFormat {
    /// Generic single-elimination: group seeds interleaved into one field,
    /// re-paired strongest-vs-weakest each round, byes to top seeds.
    SingleElimination,
    /// Argentine Primera: the two zones cross immediately (1°A vs 8°B, …)
    /// in a fixed bracket tree; the better seed hosts every round and the
    /// final is at a neutral venue.
    CrossGroupBracket,
    /// MLS Cup: per-conference brackets — wild card (8 v 9), best-of-3
    /// round one, single-game conference semifinal/final — meeting in a
    /// single final hosted by the better regular-season record.
    MlsCup,
}

impl PlayoffFormat {
    pub fn from_config_str(value: &str) -> Self {
        match value {
            "cross_group" => PlayoffFormat::CrossGroupBracket,
            "mls" => PlayoffFormat::MlsCup,
            _ => PlayoffFormat::SingleElimination,
        }
    }
}

impl LeagueSettings {
    pub fn is_time_for_new_schedule(&self, context: &SimulationContext) -> bool {
        let season_starting_date = &self.season_starting_half;
        let date = context.date.date();
        (NaiveDate::day(&date) as u8) == season_starting_date.from_day
            && (date.month() as u8) == season_starting_date.from_month
    }
}

#[cfg(test)]
mod split_season_tests {
    use super::*;
    use crate::league::LeagueTable;

    fn split_league() -> League {
        let settings = LeagueSettings {
            season_starting_half: DayMonthPeriod::new(1, 2, 30, 6),
            season_ending_half: DayMonthPeriod::new(15, 7, 15, 12),
            tier: 1,
            promotion_spots: 0,
            relegation_spots: 1,
            league_group: None,
            split_season: true,
        };
        League::new(
            1,
            "Zona A".into(),
            "zona-a".into(),
            1,
            7500,
            settings,
            false,
        )
    }

    fn row(team_id: u32, points: u8, gs: i32, gc: i32) -> LeagueTableRow {
        LeagueTableRow {
            team_id,
            played: 16,
            win: points / 3,
            draft: points % 3,
            lost: 0,
            goal_scored: gs,
            goal_concerned: gc,
            points,
            points_deduction: 0,
        }
    }

    #[test]
    fn annual_table_sums_both_tournaments_and_resorts() {
        let mut league = split_league();
        // Apertura frozen: team 2 topped it; Clausura live: team 1 in front.
        league.split_first_table = Some(vec![row(2, 35, 30, 10), row(1, 20, 15, 15)]);
        league.table = LeagueTable::new(&[1, 2]);
        league.table.rows = vec![row(1, 30, 25, 8), row(2, 10, 9, 20)];

        let annual = league.annual_table_rows();
        // Team 1: 50 pts, team 2: 45 pts — annual order flips the live one.
        assert_eq!(annual[0].team_id, 1);
        assert_eq!(annual[0].points, 50);
        assert_eq!(annual[0].played, 32);
        assert_eq!(annual[0].goal_scored, 40);
        assert_eq!(annual[1].team_id, 2);
        assert_eq!(annual[1].points, 45);
    }

    #[test]
    fn annual_table_is_live_table_for_regular_leagues() {
        let mut league = split_league();
        league.settings.split_season = false;
        league.table = LeagueTable::new(&[1, 2]);
        league.table.rows = vec![row(1, 30, 25, 8), row(2, 10, 9, 20)];
        let annual = league.annual_table_rows();
        assert_eq!(annual.len(), 2);
        assert_eq!(annual[0].team_id, 1);
        assert_eq!(annual[0].points, 30);
    }
}

// Schedule extensions for enhanced functionality
impl Schedule {
    pub fn get_matches_in_next_days(&self, from_date: NaiveDate, days: i64) -> Vec<&ScheduleItem> {
        let end_date = from_date + Duration::days(days);

        self.tours
            .iter()
            .flat_map(|t| &t.items)
            .filter(|item| {
                let item_date = item.date.date();
                item_date >= from_date && item_date <= end_date && item.result.is_none()
            })
            .collect()
    }

    pub fn get_matches_for_team_in_days(
        &self,
        team_id: u32,
        from_date: NaiveDate,
        days: i64,
    ) -> Vec<&ScheduleItem> {
        self.matches_for_team_in_days(team_id, from_date, days)
            .collect()
    }

    /// Iterator variant of `get_matches_for_team_in_days`. Avoids the
    /// per-call `Vec<&ScheduleItem>` allocation on hot paths that only
    /// need to count or short-circuit (matchday congestion checks).
    pub fn matches_for_team_in_days(
        &self,
        team_id: u32,
        from_date: NaiveDate,
        days: i64,
    ) -> impl Iterator<Item = &ScheduleItem> + '_ {
        let end_date = from_date + Duration::days(days);
        self.tours
            .iter()
            .flat_map(|t| &t.items)
            .filter(move |item| {
                let item_date = item.date.date();
                (item.home_team_id == team_id || item.away_team_id == team_id)
                    && item_date >= from_date
                    && item_date <= end_date
                    && item.result.is_none()
            })
    }

    /// Count upcoming matches for a team in the next `days` without
    /// allocating a Vec — used by congestion checks.
    pub fn count_matches_for_team_in_days(
        &self,
        team_id: u32,
        from_date: NaiveDate,
        days: i64,
    ) -> usize {
        self.matches_for_team_in_days(team_id, from_date, days)
            .count()
    }

    /// Short-circuiting variant — avoids walking the full schedule
    /// when only presence matters.
    pub fn has_matches_for_team_in_days(
        &self,
        team_id: u32,
        from_date: NaiveDate,
        days: i64,
    ) -> bool {
        self.matches_for_team_in_days(team_id, from_date, days)
            .next()
            .is_some()
    }
}
