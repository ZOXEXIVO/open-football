use super::{CompetitionStage, CompetitionTier, ContinentalMatch, ContinentalMatchResult, CHAMPIONS_LEAGUE_ID};
use crate::continent::ContinentalRankings;
use crate::r#match::{Match, MatchResult, MatchSquad, SelectionContext};
use crate::Club;
use chrono::{Datelike, NaiveDate};
use log::{debug, info};
use std::collections::HashMap;

pub const CHAMPIONS_LEAGUE_SLUG: &str = "champions-league";

// ─── Group stage table ──────────────────────────────────────────────

#[derive(Debug, Clone, Default)]
pub struct ClGroupTable {
    pub rows: Vec<ClGroupRow>,
}

#[derive(Debug, Clone)]
pub struct ClGroupRow {
    pub team_id: u32,
    pub played: u8,
    pub won: u8,
    pub drawn: u8,
    pub lost: u8,
    pub gf: u8,
    pub ga: u8,
    pub points: u8,
}

impl ClGroupTable {
    pub fn new(teams: &[u32]) -> Self {
        ClGroupTable {
            rows: teams.iter().map(|&id| ClGroupRow {
                team_id: id, played: 0, won: 0, drawn: 0, lost: 0,
                gf: 0, ga: 0, points: 0,
            }).collect(),
        }
    }

    pub fn update(&mut self, home_id: u32, away_id: u32, home_goals: u8, away_goals: u8) {
        use std::cmp::Ordering;
        match home_goals.cmp(&away_goals) {
            Ordering::Greater => {
                self.record(home_id, home_goals, away_goals, 3, true, false, false);
                self.record(away_id, away_goals, home_goals, 0, false, false, true);
            }
            Ordering::Less => {
                self.record(home_id, home_goals, away_goals, 0, false, false, true);
                self.record(away_id, away_goals, home_goals, 3, true, false, false);
            }
            Ordering::Equal => {
                self.record(home_id, home_goals, away_goals, 1, false, true, false);
                self.record(away_id, away_goals, home_goals, 1, false, true, false);
            }
        }
        self.sort();
    }

    fn record(&mut self, team_id: u32, gf: u8, ga: u8, pts: u8, won: bool, drawn: bool, lost: bool) {
        if let Some(row) = self.rows.iter_mut().find(|r| r.team_id == team_id) {
            row.played += 1;
            row.gf += gf;
            row.ga += ga;
            row.points += pts;
            if won { row.won += 1; }
            if drawn { row.drawn += 1; }
            if lost { row.lost += 1; }
        }
    }

    fn sort(&mut self) {
        self.rows.sort_by(|a, b| {
            b.points.cmp(&a.points)
                .then_with(|| (b.gf as i16 - b.ga as i16).cmp(&(a.gf as i16 - a.ga as i16)))
                .then_with(|| b.gf.cmp(&a.gf))
        });
    }

    /// Top 2 teams qualify for knockout
    pub fn qualifiers(&self) -> (u32, u32) {
        (self.rows[0].team_id, self.rows[1].team_id)
    }
}

// ─── Knockout tie ───────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct KnockoutTie {
    pub home_team: u32,
    pub away_team: u32,
    pub leg1_score: Option<(u8, u8)>,
    pub leg2_score: Option<(u8, u8)>,
    pub winner: Option<u32>,
}

impl KnockoutTie {
    pub fn new(home: u32, away: u32) -> Self {
        KnockoutTie { home_team: home, away_team: away, leg1_score: None, leg2_score: None, winner: None }
    }

    pub fn record_leg1(&mut self, home_goals: u8, away_goals: u8) {
        self.leg1_score = Some((home_goals, away_goals));
    }

    pub fn record_leg2(&mut self, home_goals: u8, away_goals: u8) {
        self.leg2_score = Some((home_goals, away_goals));
        // Determine winner on aggregate
        if let (Some((h1, a1)), Some((h2, a2))) = (self.leg1_score, self.leg2_score) {
            let agg_home = h1 as u16 + a2 as u16; // home team: scored h1 at home + a2 away
            let agg_away = a1 as u16 + h2 as u16;
            self.winner = Some(if agg_home > agg_away {
                self.home_team
            } else if agg_away > agg_home {
                self.away_team
            } else {
                // Away goals rule removed in modern CL; use coin flip
                if h2 > a1 { self.away_team } else { self.home_team }
            });
        }
    }
}

// ─── Main Champions League struct ───────────────────────────────────

#[derive(Debug, Clone)]
pub struct ChampionsLeague {
    pub participating_clubs: Vec<u32>,
    pub current_stage: CompetitionStage,
    pub groups: Vec<ClGroupTable>,
    pub knockout_round: Vec<KnockoutTie>,
    pub matches: Vec<ContinentalMatch>,
    pub prize_pool: f64,
    pub season_year: u16,
}

impl Default for ChampionsLeague {
    fn default() -> Self {
        Self::new()
    }
}

impl ChampionsLeague {
    pub fn new() -> Self {
        ChampionsLeague {
            participating_clubs: Vec::new(),
            current_stage: CompetitionStage::NotStarted,
            groups: Vec::new(),
            knockout_round: Vec::new(),
            matches: Vec::new(),
            prize_pool: 2_000_000_000.0,
            season_year: 0,
        }
    }

    /// Conduct draw: seed clubs into 8 groups of 4, generate fixtures.
    pub fn conduct_draw(&mut self, clubs: &[u32], _rankings: &ContinentalRankings, date: NaiveDate) {
        if clubs.len() < 8 {
            debug!("Champions League: not enough clubs ({}) for draw", clubs.len());
            return;
        }

        // Take up to 32 clubs (8 groups × 4)
        let count = clubs.len().min(32);
        self.participating_clubs = clubs[..count].to_vec();
        self.season_year = date.year() as u16;

        // Create groups: distribute clubs round-robin into 8 groups
        let num_groups = (count / 4).max(1).min(8);
        self.groups = (0..num_groups)
            .map(|g| {
                let team_ids: Vec<u32> = (0..4)
                    .filter_map(|i| {
                        let idx = g + i * num_groups;
                        self.participating_clubs.get(idx).copied()
                    })
                    .collect();
                ClGroupTable::new(&team_ids)
            })
            .collect();

        // Generate group stage fixtures (6 matchdays)
        self.matches.clear();
        let year = date.year();
        let matchday_dates = [
            NaiveDate::from_ymd_opt(year, 9, 17).unwrap(),  // MD1
            NaiveDate::from_ymd_opt(year, 10, 1).unwrap(),  // MD2
            NaiveDate::from_ymd_opt(year, 10, 22).unwrap(), // MD3
            NaiveDate::from_ymd_opt(year, 11, 5).unwrap(),  // MD4
            NaiveDate::from_ymd_opt(year, 11, 26).unwrap(), // MD5
            NaiveDate::from_ymd_opt(year, 12, 10).unwrap(), // MD6
        ];

        for group in &self.groups {
            let teams: Vec<u32> = group.rows.iter().map(|r| r.team_id).collect();
            if teams.len() < 4 { continue; }

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
                    date: matchday_dates[md],
                    stage: CompetitionStage::GroupStage,
                });
                self.matches.push(ContinentalMatch {
                    home_team: teams[h2],
                    away_team: teams[a2],
                    date: matchday_dates[md],
                    stage: CompetitionStage::GroupStage,
                });
            }
        }

        self.current_stage = CompetitionStage::GroupStage;

        info!(
            "Champions League draw: {} clubs in {} groups, {} fixtures",
            count, num_groups, self.matches.len()
        );
    }

    /// Generate knockout round fixtures after group stage completes.
    pub fn generate_knockout_fixtures(&mut self, year: i32) {
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
        for i in 0..num_ties {
            // Simple draw: winner i vs runner-up (i+1) % n
            let r_idx = (i + 1) % runners_up.len();
            self.knockout_round.push(KnockoutTie::new(winners[i], runners_up[r_idx]));
        }

        // Schedule R16 matches
        let r16_dates_leg1 = [
            NaiveDate::from_ymd_opt(year + 1, 2, 18).unwrap(),
            NaiveDate::from_ymd_opt(year + 1, 2, 19).unwrap(),
            NaiveDate::from_ymd_opt(year + 1, 2, 25).unwrap(),
            NaiveDate::from_ymd_opt(year + 1, 2, 26).unwrap(),
        ];
        let r16_dates_leg2 = [
            NaiveDate::from_ymd_opt(year + 1, 3, 11).unwrap(),
            NaiveDate::from_ymd_opt(year + 1, 3, 12).unwrap(),
            NaiveDate::from_ymd_opt(year + 1, 3, 18).unwrap(),
            NaiveDate::from_ymd_opt(year + 1, 3, 19).unwrap(),
        ];

        for (i, tie) in self.knockout_round.iter().enumerate() {
            let leg1_date = r16_dates_leg1[i % r16_dates_leg1.len()];
            let leg2_date = r16_dates_leg2[i % r16_dates_leg2.len()];

            self.matches.push(ContinentalMatch {
                home_team: tie.home_team,
                away_team: tie.away_team,
                date: leg1_date,
                stage: CompetitionStage::RoundOf16,
            });
            self.matches.push(ContinentalMatch {
                home_team: tie.away_team,
                away_team: tie.home_team,
                date: leg2_date,
                stage: CompetitionStage::RoundOf16,
            });
        }

        self.current_stage = CompetitionStage::RoundOf16;

        info!(
            "Champions League R16: {} ties, {} matches scheduled",
            self.knockout_round.len(),
            self.knockout_round.len() * 2
        );
    }

    pub fn has_matches_today(&self, date: NaiveDate) -> bool {
        self.matches.iter().any(|m| m.date == date)
    }

    /// Play today's matches using the real match engine.
    /// Returns MatchResults that flow through the standard stat pipeline.
    pub fn play_matches(
        &mut self,
        clubs: &HashMap<u32, &Club>,
        date: NaiveDate,
    ) -> Vec<MatchResult> {
        let todays_matches: Vec<ContinentalMatch> = self.matches
            .iter()
            .filter(|m| m.date == date)
            .cloned()
            .collect();

        if todays_matches.is_empty() {
            return Vec::new();
        }

        // Build Match objects (same pattern as League::build_match)
        let selection_ctx = SelectionContext {
            is_friendly: false,
            date,
        };

        let engine_matches: Vec<Match> = todays_matches
            .iter()
            .filter_map(|cm| {
                let home_club = clubs.get(&cm.home_team)?;
                let away_club = clubs.get(&cm.away_team)?;

                let home_team = home_club.teams.teams.first()?;
                let away_team = away_club.teams.teams.first()?;

                let home_squad = home_team.get_enhanced_match_squad(&[], &selection_ctx);
                let away_squad = away_team.get_enhanced_match_squad(&[], &selection_ctx);

                let match_id = format!("cl_{}_{}_{}",
                    date.format("%Y%m%d"), cm.home_team, cm.away_team);

                Some(Match::make(
                    match_id,
                    CHAMPIONS_LEAGUE_ID,
                    CHAMPIONS_LEAGUE_SLUG,
                    home_squad,
                    away_squad,
                    false, // not friendly — competitive
                ))
            })
            .collect();

        if engine_matches.is_empty() {
            return Vec::new();
        }

        // Play all matches through the engine pool
        let results = crate::match_engine_pool().play(engine_matches);

        // Update group tables / knockout ties from results
        for (cm, result) in todays_matches.iter().zip(results.iter()) {
            let home_goals = result.score.home_team.get();
            let away_goals = result.score.away_team.get();

            match cm.stage {
                CompetitionStage::GroupStage => {
                    // Find and update the group containing these teams
                    for group in &mut self.groups {
                        let has_home = group.rows.iter().any(|r| r.team_id == cm.home_team);
                        let has_away = group.rows.iter().any(|r| r.team_id == cm.away_team);
                        if has_home && has_away {
                            group.update(cm.home_team, cm.away_team, home_goals, away_goals);
                            break;
                        }
                    }
                }
                CompetitionStage::RoundOf16 | CompetitionStage::QuarterFinals
                | CompetitionStage::SemiFinals => {
                    for tie in &mut self.knockout_round {
                        if tie.home_team == cm.home_team && tie.away_team == cm.away_team {
                            if tie.leg1_score.is_none() {
                                tie.record_leg1(home_goals, away_goals);
                            }
                        } else if tie.home_team == cm.away_team && tie.away_team == cm.home_team {
                            if tie.leg2_score.is_none() {
                                tie.record_leg2(home_goals, away_goals);
                            }
                        }
                    }
                }
                _ => {}
            }

            debug!(
                "CL: {} {} - {} {} ({:?})",
                cm.home_team, home_goals, away_goals, cm.away_team, cm.stage
            );
        }

        // Check if group stage is complete (all group matches played)
        let group_stage_complete = matches!(self.current_stage, CompetitionStage::GroupStage)
            && self.groups.iter().all(|g| g.rows.iter().all(|r| r.played >= 6));

        if group_stage_complete {
            info!("Champions League group stage complete — generating R16 draw");
            self.generate_knockout_fixtures(date.year());
        }

        results
    }

    pub fn simulate_round(
        &mut self,
        clubs: &HashMap<u32, &Club>,
        date: NaiveDate,
    ) -> Vec<ContinentalMatchResult> {
        // Play real matches and convert to ContinentalMatchResult for financial processing
        let match_results = self.play_matches(clubs, date);

        match_results.iter().map(|r| {
            ContinentalMatchResult {
                home_team: r.home_team_id,
                away_team: r.away_team_id,
                home_score: r.score.home_team.get(),
                away_score: r.score.away_team.get(),
                competition: CompetitionTier::ChampionsLeague,
            }
        }).collect()
    }

    pub fn get_club_points(&self, club_id: u32) -> f32 {
        if !self.participating_clubs.contains(&club_id) {
            return 0.0;
        }

        // Points from group stage performance
        for group in &self.groups {
            if let Some(row) = group.rows.iter().find(|r| r.team_id == club_id) {
                return 10.0 + row.points as f32 * 2.0;
            }
        }

        10.0
    }

    /// Get the MatchResults from today's matches for stat processing.
    /// Called separately from simulate_round to feed into LeagueResult pipeline.
    pub fn take_match_results(
        &mut self,
        clubs: &HashMap<u32, &Club>,
        date: NaiveDate,
    ) -> Vec<MatchResult> {
        self.play_matches(clubs, date)
    }
}
