use crate::club::player::injury::InjuryType;
use crate::club::player::utils::PlayerUtils;
use crate::club::{
    PersonBehaviour, PlayerAttributes, PlayerClubContract, PlayerCollectionResult, PlayerMailbox,
    PlayerResult, PlayerSkills, PlayerStatusType, PlayerTraining, CONDITION_MAX_VALUE,
};
use crate::context::GlobalContext;
use crate::shared::fullname::FullName;
use crate::utils::{DateUtils, Logging};
use crate::{
    NegativeHappiness, Person, PersonAttributes, PlayerHappiness, PlayerPositionType,
    PlayerPositions, PlayerStatistics, PlayerStatisticsHistory, PlayerStatus, PlayerTrainingHistory,
    PlayerValueCalculator, PositiveHappiness, Relations,
};
use chrono::{NaiveDate, NaiveDateTime};
use std::fmt::{Display, Formatter, Result};
use std::ops::Index;
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
    pub statistics_history: PlayerStatisticsHistory,
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
        self.process_condition_recovery();

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

    /// Process injury lifecycle: recovery if injured, small random injury chance if not
    fn process_injury(&mut self, result: &mut PlayerResult, now: NaiveDate) {
        if self.player_attributes.is_injured {
            // Recover one day
            let recovered = self.player_attributes.recover_injury_day();

            if recovered {
                // Player recovered: remove Inj status, add Lmp (low match fitness)
                self.statuses.remove(PlayerStatusType::Inj);
                self.statuses.add(now, PlayerStatusType::Lmp);
                result.injury_recovered = true;
            }
        } else {
            // Small daily random injury chance (~0.05% base), modified by player factors
            let age = DateUtils::age(self.birth_date, now);
            let condition_pct = self.player_attributes.condition_percentage();
            let natural_fitness = self.skills.physical.natural_fitness;
            let jadedness = self.player_attributes.jadedness;

            // Base chance: 0.0005 (0.05%)
            let mut injury_chance: f32 = 0.0005;

            // Age modifier: players 30+ have higher risk
            if age > 30 {
                injury_chance += (age as f32 - 30.0) * 0.0002;
            }

            // Low condition increases risk
            if condition_pct < 50 {
                injury_chance += (50.0 - condition_pct as f32) * 0.00005;
            }

            // Low natural fitness increases risk
            if natural_fitness < 10.0 {
                injury_chance += (10.0 - natural_fitness) * 0.0001;
            }

            // Jadedness increases risk
            if jadedness > 5000 {
                injury_chance += (jadedness as f32 - 5000.0) * 0.00001;
            }

            if rand::random::<f32>() < injury_chance {
                let injury = InjuryType::random_spontaneous_injury();
                self.player_attributes.set_injury(injury);
                self.statuses.add(now, PlayerStatusType::Inj);
                result.injury_occurred = Some(injury);
            }
        }
    }

    /// Non-injured players slowly recover condition each day
    fn process_condition_recovery(&mut self) {
        if self.player_attributes.is_injured {
            return;
        }

        // Recovery rate based on natural_fitness: 50-200 per day (on 0-10000 scale)
        let natural_fitness = self.skills.physical.natural_fitness;
        let recovery = 50.0 + (natural_fitness / 20.0) * 150.0;
        let recovery = recovery as u16;

        if self.player_attributes.condition < CONDITION_MAX_VALUE {
            self.player_attributes.rest(recovery);
        }
    }

    /// Weekly happiness evaluation considering playing time, contract, team results
    fn process_happiness(&mut self, result: &mut PlayerResult, now: NaiveDate) {
        let age = DateUtils::age(self.birth_date, now);

        // Check playing time from statistics
        let matches_played = self.statistics.played;
        let total_possible_matches = self.statistics.played + self.statistics.played_subs;

        // Players who have a contract but aren't playing become unhappy
        if self.contract.is_some() && total_possible_matches > 5 {
            let play_ratio = if total_possible_matches > 0 {
                matches_played as f32 / total_possible_matches as f32
            } else {
                0.5 // Default if no matches yet
            };

            if play_ratio < 0.25 {
                // Playing less than 25% of possible matches
                self.happiness.add_negative(NegativeHappiness {
                    description: "Not getting enough playing time".to_string(),
                });

                // Prime-age players (24-30) are more sensitive to lack of playing time
                if age >= 24 && age <= 30 {
                    result.unhappy = true;
                    if !self.statuses.get().contains(&PlayerStatusType::Unh) {
                        self.statuses.add(now, PlayerStatusType::Unh);
                    }
                }
            } else if play_ratio > 0.60 {
                self.happiness.add_positive(PositiveHappiness {
                    description: "Getting regular playing time".to_string(),
                });
                // Remove Unh status if they're now happy about playing time
                self.statuses.remove(PlayerStatusType::Unh);
            }
        }

        // Contract satisfaction
        if let Some(ref contract) = self.contract {
            if contract.salary < 500 && age > 22 {
                self.happiness.add_negative(NegativeHappiness {
                    description: "Unhappy with contract terms".to_string(),
                });
            }
        }

        result.unhappy = !self.happiness.is_happy();
    }

    fn process_contract(&mut self, result: &mut PlayerResult, now: NaiveDateTime) {
        if let Some(ref mut contract) = self.contract {
            const HALF_YEAR_DAYS: i64 = 30 * 6;

            if contract.days_to_expiration(now) < HALF_YEAR_DAYS {
                result.contract.want_extend_contract = true;
            }
        } else {
            result.contract.no_contract = true;
        }
    }

    fn process_mailbox(&mut self, result: &mut PlayerResult, now: NaiveDate) {
        PlayerMailbox::process(self, result, now);
    }

    /// Transfer desire based on multiple factors, not just behaviour
    fn process_transfer_desire(&mut self, result: &mut PlayerResult, now: NaiveDate) {
        let mut wants_transfer = false;

        // Existing logic: poor behaviour
        if self.behaviour.is_poor() {
            wants_transfer = true;
        }

        // Unhappy for extended period (check if Unh status exists)
        let has_unh = self.statuses.statuses.iter().any(|s| {
            s.status == PlayerStatusType::Unh && (now - s.start_date).num_days() > 30
        });
        if has_unh {
            wants_transfer = true;
        }

        // Overall unhappiness
        if !self.happiness.is_happy() && self.behaviour.is_poor() {
            wants_transfer = true;
        }

        if wants_transfer {
            // Set Req (transfer request) status
            if !self.statuses.get().contains(&PlayerStatusType::Req) {
                self.statuses.add(now, PlayerStatusType::Req);
            }
            result.wants_to_leave = true;
            result.request_transfer(self.id);
        }
    }

    pub fn shirt_number(&self) -> u8 {
        if let Some(contract) = &self.contract {
            return contract.shirt_number.unwrap_or(0);
        }

        0
    }

    pub fn value(&self, date: NaiveDate) -> f64 {
        PlayerValueCalculator::calculate(self, date)
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

const DEFAULT_PLAYER_TRANSFER_BUFFER_SIZE: usize = 10;

#[derive(Debug)]
pub struct PlayerCollection {
    pub players: Vec<Player>,
}

impl PlayerCollection {
    pub fn new(players: Vec<Player>) -> Self {
        PlayerCollection { players }
    }

    pub fn simulate(&mut self, ctx: GlobalContext<'_>) -> PlayerCollectionResult {
        let player_results: Vec<PlayerResult> = self
            .players
            .iter_mut()
            .map(|player| {
                let message = &format!("simulate player: id: {}", player.id);
                Logging::estimate_result(
                    || player.simulate(ctx.with_player(Some(player.id))),
                    message,
                )
            })
            .collect();

        let mut outgoing_players = Vec::with_capacity(DEFAULT_PLAYER_TRANSFER_BUFFER_SIZE);

        for transfer_request_player_id in player_results.iter().flat_map(|p| &p.transfer_requests) {
            if let Some(player) = self.take_player(transfer_request_player_id) {
                outgoing_players.push(player)
            }
        }

        PlayerCollectionResult::new(player_results, outgoing_players)
    }

    pub fn by_position(&self, position: &PlayerPositionType) -> Vec<&Player> {
        self.players
            .iter()
            .filter(|p| p.positions().contains(position))
            .collect()
    }

    pub fn add(&mut self, player: Player) {
        self.players.push(player);
    }

    pub fn add_range(&mut self, players: Vec<Player>) {
        for player in players {
            self.players.push(player);
        }
    }

    pub fn get_week_salary(&self) -> u32 {
        self.players
            .iter()
            .filter_map(|p| p.contract.as_ref())
            .map(|c| c.salary)
            .sum::<u32>()
    }

    pub fn players(&self) -> Vec<&Player> {
        self.players.iter().map(|player| player).collect()
    }

    pub fn take_player(&mut self, player_id: &u32) -> Option<Player> {
        let player_idx = self.players.iter().position(|p| p.id == *player_id);
        match player_idx {
            Some(idx) => Some(self.players.remove(idx)),
            None => None,
        }
    }

    pub fn contains(&self, player_id: u32) -> bool {
        self.players.iter().any(|p| p.id == player_id)
    }
}

impl Index<u32> for PlayerCollection {
    type Output = Player;

    fn index(&self, player_id: u32) -> &Self::Output {
        self
            .players
            .iter()
            .find(|p| p.id == player_id)
            .expect(&format!("no player with id = {}", player_id))
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
