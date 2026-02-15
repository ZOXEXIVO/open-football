use crate::simulator::SimulatorData;
use crate::{HealthIssue, RelationshipEvent, ResignationReason, StaffContractResult, StaffMoraleEvent, StaffTrainingResult, StaffWarning};

pub struct StaffCollectionResult {
    pub staff: Vec<StaffResult>,
}

impl StaffCollectionResult {
    pub fn new(staff: Vec<StaffResult>) -> Self {
        StaffCollectionResult { staff }
    }

    pub fn process(&self, _: &mut SimulatorData) {}
}

#[derive(Debug)]
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

// Enhanced StaffResult with new fields
pub struct StaffResult {
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
    pub fn new() -> Self {
        StaffResult {
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

    pub fn process(&self, _data: &mut SimulatorData) {

    }
}
