use crate::league::LeagueMatch;
use crate::r#match::statistics::MatchStatisticType;
use crate::r#match::{ResultMatchPositionData, TeamSquad};
use std::sync::atomic::{AtomicU8, Ordering};

#[derive(Debug)]
pub struct MatchResultRaw {
    pub score: Option<Score>,

    pub position_data: ResultMatchPositionData,

    pub left_team_players: FieldSquad,
    pub right_team_players: FieldSquad,

    pub match_time_ms: u64,
    pub additional_time_ms: u64,
}

impl Clone for MatchResultRaw {
    fn clone(&self) -> Self {
        MatchResultRaw {
            score: self.score.clone(),
            position_data: self.position_data.clone(),
            left_team_players: self.left_team_players.clone(),
            right_team_players: self.right_team_players.clone(),
            match_time_ms: self.match_time_ms,
            additional_time_ms: self.additional_time_ms,
        }
    }
}

impl MatchResultRaw {
    pub fn with_match_time(match_time_ms: u64) -> Self {
        MatchResultRaw {
            score: None,
            position_data: ResultMatchPositionData::new(),
            left_team_players: FieldSquad::new(),
            right_team_players: FieldSquad::new(),
            match_time_ms,
            additional_time_ms: 0,
        }
    }

    pub fn copy_without_data_positions(&self) -> Self {
        MatchResultRaw {
            score: self.score.clone(),
            position_data: ResultMatchPositionData::new(),
            left_team_players: self.left_team_players.clone(),
            right_team_players: self.right_team_players.clone(),
            match_time_ms: self.match_time_ms,
            additional_time_ms: self.additional_time_ms,
        }
    }

    pub fn write_team_players(
        &mut self,
        home_team_players: &FieldSquad,
        away_team_players: &FieldSquad,
    ) {
        self.left_team_players = FieldSquad::from(home_team_players);
        self.right_team_players = FieldSquad::from(away_team_players);
    }
}

#[derive(Debug, Clone)]
pub struct FieldSquad {
    pub main: Vec<u32>,
    pub substitutes: Vec<u32>,
}

impl FieldSquad {
    pub fn new() -> Self {
        FieldSquad {
            main: Vec::new(),
            substitutes: Vec::new(),
        }
    }

    pub fn from(field_squad: &FieldSquad) -> Self {
        FieldSquad {
            main: field_squad.main.to_vec(),
            substitutes: field_squad.substitutes.to_vec(),
        }
    }

    pub fn from_team(squad: &TeamSquad) -> Self {
        FieldSquad {
            main: squad.main_squad.iter().map(|p| p.id).collect(),
            substitutes: squad.substitutes.iter().map(|p| p.id).collect(),
        }
    }

    pub fn count(&self) -> usize {
        self.main.len() + self.substitutes.len()
    }
}

#[derive(Debug, Clone)]
pub struct Score {
    pub home_team: TeamScore,
    pub away_team: TeamScore,

    pub details: Vec<GoalDetail>,
}

#[derive(Debug)]
pub struct TeamScore {
    pub team_id: u32,
    score: AtomicU8,
}

impl Clone for TeamScore {
    fn clone(&self) -> Self {
        TeamScore {
            team_id: self.team_id,
            score: AtomicU8::new(self.score.load(Ordering::Relaxed)),
        }
    }
}

impl TeamScore {
    pub fn new(team_id: u32) -> Self {
        TeamScore {
            team_id,
            score: AtomicU8::new(0),
        }
    }

    pub fn new_with_score(team_id: u32, score: u8) -> Self {
        TeamScore {
            team_id,
            score: AtomicU8::new(score),
        }
    }

    pub fn get(&self) -> u8 {
        self.score.load(Ordering::Relaxed)
    }
}
impl From<&TeamScore> for TeamScore {
    fn from(team_score: &TeamScore) -> Self {
        TeamScore::new_with_score(team_score.team_id, team_score.score.load(Ordering::Relaxed))
    }
}

impl PartialEq<Self> for TeamScore {
    fn eq(&self, other: &Self) -> bool {
        self.score.load(Ordering::Relaxed) == other.score.load(Ordering::Relaxed)
    }
}

impl PartialOrd for TeamScore {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        let left_score = self.score.load(Ordering::Relaxed);
        let other_score = other.score.load(Ordering::Relaxed);

        Some(left_score.cmp(&other_score))
    }
}

#[derive(Debug, Clone)]
pub struct GoalDetail {
    pub player_id: u32,
    pub stat_type: MatchStatisticType,

    pub time: u64,
}

impl Score {
    pub fn new(home_team_id: u32, away_team_id: u32) -> Self {
        Score {
            home_team: TeamScore::new(home_team_id),
            away_team: TeamScore::new(away_team_id),
            details: Vec::new(),
        }
    }

    pub fn add_goal_detail(&mut self, goal_detail: GoalDetail) {
        self.details.push(goal_detail)
    }

    pub fn detail(&self) -> &[GoalDetail] {
        &self.details
    }

    pub fn increment_home_goals(&self) {
        self.home_team.score.fetch_add(1, Ordering::Relaxed);
    }

    pub fn increment_away_goals(&self) {
        self.away_team.score.fetch_add(1, Ordering::Relaxed);
    }
}

#[derive(Debug, Clone)]
pub struct MatchResult {
    pub id: String,
    pub league_id: u32,
    pub league_slug: String,
    pub home_team_id: u32,
    pub away_team_id: u32,
    pub details: Option<MatchResultRaw>,
    pub score: Score,
}

impl MatchResult {
    pub fn copy_without_data_positions(&self) -> Self {
        MatchResult {
            id: String::from(&self.id),
            league_id: self.league_id,
            league_slug: String::from(&self.league_slug),
            home_team_id: self.home_team_id,
            away_team_id: self.away_team_id,
            details: if self.details.is_some() {
                Some(self.details.as_ref().unwrap().copy_without_data_positions())
            } else {
                None
            },
            score: self.score.clone(),
        }
    }
}

impl From<&LeagueMatch> for MatchResult {
    fn from(m: &LeagueMatch) -> Self {
        MatchResult {
            id: m.id.clone(),
            league_id: m.league_id,
            league_slug: m.league_slug.clone(),
            home_team_id: m.home_team_id,
            away_team_id: m.away_team_id,
            score: Score::new(m.home_team_id, m.away_team_id),
            details: None,
        }
    }
}

impl PartialEq for MatchResult {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id
    }
}
