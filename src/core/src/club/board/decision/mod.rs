//! Discrete, explainable board decisions. The board's `simulate` emits a
//! list of these on `BoardResult`; `BoardResult::process` applies the ones
//! with real-world effects (budgets, facilities, ownership) to the club.
//! Everything carries a machine-readable reason so a future UI can render
//! "why" without re-deriving it.

/// Which club facility a board decision targets.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BoardFacility {
    Training,
    Youth,
    Academy,
    Recruitment,
    Stadium,
}

/// Machine-readable rationale for a decision. Stable variants so the UI /
/// tests can match without string parsing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DecisionReason {
    Overperformance,
    Underperformance,
    OwnerInjection,
    FfpPressure,
    FinancialDiscipline,
    WageControl,
    StrongFinances,
    StrategicPriority,
    LowPriority,
    DebtTooHigh,
    HighAttendanceDemand,
    ExceedsBudget,
    ConflictsWithVision,
    SquadProfileMismatch,
    EliteTalentException,
    CriticalSquadGap,
    SupporterPressure,
    BoardConfidenceCollapse,
}

/// A single board decision. Variants with payloads carry exactly what
/// `process` needs to apply them.
#[derive(Debug, Clone, PartialEq)]
pub enum BoardDecision {
    IssueManagerBacking,
    IssueFormalWarning,
    HoldCrisisMeeting,
    SackManager,
    IncreaseTransferBudget { amount: i64, reason: DecisionReason },
    CutTransferBudget { amount: i64, reason: DecisionReason },
    AdjustWageBudget { amount: i64, reason: DecisionReason },
    ApproveFacilityUpgrade { facility: BoardFacility, cost: i64 },
    RejectFacilityUpgrade { facility: BoardFacility, reason: DecisionReason },
    DemandPlayerSale { reason: DecisionReason },
    BlockTransfer { player_id: u32, reason: DecisionReason },
    ApproveTransferException { player_id: u32, reason: DecisionReason },
    StartTakeoverRumour,
    CompleteTakeover,
}

impl BoardDecision {
    /// Short, stable label for logging / UI. Wrapped here so call sites
    /// don't hand-roll match arms.
    pub fn label(&self) -> &'static str {
        match self {
            BoardDecision::IssueManagerBacking => "manager_backing",
            BoardDecision::IssueFormalWarning => "formal_warning",
            BoardDecision::HoldCrisisMeeting => "crisis_meeting",
            BoardDecision::SackManager => "sack_manager",
            BoardDecision::IncreaseTransferBudget { .. } => "increase_transfer_budget",
            BoardDecision::CutTransferBudget { .. } => "cut_transfer_budget",
            BoardDecision::AdjustWageBudget { .. } => "adjust_wage_budget",
            BoardDecision::ApproveFacilityUpgrade { .. } => "approve_facility_upgrade",
            BoardDecision::RejectFacilityUpgrade { .. } => "reject_facility_upgrade",
            BoardDecision::DemandPlayerSale { .. } => "demand_player_sale",
            BoardDecision::BlockTransfer { .. } => "block_transfer",
            BoardDecision::ApproveTransferException { .. } => "approve_transfer_exception",
            BoardDecision::StartTakeoverRumour => "takeover_rumour",
            BoardDecision::CompleteTakeover => "takeover_complete",
        }
    }

    pub fn is_sacking(&self) -> bool {
        matches!(self, BoardDecision::SackManager)
    }
}
