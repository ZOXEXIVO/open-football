use crate::ai::PendingAiRequest;
use crate::club::team::coach_perception::{CoachDecisionState, date_to_week};
use crate::club::team::squad::{SquadComposition, SquadManager, TransferListManager};
use crate::context::GlobalContext;
use crate::utils::Logging;
use crate::{Team, TeamResult, TeamType};
use chrono::NaiveDate;

#[derive(Debug)]
pub struct TeamCollection {
    pub teams: Vec<Team>,
    pub coach_state: Option<CoachDecisionState>,
}

impl TeamCollection {
    pub fn new(teams: Vec<Team>) -> Self {
        TeamCollection {
            teams,
            coach_state: None,
        }
    }

    pub fn simulate(&mut self, ctx: GlobalContext<'_>) -> Vec<TeamResult> {
        self.teams
            .iter_mut()
            .map(|team| {
                let message = &format!("simulate team: {}", &team.name);
                Logging::estimate_result(|| team.simulate(ctx.with_team(team.id)), message)
            })
            .collect()
    }

    pub fn by_id(&self, id: u32) -> &Team {
        self.teams
            .iter()
            .find(|t| t.id == id)
            .expect(format!("no team with id = {}", id).as_str())
    }

    pub fn main_team_id(&self) -> Option<u32> {
        self.teams
            .iter()
            .find(|t| t.team_type == TeamType::Main)
            .map(|t| t.id)
    }

    pub fn with_league(&self, league_id: u32) -> Vec<u32> {
        self.teams
            .iter()
            .filter(|t| t.league_id == Some(league_id))
            .map(|t| t.id)
            .collect()
    }

    // ─── Coach state management ──────────────────────────────────────

    pub fn ensure_coach_state(&mut self, date: NaiveDate) {
        let main_team = match self.teams.iter().find(|t| t.team_type == TeamType::Main) {
            Some(t) => t,
            None => return,
        };

        let head_coach = main_team.staffs.head_coach();
        let coach_id = head_coach.id;

        let needs_rebuild = match &self.coach_state {
            Some(state) => state.coach_id != coach_id,
            None => true,
        };

        if needs_rebuild {
            self.coach_state = Some(CoachDecisionState::new(head_coach, date));
        }

        if let Some(ref mut state) = self.coach_state {
            state.current_week = date_to_week(date);
        }
    }

    /// Updates impressions via Option::take(). Decays emotional heat once per cycle.
    pub fn update_all_impressions(&mut self, date: NaiveDate) {
        let mut state = match self.coach_state.take() {
            Some(s) => s,
            None => return,
        };

        for team in &self.teams {
            for player in &team.players.players {
                state.update_impression(player, date, &team.team_type);
            }
        }

        // Decay emotional heat once per update cycle (not per player)
        state.emotional_heat *= 0.80;

        self.coach_state = Some(state);
    }

    /// Build pending AI requests for all squad management operations.
    /// Called during simulate() phase; actual AI calls happen in batch later.
    /// Each request carries its own handler closure — adding new request types
    /// only requires changes here.
    pub fn prepare_ai_requests(&self, date: NaiveDate, club_id: u32) -> Vec<PendingAiRequest> {
        if self.teams.len() < 2 {
            return Vec::new();
        }

        let main_idx = match self.teams.iter().position(|t| t.team_type == TeamType::Main) {
            Some(idx) => idx,
            None => return Vec::new(),
        };

        let reserve_idx = self.find_reserve_team_index();
        let youth_idx = self.find_youth_team_index();

        let mut requests = Vec::new();

        // Squad composition (priority 0 — handles promotions, demotions, and swaps)
        {
            let (query, format) = SquadComposition::prepare_request(
                &self.teams, main_idx, reserve_idx, youth_idx,
            );
            requests.push(PendingAiRequest {
                club_id,
                priority: 0,
                query,
                format,
                handler: Box::new(move |response, data| {
                    let club = data.club_mut(club_id).unwrap();
                    SquadComposition::execute_response(
                        response, &mut club.teams.teams, &mut club.teams.coach_state,
                        main_idx, reserve_idx, youth_idx, date,
                    );
                }),
            });
        }

        // Transfer listing (priority 1)
        {
            let (query, format) = TransferListManager::prepare_request(
                &self.teams, main_idx, date,
            );
            requests.push(PendingAiRequest {
                club_id,
                priority: 1,
                query,
                format,
                handler: Box::new(move |response, data| {
                    let club = data.club_mut(club_id).unwrap();
                    TransferListManager::execute_response(
                        response, &mut club.teams.teams, main_idx, date,
                    );
                }),
            });
        }

        requests
    }

    /// Daily critical squad moves: immediate demotions and ability-based swaps
    pub fn manage_critical_squad_moves(&mut self, date: NaiveDate) {
        if self.teams.len() < 2 {
            return;
        }
        let main_idx = match self.teams.iter().position(|t| t.team_type == TeamType::Main) {
            Some(idx) => idx,
            None => return,
        };
        let reserve_idx = match self.find_reserve_team_index() {
            Some(idx) => idx,
            None => return,
        };

        self.ensure_coach_state(date);

        SquadManager::manage_critical_moves(
            &mut self.teams,
            &mut self.coach_state,
            main_idx,
            reserve_idx,
            date,
        );
    }

    // ─── Helper functions ────────────────────────────────────────────

    fn find_reserve_team_index(&self) -> Option<usize> {
        self.teams.iter().position(|t| t.team_type == TeamType::B)
            .or_else(|| self.teams.iter().position(|t| t.team_type == TeamType::U23))
            .or_else(|| self.teams.iter().position(|t| t.team_type == TeamType::U21))
            .or_else(|| self.teams.iter().position(|t| t.team_type == TeamType::U19))
            .or_else(|| self.teams.iter().position(|t| t.team_type == TeamType::U18))
    }

    fn find_youth_team_index(&self) -> Option<usize> {
        self.teams.iter().position(|t| t.team_type == TeamType::U18)
            .or_else(|| self.teams.iter().position(|t| t.team_type == TeamType::U19))
    }
}
