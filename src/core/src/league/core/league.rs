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
    pub fn simulate_build(
        &mut self,
        clubs: &[Club],
        ctx: &GlobalContext<'_>,
    ) -> LeagueBuildOutput {
        let league_name = self.name.clone();
        debug!(
            "⚽ Building matchday for league: {} (Reputation: {})",
            league_name, self.reputation
        );

        self.prepare_matchday(ctx, clubs);

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
}

/// Identifies a league as one group within a larger competition.
#[derive(Debug, Clone)]
pub struct LeagueGroup {
    pub name: String,
    pub competition: String,
    pub total_groups: u8,
}

impl LeagueSettings {
    pub fn is_time_for_new_schedule(&self, context: &SimulationContext) -> bool {
        let season_starting_date = &self.season_starting_half;
        let date = context.date.date();
        (NaiveDate::day(&date) as u8) == season_starting_date.from_day
            && (date.month() as u8) == season_starting_date.from_month
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
