use crate::club::player::utils::PlayerUtils;
use crate::club::{
    PersonBehaviour, PlayerAttributes, PlayerClubContract, PlayerMailbox,
    PlayerResult, PlayerSkills, PlayerTraining,
};
use crate::context::GlobalContext;
use crate::shared::fullname::FullName;
use crate::utils::DateUtils;
use crate::{
    Person, PersonAttributes, PlayerDecisionHistory, PlayerHappiness,
    PlayerPositionType, PlayerPositions,
    PlayerStatistics, PlayerStatisticsHistory,
    PlayerStatus, PlayerTrainingHistory, PlayerValueCalculator, Relations,
};
use chrono::NaiveDate;
use std::fmt::{Display, Formatter, Result};
use crate::club::player::builder::PlayerBuilder;

#[derive(Debug)]
pub struct Player {
    //person data
    pub id: u32,
    pub full_name: FullName,
    pub birth_date: NaiveDate,
    pub country_id: u32,
    pub behaviour: PersonBehaviour,
    pub attributes: PersonAttributes,

    //player data
    pub happiness: PlayerHappiness,
    pub statuses: PlayerStatus,
    pub skills: PlayerSkills,
    pub contract: Option<PlayerClubContract>,
    pub positions: PlayerPositions,
    pub preferred_foot: PlayerPreferredFoot,
    pub player_attributes: PlayerAttributes,
    pub mailbox: PlayerMailbox,
    pub training: PlayerTraining,
    pub training_history: PlayerTrainingHistory,
    pub relations: Relations,

    pub statistics: PlayerStatistics,
    pub friendly_statistics: PlayerStatistics,
    pub statistics_history: PlayerStatisticsHistory,
    pub decision_history: PlayerDecisionHistory,
}

impl Player {
    pub fn builder() -> PlayerBuilder {
        PlayerBuilder::new()
    }

    pub fn simulate(&mut self, ctx: GlobalContext<'_>) -> PlayerResult {
        let now = ctx.simulation.date;

        let mut result = PlayerResult::new(self.id);

        // Birthday
        if DateUtils::is_birthday(self.birth_date, now.date()) {
            self.behaviour.try_increase();
        }

        // Injury recovery (daily)
        self.process_injury(&mut result, now.date());

        // Natural condition recovery for non-injured players
        self.process_condition_recovery(now.date());

        // Match readiness decay for players not playing
        self.process_match_readiness_decay();

        // Player happiness & morale evaluation (weekly)
        if ctx.simulation.is_week_beginning() {
            self.process_happiness(&mut result, now.date());
        }

        // Contract processing
        self.process_contract(&mut result, now);
        self.process_mailbox(&mut result, now.date());

        // Transfer desire based on multiple factors
        self.process_transfer_desire(&mut result, now.date());

        result
    }

    pub fn shirt_number(&self) -> u8 {
        if let Some(contract) = &self.contract {
            return contract.shirt_number.unwrap_or(0);
        }

        0
    }

    pub fn value(&self, date: NaiveDate) -> f64 {
        PlayerValueCalculator::calculate(self, date, 1.0)
    }

    pub fn value_with_price_level(&self, date: NaiveDate, price_level: f32) -> f64 {
        PlayerValueCalculator::calculate(self, date, price_level)
    }

    #[inline]
    pub fn positions(&self) -> Vec<PlayerPositionType> {
        self.positions.positions()
    }

    #[inline]
    pub fn position(&self) -> PlayerPositionType {
        *self
            .positions
            .positions()
            .first()
            .expect("no position found")
    }

    pub fn preferred_foot_str(&self) -> &'static str {
        match self.preferred_foot {
            PlayerPreferredFoot::Left => "Left",
            PlayerPreferredFoot::Right => "Right",
            PlayerPreferredFoot::Both => "Both",
        }
    }

    pub fn is_ready_for_match(&self) -> bool {
        !self.player_attributes.is_injured
            && !self.player_attributes.is_banned
            && !self.player_attributes.is_in_recovery()
            && self.player_attributes.condition_percentage() > 50
    }

    pub fn growth_potential(&self, now: NaiveDate) -> u8 {
        PlayerUtils::growth_potential(self, now)
    }
}

impl Person for Player {
    fn id(&self) -> u32 {
        self.id
    }

    fn fullname(&self) -> &FullName {
        &self.full_name
    }

    fn birthday(&self) -> NaiveDate {
        self.birth_date
    }

    fn behaviour(&self) -> &PersonBehaviour {
        &self.behaviour
    }

    fn attributes(&self) -> &PersonAttributes {
        &self.attributes
    }

    fn relations(&self) -> &Relations {
        &self.relations
    }
}

#[derive(Debug)]
pub enum PlayerPreferredFoot {
    Left,
    Right,
    Both,
}

//DISPLAY
impl Display for Player {
    fn fmt(&self, f: &mut Formatter<'_>) -> Result {
        write!(f, "{}, {}", self.full_name, self.birth_date)
    }
}

impl PartialEq for Player {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn player_is_correct() {
        assert_eq!(10, 10);
    }
}
