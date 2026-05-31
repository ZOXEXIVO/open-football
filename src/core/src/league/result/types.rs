use crate::league::{LeagueResult, LeagueTableResult, ScheduleItem};
use crate::r#match::{GoalDetail, Match, Score, TeamScore};
use chrono::NaiveDateTime;

/// What `League::simulate_build` (and the cup equivalent) hand back so a
/// caller can dispatch matches in one big batch later and then resume the
/// per-league processing. Exactly one of `pending` (matches were built)
/// or `immediate` (non-matchday — result was finalised during build) is
/// `Some`; if both are `None`, no work happened today.
pub struct LeagueBuildOutput {
    pub matches: Vec<Match>,
    pub pending: Option<LeaguePendingState>,
    pub immediate: Option<LeagueResult>,
}

/// State stashed between the build and process halves of a single
/// matchday. The build half mutates the league's table/schedule and
/// produces `Match` objects; the process half receives the played
/// `MatchResult`s and applies them against this state. Cups reuse this
/// shape with `table_result = LeagueTableResult {}` and
/// `new_season_started = false`.
pub struct LeaguePendingState {
    pub scheduled_matches: Vec<LeagueMatch>,
    pub table_result: LeagueTableResult,
    pub new_season_started: bool,
}

pub struct LeagueMatch {
    pub id: String,

    pub league_id: u32,
    pub league_slug: String,

    pub date: NaiveDateTime,

    pub home_team_id: u32,
    pub away_team_id: u32,

    pub result: Option<LeagueMatchResultResult>,

    /// Domestic-cup bracket metadata. `Some` only when the fixture is a
    /// knockout cup tie — `DomesticCup::collect_today_matches` fills these
    /// so `build_match` can compute a stage/opponent-aware importance and
    /// hand the selector a `SelectionCompetition::DomesticCup`. Normal
    /// league fixtures leave them `None`.
    pub cup_round: Option<u8>,
    pub cup_total_rounds: Option<u8>,
}

pub struct LeagueMatchResultResult {
    pub home: TeamScore,
    pub away: TeamScore,
    pub details: Vec<GoalDetail>,
}

impl LeagueMatchResultResult {
    pub fn from_score(score: &Score) -> Self {
        LeagueMatchResultResult {
            home: TeamScore::from(&score.home_team),
            away: TeamScore::from(&score.away_team),
            details: score.detail().to_vec(),
        }
    }
}

impl From<ScheduleItem> for LeagueMatch {
    fn from(item: ScheduleItem) -> Self {
        let mut result = LeagueMatch {
            id: item.id.clone(),
            league_id: item.league_id,
            league_slug: item.league_slug,
            date: item.date,
            home_team_id: item.home_team_id,
            away_team_id: item.away_team_id,
            result: None,
            cup_round: None,
            cup_total_rounds: None,
        };

        if let Some(res) = item.result {
            result.result = Some(LeagueMatchResultResult::from_score(&res));
        }

        result
    }
}
