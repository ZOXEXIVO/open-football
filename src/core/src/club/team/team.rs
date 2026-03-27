use crate::club::team::behaviour::TeamBehaviour;
use crate::club::team::builder::TeamBuilder;
use crate::context::GlobalContext;
use crate::shared::CurrencyValue;
use chrono::Datelike;
use crate::{MatchHistory, MatchTacticType, Player, PlayerCollection, PlayerSquadStatus, StaffCollection, Tactics, TacticsSelector, TeamReputation, TeamResult, TeamTraining, TrainingSchedule, TransferItem, Transfers};
use std::borrow::Cow;
use std::fmt;
use std::str::FromStr;

#[derive(Debug, Clone)]
pub struct Team {
    pub id: u32,
    pub league_id: Option<u32>,
    pub club_id: u32,
    pub name: String,
    pub slug: String,

    pub team_type: TeamType,
    pub tactics: Option<Tactics>,

    pub players: PlayerCollection,
    pub staffs: StaffCollection,

    pub behaviour: TeamBehaviour,

    pub reputation: TeamReputation,
    pub training_schedule: TrainingSchedule,
    pub transfer_list: Transfers,
    pub match_history: MatchHistory,
}

impl Team {
    pub fn builder() -> TeamBuilder {
        TeamBuilder::new()
    }

    pub fn simulate(&mut self, ctx: GlobalContext<'_>) -> TeamResult {
        // Recalculate squad statuses monthly (1st of month)
        if ctx.simulation.date.day() == 1 {
            self.update_squad_statuses(ctx.simulation.date.date());
        }

        // Pass team reputation to players via context
        let player_ctx = ctx.with_team_reputation(self.id, self.reputation.overall_score());
        let result = TeamResult::new(
            self.id,
            self.players.simulate(player_ctx.with_player(None)),
            self.staffs.simulate(ctx.with_staff(None)),
            self.behaviour
                .simulate(&mut self.players, &mut self.staffs, ctx.with_team(self.id)),
            TeamTraining::train(self, ctx.simulation.date, ctx.club_facilities_training()),
        );

        if self.tactics.is_none() {
            self.tactics = Some(TacticsSelector::select(self, self.staffs.head_coach()));
        };

        if self.training_schedule.is_default {
            //let coach = self.staffs.head_coach();
        }

        result
    }

    /// FM-style: assign squad status based on CA rank within the team.
    fn update_squad_statuses(&mut self, date: chrono::NaiveDate) {
        // Collect sorted CAs (descending)
        let mut team_cas: Vec<u8> = self.players.players.iter()
            .map(|p| p.player_attributes.current_ability)
            .collect();
        team_cas.sort_unstable_by(|a, b| b.cmp(a));

        for player in &mut self.players.players {
            if let Some(ref mut contract) = player.contract {
                let age = crate::utils::DateUtils::age(player.birth_date, date);
                contract.squad_status = PlayerSquadStatus::calculate(
                    player.player_attributes.current_ability,
                    age,
                    &team_cas,
                );
            }
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

    pub fn get_annual_salary(&self) -> u32 {
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

    pub fn tactics(&self) -> Cow<'_, Tactics> {
        if let Some(tactics) = &self.tactics {
            Cow::Borrowed(tactics)
        } else {
            Cow::Owned(Tactics::new(MatchTacticType::T442))
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum TeamType {
    Main = 0,
    B = 1,
    Reserve = 2,
    U18 = 3,
    U19 = 4,
    U20 = 5,
    U21 = 6,
    U23 = 7,
}

impl TeamType {
    pub fn as_i18n_key(&self) -> &'static str {
        match self {
            TeamType::Main => "first_team",
            TeamType::B => "b_team",
            TeamType::Reserve => "reserve_team",
            TeamType::U18 => "under_18s",
            TeamType::U19 => "under_19s",
            TeamType::U20 => "under_20s",
            TeamType::U21 => "under_21s",
            TeamType::U23 => "under_23s",
        }
    }

    /// Maximum player age allowed on this team type (None = no limit)
    pub fn max_age(&self) -> Option<u8> {
        match self {
            TeamType::U18 => Some(18),
            TeamType::U19 => Some(19),
            _ => None,
        }
    }

    /// Youth team progression order: U18 → U19 → U20 → U21 → U23
    pub const YOUTH_PROGRESSION: &'static [TeamType] = &[
        TeamType::U18,
        TeamType::U19,
        TeamType::U20,
        TeamType::U21,
        TeamType::U23,
    ];
}

impl fmt::Display for TeamType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TeamType::Main => write!(f, "First team"),
            TeamType::B => write!(f, "B Team"),
            TeamType::Reserve => write!(f, "Reserve team"),
            TeamType::U18 => write!(f, "U18"),
            TeamType::U19 => write!(f, "U19"),
            TeamType::U20 => write!(f, "U20"),
            TeamType::U21 => write!(f, "U21"),
            TeamType::U23 => write!(f, "U23"),
        }
    }
}

impl FromStr for TeamType {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "Main" => Ok(TeamType::Main),
            "B" => Ok(TeamType::B),
            "Reserve" => Ok(TeamType::Reserve),
            "U18" => Ok(TeamType::U18),
            "U19" => Ok(TeamType::U19),
            "U20" => Ok(TeamType::U20),
            "U21" => Ok(TeamType::U21),
            "U23" => Ok(TeamType::U23),
            _ => Err(format!("'{}' is not a valid value for WSType", s)),
        }
    }
}
