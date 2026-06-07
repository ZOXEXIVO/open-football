use crate::context::GlobalContext;
use crate::league::{League, LeagueDynamics, LeagueMatch, LeagueMatchResultResult, LeagueTable};
use crate::r#match::MatchSquad;
use crate::r#match::{Match, MatchResult, SelectionCompetition, SelectionContext};
use crate::{Club, ClubPhilosophy, MatchRuntime, Person, Player, Team, TeamType};
use chrono::Duration;
use chrono::{Datelike, NaiveDate};
use log::debug;
use std::collections::HashMap;

/// Per-matchday snapshot of clubs and teams indexed by id. Built once
/// at the top of `play_scheduled_matches` so `build_match` and friends
/// can fan out lookups in O(1) instead of re-scanning every club's
/// team list per fixture.
struct MatchdayLookup<'a> {
    team_by_id: HashMap<u32, &'a Team>,
    club_by_id: HashMap<u32, &'a Club>,
}

impl<'a> MatchdayLookup<'a> {
    fn build(clubs: &'a [Club]) -> Self {
        let estimated_teams = clubs.len() * 4;
        let mut team_by_id: HashMap<u32, &'a Team> = HashMap::with_capacity(estimated_teams);
        let mut club_by_id: HashMap<u32, &'a Club> = HashMap::with_capacity(clubs.len());
        for club in clubs {
            club_by_id.insert(club.id, club);
            for team in &club.teams.teams {
                team_by_id.insert(team.id, team);
            }
        }
        MatchdayLookup {
            team_by_id,
            club_by_id,
        }
    }

    fn team(&self, id: u32) -> Option<&'a Team> {
        self.team_by_id.get(&id).copied()
    }

    fn club(&self, id: u32) -> Option<&'a Club> {
        self.club_by_id.get(&id).copied()
    }
}

impl League {
    pub(in crate::league) fn prepare_matchday(&mut self, ctx: &GlobalContext<'_>, clubs: &[Club]) {
        debug!("Preparing matchday for {}", self.name);

        let current_date = ctx.simulation.date.date();
        let day_of_week = current_date.weekday();

        self.dynamics
            .update_attendance_predictions(&self.table, day_of_week, current_date.month());

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
        let upcoming = self
            .schedule
            .count_matches_for_team_in_days(team.id, current_date, 7);
        if upcoming > 2 {
            debug!(
                "⚠️ Fixture congestion for team {}: {} matches in 7 days",
                team.name, upcoming
            );
        }
    }

    /// Build (but do not play) the `Match` objects for today's fixtures.
    /// Pure read of league state — leaves `self.dynamics` untouched, so
    /// many leagues can be built up front and dispatched as one batch
    /// later. `apply_matchday_results` is the matching half: it takes
    /// the played results back, stamps them onto `scheduled_matches`
    /// and updates the per-team momentum.
    pub(in crate::league) fn build_matchday_matches(
        &self,
        scheduled_matches: &[LeagueMatch],
        clubs: &[Club],
        ctx: &GlobalContext<'_>,
        friendly: bool,
        knockout: bool,
    ) -> Vec<Match> {
        let today = ctx.simulation.date.date();
        let is_cup = self.is_cup;
        let lookup = MatchdayLookup::build(clubs);
        scheduled_matches
            .iter()
            .map(|scheduled_match| {
                // Count upcoming competitive fixtures within 5 days for
                // each side. A team with a cup tie three days from now
                // has a reason to rotate today even if the league match
                // would otherwise be high-importance.
                let home_upcoming = self.schedule.count_matches_for_team_in_days(
                    scheduled_match.home_team_id,
                    today + Duration::days(1),
                    5,
                ) as u8;
                let away_upcoming = self.schedule.count_matches_for_team_in_days(
                    scheduled_match.away_team_id,
                    today + Duration::days(1),
                    5,
                ) as u8;
                Self::build_match(
                    scheduled_match,
                    clubs,
                    &lookup,
                    ctx,
                    &self.dynamics,
                    &self.table,
                    friendly,
                    knockout,
                    is_cup,
                    (home_upcoming, away_upcoming),
                )
            })
            .collect()
    }

    /// Stamp played results back onto `scheduled_matches` (paired by
    /// index with the order returned from `build_matchday_matches`) and
    /// bump each team's momentum. Run after the engine — local or
    /// distributed — has returned the matches.
    pub(in crate::league) fn apply_matchday_results(
        &mut self,
        scheduled_matches: &mut [LeagueMatch],
        match_results: &[MatchResult],
    ) {
        for (scheduled_match, result) in scheduled_matches.iter_mut().zip(match_results.iter()) {
            scheduled_match.result = Some(LeagueMatchResultResult::from_score(&result.score));
        }
        for result in match_results {
            self.dynamics.update_team_momentum_after_match(
                result.home_team_id,
                result.away_team_id,
                result,
            );
        }
    }

    /// Backwards-compatible single-call wrapper. The production path
    /// (`Country::simulate_build` + `Continent::simulate_build` +
    /// `FootballSimulator::simulate_with`) calls the
    /// `build_matchday_matches` / `apply_matchday_results` halves
    /// directly so the world's matches dispatch in ONE global batch
    /// per tick. Kept for tests and any future single-league call
    /// site.
    #[allow(dead_code)]
    pub(in crate::league) fn play_scheduled_matches(
        &mut self,
        scheduled_matches: &mut [LeagueMatch],
        clubs: &[Club],
        ctx: &GlobalContext<'_>,
        friendly: bool,
        knockout: bool,
    ) -> Vec<MatchResult> {
        let matches = self.build_matchday_matches(scheduled_matches, clubs, ctx, friendly, knockout);
        let match_results = MatchRuntime::engine_pool().play(matches);
        self.apply_matchday_results(scheduled_matches, &match_results);
        match_results
    }

    #[allow(clippy::too_many_arguments)]
    fn build_match(
        scheduled_match: &LeagueMatch,
        clubs: &[Club],
        lookup: &MatchdayLookup<'_>,
        ctx: &GlobalContext<'_>,
        dynamics: &LeagueDynamics,
        table: &LeagueTable,
        friendly: bool,
        knockout: bool,
        is_cup: bool,
        upcoming_fixtures: (u8, u8),
    ) -> Match {
        let home_team = lookup
            .team(scheduled_match.home_team_id)
            .expect("Home team not found");

        let away_team = lookup
            .team(scheduled_match.away_team_id)
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

        // Match importance for squad selection decisions. A league game
        // reads the table; continental ties build their own context and
        // never reach this builder. A domestic cup tie scales importance by
        // bracket stage and the reputation gap — early rounds rotate, finals
        // demand the strongest XI — computed per side because the two clubs
        // face different opponents.
        let date = ctx.simulation.date.date();
        let domestic_cup_round = if knockout && is_cup {
            scheduled_match
                .cup_round
                .map(|round| (round, scheduled_match.cup_total_rounds.unwrap_or(round)))
        } else {
            None
        };

        let home_rep = home_team.reputation.market_value_score();
        let away_rep = away_team.reputation.market_value_score();

        let (home_base, away_base, home_competition, away_competition) = if friendly {
            (
                0.1,
                0.1,
                SelectionCompetition::Friendly,
                SelectionCompetition::Friendly,
            )
        } else if let Some((round, total_rounds)) = domestic_cup_round {
            (
                Self::domestic_cup_importance(round, total_rounds, home_rep, away_rep),
                Self::domestic_cup_importance(round, total_rounds, away_rep, home_rep),
                SelectionCompetition::DomesticCup {
                    round,
                    total_rounds,
                    own_reputation: home_rep,
                    opponent_reputation: away_rep,
                },
                SelectionCompetition::DomesticCup {
                    round,
                    total_rounds,
                    own_reputation: away_rep,
                    opponent_reputation: home_rep,
                },
            )
        } else if knockout {
            // Non-cup knockout run through the league scheduler keeps the
            // historical high fixed importance.
            (
                0.9,
                0.9,
                SelectionCompetition::League,
                SelectionCompetition::League,
            )
        } else {
            let imp = Self::calculate_match_importance(table, home_team, away_team, date);
            (
                imp,
                imp,
                SelectionCompetition::League,
                SelectionCompetition::League,
            )
        };

        // Fixture congestion tilt: if a team has another competitive
        // fixture within the next 5 days, dampen this match's importance
        // for them so the rotation/development logic kicks in. Applied
        // per team individually. A domestic cup final is floored so
        // congestion can never rotate it down into a weak XI.
        let congestion_dampen = |ups: u8| -> f32 {
            if ups >= 2 {
                0.55
            } else if ups == 1 {
                0.80
            } else {
                1.0
            }
        };
        let final_floor = |competition: &SelectionCompetition| -> f32 {
            match competition {
                SelectionCompetition::DomesticCup {
                    round,
                    total_rounds,
                    ..
                } if *total_rounds <= 1 || *round >= *total_rounds => {
                    Self::DOMESTIC_CUP_FINAL_IMPORTANCE_FLOOR
                }
                _ => 0.0,
            }
        };
        let home_importance = (home_base * congestion_dampen(upcoming_fixtures.0))
            .max(final_floor(&home_competition));
        let away_importance = (away_base * congestion_dampen(upcoming_fixtures.1))
            .max(final_floor(&away_competition));

        // Surface each team's club philosophy to the selector so
        // DevelopAndSell / LoanFocused sides actually put their archetype
        // on the pitch, not just on paper.
        let home_philosophy: Option<ClubPhilosophy> =
            lookup.club(home_team.club_id).map(|c| c.philosophy.clone());
        let away_philosophy: Option<ClubPhilosophy> =
            lookup.club(away_team.club_id).map(|c| c.philosophy.clone());

        // Surface each side's baseline tactic to the other so a sharp
        // coach can pick a counter. If the team hasn't locked in tactics
        // yet, the enhanced squad builder will fall back to its default.
        let home_baseline = home_team.tactics.as_ref().map(|t| t.tactic_type);
        let away_baseline = away_team.tactics.as_ref().map(|t| t.tactic_type);

        let home_ctx = SelectionContext {
            is_friendly: friendly,
            date,
            match_importance: home_importance,
            philosophy: home_philosophy,
            opponent_tactic: away_baseline,
            competition: home_competition,
            game_model: None,
        };
        let away_ctx = SelectionContext {
            is_friendly: friendly,
            date,
            match_importance: away_importance,
            philosophy: away_philosophy,
            opponent_tactic: home_baseline,
            competition: away_competition,
            game_model: None,
        };
        let selection_ctx = home_ctx;

        let (mut home_squad, mut away_squad) = if friendly {
            let mut home_supplements = Self::collect_supplementary_players(
                clubs,
                home_team.club_id,
                home_team.id,
                friendly,
            );
            let mut away_supplements = Self::collect_supplementary_players(
                clubs,
                away_team.club_id,
                away_team.id,
                friendly,
            );

            let home_overage = Self::collect_overage_development_players(
                clubs,
                home_team.club_id,
                home_team.id,
                &home_team.team_type,
                ctx.simulation.date.date(),
            );
            let away_overage = Self::collect_overage_development_players(
                clubs,
                away_team.club_id,
                away_team.id,
                &away_team.team_type,
                ctx.simulation.date.date(),
            );
            home_supplements.extend(home_overage);
            away_supplements.extend(away_overage);

            (
                home_team.get_rotation_match_squad_with_reserves(&home_supplements, &selection_ctx),
                away_team.get_rotation_match_squad_with_reserves(&away_supplements, &away_ctx),
            )
        } else {
            let home_is_main = home_team.team_type == TeamType::Main;
            let away_is_main = away_team.team_type == TeamType::Main;
            let home_reserves = Self::collect_reserve_players(
                clubs,
                home_team.club_id,
                home_team.id,
                friendly,
                home_is_main,
            );
            let away_reserves = Self::collect_reserve_players(
                clubs,
                away_team.club_id,
                away_team.id,
                friendly,
                away_is_main,
            );
            (
                home_team.get_enhanced_match_squad(&home_reserves, &selection_ctx),
                away_team.get_enhanced_match_squad(&away_reserves, &away_ctx),
            )
        };

        Self::apply_psychological_factors_static(&mut home_squad, home_momentum, home_pressure);
        Self::apply_psychological_factors_static(&mut away_squad, away_momentum, away_pressure);

        if knockout {
            Match::make_knockout(
                scheduled_match.id.clone(),
                scheduled_match.league_id,
                &scheduled_match.league_slug,
                home_squad,
                away_squad,
            )
        } else {
            Match::make(
                scheduled_match.id.clone(),
                scheduled_match.league_id,
                &scheduled_match.league_slug,
                home_squad,
                away_squad,
                friendly,
            )
        }
    }

    /// Collect available reserve players from the same club.
    ///
    /// Sweeps the senior reserves and older youth sides (B / Second / Reserve /
    /// U21 / U23) in full — the pool a manager raids for matchday cover.
    /// Force-selected players from anywhere in the club are added only when the
    /// assembling team is the Main team — the pin is a senior-XI override, so a
    /// U18 starlet flagged for the first team must not also be pulled into the
    /// B-team's reserve pool.
    ///
    /// Outfield borrowing stops there, but a final step guarantees a realistic
    /// backup-goalkeeper candidate: if the assembling team plus the swept
    /// reserves can't field a second available keeper, the deeper academy sides
    /// (U20 → U19 → U18, in that borrowing order) are tapped so the selector can
    /// always name a substitute keeper. Only the keeper gets this deep-squad
    /// rescue — clubs reliably promote a youth keeper for the bench rather than
    /// play an outfielder in goal.
    fn collect_reserve_players<'a>(
        clubs: &'a [Club],
        club_id: u32,
        team_id: u32,
        is_friendly: bool,
        for_main_team: bool,
    ) -> Vec<&'a Player> {
        let Some(club) = clubs.iter().find(|c| c.id == club_id) else {
            return Vec::new();
        };

        let mut reserves: Vec<&'a Player> = if for_main_team {
            club.get_force_selected_players()
                .into_iter()
                .filter(|p| Self::is_player_available(p, is_friendly))
                .collect()
        } else {
            Vec::new()
        };

        for p in club
            .teams
            .teams
            .iter()
            .filter(|t| {
                t.id != team_id
                    && matches!(
                        t.team_type,
                        TeamType::B
                            | TeamType::Second
                            | TeamType::Reserve
                            | TeamType::U21
                            | TeamType::U23
                    )
            })
            .flat_map(|t| t.players.iter())
            .filter(|p| Self::is_player_available(p, is_friendly))
        {
            if reserves.iter().any(|r| r.id == p.id) {
                continue;
            }
            reserves.push(p);
        }

        Self::ensure_backup_goalkeeper_candidate(club, team_id, is_friendly, &mut reserves);

        reserves
    }

    /// Ensure the reserve pool offers a backup goalkeeper when the assembling
    /// team can't cover the bench from its own roster plus the senior/older
    /// reserves already swept in. Counts available keepers on the team and in
    /// `reserves`; if short of a starter-plus-sub pair, borrows the best
    /// available academy keepers from the deeper youth tiers, preferring the
    /// oldest tier first so a U18 is only pulled up as a last resort.
    /// Availability rules (injury, ban, international duty) are respected — an
    /// unavailable keeper is never borrowed.
    fn ensure_backup_goalkeeper_candidate<'a>(
        club: &'a Club,
        team_id: u32,
        is_friendly: bool,
        reserves: &mut Vec<&'a Player>,
    ) {
        // A starter plus one substitute keeper. Borrowing is capped at this so
        // a single missing backup never strips a youth side of its whole keeper
        // corps.
        const GK_BENCH_TARGET: usize = 2;

        let team_keepers = club
            .teams
            .teams
            .iter()
            .find(|t| t.id == team_id)
            .map(|t| {
                t.players
                    .iter()
                    .filter(|p| {
                        p.positions.is_goalkeeper() && Self::is_player_available(p, is_friendly)
                    })
                    .count()
            })
            .unwrap_or(0);
        let reserve_keepers = reserves.iter().filter(|p| p.positions.is_goalkeeper()).count();

        let mut have = team_keepers + reserve_keepers;
        if have >= GK_BENCH_TARGET {
            return;
        }

        // Realistic borrowing order: oldest academy tier first, U18 last.
        for tier in [TeamType::U20, TeamType::U19, TeamType::U18] {
            let mut tier_keepers: Vec<&'a Player> = club
                .teams
                .teams
                .iter()
                .filter(|t| t.id != team_id && t.team_type == tier)
                .flat_map(|t| t.players.iter())
                .filter(|p| {
                    p.positions.is_goalkeeper() && Self::is_player_available(p, is_friendly)
                })
                .filter(|p| !reserves.iter().any(|r| r.id == p.id))
                .collect();
            // Best keeper in the tier first.
            tier_keepers.sort_by(|a, b| {
                b.player_attributes
                    .current_ability
                    .cmp(&a.player_attributes.current_ability)
            });
            for gk in tier_keepers {
                if have >= GK_BENCH_TARGET {
                    return;
                }
                reserves.push(gk);
                have += 1;
            }
            if have >= GK_BENCH_TARGET {
                return;
            }
        }
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
            .flat_map(|t| t.players.iter())
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

        let mut candidates: Vec<&Player> = club
            .teams
            .teams
            .iter()
            .filter(|t| {
                t.id != team_id
                    && matches!(t.team_type, TeamType::U20 | TeamType::U21 | TeamType::U23)
            })
            .flat_map(|t| t.players.iter())
            .filter(|p| {
                Self::is_player_available(p, true)
                    && p.player_attributes.days_since_last_match >= MIN_IDLE_DAYS
                    && p.age(date) <= 23
            })
            .collect();

        candidates.sort_by(|a, b| {
            b.player_attributes
                .days_since_last_match
                .cmp(&a.player_attributes.days_since_last_match)
        });

        candidates.truncate(MAX_OVERAGE_SLOTS);
        candidates
    }

    /// Lower bound a domestic cup final's importance is clamped to after the
    /// congestion dampener. Kept just above the `BestEleven` selection-policy
    /// threshold (0.82) rather than exactly on it, so the final stays a
    /// strong-XI affair even if that threshold or the comparison ever shifts.
    const DOMESTIC_CUP_FINAL_IMPORTANCE_FLOOR: f32 = 0.83;

    /// Domestic-cup match importance for one side's squad selection.
    /// Scales by bracket stage — early rounds rotate hard, semis and finals
    /// demand a strong XI — and nudges up against a stronger opponent.
    /// `own_rep`/`opp_rep` are the two sides' market-value scores. The
    /// result is clamped per stage; the caller then applies the shared
    /// congestion dampener (with a final-stage floor).
    ///
    /// Quarterfinals deliberately share the early/mid clamp band (max 0.62)
    /// rather than getting their own higher band: a "last eight" tie against a
    /// weak opponent should still be rotatable, while the stronger-opponent
    /// nudge can lift it toward managed-minutes / strong-with-rotation.
    fn domestic_cup_importance(round: u8, total_rounds: u8, own_rep: u16, opp_rep: u16) -> f32 {
        let total = total_rounds.max(1);
        let round_progress = round as f32 / total as f32;
        let opponent_ratio = opp_rep as f32 / own_rep.max(1) as f32;

        let is_final = total_rounds <= 1 || round >= total_rounds;
        let is_semi = !is_final && round + 1 == total_rounds;
        let is_quarter = !is_final && !is_semi && round + 2 == total_rounds;

        let stage_base = if is_final {
            0.95
        } else if is_semi {
            0.76
        } else if is_quarter {
            0.58
        } else if round_progress < 0.40 {
            0.30
        } else if round_progress < 0.70 {
            0.42
        } else {
            0.52
        };

        let opponent_adjust = if opponent_ratio < 0.45 {
            -0.08
        } else if opponent_ratio < 0.70 {
            -0.04
        } else if opponent_ratio <= 1.15 {
            0.0
        } else if opponent_ratio <= 1.50 {
            0.08
        } else {
            0.14
        };

        let importance: f32 = stage_base + opponent_adjust;
        if is_final {
            importance.clamp(0.90, 0.98)
        } else if is_semi {
            importance.clamp(0.68, 0.86)
        } else {
            // Quarterfinal shares the early/mid clamp band.
            importance.clamp(0.22, 0.62)
        }
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

        let home_row = table
            .rows
            .iter()
            .enumerate()
            .find(|(_, r)| r.team_id == home_team.id);
        let away_row = table
            .rows
            .iter()
            .enumerate()
            .find(|(_, r)| r.team_id == away_team.id);

        let (home_pos, home_played, home_points) = home_row
            .map(|(i, r)| (i + 1, r.played as f32, r.points as i32))
            .unwrap_or((total_teams / 2, 0.0, 0));

        let away_pos = away_row.map(|(i, _)| i + 1).unwrap_or(total_teams / 2);

        let total_matches = if total_teams > 1 {
            ((total_teams - 1) * 2) as f32
        } else {
            1.0
        };
        let season_progress = (home_played / total_matches).clamp(0.0, 1.0);
        let remaining_matches = (total_matches - home_played).max(0.0) as i32;

        // Points gap to key positions
        let top3_points = table.rows.get(2).map(|r| r.points as i32).unwrap_or(0);
        let relegation_pos = total_teams.saturating_sub(3);
        let relegation_points = table
            .rows
            .get(relegation_pos)
            .map(|r| r.points as i32)
            .unwrap_or(0);
        // Can the team still catch top 3? (3 pts per remaining match)
        let max_reachable = home_points + remaining_matches * 3;
        let can_reach_top3 = max_reachable >= top3_points;

        // Is the team safe from relegation? (gap too large to close)
        let is_safe =
            home_points > relegation_points + remaining_matches * 3 || home_pos <= total_teams / 2;
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
        if player.statuses.is_on_international_duty() {
            return false;
        }
        if !is_friendly && player.player_attributes.is_banned {
            return false;
        }
        true
    }

    pub(in crate::league) fn calculate_match_pressures_static(
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
        let total_teams = table.rows.len();
        // Cup (knockout) leagues have no standings table — guard the
        // position arithmetic so `total_teams - 3` can't underflow and so
        // an empty table doesn't masquerade as "top of the league".
        if total_teams == 0 {
            return 0.5;
        }

        let position = table
            .rows
            .iter()
            .position(|r| r.team_id == team.id)
            .unwrap_or(0);

        let mut pressure: f32 = 0.5;

        if position < 3 {
            pressure += 0.3;
        }

        if position >= total_teams.saturating_sub(3) {
            pressure += 0.4;
        }

        let losing_streak = dynamics.get_team_losing_streak(team.id);
        if losing_streak > 3 {
            pressure += 0.2;
        }

        pressure.min(1.0)
    }

    fn apply_psychological_factors_static(squad: &mut MatchSquad, momentum: f32, pressure: f32) {
        debug!(
            "Team {} - Momentum: {:.2}, Pressure: {:.2}",
            squad.team_name, momentum, pressure
        );
    }
}

#[cfg(test)]
mod tests {
    use super::League;
    use crate::r#match::{SelectionCompetition, SelectionContext, SelectionPolicy};

    fn cup_ctx(
        round: u8,
        total_rounds: u8,
        own: u16,
        opp: u16,
        importance: f32,
    ) -> SelectionContext {
        SelectionContext {
            match_importance: importance,
            competition: SelectionCompetition::DomesticCup {
                round,
                total_rounds,
                own_reputation: own,
                opponent_reputation: opp,
            },
            ..SelectionContext::default()
        }
    }

    #[test]
    fn domestic_cup_early_round_rotates() {
        // Round 1 of 5 against an equal-reputation opponent: low importance,
        // selector falls into CupRotation.
        let imp = League::domestic_cup_importance(1, 5, 1000, 1000);
        assert!(imp < 0.40, "early round importance should be low: {imp}");
        assert_eq!(
            SelectionPolicy::from_context(&cup_ctx(1, 5, 1000, 1000, imp)),
            SelectionPolicy::CupRotation
        );
    }

    #[test]
    fn domestic_cup_final_is_best_eleven() {
        // Round 5 of 5: the final demands the strongest available XI.
        let imp = League::domestic_cup_importance(5, 5, 1000, 1000);
        assert!(imp >= 0.90, "final importance should be high: {imp}");
        assert_eq!(
            SelectionPolicy::from_context(&cup_ctx(5, 5, 1000, 1000, imp)),
            SelectionPolicy::BestEleven
        );
    }

    #[test]
    fn domestic_cup_strong_opponent_raises_importance() {
        // A much stronger opponent (ratio > 1.5) in an early round lifts
        // importance out of full youth-development territory.
        let weak_equal = League::domestic_cup_importance(1, 5, 1000, 1000);
        let strong_opp = League::domestic_cup_importance(1, 5, 1000, 1800);
        assert!(
            strong_opp > weak_equal,
            "stronger opponent must raise importance: {strong_opp} vs {weak_equal}"
        );
        let policy = SelectionPolicy::from_context(&cup_ctx(1, 5, 1000, 1800, strong_opp));
        assert!(
            matches!(
                policy,
                SelectionPolicy::ManagedMinutes | SelectionPolicy::StrongWithRotation
            ),
            "strong opponent early cup should manage minutes, not full youth: {policy:?}"
        );
    }

    #[test]
    fn domestic_cup_importance_rises_monotonically_by_stage() {
        // Equal opponents, 5-round bracket: importance must climb round by
        // round, and the derived policy must walk up the rotation ladder.
        let r1 = League::domestic_cup_importance(1, 5, 1000, 1000);
        let r3 = League::domestic_cup_importance(3, 5, 1000, 1000);
        let r4 = League::domestic_cup_importance(4, 5, 1000, 1000);
        let r5 = League::domestic_cup_importance(5, 5, 1000, 1000);
        assert!(
            r1 < r3 && r3 < r4 && r4 < r5,
            "importance must increase by stage: {r1} {r3} {r4} {r5}"
        );

        assert_eq!(
            SelectionPolicy::from_context(&cup_ctx(1, 5, 1000, 1000, r1)),
            SelectionPolicy::CupRotation
        );
        // Quarterfinal (round 3 of 5) lands in the managed/strong band.
        assert!(matches!(
            SelectionPolicy::from_context(&cup_ctx(3, 5, 1000, 1000, r3)),
            SelectionPolicy::ManagedMinutes | SelectionPolicy::StrongWithRotation
        ));
        assert_eq!(
            SelectionPolicy::from_context(&cup_ctx(4, 5, 1000, 1000, r4)),
            SelectionPolicy::StrongWithRotation
        );
        assert_eq!(
            SelectionPolicy::from_context(&cup_ctx(5, 5, 1000, 1000, r5)),
            SelectionPolicy::BestEleven
        );
    }

    #[test]
    fn domestic_cup_final_floor_holds_under_congestion() {
        // The importance formula caps the final at <=0.98, and the build
        // path floors it above the BestEleven threshold even when congestion
        // would dampen it.
        let final_importance = League::domestic_cup_importance(4, 4, 1000, 1000);
        assert!((0.90..=0.98).contains(&final_importance));
        // Heavy congestion dampener (0.55) would pull it to ~0.52; the floor
        // keeps a final a strong-XI affair.
        let dampened = (final_importance * 0.55).max(League::DOMESTIC_CUP_FINAL_IMPORTANCE_FLOOR);
        assert!(dampened >= 0.83);
        assert_eq!(
            SelectionPolicy::from_context(&cup_ctx(4, 4, 1000, 1000, dampened)),
            SelectionPolicy::BestEleven
        );
    }

    // ========== Backup-goalkeeper borrowing ==========
    //
    // `collect_reserve_players` already sweeps senior reserves and the older
    // youth sides (B/Second/Reserve/U21/U23). These tests cover the deep-squad
    // keeper rescue layered on top: when the assembling team plus those swept
    // reserves still can't field a substitute keeper, the deeper academy tiers
    // (U20 → U19 → U18) are tapped, oldest first and respecting availability.

    use crate::academy::ClubAcademy;
    use crate::shared::Location;
    use crate::{
        Club, ClubColors, ClubFacilities, ClubFinances, ClubStatus, PeopleNameGeneratorData, Player,
        PlayerCollection, PlayerGenerator, PlayerPositionType, StaffCollection, Team, TeamBuilder,
        TeamCollection, TeamReputation, TeamType, TrainingSchedule,
    };
    use chrono::{NaiveTime, Utc};

    fn gk_names() -> PeopleNameGeneratorData {
        PeopleNameGeneratorData {
            first_names: vec!["Test".to_string()],
            last_names: vec!["Keeper".to_string()],
            nicknames: Vec::new(),
        }
    }

    fn md_player(id: u32, position: PlayerPositionType, ability: u8) -> Player {
        let mut p = PlayerGenerator::generate(1, Utc::now().date_naive(), position, 15, &gk_names());
        p.id = id;
        p.player_attributes.current_ability = ability;
        p
    }

    fn md_training() -> TrainingSchedule {
        TrainingSchedule::new(
            NaiveTime::from_hms_opt(10, 0, 0).unwrap(),
            NaiveTime::from_hms_opt(17, 0, 0).unwrap(),
        )
    }

    fn md_team(id: u32, club_id: u32, team_type: TeamType, players: Vec<Player>) -> Team {
        TeamBuilder::new()
            .id(id)
            .league_id(Some(1))
            .club_id(club_id)
            .name(format!("team-{id}"))
            .slug(format!("team-{id}"))
            .team_type(team_type)
            .players(PlayerCollection::new(players))
            .staffs(StaffCollection::new(Vec::new()))
            .reputation(TeamReputation::new(100, 100, 100))
            .training_schedule(md_training())
            .build()
            .unwrap()
    }

    fn md_club(id: u32, teams: Vec<Team>) -> Club {
        Club::new(
            id,
            "Club".to_string(),
            Location::new(1),
            ClubFinances::new(10_000_000, Vec::new()),
            ClubAcademy::new(3),
            ClubStatus::Professional,
            ClubColors::default(),
            TeamCollection::new(teams),
            ClubFacilities::default(),
        )
    }

    fn reserve_has(reserves: &[&Player], id: u32) -> bool {
        reserves.iter().any(|p| p.id == id)
    }

    #[test]
    fn collect_reserve_players_borrows_youth_keeper_when_team_lacks_backup() {
        // First team carries a lone keeper, no senior reserve keeper exists, but
        // the U19 side has one — it should be borrowed into the reserve pool.
        let main = md_team(
            1,
            100,
            TeamType::Main,
            vec![
                md_player(10, PlayerPositionType::Goalkeeper, 150),
                md_player(11, PlayerPositionType::DefenderCenter, 140),
            ],
        );
        let u19 = md_team(
            2,
            100,
            TeamType::U19,
            vec![md_player(20, PlayerPositionType::Goalkeeper, 90)],
        );
        let clubs = vec![md_club(100, vec![main, u19])];

        let reserves = League::collect_reserve_players(&clubs, 100, 1, false, true);
        assert!(
            reserve_has(&reserves, 20),
            "the U19 keeper is borrowed when the first team lacks a backup"
        );
    }

    #[test]
    fn collect_reserve_players_prefers_older_youth_keeper_tier() {
        // Both a U20 and a (higher-CA) U18 keeper are available. The older tier
        // is borrowed first, so the U18 is left untouched — tier beats ability.
        let main = md_team(
            1,
            100,
            TeamType::Main,
            vec![md_player(10, PlayerPositionType::Goalkeeper, 150)],
        );
        let u20 = md_team(
            2,
            100,
            TeamType::U20,
            vec![md_player(20, PlayerPositionType::Goalkeeper, 90)],
        );
        let u18 = md_team(
            3,
            100,
            TeamType::U18,
            vec![md_player(30, PlayerPositionType::Goalkeeper, 95)],
        );
        let clubs = vec![md_club(100, vec![main, u20, u18])];

        let reserves = League::collect_reserve_players(&clubs, 100, 1, false, true);
        assert!(reserve_has(&reserves, 20), "the U20 keeper is preferred");
        assert!(
            !reserve_has(&reserves, 30),
            "the U18 keeper is not borrowed once the U20 covers the bench"
        );
    }

    #[test]
    fn collect_reserve_players_skips_youth_keeper_when_reserve_keeper_present() {
        // A B-team keeper is already swept into the reserves, so the bench is
        // covered and no academy keeper is borrowed.
        let main = md_team(
            1,
            100,
            TeamType::Main,
            vec![md_player(10, PlayerPositionType::Goalkeeper, 150)],
        );
        let b = md_team(
            2,
            100,
            TeamType::B,
            vec![
                md_player(20, PlayerPositionType::Goalkeeper, 120),
                md_player(21, PlayerPositionType::DefenderCenter, 130),
            ],
        );
        let u19 = md_team(
            3,
            100,
            TeamType::U19,
            vec![md_player(30, PlayerPositionType::Goalkeeper, 90)],
        );
        let clubs = vec![md_club(100, vec![main, b, u19])];

        let reserves = League::collect_reserve_players(&clubs, 100, 1, false, true);
        assert!(
            reserve_has(&reserves, 20),
            "the B-team keeper is swept into the reserves"
        );
        assert!(
            !reserve_has(&reserves, 30),
            "no academy keeper is borrowed once a reserve keeper covers the bench"
        );
    }

    #[test]
    fn collect_reserve_players_does_not_borrow_unavailable_youth_keeper() {
        // The only academy keeper is injured — it must not be borrowed, leaving
        // the reserve pool keeper-less rather than naming an unavailable player.
        let main = md_team(
            1,
            100,
            TeamType::Main,
            vec![md_player(10, PlayerPositionType::Goalkeeper, 150)],
        );
        let mut injured = md_player(20, PlayerPositionType::Goalkeeper, 90);
        injured.player_attributes.is_injured = true;
        let u19 = md_team(2, 100, TeamType::U19, vec![injured]);
        let clubs = vec![md_club(100, vec![main, u19])];

        let reserves = League::collect_reserve_players(&clubs, 100, 1, false, true);
        assert!(
            !reserves.iter().any(|p| p.positions.is_goalkeeper()),
            "an injured academy keeper is never borrowed"
        );
    }
}
