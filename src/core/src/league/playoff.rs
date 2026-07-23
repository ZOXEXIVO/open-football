//! End-of-season playoff for a grouped competition (MLS Cup, the Argentine
//! Torneo Apertura/Clausura finals, Serie C promotion playoff, …).
//!
//! A `LeaguePlayoff` crowns a champion across the several round-robin
//! groups that make up one competition. The regular season runs as N
//! independent group tables — exactly as before — and when the relevant
//! stage of every group is finished, the top `qualifiers_per_group` of
//! each group are seeded into a knockout bracket per the competition's
//! [`PlayoffFormat`]:
//!
//! * [`PlayoffFormat::SingleElimination`] — generic: group seeds are
//!   interleaved into one field and re-paired strongest-vs-weakest each
//!   round, byes to top seeds (the pre-2026 behaviour).
//! * [`PlayoffFormat::CrossGroupBracket`] — Argentine Primera: the zones
//!   cross immediately (1°A vs 8°B, 2°B vs 7°A, …) in a fixed bracket
//!   tree; the better seed hosts every round; the final is nominally
//!   neutral (modelled with the better seed as home side).
//! * [`PlayoffFormat::MlsCup`] — per-conference brackets: a single-game
//!   wild card (8 v 9), a best-of-3 round one (1 v WC, 4 v 5, 2 v 7,
//!   3 v 6), single-game conference semifinals and finals, then a single
//!   cross-conference final hosted by the better regular-season record.
//!   The best overall regular-season record also takes the Supporters'
//!   Shield, recorded on the playoff itself.
//!
//! Split-season competitions (Argentine Apertura/Clausura) get TWO
//! playoffs — one per tournament ([`PlayoffStage::FirstStage`] /
//! [`PlayoffStage::SecondStage`]), each triggered by its own tournament's
//! completion and crowning its own champion.
//!
//! Like [`crate::league::DomesticCup`], the playoff drives its fixtures
//! through an inner `League` (`is_cup = true`) so it reuses the match
//! engine, per-match stat/morale/discipline fan-out, slug indexing and the
//! web layer.

use crate::Club;
use crate::MatchRuntime;
use crate::context::GlobalContext;
use crate::league::core::PlayoffFormat;
use crate::league::schedule::cup;
use crate::league::{
    CupHistoryEntry, League, LeagueBuildOutput, LeagueMatch, LeaguePendingState, LeagueResult,
    LeagueTableResult, MatchStorage, Schedule, ScheduleItem, ScheduleTour,
};
use crate::r#match::{MatchResult, Score};
use chrono::{Datelike, Duration, NaiveDate, NaiveDateTime, NaiveTime};
use log::debug;
use std::collections::{HashMap, HashSet};

/// One team's line in a group standings snapshot — enough to seed a
/// bracket and settle "better regular-season record" questions (final
/// hosting, Supporters' Shield) without borrowing the league again.
#[derive(Debug, Clone)]
pub struct StandingRow {
    pub team_id: u32,
    pub points: u16,
    pub wins: u8,
    pub goal_difference: i32,
}

/// A snapshot of one group's current standings, handed to the playoff by
/// `Country` so the playoff never has to borrow the league collection
/// itself. `rows` is the live table, best-first; `complete` is true once
/// every fixture in the group has a result. Split-season groups also
/// carry their first tournament's completion flag and (frozen or live)
/// standings.
#[derive(Debug, Clone)]
pub struct GroupStanding {
    pub league_id: u32,
    pub complete: bool,
    pub rows: Vec<StandingRow>,
    /// Split seasons: every first-tournament fixture has a result.
    pub first_stage_complete: bool,
    /// Split seasons: the first tournament's standings (the frozen
    /// `split_first_table` once the flip has happened, the live table
    /// before that). Empty for non-split groups.
    pub first_stage_rows: Vec<StandingRow>,
}

/// Which slice of the season feeds this playoff.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlayoffStage {
    /// The whole regular season (MLS Cup, Serie C, …).
    FullSeason,
    /// The first tournament of a split season (Torneo Apertura).
    FirstStage,
    /// The second tournament of a split season (Torneo Clausura).
    SecondStage,
}

/// Human-meaningful stage of a playoff round, for the web layer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlayoffRoundLabel {
    WildCard,
    RoundOne,
    RoundOf16,
    QuarterFinal,
    SemiFinal,
    ConferenceSemiFinal,
    ConferenceFinal,
    Final,
}

/// Bracket index used for the cross-conference final in per-conference
/// formats (regular brackets are 0-based positions in `group_league_ids`).
pub const CROSS_BRACKET: usize = usize::MAX;

/// One knockout tie — a single game or a best-of-N series. Games are
/// materialised as `ScheduleItem`s on the playoff's inner league; the
/// series tracks how many each side has won so far.
#[derive(Debug, Clone)]
pub struct PlayoffSeries {
    /// 1-based round within the edition (wild card = 1 for MLS).
    pub round: u8,
    /// Which bracket the series belongs to: index into `group_league_ids`
    /// for per-conference formats, 0 for merged brackets, [`CROSS_BRACKET`]
    /// for the cross-conference final.
    pub bracket: usize,
    /// Position within the round. Fixed-tree rounds pair winners of slots
    /// `2s` and `2s+1` into next-round slot `s`.
    pub slot: usize,
    /// The better seed — hosts single games and games 1 & 3 of a series.
    pub home_team_id: u32,
    pub away_team_id: u32,
    pub best_of: u8,
    pub home_wins: u8,
    pub away_wins: u8,
    /// Final at a (nominally) neutral venue — display only; the better
    /// seed still occupies the schedule's home slot.
    pub neutral: bool,
    /// How many games have been placed on the schedule so far.
    pub games_scheduled: u8,
    /// Date of the last scheduled game — anchors the conditional game 3
    /// and the next round's timing.
    pub last_game_date: Option<NaiveDate>,
}

impl PlayoffSeries {
    pub fn winner(&self) -> Option<u32> {
        let needed = self.best_of / 2 + 1;
        if self.home_wins >= needed {
            Some(self.home_team_id)
        } else if self.away_wins >= needed {
            Some(self.away_team_id)
        } else {
            None
        }
    }

    fn involves(&self, a: u32, b: u32) -> bool {
        (self.home_team_id == a && self.away_team_id == b)
            || (self.home_team_id == b && self.away_team_id == a)
    }
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
    /// order. For per-conference formats each group runs its own bracket.
    pub group_league_ids: Vec<u32>,
    /// How many top teams from each group enter the bracket.
    pub qualifiers_per_group: u8,
    /// Bracket shape.
    pub format: PlayoffFormat,
    /// Which slice of the season this playoff concludes.
    pub stage: PlayoffStage,
    /// Calendar year the current edition anchors to.
    pub season_start_year: i32,
    /// All ties of the current edition, in draw order.
    pub series: Vec<PlayoffSeries>,
    /// Seed rank (1 = group winner) per qualified team, frozen at the
    /// draw. Ranks are within the team's own group.
    pub seed_rank: HashMap<u32, u32>,
    /// Bracket index per qualified team, frozen at the draw — resolves
    /// which conference a direct qualifier belongs to.
    pub bracket_of: HashMap<u32, usize>,
    /// Regular-season points per qualified team, frozen at the draw —
    /// settles cross-bracket hosting.
    pub regular_points: HashMap<u32, u16>,
    /// Supporters' Shield (best overall regular-season record) for the
    /// current edition — MLS-format playoffs only.
    pub shield_team_id: Option<u32>,
    /// Completed editions, oldest first — powers the History tab.
    pub past_champions: Vec<CupHistoryEntry>,
    /// Past Supporters' Shield winners, oldest first (MLS format only).
    pub shield_history: Vec<CupHistoryEntry>,
    /// Season-start year of the last edition whose winner-trophy event was
    /// emitted; paired with `award_emitted_winner_team_id` to fire the
    /// champion fan-out exactly once per edition.
    pub award_emitted_season_start_year: Option<i32>,
    pub award_emitted_winner_team_id: Option<u32>,
    pub award_emitted_on: Option<NaiveDate>,
    /// Season-start year of the last edition whose Supporters' Shield
    /// fan-out was emitted.
    pub shield_award_emitted_season_start_year: Option<i32>,
}

impl LeaguePlayoff {
    pub fn new(
        league: League,
        competition: String,
        group_league_ids: Vec<u32>,
        qualifiers_per_group: u8,
        format: PlayoffFormat,
        stage: PlayoffStage,
    ) -> Self {
        LeaguePlayoff {
            league,
            competition,
            group_league_ids,
            qualifiers_per_group,
            format,
            stage,
            season_start_year: 0,
            series: Vec::new(),
            seed_rank: HashMap::new(),
            bracket_of: HashMap::new(),
            regular_points: HashMap::new(),
            shield_team_id: None,
            past_champions: Vec::new(),
            shield_history: Vec::new(),
            award_emitted_season_start_year: None,
            award_emitted_winner_team_id: None,
            award_emitted_on: None,
            shield_award_emitted_season_start_year: None,
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

    /// The standings slice this playoff's stage reads from a group.
    fn stage_rows<'a>(&self, group: &'a GroupStanding) -> &'a [StandingRow] {
        match self.stage {
            PlayoffStage::FirstStage => &group.first_stage_rows,
            _ => &group.rows,
        }
    }

    fn stage_complete(&self, group: &GroupStanding) -> bool {
        match self.stage {
            PlayoffStage::FirstStage => group.first_stage_complete,
            _ => group.complete,
        }
    }

    /// Human label for a round, per format.
    pub fn round_label(&self, round: u8) -> PlayoffRoundLabel {
        match self.format {
            PlayoffFormat::MlsCup => match round {
                1 => PlayoffRoundLabel::WildCard,
                2 => PlayoffRoundLabel::RoundOne,
                3 => PlayoffRoundLabel::ConferenceSemiFinal,
                4 => PlayoffRoundLabel::ConferenceFinal,
                _ => PlayoffRoundLabel::Final,
            },
            _ => {
                // Merged brackets: label by how many ties the round holds.
                let ties = self.series.iter().filter(|s| s.round == round).count();
                match ties {
                    1 => PlayoffRoundLabel::Final,
                    2 => PlayoffRoundLabel::SemiFinal,
                    4 => PlayoffRoundLabel::QuarterFinal,
                    8 => PlayoffRoundLabel::RoundOf16,
                    _ => PlayoffRoundLabel::RoundOne,
                }
            }
        }
    }

    /// Archive the outgoing edition's champion (with runner-up) and shield
    /// winner before the bracket that proves it is discarded.
    fn record_history(&mut self) {
        if let Some(champion) = self.champion() {
            let runner_up = self.final_series().map(|s| {
                if s.home_team_id == champion {
                    s.away_team_id
                } else {
                    s.home_team_id
                }
            });
            self.past_champions.push(CupHistoryEntry {
                season_start_year: self.season_start_year,
                champion_team_id: champion,
                runner_up_team_id: runner_up,
            });
        }
        if let Some(shield) = self.shield_team_id {
            self.shield_history.push(CupHistoryEntry {
                season_start_year: self.season_start_year,
                champion_team_id: shield,
                runner_up_team_id: None,
            });
        }
    }

    /// Start a fresh edition: archive the old champion, wipe the bracket and
    /// seed maps, re-anchor the year and re-arm the award emits. Called on
    /// the competition's season-start day, well before the groups finish
    /// and the bracket is actually drawn.
    fn reset_edition(&mut self, ctx: &GlobalContext<'_>) {
        self.record_history();
        self.league.schedule = Schedule::new();
        self.league.matches = MatchStorage::new();
        self.series = Vec::new();
        self.seed_rank = HashMap::new();
        self.bracket_of = HashMap::new();
        self.regular_points = HashMap::new();
        self.shield_team_id = None;
        self.season_start_year = ctx.simulation.date.year();
        self.award_emitted_season_start_year = None;
        self.award_emitted_winner_team_id = None;
        self.award_emitted_on = None;
        self.shield_award_emitted_season_start_year = None;
    }

    /// Register the qualified slice of a group in the seed/points/bracket
    /// maps.
    fn register_qualifiers(&mut self, rows: &[StandingRow], bracket: usize) {
        let qpg = self.qualifiers_per_group as usize;
        for (rank, row) in rows.iter().take(qpg).enumerate() {
            self.seed_rank.insert(row.team_id, rank as u32 + 1);
            self.regular_points.insert(row.team_id, row.points);
            self.bracket_of.insert(row.team_id, bracket);
        }
    }

    /// Comparator seed: lower is better. Group rank first, regular-season
    /// points break ties between equal ranks of different groups.
    fn seed_key(&self, team_id: u32) -> (u32, i32) {
        let rank = self.seed_rank.get(&team_id).copied().unwrap_or(u32::MAX);
        let pts = self.regular_points.get(&team_id).copied().unwrap_or(0) as i32;
        (rank, -pts)
    }

    fn better_seed(&self, a: u32, b: u32) -> (u32, u32) {
        if self.seed_key(a) <= self.seed_key(b) {
            (a, b)
        } else {
            (b, a)
        }
    }

    fn push_series(
        &mut self,
        round: u8,
        bracket: usize,
        slot: usize,
        high: u32,
        low: u32,
        best_of: u8,
        neutral: bool,
        current_date: NaiveDate,
    ) {
        let mut series = PlayoffSeries {
            round,
            bracket,
            slot,
            home_team_id: high,
            away_team_id: low,
            best_of,
            home_wins: 0,
            away_wins: 0,
            neutral,
            games_scheduled: 0,
            last_game_date: None,
        };
        self.schedule_initial_games(&mut series, current_date);
        self.series.push(series);
    }

    /// Place the unconditional games of a fresh series on the calendar:
    /// the single game of a best-of-1, or games 1 & 2 of a best-of-3
    /// (game 1 at the better seed, game 2 at the worse seed three days
    /// later). Game 3 is added by `advance` only if the series splits.
    fn schedule_initial_games(&mut self, series: &mut PlayoffSeries, current_date: NaiveDate) {
        let base = cup::next_midweek(current_date + Duration::days(4));
        if series.best_of >= 3 {
            self.add_game(series, base, false);
            self.add_game(series, base + Duration::days(3), true);
        } else {
            self.add_game(series, base, false);
        }
    }

    /// Materialise one game of a series as a `ScheduleItem` in the tour
    /// matching the series round. `swap_venue` puts the worse seed at home
    /// (game 2 of a best-of-3).
    fn add_game(&mut self, series: &mut PlayoffSeries, date: NaiveDate, swap_venue: bool) {
        let (home, away) = if swap_venue {
            (series.away_team_id, series.home_team_id)
        } else {
            (series.home_team_id, series.away_team_id)
        };
        let dt = NaiveDateTime::new(date, NaiveTime::from_hms_opt(0, 0, 0).unwrap());
        let item = ScheduleItem::new(
            self.league.id,
            self.league.slug.clone(),
            home,
            away,
            dt,
            None,
        );
        let tour = match self
            .league
            .schedule
            .tours
            .iter_mut()
            .find(|t| t.num == series.round)
        {
            Some(t) => t,
            None => {
                self.league
                    .schedule
                    .tours
                    .push(ScheduleTour::new(series.round, 4));
                self.league.schedule.tours.last_mut().unwrap()
            }
        };
        tour.items.push(item);
        series.games_scheduled += 1;
        series.last_game_date = Some(date);
    }

    /// Draw the opening round(s) of the edition from the completed group
    /// standings. Format-specific.
    fn draw_bracket(&mut self, members: &[&GroupStanding], current_date: NaiveDate) {
        match self.format {
            PlayoffFormat::MlsCup => self.draw_mls(members, current_date),
            PlayoffFormat::CrossGroupBracket => self.draw_cross_group(members, current_date),
            PlayoffFormat::SingleElimination => self.draw_single_elimination(members, current_date),
        }
        if self.season_start_year == 0 {
            self.season_start_year = current_date.year();
        }
    }

    /// MLS: per-conference wild card (8 v 9). Round one waits for the wild
    /// card winner. Also settles the Supporters' Shield from the combined
    /// regular-season records.
    fn draw_mls(&mut self, members: &[&GroupStanding], current_date: NaiveDate) {
        // Supporters' Shield: best combined record (points, wins, GD).
        let shield = members
            .iter()
            .flat_map(|g| self.stage_rows(g).iter())
            .max_by_key(|r| (r.points, r.wins, r.goal_difference))
            .map(|r| r.team_id);
        self.shield_team_id = shield;

        let member_rows: Vec<Vec<StandingRow>> = members
            .iter()
            .map(|g| self.stage_rows(g).to_vec())
            .collect();
        for (bracket, rows) in member_rows.iter().enumerate() {
            self.register_qualifiers(rows, bracket);
            let q: Vec<u32> = rows
                .iter()
                .take(self.qualifiers_per_group as usize)
                .map(|r| r.team_id)
                .collect();
            if q.len() >= 9 {
                // Seeds 8 v 9 play the single-game wild card.
                self.push_series(1, bracket, 0, q[7], q[8], 1, false, current_date);
            } else if q.len() >= 2 {
                // Degraded field: run round one directly over what exists.
                self.draw_mls_round_one(bracket, &q, None, current_date);
            }
        }
    }

    /// MLS round one: best-of-3 series in fixed bracket order
    /// (1 v WC, 4 v 5, 2 v 7, 3 v 6) so the conference semifinals pair
    /// slot 0/1 and slot 2/3 winners.
    fn draw_mls_round_one(
        &mut self,
        bracket: usize,
        qualified: &[u32],
        wild_card_winner: Option<u32>,
        current_date: NaiveDate,
    ) {
        let opp_of_one = wild_card_winner.unwrap_or_else(|| qualified[qualified.len() - 1]);
        let pairs: Vec<(u32, u32)> = match qualified.len() {
            n if n >= 8 => vec![
                (qualified[0], opp_of_one),
                (qualified[3], qualified[4]),
                (qualified[1], qualified[6]),
                (qualified[2], qualified[5]),
            ],
            _ => {
                // Small fields: strongest-vs-weakest one round.
                let (p, _) = cup::pair_knockout_round(qualified);
                p
            }
        };
        for (slot, (high, low)) in pairs.into_iter().enumerate() {
            self.push_series(2, bracket, slot, high, low, 3, false, current_date);
        }
    }

    /// Argentine Primera: 2×`qualifiers_per_group` teams in one fixed
    /// bracket. Opening round crosses the zones — n°A vs (q+1−n)°B — and
    /// the lines are laid out in standard seeded bracket order so the top
    /// seeds cannot meet before the semifinals.
    fn draw_cross_group(&mut self, members: &[&GroupStanding], current_date: NaiveDate) {
        if members.len() != 2 {
            return self.draw_single_elimination(members, current_date);
        }
        let qpg = self.qualifiers_per_group as usize;
        let a: Vec<StandingRow> = self.stage_rows(members[0]).to_vec();
        let b: Vec<StandingRow> = self.stage_rows(members[1]).to_vec();
        if a.len() < qpg || b.len() < qpg || !qpg.is_power_of_two() {
            return self.draw_single_elimination(members, current_date);
        }
        // Merged bracket — everyone plays in bracket 0, but keep the zone
        // index in `bracket_of` for the record.
        self.register_qualifiers(&a, 0);
        self.register_qualifiers(&b, 1);

        // Line n (0-based): (n+1) of one zone vs (q−n) of the other —
        // every qualifier appears in exactly one line and all lines cross
        // the zones (1°A v 8°B, …, 8°A v 1°B for q = 8).
        let mut lines: Vec<(u32, u32)> = Vec::with_capacity(qpg);
        for n in 0..qpg {
            let (high, low) = self.better_seed(a[n].team_id, b[qpg - 1 - n].team_id);
            lines.push((high, low));
        }
        // Standard bracket layout over the lines' strength ranking (a
        // line's strength is its better seed), so adjacent-slot pairing
        // keeps the strongest lines apart until the semifinals.
        let mut by_strength: Vec<usize> = (0..lines.len()).collect();
        by_strength.sort_by_key(|&i| self.seed_key(lines[i].0));
        let positions = bracket_positions(lines.len());
        for (slot, &strength_rank) in positions.iter().enumerate() {
            let (high, low) = lines[by_strength[strength_rank]];
            self.push_series(1, 0, slot, high, low, 1, false, current_date);
        }
    }

    /// Generic: interleave group finishing positions (E1,W1,E2,W2,…) into
    /// one field, first round strongest-vs-weakest with byes to top seeds.
    fn draw_single_elimination(&mut self, members: &[&GroupStanding], current_date: NaiveDate) {
        let qpg = self.qualifiers_per_group as usize;
        let member_rows: Vec<Vec<StandingRow>> = members
            .iter()
            .map(|g| self.stage_rows(g).to_vec())
            .collect();
        let mut field: Vec<u32> = Vec::with_capacity(qpg * members.len());
        for rank in 0..qpg {
            for rows in &member_rows {
                if let Some(row) = rows.get(rank) {
                    field.push(row.team_id);
                }
            }
        }
        for (bracket, rows) in member_rows.iter().enumerate() {
            self.register_qualifiers(rows, bracket);
        }
        if field.len() < 2 {
            debug!(
                "🏆 Playoff {} has fewer than two entrants — no bracket this season",
                self.league.name
            );
            return;
        }
        let (pairings, _byes) = cup::pair_knockout_round(&field);
        for (slot, (high, low)) in pairings.into_iter().enumerate() {
            self.push_series(1, 0, slot, high, low, 1, false, current_date);
        }
    }

    /// Everything that moves the bracket forward once results are in:
    /// conditional game 3s, next rounds per bracket, the cross-conference
    /// final. Idempotent — safe to call every tick.
    fn advance(&mut self, current_date: NaiveDate) {
        if self.series.is_empty() {
            return;
        }

        // 1. Best-of-3 series split 1-1 with both games played → game 3.
        let pending_thirds: Vec<usize> = self
            .series
            .iter()
            .enumerate()
            .filter(|(_, s)| {
                s.best_of >= 3
                    && s.winner().is_none()
                    && s.games_scheduled == 2
                    && s.home_wins + s.away_wins == 2
            })
            .map(|(i, _)| i)
            .collect();
        for idx in pending_thirds {
            let mut series = self.series[idx].clone();
            let anchor = series.last_game_date.unwrap_or(current_date);
            let date = cup::next_midweek(anchor + Duration::days(2));
            self.add_game(&mut series, date, false);
            self.series[idx] = series;
        }

        // 2. Per-bracket round progression.
        let brackets: Vec<usize> = {
            let mut b: Vec<usize> = self
                .series
                .iter()
                .map(|s| s.bracket)
                .filter(|&b| b != CROSS_BRACKET)
                .collect();
            b.sort_unstable();
            b.dedup();
            b
        };
        for bracket in brackets {
            self.advance_bracket(bracket, current_date);
        }

        // 3. Cross-conference final for per-conference formats.
        if self.format == PlayoffFormat::MlsCup {
            self.maybe_draw_cross_final(current_date);
        }
    }

    /// Draw the next round inside one bracket once its top round has fully
    /// resolved and still holds more than one live team.
    fn advance_bracket(&mut self, bracket: usize, current_date: NaiveDate) {
        let top_round = self
            .series
            .iter()
            .filter(|s| s.bracket == bracket)
            .map(|s| s.round)
            .max()
            .unwrap_or(0);
        if top_round == 0 {
            return;
        }
        let mut top: Vec<&PlayoffSeries> = self
            .series
            .iter()
            .filter(|s| s.bracket == bracket && s.round == top_round)
            .collect();
        top.sort_by_key(|s| s.slot);
        if top.iter().any(|s| s.winner().is_none()) {
            return; // round still in progress
        }
        let winners: Vec<u32> = top.iter().filter_map(|s| s.winner()).collect();
        let latest = top
            .iter()
            .filter_map(|s| s.last_game_date)
            .max()
            .unwrap_or(current_date);
        let draw_anchor = latest.max(current_date);
        let next_round = top_round + 1;

        match self.format {
            PlayoffFormat::SingleElimination => {
                // Survivors = every seeded entrant that has not lost a tie
                // yet — this keeps bye teams (who never appeared in a
                // series) alive across rounds. Re-pair strongest-vs-
                // weakest; byes keep falling to the top seeds.
                let eliminated: HashSet<u32> = self
                    .series
                    .iter()
                    .filter(|s| s.bracket == bracket)
                    .filter_map(|s| {
                        s.winner().map(|w| {
                            if w == s.home_team_id {
                                s.away_team_id
                            } else {
                                s.home_team_id
                            }
                        })
                    })
                    .collect();
                let mut alive: Vec<u32> = self
                    .seed_rank
                    .keys()
                    .copied()
                    .filter(|t| !eliminated.contains(t))
                    .collect();
                if alive.len() < 2 {
                    return; // champion decided
                }
                alive.sort_by_key(|&t| self.seed_key(t));
                let (pairings, _byes) = cup::pair_knockout_round(&alive);
                for (slot, (h, l)) in pairings.into_iter().enumerate() {
                    self.push_series(next_round, bracket, slot, h, l, 1, false, draw_anchor);
                }
            }
            _ => {
                if winners.len() == 1 {
                    // Bracket resolved. MLS wild-card special case: the
                    // one-series wild-card round feeds round one instead.
                    if self.format == PlayoffFormat::MlsCup && top_round == 1 {
                        let qualified: Vec<u32> = self.bracket_qualifiers(bracket);
                        if qualified.len() >= 8 {
                            self.draw_mls_round_one(
                                bracket,
                                &qualified,
                                Some(winners[0]),
                                draw_anchor,
                            );
                        }
                    }
                    return;
                }
                // Fixed tree: adjacent slots meet; better seed hosts. The
                // merged-bracket final (2 winners left, cross-group format)
                // is flagged neutral.
                let is_final =
                    self.format == PlayoffFormat::CrossGroupBracket && winners.len() == 2;
                for slot in 0..winners.len() / 2 {
                    let (h, l) = self.better_seed(winners[slot * 2], winners[slot * 2 + 1]);
                    self.push_series(next_round, bracket, slot, h, l, 1, is_final, draw_anchor);
                }
            }
        }
    }

    /// The qualified teams of a bracket in seed order, from the membership
    /// map frozen at the draw.
    fn bracket_qualifiers(&self, bracket: usize) -> Vec<u32> {
        let mut teams: Vec<u32> = self
            .bracket_of
            .iter()
            .filter(|&(_, &b)| b == bracket)
            .map(|(&t, _)| t)
            .collect();
        teams.sort_by_key(|&t| self.seed_key(t));
        teams
    }

    /// The current edition's champion, if the bracket has resolved.
    pub fn champion(&self) -> Option<u32> {
        self.final_series().and_then(|s| s.winner())
    }

    /// The deciding series of the edition: the cross-bracket final for
    /// per-conference formats, otherwise the single series of the top
    /// round once the tree has narrowed to one tie AND it has resolved
    /// with no further round possible.
    pub fn final_series(&self) -> Option<&PlayoffSeries> {
        if self.format == PlayoffFormat::MlsCup {
            return self.series.iter().find(|s| s.bracket == CROSS_BRACKET);
        }
        let top_round = self.series.iter().map(|s| s.round).max()?;
        let top: Vec<&PlayoffSeries> = self
            .series
            .iter()
            .filter(|s| s.round == top_round)
            .collect();
        if top.len() == 1 && self.expected_final_round() == Some(top_round) {
            Some(top[0])
        } else {
            None
        }
    }

    /// Round number the final lands on for merged brackets, from the
    /// opening field size (ceil log2). `None` before the draw.
    fn expected_final_round(&self) -> Option<u8> {
        let first_round_series = self.series.iter().filter(|s| s.round == 1).count();
        if first_round_series == 0 {
            return None;
        }
        // Opening round of a merged bracket reduces the field to a power
        // of two; each subsequent round halves it.
        let entrants: usize = self.seed_rank.len().max(first_round_series * 2);
        Some(cup::total_rounds(entrants))
    }

    /// Draw the cross-conference final once every conference bracket has
    /// resolved. Hosted by the better regular-season record.
    fn maybe_draw_cross_final(&mut self, current_date: NaiveDate) {
        if self.series.iter().any(|s| s.bracket == CROSS_BRACKET) {
            return;
        }
        let mut finalists: Vec<(u32, NaiveDate)> = Vec::new();
        for bracket in 0..self.group_league_ids.len() {
            let top_round = self
                .series
                .iter()
                .filter(|s| s.bracket == bracket)
                .map(|s| s.round)
                .max()
                .unwrap_or(0);
            // A conference bracket resolves at the conference final —
            // round 4 in the full shape (WC → R1 → semi → final), or
            // whatever single-series round the degraded shape ends on.
            let top: Vec<&PlayoffSeries> = self
                .series
                .iter()
                .filter(|s| s.bracket == bracket && s.round == top_round)
                .collect();
            if top.len() != 1 || top_round < 4 {
                return;
            }
            match (top[0].winner(), top[0].last_game_date) {
                (Some(w), Some(d)) => finalists.push((w, d)),
                _ => return,
            }
        }
        if finalists.len() != 2 {
            return;
        }
        let (a, b) = (finalists[0].0, finalists[1].0);
        let pa = self.regular_points.get(&a).copied().unwrap_or(0);
        let pb = self.regular_points.get(&b).copied().unwrap_or(0);
        let (high, low) = if pa >= pb { (a, b) } else { (b, a) };
        let anchor = finalists
            .iter()
            .map(|(_, d)| *d)
            .max()
            .unwrap_or(current_date)
            .max(current_date);
        self.push_series(5, CROSS_BRACKET, 0, high, low, 1, false, anchor);
    }

    /// Today's unplayed playoff ties, tagged with their bracket position
    /// so the match builder can scale importance by stage. The round total
    /// is the format's EXPECTED depth (not the rounds drawn so far), so an
    /// early wild-card tie doesn't carry final-level importance.
    fn collect_today_matches(&self, current_date: NaiveDate) -> Vec<LeagueMatch> {
        let drawn = self.series.iter().map(|s| s.round).max().unwrap_or(1);
        let total_rounds = match self.format {
            PlayoffFormat::MlsCup => 5,
            _ => self.expected_final_round().unwrap_or(drawn).max(drawn),
        };
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

    /// Fold one played game back into its series' win tally.
    fn apply_game_result(&mut self, result: &MatchResult) {
        let Some(winner) = game_winner(&result.score, result.home_team_id, result.away_team_id)
        else {
            return;
        };
        let (a, b) = (result.home_team_id, result.away_team_id);
        if let Some(series) = self
            .series
            .iter_mut()
            .find(|s| s.winner().is_none() && s.involves(a, b))
        {
            if winner == series.home_team_id {
                series.home_wins += 1;
            } else if winner == series.away_team_id {
                series.away_wins += 1;
            }
        }
    }

    /// Build (but do not play) today's playoff matches. Mirrors
    /// [`League::simulate_build`] for the knockout side: resets at the
    /// competition's season start, draws the bracket once the relevant
    /// stage of every member group has finished, then collects today's
    /// ties for a batched engine dispatch.
    pub fn simulate_build(
        &mut self,
        clubs: &[Club],
        groups: &[GroupStanding],
        ctx: &GlobalContext<'_>,
    ) -> LeagueBuildOutput {
        let current_date = ctx.simulation.date.date();

        if self
            .league
            .settings
            .is_time_for_new_schedule(&ctx.simulation)
        {
            self.reset_edition(ctx);
        }

        // Draw the bracket exactly once, when every member group has played
        // out the stage this playoff concludes.
        if self.series.is_empty() {
            let members = self.member_standings(groups);
            let ready = members.len() >= 2 && members.iter().all(|g| self.stage_complete(g));
            if ready {
                self.draw_bracket(&members, current_date);
                debug!(
                    "🏆 Playoff {} drawn: {} opening series",
                    self.league.name,
                    self.series.len()
                );
            }
        }

        let scheduled = self.collect_today_matches(current_date);
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

    /// Apply played playoff results back onto the bracket and progress it
    /// (game 3s, next rounds, the final) if today closed something out.
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
            self.apply_game_result(mr);
        }

        self.advance(current_date);

        LeagueResult::with_match_result(self.league.id, LeagueTableResult {}, match_results)
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

    /// Shield winner id only if its fan-out is still owed this edition.
    pub fn should_emit_shield_award(&self) -> Option<u32> {
        let team_id = self.shield_team_id?;
        if self.shield_award_emitted_season_start_year == Some(self.season_start_year) {
            return None;
        }
        Some(team_id)
    }

    pub fn mark_shield_award_emitted(&mut self) {
        self.shield_award_emitted_season_start_year = Some(self.season_start_year);
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

/// Winner of one played game, penalty-aware, resolved by team id so it is
/// immune to home/away slot swaps between the schedule item and the stored
/// `Score`. `None` only for a truly level score with no shootout (which a
/// knockout game cannot produce).
fn game_winner(score: &Score, home_team_id: u32, away_team_id: u32) -> Option<u32> {
    let (hg, ag) = if score.home_team.team_id == home_team_id {
        (score.home_team.get(), score.away_team.get())
    } else {
        (score.away_team.get(), score.home_team.get())
    };
    if hg != ag {
        return Some(if hg > ag { home_team_id } else { away_team_id });
    }
    let (hs, as_) = if score.home_team.team_id == home_team_id {
        (score.home_shootout, score.away_shootout)
    } else {
        (score.away_shootout, score.home_shootout)
    };
    match hs.cmp(&as_) {
        std::cmp::Ordering::Greater => Some(home_team_id),
        std::cmp::Ordering::Less => Some(away_team_id),
        // A knockout game can't truly end level (extra time + penalties
        // decide); the host is an arbitrary but deterministic guard so a
        // degenerate result (e.g. the 0-0 match stub) can't stall the
        // bracket — mirrors `cup::tie_winner`.
        std::cmp::Ordering::Equal => Some(home_team_id),
    }
}

/// Standard seeded bracket position order for `n` lines (n a power of
/// two): position i holds line `order[i]`, so slot-adjacent pairing puts
/// line 0 against the weakest line and keeps the strongest lines apart
/// until the last rounds. E.g. n=8 → [0, 7, 3, 4, 1, 6, 2, 5].
fn bracket_positions(n: usize) -> Vec<usize> {
    let mut order = vec![0usize];
    let mut size = 1;
    while size < n {
        size *= 2;
        let mut next = Vec::with_capacity(size);
        for &s in &order {
            next.push(s);
            next.push(size - 1 - s);
        }
        order = next;
    }
    order
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::league::{DayMonthPeriod, LeagueGroup, LeagueSettings};
    use crate::r#match::TeamScore;

    fn settings() -> LeagueSettings {
        LeagueSettings {
            season_starting_half: DayMonthPeriod::new(1, 3, 30, 6),
            season_ending_half: DayMonthPeriod::new(1, 7, 10, 12),
            tier: 0,
            promotion_spots: 0,
            relegation_spots: 0,
            league_group: Some(LeagueGroup {
                name: "Playoff".into(),
                competition: "Test League".into(),
                total_groups: 2,
                playoff: None,
            }),
            split_season: false,
        }
    }

    fn playoff(format: PlayoffFormat, qualifiers: u8) -> LeaguePlayoff {
        let league = League::new(
            900_001,
            "Test Playoff".into(),
            "test-playoff".into(),
            1,
            5000,
            settings(),
            false,
        );
        LeaguePlayoff::new(
            league,
            "Test League".into(),
            vec![10, 20],
            qualifiers,
            format,
            PlayoffStage::FullSeason,
        )
    }

    /// Standing rows for one group: `base+1 .. base+n`, points strictly
    /// descending so seed order matches id order within the group.
    fn rows(base: u32, n: u32, top_points: u16) -> Vec<StandingRow> {
        (0..n)
            .map(|i| StandingRow {
                team_id: base + i + 1,
                points: top_points - i as u16,
                wins: 0,
                goal_difference: 0,
            })
            .collect()
    }

    fn group(id: u32, rows: Vec<StandingRow>) -> GroupStanding {
        GroupStanding {
            league_id: id,
            complete: true,
            rows,
            first_stage_complete: false,
            first_stage_rows: Vec::new(),
        }
    }

    fn score(winner: u32, loser: u32, home: u32, away: u32) -> Score {
        let (hg, ag) = if home == winner {
            (1u8, 0u8)
        } else {
            (0u8, 1u8)
        };
        let _ = loser;
        Score {
            home_team: TeamScore::new_with_score(home, hg),
            away_team: TeamScore::new_with_score(away, ag),
            details: Vec::new(),
            home_shootout: 0,
            away_shootout: 0,
        }
    }

    /// Play every unplayed scheduled game with `winner_of` deciding each
    /// tie, advancing the bracket after each pass, until nothing new is
    /// scheduled. Returns the number of games played.
    fn play_out(pf: &mut LeaguePlayoff, winner_of: impl Fn(u32, u32) -> u32) -> usize {
        let mut played = 0usize;
        let mut day = NaiveDate::from_ymd_opt(2026, 10, 1).unwrap();
        loop {
            let pending: Vec<(usize, usize, u32, u32, String)> = pf
                .league
                .schedule
                .tours
                .iter()
                .enumerate()
                .flat_map(|(ti, t)| {
                    t.items
                        .iter()
                        .enumerate()
                        .filter(|(_, i)| i.result.is_none())
                        .map(move |(ii, i)| (ti, ii, i.home_team_id, i.away_team_id, i.id.clone()))
                })
                .collect();
            if pending.is_empty() {
                break;
            }
            for (ti, ii, home, away, id) in pending {
                let w = winner_of(home, away);
                let l = if w == home { away } else { home };
                let s = score(w, l, home, away);
                pf.league.schedule.tours[ti].items[ii].result = Some(s.clone());
                let mr = MatchResult {
                    league_id: pf.league.id,
                    id,
                    league_slug: pf.league.slug.clone(),
                    home_team_id: home,
                    away_team_id: away,
                    score: s,
                    details: None,
                    friendly: false,
                };
                pf.apply_game_result(&mr);
                played += 1;
            }
            day += Duration::days(7);
            pf.advance(day);
        }
        played
    }

    #[test]
    fn bracket_positions_are_standard_seeded_order() {
        assert_eq!(bracket_positions(1), vec![0]);
        assert_eq!(bracket_positions(2), vec![0, 1]);
        assert_eq!(bracket_positions(4), vec![0, 3, 1, 2]);
        assert_eq!(bracket_positions(8), vec![0, 7, 3, 4, 1, 6, 2, 5]);
    }

    #[test]
    fn cross_group_draw_crosses_zones_immediately() {
        let mut pf = playoff(PlayoffFormat::CrossGroupBracket, 8);
        pf.season_start_year = 2026;
        let members_owned = vec![group(10, rows(0, 15, 50)), group(20, rows(100, 15, 45))];
        let members: Vec<&GroupStanding> = members_owned.iter().collect();
        let today = NaiveDate::from_ymd_opt(2026, 5, 20).unwrap();
        pf.draw_bracket(&members, today);

        let r1: Vec<&PlayoffSeries> = pf.series.iter().filter(|s| s.round == 1).collect();
        assert_eq!(r1.len(), 8, "16 qualifiers → 8 round-of-16 ties");

        // Every tie crosses the zones: n°A vs (9−n)°B, better seed home.
        let mut pairs: Vec<(u32, u32)> = r1
            .iter()
            .map(|s| (s.home_team_id, s.away_team_id))
            .collect();
        pairs.sort_unstable();
        assert_eq!(
            pairs,
            vec![
                (1, 108),
                (2, 107),
                (3, 106),
                (4, 105),
                (101, 8),
                (102, 7),
                (103, 6),
                (104, 5),
            ]
        );

        // All single games — no best-of-3 in the Argentine format.
        assert!(r1.iter().all(|s| s.best_of == 1));
    }

    #[test]
    fn cross_group_bracket_resolves_with_neutral_final() {
        let mut pf = playoff(PlayoffFormat::CrossGroupBracket, 8);
        pf.season_start_year = 2026;
        let members_owned = vec![group(10, rows(0, 15, 50)), group(20, rows(100, 15, 45))];
        let members: Vec<&GroupStanding> = members_owned.iter().collect();
        pf.draw_bracket(&members, NaiveDate::from_ymd_opt(2026, 5, 20).unwrap());

        // Lower id wins everything → zone A's top seed takes the title.
        let games = play_out(&mut pf, |a, b| a.min(b));
        // 8 + 4 + 2 + 1 rounds of single games.
        assert_eq!(games, 15);
        assert_eq!(pf.champion(), Some(1));

        let final_series = pf.final_series().expect("final resolved");
        assert_eq!(final_series.round, 4);
        assert!(
            final_series.neutral,
            "Argentine final is at a neutral venue"
        );
    }

    #[test]
    fn mls_bracket_wild_card_feeds_best_of_three_round_one() {
        let mut pf = playoff(PlayoffFormat::MlsCup, 9);
        pf.season_start_year = 2026;
        // East (group 10): ids 1..15, best regular season record overall.
        // West (group 20): ids 101..115.
        let members_owned = vec![group(10, rows(0, 15, 70)), group(20, rows(100, 15, 60))];
        let members: Vec<&GroupStanding> = members_owned.iter().collect();
        let today = NaiveDate::from_ymd_opt(2026, 9, 20).unwrap();
        pf.draw_bracket(&members, today);

        // Supporters' Shield: best combined record = East seed 1.
        assert_eq!(pf.shield_team_id, Some(1));

        // One wild card per conference: 8 v 9, single game.
        let wc: Vec<&PlayoffSeries> = pf.series.iter().filter(|s| s.round == 1).collect();
        assert_eq!(wc.len(), 2);
        let mut wc_pairs: Vec<(u32, u32)> = wc
            .iter()
            .map(|s| (s.home_team_id, s.away_team_id))
            .collect();
        wc_pairs.sort_unstable();
        assert_eq!(wc_pairs, vec![(8, 9), (108, 109)]);
        assert!(wc.iter().all(|s| s.best_of == 1));

        // Higher seed wins every game.
        let games = play_out(&mut pf, |a, b| a.min(b));

        // Round one exists per conference: 1vWC(8), 4v5, 2v7, 3v6 — Bo3.
        let r2: Vec<&PlayoffSeries> = pf
            .series
            .iter()
            .filter(|s| s.round == 2 && s.bracket == 0)
            .collect();
        assert_eq!(r2.len(), 4);
        let mut r2_pairs: Vec<(u32, u32)> = r2
            .iter()
            .map(|s| (s.home_team_id, s.away_team_id))
            .collect();
        r2_pairs.sort_unstable();
        assert_eq!(r2_pairs, vec![(1, 8), (2, 7), (3, 6), (4, 5)]);
        assert!(r2.iter().all(|s| s.best_of == 3));

        // Bo3 swept 2-0 → exactly 2 games each, no game 3.
        assert!(
            r2.iter()
                .all(|s| s.home_wins + s.away_wins == 2 && s.games_scheduled == 2)
        );

        // Conference finals resolve, then the cross-conference MLS Cup:
        // hosted by the better regular-season record (East seed 1).
        let final_series = pf.final_series().expect("MLS Cup drawn");
        assert_eq!(final_series.bracket, CROSS_BRACKET);
        assert_eq!(final_series.home_team_id, 1);
        assert_eq!(final_series.away_team_id, 101);
        assert_eq!(pf.champion(), Some(1));

        // Total: 2 WC + 8×2 Bo3 sweeps + 4 semis + 2 conf finals + 1 final.
        assert_eq!(games, 2 + 16 + 4 + 2 + 1);
    }

    #[test]
    fn best_of_three_split_schedules_a_third_game() {
        let mut pf = playoff(PlayoffFormat::MlsCup, 9);
        pf.season_start_year = 2026;
        let members_owned = vec![group(10, rows(0, 15, 70)), group(20, rows(100, 15, 60))];
        let members: Vec<&GroupStanding> = members_owned.iter().collect();
        pf.draw_bracket(&members, NaiveDate::from_ymd_opt(2026, 9, 20).unwrap());

        // Whoever hosts wins each game. In a Bo3 that splits the series
        // 1-1 (game 1 at the better seed, game 2 at the worse seed) and
        // forces game 3 — back at the better seed, who takes the tie.
        // Single games are hosted by the better seed, so seeding holds.
        play_out(&mut pf, |home, _away| home);

        let bo3: Vec<&PlayoffSeries> = pf.series.iter().filter(|s| s.best_of == 3).collect();
        assert!(!bo3.is_empty());
        for s in &bo3 {
            assert_eq!(s.winner(), Some(s.home_team_id), "better seed takes game 3");
            assert_eq!(
                (s.home_wins, s.away_wins),
                (2, 1),
                "series went the distance"
            );
            assert_eq!(
                s.games_scheduled, 3,
                "game 3 was scheduled on the 1-1 split"
            );
        }
        assert!(pf.champion().is_some(), "bracket resolves");
    }

    #[test]
    fn single_elimination_keeps_bye_teams_alive() {
        // 2 groups × 3 qualifiers = 6 entrants → round one has 2 ties and
        // 2 byes; the byes must reappear in round two.
        let mut pf = playoff(PlayoffFormat::SingleElimination, 3);
        pf.season_start_year = 2026;
        let members_owned = vec![group(10, rows(0, 5, 50)), group(20, rows(100, 5, 45))];
        let members: Vec<&GroupStanding> = members_owned.iter().collect();
        pf.draw_bracket(&members, NaiveDate::from_ymd_opt(2026, 11, 1).unwrap());

        let r1_count = pf.series.iter().filter(|s| s.round == 1).count();
        assert_eq!(r1_count, 2, "6 entrants → 2 opening ties, 2 byes");

        play_out(&mut pf, |a, b| a.min(b));
        assert_eq!(pf.champion(), Some(1));
    }
}
