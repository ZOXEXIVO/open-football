mod cup_showcase;
pub mod data_access;
mod match_events;
mod physical;
mod types;

pub use data_access::{
    ClubProcessCtx, CountryLookupIndex, CountryProcessCtx, DeferredContractInteraction,
    DeferredGlobalOps, LeagueProcessAccess, StagedClubOps, WorldSnapshot,
};

pub use types::*;

use crate::league::LeagueTableResult;
use crate::r#match::MatchResult;
use crate::r#match::TeamScore;
use crate::simulator::SimulatorData;
use crate::{MatchHistoryItem, SimulationResult};

pub struct LeagueResult {
    pub league_id: u32,
    pub table_result: LeagueTableResult,
    pub match_results: Option<Vec<MatchResult>>,
    pub new_season_started: bool,
}

impl LeagueResult {
    pub fn new(league_id: u32, table_result: LeagueTableResult) -> Self {
        LeagueResult {
            league_id,
            table_result,
            match_results: None,
            new_season_started: false,
        }
    }

    pub fn with_match_result(
        league_id: u32,
        table_result: LeagueTableResult,
        match_results: Vec<MatchResult>,
    ) -> Self {
        LeagueResult {
            league_id,
            table_result,
            match_results: Some(match_results),
            new_season_started: false,
        }
    }

    pub fn process(self, data: &mut SimulatorData, result: &mut SimulationResult) {
        if let Some(match_results) = self.match_results {
            for mut match_result in match_results {
                Self::process_match_results(&mut match_result, data);

                result.match_results.push(match_result);
            }
        }
    }

    /// Country-local entry point. Same per-match pipeline as `process`,
    /// but driven through a `CountryProcessCtx` so it can run inside
    /// `Country::simulate` (Phase A) without `&mut SimulatorData`. The
    /// processed match results are pushed onto `out_match_results`; the
    /// simulator's serial Phase C drains them into
    /// `SimulationResult.match_results` after the parallel pass joins.
    pub fn process_local(
        self,
        ctx: &mut CountryProcessCtx<'_>,
        out_match_results: &mut Vec<MatchResult>,
    ) {
        if let Some(match_results) = self.match_results {
            for mut match_result in match_results {
                Self::process_match_results(&mut match_result, ctx);
                out_match_results.push(match_result);
            }
        }
    }

    /// Process a cup match result (Champions League, etc.) through the stat pipeline.
    /// Called from continental competition processing.
    pub fn process_cup_match(result: &mut MatchResult, data: &mut SimulatorData) {
        Self::process_match_results(result, data);
    }

    fn process_match_results<D: data_access::LeagueProcessAccess>(
        result: &mut MatchResult,
        data: &mut D,
    ) {
        let now = data.date();

        // Update league schedule (skip for friendlies without a league)
        if let Some(league) = data.league_mut(result.league_id) {
            league
                .schedule
                .update_match_result(&result.id, &result.score);
        }

        let home_team_id = result.score.home_team.team_id;
        let away_team_id = result.score.away_team.team_id;
        // Credit a home match against the club's matchday counter so the
        // monthly finance pass can scale the gate by actual fixtures
        // rather than a hardcoded `* 2`. Friendlies don't draw paying
        // crowds for the model, so they're skipped.
        if !result.friendly {
            let home_club_id = data.team(home_team_id).map(|t| t.club_id);
            if let Some(club_id) = home_club_id {
                if let Some(home_club) = data.club_mut(club_id) {
                    home_club.finance.record_home_match();
                }
            }
        }
        // Pull the per-side final tactic the engine recorded — captures
        // any in-match shape change so the team's match history mirrors
        // what the coach really did, not just what was on the team
        // sheet.
        let final_home_tactic = result.details.as_ref().and_then(|d| d.final_home_tactic);
        let final_away_tactic = result.details.as_ref().and_then(|d| d.final_away_tactic);
        let tactic_summary = result.details.as_ref().map(|d| {
            (
                d.starting_home_tactic,
                d.starting_away_tactic,
                d.shape_change_minute,
            )
        });
        let home_starting_eleven = result
            .details
            .as_ref()
            .map(|d| d.left_team_players.starter_slots.clone())
            .unwrap_or_default();
        let away_starting_eleven = result
            .details
            .as_ref()
            .map(|d| d.right_team_players.starter_slots.clone())
            .unwrap_or_default();

        let home_team = data
            .team_mut(home_team_id)
            .expect(&format!("home team not found: {}", home_team_id));
        let mut home_item = MatchHistoryItem::new(
            now,
            // rival is the OPPONENT, not us. The legacy code stored the
            // team's own id here, which broke every consumer that read
            // `rival_team_id` to find the opposing club (web tactics
            // history, opponent-tactic feed, etc.).
            away_team_id,
            (
                TeamScore::from(&result.score.home_team),
                TeamScore::from(&result.score.away_team),
            ),
        )
        .with_tactic(final_home_tactic)
        .with_starting_eleven(home_starting_eleven);
        if let Some((start, _, change_minute)) = tactic_summary {
            home_item = home_item.with_tactic_summary(start, final_home_tactic, change_minute);
        }
        home_team.match_history.add(home_item);

        let away_team = data
            .team_mut(away_team_id)
            .expect(&format!("away team not found: {}", away_team_id));
        let mut away_item = MatchHistoryItem::new(
            now,
            home_team_id,
            (
                TeamScore::from(&result.score.away_team),
                TeamScore::from(&result.score.home_team),
            ),
        )
        .with_tactic(final_away_tactic)
        .with_starting_eleven(away_starting_eleven);
        if let Some((_, start, change_minute)) = tactic_summary {
            away_item = away_item.with_tactic_summary(start, final_away_tactic, change_minute);
        }
        away_team.match_history.add(away_item);

        Self::process_match_events(result, data);
    }
}
