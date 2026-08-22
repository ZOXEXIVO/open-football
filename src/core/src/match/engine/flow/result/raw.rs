//! What the engine hands back when the whistle goes.
//!
//! [`MatchResultRaw`] is the full engine payload — squads, per-player
//! stat lines, substitutions, chances, the position replay — and
//! [`MatchResult`] is the league-facing wrapper that carries it (or
//! carries only the score, once the replay has been stripped).
//! [`FieldSquad`] is the roster identity both sides of that boundary
//! agree on.

use super::highlights::ChanceDetail;
use super::player_stats::{PlayerMatchEndStats, PlayerMatchPhysicalSnapshot};
use super::score::Score;
use super::substitution::SubstitutionInfo;
use crate::league::LeagueMatch;
use crate::r#match::squad::OmittedPlayer;
use crate::r#match::{MatchSquad, ResultMatchPositionData};
use crate::{MatchTacticType, PlayerPositionType};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PenaltyShootoutKick {
    pub team_id: u32,
    pub taker_id: u32,
    pub goalkeeper_id: Option<u32>,
    pub round: u8,
    pub scored: bool,
    pub sudden_death: bool,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct MatchResultRaw {
    pub score: Option<Score>,

    /// Position-replay payload. NEVER serialised over the worker wire
    /// — the bincode payload would balloon to many MB per match and the
    /// recorder is only enabled for the local viewer anyway. On the
    /// receive side an empty `ResultMatchPositionData::empty()` is
    /// substituted in.
    #[serde(skip, default = "ResultMatchPositionData::empty")]
    pub position_data: ResultMatchPositionData,

    pub left_team_players: FieldSquad,
    pub right_team_players: FieldSquad,

    pub match_time_ms: u64,
    pub additional_time_ms: u64,

    pub player_stats: HashMap<u32, PlayerMatchEndStats>,

    pub substitutions: Vec<SubstitutionInfo>,

    /// The near misses worth watching again —
    /// [`HighlightSelector::PER_TEAM`](super::highlights::HighlightSelector::PER_TEAM)
    /// per side at most, already shortlisted by the time this is built. The
    /// recording carries a clip around each one, so the two lists are written
    /// together in `build_result` and must never be filtered apart afterwards:
    /// a marker with no footage under it is a seek that lands in a grey zone.
    ///
    /// Defaulted for results written before chances were kept at all.
    #[serde(default)]
    pub chances: Vec<ChanceDetail>,

    /// Final physical snapshot per player who appeared in this match.
    /// Populated for every player on the pitch at full time AND every
    /// player who was substituted off (snapshot taken at the swap).
    /// Consumed by `LeagueResult::apply_post_match_physical_effects`
    /// to feed `final_match_energy` into the new condition-drop
    /// formula. Missing entries fall back to the legacy minutes-only
    /// path so callers that don't construct `MatchResultRaw` via the
    /// engine continue to work.
    pub physical_snapshots: HashMap<u32, PlayerMatchPhysicalSnapshot>,

    pub penalty_shootout: Vec<PenaltyShootoutKick>,

    pub player_of_the_match_id: Option<u32>,

    /// The shape each team STARTED the match in (taken from
    /// `MatchSquad::tactics` at kickoff). Lets the result and the
    /// downstream `MatchHistory` show a "starting → final" tactical
    /// summary instead of just "what shape was on the pitch at the
    /// final whistle".
    pub starting_home_tactic: Option<MatchTacticType>,
    pub starting_away_tactic: Option<MatchTacticType>,
    /// The shape each team finished the match in. Differs from the
    /// starting shape when the in-match coach probed a situational
    /// override (`evaluate_situational_shape`) — e.g. 4-4-2 → 4-3-3
    /// chasing a deficit, 4-4-2 → 4-5-1 protecting a lead. Surfaced so
    /// the league pipeline can record it on `MatchHistory` and the
    /// web tactics view can render "Plan: 4-4-2 — Last used: 4-3-3
    /// (chase) vs Spurs".
    pub final_home_tactic: Option<MatchTacticType>,
    pub final_away_tactic: Option<MatchTacticType>,
    /// Sim-minute at which the FIRST shape change fired for either
    /// side (whichever came first). `None` when neither side changed
    /// shape during the match. Stored as the marker the web view uses
    /// to label a chip with "shifted at min X".
    pub shape_change_minute: Option<u8>,
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
            chances: self.chances.clone(),
            physical_snapshots: self.physical_snapshots.clone(),
            penalty_shootout: self.penalty_shootout.clone(),
            player_of_the_match_id: self.player_of_the_match_id,
            starting_home_tactic: self.starting_home_tactic,
            starting_away_tactic: self.starting_away_tactic,
            final_home_tactic: self.final_home_tactic,
            final_away_tactic: self.final_away_tactic,
            shape_change_minute: self.shape_change_minute,
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
            chances: Vec::new(),
            physical_snapshots: HashMap::new(),
            penalty_shootout: Vec::new(),
            player_of_the_match_id: None,
            starting_home_tactic: None,
            starting_away_tactic: None,
            final_home_tactic: None,
            final_away_tactic: None,
            shape_change_minute: None,
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
            chances: self.chances.clone(),
            physical_snapshots: self.physical_snapshots.clone(),
            penalty_shootout: self.penalty_shootout.clone(),
            player_of_the_match_id: self.player_of_the_match_id,
            starting_home_tactic: self.starting_home_tactic,
            starting_away_tactic: self.starting_away_tactic,
            final_home_tactic: self.final_home_tactic,
            final_away_tactic: self.final_away_tactic,
            shape_change_minute: self.shape_change_minute,
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FieldSquad {
    pub team_id: u32,
    pub main: Vec<u32>,
    pub substitutes: Vec<u32>,
    pub substitutes_used: Vec<u32>,
    /// Important omissions surfaced at squad-selection time, threaded
    /// through the match engine so the post-match dispatcher can call
    /// `Player::on_match_dropped_with_context` with the right
    /// explanation. Empty when nothing notable happened.
    pub selection_omissions: Vec<OmittedPlayer>,
    /// Slot each starter was assigned at kickoff. Drives the post-
    /// match coach-memory observation's `role_fit` reading so a
    /// player pressed into an emergency role (e.g. a midfielder
    /// starting at fullback) registers as an out-of-position
    /// appearance instead of a natural-slot start. Empty for
    /// FieldSquads built outside the squad selector flow (tests,
    /// dev_match harness) — the dispatcher falls back to assuming
    /// a natural-role start in that case.
    #[serde(default)]
    pub starter_slots: Vec<(u32, PlayerPositionType)>,
}

impl FieldSquad {
    pub fn new() -> Self {
        FieldSquad {
            team_id: 0,
            main: Vec::new(),
            substitutes: Vec::new(),
            substitutes_used: Vec::new(),
            selection_omissions: Vec::new(),
            starter_slots: Vec::new(),
        }
    }

    pub fn from_team(squad: &MatchSquad) -> Self {
        FieldSquad {
            team_id: squad.team_id,
            main: squad.main_squad.iter().map(|p| p.id).collect(),
            substitutes: squad.substitutes.iter().map(|p| p.id).collect(),
            substitutes_used: Vec::new(),
            selection_omissions: squad.selection_omissions.clone(),
            starter_slots: squad
                .main_squad
                .iter()
                .map(|p| (p.id, p.tactical_position.current_position))
                .collect(),
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

    /// Slot the starter played at kickoff, or `None` if the
    /// `starter_slots` map was not populated (legacy paths) or the
    /// player wasn't in the starting eleven.
    pub fn starter_slot(&self, player_id: u32) -> Option<PlayerPositionType> {
        self.starter_slots
            .iter()
            .find(|(id, _)| *id == player_id)
            .map(|(_, slot)| *slot)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
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
