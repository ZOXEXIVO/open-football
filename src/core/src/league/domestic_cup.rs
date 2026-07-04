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
use crate::MatchRuntime;
use crate::TeamType;
use crate::context::GlobalContext;
use crate::league::schedule::cup;
use crate::league::{
    League, LeagueBuildOutput, LeagueMatch, LeaguePendingState, LeagueResult, LeagueTableResult,
    MatchStorage, Schedule, ScheduleItem, ScheduleTour,
};
use crate::r#match::MatchResult;
use chrono::{Datelike, Duration, NaiveDate, NaiveDateTime, NaiveTime};
use log::debug;
use std::collections::HashSet;

/// One completed edition's result, captured at the instant the next
/// season's bracket is drawn (while the just-finished bracket is still
/// intact). Powers the cup's History tab — a roll of honour of past
/// champions. We store team ids, not names, so a club that is later
/// renamed still resolves correctly when the page is rendered.
#[derive(Debug, Clone)]
pub struct CupHistoryEntry {
    /// Calendar year the winning edition's round one was drawn in —
    /// the same anchor as [`DomesticCup::season_start_year`].
    pub season_start_year: i32,
    pub champion_team_id: u32,
    /// The beaten finalist. `None` when the final's pairing couldn't be
    /// resolved (e.g. the edition was decided without a one-tie final).
    pub runner_up_team_id: Option<u32>,
}

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
    /// Season-start year of the last edition whose winner-trophy event was
    /// emitted. Combined with `award_emitted_winner_team_id`, prevents the
    /// post-final daily tick from awarding silverware on every subsequent
    /// simulation day.
    pub award_emitted_season_start_year: Option<i32>,
    /// Team id that received the winner-trophy emit for the recorded
    /// season. Paired with `award_emitted_season_start_year`: a new
    /// edition (different season year) re-arms the emit, and a different
    /// champion within the same year would also re-arm — though that
    /// shouldn't happen in practice.
    pub award_emitted_winner_team_id: Option<u32>,
    /// Calendar date the winner award fan-out actually ran. Captured for
    /// debugging and for the unit tests that probe duplicate prevention.
    pub award_emitted_on: Option<NaiveDate>,
    /// Completed editions, oldest first. Appended at each season's
    /// regeneration when the outgoing bracket produced a champion. Empty
    /// for a fresh world until the first edition is decided; powers the
    /// cup History tab.
    pub past_champions: Vec<CupHistoryEntry>,
}

impl DomesticCup {
    pub fn new(league: League) -> Self {
        DomesticCup {
            league,
            season_start_year: 0,
            award_emitted_season_start_year: None,
            award_emitted_winner_team_id: None,
            award_emitted_on: None,
            past_champions: Vec::new(),
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

    /// Append the outgoing edition to `past_champions` if its bracket
    /// resolved to a champion. The runner-up is the loser of the final
    /// pairing. Called from `regenerate_bracket` while the finished
    /// bracket (and its `season_start_year`) still describes that edition.
    fn record_history(&mut self, clubs: &[Club]) {
        let Some(champion) = self.champion(clubs) else {
            return;
        };
        let runner_up = self
            .champion_final_pairing(clubs)
            .map(|(home, away)| if home == champion { away } else { home });
        self.past_champions.push(CupHistoryEntry {
            season_start_year: self.season_start_year,
            champion_team_id: champion,
            runner_up_team_id: runner_up,
        });
    }

    /// Wipe the old bracket and draw round one from the current senior
    /// field. Called at season start (and on the very first tick). With
    /// fewer than two participants the cup exists but stages no fixtures.
    fn regenerate_bracket(&mut self, clubs: &[Club], ctx: &GlobalContext<'_>) {
        // Archive the outgoing edition's winner before the bracket that
        // proves it is discarded. No-op on the very first draw — there is
        // no prior bracket, so `champion` resolves to `None`.
        self.record_history(clubs);
        self.league.schedule = Schedule::new();
        self.league.matches = MatchStorage::new();
        self.season_start_year = ctx.simulation.date.year();
        // A fresh edition re-arms the winner emit. Without this, a club
        // that won last season and entered the new draw on the same team id
        // would silently keep the previous edition's marker and never get
        // its new trophy event.
        self.award_emitted_season_start_year = None;
        self.award_emitted_winner_team_id = None;
        self.award_emitted_on = None;

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

    /// Build (but do not play) today's cup matches. Mirrors
    /// [`League::simulate_build`] for the knockout side: regenerates the
    /// bracket at season start, collects today's ties, and returns the
    /// `Match` objects ready for a batched engine dispatch alongside a
    /// `LeaguePendingState` so [`simulate_process`] can resume cup-side
    /// bookkeeping once the engine returns the results.
    pub fn simulate_build(&mut self, clubs: &[Club], ctx: &GlobalContext<'_>) -> LeagueBuildOutput {
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
        let scheduled = self.collect_today_matches(current_date, total_rounds);
        if scheduled.is_empty() {
            return LeagueBuildOutput {
                matches: Vec::new(),
                pending: None,
                immediate: Some(LeagueResult::new(self.league.id, LeagueTableResult {})),
            };
        }

        // Knockout: build via the inner league's matchday builder with
        // `knockout = true`, so a level score is settled by extra time
        // and (if needed) penalties.
        let matches = self
            .league
            .build_matchday_matches(&scheduled, clubs, ctx, false, true);

        LeagueBuildOutput {
            matches,
            pending: Some(LeaguePendingState {
                scheduled_matches: scheduled,
                table_result: LeagueTableResult {},
                new_season_started: false,
            }),
            immediate: None,
        }
    }

    /// Apply played cup match results back onto the bracket: stamps each
    /// score onto its schedule item, stores the result on the inner
    /// league for the cup page and top-scorer tables, and draws the
    /// next round if today closed one out. Per-player stat / morale /
    /// discipline fan-out still happens via the caller's
    /// `LeagueResult::process_local`.
    pub fn simulate_process(
        &mut self,
        match_results: Vec<MatchResult>,
        pending: LeaguePendingState,
        clubs: &[Club],
        _ctx: &GlobalContext<'_>,
        current_date: NaiveDate,
    ) -> LeagueResult {
        let LeaguePendingState {
            mut scheduled_matches,
            ..
        } = pending;

        self.league
            .apply_matchday_results(&mut scheduled_matches, &match_results);

        // Cup-side bookkeeping only — no standings table.
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

    /// Backwards-compatible wrapper that runs build → engine → process
    /// in one call. Production paths go through
    /// `Country::simulate_build` so cup matches join the world's
    /// single global dispatch batch via `WorldMatchdayResult::process`.
    pub fn simulate(&mut self, clubs: &[Club], ctx: &GlobalContext<'_>) -> LeagueResult {
        let current_date = ctx.simulation.date.date();
        let output = self.simulate_build(clubs, ctx);
        if let Some(immediate) = output.immediate {
            return immediate;
        }
        let match_results = MatchRuntime::engine_pool().play(output.matches);
        let pending = output
            .pending
            .expect("cup simulate_build with matches must produce a pending state");
        self.simulate_process(match_results, pending, clubs, ctx, current_date)
    }

    /// The current edition's champion, if the bracket has resolved to a
    /// single surviving team. Rebuilds the seeded field from `clubs` so the
    /// bye-aware `advancing_teams` logic can reconcile any round-one
    /// participant who never played (top seeds with a bye in round one,
    /// then a bye in later rounds too, are still alive).
    pub fn champion(&self, clubs: &[Club]) -> Option<u32> {
        let field = Self::seeded_participants(clubs);
        cup::cup_champion(&self.league.schedule.tours, &field)
    }

    /// Champion team id only if a fresh winner-trophy fan-out is owed for
    /// this edition. Returns `None` once the marker for this season has
    /// been set, so the caller (which runs every simulation tick) can fire
    /// exactly once per edition without bookkeeping at the call site.
    pub fn should_emit_winner_award(&self, clubs: &[Club]) -> Option<u32> {
        let team_id = self.champion(clubs)?;
        if self.award_emitted_season_start_year == Some(self.season_start_year)
            && self.award_emitted_winner_team_id == Some(team_id)
        {
            return None;
        }
        Some(team_id)
    }

    /// Record that the winner fan-out has run for `team_id` on `date`.
    /// Must be called from the caller after at least one eligible player
    /// has been processed — see `Country::process_domestic_cup_winner_awards`.
    pub fn mark_winner_award_emitted(&mut self, team_id: u32, date: NaiveDate) {
        self.award_emitted_season_start_year = Some(self.season_start_year);
        self.award_emitted_winner_team_id = Some(team_id);
        self.award_emitted_on = Some(date);
    }

    /// The final's pairing `(home_team_id, away_team_id)` when the
    /// edition has resolved to a champion. The final is the last tour
    /// that contains exactly one tie — knockout rounds collapse the
    /// field by half, so a non-empty last tour with one match is the
    /// final by construction.
    ///
    /// Returns `None` when the bracket isn't resolved (no champion yet),
    /// when the last tour has zero played fixtures, or when the schedule
    /// shape is unexpected (multi-tie "final" — defensive guard).
    pub fn champion_final_pairing(&self, clubs: &[Club]) -> Option<(u32, u32)> {
        self.champion(clubs)?;
        let last = self.league.schedule.tours.last()?;
        if last.items.len() != 1 {
            return None;
        }
        let item = &last.items[0];
        if item.result.is_none() {
            return None;
        }
        Some((item.home_team_id, item.away_team_id))
    }
}

#[cfg(test)]
mod tests {
    use super::DomesticCup;
    use crate::academy::ClubAcademy;
    use crate::context::{GlobalContext, SimulationContext};
    use crate::league::schedule::cup;
    use crate::league::{
        DayMonthPeriod, League, LeagueSettings, Schedule, ScheduleItem, ScheduleTour,
    };
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

    #[test]
    fn record_history_appends_champion_and_runner_up() {
        use crate::r#match::Score;

        // Two clubs, one Main team each. A resolved single-tie final is
        // the simplest decided bracket: the home side wins, so it is the
        // champion and the away side the runner-up.
        let clubs: Vec<Club> = vec![
            club(1, vec![team(10, 1, TeamType::Main, 6000)]),
            club(2, vec![team(20, 2, TeamType::Main, 5000)]),
        ];

        let mut cup = make_cup();
        cup.season_start_year = 2026;

        let date = NaiveDate::from_ymd_opt(2027, 5, 1)
            .unwrap()
            .and_hms_opt(0, 0, 0)
            .unwrap();
        let mut tour = ScheduleTour::new(1, 1);
        let item = ScheduleItem::new(cup.league.id, cup.league.slug.clone(), 10, 20, date, None);
        let item_id = item.id.clone();
        tour.items.push(item);
        cup.league.schedule.tours.push(tour);

        // Team 10 beats team 20 2–1.
        let score = Score::new(10, 20);
        score.increment_home_goals();
        score.increment_home_goals();
        score.increment_away_goals();
        cup.league.schedule.update_match_result(&item_id, &score);

        cup.record_history(&clubs);

        assert_eq!(cup.past_champions.len(), 1, "the decided final is recorded");
        let entry = &cup.past_champions[0];
        assert_eq!(entry.champion_team_id, 10, "home winner is the champion");
        assert_eq!(entry.runner_up_team_id, Some(20), "away side is runner-up");
        assert_eq!(
            entry.season_start_year, 2026,
            "edition keyed to its anchored start year"
        );

        // An unresolved (here: emptied) bracket has no champion, so a
        // second pass records nothing — guards against double-counting a
        // season that never reached a final.
        cup.league.schedule = Schedule::new();
        cup.record_history(&clubs);
        assert_eq!(
            cup.past_champions.len(),
            1,
            "no champion resolved → no new history entry"
        );
    }
}
