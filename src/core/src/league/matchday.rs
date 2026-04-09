use crate::context::GlobalContext;
use crate::league::{LeagueDynamics, LeagueMatch, LeagueMatchResultResult, LeagueTable};
use crate::r#match::{Match, MatchResult, SelectionContext};
use crate::{Club, Person, Player, PlayerStatusType, Team, TeamType};
use chrono::{Datelike, NaiveDate};
use log::debug;

use super::League;

impl League {
    pub(super) fn prepare_matchday(&mut self, ctx: &GlobalContext<'_>, clubs: &[Club]) {
        debug!("Preparing matchday for {}", self.name);

        let current_date = ctx.simulation.date.date();
        let day_of_week = current_date.weekday();

        self.dynamics.update_attendance_predictions(
            &self.table,
            day_of_week,
            current_date.month(),
        );

        for club in clubs {
            for team in &club.teams.teams {
                if team.league_id == Some(self.id) {
                    self.check_fixture_congestion(team, current_date);
                }
            }
        }

        self.dynamics.assign_referees();
    }

    fn check_fixture_congestion(&self, team: &Team, current_date: NaiveDate) {
        let upcoming_matches = self.schedule.get_matches_for_team_in_days(team.id, current_date, 7);
        if upcoming_matches.len() > 2 {
            debug!("⚠️ Fixture congestion for team {}: {} matches in 7 days",
                   team.name, upcoming_matches.len());
        }
    }

    pub(super) fn play_scheduled_matches(
        &mut self,
        scheduled_matches: &mut [LeagueMatch],
        clubs: &[Club],
        ctx: &GlobalContext<'_>,
        friendly: bool,
    ) -> Vec<MatchResult> {
        let matches: Vec<Match> = scheduled_matches
            .iter()
            .map(|scheduled_match| {
                Self::build_match(
                    scheduled_match,
                    clubs,
                    ctx,
                    &self.dynamics,
                    &self.table,
                    friendly,
                )
            })
            .collect();

        let match_results = crate::match_engine_pool().play(matches);

        for (scheduled_match, result) in scheduled_matches.iter_mut().zip(match_results.iter()) {
            scheduled_match.result = Some(LeagueMatchResultResult::from_score(&result.score));
        }

        for result in &match_results {
            self.dynamics.update_team_momentum_after_match(
                result.home_team_id,
                result.away_team_id,
                result,
            );
        }

        match_results
    }

    fn build_match(
        scheduled_match: &LeagueMatch,
        clubs: &[Club],
        ctx: &GlobalContext<'_>,
        dynamics: &LeagueDynamics,
        table: &LeagueTable,
        friendly: bool,
    ) -> Match {
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

        let home_momentum = dynamics.get_team_momentum(scheduled_match.home_team_id);
        let away_momentum = dynamics.get_team_momentum(scheduled_match.away_team_id);

        let (home_pressure, away_pressure) = Self::calculate_match_pressures_static(
            home_team,
            away_team,
            ctx.simulation.date.date(),
            dynamics,
            table,
        );

        // Calculate match importance for squad selection decisions
        let match_importance = if friendly {
            0.1
        } else {
            Self::calculate_match_importance(table, home_team, away_team, ctx.simulation.date.date())
        };

        let selection_ctx = SelectionContext {
            is_friendly: friendly,
            date: ctx.simulation.date.date(),
            match_importance,
        };

        let (mut home_squad, mut away_squad) = if friendly {
            let mut home_supplements = Self::collect_supplementary_players(clubs, home_team.club_id, home_team.id, friendly);
            let mut away_supplements = Self::collect_supplementary_players(clubs, away_team.club_id, away_team.id, friendly);

            let home_overage = Self::collect_overage_development_players(
                clubs, home_team.club_id, home_team.id, &home_team.team_type, ctx.simulation.date.date(),
            );
            let away_overage = Self::collect_overage_development_players(
                clubs, away_team.club_id, away_team.id, &away_team.team_type, ctx.simulation.date.date(),
            );
            home_supplements.extend(home_overage);
            away_supplements.extend(away_overage);

            (
                home_team.get_rotation_match_squad_with_reserves(&home_supplements, &selection_ctx),
                away_team.get_rotation_match_squad_with_reserves(&away_supplements, &selection_ctx),
            )
        } else {
            let home_reserves = Self::collect_reserve_players(clubs, home_team.club_id, home_team.id, friendly);
            let away_reserves = Self::collect_reserve_players(clubs, away_team.club_id, away_team.id, friendly);
            (
                home_team.get_enhanced_match_squad(&home_reserves, &selection_ctx),
                away_team.get_enhanced_match_squad(&away_reserves, &selection_ctx),
            )
        };

        Self::apply_psychological_factors_static(&mut home_squad, home_momentum, home_pressure);
        Self::apply_psychological_factors_static(&mut away_squad, away_momentum, away_pressure);

        Match::make(
            scheduled_match.id.clone(),
            scheduled_match.league_id,
            &scheduled_match.league_slug,
            home_squad,
            away_squad,
            friendly,
        )
    }

    /// Collect available reserve players from the same club.
    /// Only pulls from B/U21/U23 teams — not from youth academies (U18/U19/U20).
    fn collect_reserve_players<'a>(
        clubs: &'a [Club],
        club_id: u32,
        team_id: u32,
        is_friendly: bool,
    ) -> Vec<&'a Player> {
        let Some(club) = clubs.iter().find(|c| c.id == club_id) else {
            return Vec::new();
        };

        club.teams
            .teams
            .iter()
            .filter(|t| {
                t.id != team_id
                    && matches!(t.team_type, TeamType::B | TeamType::Reserve | TeamType::U21 | TeamType::U23)
            })
            .flat_map(|t| t.players.players.iter())
            .filter(|p| Self::is_player_available(p, is_friendly))
            .collect()
    }

    /// Collect supplementary players from other teams in the same club.
    /// Used by non-main teams in friendly leagues to ensure they have enough players.
    fn collect_supplementary_players<'a>(
        clubs: &'a [Club],
        club_id: u32,
        team_id: u32,
        is_friendly: bool,
    ) -> Vec<&'a Player> {
        let Some(club) = clubs.iter().find(|c| c.id == club_id) else {
            return Vec::new();
        };

        club.teams
            .teams
            .iter()
            .filter(|t| t.id != team_id)
            .flat_map(|t| t.players.players.iter())
            .filter(|p| Self::is_player_available(p, is_friendly))
            .collect()
    }

    /// Collect up to 3 overage players from higher youth teams who need match practice.
    /// Only applies to U18/U19 teams — allows older youth players (from U20/U21/U23)
    /// who aren't getting matches to gain development time, like real overage rules.
    fn collect_overage_development_players<'a>(
        clubs: &'a [Club],
        club_id: u32,
        team_id: u32,
        team_type: &TeamType,
        date: NaiveDate,
    ) -> Vec<&'a Player> {
        const MAX_OVERAGE_SLOTS: usize = 3;
        const MIN_IDLE_DAYS: u16 = 21;

        if !matches!(team_type, TeamType::U18 | TeamType::U19) {
            return Vec::new();
        }

        let Some(club) = clubs.iter().find(|c| c.id == club_id) else {
            return Vec::new();
        };

        let mut candidates: Vec<&Player> = club.teams
            .teams
            .iter()
            .filter(|t| {
                t.id != team_id
                    && matches!(t.team_type, TeamType::U20 | TeamType::U21 | TeamType::U23)
            })
            .flat_map(|t| t.players.players.iter())
            .filter(|p| {
                Self::is_player_available(p, true)
                    && p.player_attributes.days_since_last_match >= MIN_IDLE_DAYS
                    && p.age(date) <= 23
            })
            .collect();

        candidates.sort_by(|a, b| {
            b.player_attributes.days_since_last_match
                .cmp(&a.player_attributes.days_since_last_match)
        });

        candidates.truncate(MAX_OVERAGE_SLOTS);
        candidates
    }

    /// Calculate how important a match is for squad selection decisions.
    /// Returns 0.0 (dead rubber) to 1.0 (must-win).
    ///
    /// Key principle: if a team has nothing to play for, importance drops
    /// significantly — reserves and youth get chances.
    fn calculate_match_importance(
        table: &LeagueTable,
        home_team: &Team,
        away_team: &Team,
        _date: NaiveDate,
    ) -> f32 {
        let total_teams = table.rows.len();
        if total_teams == 0 {
            return 0.5;
        }

        let home_row = table.rows.iter().enumerate().find(|(_, r)| r.team_id == home_team.id);
        let away_row = table.rows.iter().enumerate().find(|(_, r)| r.team_id == away_team.id);

        let (home_pos, home_played, home_points) = home_row
            .map(|(i, r)| (i + 1, r.played as f32, r.points as i32))
            .unwrap_or((total_teams / 2, 0.0, 0));

        let away_pos = away_row.map(|(i, _)| i + 1).unwrap_or(total_teams / 2);

        let total_matches = if total_teams > 1 { ((total_teams - 1) * 2) as f32 } else { 1.0 };
        let season_progress = (home_played / total_matches).clamp(0.0, 1.0);
        let remaining_matches = (total_matches - home_played).max(0.0) as i32;

        // Points gap to key positions
        let top3_points = table.rows.get(2).map(|r| r.points as i32).unwrap_or(0);
        let relegation_pos = total_teams.saturating_sub(3);
        let relegation_points = table.rows.get(relegation_pos).map(|r| r.points as i32).unwrap_or(0);
        // Can the team still catch top 3? (3 pts per remaining match)
        let max_reachable = home_points + remaining_matches * 3;
        let can_reach_top3 = max_reachable >= top3_points;

        // Is the team safe from relegation? (gap too large to close)
        let is_safe = home_points > relegation_points + remaining_matches * 3
            || home_pos <= total_teams / 2;
        let is_in_danger = home_points <= relegation_points + 3 && home_pos > total_teams / 2;

        // ── Determine importance ──

        // Title contenders: fighting for top 3
        if home_pos <= 3 && season_progress > 0.3 {
            return if season_progress > 0.7 { 1.0 } else { 0.85 };
        }

        // Chasing top 3 and still mathematically possible
        if home_pos <= 6 && can_reach_top3 && season_progress > 0.5 {
            let gap = top3_points - home_points;
            return if gap <= 6 { 0.85 } else { 0.7 };
        }

        // Relegation battle
        if is_in_danger && season_progress > 0.3 {
            return if season_progress > 0.7 { 1.0 } else { 0.85 };
        }

        // Direct rival: both in top 5 or both in bottom 5
        let both_top = home_pos <= 5 && away_pos <= 5;
        let both_bottom = home_pos > total_teams - 5 && away_pos > total_teams - 5;
        if both_top || both_bottom {
            return 0.8;
        }

        // ── Nothing to play for: dead rubber territory ──

        // Safe from relegation + can't reach top 3 + late season = dead rubber
        if is_safe && !can_reach_top3 && season_progress > 0.7 {
            return 0.15;
        }

        // Same but mid-season: still rotate but less aggressively
        if is_safe && !can_reach_top3 && season_progress > 0.5 {
            return 0.3;
        }

        // Safe, can't reach top 3, early season — moderate rotation
        if is_safe && !can_reach_top3 {
            return 0.4;
        }

        // Early season: everyone still optimistic, moderate importance
        if season_progress < 0.25 {
            return 0.5;
        }

        // Default: standard competitive match
        0.6
    }

    fn is_player_available(player: &Player, is_friendly: bool) -> bool {
        if player.player_attributes.is_injured {
            return false;
        }
        if player.statuses.get().contains(&PlayerStatusType::Int) {
            return false;
        }
        if !is_friendly && player.player_attributes.is_banned {
            return false;
        }
        true
    }

    pub(super) fn calculate_match_pressures_static(
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

        let mut pressure: f32 = 0.5;

        if position < 3 {
            pressure += 0.3;
        }

        if position >= total_teams - 3 {
            pressure += 0.4;
        }

        let losing_streak = dynamics.get_team_losing_streak(team.id);
        if losing_streak > 3 {
            pressure += 0.2;
        }

        pressure.min(1.0)
    }

    fn apply_psychological_factors_static(
        squad: &mut crate::r#match::MatchSquad,
        momentum: f32,
        pressure: f32,
    ) {
        debug!("Team {} - Momentum: {:.2}, Pressure: {:.2}",
               squad.team_name, momentum, pressure);
    }
}
