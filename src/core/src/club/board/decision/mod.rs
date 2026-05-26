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

/// Payload-free identity of a decision. One variant per `BoardDecision`
/// case so the UI / news / tests can group, filter, and key on a decision
/// without matching against payload fields. Stable: never reuse a name.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DecisionKind {
    IssueManagerBacking,
    IssueFormalWarning,
    HoldCrisisMeeting,
    SackManager,
    IncreaseTransferBudget,
    CutTransferBudget,
    AdjustWageBudget,
    ApproveFacilityUpgrade,
    RejectFacilityUpgrade,
    DemandPlayerSale,
    BlockTransfer,
    ApproveTransferException,
    StartTakeoverRumour,
    CompleteTakeover,
}

/// A single board decision. Variants with payloads carry exactly what
/// `process` needs to apply them.
///
/// Amount/cost conventions (all in club currency, always non-negative
/// magnitudes — the *direction* is implied by the variant):
/// * `IncreaseTransferBudget.amount` — added to the transfer budget.
/// * `CutTransferBudget.amount` — subtracted from the transfer budget.
/// * `AdjustWageBudget.amount` — signed; added to the wage budget (a
///   negative value trims it).
/// * `ApproveFacilityUpgrade.cost` — debited from club cash on apply.
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

    /// Payload-free kind, for grouping / filtering / stable test assertions.
    pub fn kind(&self) -> DecisionKind {
        match self {
            BoardDecision::IssueManagerBacking => DecisionKind::IssueManagerBacking,
            BoardDecision::IssueFormalWarning => DecisionKind::IssueFormalWarning,
            BoardDecision::HoldCrisisMeeting => DecisionKind::HoldCrisisMeeting,
            BoardDecision::SackManager => DecisionKind::SackManager,
            BoardDecision::IncreaseTransferBudget { .. } => DecisionKind::IncreaseTransferBudget,
            BoardDecision::CutTransferBudget { .. } => DecisionKind::CutTransferBudget,
            BoardDecision::AdjustWageBudget { .. } => DecisionKind::AdjustWageBudget,
            BoardDecision::ApproveFacilityUpgrade { .. } => DecisionKind::ApproveFacilityUpgrade,
            BoardDecision::RejectFacilityUpgrade { .. } => DecisionKind::RejectFacilityUpgrade,
            BoardDecision::DemandPlayerSale { .. } => DecisionKind::DemandPlayerSale,
            BoardDecision::BlockTransfer { .. } => DecisionKind::BlockTransfer,
            BoardDecision::ApproveTransferException { .. } => DecisionKind::ApproveTransferException,
            BoardDecision::StartTakeoverRumour => DecisionKind::StartTakeoverRumour,
            BoardDecision::CompleteTakeover => DecisionKind::CompleteTakeover,
        }
    }

    /// The machine-readable rationale, when the variant carries one. Budget,
    /// rejection, sale-demand and transfer-block/exception decisions explain
    /// *why*; meetings, sackings, approved upgrades and takeover events are
    /// self-describing and return `None`.
    pub fn reason(&self) -> Option<DecisionReason> {
        match self {
            BoardDecision::IncreaseTransferBudget { reason, .. }
            | BoardDecision::CutTransferBudget { reason, .. }
            | BoardDecision::AdjustWageBudget { reason, .. }
            | BoardDecision::RejectFacilityUpgrade { reason, .. }
            | BoardDecision::DemandPlayerSale { reason }
            | BoardDecision::BlockTransfer { reason, .. }
            | BoardDecision::ApproveTransferException { reason, .. } => Some(*reason),
            _ => None,
        }
    }

    /// True for exactly the decisions `BoardResult::process` turns into a
    /// concrete change to club state (budgets, facility levels, takeover
    /// cash injection). Everything else is advisory: it informs meetings,
    /// news, or the manager relationship, but `process` doesn't mutate the
    /// club from it. Keep this in lock-step with `apply_decisions`.
    pub fn is_actionable(&self) -> bool {
        matches!(
            self,
            BoardDecision::IncreaseTransferBudget { .. }
                | BoardDecision::CutTransferBudget { .. }
                | BoardDecision::AdjustWageBudget { .. }
                | BoardDecision::ApproveFacilityUpgrade { .. }
                | BoardDecision::CompleteTakeover
        )
    }

    /// True when the decision is something supporters / media would hear
    /// about — the kind of event a news feed should surface. Internal
    /// budget bookkeeping and private warnings stay quiet.
    pub fn is_public_newsworthy(&self) -> bool {
        matches!(
            self,
            BoardDecision::IssueManagerBacking
                | BoardDecision::SackManager
                | BoardDecision::ApproveFacilityUpgrade { .. }
                | BoardDecision::DemandPlayerSale { .. }
                | BoardDecision::StartTakeoverRumour
                | BoardDecision::CompleteTakeover
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One representative value per variant — used to exhaustively check
    /// the audit helpers. If a new `BoardDecision` variant is added the
    /// non-exhaustive match in `kind()` won't compile, forcing this list to
    /// be updated too.
    fn all_decisions() -> Vec<BoardDecision> {
        vec![
            BoardDecision::IssueManagerBacking,
            BoardDecision::IssueFormalWarning,
            BoardDecision::HoldCrisisMeeting,
            BoardDecision::SackManager,
            BoardDecision::IncreaseTransferBudget {
                amount: 1,
                reason: DecisionReason::OwnerInjection,
            },
            BoardDecision::CutTransferBudget {
                amount: 1,
                reason: DecisionReason::FfpPressure,
            },
            BoardDecision::AdjustWageBudget {
                amount: 1,
                reason: DecisionReason::WageControl,
            },
            BoardDecision::ApproveFacilityUpgrade {
                facility: BoardFacility::Training,
                cost: 1,
            },
            BoardDecision::RejectFacilityUpgrade {
                facility: BoardFacility::Training,
                reason: DecisionReason::DebtTooHigh,
            },
            BoardDecision::DemandPlayerSale {
                reason: DecisionReason::WageControl,
            },
            BoardDecision::BlockTransfer {
                player_id: 1,
                reason: DecisionReason::ExceedsBudget,
            },
            BoardDecision::ApproveTransferException {
                player_id: 1,
                reason: DecisionReason::EliteTalentException,
            },
            BoardDecision::StartTakeoverRumour,
            BoardDecision::CompleteTakeover,
        ]
    }

    #[test]
    fn every_variant_has_a_distinct_stable_kind() {
        let decisions = all_decisions();
        let mut kinds: Vec<DecisionKind> = decisions.iter().map(|d| d.kind()).collect();
        let count = kinds.len();
        kinds.sort_by_key(|k| format!("{k:?}"));
        kinds.dedup();
        assert_eq!(
            kinds.len(),
            count,
            "each decision variant must map to a unique kind"
        );
    }

    #[test]
    fn actionable_set_is_exactly_the_state_mutating_decisions() {
        // This is the contract `BoardResult::apply_decisions` relies on:
        // the actionable decisions are precisely budget/wage/facility/
        // takeover-injection changes.
        let actionable: Vec<DecisionKind> = all_decisions()
            .into_iter()
            .filter(|d| d.is_actionable())
            .map(|d| d.kind())
            .collect();
        let expected = [
            DecisionKind::IncreaseTransferBudget,
            DecisionKind::CutTransferBudget,
            DecisionKind::AdjustWageBudget,
            DecisionKind::ApproveFacilityUpgrade,
            DecisionKind::CompleteTakeover,
        ];
        assert_eq!(actionable.len(), expected.len());
        for k in expected {
            assert!(actionable.contains(&k), "missing actionable kind {k:?}");
        }
    }

    #[test]
    fn budget_and_rejection_decisions_carry_a_reason() {
        for d in all_decisions() {
            match d.kind() {
                DecisionKind::IncreaseTransferBudget
                | DecisionKind::CutTransferBudget
                | DecisionKind::AdjustWageBudget
                | DecisionKind::RejectFacilityUpgrade
                | DecisionKind::DemandPlayerSale
                | DecisionKind::BlockTransfer
                | DecisionKind::ApproveTransferException => {
                    assert!(d.reason().is_some(), "{:?} should carry a reason", d.kind());
                }
                _ => {}
            }
        }
    }

    #[test]
    fn sackings_and_takeovers_are_newsworthy_budget_tweaks_are_not() {
        assert!(BoardDecision::SackManager.is_public_newsworthy());
        assert!(BoardDecision::StartTakeoverRumour.is_public_newsworthy());
        assert!(
            !BoardDecision::CutTransferBudget {
                amount: 1,
                reason: DecisionReason::FfpPressure
            }
            .is_public_newsworthy()
        );
    }
}
