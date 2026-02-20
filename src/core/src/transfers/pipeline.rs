use crate::PlayerPositionType;
use chrono::NaiveDate;

// ============================================================
// Transfer Need Priority & Reason
// ============================================================

#[derive(Debug, Clone, PartialEq)]
pub enum TransferNeedPriority {
    Critical,
    Important,
    Optional,
}

/// Why the coach is requesting this position - derived from tactical analysis.
#[derive(Debug, Clone, PartialEq)]
pub enum TransferNeedReason {
    /// Formation requires this position and we have no one (e.g. 4-2-3-1 needs AMC, we have none)
    FormationGap,
    /// We have a player here but they're not good enough for our level
    QualityUpgrade,
    /// Only one player for a critical position - need backup
    DepthCover,
    /// Key player is aging, need successor within 1-2 seasons
    SuccessionPlanning,
    /// Young prospect with high potential to develop
    DevelopmentSigning,
}

#[derive(Debug, Clone, PartialEq)]
pub enum TransferRequestStatus {
    Pending,
    ScoutingActive,
    Shortlisted,
    Negotiating,
    Fulfilled,
    Abandoned,
}

// ============================================================
// TransferRequest - Coach tells DoF what the squad needs
// The coach says WHAT position and WHY; the DoF decides HOW (buy/loan)
// ============================================================

#[derive(Debug, Clone)]
pub struct TransferRequest {
    pub id: u32,
    pub position: PlayerPositionType,
    pub priority: TransferNeedPriority,
    pub reason: TransferNeedReason,
    pub min_ability: u8,
    pub ideal_ability: u8,
    pub preferred_age_min: u8,
    pub preferred_age_max: u8,
    pub budget_allocation: f64,
    pub status: TransferRequestStatus,
}

impl TransferRequest {
    pub fn new(
        id: u32,
        position: PlayerPositionType,
        priority: TransferNeedPriority,
        reason: TransferNeedReason,
        min_ability: u8,
        ideal_ability: u8,
        budget_allocation: f64,
    ) -> Self {
        // Age ranges based on the reason for the request - mirrors real-world logic
        let (age_min, age_max) = match reason {
            TransferNeedReason::FormationGap | TransferNeedReason::QualityUpgrade => {
                // Need someone ready now
                match priority {
                    TransferNeedPriority::Critical => (23, 30),
                    TransferNeedPriority::Important => (21, 29),
                    TransferNeedPriority::Optional => (20, 28),
                }
            }
            TransferNeedReason::DepthCover => (20, 32),
            TransferNeedReason::SuccessionPlanning => (19, 24),
            TransferNeedReason::DevelopmentSigning => (16, 21),
        };

        TransferRequest {
            id,
            position,
            priority,
            reason,
            min_ability,
            ideal_ability,
            preferred_age_min: age_min,
            preferred_age_max: age_max,
            budget_allocation,
            status: TransferRequestStatus::Pending,
        }
    }
}

// ============================================================
// PlayerObservation - Tracks multi-day observations per player
// ============================================================

#[derive(Debug, Clone)]
pub struct PlayerObservation {
    pub player_id: u32,
    pub observation_count: u32,
    pub assessed_ability: u8,
    pub assessed_potential: u8,
    pub confidence: f32,
    pub last_observed: NaiveDate,
}

impl PlayerObservation {
    pub fn new(player_id: u32, assessed_ability: u8, assessed_potential: u8, date: NaiveDate) -> Self {
        PlayerObservation {
            player_id,
            observation_count: 1,
            assessed_ability,
            assessed_potential,
            confidence: 0.3,
            last_observed: date,
        }
    }

    pub fn add_observation(
        &mut self,
        assessed_ability: u8,
        assessed_potential: u8,
        date: NaiveDate,
    ) {
        self.observation_count += 1;
        let weight = 1.0 / self.observation_count as f32;
        let old_weight = 1.0 - weight;
        self.assessed_ability =
            (old_weight * self.assessed_ability as f32 + weight * assessed_ability as f32) as u8;
        self.assessed_potential =
            (old_weight * self.assessed_potential as f32 + weight * assessed_potential as f32) as u8;
        self.confidence = 1.0 - (1.0 / (self.observation_count as f32 + 1.0));
        self.last_observed = date;
    }
}

// ============================================================
// ScoutingAssignment - DoF assigns scouts to find candidates
// ============================================================

#[derive(Debug, Clone)]
pub struct ScoutingAssignment {
    pub id: u32,
    pub transfer_request_id: u32,
    pub scout_staff_id: Option<u32>,
    pub target_position: PlayerPositionType,
    pub min_ability: u8,
    pub preferred_age_min: u8,
    pub preferred_age_max: u8,
    pub max_budget: f64,
    pub observations: Vec<PlayerObservation>,
    pub reports_produced: u32,
    pub completed: bool,
}

impl ScoutingAssignment {
    pub fn new(
        id: u32,
        transfer_request_id: u32,
        scout_staff_id: Option<u32>,
        target_position: PlayerPositionType,
        min_ability: u8,
        preferred_age_min: u8,
        preferred_age_max: u8,
        max_budget: f64,
    ) -> Self {
        ScoutingAssignment {
            id,
            transfer_request_id,
            scout_staff_id,
            target_position,
            min_ability,
            preferred_age_min,
            preferred_age_max,
            max_budget,
            observations: Vec::new(),
            reports_produced: 0,
            completed: false,
        }
    }

    pub fn find_observation_mut(&mut self, player_id: u32) -> Option<&mut PlayerObservation> {
        self.observations.iter_mut().find(|o| o.player_id == player_id)
    }

    pub fn has_observation_for(&self, player_id: u32) -> bool {
        self.observations.iter().any(|o| o.player_id == player_id)
    }
}

// ============================================================
// DetailedScoutingReport - Scout's final assessment (3+ obs)
// ============================================================

#[derive(Debug, Clone)]
pub struct DetailedScoutingReport {
    pub player_id: u32,
    pub assignment_id: u32,
    pub assessed_ability: u8,
    pub assessed_potential: u8,
    pub confidence: f32,
    pub estimated_value: f64,
    pub recommendation: ScoutingRecommendation,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ScoutingRecommendation {
    StrongBuy,
    Buy,
    Consider,
    Pass,
}

// ============================================================
// TransferShortlist - DoF's ranked candidate list per position
// ============================================================

#[derive(Debug, Clone, PartialEq)]
pub enum ShortlistCandidateStatus {
    Available,
    CurrentlyPursuing,
    NegotiationFailed,
    Signed,
    Unavailable,
}

/// How the DoF decided to pursue this candidate - determined at negotiation time.
#[derive(Debug, Clone, PartialEq)]
pub enum TransferApproach {
    /// Permanent transfer - club buys the player outright
    PermanentTransfer,
    /// Loan with option to buy
    LoanWithOption,
    /// Pure loan (temporary)
    Loan,
}

#[derive(Debug, Clone)]
pub struct ShortlistCandidate {
    pub player_id: u32,
    pub score: f32,
    pub estimated_fee: f64,
    pub status: ShortlistCandidateStatus,
}

#[derive(Debug, Clone)]
pub struct TransferShortlist {
    pub transfer_request_id: u32,
    pub candidates: Vec<ShortlistCandidate>,
    pub allocated_budget: f64,
    pub current_pursuit_index: usize,
}

impl TransferShortlist {
    pub fn new(transfer_request_id: u32, allocated_budget: f64) -> Self {
        TransferShortlist {
            transfer_request_id,
            candidates: Vec::new(),
            allocated_budget,
            current_pursuit_index: 0,
        }
    }

    pub fn current_candidate(&self) -> Option<&ShortlistCandidate> {
        self.candidates.get(self.current_pursuit_index)
    }

    pub fn current_candidate_mut(&mut self) -> Option<&mut ShortlistCandidate> {
        self.candidates.get_mut(self.current_pursuit_index)
    }

    pub fn advance_to_next(&mut self) -> bool {
        self.current_pursuit_index += 1;
        self.current_pursuit_index < self.candidates.len()
    }

    pub fn all_exhausted(&self) -> bool {
        self.current_pursuit_index >= self.candidates.len()
    }

    pub fn has_pursuing_candidate(&self) -> bool {
        self.candidates.iter().any(|c| c.status == ShortlistCandidateStatus::CurrentlyPursuing)
    }
}

// ============================================================
// LoanOutCandidate - Players identified for loan out
// ============================================================

#[derive(Debug, Clone, PartialEq)]
pub enum LoanOutReason {
    /// Young player needs regular first-team football to develop (elite/continental clubs)
    NeedsGameTime,
    /// Good player but blocked by better players in same position
    BlockedByBetterPlayer,
    /// Player surplus to squad requirements
    Surplus,
    /// Club needs to reduce wage bill
    FinancialRelief,
}

#[derive(Debug, Clone, PartialEq)]
pub enum LoanOutStatus {
    Identified,
    Listed,
    Negotiating,
    Completed,
}

#[derive(Debug, Clone)]
pub struct LoanOutCandidate {
    pub player_id: u32,
    pub reason: LoanOutReason,
    pub status: LoanOutStatus,
}

// ============================================================
// ClubTransferPlan - Top-level state per club
// ============================================================

#[derive(Debug, Clone)]
pub struct ClubTransferPlan {
    pub total_budget: f64,
    pub spent: f64,
    pub reserved: f64,

    pub transfer_requests: Vec<TransferRequest>,
    pub scouting_assignments: Vec<ScoutingAssignment>,
    pub scouting_reports: Vec<DetailedScoutingReport>,
    pub shortlists: Vec<TransferShortlist>,

    pub loan_out_candidates: Vec<LoanOutCandidate>,

    pub max_concurrent_negotiations: u32,
    pub active_negotiation_count: u32,

    pub next_request_id: u32,
    pub next_assignment_id: u32,

    pub last_evaluation_date: Option<NaiveDate>,
    pub initialized: bool,
}

impl ClubTransferPlan {
    pub fn new() -> Self {
        ClubTransferPlan {
            total_budget: 0.0,
            spent: 0.0,
            reserved: 0.0,
            transfer_requests: Vec::new(),
            scouting_assignments: Vec::new(),
            scouting_reports: Vec::new(),
            shortlists: Vec::new(),
            loan_out_candidates: Vec::new(),
            max_concurrent_negotiations: 2,
            active_negotiation_count: 0,
            next_request_id: 1,
            next_assignment_id: 1,
            last_evaluation_date: None,
            initialized: false,
        }
    }

    pub fn available_budget(&self) -> f64 {
        (self.total_budget - self.spent - self.reserved).max(0.0)
    }

    pub fn next_request_id(&mut self) -> u32 {
        let id = self.next_request_id;
        self.next_request_id += 1;
        id
    }

    pub fn next_assignment_id(&mut self) -> u32 {
        let id = self.next_assignment_id;
        self.next_assignment_id += 1;
        id
    }

    pub fn can_start_negotiation(&self) -> bool {
        self.active_negotiation_count < self.max_concurrent_negotiations
    }

    pub fn has_pending_requests(&self) -> bool {
        self.transfer_requests.iter().any(|r| r.status == TransferRequestStatus::Pending)
    }

    pub fn reset_for_window(&mut self) {
        self.transfer_requests.clear();
        self.scouting_assignments.clear();
        self.scouting_reports.clear();
        self.shortlists.clear();
        self.loan_out_candidates.clear();
        self.active_negotiation_count = 0;
        self.spent = 0.0;
        self.reserved = 0.0;
        self.initialized = false;
        self.last_evaluation_date = None;
    }
}

impl Default for ClubTransferPlan {
    fn default() -> Self {
        Self::new()
    }
}
