use crate::club::player::behaviour_config::HappinessConfig;
use crate::club::player::mind::{ActorRef, EpisodeKind};
use crate::club::staff::mind::organs::judgements::CoachDecisionState;
use crate::club::staff::perception::{CoachProfile, date_to_week};
use crate::club::team::squad::SquadSatisfaction;
use crate::club::team::squad::{ContractRenewalManager, SquadManager};
use crate::context::GlobalContext;
use crate::utils::Logging;
use crate::{HappinessEventType, PlayerStatusType, Team, TeamResult, TeamType};
use chrono::NaiveDate;
use rayon::iter::IntoParallelRefMutIterator;
use rayon::iter::ParallelIterator;
use std::mem;
use std::slice::Iter;
use std::slice::IterMut;

#[derive(Debug, Clone)]
pub struct TeamCollection {
    pub teams: Vec<Team>,
    /// Who was in the head-coach seat the last time the club looked.
    ///
    /// The decision state itself lives on the man (S2 of
    /// `docs/staff_mind.md`); what stays here is the one genuinely
    /// club-side fact — who the club last had — because that is what
    /// tells it a manager has *changed* and the squad should react.
    previous_head_coach_id: Option<u32>,
}

impl TeamCollection {
    pub fn new(teams: Vec<Team>) -> Self {
        TeamCollection {
            teams,
            previous_head_coach_id: None,
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

    /// Which squad currently holds this player's registration. Callers that
    /// classify a player as a club ASSET need this: a squad label only
    /// carries a first-team promise when it was minted by the first team
    /// (see [`crate::PlayerSquadStatus::as_first_team_designation`]).
    /// Falls back to `Main` for an unknown id so a caller that has already
    /// resolved the player elsewhere keeps the pre-existing reading.
    pub fn squad_tier_of(&self, player_id: u32) -> TeamType {
        self.teams
            .iter()
            .find(|t| t.players.players.iter().any(|p| p.id == player_id))
            .map(|t| t.team_type)
            .unwrap_or(TeamType::Main)
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

    /// The head coach's decision state — his impressions of the squad
    /// and the three accumulators that go with them.
    ///
    /// It lives on the man (S2 of `docs/staff_mind.md`), so this walks
    /// the same manager → caretaker → assistant chain `head_coach()`
    /// does and returns `None` only when every seat is genuinely
    /// vacant. Read through this rather than reaching for the field:
    /// the S2 census in `staff/mind/organs/judgements/census.rs` guards
    /// it, and it is the one place that knows where the state now is.
    pub fn head_coach_decision_state(&self) -> Option<&CoachDecisionState> {
        let coach = self.main()?.staffs.head_coach();
        if coach.id == 0 {
            return None;
        }
        Some(&coach.decision_state)
    }

    /// Lift the head coach's state out for the duration of a pass that
    /// also needs the squads.
    ///
    /// The state lives *inside* `teams[main]` now, and every consumer
    /// wants the squads at the same time. Taking it out and putting it
    /// back is the only way to hold both — the same `Option::take`
    /// dance this collection did before the state moved, one level
    /// further down. Safe because no pass that borrows it moves staff
    /// between teams; only players move.
    fn take_coach_state(&mut self) -> Option<CoachDecisionState> {
        let coach = self.main_mut()?.staffs.head_coach_mut()?;
        if coach.id == 0 {
            return None;
        }
        Some(mem::take(&mut coach.decision_state))
    }

    /// Put back what [`Self::take_coach_state`] lifted out.
    fn restore_coach_state(&mut self, state: Option<CoachDecisionState>) {
        let Some(state) = state else {
            return;
        };
        let Some(coach) = self
            .main_mut()
            .and_then(|team| team.staffs.head_coach_mut())
        else {
            return;
        };
        coach.decision_state = state;
    }

    /// Returns `true` when a genuine manager CHANGE was detected this
    /// call (a previous coach existed and the head-coach id moved) — the
    /// club-level caller uses that to open the new manager's squad
    /// review window on the transfer plan.
    ///
    /// Binding is now the whole of the work. Before S2 this rebuilt the
    /// state from nothing at every change of manager; the state lives on
    /// the man, so a new arrival simply brings his own — including what
    /// he already thinks of any player he has coached before.
    pub fn ensure_coach_state(&mut self, date: NaiveDate) -> bool {
        let Some(main_team) = self.main() else {
            return false;
        };
        let head_coach = main_team.staffs.head_coach();
        let coach_id = head_coach.id;
        if coach_id == 0 {
            return false;
        }
        let profile = CoachProfile::from_staff(head_coach);
        let previously_bound = head_coach.decision_state.is_bound();
        let same_man = head_coach.decision_state.coach_id == coach_id;

        // A change of manager is detected against the man in the seat,
        // not against a club-side record: the seat changed if the coach
        // now standing in it is holding a state bound to someone else,
        // or none at all while the club has already run a coach before.
        let manager_changed = self.previous_head_coach_id.is_some_and(|id| id != coach_id);
        let previous_coach_id = self.previous_head_coach_id;
        self.previous_head_coach_id = Some(coach_id);

        if let Some(coach) = self.main_mut().and_then(|t| t.staffs.head_coach_mut()) {
            if !same_man || !previously_bound {
                coach.decision_state.bind(coach_id, profile, date);
            } else {
                coach.decision_state.current_week = date_to_week(date);
            }
        }

        if manager_changed {
            // Manager-change shock: only fire when there actually was a
            // previous coach (not on first-ever initialization). Players
            // who had a strong bond with the outgoing coach take a hit;
            // those whose relationship had soured get a fresh-start bump.
            // Then the whole squad feels the new-manager bounce.
            if let Some(prev_id) = previous_coach_id {
                Self::fire_manager_departure_events(&mut self.teams, prev_id, date);
                Self::fire_new_manager_bounce_events(&mut self.teams, coach_id, date);
            }
        }

        // Refresh the coach's squad-satisfaction read (size / performance /
        // quality spread / position coverage) — cheap, and it's the "how
        // complete is my squad" signal recruitment urgency consumes. Lift
        // the state out so the team can be read while the state is
        // written; they share an owner now.
        if let Some(idx) = self.main_index() {
            let mut state = self.take_coach_state();
            if let Some(state) = state.as_mut() {
                state.squad_satisfaction = SquadSatisfaction::compute(&self.teams[idx], state);
            }
            self.restore_coach_state(state);
        }
        manager_changed
    }

    fn fire_manager_departure_events(teams: &mut [Team], outgoing_coach_id: u32, date: NaiveDate) {
        for team in teams.iter_mut() {
            if !matches!(team.team_type, TeamType::Main) {
                continue;
            }
            let club_id = team.club_id;
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
                // And in memory, filed against the man rather than the
                // badge -- a grudge or a debt follows a coach to his
                // next job, and the club he leaves behind owns neither.
                let ctx = player.mind_context(date, Some(club_id));
                player.mind.remember(
                    EpisodeKind::ManagerLeftClub,
                    ActorRef::staff(outgoing_coach_id),
                    &ctx,
                );
            }
        }
    }

    /// The other half of a manager change: the squad-wide new-manager
    /// bounce. Everyone gets a small lift of fresh expectation; players
    /// the old regime had frozen out — low morale, formally unhappy, or
    /// club-listed — hope hardest, because the clean slate is real: the
    /// incoming coach brings his own judgement organ, so nothing the
    /// last man thought of them carries over.
    ///
    /// With one exception, and it is the right one: a coach who has
    /// worked with a player before arrives still holding that view.
    fn fire_new_manager_bounce_events(teams: &mut [Team], incoming_coach_id: u32, date: NaiveDate) {
        let base = HappinessConfig::default().catalog.new_manager_bounce;
        for team in teams.iter_mut() {
            if !matches!(team.team_type, TeamType::Main) {
                continue;
            }
            let club_id = team.club_id;
            for player in team.players.players.iter_mut() {
                let frozen_out = player.happiness.morale < 40.0
                    || player.statuses.has(PlayerStatusType::Unh)
                    || player
                        .contract
                        .as_ref()
                        .map(|c| c.is_transfer_listed)
                        .unwrap_or(false);
                let magnitude = if frozen_out { base * 1.8 } else { base };
                player
                    .happiness
                    .add_event(HappinessEventType::NewManagerBounce, magnitude);
                if incoming_coach_id != 0 {
                    let ctx = player.mind_context(date, Some(club_id));
                    player.mind.remember(
                        EpisodeKind::ManagerArrived,
                        ActorRef::staff(incoming_coach_id),
                        &ctx,
                    );
                }
            }
        }
    }

    /// Updates impressions via Option::take(). Decays emotional heat once per cycle.
    pub fn update_all_impressions(&mut self, date: NaiveDate) {
        let Some(mut state) = self.take_coach_state() else {
            return;
        };

        for team in self.teams.iter() {
            for player in team.players.iter() {
                state.update_impression(player, date, &team.team_type);
            }
        }

        // Decay emotional heat once per update cycle (not per player)
        state.emotional_heat *= 0.80;

        self.restore_coach_state(Some(state));
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
        // Main squad first, then the reserve / U21 squad, so a valuable
        // prospect or depth player housed there isn't left to run his deal
        // down to the single expiry-day panic offer (and lost on a Bosman).
        // Each squad negotiates against its own wage structure, but both
        // passes draw down ONE club-level wage budget — a single
        // `run_for_squads` call keeps the shared bill honest across them.
        let mut squad_indexes = vec![main_idx];
        if let Some(reserve_idx) = self.reserve_index() {
            if reserve_idx != main_idx {
                squad_indexes.push(reserve_idx);
            }
        }
        ContractRenewalManager::run_for_squads(
            &mut self.teams,
            &squad_indexes,
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

        let mut state = self.take_coach_state();
        SquadManager::manage_critical_moves(
            &mut self.teams,
            &mut state,
            main_idx,
            reserve_idx,
            date,
        );
        self.restore_coach_state(state);
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

    /// The same team with an empty dugout.
    fn main_team_without_staff(players: Vec<Player>) -> Team {
        let mut team = main_team(coach(1), players);
        team.staffs = StaffCollection::new(Vec::new());
        team
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

    #[test]
    fn a_coach_takes_his_impressions_with_him() {
        // The point of S2. A manager who has formed a view of a player
        // at one club still holds it at the next — which is how managers
        // sign the same players repeatedly, and what the club-side store
        // made impossible.
        let mut first_club = TeamCollection::new(vec![main_team(coach(1), vec![squad_player(7)])]);
        let start = NaiveDate::from_ymd_opt(2026, 6, 1).unwrap();
        for week in 0..6i64 {
            let date = start + chrono::Duration::days(week * 7);
            first_club.ensure_coach_state(date);
            first_club.update_all_impressions(date);
        }
        let formed = first_club
            .head_coach_decision_state()
            .expect("a bound coach has a state")
            .impressions
            .len();
        assert!(formed > 0, "he formed a view of the squad");

        // He is poached. The man moves; the state moves with him.
        let moving = first_club.teams[0]
            .staffs
            .iter()
            .find(|staff| staff.id == 1)
            .cloned()
            .expect("the coach is on the roster");

        let mut second_club = TeamCollection::new(vec![main_team(moving, vec![squad_player(7)])]);
        second_club.ensure_coach_state(NaiveDate::from_ymd_opt(2026, 8, 1).unwrap());

        let carried = second_club
            .head_coach_decision_state()
            .expect("he is bound at the new club too");
        assert_eq!(
            carried.impressions.len(),
            formed,
            "what he thinks of players is not the old club's property"
        );
        assert_eq!(carried.coach_id, 1, "and it is still his");
    }

    #[test]
    fn a_new_manager_inherits_nothing_from_his_predecessor() {
        // The other half of the same rule. The state travels with the
        // man, so the man who replaces him starts with his own — which
        // is what makes the new-manager bounce a real clean slate.
        let mut collection = TeamCollection::new(vec![main_team(coach(1), vec![squad_player(7)])]);
        let first = NaiveDate::from_ymd_opt(2026, 6, 1).unwrap();
        collection.ensure_coach_state(first);
        collection.update_all_impressions(first);
        assert!(
            !collection
                .head_coach_decision_state()
                .expect("bound")
                .impressions
                .is_empty()
        );

        collection.teams[0].staffs = StaffCollection::new(vec![coach(2)]);
        let next = NaiveDate::from_ymd_opt(2026, 6, 8).unwrap();
        assert!(collection.ensure_coach_state(next));

        let state = collection.head_coach_decision_state().expect("bound");
        assert_eq!(state.coach_id, 2, "the new man's own state is in play");
        assert!(
            state.impressions.is_empty(),
            "he has not inherited a predecessor's opinions"
        );
    }

    #[test]
    fn a_club_with_no_coach_at_all_has_no_decision_state() {
        // Before S2 a vacant club got a state bound to the internal stub
        // (id 0) and quietly accumulated pressure nobody was feeling.
        // Nobody in the dugout now means nobody deciding.
        let mut collection =
            TeamCollection::new(vec![main_team_without_staff(vec![squad_player(7)])]);
        let date = NaiveDate::from_ymd_opt(2026, 6, 1).unwrap();

        assert!(
            !collection.ensure_coach_state(date),
            "a vacant seat is not a manager change"
        );
        assert!(collection.head_coach_decision_state().is_none());
        // And the passes that read it are no-ops rather than panics.
        collection.update_all_impressions(date);
    }
}
