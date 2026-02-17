// Assuming rand is available
extern crate rand;
use crate::club::{
    PersonBehaviour, StaffClubContract, StaffPosition, StaffStatus
};
use crate::context::GlobalContext;
use crate::shared::fullname::FullName;
use crate::utils::DateUtils;
use crate::{CoachFocus, Logging, Person, PersonAttributes, PersonBehaviourState, Player, PlayerSquadStatus, PlayerStatusType, Relations, StaffAttributes, StaffCollectionResult, StaffResponsibility, StaffResult, StaffStub, TeamType, TrainingIntensity, TrainingType};
use chrono::{Datelike, NaiveDate, NaiveDateTime, Timelike};

#[derive(Debug)]
pub struct Staff {
    pub id: u32,
    pub full_name: FullName,
    pub country_id: u32,
    pub birth_date: NaiveDate,
    pub attributes: PersonAttributes,
    pub behaviour: PersonBehaviour,
    pub staff_attributes: StaffAttributes,
    pub contract: Option<StaffClubContract>,
    pub relations: Relations,
    pub license: StaffLicenseType,
    pub focus: Option<CoachFocus>,

    // New fields for enhanced simulation
    pub fatigue: f32,  // 0-100, affects performance
    pub job_satisfaction: f32,  // 0-100, affects retention
    pub recent_performance: StaffPerformance,
    pub coaching_style: CoachingStyle,
    pub training_schedule: Vec<StaffTrainingSession>,
}

#[derive(Debug)]
pub struct StaffCollection {
    pub staffs: Vec<Staff>,

    pub responsibility: StaffResponsibility,

    stub: Staff,
}

impl StaffCollection {
    pub fn new(staffs: Vec<Staff>) -> Self {
        StaffCollection {
            staffs,
            responsibility: StaffResponsibility::default(),
            stub: StaffStub::default(),
        }
    }

    pub fn simulate(&mut self, ctx: GlobalContext<'_>) -> StaffCollectionResult {
        let staff_results = self
            .staffs
            .iter_mut()
            .map(|staff| {
                let message = &format!("simulate staff: id: {}", &staff.id);
                Logging::estimate_result(|| staff.simulate(ctx.with_staff(Some(staff.id))), message)
            })
            .collect();

        StaffCollectionResult::new(staff_results)
    }

    pub fn training_coach(&self, team_type: &TeamType) -> &Staff {
        let responsibility_coach = match team_type {
            TeamType::Main => self.responsibility.training.training_first_team,
            _ => self.responsibility.training.training_youth_team,
        };

        match responsibility_coach {
            Some(_) => self.get_by_id(responsibility_coach.unwrap()),
            None => self.get_by_position(StaffPosition::Coach),
        }
    }

    fn manager(&self) -> Option<&Staff> {
        let manager = self
            .staffs
            .iter()
            .filter(|staff| staff.contract.is_some())
            .find(|staff| {
                staff
                    .contract
                    .as_ref()
                    .expect("no staff contract found")
                    .position
                    == StaffPosition::Manager
            });

        match manager {
            Some(_) => manager,
            None => None,
        }
    }

    pub fn head_coach(&self) -> &Staff {
        match self.manager() {
            Some(ref head_coach) => head_coach,
            None => self.get_by_position(StaffPosition::AssistantManager),
        }
    }

    pub fn contract_resolver(&self, team_type: TeamType) -> &Staff {
        let staff_id = match team_type {
            TeamType::Main => {
                self.responsibility
                    .contract_renewal
                    .handle_first_team_contracts
            }
            TeamType::B => {
                self.responsibility
                    .contract_renewal
                    .handle_other_staff_contracts
            }
            _ => {
                self.responsibility
                    .contract_renewal
                    .handle_youth_team_contracts
            }
        };

        self.get_by_id(staff_id.unwrap())
    }

    /// Staff responsible for outgoing transfers evaluates squad and decides who to list
    pub fn evaluate_outgoing_transfers(&self, players: &[&Player], date: NaiveDate) -> Vec<u32> {
        // Find the staff member responsible for outgoing first-team transfers
        let staff = self
            .responsibility
            .outgoing_transfers
            .find_clubs_for_transfers_and_loans_listed_first_team
            .and_then(|id| self.staffs.iter().find(|s| s.id == id))
            .or_else(|| {
                // Fallback: Director of Football, then Manager
                self.staffs.iter().find(|s| {
                    s.contract.as_ref().map(|c| &c.position)
                        == Some(&StaffPosition::DirectorOfFootball)
                }).or_else(|| {
                    self.manager()
                })
            });

        let staff = match staff {
            Some(s) => s,
            None => return Vec::new(),
        };

        let judging_ability = staff.staff_attributes.knowledge.judging_player_ability as f32;

        // Calculate squad average ability
        let total_ability: u32 = players.iter()
            .map(|p| p.player_attributes.current_ability as u32)
            .sum();
        let avg_ability = if !players.is_empty() {
            total_ability as f32 / players.len() as f32
        } else {
            return Vec::new();
        };

        let mut to_list = Vec::new();

        for player in players {
            // Already listed
            if player.statuses.get().contains(&PlayerStatusType::Lst) {
                continue;
            }

            // Player requested transfer — always list
            if player.statuses.get().contains(&PlayerStatusType::Req) {
                to_list.push(player.id);
                continue;
            }

            // Player unhappy — always list
            if player.statuses.get().contains(&PlayerStatusType::Unh) {
                to_list.push(player.id);
                continue;
            }

            // Contract says not needed — always list
            if let Some(ref contract) = player.contract {
                if matches!(contract.squad_status, PlayerSquadStatus::NotNeeded) {
                    to_list.push(player.id);
                    continue;
                }
                if contract.is_transfer_listed {
                    to_list.push(player.id);
                    continue;
                }
            }

            // Staff evaluates: low ability relative to squad
            // Better staff (higher judging_ability) sets a tighter threshold
            let threshold = 10.0 + (20.0 - judging_ability) * 0.5; // 10-20 range
            let ability = player.player_attributes.current_ability as f32;
            if ability < avg_ability - threshold {
                to_list.push(player.id);
                continue;
            }

            // Aging player with declining ability and low potential gap
            let age = player.age(date);
            if age >= 32 {
                let potential_gap = player.player_attributes.potential_ability as i16
                    - player.player_attributes.current_ability as i16;
                if potential_gap <= 0 && ability < avg_ability {
                    to_list.push(player.id);
                }
            }
        }

        to_list
    }

    fn get_by_position(&self, position: StaffPosition) -> &Staff {
        let staffs: Vec<&Staff> = self
            .staffs
            .iter()
            .filter(|staff| {
                staff.contract.is_some() && staff.contract.as_ref().unwrap().position == position
            })
            .collect();

        if staffs.is_empty() {
            return &self.stub;
        }

        //TODO most relevant

        staffs.first().unwrap()
    }

    fn get_by_id(&self, id: u32) -> &Staff {
        self.staffs.iter().find(|staff| staff.id == id).unwrap()
    }
}

#[derive(Debug, Clone)]
pub struct StaffPerformance {
    pub training_effectiveness: f32,  // 0-1 multiplier
    pub player_development_rate: f32,  // 0-1 multiplier
    pub injury_prevention_rate: f32,  // 0-1 multiplier
    pub tactical_implementation: f32,  // 0-1 multiplier
    pub last_evaluation_date: Option<NaiveDate>,
}

#[derive(Debug, Clone)]
pub enum CoachingStyle {
    Authoritarian,    // Strict discipline, high demands
    Democratic,       // Collaborative, player input
    LaissezFaire,     // Hands-off, player autonomy
    Transformational, // Inspirational, vision-focused
    Tactical,         // Detail-oriented, system-focused
}

impl Staff {
    pub fn new(
        id: u32,
        full_name: FullName,
        country_id: u32,
        birth_date: NaiveDate,
        staff_attributes: StaffAttributes,
        contract: Option<StaffClubContract>,
        attributes: PersonAttributes,
        license: StaffLicenseType,
        focus: Option<CoachFocus>,
    ) -> Self {
        Staff {
            id,
            full_name,
            country_id,
            birth_date,
            staff_attributes,
            contract,
            behaviour: PersonBehaviour::default(),
            relations: Relations::new(),
            attributes,
            license,
            focus,
            fatigue: 0.0,
            job_satisfaction: 50.0,
            recent_performance: StaffPerformance::default(),
            coaching_style: CoachingStyle::default(),
            training_schedule: Vec::new(),
        }
    }

    pub fn simulate(&mut self, ctx: GlobalContext<'_>) -> StaffResult {
        let now = ctx.simulation.date;
        let mut result = StaffResult::new();

        // Birthday handling - improves mood
        if DateUtils::is_birthday(self.birth_date, now.date()) {
            self.behaviour.try_increase();
            self.job_satisfaction = (self.job_satisfaction + 5.0).min(100.0);
            result.add_event(StaffMoraleEvent::Birthday);
        }

        // Process contract status and negotiations
        self.process_contract(&mut result, now);

        // Update fatigue based on workload
        self.update_fatigue(&ctx, &mut result);

        // Process training responsibilities
        self.process_training_duties(&ctx, &mut result);

        // Update job satisfaction
        self.update_job_satisfaction(&ctx, &mut result);

        // Check for burnout or resignation triggers
        self.check_resignation_triggers(&mut result);

        // Process relationships with players and other staff
        self.process_relationships(&ctx, &mut result);

        // Handle performance evaluation
        if self.should_evaluate_performance(now.date()) {
            self.evaluate_performance(&ctx, &mut result);
        }

        // Process professional development
        self.process_professional_development(&ctx, &mut result);

        // Scouting duties for scouts
        self.process_scouting(&ctx, &mut result);

        result
    }

    fn process_contract(&mut self, result: &mut StaffResult, now: NaiveDateTime) {
        if let Some(ref mut contract) = self.contract {
            const THREE_MONTHS_DAYS: i64 = 90;
            const SIX_MONTHS_DAYS: i64 = 180;

            let days_remaining = contract.days_to_expiration(now);

            // Check if contract expired
            if days_remaining <= 0 {
                contract.status = StaffStatus::ExpiredContract;
                result.contract.expired = true;

                // Decide if staff wants to renew
                if self.wants_renewal() {
                    result.contract.wants_renewal = true;
                    result.contract.requested_salary = self.calculate_desired_salary();
                } else {
                    result.contract.leaving = true;
                }
            }
            // Contract expiring soon - start negotiations
            else if days_remaining < SIX_MONTHS_DAYS {
                if days_remaining < THREE_MONTHS_DAYS && !result.contract.negotiating {
                    // Urgent renewal needed
                    result.contract.negotiating = true;
                    result.contract.urgent = true;

                    if self.job_satisfaction < 40.0 {
                        // Unhappy - likely to leave
                        result.contract.likely_to_leave = true;
                    }
                }

                // Staff member initiates renewal discussion
                if self.attributes.ambition > 15.0 && self.recent_performance.training_effectiveness > 0.7 {
                    result.contract.wants_improved_terms = true;
                    result.contract.requested_salary = contract.salary as f32 * 1.3;
                }
            }
        } else {
            // No contract - staff is likely temporary or consultant
            result.contract.no_contract = true;

            if self.recent_performance.training_effectiveness > 0.8 {
                // Performing well, should offer contract
                result.contract.deserves_contract = true;
            }
        }
    }

    fn update_fatigue(&mut self, ctx: &GlobalContext<'_>, result: &mut StaffResult) {
        // Calculate workload based on responsibilities
        let workload = self.calculate_workload(ctx);

        // Increase fatigue based on workload
        self.fatigue += workload * 2.0;

        // Recovery on weekends
        if ctx.simulation.date.weekday() == chrono::Weekday::Sun {
            self.fatigue = (self.fatigue - 15.0).max(0.0);
        }

        // Vacation periods provide significant recovery
        if self.is_on_vacation(ctx.simulation.date.date()) {
            self.fatigue = (self.fatigue - 30.0).max(0.0);
        }

        // Cap fatigue at 100
        self.fatigue = self.fatigue.min(100.0);

        // High fatigue affects performance and morale
        if self.fatigue > 80.0 {
            result.add_warning(StaffWarning::HighFatigue);
            self.recent_performance.training_effectiveness *= 0.8;
            self.job_satisfaction -= 2.0;
        }

        // Extreme fatigue can lead to health issues
        if self.fatigue > 95.0 {
            result.add_warning(StaffWarning::BurnoutRisk);
            if rand::random::<f32>() < 0.05 {
                result.health_issue = Some(HealthIssue::StressRelated);
            }
        }
    }

    fn process_training_duties(&mut self, ctx: &GlobalContext<'_>, result: &mut StaffResult) {
        // Only process if staff has coaching responsibilities
        if !self.has_coaching_duties() {
            return;
        }

        // Plan training sessions for the week
        if ctx.simulation.is_week_beginning() {
            self.training_schedule = self.plan_weekly_training(ctx);
            result.training.sessions_planned = self.training_schedule.len() as u8;
        }

        // Execute today's training if scheduled
        if let Some(session) = self.get_todays_training(ctx.simulation.date) {
            // Training effectiveness based on various factors
            let effectiveness = self.calculate_training_effectiveness();

            result.training.session_conducted = true;
            result.training.effectiveness = effectiveness;
            result.training.session_type = session.session_type.clone();

            // Track which players attended
            if let Some(team_id) = ctx.team.as_ref().map(|t| t.id) {
                result.training.team_id = Some(team_id);
            }

            // Fatigue from conducting training
            self.fatigue += match session.intensity {
                TrainingIntensity::VeryLight => 1.0,
                TrainingIntensity::Light => 2.0,
                TrainingIntensity::Moderate => 3.0,
                TrainingIntensity::High => 4.0,
                TrainingIntensity::VeryHigh => 5.0,
            };
        }
    }

    fn update_job_satisfaction(&mut self, ctx: &GlobalContext<'_>, result: &mut StaffResult) {
        let mut satisfaction_change = 0.0;

        // Positive factors
        if self.recent_performance.training_effectiveness > 0.75 {
            satisfaction_change += 1.0; // Good performance
        }

        if self.behaviour.state == PersonBehaviourState::Good {
            satisfaction_change += 0.5; // Good relationships
        }

        // Check team performance if applicable
        if let Some(_club) = ctx.club.as_ref() {
            // Would need team performance metrics
            // satisfaction_change += team_performance_factor;
        }

        // Negative factors
        if self.fatigue > 70.0 {
            satisfaction_change -= 2.0; // Overworked
        }

        if let Some(contract) = &self.contract {
            if self.is_underpaid(contract.salary) {
                satisfaction_change -= 1.5; // Salary dissatisfaction
            }
        }

        // Apply change with dampening
        self.job_satisfaction = (self.job_satisfaction + satisfaction_change * 0.5)
            .clamp(0.0, 100.0);

        // Report significant satisfaction issues
        if self.job_satisfaction < 30.0 {
            result.add_warning(StaffWarning::LowMorale);
        } else if self.job_satisfaction > 80.0 {
            result.add_event(StaffMoraleEvent::HighSatisfaction);
        }
    }

    fn check_resignation_triggers(&self, result: &mut StaffResult) {
        // Multiple factors can trigger resignation consideration
        let resignation_probability = self.calculate_resignation_probability();

        if resignation_probability > 0.0 {
            if rand::random::<f32>() < resignation_probability {
                result.resignation_risk = true;

                if resignation_probability > 0.5 {
                    // Actually submit resignation
                    result.resigned = true;
                    result.resignation_reason = Some(self.determine_resignation_reason());
                }
            }
        }
    }

    fn process_relationships(&mut self, ctx: &GlobalContext<'_>, result: &mut StaffResult) {
        // Daily relationship updates are minimal
        // Major updates happen during training and matches

        if ctx.simulation.date.hour() == 12 {  // Midday check
            // Small random relationship events
            if rand::random::<f32>() < 0.1 {
                // Positive interaction with random player
                result.relationship_event = Some(RelationshipEvent::PositiveInteraction);

                // This would update the actual relations
                // self.relations.update_simple(player_id, 0.5);
            }

            if rand::random::<f32>() < 0.05 && self.job_satisfaction < 40.0 {
                // Conflict when satisfaction is low
                result.relationship_event = Some(RelationshipEvent::Conflict);
            }
        }
    }

    fn evaluate_performance(&mut self, ctx: &GlobalContext<'_>, result: &mut StaffResult) {
        // Monthly performance evaluation
        let prev_effectiveness = self.recent_performance.training_effectiveness;

        // Calculate new performance metrics
        self.recent_performance = self.calculate_performance_metrics(ctx);
        self.recent_performance.last_evaluation_date = Some(ctx.simulation.date.date());

        // Report performance change
        if self.recent_performance.training_effectiveness > prev_effectiveness + 0.1 {
            result.performance_improved = true;
        } else if self.recent_performance.training_effectiveness < prev_effectiveness - 0.1 {
            result.performance_declined = true;
        }

        // Board/management reaction to performance
        if self.recent_performance.training_effectiveness < 0.4 {
            result.add_warning(StaffWarning::PoorPerformance);
        } else if self.recent_performance.training_effectiveness > 0.8 {
            result.add_event(StaffMoraleEvent::ExcellentPerformance);
            self.job_satisfaction += 5.0;
        }
    }

    fn process_professional_development(&mut self, ctx: &GlobalContext<'_>, result: &mut StaffResult) {
        // Check for license upgrade opportunities
        if self.should_upgrade_license() {
            if rand::random::<f32>() < 0.01 {  // Small daily chance
                result.license_upgrade_available = true;

                if self.attributes.ambition > 15.0 {
                    result.wants_license_upgrade = true;
                }
            }
        }

        // Learning from experience
        if ctx.simulation.is_month_beginning() {
            self.improve_attributes_from_experience();
        }

        // Attending courses or conferences
        if self.is_on_course(ctx.simulation.date.date()) {
            result.on_professional_development = true;
            self.fatigue = (self.fatigue - 5.0).max(0.0);  // Courses are refreshing
        }
    }

    fn process_scouting(&mut self, _ctx: &GlobalContext<'_>, _result: &mut StaffResult) {
        // Real scouting is now handled at country level (Country::process_scouting)
        // which has access to all clubs and players for cross-club evaluation.
    }

    // Helper methods

    fn wants_renewal(&self) -> bool {
        self.job_satisfaction > 40.0 &&
            self.behaviour.state != PersonBehaviourState::Poor
    }

    fn calculate_desired_salary(&self) -> f32 {
        let base = self.contract.as_ref().map(|c| c.salary).unwrap_or(50000) as f32;
        let performance_multiplier = 1.0 + (self.recent_performance.training_effectiveness - 0.5);
        let ambition_multiplier = 1.0 + (self.attributes.ambition / 20.0) * 0.3;

        base * performance_multiplier * ambition_multiplier
    }

    fn calculate_workload(&self, _ctx: &GlobalContext<'_>) -> f32 {
        // Base workload from position
        let position_load = match self.contract.as_ref().map(|c| &c.position) {
            Some(StaffPosition::Manager) => 8.0,
            Some(StaffPosition::AssistantManager) => 6.0,
            Some(StaffPosition::Coach) => 5.0,
            Some(StaffPosition::FitnessCoach) => 4.0,
            Some(StaffPosition::GoalkeeperCoach) => 3.0,
            Some(StaffPosition::Scout) => 4.0,
            Some(StaffPosition::Physio) => 5.0,
            _ => 3.0,
        };

        // Additional load from training sessions
        let training_load = self.training_schedule.len() as f32 * 0.5;

        position_load + training_load
    }

    fn is_on_vacation(&self, date: NaiveDate) -> bool {
        // Summer break (simplified)
        date.month() == 7 && date.day() <= 14
    }

    fn has_coaching_duties(&self) -> bool {
        matches!(
            self.contract.as_ref().map(|c| &c.position),
            Some(StaffPosition::Manager) |
            Some(StaffPosition::AssistantManager) |
            Some(StaffPosition::Coach) |
            Some(StaffPosition::FitnessCoach) |
            Some(StaffPosition::GoalkeeperCoach) |
            Some(StaffPosition::FirstTeamCoach) |
            Some(StaffPosition::YouthCoach)
        )
    }

    fn plan_weekly_training(&self, _ctx: &GlobalContext<'_>) -> Vec<StaffTrainingSession> {
        let mut sessions = Vec::new();

        // Simplified training plan
        // Monday - Recovery
        sessions.push(StaffTrainingSession {
            session_type: TrainingType::Recovery,
            intensity: TrainingIntensity::Light,
            duration_minutes: 60,
        });

        // Tuesday - Technical
        sessions.push(StaffTrainingSession {
            session_type: TrainingType::BallControl,
            intensity: TrainingIntensity::Moderate,
            duration_minutes: 90,
        });

        // Wednesday - Tactical
        sessions.push(StaffTrainingSession {
            session_type: TrainingType::TeamShape,
            intensity: TrainingIntensity::Moderate,
            duration_minutes: 90,
        });

        // Thursday - Physical
        sessions.push(StaffTrainingSession {
            session_type: TrainingType::Endurance,
            intensity: TrainingIntensity::High,
            duration_minutes: 75,
        });

        // Friday - Match preparation
        sessions.push(StaffTrainingSession {
            session_type: TrainingType::Positioning,
            intensity: TrainingIntensity::Light,
            duration_minutes: 60,
        });

        sessions
    }

    fn get_todays_training(&self, date: NaiveDateTime) -> Option<&StaffTrainingSession> {
        // Map weekday to training session
        let weekday = date.weekday();
        let index = match weekday {
            chrono::Weekday::Mon => 0,
            chrono::Weekday::Tue => 1,
            chrono::Weekday::Wed => 2,
            chrono::Weekday::Thu => 3,
            chrono::Weekday::Fri => 4,
            _ => return None,  // No training on weekends
        };

        self.training_schedule.get(index)
    }

    fn calculate_training_effectiveness(&self) -> f32 {
        let base = (self.staff_attributes.coaching.technical as f32 +
            self.staff_attributes.coaching.tactical as f32 +
            self.staff_attributes.coaching.fitness as f32 +
            self.staff_attributes.coaching.mental as f32) / 80.0;

        let fatigue_penalty = if self.fatigue > 50.0 {
            1.0 - ((self.fatigue - 50.0) / 100.0)
        } else {
            1.0
        };

        let morale_bonus = self.job_satisfaction / 100.0;

        (base * fatigue_penalty * morale_bonus).clamp(0.1, 1.0)
    }

    fn is_underpaid(&self, salary: u32) -> bool {
        // Compare to market rate based on attributes and performance
        let expected_salary = self.calculate_market_value();
        salary < (expected_salary as u32)
    }

    fn calculate_market_value(&self) -> f32 {
        // Base salary by position and license
        let base = match self.license {
            StaffLicenseType::ContinentalPro => 100000.0,
            StaffLicenseType::ContinentalA => 70000.0,
            StaffLicenseType::ContinentalB => 50000.0,
            StaffLicenseType::ContinentalC => 35000.0,
            StaffLicenseType::NationalA => 30000.0,
            StaffLicenseType::NationalB => 25000.0,
            StaffLicenseType::NationalC => 20000.0,
        };

        let skill_multiplier = (self.staff_attributes.coaching.tactical as f32 +
            self.staff_attributes.coaching.technical as f32) / 40.0 + 0.5;

        base * skill_multiplier * self.recent_performance.training_effectiveness
    }

    fn calculate_resignation_probability(&self) -> f32 {
        let mut prob: f32 = 0.0;

        // Job satisfaction is primary factor
        if self.job_satisfaction < 20.0 {
            prob += 0.3;
        } else if self.job_satisfaction < 35.0 {
            prob += 0.1;
        }

        // Extreme fatigue
        if self.fatigue > 90.0 {
            prob += 0.2;
        }

        // Poor relationships
        if self.behaviour.state == PersonBehaviourState::Poor {
            prob += 0.15;
        }

        // Contract issues
        if self.contract.is_none() {
            prob += 0.1;
        } else if let Some(contract) = &self.contract {
            if self.is_underpaid(contract.salary) {
                prob += 0.1;
            }
        }

        prob.min(0.9)  // Cap at 90% chance
    }

    fn determine_resignation_reason(&self) -> ResignationReason {
        if self.job_satisfaction < 30.0 {
            ResignationReason::LowSatisfaction
        } else if self.fatigue > 85.0 {
            ResignationReason::Burnout
        } else if self.behaviour.state == PersonBehaviourState::Poor {
            ResignationReason::PersonalReasons
        } else {
            ResignationReason::BetterOpportunity
        }
    }

    fn should_evaluate_performance(&self, date: NaiveDate) -> bool {
        // Monthly evaluation
        if let Some(last_eval) = self.recent_performance.last_evaluation_date {
            (date - last_eval).num_days() >= 30
        } else {
            true  // First evaluation
        }
    }

    fn calculate_performance_metrics(&self, ctx: &GlobalContext<'_>) -> StaffPerformance {
        // Simplified calculation - would need actual team/player data
        StaffPerformance {
            training_effectiveness: self.calculate_training_effectiveness(),
            player_development_rate: (self.staff_attributes.coaching.working_with_youngsters as f32 / 20.0),
            injury_prevention_rate: (self.staff_attributes.medical.sports_science as f32 / 20.0),
            tactical_implementation: (self.staff_attributes.coaching.tactical as f32 / 20.0),
            last_evaluation_date: Some(ctx.simulation.date.date()),
        }
    }

    fn should_upgrade_license(&self) -> bool {
        // Check if eligible for license upgrade
        match self.license {
            StaffLicenseType::NationalC => {
                self.staff_attributes.coaching.tactical > 10
            },
            StaffLicenseType::NationalB => {
                self.staff_attributes.coaching.tactical > 12 &&
                    self.staff_attributes.coaching.technical > 12
            },
            StaffLicenseType::NationalA => {
                self.staff_attributes.coaching.tactical > 14 &&
                    self.staff_attributes.coaching.technical > 14
            },
            _ => false,  // Continental licenses need special conditions
        }
    }

    fn improve_attributes_from_experience(&mut self) {
        // Slow improvement over time
        if rand::random::<f32>() < 0.3 {
            // Small chance of improvement each month
            let improvement = 1;

            // Improve a random coaching attribute
            match rand::random::<u8>() % 6 {
                0 => self.staff_attributes.coaching.attacking =
                    (self.staff_attributes.coaching.attacking + improvement).min(20),
                1 => self.staff_attributes.coaching.defending =
                    (self.staff_attributes.coaching.defending + improvement).min(20),
                2 => self.staff_attributes.coaching.tactical =
                    (self.staff_attributes.coaching.tactical + improvement).min(20),
                3 => self.staff_attributes.coaching.technical =
                    (self.staff_attributes.coaching.technical + improvement).min(20),
                4 => self.staff_attributes.coaching.fitness =
                    (self.staff_attributes.coaching.fitness + improvement).min(20),
                _ => self.staff_attributes.coaching.mental =
                    (self.staff_attributes.coaching.mental + improvement).min(20),
            }
        }
    }

    fn is_on_course(&self, date: NaiveDate) -> bool {
        // Simplified - courses in January
        date.month() == 1 && date.day() >= 15 && date.day() <= 20
    }
}

#[derive(Debug, Default)]
pub struct StaffContractResult {
    pub expired: bool,
    pub no_contract: bool,
    pub negotiating: bool,
    pub urgent: bool,
    pub wants_renewal: bool,
    pub wants_improved_terms: bool,
    pub likely_to_leave: bool,
    pub leaving: bool,
    pub deserves_contract: bool,
    pub requested_salary: f32,
}

#[derive(Debug, Default)]
pub struct StaffTrainingResult {
    pub sessions_planned: u8,
    pub session_conducted: bool,
    pub effectiveness: f32,
    pub session_type: TrainingType,
    pub team_id: Option<u32>,
}

#[derive(Debug)]
pub enum StaffWarning {
    HighFatigue,
    BurnoutRisk,
    LowMorale,
    PoorPerformance,
}

#[derive(Debug)]
pub enum StaffMoraleEvent {
    Birthday,
    HighSatisfaction,
    ExcellentPerformance,
}

#[derive(Debug)]
pub enum ResignationReason {
    LowSatisfaction,
    Burnout,
    PersonalReasons,
    BetterOpportunity,
    Retirement,
}

#[derive(Debug)]
pub enum HealthIssue {
    StressRelated,
    PhysicalInjury,
    Illness,
}

#[derive(Debug)]
pub enum RelationshipEvent {
    PositiveInteraction,
    Conflict,
    MentorshipStarted,
    TrustBuilt,
}

#[derive(Debug)]
pub enum StaffLicenseType {
    ContinentalPro,
    ContinentalA,
    ContinentalB,
    ContinentalC,
    NationalA,
    NationalB,
    NationalC
}

// Default implementations
impl Default for StaffPerformance {
    fn default() -> Self {
        StaffPerformance {
            training_effectiveness: 0.5,
            player_development_rate: 0.5,
            injury_prevention_rate: 0.5,
            tactical_implementation: 0.5,
            last_evaluation_date: None,
        }
    }
}

impl Default for CoachingStyle {
    fn default() -> Self {
        CoachingStyle::Democratic
    }
}

#[derive(Debug, Clone)]
pub struct StaffTrainingSession {
    pub session_type: TrainingType,
    pub intensity: TrainingIntensity,
    pub duration_minutes: u16,
}

// Additional helper trait implementations
impl StaffClubContract {
    pub fn days_to_expiration(&self, now: NaiveDateTime) -> i64 {
        (self.expired - now.date()).num_days()
    }
}

