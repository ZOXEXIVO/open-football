//! End-of-season playoff for a grouped competition (MLS Cup, Serie C
//! promotion playoff, …).
//!
//! A `LeaguePlayoff` crowns a *single* champion across the several
//! round-robin groups that make up one competition (e.g. MLS Eastern +
//! Western Conference). The regular season runs as N independent group
//! tables — exactly as before — and when every group has finished its
//! programme the top `qualifiers_per_group` of each group are seeded into
//! a knockout bracket. The bracket's sole survivor is the competition
//! champion.
//!
//! Like [`crate::league::DomesticCup`], the playoff drives its fixtures
//! through an inner `League` (`is_cup = true`) so it reuses the match
//! engine, per-match stat/morale/discipline fan-out, slug indexing and the
//! web layer. The knockout maths (seeding, byes, progressive draw, champion
//! resolution) are the shared free functions in
//! [`crate::league::schedule::cup`] — the same ones the domestic cup uses.
//!
//! What differs from the cup:
//!   * **Seed source** — the entrants come from the member groups' final
//!     standings (top-N per group), not the whole country by reputation.
//!   * **Draw trigger** — round one is drawn only once every member group
//!     has played its full schedule, not at season start.
//!   * **Cardinality** — a country can host several grouped competitions,
//!     so playoffs live in a `Vec` on `Country`, not a single slot.

use crate::Club;
use crate::MatchRuntime;
use crate::context::GlobalContext;
use crate::league::schedule::cup;
use crate::league::{
    CupHistoryEntry, League, LeagueBuildOutput, LeagueMatch, LeaguePendingState, LeagueResult,
    LeagueTableResult, MatchStorage, Schedule, ScheduleItem, ScheduleTour,
};
use crate::r#match::MatchResult;
use chrono::{Datelike, Duration, NaiveDate, NaiveDateTime, NaiveTime};
use log::debug;

/// A snapshot of one group's current standings, handed to the playoff by
/// `Country` so the playoff never has to borrow the league collection
/// itself. `ordered_team_ids` is the live table, best-first (the same order
/// `final_table` freezes at season end); `complete` is true once every
/// fixture in the group has a result.
#[derive(Debug, Clone)]
pub struct GroupStanding {
    pub league_id: u32,
    pub complete: bool,
    pub ordered_team_ids: Vec<u32>,
}

#[derive(Debug, Clone)]
pub struct LeaguePlayoff {
    /// The bracket is run through a `League` flagged `is_cup = true`, so it
    /// inherits match execution, result processing, stat routing and
    /// slug/web wiring for free.
    pub league: League,
    /// Parent competition name shared by the member groups (e.g.
    /// "Major League Soccer"). Purely descriptive / for the web layer.
    pub competition: String,
    /// League ids of the groups that feed this playoff, in seed-priority
    /// order (group 0's winner outranks group 1's winner on byes, etc.).
    pub group_league_ids: Vec<u32>,
    /// How many top teams from each group enter the bracket.
    pub qualifiers_per_group: u8,
    /// Calendar year the current edition's round one was (or will be)
    /// drawn in. Anchors the season window for later-round scheduling.
    pub season_start_year: i32,
    /// The seeded entrant field, frozen at the round-one draw. Empty until
    /// the groups finish and the bracket is drawn. All bracket maths
    /// (advancing teams, champion, byes) reconcile against this field, so
    /// it must not change once a bracket is live.
    pub frozen_field: Vec<u32>,
    /// Completed editions, oldest first — powers the History tab.
    pub past_champions: Vec<CupHistoryEntry>,
    /// Season-start year of the last edition whose winner-trophy event was
    /// emitted; paired with `award_emitted_winner_team_id` to fire the
    /// champion fan-out exactly once per edition.
    pub award_emitted_season_start_year: Option<i32>,
    pub award_emitted_winner_team_id: Option<u32>,
    pub award_emitted_on: Option<NaiveDate>,
}

impl LeaguePlayoff {
    pub fn new(
        league: League,
        competition: String,
        group_league_ids: Vec<u32>,
        qualifiers_per_group: u8,
    ) -> Self {
        LeaguePlayoff {
            league,
            competition,
            group_league_ids,
            qualifiers_per_group,
            season_start_year: 0,
            frozen_field: Vec::new(),
            past_champions: Vec::new(),
            award_emitted_season_start_year: None,
            award_emitted_winner_team_id: None,
            award_emitted_on: None,
        }
    }

    pub fn id(&self) -> u32 {
        self.league.id
    }

    pub fn slug(&self) -> &str {
        &self.league.slug
    }

    /// The member group standings for this playoff, in `group_league_ids`
    /// order (so seeding priority is stable and deterministic).
    fn member_standings<'a>(&self, groups: &'a [GroupStanding]) -> Vec<&'a GroupStanding> {
        self.group_league_ids
            .iter()
            .filter_map(|id| groups.iter().find(|g| g.league_id == *id))
            .collect()
    }

    /// Seed the entrant field by interleaving finishing positions across
    /// groups: every group's winner first (in group order), then every
    /// group's runner-up, and so on down to `qualifiers_per_group`. This
    /// balances the merged bracket — the byes a non-power-of-two field
    /// hands out fall to the group winners — and guarantees the strongest
    /// side of each group is protected in the early rounds.
    fn build_seed_field(&self, members: &[&GroupStanding]) -> Vec<u32> {
        let qpg = self.qualifiers_per_group as usize;
        let mut field = Vec::with_capacity(qpg * members.len());
        for rank in 0..qpg {
            for g in members {
                if let Some(&team_id) = g.ordered_team_ids.get(rank) {
                    field.push(team_id);
                }
            }
        }
        field
    }

    /// Season `[start, end]` for the current edition, derived from the
    /// playoff's (group-mirroring) settings and the anchored start year.
    /// Handles autumn-spring seasons that wrap into the next calendar year.
    fn season_window(&self) -> (NaiveDate, NaiveDate) {
        let s = &self.league.settings;
        let year = self.season_start_year;
        let start = NaiveDate::from_ymd_opt(
            year,
            s.season_starting_half.from_month as u32,
            s.season_starting_half.from_day as u32,
        )
        .unwrap_or_else(|| NaiveDate::from_ymd_opt(year, 8, 1).unwrap());
        let mut end = NaiveDate::from_ymd_opt(
            year,
            s.season_ending_half.to_month as u32,
            s.season_ending_half.to_day as u32,
        )
        .unwrap_or_else(|| NaiveDate::from_ymd_opt(year, 5, 31).unwrap());
        // Autumn-spring campaign: the ending half lands in the next year.
        if end <= start {
            end = NaiveDate::from_ymd_opt(
                year + 1,
                s.season_ending_half.to_month as u32,
                s.season_ending_half.to_day as u32,
            )
            .unwrap_or(end);
        }
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

    /// Today's unplayed playoff ties, tagged with their bracket position
    /// so the match builder can scale importance by stage.
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

    /// Archive the outgoing edition's champion (with runner-up) before the
    /// bracket that proves it is discarded. No-op until an edition resolves.
    fn record_history(&mut self) {
        let Some(champion) = self.champion() else {
            return;
        };
        let runner_up = self
            .final_pairing()
            .map(|(home, away)| if home == champion { away } else { home });
        self.past_champions.push(CupHistoryEntry {
            season_start_year: self.season_start_year,
            champion_team_id: champion,
            runner_up_team_id: runner_up,
        });
    }

    /// Start a fresh edition: archive the old champion, wipe the bracket and
    /// seed field, re-anchor the year and re-arm the winner emit. Called on
    /// the competition's season-start day (mirrors the cup's regeneration),
    /// well before the groups finish and round one is actually drawn.
    fn reset_edition(&mut self, ctx: &GlobalContext<'_>) {
        self.record_history();
        self.league.schedule = Schedule::new();
        self.league.matches = MatchStorage::new();
        self.frozen_field = Vec::new();
        self.season_start_year = ctx.simulation.date.year();
        self.award_emitted_season_start_year = None;
        self.award_emitted_winner_team_id = None;
        self.award_emitted_on = None;
    }

    /// Draw round one from the frozen group-seeded field. Requires at least
    /// two entrants; the first tie is placed on the playoff window's first
    /// midweek that has not already elapsed.
    fn draw_round_one(&mut self, members: &[&GroupStanding], current_date: NaiveDate) {
        let field = self.build_seed_field(members);
        if field.len() < 2 {
            debug!(
                "🏆 Playoff {} has fewer than two entrants — no bracket this season",
                self.league.name
            );
            return;
        }
        self.frozen_field = field.clone();

        let (pairings, _byes) = cup::pair_knockout_round(&field);
        let total = cup::total_rounds(field.len());
        if self.season_start_year == 0 {
            self.season_start_year = current_date.year();
        }
        let (season_start, season_end) = self.season_window();
        let mut date = cup::cup_round_date(season_start, season_end, 1, total);
        // The groups only finish late in the window, so the evenly-spaced
        // round-one slot is usually in the past by the time we draw. Push
        // it onto the next midweek so the bracket actually runs.
        if date <= current_date {
            date = cup::next_midweek(current_date + Duration::days(7));
        }
        let dt = NaiveDateTime::new(date, NaiveTime::from_hms_opt(0, 0, 0).unwrap());
        let tour = self.build_tour(1, &pairings, dt);
        self.league.schedule.tours.push(tour);
        debug!(
            "🏆 Playoff {} round 1 drawn: {} ties, {} entrants, first leg {}",
            self.league.name,
            self.league.schedule.tours[0].items.len(),
            field.len(),
            date
        );
    }

    /// Once every existing round is fully played and more than one team is
    /// left, draw the next round from the survivors.
    fn maybe_generate_next_round(&mut self, current_date: NaiveDate) {
        if self.frozen_field.len() < 2 {
            return;
        }
        let alive = match cup::advancing_teams(&self.league.schedule.tours, &self.frozen_field) {
            Some(a) => a,
            None => return, // a tie in the latest round is still pending
        };
        if alive.len() < 2 {
            return; // champion decided — bracket complete
        }
        // Reseed survivors by their original entry seed so byes keep
        // falling to the stronger sides.
        let alive_seeded: Vec<u32> = self
            .frozen_field
            .iter()
            .copied()
            .filter(|t| alive.contains(t))
            .collect();
        let (pairings, _byes) = cup::pair_knockout_round(&alive_seeded);
        if pairings.is_empty() {
            return;
        }
        let next_round = (self.league.schedule.tours.len() + 1) as u8;
        let total = cup::total_rounds(self.frozen_field.len());
        let (season_start, season_end) = self.season_window();
        let mut date = cup::cup_round_date(season_start, season_end, next_round, total);
        if date <= current_date {
            date = cup::next_midweek(current_date + Duration::days(7));
        }
        let dt = NaiveDateTime::new(date, NaiveTime::from_hms_opt(0, 0, 0).unwrap());
        let tour = self.build_tour(next_round, &pairings, dt);
        debug!(
            "🏆 Playoff {} round {} drawn: {} ties on {}",
            self.league.name,
            next_round,
            tour.items.len(),
            date
        );
        self.league.schedule.tours.push(tour);
    }

    /// Build (but do not play) today's playoff matches. Mirrors
    /// [`League::simulate_build`] for the knockout side: resets at the
    /// competition's season start, draws round one once the groups have all
    /// finished, then collects today's ties for a batched engine dispatch.
    pub fn simulate_build(
        &mut self,
        clubs: &[Club],
        groups: &[GroupStanding],
        ctx: &GlobalContext<'_>,
    ) -> LeagueBuildOutput {
        let current_date = ctx.simulation.date.date();

        if self.league.settings.is_time_for_new_schedule(&ctx.simulation) {
            self.reset_edition(ctx);
        }

        // Draw round one exactly once, when every member group has played
        // out its full programme.
        if self.league.schedule.tours.is_empty() {
            let members = self.member_standings(groups);
            let ready = members.len() >= 2 && members.iter().all(|g| g.complete);
            if ready {
                self.draw_round_one(&members, current_date);
            }
        }

        let total_rounds = cup::total_rounds(self.frozen_field.len());
        let scheduled = self.collect_today_matches(current_date, total_rounds);
        if scheduled.is_empty() {
            return LeagueBuildOutput {
                matches: Vec::new(),
                pending: None,
                immediate: Some(LeagueResult::new(self.league.id, LeagueTableResult {})),
            };
        }

        // Knockout: `knockout = true` so a level score is settled by extra
        // time and (if needed) penalties.
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

    /// Apply played playoff results back onto the bracket and draw the next
    /// round if today closed one out.
    pub fn simulate_process(
        &mut self,
        match_results: Vec<MatchResult>,
        pending: LeaguePendingState,
        _clubs: &[Club],
        _ctx: &GlobalContext<'_>,
        current_date: NaiveDate,
    ) -> LeagueResult {
        let LeaguePendingState {
            mut scheduled_matches,
            ..
        } = pending;

        self.league
            .apply_matchday_results(&mut scheduled_matches, &match_results);

        for mr in &match_results {
            self.league
                .matches
                .push(mr.copy_without_data_positions(), current_date);
            self.league.schedule.update_match_result(&mr.id, &mr.score);
        }

        self.maybe_generate_next_round(current_date);

        LeagueResult::with_match_result(self.league.id, LeagueTableResult {}, match_results)
    }

    /// The current edition's champion, if the bracket has resolved to a
    /// single surviving team.
    pub fn champion(&self) -> Option<u32> {
        cup::cup_champion(&self.league.schedule.tours, &self.frozen_field)
    }

    /// The final tie's `(home, away)` pairing once the edition has a
    /// champion — the last tour holds exactly the one deciding match.
    fn final_pairing(&self) -> Option<(u32, u32)> {
        self.champion()?;
        let last = self.league.schedule.tours.last()?;
        if last.items.len() == 1 {
            let i = &last.items[0];
            Some((i.home_team_id, i.away_team_id))
        } else {
            None
        }
    }

    /// Champion team id only if a fresh winner-trophy fan-out is still owed
    /// for this edition (fires exactly once per edition).
    pub fn should_emit_winner_award(&self) -> Option<u32> {
        let team_id = self.champion()?;
        if self.award_emitted_season_start_year == Some(self.season_start_year)
            && self.award_emitted_winner_team_id == Some(team_id)
        {
            return None;
        }
        Some(team_id)
    }

    /// Record that the winner fan-out has run for `team_id` on `date`.
    pub fn mark_winner_award_emitted(&mut self, team_id: u32, date: NaiveDate) {
        self.award_emitted_season_start_year = Some(self.season_start_year);
        self.award_emitted_winner_team_id = Some(team_id);
        self.award_emitted_on = Some(date);
    }

    /// Back-compat single-shot driver (build → engine → process). Production
    /// paths go through `Country::simulate_build` so playoff matches join
    /// the world's single global dispatch batch.
    pub fn simulate(
        &mut self,
        clubs: &[Club],
        groups: &[GroupStanding],
        ctx: &GlobalContext<'_>,
    ) -> LeagueResult {
        let current_date = ctx.simulation.date.date();
        let output = self.simulate_build(clubs, groups, ctx);
        if let Some(immediate) = output.immediate {
            return immediate;
        }
        let match_results = MatchRuntime::engine_pool().play(output.matches);
        let pending = output
            .pending
            .expect("playoff simulate_build with matches must produce a pending state");
        self.simulate_process(match_results, pending, clubs, ctx, current_date)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::league::{LeagueSettings, LeagueGroup};
    use crate::r#match::{Score, TeamScore};

    fn settings() -> LeagueSettings {
        LeagueSettings {
            season_starting_half: crate::league::DayMonthPeriod::new(1, 3, 30, 6),
            season_ending_half: crate::league::DayMonthPeriod::new(1, 7, 10, 12),
            tier: 1,
            promotion_spots: 0,
            relegation_spots: 0,
            league_group: Some(LeagueGroup {
                name: "Playoff".into(),
                competition: "Test League".into(),
                total_groups: 2,
                playoff: None,
            }),
        }
    }

    fn playoff() -> LeaguePlayoff {
        let league = League::new(900_001, "Test Playoff".into(), "test-playoff".into(), 1, 5000, settings(), false);
        LeaguePlayoff::new(league, "Test League".into(), vec![10, 20], 4)
    }

    fn group(id: u32, complete: bool, teams: Vec<u32>) -> GroupStanding {
        GroupStanding { league_id: id, complete, ordered_team_ids: teams }
    }

    #[test]
    fn seeds_interleave_groups_by_finishing_position() {
        let pf = playoff();
        let members = vec![
            group(10, true, vec![1, 2, 3, 4, 5]),
            group(20, true, vec![6, 7, 8, 9, 10]),
        ];
        let refs: Vec<&GroupStanding> = members.iter().collect();
        // qpg = 4 → E1,W1,E2,W2,E3,W3,E4,W4
        assert_eq!(pf.build_seed_field(&refs), vec![1, 6, 2, 7, 3, 8, 4, 9]);
    }

    #[test]
    fn not_ready_until_all_groups_complete() {
        let pf = playoff();
        let members = vec![
            group(10, true, vec![1, 2, 3, 4]),
            group(20, false, vec![5, 6, 7, 8]),
        ];
        let refs = pf.member_standings(&members);
        assert_eq!(refs.len(), 2);
        assert!(!refs.iter().all(|g| g.complete));
    }

    /// Drive a full 8-team bracket by hand (seed → draw → play each round →
    /// champion) to prove the group-seeded field resolves to one winner.
    #[test]
    fn eight_team_bracket_resolves_to_single_champion() {
        let mut pf = playoff();
        pf.season_start_year = 2026;
        let members_owned = vec![
            group(10, true, vec![1, 2, 3, 4]),
            group(20, true, vec![5, 6, 7, 8]),
        ];
        let members: Vec<&GroupStanding> = members_owned.iter().collect();
        let today = NaiveDate::from_ymd_opt(2026, 11, 1).unwrap();
        pf.draw_round_one(&members, today);
        assert_eq!(pf.frozen_field.len(), 8);
        assert!(pf.champion().is_none());

        // Play out every round: lower id always wins, then draw the next.
        let mut day = today;
        loop {
            let Some(tour) = pf.league.schedule.tours.last() else { break };
            if tour.items.iter().all(|i| i.result.is_some()) {
                // nothing new to play; try to advance
            }
            let round = tour.num;
            let pairings: Vec<(usize, u32, u32)> = tour
                .items
                .iter()
                .enumerate()
                .filter(|(_, i)| i.result.is_none())
                .map(|(idx, i)| (idx, i.home_team_id, i.away_team_id))
                .collect();
            if pairings.is_empty() {
                break;
            }
            for (idx, home, away) in pairings {
                let (hg, ag) = if home < away { (1u8, 0u8) } else { (0u8, 1u8) };
                let score = Score {
                    home_team: TeamScore::new_with_score(home, hg),
                    away_team: TeamScore::new_with_score(away, ag),
                    details: Vec::new(),
                    home_shootout: 0,
                    away_shootout: 0,
                };
                pf.league.schedule.tours[(round - 1) as usize].items[idx].result = Some(score);
            }
            day += Duration::days(7);
            pf.maybe_generate_next_round(day);
        }
        // Team 1 (top overall seed, lowest id) wins every tie → champion.
        assert_eq!(pf.champion(), Some(1));
    }
}
