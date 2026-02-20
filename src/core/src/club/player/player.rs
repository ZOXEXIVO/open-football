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
    HappinessEventType, Person, PersonAttributes, PlayerHappiness, PlayerPositionType,
    PlayerPositions, PlayerSquadStatus, PlayerStatistics, PlayerStatisticsHistory,
    PlayerStatus, PlayerTrainingHistory,
    PlayerValueCalculator, Relations,
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

    /// Non-injured players slowly recover condition each day, with age and jadedness awareness
    fn process_condition_recovery(&mut self, now: NaiveDate) {
        if self.player_attributes.is_injured {
            return;
        }

        let natural_fitness = self.skills.physical.natural_fitness;
        let age = DateUtils::age(self.birth_date, now);
        let jadedness = self.player_attributes.jadedness;

        // Base recovery: 80-250 per day based on natural_fitness
        let base_recovery = 80.0 + (natural_fitness / 20.0) * 170.0;

        // Age penalty: older players recover slower
        let age_factor = if age > 30 {
            1.0 - (age as f32 - 30.0) * 0.06
        } else if age < 23 {
            1.1
        } else {
            1.0
        };

        // Jadedness penalty: jaded players recover slower
        let jadedness_factor = 1.0 - (jadedness as f32 / 10000.0) * 0.3;

        let recovery =
            (base_recovery * age_factor.max(0.5) * jadedness_factor.max(0.5)) as u16;

        if self.player_attributes.condition < CONDITION_MAX_VALUE {
            self.player_attributes.rest(recovery);
        }

        // Jadedness natural decay: -100/day when no match for 3+ days
        if self.player_attributes.days_since_last_match > 3 {
            self.player_attributes.jadedness =
                (self.player_attributes.jadedness - 100).max(0);
        }

        // Remove Rst status when jadedness drops below threshold
        if self.player_attributes.jadedness < 4000 {
            self.statuses.remove(PlayerStatusType::Rst);
        }

        // Increment days since last match
        self.player_attributes.days_since_last_match += 1;
    }

    /// Players not playing lose match sharpness over time
    fn process_match_readiness_decay(&mut self) {
        if self.player_attributes.days_since_last_match > 7 {
            // Accelerated decay after a week without matches
            self.skills.physical.match_readiness =
                (self.skills.physical.match_readiness - 0.15).max(0.0);
        } else if self.player_attributes.days_since_last_match > 3 {
            // Gradual decay
            self.skills.physical.match_readiness =
                (self.skills.physical.match_readiness - 0.08).max(0.0);
        }
        // No decay for first 3 days (normal rest period)
    }

    /// Weekly happiness evaluation with 6 real-world factors
    fn process_happiness(&mut self, result: &mut PlayerResult, now: NaiveDate) {
        let age = DateUtils::age(self.birth_date, now);
        let age_sensitivity = if age >= 24 && age <= 30 { 1.3 } else { 1.0 };

        // Decay old events weekly
        self.happiness.decay_events();

        // 1. Playing time vs squad status
        let playing_time_factor = self.calculate_playing_time_factor(age_sensitivity);
        self.happiness.factors.playing_time = playing_time_factor;

        // 2. Salary vs ability
        let salary_factor = self.calculate_salary_factor(age);
        self.happiness.factors.salary_satisfaction = salary_factor;

        // 3. Manager relationship
        let manager_factor = self.calculate_manager_relationship_factor();
        self.happiness.factors.manager_relationship = manager_factor;

        // 4. Injury frustration
        let injury_factor = self.calculate_injury_frustration();
        self.happiness.factors.injury_frustration = injury_factor;

        // 5. Ambition vs club level
        let ambition_factor = self.calculate_ambition_fit();
        self.happiness.factors.ambition_fit = ambition_factor;

        // 6. Praise/discipline from recent events (tracked separately)
        let praise: f32 = self.happiness.recent_events.iter()
            .filter(|e| e.event_type == HappinessEventType::ManagerPraise)
            .map(|e| e.magnitude * (1.0 - e.days_ago as f32 / 60.0).max(0.0))
            .sum();
        self.happiness.factors.recent_praise = praise.clamp(0.0, 10.0);

        let discipline: f32 = self.happiness.recent_events.iter()
            .filter(|e| e.event_type == HappinessEventType::ManagerDiscipline)
            .map(|e| e.magnitude * (1.0 - e.days_ago as f32 / 60.0).max(0.0))
            .sum();
        self.happiness.factors.recent_discipline = discipline.clamp(-10.0, 0.0);

        // Recalculate overall morale
        self.happiness.recalculate_morale();

        // Set Unh status if morale < 35
        if self.happiness.morale < 35.0 {
            if !self.statuses.get().contains(&PlayerStatusType::Unh) {
                self.statuses.add(now, PlayerStatusType::Unh);
            }
            result.unhappy = true;
        } else if self.happiness.morale > 50.0 {
            self.statuses.remove(PlayerStatusType::Unh);
            result.unhappy = false;
        } else {
            result.unhappy = !self.happiness.is_happy();
        }
    }

    fn calculate_playing_time_factor(&self, age_sensitivity: f32) -> f32 {
        let total = self.statistics.played + self.statistics.played_subs;
        if total < 5 {
            return 0.0;
        }

        let play_ratio = self.statistics.played as f32 / total as f32;

        let (expected_ratio, unhappy_threshold) = if let Some(ref contract) = self.contract {
            match contract.squad_status {
                PlayerSquadStatus::KeyPlayer => (0.70, 0.50),
                PlayerSquadStatus::FirstTeamRegular => (0.50, 0.30),
                PlayerSquadStatus::FirstTeamSquadRotation => (0.25, 0.15),
                PlayerSquadStatus::MainBackupPlayer => (0.20, 0.10),
                PlayerSquadStatus::HotProspectForTheFuture => (0.10, 0.05),
                PlayerSquadStatus::DecentYoungster => (0.10, 0.05),
                PlayerSquadStatus::NotNeeded => (0.05, 0.0),
                _ => (0.30, 0.15),
            }
        } else {
            (0.30, 0.15)
        };

        let factor = if play_ratio >= expected_ratio {
            // Meeting or exceeding expectations
            let excess = (play_ratio - expected_ratio) / (1.0 - expected_ratio).max(0.01);
            excess * 20.0
        } else if play_ratio < unhappy_threshold {
            // Below unhappy threshold
            let deficit = (unhappy_threshold - play_ratio) / unhappy_threshold.max(0.01);
            -deficit * 20.0 * age_sensitivity
        } else {
            // Between unhappy and expected - mild dissatisfaction
            let range = expected_ratio - unhappy_threshold;
            let position = (play_ratio - unhappy_threshold) / range.max(0.01);
            (position - 0.5) * 10.0
        };

        factor.clamp(-20.0, 20.0)
    }

    fn calculate_salary_factor(&self, age: u8) -> f32 {
        if let Some(ref contract) = self.contract {
            // Expected salary based on ability level (rough scaling)
            let ability = self.player_attributes.current_ability as f32;
            let expected_base = ability * ability * 0.5; // quadratic scaling
            let age_factor = if age < 22 { 0.6 } else if age > 30 { 0.85 } else { 1.0 };
            let expected = expected_base * age_factor;

            if expected < 1.0 {
                return 0.0;
            }

            let ratio = contract.salary as f32 / expected;
            if ratio >= 1.2 {
                // Well paid
                10.0_f32.min(ratio * 5.0)
            } else if ratio >= 0.8 {
                // Fairly paid
                (ratio - 0.8) * 25.0 // 0 to 10
            } else {
                // Underpaid
                (ratio - 0.8) * 37.5 // -30 to 0, clamped
            }
        } else {
            -5.0 // No contract is slightly negative
        }
        .clamp(-15.0, 15.0)
    }

    fn calculate_manager_relationship_factor(&self) -> f32 {
        // This factor is primarily driven by manager talks (Area 3)
        // which update it via happiness.factors.manager_relationship directly.
        // Return the current stored value.
        self.happiness.factors.manager_relationship
    }

    fn calculate_injury_frustration(&self) -> f32 {
        if !self.player_attributes.is_injured {
            return 0.0;
        }

        let injury_days = self.player_attributes.injury_days_remaining as f32;
        if injury_days <= 14.0 {
            return -2.0;
        }

        // Longer injuries cause more frustration: -5 to -10
        let severity = ((injury_days - 14.0) / 60.0).min(1.0);
        -(5.0 + severity * 5.0)
    }

    fn calculate_ambition_fit(&self) -> f32 {
        // Compare player ambition against their current reputation as proxy for club level
        // High ambition (>15) at a low-rep situation creates unhappiness
        let ambition = self.attributes.ambition;
        let rep = self.player_attributes.current_reputation as f32;

        if ambition <= 10.0 {
            return 0.0; // Low ambition players don't care much
        }

        // Ambition expects a certain reputation level
        // ambition 20 expects rep ~8000+, ambition 15 expects ~4000+
        let expected_rep = (ambition - 10.0) * 800.0;

        if rep >= expected_rep {
            // At or above expected level
            let excess = ((rep - expected_rep) / 2000.0).min(1.0);
            excess * 5.0
        } else {
            // Below expected level
            let deficit = ((expected_rep - rep) / expected_rep).min(1.0);
            -deficit * 10.0
        }
        .clamp(-10.0, 10.0)
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
        // Under-16 players cannot request transfers — only free release
        let age = DateUtils::age(self.birth_date, now);
        if age < 16 {
            return;
        }

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
