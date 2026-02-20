use crate::simulator::SimulatorData;
use crate::{ChangeType, HappinessEventType, HealthIssue, RelationshipChange, RelationshipEvent, ResignationReason, StaffContractResult, StaffMoraleEvent, StaffTrainingResult, StaffWarning};

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
                    // Pick a random player and give a small positive relationship update
                    if let Some(player) = Self::random_player(data) {
                        let change = RelationshipChange::positive(
                            ChangeType::CoachingSuccess,
                            0.5,
                        );
                        player.relations.update_staff_relationship(self.staff_id, change, sim_date);
                    }
                }
                RelationshipEvent::Conflict => {
                    // Small negative relationship with a random player
                    if let Some(player) = Self::random_player(data) {
                        let change = RelationshipChange::negative(
                            ChangeType::TacticalDisagreement,
                            0.3,
                        );
                        player.relations.update_staff_relationship(self.staff_id, change, sim_date);

                        // Also affect player morale slightly
                        player.happiness.add_event(HappinessEventType::ManagerDiscipline, -1.0);
                    }
                }
                RelationshipEvent::MentorshipStarted => {
                    if let Some(player) = Self::random_player(data) {
                        let change = RelationshipChange::positive(
                            ChangeType::PersonalSupport,
                            0.8,
                        );
                        player.relations.update_staff_relationship(self.staff_id, change, sim_date);
                    }
                }
                RelationshipEvent::TrustBuilt => {
                    if let Some(player) = Self::random_player(data) {
                        let change = RelationshipChange::positive(
                            ChangeType::CoachingSuccess,
                            0.6,
                        );
                        player.relations.update_staff_relationship(self.staff_id, change, sim_date);
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

    fn random_player(data: &mut SimulatorData) -> Option<&mut crate::Player> {
        // Pick a random player from the simulation data
        let player_count: usize = data.continents.iter()
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
