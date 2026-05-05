// Assuming rand is available
extern crate rand;
use crate::club::{PersonBehaviour, StaffClubContract, StaffPosition, StaffStatus};
use crate::context::GlobalContext;
use crate::shared::fullname::FullName;
use crate::utils::DateUtils;
use crate::{
    CoachFocus, Logging, PersonAttributes, PersonBehaviourState, PlayerFieldPositionGroup,
    Relations, StaffAttributes, StaffCollectionResult, StaffResponsibility, StaffResult, StaffStub,
    TeamType, TrainingIntensity, TrainingType,
};
use chrono::{Datelike, NaiveDate, NaiveDateTime, Timelike};

#[derive(Debug, Clone)]
pub struct StaffEvent {
    pub event_type: StaffEventType,
    pub days_ago: u16,
}

#[derive(Debug, Clone, PartialEq)]
pub enum StaffEventType {
    TrainingConducted,
    MatchObserved,
    PlayerScouted,
    PositiveInteraction,
    Conflict,
    MentorshipStarted,
    TrustBuilt,
    PerformanceExcellent,
    PerformanceDeclined,
    LicenseUpgrade,
    ProfessionalDevelopment,
    Birthday,
    HighFatigue,
    ContractNegotiation,
    /// Staff attended a recruitment meeting that voted on transfer
    /// targets. Surfaces in the staff event feed so users can see when
    /// scouts have been participating in board-facing decisions.
    RecruitmentMeeting,
    /// Staff publicly recommended a target which the meeting promoted
    /// to the shortlist or sent to the board.
    TargetRecommended,
    /// Staff voted to reject a target which the meeting then dropped.
    TargetRejected,
    /// Staff presented a recruitment dossier to the board for approval.
    BoardPresentation,
}

#[derive(Debug, Clone)]
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
    pub fatigue: f32,          // 0-100, affects performance
    pub job_satisfaction: f32, // 0-100, affects retention
    pub recent_performance: StaffPerformance,
    pub coaching_style: CoachingStyle,
    pub training_schedule: Vec<StaffTrainingSession>,
    pub recent_events: Vec<StaffEvent>,

    /// Cumulative specialization days per position group (GK/DEF/MID/FWD).
    /// A coach who spends most sessions training midfielders will eventually
    /// become a "midfield specialist" — they extract extra gains from that
    /// group compared to generalist coaching. Grows over time; never resets.
    ///
    /// Indices match `PlayerFieldPositionGroup` discriminants.
    pub specialization_days: [u32; 4],
}

#[derive(Debug, Clone)]
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

    pub fn head_coach(&self) -> &Staff {
        match self.manager() {
            Some(head_coach) => head_coach,
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
            TeamType::B | TeamType::Second => {
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

    /// Borrow a staff member by id.
    pub fn find(&self, staff_id: u32) -> Option<&Staff> {
        self.staffs.iter().find(|s| s.id == staff_id)
    }

    /// Mutable variant of `find`.
    pub fn find_mut(&mut self, staff_id: u32) -> Option<&mut Staff> {
        self.staffs.iter_mut().find(|s| s.id == staff_id)
    }

    /// Is this id on the staff?
    pub fn contains(&self, staff_id: u32) -> bool {
        self.staffs.iter().any(|s| s.id == staff_id)
    }

    pub fn iter(&self) -> std::slice::Iter<'_, Staff> {
        self.staffs.iter()
    }

    pub fn iter_mut(&mut self) -> std::slice::IterMut<'_, Staff> {
        self.staffs.iter_mut()
    }

    pub fn len(&self) -> usize {
        self.staffs.len()
    }

    pub fn is_empty(&self) -> bool {
        self.staffs.is_empty()
    }

    /// Sum of annual salaries across every staff member with a contract.
    /// Mirrors `PlayerCollection::get_annual_salary` so the club finance
    /// tick doesn't have to open-code the iter + filter_map + map + sum
    /// pipeline.
    pub fn get_annual_salary(&self) -> u32 {
        self.staffs
            .iter()
            .filter_map(|s| s.contract.as_ref())
            .map(|c| c.salary)
            .sum()
    }

    /// Borrow the staff member currently holding `position`, if any.
    /// Unlike `get_by_position`, this does NOT fall back to the stub.
    pub fn find_by_position(&self, position: StaffPosition) -> Option<&Staff> {
        self.staffs.iter().find(|s| {
            s.contract
                .as_ref()
                .map(|c| c.position == position)
                .unwrap_or(false)
        })
    }

    /// Mutable counterpart of `find_by_position`.
    pub fn find_mut_by_position(&mut self, position: StaffPosition) -> Option<&mut Staff> {
        self.staffs.iter_mut().find(|s| {
            s.contract
                .as_ref()
                .map(|c| c.position == position)
                .unwrap_or(false)
        })
    }

    /// Any contracted staff matching one of the supplied positions.
    /// Useful for "find any current coach" or similar role queries.
    pub fn find_by_any_position(&self, positions: &[StaffPosition]) -> Option<&Staff> {
        self.staffs.iter().find(|s| {
            s.contract
                .as_ref()
                .map(|c| positions.contains(&c.position))
                .unwrap_or(false)
        })
    }

    /// The head coach slot — permanent manager OR interim caretaker.
    /// Returns `None` if neither seat is filled.
    pub fn manager(&self) -> Option<&Staff> {
        self.find_by_any_position(&[StaffPosition::Manager, StaffPosition::CaretakerManager])
    }

    /// Mutable counterpart of `manager`.
    pub fn manager_mut(&mut self) -> Option<&mut Staff> {
        self.staffs.iter_mut().find(|s| {
            s.contract
                .as_ref()
                .map(|c| {
                    matches!(
                        c.position,
                        StaffPosition::Manager | StaffPosition::CaretakerManager
                    )
                })
                .unwrap_or(false)
        })
    }

    /// Iterate contracted coaching staff (Manager, assistants, coaches, GK/fitness/youth coaches).
    pub fn coaches(&self) -> impl Iterator<Item = &Staff> {
        self.staffs.iter().filter(|s| {
            s.contract
                .as_ref()
                .map(|c| c.position.is_coaching())
                .unwrap_or(false)
        })
    }

    /// Iterate contracted medical staff (Physio, Head of Physio).
    pub fn medical(&self) -> impl Iterator<Item = &Staff> {
        self.staffs.iter().filter(|s| {
            s.contract
                .as_ref()
                .map(|c| c.position.is_medical())
                .unwrap_or(false)
        })
    }

    /// Iterate contracted scouting staff.
    pub fn scouts(&self) -> impl Iterator<Item = &Staff> {
        self.staffs.iter().filter(|s| {
            s.contract
                .as_ref()
                .map(|c| c.position.is_scouting())
                .unwrap_or(false)
        })
    }

    /// Pick the coaching-staff member that maximises `score_fn`. Used to
    /// choose a caretaker when the manager seat opens up. Returns id
    /// rather than a borrow so callers can then take a mutable borrow
    /// to update the contract without overlapping borrows.
    pub fn best_coach_id<F>(&self, mut score_fn: F) -> Option<u32>
    where
        F: FnMut(&Staff) -> u32,
    {
        self.coaches().max_by_key(|s| score_fn(s)).map(|s| s.id)
    }

    /// Remove the staff member currently holding `position` and return them
    /// owned. Used by the sacking / poaching paths so the caller can route
    /// the freed staff member into the global free-agent pool. Returns
    /// `None` when no staff member holds that position.
    pub fn take_by_position(&mut self, position: StaffPosition) -> Option<Staff> {
        let idx = self.staffs.iter().position(|s| {
            s.contract
                .as_ref()
                .map(|c| c.position == position)
                .unwrap_or(false)
        })?;
        Some(self.staffs.remove(idx))
    }

    /// Remove a staff member by id and return them owned. Mirrors
    /// `take_by_position` for cases where the caller already knows the id
    /// (e.g. manager-market poaching where the candidate is selected by
    /// id before the move).
    pub fn take_by_id(&mut self, id: u32) -> Option<Staff> {
        let idx = self.staffs.iter().position(|s| s.id == id)?;
        Some(self.staffs.remove(idx))
    }

    /// Append a staff member to the collection. Used by the manager-market
    /// pipeline when a free agent or poached coach is signed.
    pub fn push(&mut self, staff: Staff) {
        self.staffs.push(staff);
    }

    /// Best `working_with_youngsters` attribute among staff assigned to
    /// youth-development roles (Head of Youth Development, Youth Coach).
    /// Returns `fallback` when no such staff member is on the books —
    /// used by the mentorship pass to scale transfer rates.
    pub fn best_youth_development_wwy(&self, fallback: u8) -> u8 {
        self.staffs
            .iter()
            .filter(|s| {
                s.contract
                    .as_ref()
                    .map(|c| {
                        matches!(
                            c.position,
                            StaffPosition::HeadOfYouthDevelopment | StaffPosition::YouthCoach
                        )
                    })
                    .unwrap_or(false)
            })
            .map(|s| s.staff_attributes.coaching.working_with_youngsters)
            .max()
            .unwrap_or(fallback)
    }

    /// Best `sports_science` attribute across the whole medical department.
    /// Drives preventive-rest eligibility and injury-risk reductions.
    pub fn best_sports_science(&self) -> u8 {
        self.staffs
            .iter()
            .map(|s| s.staff_attributes.medical.sports_science)
            .max()
            .unwrap_or(0)
    }

    /// Best `physiotherapy` attribute across the whole medical department.
    pub fn best_physiotherapy(&self) -> u8 {
        self.staffs
            .iter()
            .map(|s| s.staff_attributes.medical.physiotherapy)
            .max()
            .unwrap_or(0)
    }

    /// Find the most relevant contracted staff member for `position`.
    /// "Relevant" is computed by `Staff::relevance_score_for` — a
    /// per-position blend of coaching / knowledge / medical attributes
    /// (e.g. Manager rewards tactical_knowledge + man_management;
    /// GoalkeeperCoach rewards goalkeeping skills; Physio rewards
    /// physiotherapy + sports_science). Falls back to the stub only
    /// when no contracted staff hold the role.
    fn get_by_position(&self, position: StaffPosition) -> &Staff {
        let mut best: Option<(&Staff, u32)> = None;
        for staff in &self.staffs {
            let Some(contract) = staff.contract.as_ref() else {
                continue;
            };
            if contract.position != position {
                continue;
            }
            let score = staff.relevance_score_for(&position);
            best = match best {
                None => Some((staff, score)),
                Some((_, prev)) if score > prev => Some((staff, score)),
                Some(prev) => Some(prev),
            };
        }
        match best {
            Some((staff, _)) => staff,
            None => &self.stub,
        }
    }

    fn get_by_id(&self, id: u32) -> &Staff {
        self.staffs.iter().find(|staff| staff.id == id).unwrap()
    }
}

#[derive(Debug, Clone)]
pub struct StaffPerformance {
    pub training_effectiveness: f32,  // 0-1 multiplier
    pub player_development_rate: f32, // 0-1 multiplier
    pub injury_prevention_rate: f32,  // 0-1 multiplier
    pub tactical_implementation: f32, // 0-1 multiplier
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
            recent_events: Vec::new(),
            specialization_days: [0; 4],
        }
    }

    /// Score this staff member's fit for a specific role. The blend
    /// per role is hand-tuned from the FM-style heuristic: each
    /// "important" attribute counts at full weight (10), with one or
    /// two "secondary" attributes at half weight (5). The result is a
    /// dimensionless u32 used by `StaffCollection::get_by_position` to
    /// pick the most relevant candidate among all who hold the role.
    /// Score scales with attribute values (0..20) so a 20-rated coach
    /// always outranks a 10-rated coach in the same seat.
    pub fn relevance_score_for(&self, position: &StaffPosition) -> u32 {
        let coaching = &self.staff_attributes.coaching;
        let knowledge = &self.staff_attributes.knowledge;
        let mental = &self.staff_attributes.mental;
        let medical = &self.staff_attributes.medical;
        let goalkeeping = &self.staff_attributes.goalkeeping;
        let data = &self.staff_attributes.data_analysis;

        // Helper: weighted attribute sum.
        let s = |w: u32, a: u8| w * a as u32;

        match position {
            StaffPosition::Manager
            | StaffPosition::CaretakerManager
            | StaffPosition::AssistantManager => {
                // Tactical brain + man management + ability to read the room.
                s(10, knowledge.tactical_knowledge)
                    + s(10, mental.man_management)
                    + s(8, mental.motivating)
                    + s(7, mental.discipline)
                    + s(5, knowledge.judging_player_ability)
                    + s(4, mental.adaptability)
                    + s(3, mental.determination)
            }
            StaffPosition::Coach | StaffPosition::FirstTeamCoach => {
                // Generalist outfield coach — technical / tactical / mental
                // weighted equally, with fitness as a secondary contributor.
                s(8, coaching.technical)
                    + s(8, coaching.tactical)
                    + s(8, coaching.mental)
                    + s(5, coaching.attacking)
                    + s(5, coaching.defending)
                    + s(4, coaching.fitness)
            }
            StaffPosition::FitnessCoach => {
                // Fitness specialism + sports science cross-talk.
                s(10, coaching.fitness)
                    + s(6, medical.sports_science)
                    + s(4, coaching.mental)
            }
            StaffPosition::GoalkeeperCoach => {
                // GK technical specialism dominates; coaching.technical is
                // the outfield analogue and only counts a little.
                s(8, goalkeeping.shot_stopping)
                    + s(8, goalkeeping.handling)
                    + s(6, goalkeeping.distribution)
                    + s(3, coaching.technical)
            }
            StaffPosition::YouthCoach => {
                // Working with youngsters is the killer attribute here.
                s(10, coaching.working_with_youngsters)
                    + s(6, coaching.technical)
                    + s(6, coaching.tactical)
                    + s(4, coaching.mental)
            }
            StaffPosition::U21Manager | StaffPosition::U19Manager => {
                s(10, coaching.working_with_youngsters)
                    + s(8, knowledge.tactical_knowledge)
                    + s(6, mental.man_management)
                    + s(4, mental.motivating)
            }
            StaffPosition::HeadOfYouthDevelopment => {
                s(10, coaching.working_with_youngsters)
                    + s(7, knowledge.judging_player_potential)
                    + s(5, mental.man_management)
            }
            StaffPosition::Physio => {
                s(10, medical.physiotherapy) + s(6, medical.sports_science)
            }
            StaffPosition::HeadOfPhysio => {
                s(10, medical.physiotherapy)
                    + s(8, medical.sports_science)
                    + s(4, mental.man_management)
            }
            StaffPosition::Scout => {
                s(8, knowledge.judging_player_ability)
                    + s(8, knowledge.judging_player_potential)
                    + s(5, mental.adaptability)
                    + s(4, knowledge.tactical_knowledge)
            }
            StaffPosition::ChiefScout | StaffPosition::HeadOfRecruitment => {
                s(10, knowledge.judging_player_ability)
                    + s(10, knowledge.judging_player_potential)
                    + s(6, mental.adaptability)
                    + s(5, mental.man_management)
                    + s(4, knowledge.tactical_knowledge)
            }
            StaffPosition::DataAnalyst => {
                s(10, data.judging_player_data)
                    + s(8, data.judging_team_data)
                    + s(6, data.presenting_data)
            }
            StaffPosition::DirectorOfFootball => {
                // Strategic role — knowledge of squad construction, plus
                // soft skills to manage the manager.
                s(8, knowledge.judging_player_ability)
                    + s(8, knowledge.judging_player_potential)
                    + s(6, mental.man_management)
                    + s(4, knowledge.tactical_knowledge)
            }
            // Executive / non-coaching roles — very few discriminating
            // attributes; fall back to determination + adaptability so a
            // candidate with usable mental traits still ranks above an
            // empty profile.
            StaffPosition::Chairman
            | StaffPosition::Director
            | StaffPosition::ManagingDirector
            | StaffPosition::GeneralManager
            | StaffPosition::MediaPundit
            | StaffPosition::Free => {
                s(5, mental.determination) + s(4, mental.adaptability)
            }
        }
    }

    fn position_group_index(group: PlayerFieldPositionGroup) -> usize {
        match group {
            PlayerFieldPositionGroup::Goalkeeper => 0,
            PlayerFieldPositionGroup::Defender => 1,
            PlayerFieldPositionGroup::Midfielder => 2,
            PlayerFieldPositionGroup::Forward => 3,
        }
    }

    /// Training-gain bonus multiplier for a given position group based on
    /// accumulated specialization days. Returns 1.0 for untrained groups
    /// and up to 1.40 for deeply specialized ones (~1500+ days).
    pub fn specialization_bonus(&self, group: PlayerFieldPositionGroup) -> f32 {
        let idx = Self::position_group_index(group);
        let days = self.specialization_days[idx] as f32;
        if days < 200.0 {
            1.0
        } else if days < 600.0 {
            1.0 + (days - 200.0) / 400.0 * 0.12
        } else if days < 1500.0 {
            1.12 + (days - 600.0) / 900.0 * 0.16
        } else {
            (1.28 + (days - 1500.0) / 2000.0 * 0.12).min(1.40)
        }
    }

    /// Record a training day for this coach focused on `group`.
    /// Called from the team training tick.
    pub fn accrue_specialization(&mut self, group: PlayerFieldPositionGroup, days: u32) {
        let idx = Self::position_group_index(group);
        self.specialization_days[idx] = self.specialization_days[idx].saturating_add(days);
    }

    pub fn add_event(&mut self, event_type: StaffEventType) {
        self.recent_events.push(StaffEvent {
            event_type,
            days_ago: 0,
        });
        if self.recent_events.len() > 10 {
            self.recent_events.remove(0);
        }
    }

    pub fn decay_events(&mut self) {
        for event in &mut self.recent_events {
            event.days_ago += 7;
        }
        self.recent_events.retain(|e| e.days_ago <= 60);
    }

    pub fn simulate(&mut self, ctx: GlobalContext<'_>) -> StaffResult {
        let now = ctx.simulation.date;
        let mut result = StaffResult::new(self.id);

        // Weekly event decay
        if ctx.simulation.is_week_beginning() {
            self.decay_events();
        }

        // Birthday handling - improves mood
        if DateUtils::is_birthday(self.birth_date, now.date()) {
            self.behaviour.try_increase();
            self.job_satisfaction = (self.job_satisfaction + 5.0).min(100.0);
            result.add_event(StaffMoraleEvent::Birthday);
            self.add_event(StaffEventType::Birthday);
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
        let mut negotiation_started = false;

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
                    negotiation_started = true;

                    if self.job_satisfaction < 40.0 {
                        // Unhappy - likely to leave
                        result.contract.likely_to_leave = true;
                    }
                }

                // Staff member initiates renewal discussion
                if self.attributes.ambition > 15.0
                    && self.recent_performance.training_effectiveness > 0.7
                {
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

        if negotiation_started {
            self.add_event(StaffEventType::ContractNegotiation);
        }
    }

    fn update_fatigue(&mut self, ctx: &GlobalContext<'_>, result: &mut StaffResult) {
        // Calculate workload based on responsibilities
        let workload = self.calculate_workload(ctx);

        // Increase fatigue based on workload
        self.fatigue += workload;

        // Daily natural recovery
        self.fatigue -= 2.0;

        // Recovery on weekends
        if ctx.simulation.date.weekday() == chrono::Weekday::Sun
            || ctx.simulation.date.weekday() == chrono::Weekday::Sat
        {
            self.fatigue -= 5.0;
        }

        // Vacation periods provide significant recovery (off-season)
        let season = ctx
            .country
            .as_ref()
            .map(|c| c.season_dates)
            .unwrap_or_default();
        if season.is_off_season(ctx.simulation.date.date()) {
            self.fatigue = (self.fatigue - 30.0).max(0.0);
        }

        // Clamp fatigue to 0-100
        self.fatigue = self.fatigue.clamp(0.0, 100.0);

        // High fatigue affects performance and morale
        if self.fatigue > 80.0 {
            result.add_warning(StaffWarning::HighFatigue);
            self.recent_performance.training_effectiveness *= 0.8;
            self.job_satisfaction -= 2.0;
            self.add_event(StaffEventType::HighFatigue);
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
        let today_session = self
            .get_todays_training(ctx.simulation.date)
            .map(|s| (s.session_type.clone(), s.intensity.clone()));

        if let Some((session_type, intensity)) = today_session {
            // Training effectiveness based on various factors
            let effectiveness = self.calculate_training_effectiveness();

            result.training.session_conducted = true;
            result.training.effectiveness = effectiveness;
            result.training.session_type = session_type;
            self.add_event(StaffEventType::TrainingConducted);

            // Track which players attended
            if let Some(team_id) = ctx.team.as_ref().map(|t| t.id) {
                result.training.team_id = Some(team_id);
            }

            // Fatigue from conducting training
            self.fatigue += match intensity {
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
        self.job_satisfaction =
            (self.job_satisfaction + satisfaction_change * 0.5).clamp(0.0, 100.0);

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
        if ctx.simulation.date.hour() == 12 {
            // Small random relationship events
            if rand::random::<f32>() < 0.1 {
                result.relationship_event = Some(RelationshipEvent::PositiveInteraction);
                self.add_event(StaffEventType::PositiveInteraction);
            } else if rand::random::<f32>() < 0.05 && self.job_satisfaction < 40.0 {
                result.relationship_event = Some(RelationshipEvent::Conflict);
                self.add_event(StaffEventType::Conflict);
            } else if rand::random::<f32>() < 0.02 && self.job_satisfaction > 70.0 {
                result.relationship_event = Some(RelationshipEvent::TrustBuilt);
                self.add_event(StaffEventType::TrustBuilt);
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
            self.add_event(StaffEventType::PerformanceDeclined);
        }

        // Board/management reaction to performance
        if self.recent_performance.training_effectiveness < 0.4 {
            result.add_warning(StaffWarning::PoorPerformance);
        } else if self.recent_performance.training_effectiveness > 0.8 {
            result.add_event(StaffMoraleEvent::ExcellentPerformance);
            self.job_satisfaction += 5.0;
            self.add_event(StaffEventType::PerformanceExcellent);
        }
    }

    fn process_professional_development(
        &mut self,
        ctx: &GlobalContext<'_>,
        result: &mut StaffResult,
    ) {
        // Check for license upgrade opportunities
        if self.should_upgrade_license() {
            if rand::random::<f32>() < 0.01 {
                // Small daily chance
                result.license_upgrade_available = true;
                self.add_event(StaffEventType::LicenseUpgrade);

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
            self.fatigue = (self.fatigue - 5.0).max(0.0); // Courses are refreshing
            self.add_event(StaffEventType::ProfessionalDevelopment);
        }
    }

    fn process_scouting(&mut self, _ctx: &GlobalContext<'_>, _result: &mut StaffResult) {
        // Real scouting is now handled at country level (Country::process_scouting)
        // which has access to all clubs and players for cross-club evaluation.
    }

    // Helper methods

    fn wants_renewal(&self) -> bool {
        self.job_satisfaction > 40.0 && self.behaviour.state != PersonBehaviourState::Poor
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
            Some(StaffPosition::Manager) => 5.0,
            Some(StaffPosition::AssistantManager) => 4.0,
            Some(StaffPosition::Coach) => 3.0,
            Some(StaffPosition::FitnessCoach) => 2.5,
            Some(StaffPosition::GoalkeeperCoach) => 2.0,
            Some(StaffPosition::Scout) | Some(StaffPosition::ChiefScout) => 2.0,
            Some(StaffPosition::Physio) => 3.0,
            _ => 2.0,
        };

        // Additional load from training sessions
        let training_load = self.training_schedule.len() as f32 * 0.3;

        position_load + training_load
    }

    fn has_coaching_duties(&self) -> bool {
        matches!(
            self.contract.as_ref().map(|c| &c.position),
            Some(StaffPosition::Manager)
                | Some(StaffPosition::AssistantManager)
                | Some(StaffPosition::Coach)
                | Some(StaffPosition::FitnessCoach)
                | Some(StaffPosition::GoalkeeperCoach)
                | Some(StaffPosition::FirstTeamCoach)
                | Some(StaffPosition::YouthCoach)
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
            _ => return None, // No training on weekends
        };

        self.training_schedule.get(index)
    }

    fn calculate_training_effectiveness(&self) -> f32 {
        let base = (self.staff_attributes.coaching.technical as f32
            + self.staff_attributes.coaching.tactical as f32
            + self.staff_attributes.coaching.fitness as f32
            + self.staff_attributes.coaching.mental as f32)
            / 80.0;

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

        let skill_multiplier = (self.staff_attributes.coaching.tactical as f32
            + self.staff_attributes.coaching.technical as f32)
            / 40.0
            + 0.5;

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

        prob.min(0.9) // Cap at 90% chance
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
            true // First evaluation
        }
    }

    fn calculate_performance_metrics(&self, ctx: &GlobalContext<'_>) -> StaffPerformance {
        // Simplified calculation - would need actual team/player data
        StaffPerformance {
            training_effectiveness: self.calculate_training_effectiveness(),
            player_development_rate: (self.staff_attributes.coaching.working_with_youngsters
                as f32
                / 20.0),
            injury_prevention_rate: (self.staff_attributes.medical.sports_science as f32 / 20.0),
            tactical_implementation: (self.staff_attributes.coaching.tactical as f32 / 20.0),
            last_evaluation_date: Some(ctx.simulation.date.date()),
        }
    }

    fn should_upgrade_license(&self) -> bool {
        // Check if eligible for license upgrade
        match self.license {
            StaffLicenseType::NationalC => self.staff_attributes.coaching.tactical > 10,
            StaffLicenseType::NationalB => {
                self.staff_attributes.coaching.tactical > 12
                    && self.staff_attributes.coaching.technical > 12
            }
            StaffLicenseType::NationalA => {
                self.staff_attributes.coaching.tactical > 14
                    && self.staff_attributes.coaching.technical > 14
            }
            _ => false, // Continental licenses need special conditions
        }
    }

    fn improve_attributes_from_experience(&mut self) {
        let is_scout = self
            .contract
            .as_ref()
            .map(|c| matches!(c.position, StaffPosition::Scout | StaffPosition::ChiefScout))
            .unwrap_or(false);

        if is_scout {
            // Scouts sharpen judging skills by doing the job.
            // Higher ceiling for Chief Scouts means faster gains at the top end.
            if rand::random::<f32>() < 0.25 {
                let improvement = 1;
                match rand::random::<u8>() % 3 {
                    0 => {
                        self.staff_attributes.knowledge.judging_player_ability =
                            (self.staff_attributes.knowledge.judging_player_ability + improvement)
                                .min(20)
                    }
                    1 => {
                        self.staff_attributes.knowledge.judging_player_potential =
                            (self.staff_attributes.knowledge.judging_player_potential + improvement)
                                .min(20)
                    }
                    _ => {
                        self.staff_attributes.data_analysis.judging_player_data =
                            (self.staff_attributes.data_analysis.judging_player_data + improvement)
                                .min(20)
                    }
                }
            }
            return;
        }

        // Slow improvement over time
        if rand::random::<f32>() < 0.3 {
            // Small chance of improvement each month
            let improvement = 1;

            // Improve a random coaching attribute
            match rand::random::<u8>() % 6 {
                0 => {
                    self.staff_attributes.coaching.attacking =
                        (self.staff_attributes.coaching.attacking + improvement).min(20)
                }
                1 => {
                    self.staff_attributes.coaching.defending =
                        (self.staff_attributes.coaching.defending + improvement).min(20)
                }
                2 => {
                    self.staff_attributes.coaching.tactical =
                        (self.staff_attributes.coaching.tactical + improvement).min(20)
                }
                3 => {
                    self.staff_attributes.coaching.technical =
                        (self.staff_attributes.coaching.technical + improvement).min(20)
                }
                4 => {
                    self.staff_attributes.coaching.fitness =
                        (self.staff_attributes.coaching.fitness + improvement).min(20)
                }
                _ => {
                    self.staff_attributes.coaching.mental =
                        (self.staff_attributes.coaching.mental + improvement).min(20)
                }
            }
        }
    }

    fn is_on_course(&self, date: NaiveDate) -> bool {
        // Simplified - courses in January
        date.month() == 1 && date.day() >= 15 && date.day() <= 20
    }
}

#[derive(Debug, Clone, Default)]
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

#[derive(Debug, Clone, Default)]
pub struct StaffTrainingResult {
    pub sessions_planned: u8,
    pub session_conducted: bool,
    pub effectiveness: f32,
    pub session_type: TrainingType,
    pub team_id: Option<u32>,
}

#[derive(Debug, Clone)]
pub enum StaffWarning {
    HighFatigue,
    BurnoutRisk,
    LowMorale,
    PoorPerformance,
}

#[derive(Debug, Clone)]
pub enum StaffMoraleEvent {
    Birthday,
    HighSatisfaction,
    ExcellentPerformance,
}

#[derive(Debug, Clone)]
pub enum ResignationReason {
    LowSatisfaction,
    Burnout,
    PersonalReasons,
    BetterOpportunity,
    Retirement,
}

#[derive(Debug, Clone)]
pub enum HealthIssue {
    StressRelated,
    PhysicalInjury,
    Illness,
}

#[derive(Debug, Clone)]
pub enum RelationshipEvent {
    PositiveInteraction,
    Conflict,
    MentorshipStarted,
    TrustBuilt,
}

#[derive(Debug, Clone)]
pub enum StaffLicenseType {
    ContinentalPro,
    ContinentalA,
    ContinentalB,
    ContinentalC,
    NationalA,
    NationalB,
    NationalC,
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::StaffStatus;

    fn make_contracted_staff(id: u32, position: StaffPosition) -> Staff {
        let mut staff = StaffStub::default();
        staff.id = id;
        staff.contract = Some(StaffClubContract::new(
            50_000,
            chrono::NaiveDate::from_ymd_opt(2030, 6, 30).unwrap(),
            position,
            StaffStatus::Active,
        ));
        staff
    }

    #[test]
    fn manager_selection_picks_highest_tactical_man_management() {
        let mut weak = make_contracted_staff(1, StaffPosition::Manager);
        let mut strong = make_contracted_staff(2, StaffPosition::Manager);
        weak.staff_attributes.knowledge.tactical_knowledge = 5;
        weak.staff_attributes.mental.man_management = 5;
        strong.staff_attributes.knowledge.tactical_knowledge = 18;
        strong.staff_attributes.mental.man_management = 17;
        // Insert "weak" first so the old "first match" logic would pick it.
        let collection = StaffCollection::new(vec![weak, strong]);
        // get_by_position is the routing-by-relevance entry point; the
        // separate `manager()` helper uses `find_by_any_position` which
        // is intentionally first-match for the seat-filling check.
        let chosen = collection.get_by_position(StaffPosition::Manager);
        assert_eq!(chosen.id, 2);
    }

    #[test]
    fn goalkeeper_coach_selection_prefers_gk_specialism_over_outfield_skills() {
        // Both are GoalkeeperCoaches. One is a strong outfield coach but
        // mediocre GK specialist; the other is a weak outfield coach but
        // strong GK specialist. Selection must follow the GK signal.
        let mut outfield_strong = make_contracted_staff(1, StaffPosition::GoalkeeperCoach);
        outfield_strong.staff_attributes.coaching.technical = 18;
        outfield_strong.staff_attributes.goalkeeping.shot_stopping = 5;
        outfield_strong.staff_attributes.goalkeeping.handling = 5;
        outfield_strong.staff_attributes.goalkeeping.distribution = 5;

        let mut gk_specialist = make_contracted_staff(2, StaffPosition::GoalkeeperCoach);
        gk_specialist.staff_attributes.coaching.technical = 5;
        gk_specialist.staff_attributes.goalkeeping.shot_stopping = 18;
        gk_specialist.staff_attributes.goalkeeping.handling = 17;
        gk_specialist.staff_attributes.goalkeeping.distribution = 16;

        let collection = StaffCollection::new(vec![outfield_strong, gk_specialist]);
        let chosen = collection.get_by_position(StaffPosition::GoalkeeperCoach);
        assert_eq!(chosen.id, 2);
    }

    #[test]
    fn physio_selection_prefers_physiotherapy_and_sports_science() {
        let mut weak = make_contracted_staff(1, StaffPosition::Physio);
        weak.staff_attributes.medical.physiotherapy = 5;
        weak.staff_attributes.medical.sports_science = 5;
        let mut strong = make_contracted_staff(2, StaffPosition::Physio);
        strong.staff_attributes.medical.physiotherapy = 18;
        strong.staff_attributes.medical.sports_science = 16;
        let collection = StaffCollection::new(vec![weak, strong]);
        let chosen = collection.get_by_position(StaffPosition::Physio);
        assert_eq!(chosen.id, 2);
    }

    #[test]
    fn scout_selection_prefers_judging_attributes() {
        let mut generalist = make_contracted_staff(1, StaffPosition::Scout);
        generalist.staff_attributes.knowledge.judging_player_ability = 6;
        generalist.staff_attributes.knowledge.judging_player_potential = 6;
        generalist.staff_attributes.mental.adaptability = 18;
        let mut specialist = make_contracted_staff(2, StaffPosition::Scout);
        specialist.staff_attributes.knowledge.judging_player_ability = 17;
        specialist.staff_attributes.knowledge.judging_player_potential = 18;
        specialist.staff_attributes.mental.adaptability = 8;
        let collection = StaffCollection::new(vec![generalist, specialist]);
        let chosen = collection.get_by_position(StaffPosition::Scout);
        assert_eq!(chosen.id, 2);
    }

    #[test]
    fn youth_coach_selection_prefers_working_with_youngsters() {
        let mut technical = make_contracted_staff(1, StaffPosition::YouthCoach);
        technical.staff_attributes.coaching.working_with_youngsters = 5;
        technical.staff_attributes.coaching.technical = 18;
        technical.staff_attributes.coaching.tactical = 18;
        let mut youth_focus = make_contracted_staff(2, StaffPosition::YouthCoach);
        youth_focus.staff_attributes.coaching.working_with_youngsters = 18;
        youth_focus.staff_attributes.coaching.technical = 8;
        youth_focus.staff_attributes.coaching.tactical = 8;
        let collection = StaffCollection::new(vec![technical, youth_focus]);
        let chosen = collection.get_by_position(StaffPosition::YouthCoach);
        assert_eq!(chosen.id, 2);
    }

    #[test]
    fn empty_position_falls_back_to_stub() {
        // Collection has no managers — a Manager lookup should return
        // the stub (id 0), not panic.
        let coach = make_contracted_staff(1, StaffPosition::Coach);
        let collection = StaffCollection::new(vec![coach]);
        let chosen = collection.get_by_position(StaffPosition::Manager);
        assert_eq!(chosen.id, 0);
    }
}
