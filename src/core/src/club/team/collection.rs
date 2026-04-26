use crate::ai::PendingAiRequest;
use crate::club::staff::perception::{CoachDecisionState, date_to_week};
use crate::club::team::squad::{ContractRenewalManager, SquadComposition, SquadManager, TransferListManager};
use crate::context::GlobalContext;
use crate::utils::Logging;
use crate::{HappinessEventType, Team, TeamResult, TeamType};
use chrono::NaiveDate;
use rayon::iter::IntoParallelRefMutIterator;
use rayon::iter::ParallelIterator;

#[derive(Debug, Clone)]
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
            .par_iter_mut()
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

    /// Borrow a team by id. Unlike `by_id`, returns `None` for missing ids
    /// — prefer this when the caller can gracefully handle absence.
    pub fn find(&self, team_id: u32) -> Option<&Team> {
        self.teams.iter().find(|t| t.id == team_id)
    }

    /// Mutable variant of `find`.
    pub fn find_mut(&mut self, team_id: u32) -> Option<&mut Team> {
        self.teams.iter_mut().find(|t| t.id == team_id)
    }

    pub fn contains(&self, team_id: u32) -> bool {
        self.teams.iter().any(|t| t.id == team_id)
    }

    pub fn iter(&self) -> std::slice::Iter<'_, Team> {
        self.teams.iter()
    }

    pub fn iter_mut(&mut self) -> std::slice::IterMut<'_, Team> {
        self.teams.iter_mut()
    }

    pub fn len(&self) -> usize {
        self.teams.len()
    }

    pub fn is_empty(&self) -> bool {
        self.teams.is_empty()
    }

    /// Borrow the main first team if one exists.
    pub fn main(&self) -> Option<&Team> {
        self.teams.iter().find(|t| t.team_type == TeamType::Main)
    }

    /// Mutable variant of `main`.
    pub fn main_mut(&mut self) -> Option<&mut Team> {
        self.teams.iter_mut().find(|t| t.team_type == TeamType::Main)
    }

    /// Array index of the main team, if any.
    pub fn main_index(&self) -> Option<usize> {
        self.teams.iter().position(|t| t.team_type == TeamType::Main)
    }

    /// Borrow the first team matching a specific TeamType.
    pub fn by_type(&self, team_type: TeamType) -> Option<&Team> {
        self.teams.iter().find(|t| t.team_type == team_type)
    }

    /// Mutable variant of `by_type`.
    pub fn by_type_mut(&mut self, team_type: TeamType) -> Option<&mut Team> {
        self.teams.iter_mut().find(|t| t.team_type == team_type)
    }

    /// Array index of the first team matching a specific TeamType.
    pub fn index_of_type(&self, team_type: TeamType) -> Option<usize> {
        self.teams.iter().position(|t| t.team_type == team_type)
    }

    pub fn main_team_id(&self) -> Option<u32> {
        self.main().map(|t| t.id)
    }

    pub fn with_league(&self, league_id: u32) -> Vec<u32> {
        self.teams
            .iter()
            .filter(|t| t.league_id == Some(league_id))
            .map(|t| t.id)
            .collect()
    }

    /// Is a player with this id currently registered with any of the
    /// teams in this collection?
    pub fn contains_player(&self, player_id: u32) -> bool {
        self.teams.iter().any(|t| t.players.contains(player_id))
    }

    /// Find the team that currently holds a given player.
    pub fn find_team_with_player(&self, player_id: u32) -> Option<&Team> {
        self.teams.iter().find(|t| t.players.contains(player_id))
    }

    /// Mutable variant of `find_team_with_player`.
    pub fn find_team_with_player_mut(&mut self, player_id: u32) -> Option<&mut Team> {
        self.teams.iter_mut().find(|t| t.players.contains(player_id))
    }

    /// Index of the first reserve-tier team: prefers B-team, then
    /// Reserve, then the highest youth tier available. Use this instead
    /// of open-coding the fallback chain.
    pub fn reserve_index(&self) -> Option<usize> {
        self.find_reserve_team_index()
    }

    /// Index of a youth team (U18 preferred, then U19).
    pub fn youth_index(&self) -> Option<usize> {
        self.find_youth_team_index()
    }

    // ─── Coach state management ──────────────────────────────────────

    pub fn ensure_coach_state(&mut self, date: NaiveDate) {
        let main_team = match self.main() {
            Some(t) => t,
            None => return,
        };

        let head_coach = main_team.staffs.head_coach();
        let coach_id = head_coach.id;

        let previous_coach_id = self.coach_state.as_ref().map(|state| state.coach_id);
        let needs_rebuild = previous_coach_id
            .map(|pid| pid != coach_id)
            .unwrap_or(true);

        if needs_rebuild {
            self.coach_state = Some(CoachDecisionState::new(head_coach, date));

            // Manager-change shock: only fire when there actually was a
            // previous coach (not on first-ever initialization). Players
            // who had a strong bond with the outgoing coach take a hit;
            // those whose relationship had soured get a fresh-start bump.
            if let Some(prev_id) = previous_coach_id {
                Self::fire_manager_departure_events(&mut self.teams, prev_id);
            }
        }

        if let Some(ref mut state) = self.coach_state {
            state.current_week = date_to_week(date);
        }
    }

    fn fire_manager_departure_events(teams: &mut [Team], outgoing_coach_id: u32) {
        for team in teams.iter_mut() {
            if !matches!(team.team_type, TeamType::Main) {
                continue;
            }
            for player in team.players.players.iter_mut() {
                let magnitude = match player.relations.get_staff(outgoing_coach_id) {
                    Some(rel) => {
                        let bond = rel.personal_bond + rel.trust_in_abilities + rel.loyalty * 0.5;
                        if bond >= 150.0 {
                            -8.0
                        } else if bond >= 100.0 {
                            -4.0
                        } else if bond <= -50.0 {
                            3.0
                        } else if rel.authority_respect < 30.0 {
                            2.0
                        } else {
                            -1.0
                        }
                    }
                    None => -1.0,
                };
                player
                    .happiness
                    .add_event(HappinessEventType::ManagerDeparture, magnitude);
            }
        }
    }

    /// Updates impressions via Option::take(). Decays emotional heat once per cycle.
    pub fn update_all_impressions(&mut self, date: NaiveDate) {
        let mut state = match self.coach_state.take() {
            Some(s) => s,
            None => return,
        };

        for team in self.teams.iter() {
            for player in team.players.iter() {
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

        let main_idx = match self.main_index() {
            Some(idx) => idx,
            None => return Vec::new(),
        };

        let reserve_idx = self.find_reserve_team_index();
        let youth_idx = self.find_youth_team_index();

        let mut requests = Vec::new();

        // Squad composition (priority 0 — handles promotions, demotions, and swaps)
        {
            let (query, format) = SquadComposition::prepare_request(
                &self.teams, main_idx, reserve_idx, youth_idx, date,
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

    /// Proactively offer contract renewals to valuable players whose
    /// contracts are approaching expiry. Called monthly before the
    /// transfer listing AI so valuable players are locked in first.
    pub fn run_contract_renewals(&mut self, date: NaiveDate) {
        self.run_contract_renewals_with_budget(date, None, 5_000)
    }

    /// Variant aware of the chairman's wage budget and league reputation.
    /// Renewal offers will not collectively bust the budget and will
    /// scale with league prestige (Premier League pays more than Maltese
    /// Premier League at the same ability).
    pub fn run_contract_renewals_with_budget(
        &mut self,
        date: NaiveDate,
        wage_budget: Option<u32>,
        league_reputation: u16,
    ) {
        if self.teams.is_empty() {
            return;
        }
        let main_idx = match self.main_index() {
            Some(idx) => idx,
            None => return,
        };
        ContractRenewalManager::run_with_budget(
            &mut self.teams,
            main_idx,
            date,
            wage_budget,
            league_reputation,
        );
    }

    /// Daily critical squad moves: immediate demotions and ability-based swaps
    pub fn manage_critical_squad_moves(&mut self, date: NaiveDate) {
        if self.teams.len() < 2 {
            return;
        }
        let main_idx = match self.main_index() {
            Some(idx) => idx,
            None => return,
        };
        let reserve_idx = match self.reserve_index() {
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
            .or_else(|| self.teams.iter().position(|t| t.team_type == TeamType::Second))
            .or_else(|| self.teams.iter().position(|t| t.team_type == TeamType::Reserve))
            .or_else(|| self.teams.iter().position(|t| t.team_type == TeamType::U23))
            .or_else(|| self.teams.iter().position(|t| t.team_type == TeamType::U21))
            .or_else(|| self.teams.iter().position(|t| t.team_type == TeamType::U20))
            .or_else(|| self.teams.iter().position(|t| t.team_type == TeamType::U19))
            .or_else(|| self.teams.iter().position(|t| t.team_type == TeamType::U18))
    }

    fn find_youth_team_index(&self) -> Option<usize> {
        self.teams.iter().position(|t| t.team_type == TeamType::U18)
            .or_else(|| self.teams.iter().position(|t| t.team_type == TeamType::U19))
    }
}
