use crate::simulator::SimulatorData;
use crate::{
    ChangeType, HappinessEventCause, HappinessEventContext, HappinessEventEvidence,
    HappinessEventFollowUp, HappinessEventScope, HappinessEventSeverity, HappinessEventType,
    HealthIssue, Player, RelationshipChange, RelationshipEvent, ResignationReason,
    StaffContractResult, StaffMoraleEvent, StaffTrainingResult, StaffWarning, SupportEventContext,
    SupportMatchPhase, SupportSetting, SupportSource, SupportTrigger,
};

pub struct StaffCollectionResult {
    pub staff: Vec<StaffResult>,
}

impl StaffCollectionResult {
    pub fn new(staff: Vec<StaffResult>) -> Self {
        StaffCollectionResult { staff }
    }

    pub fn process(&self, data: &mut SimulatorData) {
        for staff_result in &self.staff {
            staff_result.process(data);
        }
    }
}

#[derive(Debug, Clone)]
pub struct ScoutingReport {
    pub player_id: u32,
    pub assessed_ability: u8,
    pub assessed_potential: u8,
    pub recommendation: ScoutRecommendation,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScoutRecommendation {
    Sign,
    Monitor,
    Pass,
}

// Enhanced StaffResult with staff_id
pub struct StaffResult {
    pub staff_id: u32,
    pub transfer_requests: Vec<u32>,
    pub contract: StaffContractResult,
    pub training: StaffTrainingResult,
    pub resigned: bool,
    pub resignation_reason: Option<ResignationReason>,
    pub resignation_risk: bool,
    pub performance_improved: bool,
    pub performance_declined: bool,
    pub license_upgrade_available: bool,
    pub wants_license_upgrade: bool,
    pub on_professional_development: bool,
    pub health_issue: Option<HealthIssue>,
    pub relationship_event: Option<RelationshipEvent>,
    pub warnings: Vec<StaffWarning>,
    pub events: Vec<StaffMoraleEvent>,
    pub scouting_reports: Vec<ScoutingReport>,
}

impl StaffResult {
    pub fn new(staff_id: u32) -> Self {
        StaffResult {
            staff_id,
            transfer_requests: Vec::new(),
            contract: StaffContractResult::default(),
            training: StaffTrainingResult::default(),
            resigned: false,
            resignation_reason: None,
            resignation_risk: false,
            performance_improved: false,
            performance_declined: false,
            license_upgrade_available: false,
            wants_license_upgrade: false,
            on_professional_development: false,
            health_issue: None,
            relationship_event: None,
            warnings: Vec::new(),
            events: Vec::new(),
            scouting_reports: Vec::new(),
        }
    }

    pub fn request_transfer(&mut self, player_id: u32) {
        self.transfer_requests.push(player_id);
    }

    pub fn add_warning(&mut self, warning: StaffWarning) {
        self.warnings.push(warning);
    }

    pub fn add_event(&mut self, event: StaffMoraleEvent) {
        self.events.push(event);
    }

    pub fn process(&self, data: &mut SimulatorData) {
        let sim_date = data.date.date();

        // Process relationship events with random players
        if let Some(ref event) = self.relationship_event {
            match event {
                RelationshipEvent::PositiveInteraction => {
                    if let Some(player) = Self::random_player(data) {
                        let change = RelationshipChange::positive(ChangeType::CoachingSuccess, 0.5);
                        player
                            .relations
                            .update_staff_relationship(self.staff_id, change, sim_date);
                        let ctx = StaffSupportContextBuilder::manager_encouragement(
                            self.staff_id,
                            player,
                            SupportTrigger::Generic,
                            SupportSetting::PrivateTalk,
                            1.5,
                        );
                        player.happiness.add_event_with_context(
                            HappinessEventType::ManagerEncouragement,
                            1.5,
                            None,
                            ctx,
                        );
                    }
                }
                RelationshipEvent::Conflict => {
                    if let Some(player) = Self::random_player(data) {
                        let change =
                            RelationshipChange::negative(ChangeType::TacticalDisagreement, 0.3);
                        player
                            .relations
                            .update_staff_relationship(self.staff_id, change, sim_date);
                        player
                            .happiness
                            .add_event(HappinessEventType::ManagerCriticism, -2.0);
                    }
                }
                RelationshipEvent::MentorshipStarted => {
                    if let Some(player) = Self::random_player(data) {
                        let change = RelationshipChange::positive(ChangeType::PersonalSupport, 0.8);
                        player
                            .relations
                            .update_staff_relationship(self.staff_id, change, sim_date);
                        let ctx = StaffSupportContextBuilder::manager_encouragement(
                            self.staff_id,
                            player,
                            SupportTrigger::LeadershipMoment,
                            SupportSetting::TrainingGround,
                            2.0,
                        );
                        player.happiness.add_event_with_context(
                            HappinessEventType::ManagerEncouragement,
                            2.0,
                            None,
                            ctx,
                        );
                    }
                }
                RelationshipEvent::TrustBuilt => {
                    if let Some(player) = Self::random_player(data) {
                        let change = RelationshipChange::positive(ChangeType::CoachingSuccess, 0.6);
                        player
                            .relations
                            .update_staff_relationship(self.staff_id, change, sim_date);
                        player
                            .happiness
                            .add_event_default(HappinessEventType::ManagerTacticalInstruction);
                    }
                }
            }
        }

        // Process performance events that affect staff morale on players
        for event in &self.events {
            match event {
                StaffMoraleEvent::ExcellentPerformance => {
                    // Good coaching performance gives small morale boost to all players
                    // (handled via training results, no extra action needed here)
                }
                _ => {}
            }
        }
    }

    fn random_player(data: &mut SimulatorData) -> Option<&mut Player> {
        // Pick a random player from the simulation data
        let player_count: usize = data
            .continents
            .iter()
            .flat_map(|c| &c.countries)
            .flat_map(|c| &c.clubs)
            .flat_map(|c| &c.teams.teams)
            .map(|t| t.players.players.len())
            .sum();

        if player_count == 0 {
            return None;
        }

        let target = (rand::random::<f32>() * player_count as f32) as usize;
        let mut current = 0;

        for continent in &mut data.continents {
            for country in &mut continent.countries {
                for club in &mut country.clubs {
                    for team in &mut club.teams.teams {
                        for player in &mut team.players.players {
                            if current == target {
                                return Some(player);
                            }
                            current += 1;
                        }
                    }
                }
            }
        }

        None
    }
}

/// Builder for the structured `HappinessEventContext` payloads attached
/// to staff-driven `ManagerEncouragement` events (positive
/// interactions, mentorship moments). Bundled under a named type so the
/// processing branches in `StaffResult::process` read as thin
/// orchestration of context construction.
pub struct StaffSupportContextBuilder;

impl StaffSupportContextBuilder {
    /// Build a `HappinessEventContext` for a staff-driven manager
    /// encouragement gesture. Captures the source/setting/trigger and a
    /// small evidence list pulled from the player's current state so the
    /// renderer can describe what the manager saw.
    pub fn manager_encouragement(
        staff_id: u32,
        player: &Player,
        trigger: SupportTrigger,
        setting: SupportSetting,
        magnitude: f32,
    ) -> HappinessEventContext {
        let mut support = SupportEventContext::new(SupportSource::Manager, setting, trigger)
            .with_speaker_staff_id(staff_id);
        if player.happiness.morale < 35.0 {
            support = support.with_phase(SupportMatchPhase::InMatch);
        }

        let mut ctx = HappinessEventContext::new(
            HappinessEventCause::ManagerSupport,
            HappinessEventSeverity::from_magnitude(magnitude),
            match setting {
                SupportSetting::TrainingGround => HappinessEventScope::TrainingGround,
                SupportSetting::DressingRoom => HappinessEventScope::DressingRoom,
                _ => HappinessEventScope::Personal,
            },
        )
        .with_support_context(support);

        if player.happiness.morale < 35.0 {
            ctx = ctx.with_evidence(HappinessEventEvidence::PoorMoraleBeforeTalk);
        }
        if player.attributes.professionalism >= 15.0 {
            ctx = ctx.with_evidence(HappinessEventEvidence::HighProfessionalism);
        }
        if let Some(rel) = player.relations.get_staff(staff_id) {
            if rel.level >= 50.0 {
                ctx = ctx.with_evidence(HappinessEventEvidence::ManagerTrust);
                ctx = ctx.with_evidence(HappinessEventEvidence::StrongCoachRapport);
            } else if rel.level <= -25.0 {
                ctx = ctx.with_evidence(HappinessEventEvidence::WeakCoachRapport);
            }
        }

        ctx.with_follow_up(HappinessEventFollowUp::ManagerTrustRising)
    }
}
