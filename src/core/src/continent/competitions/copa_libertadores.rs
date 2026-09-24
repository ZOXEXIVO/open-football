use super::{
    COPA_LIBERTADORES_ID, CompetitionStage, ContinentalMatch, ContinentalMatchweek, GroupTable,
    KnockoutTie,
};
use crate::Club;
use crate::continent::ContinentalRankings;
use crate::league::Season;
use crate::league::simulation::matchday::{MatchdayCommitments, MatchdayPool};
use crate::r#match::squad::selection::model::MatchSelectionGameModel;
use crate::r#match::{Match, MatchResult, SelectionCompetition, SelectionContext};
use crate::{MatchRuntime, TeamType};
use chrono::{Datelike, NaiveDate, Weekday};
use log::{debug, info};
use std::collections::HashMap;

pub const COPA_LIBERTADORES_SLUG: &str = "copa-libertadores";

// ─── Main Copa Libertadores struct ──────────────────────────────────
//
// South America's elite continental club competition (CONMEBOL).
// Architecturally identical to the Champions League — the core
// continental engine is competition-agnostic — but seeded only from
// South-American leagues and scheduled on a Thursday cadence shifted a
// day off the UEFA midweek dates so the two continents don't collide.

#[derive(Debug, Clone)]
pub struct CopaLibertadores {
    pub participating_clubs: Vec<u32>,
    pub current_stage: CompetitionStage,
    pub groups: Vec<GroupTable>,
    pub knockout_round: Vec<KnockoutTie>,
    pub matches: Vec<ContinentalMatch>,
    pub prize_pool: f64,
    pub season_year: u16,
}

impl Default for CopaLibertadores {
    fn default() -> Self {
        Self::new()
    }
}

impl CopaLibertadores {
    /// Tuesday to Thursday nights, the groups spread across them.
    const MATCH_WEEKDAYS: [Weekday; 3] = [Weekday::Tue, Weekday::Wed, Weekday::Thu];
    /// The single-match final is a Saturday showpiece.
    const FINAL_WEEKDAY: [Weekday; 1] = [Weekday::Sat];

    pub fn new() -> Self {
        CopaLibertadores {
            participating_clubs: Vec::new(),
            current_stage: CompetitionStage::NotStarted,
            groups: Vec::new(),
            knockout_round: Vec::new(),
            matches: Vec::new(),
            prize_pool: 300_000_000.0,
            season_year: 0,
        }
    }

    /// Conduct draw: seed clubs into 8 groups of 4, generate fixtures.
    pub fn conduct_draw(
        &mut self,
        clubs: &[u32],
        _rankings: &ContinentalRankings,
        date: NaiveDate,
    ) {
        if clubs.len() < 8 {
            debug!(
                "Copa Libertadores: not enough clubs ({}) for draw",
                clubs.len()
            );
            return;
        }

        // Take up to 32 clubs (8 groups x 4)
        let count = clubs.len().min(32);
        self.participating_clubs = clubs[..count].to_vec();
        self.season_year = date.year() as u16;

        // Create groups: distribute clubs round-robin into 8 groups
        let num_groups = (count / 4).clamp(1, 8);
        self.groups = (0..num_groups)
            .map(|g| {
                let team_ids: Vec<u32> = (0..4)
                    .filter_map(|i| {
                        let idx = g + i * num_groups;
                        self.participating_clubs.get(idx).copied()
                    })
                    .collect();
                GroupTable::new(&team_ids)
            })
            .collect();

        // Generate group stage fixtures (6 matchdays). Thursday cadence,
        // one day after the UEFA midweek slate.
        self.matches.clear();
        let season = Season::new(self.season_year);

        for (group_idx, group) in self.groups.iter().enumerate() {
            let teams: Vec<u32> = group.rows.iter().map(|r| r.team_id).collect();
            if teams.len() < 4 {
                continue;
            }

            // Round-robin: 6 matches per group (each pair plays home & away)
            let fixtures = [
                (0, 1, 2, 3, 0), // MD1: 0v1, 2v3
                (2, 0, 3, 1, 1), // MD2: 2v0, 3v1
                (0, 3, 1, 2, 2), // MD3: 0v3, 1v2
                (3, 0, 2, 1, 3), // MD4: 3v0, 2v1 (reverse)
                (1, 0, 3, 2, 4), // MD5: 1v0, 3v2
                (0, 2, 1, 3, 5), // MD6: 0v2, 1v3
            ];

            for (h1, a1, h2, a2, md) in fixtures {
                self.matches.push(ContinentalMatch {
                    home_team: teams[h1],
                    away_team: teams[a1],
                    date: ContinentalMatchweek::day(
                        &season,
                        ContinentalMatchweek::LEAGUE_PHASE[md],
                        &Self::MATCH_WEEKDAYS,
                        group_idx,
                    ),
                    stage: CompetitionStage::GroupStage,
                    match_id: String::new(),
                    result: None,
                });
                self.matches.push(ContinentalMatch {
                    home_team: teams[h2],
                    away_team: teams[a2],
                    date: ContinentalMatchweek::day(
                        &season,
                        ContinentalMatchweek::LEAGUE_PHASE[md],
                        &Self::MATCH_WEEKDAYS,
                        group_idx,
                    ),
                    stage: CompetitionStage::GroupStage,
                    match_id: String::new(),
                    result: None,
                });
            }
        }

        self.current_stage = CompetitionStage::GroupStage;
    }

    /// Generate knockout round fixtures after group stage completes.
    pub fn generate_knockout_fixtures(&mut self) {
        // Collect group winners and runners-up
        let mut winners = Vec::new();
        let mut runners_up = Vec::new();

        for group in &self.groups {
            if group.rows.len() >= 2 {
                let (w, r) = group.qualifiers();
                winners.push(w);
                runners_up.push(r);
            }
        }

        // Draw: match winners against runners-up (avoiding same group)
        self.knockout_round.clear();
        let num_ties = winners.len().min(runners_up.len());
        for (i, &winner) in winners.iter().enumerate().take(num_ties) {
            // Simple draw: winner i vs runner-up (i+1) % n
            let r_idx = (i + 1) % runners_up.len();
            self.knockout_round
                .push(KnockoutTie::new(winner, runners_up[r_idx]));
        }

        // Schedule R16 matches (Thursday cadence, next calendar year).
        let season = Season::new(self.season_year);

        for (i, tie) in self.knockout_round.iter().enumerate() {
            let leg1_date = ContinentalMatchweek::leg(
                &season,
                &ContinentalMatchweek::ROUND_OF_16_FIRST_LEGS,
                &Self::MATCH_WEEKDAYS,
                i,
            );
            let leg2_date = ContinentalMatchweek::leg(
                &season,
                &ContinentalMatchweek::ROUND_OF_16_SECOND_LEGS,
                &Self::MATCH_WEEKDAYS,
                i,
            );

            self.matches.push(ContinentalMatch {
                home_team: tie.home_team,
                away_team: tie.away_team,
                date: leg1_date,
                stage: CompetitionStage::RoundOf16,
                match_id: String::new(),
                result: None,
            });
            self.matches.push(ContinentalMatch {
                home_team: tie.away_team,
                away_team: tie.home_team,
                date: leg2_date,
                stage: CompetitionStage::RoundOf16,
                match_id: String::new(),
                result: None,
            });
        }

        self.current_stage = CompetitionStage::RoundOf16;

        info!(
            "Copa Libertadores R16: {} ties, {} matches scheduled",
            self.knockout_round.len(),
            self.knockout_round.len() * 2
        );
    }

    pub fn has_matches_today(&self, date: NaiveDate) -> bool {
        self.matches.iter().any(|m| m.date == date)
    }

    /// Play today's matches using the real match engine.
    /// Returns MatchResults that flow through the standard stat pipeline.
    pub(crate) fn play_matches(
        &mut self,
        clubs: &HashMap<u32, &Club>,
        date: NaiveDate,
        commitments: &MatchdayCommitments,
    ) -> Vec<MatchResult> {
        let todays_matches: Vec<ContinentalMatch> = self
            .matches
            .iter()
            .filter(|m| m.date == date)
            .cloned()
            .collect();

        if todays_matches.is_empty() {
            return Vec::new();
        }

        // Build Match objects (same pattern as League::build_match).
        // Each side gets a `selection_ctx` carrying the OTHER side's
        // baseline tactic so a tactically-aware coach can flip to a
        // counter shape — without this every continental fixture
        // resolved the counter branch with `opponent_tactic = None`
        // and stayed on the persistent shape regardless of opponent.
        let engine_matches: Vec<Match> = todays_matches
            .iter()
            .filter_map(|cm| {
                let home_club = clubs.get(&cm.home_team)?;
                let away_club = clubs.get(&cm.away_team)?;

                let home_team = home_club.teams.teams.first()?;
                let away_team = away_club.teams.teams.first()?;

                // Same matchday pool the league builds — reserves, earned
                // academy call-ups, the keeper room and the shortfall borrow.
                // A club in a crisis names the same eighteen in Europe that it
                // names on a Saturday.
                let home_force = MatchdayPool::offer(
                    home_club,
                    home_team.id,
                    false,
                    home_team.team_type == TeamType::Main,
                    date,
                    commitments,
                );
                let away_force = MatchdayPool::offer(
                    away_club,
                    away_team.id,
                    false,
                    away_team.team_type == TeamType::Main,
                    date,
                    commitments,
                );

                let home_baseline = home_team.tactics.as_ref().map(|t| t.tactic_type);
                let away_baseline = away_team.tactics.as_ref().map(|t| t.tactic_type);

                let mut home_ctx = SelectionContext {
                    is_friendly: false,
                    date,
                    match_importance: 1.0,
                    philosophy: None,
                    opponent_tactic: away_baseline,
                    competition: SelectionCompetition::ContinentalCup,
                    game_model: None,
                    ..SelectionContext::default()
                };
                let mut away_ctx = SelectionContext {
                    is_friendly: false,
                    date,
                    match_importance: 1.0,
                    philosophy: None,
                    opponent_tactic: home_baseline,
                    competition: SelectionCompetition::ContinentalCup,
                    game_model: None,
                    ..SelectionContext::default()
                };

                // Fixture-aware game model per side (opponent roster threat
                // axes + venue). Continental scheduling doesn't expose an
                // upcoming-fixture count here, so congestion stays neutral.
                let is_derby = home_club.is_rival(away_club.id) || away_club.is_rival(home_club.id);
                home_ctx.game_model = Some(MatchSelectionGameModel::build_for_fixture(
                    &home_ctx, home_team, away_team, true, 0, is_derby,
                ));
                away_ctx.game_model = Some(MatchSelectionGameModel::build_for_fixture(
                    &away_ctx, away_team, home_team, false, 0, is_derby,
                ));

                let home_squad = home_team.get_enhanced_match_squad(&home_force, &home_ctx);
                let away_squad = away_team.get_enhanced_match_squad(&away_force, &away_ctx);

                let match_id = format!(
                    "lib_{}_{}_{}",
                    date.format("%Y%m%d"),
                    cm.home_team,
                    cm.away_team
                );

                // Knockout legs need penalties for the second leg if
                // aggregate ends level. Group games use the regular
                // (non-knockout) constructor — a draw is a valid group
                // outcome.
                let is_knockout_stage = matches!(
                    cm.stage,
                    CompetitionStage::RoundOf16
                        | CompetitionStage::QuarterFinals
                        | CompetitionStage::SemiFinals
                        | CompetitionStage::Final
                );
                Some(if is_knockout_stage {
                    Match::make_knockout(
                        match_id,
                        COPA_LIBERTADORES_ID,
                        COPA_LIBERTADORES_SLUG,
                        home_squad,
                        away_squad,
                    )
                } else {
                    Match::make(
                        match_id,
                        COPA_LIBERTADORES_ID,
                        COPA_LIBERTADORES_SLUG,
                        home_squad,
                        away_squad,
                        false, // not friendly -- competitive
                    )
                })
            })
            .collect();

        if engine_matches.is_empty() {
            return Vec::new();
        }

        // Play all matches through the engine pool
        let results = MatchRuntime::engine_pool().play(engine_matches);

        // Store results back on the matches
        for (cm, result) in todays_matches.iter().zip(results.iter()) {
            let home_goals = result.score.home_team.get();
            let away_goals = result.score.away_team.get();
            if let Some(m) = self.matches.iter_mut().find(|m| {
                m.date == cm.date && m.home_team == cm.home_team && m.away_team == cm.away_team
            }) {
                m.match_id = result.id.clone();
                m.result = Some((home_goals, away_goals));
            }
        }

        // Update group tables / knockout ties / final from results
        for (cm, result) in todays_matches.iter().zip(results.iter()) {
            let home_goals = result.score.home_team.get();
            let away_goals = result.score.away_team.get();
            let shootout = if result.score.had_shootout() {
                Some((result.score.home_shootout, result.score.away_shootout))
            } else {
                None
            };

            self.apply_match_result(
                &cm.stage,
                cm.home_team,
                cm.away_team,
                home_goals,
                away_goals,
                shootout,
            );

            debug!(
                "Libertadores: {} {} - {} {} ({:?})",
                cm.home_team, home_goals, away_goals, cm.away_team, cm.stage
            );
        }

        // Check if group stage is complete (all group matches played)
        let group_stage_complete = matches!(self.current_stage, CompetitionStage::GroupStage)
            && self
                .groups
                .iter()
                .all(|g| g.rows.iter().all(|r| r.played >= 6));

        if group_stage_complete {
            info!("Copa Libertadores group stage complete -- generating R16 draw");
            self.generate_knockout_fixtures();
        }

        // Advance the knockout bracket if today's results completed the
        // current round (R16 -> QF -> SF -> Final).
        self.maybe_advance_knockout();

        results
    }

    /// Apply one played fixture's score to the group table, knockout tie,
    /// or final it belongs to. Knockout legs fold extra-time / penalties in
    /// through `shootout`; the single-match final reads its winner straight
    /// from the (already decisive) knockout score.
    fn apply_match_result(
        &mut self,
        stage: &CompetitionStage,
        home_team: u32,
        away_team: u32,
        home_goals: u8,
        away_goals: u8,
        shootout: Option<(u8, u8)>,
    ) {
        match stage {
            CompetitionStage::GroupStage => {
                // Find and update the group containing these teams
                for group in &mut self.groups {
                    let has_home = group.rows.iter().any(|r| r.team_id == home_team);
                    let has_away = group.rows.iter().any(|r| r.team_id == away_team);
                    if has_home && has_away {
                        group.update(home_team, away_team, home_goals, away_goals);
                        break;
                    }
                }
            }
            CompetitionStage::RoundOf16
            | CompetitionStage::QuarterFinals
            | CompetitionStage::SemiFinals => {
                for tie in &mut self.knockout_round {
                    if tie.home_team == home_team && tie.away_team == away_team {
                        if tie.leg1_score.is_none() {
                            tie.record_leg1(home_goals, away_goals);
                        }
                    } else if tie.home_team == away_team
                        && tie.away_team == home_team
                        && tie.leg2_score.is_none()
                    {
                        tie.record_leg2_with_shootout(home_goals, away_goals, shootout);
                    }
                }
            }
            CompetitionStage::Final => {
                // One-match final: the engine's knockout score is decisive,
                // so the winner comes directly from goals (with the shootout
                // breaking a level score). Record it on the lone final tie
                // and lock the stage so `final_result()` can read it.
                let winner = Self::single_match_winner(
                    home_team, away_team, home_goals, away_goals, shootout,
                );
                if let Some(tie) = self.knockout_round.iter_mut().find(|t| {
                    (t.home_team == home_team && t.away_team == away_team)
                        || (t.home_team == away_team && t.away_team == home_team)
                }) {
                    tie.leg1_score = Some((home_goals, away_goals));
                    tie.shootout = shootout;
                    tie.winner = Some(winner);
                }
                self.current_stage = CompetitionStage::Final;
            }
            _ => {}
        }
    }

    /// Winner of a one-off knockout match. The engine's knockout score is
    /// already decisive (extra time + penalties folded in), so equal goals
    /// means a shootout settled it.
    fn single_match_winner(
        home: u32,
        away: u32,
        home_goals: u8,
        away_goals: u8,
        shootout: Option<(u8, u8)>,
    ) -> u32 {
        use std::cmp::Ordering;
        match home_goals.cmp(&away_goals) {
            Ordering::Greater => home,
            Ordering::Less => away,
            Ordering::Equal => match shootout {
                Some((sh, sa)) if sa > sh => away,
                _ => home,
            },
        }
    }

    /// True once every fixture scheduled at `stage` has a recorded result.
    fn stage_matches_played(&self, stage: &CompetitionStage) -> bool {
        let want = std::mem::discriminant(stage);
        let mut found = false;
        for m in self.matches.iter() {
            if std::mem::discriminant(&m.stage) == want {
                found = true;
                if m.result.is_none() {
                    return false;
                }
            }
        }
        found
    }

    /// True when the current knockout round is fully resolved and ready to
    /// feed the next round: every fixture at `stage` is played and every
    /// live tie has a decided winner.
    fn knockout_stage_complete(&self, stage: CompetitionStage) -> bool {
        self.stage_matches_played(&stage)
            && !self.knockout_round.is_empty()
            && self.knockout_round.iter().all(|t| t.winner.is_some())
    }

    /// Winners of the current knockout round, in tie order.
    fn completed_winners(&self) -> Vec<u32> {
        self.knockout_round
            .iter()
            .filter_map(|t| t.winner)
            .collect()
    }

    /// Replace the live knockout round with a fresh two-legged round drawn
    /// from `winners` (paired 0v1, 2v3, ...) and schedule both legs. Leg 1
    /// is hosted by the first team of each pair, leg 2 by the second.
    fn schedule_two_leg_round(
        &mut self,
        winners: &[u32],
        stage: CompetitionStage,
        (leg1_weeks, leg2_weeks): ([u32; 1], [u32; 1]),
    ) {
        let ties: Vec<KnockoutTie> = winners
            .as_chunks::<2>()
            .0
            .iter()
            .map(|pair| KnockoutTie::new(pair[0], pair[1]))
            .collect();

        let season = Season::new(self.season_year);
        for (i, tie) in ties.iter().enumerate() {
            let leg1_date =
                ContinentalMatchweek::leg(&season, &leg1_weeks, &Self::MATCH_WEEKDAYS, i);
            let leg2_date =
                ContinentalMatchweek::leg(&season, &leg2_weeks, &Self::MATCH_WEEKDAYS, i);

            self.matches.push(ContinentalMatch {
                home_team: tie.home_team,
                away_team: tie.away_team,
                date: leg1_date,
                stage: stage.clone(),
                match_id: String::new(),
                result: None,
            });
            self.matches.push(ContinentalMatch {
                home_team: tie.away_team,
                away_team: tie.home_team,
                date: leg2_date,
                stage: stage.clone(),
                match_id: String::new(),
                result: None,
            });
        }

        self.knockout_round = ties;
        self.current_stage = stage;
    }

    /// Schedule the single-match final between the two semifinal winners and
    /// move the bracket to `Final`. The winner is recorded later, when the
    /// match is played (see `apply_match_result`).
    fn schedule_final(&mut self, finalists: &[u32]) {
        if finalists.len() < 2 {
            debug!(
                "Copa Libertadores: cannot schedule final with {} finalist(s)",
                finalists.len()
            );
            return;
        }

        self.knockout_round = vec![KnockoutTie::new(finalists[0], finalists[1])];
        self.matches.push(ContinentalMatch {
            home_team: finalists[0],
            away_team: finalists[1],
            date: ContinentalMatchweek::day(
                &Season::new(self.season_year),
                ContinentalMatchweek::FINAL,
                &Self::FINAL_WEEKDAY,
                0,
            ),
            stage: CompetitionStage::Final,
            match_id: String::new(),
            result: None,
        });
        self.current_stage = CompetitionStage::Final;
    }

    /// Advance the knockout bracket when today's results finish the current
    /// round: R16 -> QF -> SF -> Final, two legs each except the one-match
    /// final. A round with an undecided tie holds the next draw (logged)
    /// instead of advancing.
    fn maybe_advance_knockout(&mut self) {
        match self.current_stage {
            CompetitionStage::RoundOf16 => {
                if self.knockout_stage_complete(CompetitionStage::RoundOf16) {
                    let winners = self.completed_winners();
                    self.schedule_two_leg_round(
                        &winners,
                        CompetitionStage::QuarterFinals,
                        ContinentalMatchweek::QUARTER_FINAL_LEGS,
                    );
                    info!(
                        "Copa Libertadores QF: {} ties scheduled",
                        self.knockout_round.len()
                    );
                } else if self.stage_matches_played(&CompetitionStage::RoundOf16) {
                    debug!("Copa Libertadores: R16 legs done but a tie is undecided; QF draw held");
                }
            }
            CompetitionStage::QuarterFinals => {
                if self.knockout_stage_complete(CompetitionStage::QuarterFinals) {
                    let winners = self.completed_winners();
                    self.schedule_two_leg_round(
                        &winners,
                        CompetitionStage::SemiFinals,
                        ContinentalMatchweek::SEMI_FINAL_LEGS,
                    );
                    info!(
                        "Copa Libertadores SF: {} ties scheduled",
                        self.knockout_round.len()
                    );
                } else if self.stage_matches_played(&CompetitionStage::QuarterFinals) {
                    debug!("Copa Libertadores: QF legs done but a tie is undecided; SF draw held");
                }
            }
            CompetitionStage::SemiFinals => {
                if self.knockout_stage_complete(CompetitionStage::SemiFinals) {
                    let finalists = self.completed_winners();
                    self.schedule_final(&finalists);
                    info!("Copa Libertadores Final scheduled");
                } else if self.stage_matches_played(&CompetitionStage::SemiFinals) {
                    debug!("Copa Libertadores: SF legs done but a tie is undecided; final held");
                }
            }
            _ => {}
        }
    }

    pub fn get_club_points(&self, club_id: u32) -> f32 {
        if !self.participating_clubs.contains(&club_id) {
            return 0.0;
        }

        // Points from group stage performance. Slightly below the UEFA
        // Champions League global coefficient base (10.0) but with the
        // same performance multiplier — Libertadores is South America's
        // elite competition.
        for group in &self.groups {
            if let Some(row) = group.rows.iter().find(|r| r.team_id == club_id) {
                return 9.0 + row.points as f32 * 2.0;
            }
        }

        9.0
    }

    /// Final-result accessor used by the season-end happiness pipeline to
    /// fire `TrophyWon` / `CupFinalDefeat`. Returns `(winner, loser)` once
    /// the Final has been resolved (single knockout tie at `Final` stage
    /// with a winner recorded), and `None` until `maybe_advance_knockout`
    /// has scheduled the final and its result has been applied.
    pub fn final_result(&self) -> Option<(u32, u32)> {
        if !matches!(self.current_stage, CompetitionStage::Final) {
            return None;
        }
        let tie = self.knockout_round.first()?;
        let winner = tie.winner?;
        let loser = if winner == tie.home_team {
            tie.away_team
        } else {
            tie.home_team
        };
        Some((winner, loser))
    }
}

#[cfg(test)]
mod lifecycle_tests {
    use super::*;
    use crate::continent::ContinentalRankings;

    fn draw_date() -> NaiveDate {
        NaiveDate::from_ymd_opt(2025, 9, 1).unwrap()
    }

    /// 32 distinct club ids (1..=32) — a full group-stage field.
    fn thirty_two_clubs() -> Vec<u32> {
        (1..=32).collect()
    }

    /// Resolve every tie in the current knockout round in favour of its
    /// home team (leg 1: 2-0 home; leg 2: 0-0 → aggregate 2-0) and mark all
    /// of that stage's fixtures played — exactly the state
    /// `maybe_advance_knockout` reads before drawing the next round.
    fn resolve_current_round(copa: &mut CopaLibertadores) {
        let stage = copa.current_stage.clone();
        let pairings: Vec<(u32, u32)> = copa
            .knockout_round
            .iter()
            .map(|t| (t.home_team, t.away_team))
            .collect();
        for (home, away) in pairings {
            copa.apply_match_result(&stage, home, away, 2, 0, None); // leg 1
            copa.apply_match_result(&stage, away, home, 0, 0, None); // leg 2
        }
        let want = std::mem::discriminant(&stage);
        for m in copa.matches.iter_mut() {
            if std::mem::discriminant(&m.stage) == want {
                m.result.get_or_insert((1, 0));
            }
        }
    }

    #[test]
    fn every_fixture_is_midweek_except_the_saturday_final_in_any_year() {
        for year in [2026, 2027, 2028] {
            let mut copa = CopaLibertadores::new();
            copa.conduct_draw(
                &thirty_two_clubs(),
                &ContinentalRankings::new(),
                NaiveDate::from_ymd_opt(year, 8, 16).unwrap(),
            );
            copa.generate_knockout_fixtures();
            for _ in 0..3 {
                resolve_current_round(&mut copa);
                copa.maybe_advance_knockout();
            }
            assert!(matches!(copa.current_stage, CompetitionStage::Final));

            for m in &copa.matches {
                let day = m.date.weekday();
                if matches!(m.stage, CompetitionStage::Final) {
                    assert_eq!(day, Weekday::Sat, "{year}: the final is a Saturday");
                } else {
                    assert!(
                        CopaLibertadores::MATCH_WEEKDAYS.contains(&day),
                        "{year}: {:?} fixture on a {}",
                        m.stage,
                        day
                    );
                }
            }

            // Every two-legged tie plays its return leg after the first. A
            // round is scheduled first leg, then return leg, tie by tie.
            for stage in [
                CompetitionStage::RoundOf16,
                CompetitionStage::QuarterFinals,
                CompetitionStage::SemiFinals,
            ] {
                let want = std::mem::discriminant(&stage);
                let legs: Vec<&ContinentalMatch> = copa
                    .matches
                    .iter()
                    .filter(|m| std::mem::discriminant(&m.stage) == want)
                    .collect();
                for pair in legs.chunks(2) {
                    let (first, ret) = (pair[0], pair[1]);
                    assert_eq!(
                        (first.home_team, first.away_team),
                        (ret.away_team, ret.home_team)
                    );
                    assert!(
                        first.date < ret.date,
                        "{year}: return leg not after the first"
                    );
                }
            }
        }
    }

    #[test]
    fn conduct_draw_creates_eight_groups_and_ninety_six_matches() {
        let mut copa = CopaLibertadores::new();
        copa.conduct_draw(
            &thirty_two_clubs(),
            &ContinentalRankings::new(),
            draw_date(),
        );

        assert_eq!(copa.groups.len(), 8);
        let group_matches = copa
            .matches
            .iter()
            .filter(|m| matches!(m.stage, CompetitionStage::GroupStage))
            .count();
        assert_eq!(group_matches, 96);
        assert!(matches!(copa.current_stage, CompetitionStage::GroupStage));
        assert_eq!(copa.season_year, 2025);
    }

    #[test]
    fn group_stage_completion_schedules_eight_r16_ties() {
        let mut copa = CopaLibertadores::new();
        copa.conduct_draw(
            &thirty_two_clubs(),
            &ContinentalRankings::new(),
            draw_date(),
        );

        // The R16 draw reads the current group standings; with mocked-complete
        // tables it yields one knockout berth pairing per group.
        copa.generate_knockout_fixtures();

        assert_eq!(copa.knockout_round.len(), 8);
        let r16_matches = copa
            .matches
            .iter()
            .filter(|m| matches!(m.stage, CompetitionStage::RoundOf16))
            .count();
        assert_eq!(r16_matches, 16);
        assert!(matches!(copa.current_stage, CompetitionStage::RoundOf16));
    }

    #[test]
    fn knockout_advances_r16_to_qf_to_sf_to_final() {
        let mut copa = CopaLibertadores::new();
        copa.conduct_draw(
            &thirty_two_clubs(),
            &ContinentalRankings::new(),
            draw_date(),
        );
        copa.generate_knockout_fixtures();

        // R16 -> QF: 8 winners draw into 4 two-legged ties.
        resolve_current_round(&mut copa);
        copa.maybe_advance_knockout();
        assert!(matches!(
            copa.current_stage,
            CompetitionStage::QuarterFinals
        ));
        assert_eq!(copa.knockout_round.len(), 4);
        assert_eq!(
            copa.matches
                .iter()
                .filter(|m| matches!(m.stage, CompetitionStage::QuarterFinals))
                .count(),
            8
        );

        // QF -> SF: 4 winners draw into 2 ties.
        resolve_current_round(&mut copa);
        copa.maybe_advance_knockout();
        assert!(matches!(copa.current_stage, CompetitionStage::SemiFinals));
        assert_eq!(copa.knockout_round.len(), 2);
        assert_eq!(
            copa.matches
                .iter()
                .filter(|m| matches!(m.stage, CompetitionStage::SemiFinals))
                .count(),
            4
        );

        // SF -> Final: 2 finalists, one match.
        resolve_current_round(&mut copa);
        copa.maybe_advance_knockout();
        assert!(matches!(copa.current_stage, CompetitionStage::Final));
        assert_eq!(copa.knockout_round.len(), 1);
        assert_eq!(
            copa.matches
                .iter()
                .filter(|m| matches!(m.stage, CompetitionStage::Final))
                .count(),
            1
        );
        // Final scheduled but not yet played → no resolved result.
        assert!(copa.final_result().is_none());
    }

    #[test]
    fn final_result_returns_winner_and_loser_after_final_recorded() {
        let mut copa = CopaLibertadores::new();
        copa.season_year = 2025;
        copa.schedule_final(&[7, 13]);
        assert!(matches!(copa.current_stage, CompetitionStage::Final));
        assert!(copa.final_result().is_none());

        // Record the final: 2-1 to the home finalist.
        copa.apply_match_result(&CompetitionStage::Final, 7, 13, 2, 1, None);
        assert_eq!(copa.final_result(), Some((7, 13)));
    }

    #[test]
    fn final_result_reads_shootout_winner_on_level_score() {
        let mut copa = CopaLibertadores::new();
        copa.schedule_final(&[7, 13]);
        // Level after extra time; the away finalist wins the shootout.
        copa.apply_match_result(&CompetitionStage::Final, 7, 13, 1, 1, Some((4, 5)));
        assert_eq!(copa.final_result(), Some((13, 7)));
    }

    #[test]
    fn undecided_tie_holds_the_next_round() {
        let mut copa = CopaLibertadores::new();
        copa.conduct_draw(
            &thirty_two_clubs(),
            &ContinentalRankings::new(),
            draw_date(),
        );
        copa.generate_knockout_fixtures();

        // Each leg is a 1-0 home win → aggregate level, no shootout → the
        // tie has no winner.
        let pairings: Vec<(u32, u32)> = copa
            .knockout_round
            .iter()
            .map(|t| (t.home_team, t.away_team))
            .collect();
        for (home, away) in pairings {
            copa.apply_match_result(&CompetitionStage::RoundOf16, home, away, 1, 0, None);
            copa.apply_match_result(&CompetitionStage::RoundOf16, away, home, 1, 0, None);
        }
        for m in copa.matches.iter_mut() {
            if matches!(m.stage, CompetitionStage::RoundOf16) {
                m.result.get_or_insert((1, 0));
            }
        }
        assert!(copa.knockout_round.iter().all(|t| t.winner.is_none()));

        copa.maybe_advance_knockout();
        // Held at R16: the QF draw must not fire while a tie is undecided.
        assert!(matches!(copa.current_stage, CompetitionStage::RoundOf16));
        assert!(
            !copa
                .matches
                .iter()
                .any(|m| matches!(m.stage, CompetitionStage::QuarterFinals))
        );
    }
}
