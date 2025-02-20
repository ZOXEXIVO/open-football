use crate::club::team::behaviour::TeamBehaviour;
use crate::context::GlobalContext;
use crate::r#match::{SquadSelector, TeamSquad};
use crate::shared::CurrencyValue;
use crate::{
    MatchHistory, Player, PlayerCollection, StaffCollection, Tactics, MatchTacticType,
    TacticsSelector, TeamReputation, TeamResult, TeamTraining, TrainingSchedule, TransferItem,
    Transfers,
};
use std::borrow::Cow;
use std::str::FromStr;

#[derive(Debug, PartialEq)]
pub enum TeamType {
    Main = 0,
    B = 1,
    U18 = 2,
    U19 = 3,
    U21 = 4,
    U23 = 5,
}

#[derive(Debug)]
pub struct Team {
    pub id: u32,
    pub league_id: u32,
    pub club_id: u32,
    pub name: String,
    pub slug: String,

    pub team_type: TeamType,
    pub tactics: Option<Tactics>,

    pub players: PlayerCollection,
    pub staffs: StaffCollection,

    pub reputation: TeamReputation,
    pub training_schedule: TrainingSchedule,
    pub transfer_list: Transfers,
    pub match_history: MatchHistory,
}

impl Team {
    pub fn new(
        id: u32,
        league_id: u32,
        club_id: u32,
        name: String,
        slug: String,
        team_type: TeamType,
        training_schedule: TrainingSchedule,
        reputation: TeamReputation,
        players: PlayerCollection,
        staffs: StaffCollection,
    ) -> Self {
        Team {
            id,
            league_id,
            club_id,
            name,
            slug,
            team_type,
            players,
            staffs,
            reputation,
            tactics: None,
            training_schedule,
            transfer_list: Transfers::new(),
            match_history: MatchHistory::new(),
        }
    }

    pub fn players(&self) -> Vec<&Player> {
        self.players.players()
    }

    pub fn add_player_to_transfer_list(&mut self, player_id: u32, value: CurrencyValue) {
        self.transfer_list.add(TransferItem {
            player_id,
            amount: value,
        })
    }

    pub fn get_week_salary(&self) -> u32 {
        self.players
            .players
            .iter()
            .filter_map(|p| p.contract.as_ref())
            .map(|c| c.salary)
            .chain(
                self.staffs
                    .staffs
                    .iter()
                    .filter_map(|p| p.contract.as_ref())
                    .map(|c| c.salary),
            )
            .sum()
    }

    pub fn get_match_squad(&self) -> TeamSquad {
        let head_coach = self.staffs.head_coach();

        let squad = SquadSelector::select(self, head_coach);

        TeamSquad {
            team_id: self.id,
            team_name: self.name.clone(),
            tactics: TacticsSelector::select(self, head_coach),
            main_squad: squad.main_squad,
            substitutes: squad.substitutes,
        }
    }

    pub fn tactics(&self) -> Cow<Tactics> {
        if let Some(tactics) = &self.tactics {
            Cow::Borrowed(tactics)
        } else {
            Cow::Owned(Tactics::new(MatchTacticType::T442))
        }
    }

    pub fn simulate(&mut self, ctx: GlobalContext<'_>) -> TeamResult {
        let result = TeamResult::new(
            self.id,
            self.players.simulate(ctx.with_player(None)),
            self.staffs.simulate(ctx.with_staff(None)),
            TeamBehaviour::simulate(&mut self.players, &mut self.staffs),
            TeamTraining::train(self, ctx.simulation.date),
        );

        if self.tactics.is_none() {
            self.tactics = Some(TacticsSelector::select(self, self.staffs.head_coach()));
        };

        if self.training_schedule.is_default {
            //let coach = self.staffs.head_coach();
        }

        result
    }
}

impl FromStr for TeamType {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "Main" => Ok(TeamType::Main),
            "B" => Ok(TeamType::B),
            "U18" => Ok(TeamType::U18),
            "U19" => Ok(TeamType::U19),
            "U21" => Ok(TeamType::U21),
            "U23" => Ok(TeamType::U23),
            _ => Err(format!("'{}' is not a valid value for WSType", s)),
        }
    }
}
