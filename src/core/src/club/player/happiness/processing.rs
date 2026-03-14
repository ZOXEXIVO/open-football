use crate::club::player::player::Player;
use crate::club::{PlayerResult, PlayerStatusType};
use crate::utils::DateUtils;
use crate::{ContractType, HappinessEventType, PlayerSquadStatus};
use chrono::NaiveDate;

impl Player {
    /// Weekly happiness evaluation with 6 real-world factors
    pub(crate) fn process_happiness(&mut self, result: &mut PlayerResult, now: NaiveDate, team_reputation: f32) {
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
        let ambition_factor = self.calculate_ambition_fit(team_reputation);
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

        // Salary unhappy: player wants contract renegotiation (with 90-day cooldown)
        // After 2 failed requests (180+ days unhappy), player accepts situation
        // and salary frustration dampens — prevents permanent unhappiness loops
        if salary_factor <= -5.0 {
            let days_since_first_request = self.happiness.last_salary_negotiation
                .map(|d| (now - d).num_days())
                .unwrap_or(0);

            if days_since_first_request > 180 {
                // Player gives up on salary demands — dampen frustration
                self.happiness.factors.salary_satisfaction =
                    (self.happiness.factors.salary_satisfaction * 0.5).clamp(-5.0, 0.0);
            } else {
                let cooldown_passed = self.happiness.last_salary_negotiation
                    .map(|d| (now - d).num_days() >= 90)
                    .unwrap_or(true);

                if cooldown_passed {
                    result.contract.want_improve_contract = true;
                    if self.happiness.last_salary_negotiation.is_none() {
                        self.happiness.last_salary_negotiation = Some(now);
                    }
                }
            }
        } else {
            // Salary is acceptable now — reset negotiation tracking
            self.happiness.last_salary_negotiation = None;
        }

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

        // Only skilled players care strongly about playing time.
        // Low-ability players (bench warmers) accept their role more easily.
        let ability = self.player_attributes.current_ability as f32;
        // ability_factor: 0.0 at ability 40, 1.0 at ability 120+
        // Players below 40 CA don't get upset about playing time at all
        if ability < 40.0 {
            return 0.0;
        }
        let ability_factor = ((ability - 40.0) / 80.0).clamp(0.0, 1.0);

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
            // Below unhappy threshold — scaled by ability
            let deficit = (unhappy_threshold - play_ratio) / unhappy_threshold.max(0.01);
            -deficit * 20.0 * age_sensitivity * ability_factor
        } else {
            // Between unhappy and expected - mild dissatisfaction, scaled by ability
            let range = expected_ratio - unhappy_threshold;
            let position = (play_ratio - unhappy_threshold) / range.max(0.01);
            (position - 0.5) * 10.0 * ability_factor
        };

        factor.clamp(-20.0, 20.0)
    }

    fn calculate_salary_factor(&self, age: u8) -> f32 {
        let Some(ref contract) = self.contract else {
            return -5.0;
        };

        // Youth/amateur players don't evaluate salary competitively
        match contract.contract_type {
            ContractType::Youth | ContractType::Amateur | ContractType::NonContract => return 0.0,
            _ => {}
        }

        let ability = self.player_attributes.current_ability as f32;

        // Map ability to expected salary matching the generation curve:
        // Salaries are generated as random(2k + rep*30k, 10k + rep*190k)
        // Ability ~30-170 roughly maps to rep_factor 0.0-1.0
        let ability_ratio = ((ability - 30.0) / 140.0).clamp(0.0, 1.0);
        let expected_base = 6000.0 + ability_ratio * 110000.0;

        let age_factor = if age < 22 { 0.7 } else if age > 30 { 0.85 } else { 1.0 };
        let expected = expected_base * age_factor;

        if expected < 1.0 {
            return 0.0;
        }

        let ratio = contract.salary as f32 / expected;
        let factor = if ratio >= 1.2 {
            // Well paid
            10.0_f32.min(ratio * 5.0)
        } else if ratio >= 0.8 {
            // Fairly paid
            (ratio - 0.8) * 25.0 // 0 to 10
        } else {
            // Underpaid
            (ratio - 0.8) * 37.5
        };

        factor.clamp(-15.0, 15.0)
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

    fn calculate_ambition_fit(&self, team_reputation: f32) -> f32 {
        // Compare player ambition against their club's reputation
        // High ambition (>15) at a low-rep club creates unhappiness
        let ambition = self.attributes.ambition;

        // Use main club reputation (overall_score 0.0-1.0, scale to 0-10000)
        let club_rep = team_reputation * 10000.0;

        if ambition <= 10.0 {
            return 0.0; // Low ambition players don't care much
        }

        // Ambition expects a certain club reputation level
        // ambition 20 expects rep ~8000+, ambition 15 expects ~4000+
        let expected_rep = (ambition - 10.0) * 800.0;

        if club_rep >= expected_rep {
            // At or above expected level
            let excess = ((club_rep - expected_rep) / 2000.0).min(1.0);
            excess * 5.0
        } else {
            // Below expected level
            let deficit = ((expected_rep - club_rep) / expected_rep).min(1.0);
            -deficit * 10.0
        }
        .clamp(-10.0, 10.0)
    }
}
