use super::{CompetitionStage, CompetitionTier, ContinentalMatch, ContinentalMatchResult, GroupTable, KnockoutTie, CONFERENCE_LEAGUE_ID};
use crate::continent::ContinentalRankings;
use crate::r#match::{Match, MatchResult, SelectionContext};
use crate::Club;
use chrono::{Datelike, NaiveDate};
use log::{debug, info};
use std::collections::HashMap;

pub const CONFERENCE_LEAGUE_SLUG: &str = "conference-league";

#[derive(Debug, Clone)]
pub struct ConferenceLeague {
    pub participating_clubs: Vec<u32>,
    pub current_stage: CompetitionStage,
    pub groups: Vec<GroupTable>,
    pub knockout_round: Vec<KnockoutTie>,
    pub matches: Vec<ContinentalMatch>,
    pub prize_pool: f64,
    pub season_year: u16,
}

impl Default for ConferenceLeague {
    fn default() -> Self {
        Self::new()
    }
}

impl ConferenceLeague {
    pub fn new() -> Self {
        ConferenceLeague {
            participating_clubs: Vec::new(),
            current_stage: CompetitionStage::NotStarted,
            groups: Vec::new(),
            knockout_round: Vec::new(),
            matches: Vec::new(),
            prize_pool: 250_000_000.0,
            season_year: 0,
        }
    }

    /// Conduct draw: seed clubs into groups of 4, generate fixtures.
    pub fn conduct_draw(&mut self, clubs: &[u32], _rankings: &ContinentalRankings, date: NaiveDate) {
        if clubs.len() < 8 {
            debug!("Conference League: not enough clubs ({}) for draw", clubs.len());
            return;
        }

        let count = clubs.len().min(32);
        self.participating_clubs = clubs[..count].to_vec();
        self.season_year = date.year() as u16;

        let num_groups = (count / 4).max(1).min(8);
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

        // Generate group stage fixtures (6 matchdays, Thursdays — same weeks as EL)
        self.matches.clear();
        let year = date.year();
        let matchday_dates = [
            NaiveDate::from_ymd_opt(year, 9, 19).unwrap(),  // MD1
            NaiveDate::from_ymd_opt(year, 10, 3).unwrap(),  // MD2
            NaiveDate::from_ymd_opt(year, 10, 24).unwrap(), // MD3
            NaiveDate::from_ymd_opt(year, 11, 7).unwrap(),  // MD4
            NaiveDate::from_ymd_opt(year, 11, 28).unwrap(), // MD5
            NaiveDate::from_ymd_opt(year, 12, 12).unwrap(), // MD6
        ];

        for group in &self.groups {
            let teams: Vec<u32> = group.rows.iter().map(|r| r.team_id).collect();
            if teams.len() < 4 { continue; }

            let fixtures = [
                (0, 1, 2, 3, 0),
                (2, 0, 3, 1, 1),
                (0, 3, 1, 2, 2),
                (3, 0, 2, 1, 3),
                (1, 0, 3, 2, 4),
                (0, 2, 1, 3, 5),
            ];

            for (h1, a1, h2, a2, md) in fixtures {
                self.matches.push(ContinentalMatch {
                    home_team: teams[h1],
                    away_team: teams[a1],
                    date: matchday_dates[md],
                    stage: CompetitionStage::GroupStage,
                    match_id: String::new(),
                    result: None,
                });
                self.matches.push(ContinentalMatch {
                    home_team: teams[h2],
                    away_team: teams[a2],
                    date: matchday_dates[md],
                    stage: CompetitionStage::GroupStage,
                    match_id: String::new(),
                    result: None,
                });
            }
        }

        self.current_stage = CompetitionStage::GroupStage;

        info!(
            "Conference League draw: {} clubs in {} groups, {} fixtures",
            count, num_groups, self.matches.len()
        );
    }

    pub fn generate_knockout_fixtures(&mut self, year: i32) {
        let mut winners = Vec::new();
        let mut runners_up = Vec::new();

        for group in &self.groups {
            if group.rows.len() >= 2 {
                let (w, r) = group.qualifiers();
                winners.push(w);
                runners_up.push(r);
            }
        }

        self.knockout_round.clear();
        let num_ties = winners.len().min(runners_up.len());
        for i in 0..num_ties {
            let r_idx = (i + 1) % runners_up.len();
            self.knockout_round.push(KnockoutTie::new(winners[i], runners_up[r_idx]));
        }

        // Schedule R16 matches (Thursdays)
        let r16_dates_leg1 = [
            NaiveDate::from_ymd_opt(year + 1, 2, 20).unwrap(),
            NaiveDate::from_ymd_opt(year + 1, 2, 27).unwrap(),
        ];
        let r16_dates_leg2 = [
            NaiveDate::from_ymd_opt(year + 1, 3, 13).unwrap(),
            NaiveDate::from_ymd_opt(year + 1, 3, 20).unwrap(),
        ];

        for (i, tie) in self.knockout_round.iter().enumerate() {
            let leg1_date = r16_dates_leg1[i % r16_dates_leg1.len()];
            let leg2_date = r16_dates_leg2[i % r16_dates_leg2.len()];

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
            "Conference League R16: {} ties, {} matches scheduled",
            self.knockout_round.len(),
            self.knockout_round.len() * 2
        );
    }

    pub fn has_matches_today(&self, date: NaiveDate) -> bool {
        self.matches.iter().any(|m| m.date == date)
    }

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

        let selection_ctx = SelectionContext {
            is_friendly: false,
            date,
            match_importance: 0.9,
            philosophy: None,
            opponent_tactic: None,
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

                let match_id = format!("conf_{}_{}_{}",
                    date.format("%Y%m%d"), cm.home_team, cm.away_team);

                Some(Match::make(
                    match_id,
                    CONFERENCE_LEAGUE_ID,
                    CONFERENCE_LEAGUE_SLUG,
                    home_squad,
                    away_squad,
                    false,
                ))
            })
            .collect();

        if engine_matches.is_empty() {
            return Vec::new();
        }

        let results = crate::match_engine_pool().play(engine_matches);

        // Store results back on the matches
        for (cm, result) in todays_matches.iter().zip(results.iter()) {
            let home_goals = result.score.home_team.get();
            let away_goals = result.score.away_team.get();
            if let Some(m) = self.matches.iter_mut().find(|m|
                m.date == cm.date && m.home_team == cm.home_team && m.away_team == cm.away_team
            ) {
                m.match_id = result.id.clone();
                m.result = Some((home_goals, away_goals));
            }
        }

        for (cm, result) in todays_matches.iter().zip(results.iter()) {
            let home_goals = result.score.home_team.get();
            let away_goals = result.score.away_team.get();

            match cm.stage {
                CompetitionStage::GroupStage => {
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
                "CONF: {} {} - {} {} ({:?})",
                cm.home_team, home_goals, away_goals, cm.away_team, cm.stage
            );
        }

        let group_stage_complete = matches!(self.current_stage, CompetitionStage::GroupStage)
            && self.groups.iter().all(|g| g.rows.iter().all(|r| r.played >= 6));

        if group_stage_complete {
            info!("Conference League group stage complete -- generating R16 draw");
            self.generate_knockout_fixtures(date.year());
        }

        results
    }

    pub fn simulate_round(
        &mut self,
        clubs: &HashMap<u32, &Club>,
        date: NaiveDate,
    ) -> Vec<ContinentalMatchResult> {
        let match_results = self.play_matches(clubs, date);

        match_results.iter().map(|r| {
            ContinentalMatchResult {
                home_team: r.home_team_id,
                away_team: r.away_team_id,
                home_score: r.score.home_team.get(),
                away_score: r.score.away_team.get(),
                competition: CompetitionTier::ConferenceLeague,
            }
        }).collect()
    }

    pub fn get_club_points(&self, club_id: u32) -> f32 {
        if !self.participating_clubs.contains(&club_id) {
            return 0.0;
        }

        for group in &self.groups {
            if let Some(row) = group.rows.iter().find(|r| r.team_id == club_id) {
                return 3.0 + row.points as f32;
            }
        }

        3.0
    }
}
