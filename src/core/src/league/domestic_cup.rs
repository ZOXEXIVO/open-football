//! Domestic knockout cup (FA Cup, Copa del Rey, Coppa Italia, …).
//!
//! A `DomesticCup` is the core crate's own representation of a country's
//! club cup. It lives on `Country::domestic_cup`, *outside* the standings
//! `LeagueCollection`, so the league programme stays purely round-robin.
//!
//! The cup drives its fixtures through an inner `League` (`is_cup = true`,
//! `friendly = false`) so it transparently reuses the match engine, the
//! per-match stat / morale / discipline / reputation fan-out
//! (`LeagueResult::process_local`), slug indexing and the web layer — all
//! of which key off `League`. What this wrapper adds on top is the
//! knockout-specific behaviour the round-robin scheduler can't express:
//! a progressively drawn single-leg bracket with byes for the top seeds.

use crate::Club;
use crate::TeamType;
use crate::context::GlobalContext;
use crate::league::schedule::cup;
use crate::league::{
    League, LeagueMatch, LeagueResult, LeagueTableResult, MatchStorage, Schedule, ScheduleItem,
    ScheduleTour,
};
use chrono::{Datelike, Duration, NaiveDate, NaiveDateTime, NaiveTime};
use log::debug;
use std::collections::HashSet;

#[derive(Debug, Clone)]
pub struct DomesticCup {
    /// The cup is run through a `League` flagged `is_cup = true`. Reusing
    /// `League` means the cup inherits match execution, result processing,
    /// stat routing and slug/web wiring for free.
    pub league: League,
    /// Calendar year the current edition's round one was drawn in. Anchors
    /// season-window maths so later rounds (which may be generated after a
    /// New Year roll-over) still place against the correct start year.
    pub season_start_year: i32,
}

impl DomesticCup {
    pub fn new(league: League) -> Self {
        DomesticCup {
            league,
            season_start_year: 0,
        }
    }

    pub fn id(&self) -> u32 {
        self.league.id
    }

    pub fn slug(&self) -> &str {
        &self.league.slug
    }

    /// Senior first teams of the country's clubs, in seed order (highest
    /// team reputation first; team id breaks ties for determinism). Only
    /// `TeamType::Main` squads enter — B / Second / Reserve / youth sides
    /// are excluded even when they play in a real division.
    fn seeded_participants(clubs: &[Club]) -> Vec<u32> {
        let mut teams: Vec<(u32, u16)> = Vec::new();
        for club in clubs {
            for team in &club.teams.teams {
                if team.team_type == TeamType::Main && team.league_id.is_some() {
                    teams.push((team.id, team.reputation.market_value_score()));
                }
            }
        }
        teams.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
        teams.into_iter().map(|(id, _)| id).collect()
    }

    /// Order the surviving teams by their original round-one seed so the
    /// next round's byes again fall to the strongest sides.
    fn reseed(seeded_field: &[u32], alive: &[u32]) -> Vec<u32> {
        let alive_set: HashSet<u32> = alive.iter().copied().collect();
        seeded_field
            .iter()
            .copied()
            .filter(|t| alive_set.contains(t))
            .collect()
    }

    /// Season `[start, end]` for the current edition, derived from the cup's
    /// (tier-1-mirroring) settings and the anchored start year. Handles
    /// autumn-spring seasons that wrap into the next calendar year.
    fn season_window(&self) -> (NaiveDate, NaiveDate) {
        let s = &self.league.settings;
        let year = self.season_start_year;
        let start = NaiveDate::from_ymd_opt(
            year,
            s.season_starting_half.from_month as u32,
            s.season_starting_half.from_day as u32,
        )
        .unwrap_or_else(|| NaiveDate::from_ymd_opt(year, 8, 1).unwrap());
        // An end month at or before the start month means the campaign runs
        // into the following calendar year (e.g. Aug → May).
        let end_year = if s.season_ending_half.to_month <= s.season_starting_half.from_month {
            year + 1
        } else {
            year
        };
        let end = NaiveDate::from_ymd_opt(
            end_year,
            s.season_ending_half.to_month as u32,
            s.season_ending_half.to_day as u32,
        )
        .unwrap_or_else(|| NaiveDate::from_ymd_opt(end_year, 5, 31).unwrap());
        (start, end)
    }

    fn build_tour(&self, round: u8, pairings: &[(u32, u32)], date: NaiveDateTime) -> ScheduleTour {
        let mut tour = ScheduleTour::new(round, pairings.len());
        for (home, away) in pairings {
            tour.items.push(ScheduleItem::new(
                self.league.id,
                self.league.slug.clone(),
                *home,
                *away,
                date,
                None,
            ));
        }
        tour
    }

    /// Today's unplayed cup ties, tagged with their bracket position. The
    /// tour number is the 1-based round; `total_rounds` is the size of the
    /// whole bracket (passed in precomputed from the seeded field) so the
    /// match builder can tell an early round from the final.
    fn collect_today_matches(&self, current_date: NaiveDate, total_rounds: u8) -> Vec<LeagueMatch> {
        self.league
            .schedule
            .tours
            .iter()
            .flat_map(|t| {
                let round = t.num;
                t.items.iter().map(move |i| (round, i))
            })
            .filter(|(_, i)| i.date.date() == current_date && i.result.is_none())
            .map(|(round, i)| LeagueMatch {
                id: i.id.clone(),
                league_id: i.league_id,
                league_slug: i.league_slug.clone(),
                date: i.date,
                home_team_id: i.home_team_id,
                away_team_id: i.away_team_id,
                result: None,
                cup_round: Some(round),
                cup_total_rounds: Some(total_rounds),
            })
            .collect()
    }

    /// Wipe the old bracket and draw round one from the current senior
    /// field. Called at season start (and on the very first tick). With
    /// fewer than two participants the cup exists but stages no fixtures.
    fn regenerate_bracket(&mut self, clubs: &[Club], ctx: &GlobalContext<'_>) {
        self.league.schedule = Schedule::new();
        self.league.matches = MatchStorage::new();
        self.season_start_year = ctx.simulation.date.year();

        let field = Self::seeded_participants(clubs);
        if field.len() < 2 {
            debug!(
                "🏆 Cup {} has fewer than two participants — no draw this season",
                self.league.name
            );
            return;
        }

        let (pairings, _byes) = cup::pair_knockout_round(&field);
        let total = cup::total_rounds(field.len());
        let (season_start, season_end) = self.season_window();
        let mut date = cup::cup_round_date(season_start, season_end, 1, total);
        // If the game begins mid-season (e.g. a calendar-year league whose
        // campaign already started before the world's August kick-off), the
        // evenly-spaced round-one slot can land in the past and never play.
        // Push it to the next midweek so the cup still runs this season.
        let today = ctx.simulation.date.date();
        if date <= today {
            date = cup::next_midweek(today + Duration::days(7));
        }
        let dt = NaiveDateTime::new(date, NaiveTime::from_hms_opt(0, 0, 0).unwrap());
        let tour = self.build_tour(1, &pairings, dt);
        self.league.schedule.tours.push(tour);
        debug!(
            "🏆 Cup {} round 1 drawn: {} ties, {} entrants, first leg {}",
            self.league.name,
            self.league.schedule.tours[0].items.len(),
            field.len(),
            date
        );
    }

    /// Once every existing round is fully played and more than one team is
    /// left, draw the next round from the survivors. A single survivor is
    /// the champion — the season's bracket is complete.
    fn maybe_generate_next_round(&mut self, clubs: &[Club], current_date: NaiveDate) {
        let field = Self::seeded_participants(clubs);
        if field.len() < 2 {
            return;
        }

        let alive = match cup::advancing_teams(&self.league.schedule.tours, &field) {
            Some(a) => a,
            None => return, // a tie in the latest round is still pending
        };
        if alive.len() < 2 {
            return; // champion decided (or empty field) — done for the season
        }

        let alive_seeded = Self::reseed(&field, &alive);
        let (pairings, _byes) = cup::pair_knockout_round(&alive_seeded);
        if pairings.is_empty() {
            return;
        }

        let next_round = (self.league.schedule.tours.len() + 1) as u8;
        let total = cup::total_rounds(field.len());
        let (season_start, season_end) = self.season_window();
        let mut date = cup::cup_round_date(season_start, season_end, next_round, total);
        // Never schedule into the past: the previous round may have run long
        // enough that the evenly-spaced slot has already elapsed.
        if date <= current_date {
            date = cup::next_midweek(current_date + Duration::days(7));
        }
        let dt = NaiveDateTime::new(date, NaiveTime::from_hms_opt(0, 0, 0).unwrap());
        let tour = self.build_tour(next_round, &pairings, dt);
        debug!(
            "🏆 Cup {} round {} drawn: {} ties on {}",
            self.league.name,
            next_round,
            tour.items.len(),
            date
        );
        self.league.schedule.tours.push(tour);
    }

    /// Advance the cup one simulation day. Mirrors `League::simulate`'s
    /// shape (regenerate at season start, play today's fixtures, return a
    /// `LeagueResult`) but on a knockout bracket instead of a table.
    ///
    /// The returned `LeagueResult` carries the day's match results so the
    /// caller's `process_local` pass runs the same per-player stat / morale
    /// / discipline / reputation fan-out as for league games — and, because
    /// the inner league is `is_cup = true`, those land in the cup buckets.
    pub fn simulate(&mut self, clubs: &[Club], ctx: &GlobalContext<'_>) -> LeagueResult {
        let current_date = ctx.simulation.date.date();

        let new_schedule = self.league.schedule.tours.is_empty()
            || self
                .league
                .settings
                .is_time_for_new_schedule(&ctx.simulation);
        if new_schedule {
            self.regenerate_bracket(clubs, ctx);
        }

        // Bracket size resolves which round is the final, so the match
        // builder can scale importance by stage. Computed from the seeded
        // field that round one was drawn from.
        let total_rounds = cup::total_rounds(Self::seeded_participants(clubs).len());
        let mut scheduled = self.collect_today_matches(current_date, total_rounds);
        if scheduled.is_empty() {
            return LeagueResult::new(self.league.id, LeagueTableResult {});
        }

        // Knockout: build via `Match::make_knockout` so a level score is
        // settled by extra time / penalties.
        let match_results =
            self.league
                .play_scheduled_matches(&mut scheduled, clubs, ctx, false, true);

        // Cup-side bookkeeping only — no standings table. Store results for
        // the cup page / top scorers, and write them into the bracket so we
        // can work out who advances. (The per-player stat fan-out happens in
        // the caller's `process_local`.)
        for mr in &match_results {
            self.league
                .matches
                .push(mr.copy_without_data_positions(), current_date);
            self.league.schedule.update_match_result(&mr.id, &mr.score);
        }

        // If the round just completed, draw the next one immediately so its
        // fixtures are on the calendar for upcoming ticks.
        self.maybe_generate_next_round(clubs, current_date);

        LeagueResult::with_match_result(self.league.id, LeagueTableResult {}, match_results)
    }
}

#[cfg(test)]
mod tests {
    use super::DomesticCup;
    use crate::academy::ClubAcademy;
    use crate::context::{GlobalContext, SimulationContext};
    use crate::league::schedule::cup;
    use crate::league::{DayMonthPeriod, League, LeagueSettings};
    use crate::shared::Location;
    use crate::{
        Club, ClubColors, ClubFacilities, ClubFinances, ClubStatus, PlayerCollection,
        StaffCollection, Team, TeamBuilder, TeamCollection, TeamReputation, TeamType,
        TrainingSchedule,
    };
    use chrono::{Datelike, NaiveDate, NaiveTime};

    fn team(id: u32, club_id: u32, team_type: TeamType, rep: u16) -> Team {
        TeamBuilder::new()
            .id(id)
            .league_id(Some(1))
            .club_id(club_id)
            .name(format!("T{id}"))
            .slug(format!("t{id}"))
            .team_type(team_type)
            .players(PlayerCollection::new(Vec::new()))
            .staffs(StaffCollection::new(Vec::new()))
            .reputation(TeamReputation::new(rep, rep, rep))
            .training_schedule(TrainingSchedule::new(
                NaiveTime::from_hms_opt(9, 0, 0).unwrap(),
                NaiveTime::from_hms_opt(15, 0, 0).unwrap(),
            ))
            .build()
            .unwrap()
    }

    fn club(id: u32, teams: Vec<Team>) -> Club {
        Club::new(
            id,
            format!("C{id}"),
            Location::new(1),
            ClubFinances::new(1_000_000, Vec::new()),
            ClubAcademy::new(3),
            ClubStatus::Professional,
            ClubColors::default(),
            TeamCollection::new(teams),
            ClubFacilities::default(),
        )
    }

    #[test]
    fn only_main_teams_are_cup_participants() {
        // Each club fields a Main plus reserve/youth sides. Only the Main
        // teams should enter the cup, ordered by reputation (best first).
        let c1 = club(
            1,
            vec![
                team(10, 1, TeamType::Main, 5000),
                team(11, 1, TeamType::B, 4000),
                team(12, 1, TeamType::U19, 1000),
            ],
        );
        let c2 = club(
            2,
            vec![
                team(20, 2, TeamType::Main, 3000),
                team(21, 2, TeamType::Second, 2500),
                team(22, 2, TeamType::U23, 900),
            ],
        );

        let field = DomesticCup::seeded_participants(&[c1, c2]);
        assert_eq!(
            field,
            vec![10, 20],
            "only Main teams enter the cup, seeded by reputation"
        );
    }

    fn make_cup() -> DomesticCup {
        let settings = LeagueSettings {
            season_starting_half: DayMonthPeriod::new(1, 8, 30, 12),
            season_ending_half: DayMonthPeriod::new(1, 1, 31, 5),
            tier: 0,
            promotion_spots: 0,
            relegation_spots: 0,
            league_group: None,
        };
        let mut league = League::new(
            800_000_999,
            "Test Cup".into(),
            "test-cup".into(),
            1,
            5000,
            settings,
            false,
        );
        league.is_cup = true;
        DomesticCup::new(league)
    }

    #[test]
    fn season_start_draws_first_round_without_playing_yet() {
        // Six clubs, each a single Main team. Round one should reduce the
        // field of 6 to 4: two ties plus two top-seed byes.
        let clubs: Vec<Club> = (0..6)
            .map(|i| {
                let id = (i + 1) as u32;
                club(
                    id,
                    vec![team(id * 10, id, TeamType::Main, 6000 - (i as u16) * 200)],
                )
            })
            .collect();

        let mut cup = make_cup();
        // Season-start tick (1 August) — `is_time_for_new_schedule` fires.
        let date = NaiveDate::from_ymd_opt(2026, 8, 1)
            .unwrap()
            .and_hms_opt(0, 0, 0)
            .unwrap();
        let base = GlobalContext::new(SimulationContext::new(date));
        let ctx = base.with_league(
            cup.league.id,
            cup.league.slug.clone(),
            &[],
            cup.league.reputation,
        );

        let result = cup.simulate(&clubs, &ctx);

        // Round one is drawn but dated in September, so nothing plays today.
        assert_eq!(cup.league.schedule.tours.len(), 1, "round one drawn");
        assert_eq!(
            cup.league.schedule.tours[0].items.len(),
            2,
            "6 entrants → 2 ties (4 advance) with 2 byes"
        );
        assert!(
            result.match_results.is_none(),
            "first round is future-dated; no fixtures today"
        );
        // The fixtures are midweek (Wednesday) and in the future.
        let tie_date = cup.league.schedule.tours[0].items[0].date.date();
        assert!(tie_date > date.date());
        assert_eq!(tie_date.weekday(), chrono::Weekday::Wed);
    }

    #[test]
    fn collect_today_matches_tags_cup_round_metadata() {
        // Plumbing guard: the LeagueMatch values handed to the match builder
        // must carry the round, total bracket size, and cup league id so
        // `build_match` can pick the DomesticCup competition + stage.
        let clubs: Vec<Club> = (0..6)
            .map(|i| {
                let id = (i + 1) as u32;
                club(
                    id,
                    vec![team(id * 10, id, TeamType::Main, 6000 - (i as u16) * 200)],
                )
            })
            .collect();

        let mut cup = make_cup();
        let date = NaiveDate::from_ymd_opt(2026, 8, 1)
            .unwrap()
            .and_hms_opt(0, 0, 0)
            .unwrap();
        let base = GlobalContext::new(SimulationContext::new(date));
        let ctx = base.with_league(
            cup.league.id,
            cup.league.slug.clone(),
            &[],
            cup.league.reputation,
        );
        cup.simulate(&clubs, &ctx);

        let tie_date = cup.league.schedule.tours[0].items[0].date.date();
        let total = cup::total_rounds(DomesticCup::seeded_participants(&clubs).len());
        let matches = cup.collect_today_matches(tie_date, total);

        assert!(
            !matches.is_empty(),
            "the round-one ties should be collected"
        );
        for m in &matches {
            assert_eq!(m.cup_round, Some(1), "round-one fixtures tagged as round 1");
            assert_eq!(m.cup_total_rounds, Some(total), "bracket size carried");
            assert_eq!(
                m.league_id, cup.league.id,
                "fixtures keyed to the cup league"
            );
        }
    }
}
