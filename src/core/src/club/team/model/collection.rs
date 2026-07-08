use crate::club::player::behaviour_config::HappinessConfig;
use crate::club::staff::perception::{CoachDecisionState, date_to_week};
use crate::club::team::squad::SquadSatisfaction;
use crate::club::team::squad::{ContractRenewalManager, SquadManager};
use crate::context::GlobalContext;
use crate::utils::Logging;
use crate::{HappinessEventType, PlayerStatusType, Team, TeamResult, TeamType};
use chrono::NaiveDate;
use rayon::iter::IntoParallelRefMutIterator;
use rayon::iter::ParallelIterator;
use std::slice::Iter;
use std::slice::IterMut;

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

    pub fn iter(&self) -> Iter<'_, Team> {
        self.teams.iter()
    }

    pub fn iter_mut(&mut self) -> IterMut<'_, Team> {
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
        self.teams
            .iter_mut()
            .find(|t| t.team_type == TeamType::Main)
    }

    /// Array index of the main team, if any.
    pub fn main_index(&self) -> Option<usize> {
        self.teams
            .iter()
            .position(|t| t.team_type == TeamType::Main)
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
        self.teams
            .iter_mut()
            .find(|t| t.players.contains(player_id))
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

    /// Returns `true` when a genuine manager CHANGE was detected this
    /// call (a previous coach existed and the head-coach id moved) — the
    /// club-level caller uses that to open the new manager's squad
    /// review window on the transfer plan.
    pub fn ensure_coach_state(&mut self, date: NaiveDate) -> bool {
        let main_team = match self.main() {
            Some(t) => t,
            None => return false,
        };

        let head_coach = main_team.staffs.head_coach();
        let coach_id = head_coach.id;

        let previous_coach_id = self.coach_state.as_ref().map(|state| state.coach_id);
        let needs_rebuild = previous_coach_id.map(|pid| pid != coach_id).unwrap_or(true);

        let mut manager_changed = false;
        if needs_rebuild {
            self.coach_state = Some(CoachDecisionState::new(head_coach, date));

            // Manager-change shock: only fire when there actually was a
            // previous coach (not on first-ever initialization). Players
            // who had a strong bond with the outgoing coach take a hit;
            // those whose relationship had soured get a fresh-start bump.
            // Then the whole squad feels the new-manager bounce.
            if let Some(prev_id) = previous_coach_id {
                Self::fire_manager_departure_events(&mut self.teams, prev_id);
                Self::fire_new_manager_bounce_events(&mut self.teams);
                manager_changed = true;
            }
        }

        if let Some(ref mut state) = self.coach_state {
            state.current_week = date_to_week(date);
        }

        // Refresh the coach's squad-satisfaction read (size / performance /
        // quality spread / position coverage) — cheap, and it's the "how
        // complete is my squad" signal recruitment urgency consumes. Split
        // borrow: the team is read-only, the state is written.
        if let Some(idx) = self.main_index() {
            let sat = self
                .coach_state
                .as_ref()
                .map(|state| SquadSatisfaction::compute(&self.teams[idx], state));
            if let (Some(sat), Some(state)) = (sat, self.coach_state.as_mut()) {
                state.squad_satisfaction = sat;
            }
        }
        manager_changed
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

    /// The other half of a manager change: the squad-wide new-manager
    /// bounce. Everyone gets a small lift of fresh expectation; players
    /// the old regime had frozen out — low morale, formally unhappy, or
    /// club-listed — hope hardest, because the clean slate is real: the
    /// new coach's selection memory starts empty.
    fn fire_new_manager_bounce_events(teams: &mut [Team]) {
        let base = HappinessConfig::default().catalog.new_manager_bounce;
        for team in teams.iter_mut() {
            if !matches!(team.team_type, TeamType::Main) {
                continue;
            }
            for player in team.players.players.iter_mut() {
                let statuses = player.statuses.get();
                let frozen_out = player.happiness.morale < 40.0
                    || statuses.contains(&PlayerStatusType::Unh)
                    || player
                        .contract
                        .as_ref()
                        .map(|c| c.is_transfer_listed)
                        .unwrap_or(false);
                let magnitude = if frozen_out { base * 1.8 } else { base };
                player
                    .happiness
                    .add_event(HappinessEventType::NewManagerBounce, magnitude);
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
        // Also open early talks with the reserve / U21 squad, so a valuable
        // prospect or depth player housed there isn't left to run his deal
        // down to the single expiry-day panic offer (and lost on a Bosman).
        // Runs against that squad's own wage structure and the shared club
        // budget.
        if let Some(reserve_idx) = self.reserve_index() {
            if reserve_idx != main_idx {
                ContractRenewalManager::run_with_budget(
                    &mut self.teams,
                    reserve_idx,
                    date,
                    wage_budget,
                    league_reputation,
                );
            }
        }
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
        self.teams
            .iter()
            .position(|t| t.team_type == TeamType::B)
            .or_else(|| {
                self.teams
                    .iter()
                    .position(|t| t.team_type == TeamType::Second)
            })
            .or_else(|| {
                self.teams
                    .iter()
                    .position(|t| t.team_type == TeamType::Reserve)
            })
            .or_else(|| self.teams.iter().position(|t| t.team_type == TeamType::U23))
            .or_else(|| self.teams.iter().position(|t| t.team_type == TeamType::U21))
            .or_else(|| self.teams.iter().position(|t| t.team_type == TeamType::U20))
            .or_else(|| self.teams.iter().position(|t| t.team_type == TeamType::U19))
            .or_else(|| self.teams.iter().position(|t| t.team_type == TeamType::U18))
    }

    fn find_youth_team_index(&self) -> Option<usize> {
        self.teams
            .iter()
            .position(|t| t.team_type == TeamType::U18)
            .or_else(|| self.teams.iter().position(|t| t.team_type == TeamType::U19))
    }
}

#[cfg(test)]
mod coach_change_tests {
    //! The manager-change arc: swapping the head coach fires the
    //! loyalists' `ManagerDeparture` AND the squad-wide
    //! `NewManagerBounce`, and reports the change to the club level so
    //! the transfer plan can open the new manager's review window.
    use super::*;
    use crate::club::StaffStub;
    use crate::club::player::builder::PlayerBuilder;
    use crate::club::staff::{StaffClubContract, StaffPosition, StaffStatus};
    use crate::shared::fullname::FullName;
    use crate::{
        PersonAttributes, Player, PlayerAttributes, PlayerCollection, PlayerPosition,
        PlayerPositionType, PlayerPositions, PlayerSkills, StaffCollection, Team, TeamBuilder,
        TeamReputation, TrainingSchedule,
    };
    use chrono::NaiveTime;

    fn coach(id: u32) -> crate::Staff {
        let mut staff = StaffStub::default();
        staff.id = id;
        staff.contract = Some(StaffClubContract::new(
            50_000,
            NaiveDate::from_ymd_opt(2030, 6, 30).unwrap(),
            StaffPosition::Manager,
            StaffStatus::Active,
        ));
        staff
    }

    fn squad_player(id: u32) -> Player {
        PlayerBuilder::new()
            .id(id)
            .full_name(FullName::new("T".into(), id.to_string()))
            .birth_date(NaiveDate::from_ymd_opt(1998, 1, 1).unwrap())
            .country_id(1)
            .attributes(PersonAttributes::default())
            .skills(PlayerSkills::default())
            .positions(PlayerPositions {
                positions: vec![PlayerPosition {
                    position: PlayerPositionType::Striker,
                    level: 20,
                }],
            })
            .player_attributes(PlayerAttributes::default())
            .build()
            .unwrap()
    }

    fn main_team(head: crate::Staff, players: Vec<Player>) -> Team {
        TeamBuilder::new()
            .id(1)
            .league_id(Some(1))
            .club_id(1)
            .name("Main".to_string())
            .slug("main".to_string())
            .team_type(TeamType::Main)
            .players(PlayerCollection::new(players))
            .staffs(StaffCollection::new(vec![head]))
            .reputation(TeamReputation::new(100, 100, 200))
            .training_schedule(TrainingSchedule::new(
                NaiveTime::from_hms_opt(9, 0, 0).unwrap(),
                NaiveTime::from_hms_opt(15, 0, 0).unwrap(),
            ))
            .build()
            .unwrap()
    }

    fn count(player: &Player, kind: HappinessEventType) -> usize {
        player
            .happiness
            .recent_events
            .iter()
            .filter(|e| e.event_type == kind)
            .count()
    }

    #[test]
    fn manager_change_fires_departure_and_bounce() {
        let mut collection = TeamCollection::new(vec![main_team(coach(1), vec![squad_player(7)])]);
        let first = NaiveDate::from_ymd_opt(2026, 6, 1).unwrap();
        assert!(
            !collection.ensure_coach_state(first),
            "first-ever initialization is not a manager change"
        );

        // The board replaces the head coach between ticks.
        collection.teams[0].staffs = StaffCollection::new(vec![coach(2)]);
        let next = NaiveDate::from_ymd_opt(2026, 6, 8).unwrap();
        assert!(
            collection.ensure_coach_state(next),
            "a head-coach id change must be reported to the club level"
        );

        let p = &collection.teams[0].players.players[0];
        assert_eq!(
            count(p, HappinessEventType::ManagerDeparture),
            1,
            "the outgoing coach's departure lands on the squad"
        );
        assert_eq!(
            count(p, HappinessEventType::NewManagerBounce),
            1,
            "the new-manager bounce lands on the squad"
        );
    }
}
