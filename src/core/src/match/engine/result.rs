use crate::PlayerFieldPositionGroup;
use crate::league::LeagueMatch;
use crate::r#match::player::statistics::MatchStatisticType;
use crate::r#match::{MatchSquad, ResultMatchPositionData};
use std::collections::HashMap;
use std::sync::atomic::{AtomicU8, Ordering};

#[derive(Debug, Clone)]
pub struct SubstitutionInfo {
    pub team_id: u32,
    pub player_out_id: u32,
    pub player_in_id: u32,
    pub match_time_ms: u64,
}

#[derive(Debug, Clone)]
pub struct PlayerMatchEndStats {
    pub shots_on_target: u16,
    pub shots_total: u16,
    pub passes_attempted: u16,
    pub passes_completed: u16,
    pub tackles: u16,
    pub interceptions: u16,
    pub saves: u16,
    /// Shots-on-target the player (typically a GK) had to deal with —
    /// `saves` + goals conceded. Drives the save-percentage component
    /// of the rating helper.
    pub shots_faced: u16,
    pub goals: u16,
    pub assists: u16,
    pub match_rating: f32,
    /// Sum of expected goals from this player's shots in this match.
    pub xg: f32,
    /// Player's position group for position-aware rating calculation.
    pub position_group: PlayerFieldPositionGroup,
    /// Fouls committed by the player in this match.
    pub fouls: u16,
    /// Yellow cards received (0, 1, or 2).
    pub yellow_cards: u16,
    /// 1 if the player was sent off (either two yellows or direct red).
    pub red_cards: u16,
    /// Match minutes played. Used by the rating helper to dampen event
    /// bonuses for short cameos.
    pub minutes_played: u16,
    /// Modern build-up / chance-creation stats — feed the rating helper
    /// and end-of-match calibration. All zero for legacy callers.
    pub key_passes: u16,
    pub progressive_passes: u16,
    pub progressive_carries: u16,
    pub successful_dribbles: u16,
    pub attempted_dribbles: u16,
    pub successful_pressures: u16,
    /// Total close-range pressures applied — superset of
    /// `successful_pressures`. Used for a small "pressing volume" credit
    /// that's worth less per event than a successful pressure.
    pub pressures: u16,
    pub blocks: u16,
    pub clearances: u16,
    /// Completed passes finishing inside the opposition penalty area —
    /// chance-creation indicator independent of the eventual shot.
    pub passes_into_box: u16,
    pub crosses_attempted: u16,
    pub crosses_completed: u16,
    /// xG of all shots in possessions this player participated in. Used
    /// for build-up credit (small) without double-counting goals.
    pub xg_chain: f32,
    /// xG of build-up chains excluding the player's own shots / assists.
    /// Pure "made the chance happen" signal.
    pub xg_buildup: f32,
    /// First-touch resolutions that fluffed the ball.
    pub miscontrols: u16,
    /// First-touch resolutions in the heavy-touch band — kept the ball
    /// alive but gave it away in tempo.
    pub heavy_touches: u16,
    /// Cumulative pitch-units carried under control. Tie-breaker only.
    pub carry_distance: u32,
    pub errors_leading_to_shot: u16,
    pub errors_leading_to_goal: u16,
    /// (GK) Post-shot xG faced minus goals conceded. Positive values
    /// indicate above-expectation shot-stopping.
    pub xg_prevented: f32,
}

#[derive(Debug, Clone)]
pub struct PenaltyShootoutKick {
    pub team_id: u32,
    pub taker_id: u32,
    pub goalkeeper_id: Option<u32>,
    pub round: u8,
    pub scored: bool,
    pub sudden_death: bool,
}

#[derive(Debug)]
pub struct MatchResultRaw {
    pub score: Option<Score>,

    pub position_data: ResultMatchPositionData,

    pub left_team_players: FieldSquad,
    pub right_team_players: FieldSquad,

    pub match_time_ms: u64,
    pub additional_time_ms: u64,

    pub player_stats: HashMap<u32, PlayerMatchEndStats>,

    pub substitutions: Vec<SubstitutionInfo>,

    pub penalty_shootout: Vec<PenaltyShootoutKick>,

    pub player_of_the_match_id: Option<u32>,
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
            player_stats: self.player_stats.clone(),
            substitutions: self.substitutions.clone(),
            penalty_shootout: self.penalty_shootout.clone(),
            player_of_the_match_id: self.player_of_the_match_id,
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
            player_stats: HashMap::new(),
            substitutions: Vec::new(),
            penalty_shootout: Vec::new(),
            player_of_the_match_id: None,
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
            player_stats: self.player_stats.clone(),
            substitutions: self.substitutions.clone(),
            penalty_shootout: self.penalty_shootout.clone(),
            player_of_the_match_id: self.player_of_the_match_id,
        }
    }

    pub fn write_team_players(
        &mut self,
        home_team_players: &FieldSquad,
        away_team_players: &FieldSquad,
    ) {
        self.left_team_players = home_team_players.clone();
        self.right_team_players = away_team_players.clone();
    }
}

#[derive(Debug, Clone)]
pub struct FieldSquad {
    pub team_id: u32,
    pub main: Vec<u32>,
    pub substitutes: Vec<u32>,
    pub substitutes_used: Vec<u32>,
}

impl FieldSquad {
    pub fn new() -> Self {
        FieldSquad {
            team_id: 0,
            main: Vec::new(),
            substitutes: Vec::new(),
            substitutes_used: Vec::new(),
        }
    }

    pub fn from_team(squad: &MatchSquad) -> Self {
        FieldSquad {
            team_id: squad.team_id,
            main: squad.main_squad.iter().map(|p| p.id).collect(),
            substitutes: squad.substitutes.iter().map(|p| p.id).collect(),
            substitutes_used: Vec::new(),
        }
    }

    pub fn mark_substitute_used(&mut self, player_id: u32) {
        if self.substitutes.contains(&player_id) && !self.substitutes_used.contains(&player_id) {
            self.substitutes_used.push(player_id);
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

    /// Penalty-shootout tally (0 if no shootout took place).
    pub home_shootout: u8,
    pub away_shootout: u8,
}

/// Outcome of a match from the home team's perspective, considering
/// both the regulation (+ extra time) score and the shootout tally.
/// Distinct from `club::MatchOutcome`, which is team-relative (Win/Draw/Loss).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MatchResultOutcome {
    HomeWin,
    AwayWin,
    Draw,
}

impl Score {
    /// True if the regulation + extra-time score is level.
    pub fn is_tied(&self) -> bool {
        self.home_team.get() == self.away_team.get()
    }

    /// Did a shootout take place? (Either side has shootout goals recorded.)
    pub fn had_shootout(&self) -> bool {
        self.home_shootout > 0 || self.away_shootout > 0
    }

    /// True outcome — accounts for penalty shootout tiebreak in knockouts.
    /// Regulation-only consumers (league table, points) should use
    /// `home_team.get()` / `away_team.get()` directly; those stay as-is.
    pub fn outcome(&self) -> MatchResultOutcome {
        let h = self.home_team.get();
        let a = self.away_team.get();
        if h > a {
            return MatchResultOutcome::HomeWin;
        }
        if a > h {
            return MatchResultOutcome::AwayWin;
        }
        // Regulation tied — use shootout if any kick was taken.
        if self.had_shootout() {
            if self.home_shootout > self.away_shootout {
                MatchResultOutcome::HomeWin
            } else if self.away_shootout > self.home_shootout {
                MatchResultOutcome::AwayWin
            } else {
                // Shootout technically can't end tied, but belt-and-braces.
                MatchResultOutcome::Draw
            }
        } else {
            MatchResultOutcome::Draw
        }
    }
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
    pub is_auto_goal: bool,
    pub time: u64,
}

impl Score {
    pub fn new(home_team_id: u32, away_team_id: u32) -> Self {
        Score {
            home_team: TeamScore::new(home_team_id),
            away_team: TeamScore::new(away_team_id),
            details: Vec::new(),
            home_shootout: 0,
            away_shootout: 0,
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
    pub friendly: bool,
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
            friendly: self.friendly,
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
            friendly: false,
        }
    }
}

impl PartialEq for MatchResult {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id
    }
}
