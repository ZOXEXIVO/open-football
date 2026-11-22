use crate::club::finance::RevenueModel;
use crate::club::staff::perception::DevelopmentFormEvidence;
use crate::context::GlobalContext;
use crate::league::simulation::context::MatchdaySide;
use crate::league::{
    KnockoutStakes, KnockoutTieReading, League, LeagueMatch, LeagueMatchResultResult, LeagueTable,
    Schedule, TableStakes,
};
use crate::r#match::squad::selection::helpers::PlayerAvailability;
use crate::r#match::{CompetitionKind, FixtureContext, Match, MatchResult, SelectionCompetition};
use crate::{Club, MatchRuntime, Person, Player, Team, TeamType};
use chrono::{Datelike, NaiveDate};
use log::debug;
use rayon::prelude::*;
use rustc_hash::{FxHashMap, FxHashSet};
use std::cmp::Reverse;

/// Per-matchday snapshot of clubs and teams indexed by id. Built once
/// at the top of `play_scheduled_matches` so `build_match` and friends
/// can fan out lookups in O(1) instead of re-scanning every club's
/// team list per fixture.
struct MatchdayLookup<'a> {
    team_by_id: FxHashMap<u32, &'a Team>,
    club_by_id: FxHashMap<u32, &'a Club>,
}

impl<'a> MatchdayLookup<'a> {
    fn build(clubs: &'a [Club]) -> Self {
        let estimated_teams = clubs.len() * 4;
        let mut team_by_id: FxHashMap<u32, &'a Team> =
            FxHashMap::with_capacity_and_hasher(estimated_teams, Default::default());
        let mut club_by_id: FxHashMap<u32, &'a Club> =
            FxHashMap::with_capacity_and_hasher(clubs.len(), Default::default());
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
    /// the played results back and stamps them onto `scheduled_matches`.
    pub(in crate::league) fn build_matchday_matches(
        &self,
        scheduled_matches: &[LeagueMatch],
        clubs: &[Club],
        ctx: &GlobalContext<'_>,
        friendly: bool,
        knockout: bool,
        commitments: &MatchdayCommitments,
    ) -> Vec<Match> {
        let is_cup = self.is_cup;
        let lookup = MatchdayLookup::build(clubs);
        // A table says what a fixture is worth to each side once for the
        // whole matchday; a bracket says it per tie.
        let table_stakes = if friendly || knockout {
            FxHashMap::default()
        } else {
            self.table_stakes()
        };
        // Each fixture's two squads are picked from shared reads only, so a
        // matchday's fixtures are built in parallel; the indexed collect keeps
        // them paired with `scheduled_matches` for `apply_matchday_results`.
        scheduled_matches
            .par_iter()
            .map(|scheduled_match| {
                Self::build_match(
                    scheduled_match,
                    clubs,
                    &lookup,
                    ctx,
                    &self.table,
                    &table_stakes,
                    friendly,
                    knockout,
                    is_cup,
                    self.regulations.bench_size,
                    commitments,
                )
            })
            .collect()
    }

    /// Stamp played results back onto `scheduled_matches` (paired by
    /// index with the order returned from `build_matchday_matches`). Run
    /// after the engine — local or distributed — has returned the matches.
    pub(in crate::league) fn apply_matchday_results(
        &mut self,
        scheduled_matches: &mut [LeagueMatch],
        match_results: &[MatchResult],
    ) {
        for (scheduled_match, result) in scheduled_matches.iter_mut().zip(match_results.iter()) {
            scheduled_match.result = Some(LeagueMatchResultResult::from_score(&result.score));
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
        let commitments = MatchdayCommitments::gather([&self.schedule], ctx.simulation.date.date());
        let matches = self.build_matchday_matches(
            scheduled_matches,
            clubs,
            ctx,
            friendly,
            knockout,
            &commitments,
        );
        let match_results = MatchRuntime::engine_pool().play(matches);
        self.apply_matchday_results(scheduled_matches, &match_results);
        match_results
    }

    #[allow(clippy::too_many_arguments)]
    #[expect(clippy::too_many_lines)]
    fn build_match(
        scheduled_match: &LeagueMatch,
        clubs: &[Club],
        lookup: &MatchdayLookup<'_>,
        ctx: &GlobalContext<'_>,
        table: &LeagueTable,
        table_stakes: &FxHashMap<u32, f32>,
        friendly: bool,
        knockout: bool,
        is_cup: bool,
        bench_size: u8,
        commitments: &MatchdayCommitments,
    ) -> Match {
        let home_team = lookup
            .team(scheduled_match.home_team_id)
            .expect("Home team not found");

        let away_team = lookup
            .team(scheduled_match.away_team_id)
            .expect("Away team not found");

        let date = ctx.simulation.date.date();
        let home_club = lookup.club(home_team.club_id);
        let away_club = lookup.club(away_team.club_id);
        // Either club listing the other as a rival makes the fixture a derby.
        let is_derby = match (home_club, away_club) {
            (Some(h), Some(a)) => h.is_rival(a.id) || a.is_rival(h.id),
            _ => false,
        };

        // A tie with no round on it is the only one there is.
        let bracket = knockout.then(|| {
            let round = scheduled_match.cup_round.unwrap_or(1);
            (round, scheduled_match.cup_total_rounds.unwrap_or(round))
        });
        let home_rep = home_team.reputation.market_value_score();
        let away_rep = away_team.reputation.market_value_score();

        // What the fixture is worth is read for each side on its own: the
        // two clubs stand at different places in a table and face different
        // opponents in a bracket.
        let stakes_of = |team_id: u32, own: u16, other: u16| -> f32 {
            if friendly {
                return MatchdaySide::FRIENDLY_STAKES;
            }
            let stakes = match bracket {
                Some((round, total_rounds)) => KnockoutStakes::of(KnockoutTieReading {
                    round,
                    total_rounds,
                    opponent_ratio: other as f32 / own.max(1) as f32,
                    aggregate_lead: None,
                }),
                None => table_stakes
                    .get(&team_id)
                    .copied()
                    .unwrap_or(TableStakes::ROUTINE),
            };
            if is_derby {
                TableStakes::with_rivalry(stakes)
            } else {
                stakes
            }
        };
        let competition_of = |own: u16, other: u16| match bracket {
            _ if friendly => SelectionCompetition::Friendly,
            Some((round, total_rounds)) if is_cup => SelectionCompetition::DomesticCup {
                round,
                total_rounds,
                own_reputation: own,
                opponent_reputation: other,
            },
            _ => SelectionCompetition::League,
        };
        // Exact season length per side for the development minutes plan —
        // the busiest-player fallback under-reads in a rotated squad.
        let played_in_table = |team_id: u32| -> Option<f32> {
            table
                .rows
                .iter()
                .find(|r| r.team_id == team_id)
                .map(|r| r.played as f32)
        };
        let cup_slug = (is_cup && !friendly).then_some(scheduled_match.league_slug.as_str());
        let home_side = MatchdaySide {
            team: home_team,
            club: home_club,
            opponent: away_team,
            home: true,
            stakes: stakes_of(home_team.id, home_rep, away_rep),
            competition: competition_of(home_rep, away_rep),
            derby: is_derby,
            friendly,
            date,
            season_matches_played: played_in_table(home_team.id),
            cup_slug,
            bench_size,
        };
        let away_side = MatchdaySide {
            team: away_team,
            club: away_club,
            opponent: home_team,
            home: false,
            stakes: stakes_of(away_team.id, away_rep, home_rep),
            competition: competition_of(away_rep, home_rep),
            derby: is_derby,
            friendly,
            date,
            season_matches_played: played_in_table(away_team.id),
            cup_slug,
            bench_size,
        };
        let mut home_ctx = home_side.selection_context();
        let mut away_ctx = away_side.selection_context();

        let (home_squad, away_squad) = if friendly {
            let mut home_supplements = Self::collect_supplementary_players(
                clubs,
                home_team.club_id,
                home_team.id,
                friendly,
                commitments,
            );
            let mut away_supplements = Self::collect_supplementary_players(
                clubs,
                away_team.club_id,
                away_team.id,
                friendly,
                commitments,
            );

            let home_overage = Self::collect_overage_development_players(
                clubs,
                home_team.club_id,
                home_team.id,
                &home_team.team_type,
                ctx.simulation.date.date(),
                commitments,
            );
            let away_overage = Self::collect_overage_development_players(
                clubs,
                away_team.club_id,
                away_team.id,
                &away_team.team_type,
                ctx.simulation.date.date(),
                commitments,
            );
            // Flag the overage picks as development guests so the rotation
            // selector admits them even when the roster is full — they are
            // here for match practice, not shortfall cover.
            // So are the men the club put with the side the day before.
            let guests = |overage: &[&Player], club: Option<&Club>, team_id: u32| -> Vec<u32> {
                overage
                    .iter()
                    .map(|p| p.id)
                    .chain(
                        club.into_iter()
                            .flat_map(|club| commitments.assigned_to(club, team_id))
                            .map(|p| p.id),
                    )
                    .collect()
            };
            home_ctx.development_guest_ids = guests(&home_overage, home_club, home_team.id);
            away_ctx.development_guest_ids = guests(&away_overage, away_club, away_team.id);
            home_supplements.extend(home_overage);
            away_supplements.extend(away_overage);

            (
                home_team.get_rotation_match_squad_with_reserves(&home_supplements, &home_ctx),
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
                date,
                commitments,
            );
            let away_reserves = Self::collect_reserve_players(
                clubs,
                away_team.club_id,
                away_team.id,
                friendly,
                away_is_main,
                date,
                commitments,
            );
            (
                home_team.get_enhanced_match_squad(&home_reserves, &home_ctx),
                away_team.get_enhanced_match_squad(&away_reserves, &away_ctx),
            )
        };

        let competition = if friendly {
            CompetitionKind::Friendly
        } else if is_cup {
            CompetitionKind::DomesticCup
        } else {
            CompetitionKind::League
        };
        let mut fixture = FixtureContext::new(&scheduled_match.id, date, competition, knockout)
            .with_importance((home_side.stakes + away_side.stakes) * 0.5)
            .with_gate(RevenueModel::utilisation(
                home_team.reputation.overall_score(),
                1.0,
                1.0,
            ))
            .with_rivalry(if is_derby { 1.0 } else { 0.0 });
        if let Some(continent) = &ctx.continent {
            fixture = fixture.on_continent(continent.id());
        }
        Match::make(
            scheduled_match.id.clone(),
            scheduled_match.league_id,
            &scheduled_match.league_slug,
            home_squad,
            away_squad,
            fixture,
        )
    }

    /// Resolve the club and hand the fixture to [`MatchdayPool`].
    fn collect_reserve_players<'a>(
        clubs: &'a [Club],
        club_id: u32,
        team_id: u32,
        is_friendly: bool,
        for_main_team: bool,
        date: NaiveDate,
        commitments: &MatchdayCommitments,
    ) -> Vec<&'a Player> {
        let Some(club) = clubs.iter().find(|c| c.id == club_id) else {
            return Vec::new();
        };
        MatchdayPool::offer(club, team_id, is_friendly, for_main_team, date, commitments)
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
        spoken_for: &FxHashSet<u32>,
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
                        p.positions.is_goalkeeper()
                            && Self::is_player_available(p, is_friendly)
                            && !p.move_is_agreed()
                    })
                    .count()
            })
            .unwrap_or(0);
        let reserve_keepers = reserves
            .iter()
            .filter(|p| p.positions.is_goalkeeper())
            .count();

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
                .filter(|p| !spoken_for.contains(&p.id))
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
        commitments: &MatchdayCommitments,
    ) -> Vec<&'a Player> {
        let Some(club) = clubs.iter().find(|c| c.id == club_id) else {
            return Vec::new();
        };
        let spoken_for = commitments.spoken_for(club, team_id);

        club.teams
            .teams
            .iter()
            .filter(|t| t.id != team_id)
            .flat_map(|t| t.players.iter())
            .filter(|p| !spoken_for.contains(&p.id))
            .filter(|p| Self::is_player_available(p, is_friendly))
            .collect()
    }

    /// Collect overage players from higher youth teams who need match practice.
    /// Only applies to U18/U19 teams — allows older youth players (from U20/U21/U23)
    /// who aren't getting matches to gain development time, like real overage rules.
    ///
    /// The quota is position-aware: keepers get a dedicated slot instead of
    /// competing with outfielders in one mixed list. A fixture-less older
    /// squad can hold a large idle keeper corps, and there is only one
    /// goalkeeping shirt per match — a keeper who is offered but not picked
    /// never resets his idle clock, so under a mixed quota the longest-idle
    /// keepers would permanently occupy every slot while the outfielders
    /// behind them (and the keepers behind *them*) never rotate in.
    fn collect_overage_development_players<'a>(
        clubs: &'a [Club],
        club_id: u32,
        team_id: u32,
        team_type: &TeamType,
        date: NaiveDate,
        commitments: &MatchdayCommitments,
    ) -> Vec<&'a Player> {
        const MAX_OVERAGE_OUTFIELD_SLOTS: usize = 3;
        const MAX_OVERAGE_KEEPER_SLOTS: usize = 1;
        const MIN_IDLE_DAYS: u16 = 21;

        if !matches!(team_type, TeamType::U18 | TeamType::U19) {
            return Vec::new();
        }

        let Some(club) = clubs.iter().find(|c| c.id == club_id) else {
            return Vec::new();
        };
        let spoken_for = commitments.spoken_for(club, team_id);

        let mut candidates: Vec<&Player> = club
            .teams
            .teams
            .iter()
            .filter(|t| {
                t.id != team_id
                    && matches!(t.team_type, TeamType::U20 | TeamType::U21 | TeamType::U23)
            })
            .flat_map(|t| t.players.iter())
            .filter(|p| !spoken_for.contains(&p.id))
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

        let mut picked: Vec<&Player> =
            Vec::with_capacity(MAX_OVERAGE_OUTFIELD_SLOTS + MAX_OVERAGE_KEEPER_SLOTS);
        let mut outfield = 0usize;
        let mut keepers = 0usize;
        for p in candidates {
            if p.positions.is_goalkeeper() {
                if keepers < MAX_OVERAGE_KEEPER_SLOTS {
                    picked.push(p);
                    keepers += 1;
                }
            } else if outfield < MAX_OVERAGE_OUTFIELD_SLOTS {
                picked.push(p);
                outfield += 1;
            }
            if outfield == MAX_OVERAGE_OUTFIELD_SLOTS && keepers == MAX_OVERAGE_KEEPER_SLOTS {
                break;
            }
        }
        picked
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
}

/// Every team with a fixture on one date, in whichever competitions the
/// caller read. A player whose own team is in here is spoken for that day
/// and is never lent to another of his club's sides.
#[derive(Debug, Clone, Default)]
pub(crate) struct MatchdayCommitments {
    teams: FxHashSet<u32>,
    date: NaiveDate,
}

impl MatchdayCommitments {
    pub(crate) fn gather<'s>(
        schedules: impl IntoIterator<Item = &'s Schedule>,
        date: NaiveDate,
    ) -> Self {
        MatchdayCommitments {
            teams: schedules
                .into_iter()
                .flat_map(|s| s.teams_playing_on(date))
                .collect(),
            date,
        }
    }

    pub(crate) fn plays(&self, team_id: u32) -> bool {
        self.teams.contains(&team_id)
    }

    /// The players of the club's other sides it put with this one the day
    /// before.
    pub(crate) fn assigned_to<'a>(
        &self,
        club: &'a Club,
        team_id: u32,
    ) -> impl Iterator<Item = &'a Player> + use<'a> {
        let date = self.date;
        club.teams
            .teams
            .iter()
            .filter(move |t| t.id != team_id)
            .flat_map(|t| t.players.iter())
            .filter(move |p| p.assigned_side_on(date) == Some(team_id))
    }

    /// The club's players the assembling side may not borrow today: those
    /// whose own side plays, and — when a more senior side of the club
    /// plays too — everyone, because the club lends to one side a day and
    /// the most senior is served first. A player the club assigned the day
    /// before is with that side and no other, whoever else plays.
    fn spoken_for(&self, club: &Club, assembling_team_id: u32) -> FxHashSet<u32> {
        let rank = |t: &Team| (t.team_type.seniority(), t.id);
        let outranked = club
            .teams
            .teams
            .iter()
            .find(|t| t.id == assembling_team_id)
            .is_some_and(|assembling| {
                club.teams
                    .teams
                    .iter()
                    .any(|t| self.plays(t.id) && rank(t) < rank(assembling))
            });
        club.teams
            .teams
            .iter()
            .flat_map(|t| t.players.iter().map(move |p| (t, p)))
            .filter(|(t, p)| match p.assigned_side_on(self.date) {
                Some(side) => side != assembling_team_id,
                None => t.id != assembling_team_id && (outranked || self.plays(t.id)),
            })
            .map(|(_, p)| p.id)
            .collect()
    }
}

/// Everyone a club may name for one fixture beyond the assembling team's own
/// roster.
///
/// Sweeps the senior reserves and older youth sides (B / Second / Reserve /
/// U21 / U23) in full — the pool a manager raids for matchday cover.
/// Force-selected players from anywhere in the club are added only when the
/// assembling team is the Main team — the pin is a senior-XI override, so a
/// U18 starlet flagged for the first team must not also be pulled into the
/// B-team's reserve pool.
///
/// On top of that sit the three narrower sweeps, each owning one question:
/// [`YouthSeniorCallUp`] (who has earned a look), [`KeeperCallUp`] and
/// [`League::ensure_backup_goalkeeper_candidate`] (the keeper room), and
/// [`MatchdayShortfall`] (can the selector name eighteen at all).
///
/// Every competition that puts a club on a pitch reads this — the league
/// matchday and the four continental brackets — so a club in an injury crisis
/// gets the same bench in Europe that it gets on a Saturday.
pub(crate) struct MatchdayPool;

impl MatchdayPool {
    pub(crate) fn offer<'a>(
        club: &'a Club,
        team_id: u32,
        is_friendly: bool,
        for_main_team: bool,
        date: NaiveDate,
        commitments: &MatchdayCommitments,
    ) -> Vec<&'a Player> {
        let spoken_for = commitments.spoken_for(club, team_id);

        let mut reserves: Vec<&'a Player> = if for_main_team {
            club.get_force_selected_players()
                .into_iter()
                .filter(|p| League::is_player_available(p, is_friendly))
                .filter(|p| !spoken_for.contains(&p.id))
                .collect()
        } else {
            Vec::new()
        };

        for p in commitments.assigned_to(club, team_id) {
            if !reserves.iter().any(|r| r.id == p.id) {
                reserves.push(p);
            }
        }

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
            .filter(|p| League::is_player_available(p, is_friendly))
            .filter(|p| !spoken_for.contains(&p.id))
        {
            if reserves.iter().any(|r| r.id == p.id) {
                continue;
            }
            reserves.push(p);
        }

        // Academy call-ups: U18-U20 outfielders who have earned a place
        // in the senior matchday pool (near-senior observable level, or
        // breakout youth-league form).
        if for_main_team {
            YouthSeniorCallUp::sweep(club, team_id, is_friendly, &spoken_for, &mut reserves);
            // Keepers come up a different way, because the judgement is a
            // different one: not "is he as good as the man ahead of him"
            // — he never is — but "is he ready to be around it". That is
            // the goalkeeping coach's call, and the sweep above stays
            // outfield-only precisely so it does not have to make it.
            KeeperCallUp::sweep(club, team_id, is_friendly, date, &spoken_for, &mut reserves);
        }

        League::ensure_backup_goalkeeper_candidate(
            club,
            team_id,
            is_friendly,
            &spoken_for,
            &mut reserves,
        );
        MatchdayShortfall::sweep(club, team_id, is_friendly, &spoken_for, &mut reserves);

        reserves
    }
}

/// Gated senior call-up sweep: the U18-U20 academy players good enough to
/// train and travel with the first team join the Main matchday reserve
/// pool, where the competitive selector's future-pathway layer decides
/// whether the fixture is right for a cameo (early cup rounds, dead
/// rubbers, congestion). This is the missing first rung of the real-life
/// introduction ladder — stay youth-rostered, get named in the senior
/// squad, collect cameos, and only then earn the permanent move (the
/// cameos feed `PromotionEvidence` in the weekly squad rebalance).
///
/// Visible skills must approach the senior playing standard. Breakout youth
/// form widens the band modestly; it never bypasses readiness. A missing
/// position group uses the senior outfield standard rather than admitting
/// an unready youngster automatically. Capped per matchday so the academy
/// sides keep their squads, and goalkeepers are excluded — GK borrowing stays with
/// the emergency sweep, since a green keeper cameo is a different risk
/// class from an outfield one.
struct YouthSeniorCallUp;

impl YouthSeniorCallUp {
    const TIERS: [TeamType; 3] = [TeamType::U20, TeamType::U19, TeamType::U18];
    /// At most this many academy call-ups join the pool per matchday.
    const MAX_PER_MATCHDAY: usize = 2;
    /// Visible-skill band (1..200 scale) below the senior playing standard.
    const LEVEL_BAND: u8 = 15;
    /// Youth results earn a little more latitude, never an ability exemption.
    const BREAKOUT_LEVEL_BAND: u8 = 25;
    /// Breakout-form arm: a real sample of games...
    const FORM_MIN_GAMES: u16 = 6;
    /// ...at a regressed rating clearly above the ~6.6 positional neutral.
    const FORM_MIN_RATING: f32 = 7.0;

    fn sweep<'a>(
        club: &'a Club,
        main_team_id: u32,
        is_friendly: bool,
        spoken_for: &FxHashSet<u32>,
        reserves: &mut Vec<&'a Player>,
    ) {
        let Some(main) = club.teams.teams.iter().find(|t| t.id == main_team_id) else {
            return;
        };

        use crate::r#match::squad::selection::helpers::CallUpReadiness;
        let senior_roster = main.players.players();

        let mut candidates: Vec<(&'a Player, u8)> = Vec::new();
        for team in club
            .teams
            .teams
            .iter()
            .filter(|t| t.id != main_team_id && Self::TIERS.contains(&t.team_type))
        {
            for p in team.players.iter() {
                if p.positions.is_goalkeeper() {
                    continue;
                }
                if !League::is_player_available(p, is_friendly) {
                    continue;
                }
                if spoken_for.contains(&p.id) || reserves.iter().any(|r| r.id == p.id) {
                    continue;
                }
                let level = CallUpReadiness::level(p);
                let band = if Self::breakout_form(p) {
                    Self::BREAKOUT_LEVEL_BAND
                } else {
                    Self::LEVEL_BAND
                };
                if CallUpReadiness::meets_level(p, &senior_roster, band) {
                    candidates.push((p, level));
                }
            }
        }

        candidates.sort_by_key(|c| Reverse(c.1));
        for (p, _) in candidates.into_iter().take(Self::MAX_PER_MATCHDAY) {
            reserves.push(p);
        }
    }

    /// Breakout youth-league form: judged on the season bucket that
    /// actually carries the player's football (friendly bucket for youth
    /// leagues), reliability-regressed so a hot streak can't trigger it.
    fn breakout_form(player: &Player) -> bool {
        if DevelopmentFormEvidence::games(player) < Self::FORM_MIN_GAMES {
            return false;
        }
        DevelopmentFormEvidence::regressed_rating(player) >= Self::FORM_MIN_RATING
    }
}

/// Fill the matchday pool before optimising it.
///
/// [`YouthSeniorCallUp`] is an *opportunity* mechanism — two academy players
/// a matchday, the ones who have earned a look at the first team. For a club
/// whose only other sides are U18/U20 it is also the only route into a senior
/// matchday squad that exists, and those are not the same job. A first team
/// missing half its squad to injury and international duty does not go out
/// with three substitutes because only two boys were call-up ready; it names
/// whoever is fit and worries about who deserved it afterwards.
///
/// So once every other sweep has run, whatever the pool is still short of a
/// full eighteen is borrowed from the academy, oldest tier first and best
/// first inside a tier — and only the shortfall, so a well-stocked senior
/// squad borrows nobody and a thin one never takes more from the academy side
/// than it actually needs. Same shape as
/// [`ensure_backup_goalkeeper_candidate`](League::ensure_backup_goalkeeper_candidate),
/// which has always done exactly this for the one position where going short
/// was unmissable — and which keeps the keeper question, so this sweep is
/// outfield-only for the same reason [`YouthSeniorCallUp`] is.
struct MatchdayShortfall;

impl MatchdayShortfall {
    /// A starting eleven plus a full bench — what the selector is asked for
    /// and what it silently gave up on when the pool could not reach it.
    const MATCHDAY_SQUAD: usize = 18;

    /// The selector's own availability read, not the matchday one.
    ///
    /// This sweep exists to answer "will the selector be able to name
    /// eighteen", so it has to count the players the selector will count:
    /// [`League::is_player_available`] knows nothing about the hard condition
    /// floor that drops a man on an empty tank, and counting him would leave
    /// the bench short by exactly the number of walking wounded on the
    /// roster.
    fn fit(player: &Player, is_friendly: bool) -> bool {
        PlayerAvailability::is_available(player, is_friendly)
    }

    /// Borrowing order: the oldest academy tier first, U18 last.
    const TIERS: [TeamType; 5] = [
        TeamType::U23,
        TeamType::U21,
        TeamType::U20,
        TeamType::U19,
        TeamType::U18,
    ];

    fn sweep<'a>(
        club: &'a Club,
        team_id: u32,
        is_friendly: bool,
        spoken_for: &FxHashSet<u32>,
        reserves: &mut Vec<&'a Player>,
    ) {
        let mut offered: FxHashSet<u32> = reserves.iter().map(|p| p.id).collect();
        let own = club
            .teams
            .teams
            .iter()
            .find(|t| t.id == team_id)
            .map(|t| {
                t.players
                    .iter()
                    .filter(|p| Self::fit(p, is_friendly))
                    .filter(|p| !offered.contains(&p.id))
                    .count()
            })
            .unwrap_or(0);

        let mut have = own + offered.len();
        if have >= Self::MATCHDAY_SQUAD {
            return;
        }

        for tier in Self::TIERS {
            let mut pool: Vec<&'a Player> = club
                .teams
                .teams
                .iter()
                .filter(|t| t.id != team_id && t.team_type == tier)
                .flat_map(|t| t.players.iter())
                .filter(|p| !p.positions.is_goalkeeper())
                .filter(|p| Self::fit(p, is_friendly))
                .filter(|p| !offered.contains(&p.id) && !spoken_for.contains(&p.id))
                .collect();
            pool.sort_by(|a, b| {
                b.player_attributes
                    .current_ability
                    .cmp(&a.player_attributes.current_ability)
            });
            for player in pool {
                if have >= Self::MATCHDAY_SQUAD {
                    return;
                }
                offered.insert(player.id);
                reserves.push(player);
                have += 1;
            }
        }
    }
}

/// Academy keepers the goalkeeping department wants around the first team.
///
/// The outfield sweep asks whether a boy is nearly as good as the men ahead
/// of him. For a keeper that question has no useful answer — he is one of
/// three or four in the whole club and he is never nearly as good — so the
/// sweep above leaves keepers alone and this one asks the question a
/// goalkeeping coach actually asks: is he ready to be around it.
///
/// The answer is already in the department's plan, revised monthly across
/// every squad the club owns, so all this does is carry the named keepers
/// into the senior matchday pool. Without a plan it does nothing, and the
/// only route into a senior squad for an academy keeper stays what it was:
/// the emergency borrow below.
struct KeeperCallUp;

impl KeeperCallUp {
    /// At most one academy keeper joins the senior pool per matchday. A
    /// second is of no use to anybody — there is one shirt and one place on
    /// the bench — and it would strip the academy side of its own keeper.
    const MAX_PER_MATCHDAY: usize = 1;

    fn sweep<'a>(
        club: &'a Club,
        main_team_id: u32,
        is_friendly: bool,
        date: NaiveDate,
        spoken_for: &FxHashSet<u32>,
        reserves: &mut Vec<&'a Player>,
    ) {
        let called_up: Vec<&'a Player> = club
            .keeper_call_ups(main_team_id, date)
            .into_iter()
            .filter(|p| League::is_player_available(p, is_friendly))
            .filter(|p| !spoken_for.contains(&p.id))
            .filter(|p| !reserves.iter().any(|r| r.id == p.id))
            .take(Self::MAX_PER_MATCHDAY)
            .collect();
        reserves.extend(called_up);
    }
}

#[cfg(test)]
mod tests {
    use super::{League, MatchdayCommitments};

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
        Club, ClubColors, ClubFacilities, ClubFinances, ClubStatus, PeopleNameGeneratorData,
        Player, PlayerCollection, PlayerGenerator, PlayerPositionType, PlayerSkills,
        StaffCollection, Team, TeamBuilder, TeamCollection, TeamReputation, TeamType,
        TrainingSchedule,
    };
    use chrono::{Datelike, NaiveDate, NaiveTime, Utc};

    fn gk_names() -> PeopleNameGeneratorData {
        PeopleNameGeneratorData {
            first_names: vec!["Test".to_string()],
            last_names: vec!["Keeper".to_string()],
            nicknames: Vec::new(),
        }
    }

    fn md_date() -> NaiveDate {
        Utc::now().date_naive()
    }

    fn md_player(id: u32, position: PlayerPositionType, ability: u8) -> Player {
        let mut p =
            PlayerGenerator::generate(1, Utc::now().date_naive(), position, 15, &gk_names());
        p.id = id;
        p.player_attributes.current_ability = ability;
        // The generator's academy skills are random — stamp them from the
        // intended ability so observable-level reads (the call-up gate)
        // order fixture players the way the test means them to be ordered.
        p.skills = PlayerSkills::flat_for_ability(ability);
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

        let reserves = League::collect_reserve_players(
            &clubs,
            100,
            1,
            false,
            true,
            md_date(),
            &MatchdayCommitments::default(),
        );
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

        let reserves = League::collect_reserve_players(
            &clubs,
            100,
            1,
            false,
            true,
            md_date(),
            &MatchdayCommitments::default(),
        );
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

        let reserves = League::collect_reserve_players(
            &clubs,
            100,
            1,
            false,
            true,
            md_date(),
            &MatchdayCommitments::default(),
        );
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

        let reserves = League::collect_reserve_players(
            &clubs,
            100,
            1,
            false,
            true,
            md_date(),
            &MatchdayCommitments::default(),
        );
        assert!(
            !reserves.iter().any(|p| p.positions.is_goalkeeper()),
            "an injured academy keeper is never borrowed"
        );
    }

    // ========== Overage development sweep (U20+ → U18/U19 dev fixtures) ==========

    fn idle_u20_player(
        id: u32,
        position: PlayerPositionType,
        idle_days: u16,
        date: NaiveDate,
    ) -> Player {
        let mut p = md_player(id, position, 100);
        p.player_attributes.days_since_last_match = idle_days;
        p.birth_date = NaiveDate::from_ymd_opt(date.year() - 21, 1, 1).unwrap();
        p
    }

    #[test]
    fn overage_development_sweep_caps_keepers_to_a_dedicated_slot() {
        // A fixture-less U20 squad holding ten idle keepers and four idle
        // outfielders. There is one goalkeeping shirt per match and an
        // offered-but-unpicked keeper never resets his idle clock, so under
        // a mixed longest-idle quota the keeper corps would permanently
        // occupy every slot. The sweep must offer exactly one keeper (the
        // longest-idle) plus three outfielders.
        let date = Utc::now().date_naive();
        let u18 = md_team(1, 100, TeamType::U18, Vec::new());
        let mut u20_players = Vec::new();
        for i in 0..10u32 {
            u20_players.push(idle_u20_player(
                200 + i,
                PlayerPositionType::Goalkeeper,
                60 - i as u16,
                date,
            ));
        }
        for i in 0..4u32 {
            u20_players.push(idle_u20_player(
                300 + i,
                PlayerPositionType::MidfielderCenter,
                30 + i as u16,
                date,
            ));
        }
        let u20 = md_team(2, 100, TeamType::U20, u20_players);
        let clubs = vec![md_club(100, vec![u18, u20])];

        let picked = League::collect_overage_development_players(
            &clubs,
            100,
            1,
            &TeamType::U18,
            date,
            &MatchdayCommitments::default(),
        );

        let keepers = picked
            .iter()
            .filter(|p| p.positions.is_goalkeeper())
            .count();
        assert_eq!(keepers, 1, "exactly one keeper takes the dedicated slot");
        assert_eq!(
            picked.len() - keepers,
            3,
            "three outfield slots are filled despite ten longer-idle keepers"
        );
        assert!(
            picked.iter().any(|p| p.id == 200),
            "the longest-idle keeper is the one offered"
        );
    }

    #[test]
    fn ready_youth_outfielder_joins_main_reserve_pool() {
        // A U19 midfielder within the observable band of the weakest senior
        // peer is called into the senior matchday pool; a clearly-below kid
        // with no form case is not. The first team is a full eighteen so the
        // shortfall sweep stays out of it and the gate is what is measured.
        let main_players: Vec<Player> = (1..=18)
            .map(|id| md_player(id, PlayerPositionType::MidfielderCenter, 100))
            .collect();
        let main = md_team(1, 100, TeamType::Main, main_players);
        let u19 = md_team(
            2,
            100,
            TeamType::U19,
            vec![
                md_player(20, PlayerPositionType::MidfielderCenter, 95),
                md_player(21, PlayerPositionType::MidfielderCenter, 40),
            ],
        );
        let clubs = vec![md_club(100, vec![main, u19])];

        let reserves = League::collect_reserve_players(
            &clubs,
            100,
            1,
            false,
            true,
            md_date(),
            &MatchdayCommitments::default(),
        );
        assert!(
            reserve_has(&reserves, 20),
            "a near-senior U19 midfielder earns a call-up to the senior pool"
        );
        assert!(
            !reserve_has(&reserves, 21),
            "a kid far below senior level with no form case stays with the U19s"
        );
    }

    #[test]
    fn youth_call_ups_are_capped_per_matchday() {
        // A full first team: nothing is short, so the only thing bringing
        // academy players in is the opportunity cap this test is about.
        let main_players: Vec<Player> = (1..=18)
            .map(|id| md_player(id, PlayerPositionType::MidfielderCenter, 100))
            .collect();
        let main = md_team(1, 100, TeamType::Main, main_players);
        let u19 = md_team(
            2,
            100,
            TeamType::U19,
            vec![
                md_player(30, PlayerPositionType::MidfielderCenter, 95),
                md_player(31, PlayerPositionType::MidfielderCenter, 96),
                md_player(32, PlayerPositionType::MidfielderCenter, 97),
            ],
        );
        let clubs = vec![md_club(100, vec![main, u19])];

        let reserves = League::collect_reserve_players(
            &clubs,
            100,
            1,
            false,
            true,
            md_date(),
            &MatchdayCommitments::default(),
        );
        let called_up = reserves
            .iter()
            .filter(|p| (30..=32).contains(&p.id))
            .count();
        assert_eq!(
            called_up, 2,
            "academy call-ups are capped so the youth side keeps its squad"
        );
    }

    #[test]
    fn breakout_youth_form_earns_call_up() {
        // Breakout youth form modestly widens the skill band; it cannot
        // make a player far below the senior level ready.
        let main_players: Vec<Player> = (1..=18)
            .map(|id| md_player(id, PlayerPositionType::MidfielderCenter, 130))
            .collect();
        let main = md_team(1, 100, TeamType::Main, main_players);
        let mut breakout = md_player(40, PlayerPositionType::MidfielderCenter, 110);
        breakout.friendly_statistics.played = 12;
        for _ in 0..12 {
            breakout
                .friendly_statistics
                .record_match_rating(8.0, 90, true);
        }
        let quiet = md_player(41, PlayerPositionType::MidfielderCenter, 110);
        let mut unready = md_player(42, PlayerPositionType::MidfielderCenter, 60);
        unready.friendly_statistics = breakout.friendly_statistics.clone();
        let u19 = md_team(2, 100, TeamType::U19, vec![breakout, quiet, unready]);
        let clubs = vec![md_club(100, vec![main, u19])];

        let reserves = League::collect_reserve_players(
            &clubs,
            100,
            1,
            false,
            true,
            md_date(),
            &MatchdayCommitments::default(),
        );
        assert!(
            reserve_has(&reserves, 40),
            "a breakout youth-league season earns the senior call-up"
        );
        assert!(
            !reserve_has(&reserves, 41),
            "the same level without the form case does not"
        );
        assert!(
            !reserve_has(&reserves, 42),
            "youth form cannot bridge a large skill gap"
        );
    }

    #[test]
    fn a_short_first_team_calls_up_past_the_opportunity_cap() {
        // The reported bench: a first team with twelve fit bodies and an
        // academy that only produced two "ready" boys went out with three
        // substitutes and could make two changes all afternoon. The cap is an
        // opportunity rule, not a rule about how many players a club may name.
        let main_players: Vec<Player> = (1..=12)
            .map(|id| md_player(id, PlayerPositionType::MidfielderCenter, 150))
            .collect();
        let main = md_team(1, 100, TeamType::Main, main_players);
        let u20_players: Vec<Player> = (20..=30)
            .map(|id| md_player(id, PlayerPositionType::MidfielderCenter, 60))
            .collect();
        let u20 = md_team(2, 100, TeamType::U20, u20_players);
        let clubs = vec![md_club(100, vec![main, u20])];

        let reserves = League::collect_reserve_players(
            &clubs,
            100,
            1,
            false,
            true,
            md_date(),
            &MatchdayCommitments::default(),
        );
        assert_eq!(
            reserves.len(),
            6,
            "twelve fit seniors plus the borrowed shortfall make an eighteen"
        );
    }

    #[test]
    fn a_full_first_team_borrows_nobody() {
        // The other side of the same rule: a club that can name eighteen of
        // its own takes nothing from the academy beyond the two who earned it.
        let main_players: Vec<Player> = (1..=20)
            .map(|id| md_player(id, PlayerPositionType::MidfielderCenter, 150))
            .collect();
        let main = md_team(1, 100, TeamType::Main, main_players);
        let u20_players: Vec<Player> = (20..=30)
            .map(|id| md_player(id, PlayerPositionType::MidfielderCenter, 60))
            .collect();
        let u20 = md_team(2, 100, TeamType::U20, u20_players);
        let clubs = vec![md_club(100, vec![main, u20])];

        let reserves = League::collect_reserve_players(
            &clubs,
            100,
            1,
            false,
            true,
            md_date(),
            &MatchdayCommitments::default(),
        );
        assert!(
            reserves.is_empty(),
            "a stocked first team borrows nobody; got {}",
            reserves.len()
        );
    }

    #[test]
    fn youth_keepers_are_not_called_up_as_outfield_reserves() {
        // Keeper call-ups belong to the emergency-borrow path; the academy
        // sweep must never pull a keeper on the outfield gate. Main already
        // has two fit keepers, so the emergency path stays quiet too.
        let main = md_team(
            1,
            100,
            TeamType::Main,
            vec![
                md_player(1, PlayerPositionType::Goalkeeper, 100),
                md_player(2, PlayerPositionType::Goalkeeper, 100),
                md_player(3, PlayerPositionType::MidfielderCenter, 100),
            ],
        );
        let u19 = md_team(
            2,
            100,
            TeamType::U19,
            vec![md_player(20, PlayerPositionType::Goalkeeper, 99)],
        );
        let clubs = vec![md_club(100, vec![main, u19])];

        let reserves = League::collect_reserve_players(
            &clubs,
            100,
            1,
            false,
            true,
            md_date(),
            &MatchdayCommitments::default(),
        );
        assert!(
            !reserve_has(&reserves, 20),
            "a youth keeper is not part of the outfield call-up sweep"
        );
    }

    /// The seam between the club's goalkeeping department and the matchday
    /// squad: the boy the specialist has asked for reaches the senior pool
    /// even though the first team is not short of keepers, which is the one
    /// thing the emergency borrow above can never do.
    #[test]
    fn the_goalkeeping_department_calls_its_prospect_into_the_senior_pool() {
        let mut keeper_one = md_player(10, PlayerPositionType::Goalkeeper, 150);
        keeper_one.birth_date = md_date() - chrono::Duration::days(365 * 34);
        let mut keeper_two = md_player(11, PlayerPositionType::Goalkeeper, 138);
        keeper_two.birth_date = md_date() - chrono::Duration::days(365 * 28);
        let mut keeper_three = md_player(12, PlayerPositionType::Goalkeeper, 118);
        keeper_three.birth_date = md_date() - chrono::Duration::days(365 * 37);
        let mut prospect = md_player(20, PlayerPositionType::Goalkeeper, 108);
        prospect.birth_date = md_date() - chrono::Duration::days(365 * 18);
        prospect.attributes.professionalism = 15.0;
        prospect.attributes.ambition = 15.0;

        let mut main = md_team(
            1,
            100,
            TeamType::Main,
            vec![
                keeper_one,
                keeper_two,
                keeper_three,
                md_player(13, PlayerPositionType::DefenderCenter, 140),
            ],
        );
        main.staffs = StaffCollection::new(vec![md_goalkeeping_coach(900)]);
        let u18 = md_team(2, 100, TeamType::U18, vec![prospect]);

        let mut club = md_club(100, vec![main, u18]);
        club.review_goalkeeping_department(md_date());
        assert_eq!(
            club.keeper_plan().and_then(|p| p.nominated(md_date())),
            Some(20),
            "the department asks for the boy"
        );

        let clubs = vec![club];
        let reserves = League::collect_reserve_players(
            &clubs,
            100,
            1,
            false,
            true,
            md_date(),
            &MatchdayCommitments::default(),
        );
        assert!(
            reserve_has(&reserves, 20),
            "and he reaches the first team's matchday pool with three fit keepers ahead of him"
        );
    }

    // ========== Same-day commitments ==========

    fn md_commitments(team_ids: &[u32]) -> MatchdayCommitments {
        use crate::league::{Schedule, ScheduleItem, ScheduleTour};
        let kickoff = md_date().and_hms_opt(15, 0, 0).unwrap();
        let mut tour = ScheduleTour::new(1, team_ids.len());
        for (i, team_id) in team_ids.iter().enumerate() {
            tour.items.push(ScheduleItem::new(
                1,
                "l".into(),
                *team_id,
                9_000 + i as u32,
                kickoff,
                None,
            ));
        }
        MatchdayCommitments::gather([&Schedule { tours: vec![tour] }], md_date())
    }

    #[test]
    fn commitments_gather_every_team_playing_that_day_across_schedules() {
        use crate::league::{Schedule, ScheduleItem, ScheduleTour};
        let day = md_date();
        let fixture = |home: u32, away: u32, date: NaiveDate| {
            let mut tour = ScheduleTour::new(1, 1);
            tour.items.push(ScheduleItem::new(
                1,
                "l".into(),
                home,
                away,
                date.and_hms_opt(15, 0, 0).unwrap(),
                None,
            ));
            Schedule { tours: vec![tour] }
        };
        let league = fixture(1, 2, day);
        let cup = fixture(3, 4, day);
        let tomorrow = fixture(5, 6, day + chrono::Duration::days(1));

        let commitments = MatchdayCommitments::gather([&league, &cup, &tomorrow], day);
        for team in [1, 2, 3, 4] {
            assert!(commitments.plays(team), "team {team} plays today");
        }
        assert!(!commitments.plays(5) && !commitments.plays(6));
    }

    fn b_team_club() -> Vec<Club> {
        let main_players: Vec<Player> = (1..=12)
            .map(|id| md_player(id, PlayerPositionType::MidfielderCenter, 150))
            .collect();
        let main = md_team(1, 100, TeamType::Main, main_players);
        let b_players: Vec<Player> = (20..=30)
            .map(|id| md_player(id, PlayerPositionType::MidfielderCenter, 120))
            .collect();
        let b = md_team(2, 100, TeamType::Second, b_players);
        vec![md_club(100, vec![main, b])]
    }

    #[test]
    fn a_b_team_playing_today_lends_nobody_to_the_first_team() {
        let clubs = b_team_club();
        let reserves = League::collect_reserve_players(
            &clubs,
            100,
            1,
            false,
            true,
            md_date(),
            &md_commitments(&[1, 2]),
        );
        assert!(
            !(20..=30).any(|id| reserve_has(&reserves, id)),
            "the second team's men play their own match today"
        );
    }

    #[test]
    fn an_idle_b_team_lends_to_the_first_team_as_before() {
        let clubs = b_team_club();
        let reserves = League::collect_reserve_players(
            &clubs,
            100,
            1,
            false,
            true,
            md_date(),
            &md_commitments(&[1]),
        );
        assert!(
            (20..=30).all(|id| reserve_has(&reserves, id)),
            "a second team with no fixture is the first team's reserve pool"
        );
    }

    #[test]
    fn the_shortfall_borrow_skips_a_youth_side_playing_today() {
        let main_players: Vec<Player> = (1..=10)
            .map(|id| md_player(id, PlayerPositionType::MidfielderCenter, 150))
            .collect();
        let main = md_team(1, 100, TeamType::Main, main_players);
        let u19_players: Vec<Player> = (20..=30)
            .map(|id| md_player(id, PlayerPositionType::MidfielderCenter, 70))
            .collect();
        let u19 = md_team(3, 100, TeamType::U19, u19_players);
        let u18_players: Vec<Player> = (40..=50)
            .map(|id| md_player(id, PlayerPositionType::MidfielderCenter, 60))
            .collect();
        let u18 = md_team(4, 100, TeamType::U18, u18_players);
        let clubs = vec![md_club(100, vec![main, u19, u18])];

        let reserves = League::collect_reserve_players(
            &clubs,
            100,
            1,
            false,
            true,
            md_date(),
            &md_commitments(&[1, 3]),
        );
        assert!(
            !(20..=30).any(|id| reserve_has(&reserves, id)),
            "the U19 plays its own fixture today"
        );
        assert_eq!(
            reserves.len(),
            8,
            "ten fit seniors plus eight idle U18s make the eighteen"
        );
        assert!(reserves.iter().all(|p| (40..=50).contains(&p.id)));
    }

    fn first_and_second_with_an_idle_u20() -> Vec<Club> {
        let main_players: Vec<Player> = (1..=10)
            .map(|id| md_player(id, PlayerPositionType::MidfielderCenter, 150))
            .collect();
        let main = md_team(1, 100, TeamType::Main, main_players);
        let second_players: Vec<Player> = (20..=29)
            .map(|id| md_player(id, PlayerPositionType::MidfielderCenter, 120))
            .collect();
        let second = md_team(2, 100, TeamType::Second, second_players);
        let u20_players: Vec<Player> = (40..=50)
            .map(|id| md_player(id, PlayerPositionType::MidfielderCenter, 70))
            .collect();
        let u20 = md_team(3, 100, TeamType::U20, u20_players);
        vec![md_club(100, vec![main, second, u20])]
    }

    #[test]
    fn on_a_shared_day_only_the_first_team_borrows_the_idle_academy() {
        let clubs = first_and_second_with_an_idle_u20();
        let both_play = md_commitments(&[1, 2]);

        let first =
            League::collect_reserve_players(&clubs, 100, 1, false, true, md_date(), &both_play);
        assert!(
            (40..=50).any(|id| reserve_has(&first, id)),
            "the first team fills its bench from the idle U20"
        );

        let second =
            League::collect_reserve_players(&clubs, 100, 2, false, false, md_date(), &both_play);
        assert!(
            second.is_empty(),
            "the second side plays with its own men when the first team plays too"
        );
    }

    #[test]
    fn a_second_side_playing_alone_borrows_the_idle_academy() {
        let clubs = first_and_second_with_an_idle_u20();
        let second = League::collect_reserve_players(
            &clubs,
            100,
            2,
            false,
            false,
            md_date(),
            &md_commitments(&[2]),
        );
        assert!(
            (40..=50).any(|id| reserve_has(&second, id)),
            "with the first team idle, the second side's shortfall reaches the U20"
        );
    }

    #[test]
    fn a_youth_side_gets_no_first_team_supplement_on_a_first_team_matchday() {
        let main_players: Vec<Player> = (1..=18)
            .map(|id| md_player(id, PlayerPositionType::MidfielderCenter, 150))
            .collect();
        let main = md_team(1, 100, TeamType::Main, main_players);
        let u20_players: Vec<Player> = (20..=25)
            .map(|id| md_player(id, PlayerPositionType::MidfielderCenter, 80))
            .collect();
        let u20 = md_team(2, 100, TeamType::U20, u20_players);
        let u18 = md_team(
            3,
            100,
            TeamType::U18,
            vec![md_player(40, PlayerPositionType::MidfielderCenter, 60)],
        );
        let clubs = vec![md_club(100, vec![main, u20, u18])];

        let supplements =
            League::collect_supplementary_players(&clubs, 100, 3, true, &md_commitments(&[1, 3]));
        assert!(
            supplements.is_empty(),
            "the first team plays today, so it alone borrows; the U18 plays with its own"
        );

        let idle_day =
            League::collect_supplementary_players(&clubs, 100, 3, true, &md_commitments(&[3]));
        assert!((1..=18).all(|id| reserve_has(&idle_day, id)));
        assert!(
            (20..=25).all(|id| reserve_has(&idle_day, id)),
            "on its own matchday the U18 is supplemented from every idle squad"
        );
    }

    /// A goalkeeping coach good enough to be listened to, plus the manager
    /// whose seat the plan otherwise falls back to.
    fn md_goalkeeping_coach(id: u32) -> crate::Staff {
        use crate::{StaffClubContract, StaffPosition, StaffStatus, StaffStub};
        let mut staff = StaffStub::build();
        staff.id = id;
        staff.contract = Some(StaffClubContract::new(
            50_000,
            md_date() + chrono::Duration::days(900),
            StaffPosition::GoalkeeperCoach,
            StaffStatus::Active,
        ));
        let gk = &mut staff.staff_attributes.goalkeeping;
        gk.shot_stopping = 18;
        gk.handling = 18;
        gk.distribution = 16;
        let knowledge = &mut staff.staff_attributes.knowledge;
        knowledge.judging_player_ability = 17;
        knowledge.judging_player_potential = 17;
        let mental = &mut staff.staff_attributes.mental;
        mental.man_management = 16;
        mental.adaptability = 15;
        staff
    }

    // ========== What each side's selection is told ==========

    use super::MatchdayLookup;
    use crate::context::{GlobalContext, SimulationContext};
    use crate::league::{
        KnockoutStakes, KnockoutTieReading, LeagueMatch, LeagueTable, LeagueTableRow, TableStakes,
    };
    use crate::r#match::{FieldSquad, Match};
    use rustc_hash::FxHashMap;

    fn md_full_squad(base: u32, ability: u8) -> Vec<Player> {
        use PlayerPositionType::*;
        [
            Goalkeeper,
            Goalkeeper,
            DefenderLeft,
            DefenderCenterLeft,
            DefenderCenterRight,
            DefenderRight,
            DefenderCenter,
            MidfielderLeft,
            MidfielderCenterLeft,
            MidfielderCenterRight,
            MidfielderRight,
            MidfielderCenter,
            ForwardLeft,
            ForwardRight,
            Striker,
            Striker,
            DefenderLeft,
            MidfielderCenter,
        ]
        .iter()
        .enumerate()
        .map(|(i, &position)| md_player(base + i as u32, position, ability))
        .collect()
    }

    fn md_first_team(id: u32, club_id: u32, reputation: u16) -> Team {
        let mut team = md_team(id, club_id, TeamType::Main, md_full_squad(id * 100, 120));
        team.reputation = TeamReputation::new(reputation, reputation, reputation);
        team
    }

    fn md_fixture(home: u32, away: u32, cup_round: Option<(u8, u8)>) -> LeagueMatch {
        LeagueMatch {
            id: format!("md_{home}_{away}"),
            league_id: 1,
            league_slug: "league".to_string(),
            date: md_date().and_hms_opt(15, 0, 0).unwrap(),
            home_team_id: home,
            away_team_id: away,
            result: None,
            cup_round: cup_round.map(|(round, _)| round),
            cup_total_rounds: cup_round.map(|(_, total)| total),
        }
    }

    fn md_row(team_id: u32, played: u8, points: u8) -> LeagueTableRow {
        LeagueTableRow {
            team_id,
            played,
            win: points / 3,
            draft: points % 3,
            lost: played.saturating_sub(points / 3 + points % 3),
            goal_scored: 0,
            goal_concerned: 0,
            points,
            points_deduction: 0,
        }
    }

    /// A top division of twenty with three relegated and seven continental
    /// places, `rounds_left` of its 38 rounds still to play.
    fn md_league(table: LeagueTable, rounds_left: u8) -> League {
        use crate::league::{
            DayMonthPeriod, LeagueSettings, Schedule, ScheduleItem, ScheduleTour, TableLine,
        };
        let mut league = League::new(
            1,
            "League".to_string(),
            "league".to_string(),
            1,
            7000,
            LeagueSettings {
                season_starting_half: DayMonthPeriod::new(1, 8, 31, 12),
                season_ending_half: DayMonthPeriod::new(1, 1, 31, 5),
                tier: 1,
                promotion_spots: 0,
                relegation_spots: 3,
                league_group: None,
                split_season: false,
            },
            false,
        );
        league.continental_lines = vec![TableLine::below(4, 0.95), TableLine::below(7, 0.8)];
        let teams: Vec<u32> = table.rows.iter().map(|r| r.team_id).collect();
        let mut tours = Vec::new();
        for round in 0..rounds_left {
            let mut tour = ScheduleTour::new(round + 1, teams.len() / 2);
            for pair in teams.chunks(2) {
                tour.items.push(ScheduleItem::new(
                    1,
                    "league".into(),
                    pair[0],
                    pair[1],
                    (md_date() + chrono::Duration::days(i64::from(round) * 7))
                        .and_hms_opt(15, 0, 0)
                        .unwrap(),
                    None,
                ));
            }
            tours.push(tour);
        }
        league.schedule = Schedule { tours };
        league.table = table;
        league
    }

    fn md_build(clubs: &[Club], fixture: &LeagueMatch, league: &League, cup: bool) -> Match {
        let ctx = GlobalContext::new(SimulationContext::new(
            md_date().and_hms_opt(0, 0, 0).unwrap(),
        ));
        League::build_match(
            fixture,
            clubs,
            &MatchdayLookup::build(clubs),
            &ctx,
            &league.table,
            &league.table_stakes(),
            false,
            cup,
            cup,
            league.regulations.bench_size,
            &MatchdayCommitments::default(),
        )
    }

    #[test]
    fn a_player_assigned_the_day_before_is_named_by_that_side_only() {
        // First team and "2" side share a Saturday. The club put one of the
        // first team's men with the "2" side on Friday.
        const RELEASED: u32 = 117;
        let mut main = md_first_team(1, 100, 5000);
        let released = main
            .players
            .players
            .iter_mut()
            .find(|p| p.id == RELEASED)
            .expect("the first team's last man");
        released.on_matchday_assigned(md_date(), 2);
        let second = md_team(2, 100, TeamType::Second, md_full_squad(200, 90));
        let clubs = vec![
            md_club(100, vec![main, second]),
            md_club(300, vec![md_first_team(3, 300, 5000)]),
            md_club(400, vec![md_first_team(4, 400, 5000)]),
        ];
        let commitments = md_commitments(&[1, 2]);

        let pool = |team_id: u32, main_team: bool| {
            super::MatchdayPool::offer(
                &clubs[0],
                team_id,
                false,
                main_team,
                md_date(),
                &commitments,
            )
        };
        assert!(
            reserve_has(&pool(2, false), RELEASED),
            "offered to the side he was put with"
        );
        assert!(!reserve_has(&pool(1, true), RELEASED));

        let league = md_league(LeagueTable::default(), 10);
        let build = |home: u32, away: u32| {
            let ctx = GlobalContext::new(SimulationContext::new(
                md_date().and_hms_opt(0, 0, 0).unwrap(),
            ));
            League::build_match(
                &md_fixture(home, away, None),
                &clubs,
                &MatchdayLookup::build(&clubs),
                &ctx,
                &league.table,
                &league.table_stakes(),
                false,
                false,
                false,
                league.regulations.bench_size,
                &commitments,
            )
        };
        let named = |built: &Match| {
            built
                .home_squad
                .main_squad
                .iter()
                .chain(built.home_squad.substitutes.iter())
                .any(|p| p.id == RELEASED)
        };
        assert!(!named(&build(1, 3)), "his own side does not name him");
        assert!(named(&build(2, 4)), "the side he was released to does");
    }

    #[test]
    fn a_built_match_carries_what_each_selection_was_told() {
        let giant = md_first_team(1, 100, 9000);
        let minnow = md_first_team(2, 200, 2000);
        let (giant_rep, minnow_rep) = (
            giant.reputation.market_value_score(),
            minnow.reputation.market_value_score(),
        );
        let clubs = vec![md_club(100, vec![giant]), md_club(200, vec![minnow])];
        let built = md_build(
            &clubs,
            &md_fixture(1, 2, Some((1, 5))),
            &md_league(LeagueTable::default(), 0),
            true,
        );
        let worth = |own: u16, other: u16| {
            KnockoutStakes::of(KnockoutTieReading {
                round: 1,
                total_rounds: 5,
                opponent_ratio: f32::from(other) / f32::from(own),
                aggregate_lead: None,
            })
        };

        let home = FieldSquad::from_team(&built.home_squad);
        let away = FieldSquad::from_team(&built.away_squad);
        assert_eq!(home.selection.importance, worth(giant_rep, minnow_rep));
        assert_eq!(away.selection.importance, worth(minnow_rep, giant_rep));
        assert!(
            away.selection.importance > home.selection.importance + 0.4,
            "the minnow has far more riding on the tie than the giant"
        );
        assert_eq!(home.selection.room, 1.0 - home.selection.importance);
        assert_eq!(
            home.selection.days_to_next, None,
            "no other fixture in sight"
        );
    }

    /// A twenty-team table after 34 of 38 rounds: the leaders far clear, a
    /// mid-table side that can neither reach the top seven nor be caught by
    /// the bottom three, and a side one point above the drop.
    fn md_late_season_table(mid_table: u32, in_danger: u32) -> LeagueTable {
        let mut rows: Vec<LeagueTableRow> = (0..20u32)
            .map(|place| {
                let points = match place {
                    0..=6 => 80 - place as u8 * 2,
                    7..=13 => 50 - (place as u8 - 7),
                    14..=16 => 33 - (place as u8 - 14),
                    _ => 30 - (place as u8 - 17),
                };
                md_row(1000 + place, 34, points)
            })
            .collect();
        rows[9].team_id = mid_table;
        rows[16].team_id = in_danger;
        LeagueTable {
            rows,
            ..LeagueTable::default()
        }
    }

    #[test]
    fn visitors_fighting_relegation_are_told_their_own_stakes() {
        let settled = md_first_team(1, 100, 5000);
        let threatened = md_first_team(2, 200, 5000);
        let clubs = vec![md_club(100, vec![settled]), md_club(200, vec![threatened])];
        let league = md_league(md_late_season_table(1, 2), 4);
        let built = md_build(&clubs, &md_fixture(1, 2, None), &league, false);

        let home = built.home_squad.selection.importance;
        let away = built.away_squad.selection.importance;
        assert!(
            away >= 0.85,
            "one point above the drop with four to play is near a must-win: {away}"
        );
        assert_eq!(
            home,
            TableStakes::PROFESSIONAL_FLOOR,
            "the home side has nothing left to play for"
        );
    }

    #[test]
    fn a_loanee_is_not_named_against_his_parent_club() {
        use crate::PlayerClubContract;

        let mut borrower = md_first_team(1, 100, 5000);
        let mut loanee = md_player(190, PlayerPositionType::Striker, 180);
        loanee.contract_loan = Some(PlayerClubContract::new_loan(
            50_000,
            md_date() + chrono::Duration::days(200),
            200,
            2,
            100,
        ));
        borrower.players.add(loanee);
        let owner = md_first_team(2, 200, 5000);
        let clubs = vec![md_club(100, vec![borrower]), md_club(200, vec![owner])];
        let built = md_build(
            &clubs,
            &md_fixture(1, 2, None),
            &md_league(md_late_season_table(1, 2), 4),
            false,
        );

        let sheet = FieldSquad::from_team(&built.home_squad);
        assert!(
            !sheet.dressing_room().any(|id| id == 190),
            "the best player in the squad is on loan from today's opponent and was named"
        );
    }

    #[test]
    fn a_continental_tie_hears_the_declared_keeper_order() {
        use crate::continent::competitions::ChampionsLeague;

        // The department sees the better goalkeeper; hidden ability says
        // the other man.
        let mut home = md_first_team(1, 100, 8000);
        let mut declared = md_player(180, PlayerPositionType::Goalkeeper, 150);
        declared.player_attributes.current_ability = 128;
        declared.birth_date = md_date() - chrono::Duration::days(365 * 28);
        let mut hidden_better = md_player(181, PlayerPositionType::Goalkeeper, 130);
        hidden_better.player_attributes.current_ability = 134;
        hidden_better.birth_date = md_date() - chrono::Duration::days(365 * 28);
        let outfield: Vec<Player> = home
            .players
            .players
            .drain(..)
            .filter(|p| !p.positions.is_goalkeeper())
            .collect();
        home.players = PlayerCollection::new(
            outfield
                .into_iter()
                .chain([declared, hidden_better])
                .collect(),
        );
        home.staffs = StaffCollection::new(vec![md_goalkeeping_coach(900)]);
        let mut home_club = md_club(100, vec![home]);
        home_club.review_goalkeeping_department(md_date());
        assert_eq!(
            home_club.keeper_plan().and_then(|p| p.number_one()),
            Some(180)
        );

        let clubs = vec![home_club, md_club(200, vec![md_first_team(2, 200, 8000)])];
        let keeper = |built: &Match| {
            built
                .home_squad
                .main_squad
                .iter()
                .find(|p| p.tactical_position.current_position == PlayerPositionType::Goalkeeper)
                .map(|p| p.id)
        };

        let league = md_build(
            &clubs,
            &md_fixture(1, 2, None),
            &md_league(md_late_season_table(1, 2), 4),
            false,
        );
        assert_eq!(
            keeper(&league),
            Some(180),
            "the league fixture plays the declared one"
        );

        let mut competition = ChampionsLeague::new();
        competition.matches.push(ContinentalMatch {
            home_team: 100,
            away_team: 200,
            date: md_date(),
            stage: CompetitionStage::GroupStage,
            match_id: String::new(),
            result: None,
        });
        let by_id: FxHashMap<u32, &Club> = clubs.iter().map(|c| (c.id, c)).collect();
        let matchday =
            competition.prepare_matchday(&by_id, md_date(), &MatchdayCommitments::default());
        assert_eq!(
            keeper(&matchday.matches[0]),
            Some(180),
            "the continental tie was not given the declared order"
        );
    }

    // ========== What a continental tie is worth, and where a club sees it ==========

    use crate::continent::competitions::{
        CompetitionStage, CompetitionTier, ContinentalCompetitions, ContinentalFixture,
        ContinentalMatch, GroupTable, KnockoutTie,
    };

    fn md_continental_tie(home: u32, away: u32, stage: CompetitionStage) -> ContinentalMatch {
        ContinentalMatch {
            home_team: home,
            away_team: away,
            date: md_date() + chrono::Duration::days(3),
            stage,
            match_id: String::new(),
            result: None,
        }
    }

    /// Clubs 100, 200, 300 and 400 after five of six group matches.
    fn md_group(points: [u8; 4]) -> GroupTable {
        let mut group = GroupTable::new(&[100, 200, 300, 400]);
        for (row, points) in group.rows.iter_mut().zip(points) {
            row.played = 5;
            row.points = points;
        }
        group
    }

    fn md_continental_stakes(
        tie: &ContinentalMatch,
        groups: &[GroupTable],
        knockout: &[KnockoutTie],
    ) -> (f32, f32) {
        let home = md_first_team(1, tie.home_team, 8000);
        let away = md_first_team(2, tie.away_team, 8000);
        ContinentalFixture {
            tier: CompetitionTier::ChampionsLeague,
            tie,
            groups,
            knockout,
        }
        .stakes(&home, &away)
    }

    #[test]
    fn a_group_already_won_is_worth_little_to_the_winner_and_a_lot_to_a_chaser() {
        // First is settled; third is a point behind second with one to play.
        let groups = [md_group([15, 7, 6, 1])];
        let tie = md_continental_tie(100, 300, CompetitionStage::GroupStage);
        let (winner, chaser) = md_continental_stakes(&tie, &groups, &[]);
        assert_eq!(winner, TableStakes::PROFESSIONAL_FLOOR);
        assert!(
            chaser >= 0.8,
            "a point off qualifying with one to play: {chaser}"
        );
    }

    #[test]
    fn a_second_leg_reads_the_aggregate_for_both_sides() {
        let tie = md_continental_tie(200, 100, CompetitionStage::RoundOf16);
        let mut level = KnockoutTie::new(100, 200);
        level.record_leg1(1, 1);
        assert_eq!(md_continental_stakes(&tie, &[], &[level]), (1.0, 1.0));

        let mut settled = KnockoutTie::new(100, 200);
        settled.record_leg1(4, 0);
        let (trailing, leading) = md_continental_stakes(&tie, &[], &[settled]);
        assert!(leading < 0.5, "four goals ahead on aggregate: {leading}");
        assert_eq!(trailing, leading);

        let first_leg = md_continental_tie(100, 200, CompetitionStage::RoundOf16);
        let (home, away) = md_continental_stakes(&first_leg, &[], &[KnockoutTie::new(100, 200)]);
        assert!(
            home > 0.6 && home < 0.9 && home == away,
            "an even first leg: {home}"
        );
    }

    #[test]
    fn a_club_in_a_continental_group_has_the_tie_in_its_window() {
        let clubs = vec![
            md_club(100, vec![md_first_team(1, 100, 8000)]),
            md_club(200, vec![md_first_team(2, 200, 8000)]),
        ];
        let by_id: FxHashMap<u32, &Club> = clubs.iter().map(|c| (c.id, c)).collect();
        let mut competitions = ContinentalCompetitions::new();
        competitions.champions_league.groups = vec![md_group([7, 7, 7, 3])];
        let mut played = md_continental_tie(200, 100, CompetitionStage::GroupStage);
        played.date = md_date() - chrono::Duration::days(14);
        played.result = Some((1, 1));
        competitions.champions_league.matches = vec![
            played,
            md_continental_tie(100, 200, CompetitionStage::GroupStage),
        ];

        let runs = competitions.fixture_runs(md_date(), &by_id);
        let run = &runs[&100];
        assert_eq!(run.recent, vec![md_date() - chrono::Duration::days(14)]);
        assert_eq!(run.upcoming.len(), 1);
        assert_eq!(run.upcoming[0].date, md_date() + chrono::Duration::days(3));
        assert!(
            run.upcoming[0].importance > 0.8,
            "three sides level for two places with one to play"
        );

        let mut team = md_first_team(1, 100, 8000);
        team.fixture_window.continental = run.clone();
        let week = team.fixture_window.week(md_date());
        assert_eq!(week.to_next, Some(3));
        assert_eq!(week.next_importance, run.upcoming[0].importance);
    }
}
