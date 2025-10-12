use crate::club::{
    PersonBehaviour, PlayerAttributes, PlayerClubContract, PlayerMailbox,
    PlayerSkills, PlayerTraining,
};
use crate::shared::fullname::FullName;
use crate::{PersonAttributes, Player, PlayerHappiness, PlayerPositions, PlayerPreferredFoot, PlayerStatistics, PlayerStatisticsHistory, PlayerStatus, PlayerTrainingHistory, Relations};
use chrono::NaiveDate;

// Builder for Player
#[derive(Default)]
pub struct PlayerBuilder {
    id: Option<u32>,
    full_name: Option<FullName>,
    birth_date: Option<NaiveDate>,
    country_id: Option<u32>,
    behaviour: Option<PersonBehaviour>,
    attributes: Option<PersonAttributes>,
    happiness: Option<PlayerHappiness>,
    statuses: Option<PlayerStatus>,
    skills: Option<PlayerSkills>,
    contract: Option<Option<PlayerClubContract>>,
    positions: Option<PlayerPositions>,
    preferred_foot: Option<PlayerPreferredFoot>,
    player_attributes: Option<PlayerAttributes>,
    mailbox: Option<PlayerMailbox>,
    training: Option<PlayerTraining>,
    training_history: Option<PlayerTrainingHistory>,
    relations: Option<Relations>,
    statistics: Option<PlayerStatistics>,
    statistics_history: Option<PlayerStatisticsHistory>,
}

impl PlayerBuilder {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn id(mut self, id: u32) -> Self {
        self.id = Some(id);
        self
    }

    pub fn full_name(mut self, full_name: FullName) -> Self {
        self.full_name = Some(full_name);
        self
    }

    pub fn birth_date(mut self, birth_date: NaiveDate) -> Self {
        self.birth_date = Some(birth_date);
        self
    }

    pub fn country_id(mut self, country_id: u32) -> Self {
        self.country_id = Some(country_id);
        self
    }

    pub fn behaviour(mut self, behaviour: PersonBehaviour) -> Self {
        self.behaviour = Some(behaviour);
        self
    }

    pub fn attributes(mut self, attributes: PersonAttributes) -> Self {
        self.attributes = Some(attributes);
        self
    }

    pub fn happiness(mut self, happiness: PlayerHappiness) -> Self {
        self.happiness = Some(happiness);
        self
    }

    pub fn statuses(mut self, statuses: PlayerStatus) -> Self {
        self.statuses = Some(statuses);
        self
    }

    pub fn skills(mut self, skills: PlayerSkills) -> Self {
        self.skills = Some(skills);
        self
    }

    pub fn contract(mut self, contract: Option<PlayerClubContract>) -> Self {
        self.contract = Some(contract);
        self
    }

    pub fn positions(mut self, positions: PlayerPositions) -> Self {
        self.positions = Some(positions);
        self
    }

    pub fn preferred_foot(mut self, preferred_foot: PlayerPreferredFoot) -> Self {
        self.preferred_foot = Some(preferred_foot);
        self
    }

    pub fn player_attributes(mut self, player_attributes: PlayerAttributes) -> Self {
        self.player_attributes = Some(player_attributes);
        self
    }

    pub fn mailbox(mut self, mailbox: PlayerMailbox) -> Self {
        self.mailbox = Some(mailbox);
        self
    }

    pub fn training(mut self, training: PlayerTraining) -> Self {
        self.training = Some(training);
        self
    }

    pub fn training_history(mut self, training_history: PlayerTrainingHistory) -> Self {
        self.training_history = Some(training_history);
        self
    }

    pub fn relations(mut self, relations: Relations) -> Self {
        self.relations = Some(relations);
        self
    }

    pub fn statistics(mut self, statistics: PlayerStatistics) -> Self {
        self.statistics = Some(statistics);
        self
    }

    pub fn statistics_history(mut self, statistics_history: PlayerStatisticsHistory) -> Self {
        self.statistics_history = Some(statistics_history);
        self
    }

    pub fn build(self) -> Result<Player, String> {
        Ok(Player {
            id: self.id.ok_or("id is required")?,
            full_name: self.full_name.ok_or("full_name is required")?,
            birth_date: self.birth_date.ok_or("birth_date is required")?,
            country_id: self.country_id.ok_or("country_id is required")?,
            behaviour: self.behaviour.unwrap_or_default(),
            attributes: self.attributes.ok_or("attributes is required")?,
            happiness: self.happiness.unwrap_or_else(PlayerHappiness::new),
            statuses: self.statuses.unwrap_or_else(PlayerStatus::new),
            skills: self.skills.ok_or("skills is required")?,
            contract: self.contract.unwrap_or(None),
            positions: self.positions.ok_or("positions is required")?,
            preferred_foot: self.preferred_foot.unwrap_or(PlayerPreferredFoot::Right),
            player_attributes: self.player_attributes.ok_or("player_attributes is required")?,
            mailbox: self.mailbox.unwrap_or_else(PlayerMailbox::new),
            training: self.training.unwrap_or_else(PlayerTraining::new),
            training_history: self.training_history.unwrap_or_else(PlayerTrainingHistory::new),
            relations: self.relations.unwrap_or_else(Relations::new),
            statistics: self.statistics.unwrap_or_default(),
            statistics_history: self.statistics_history.unwrap_or_else(PlayerStatisticsHistory::new),
        })
    }
}
