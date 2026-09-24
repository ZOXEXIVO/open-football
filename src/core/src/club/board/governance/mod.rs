//! The transfer hearing: what the board does when the recruitment team
//! puts a name in front of it.
//!
//! One question about money and a short list of things money cannot buy.
//! The money question is [`ClubBoard::hear`]: is the fee inside the
//! envelope this board priced for this PURPOSE, plus the rope its
//! temperament and its recent record earn it? Everything else here is a
//! veto about something other than price — a sporting case nobody believes,
//! a signing that contradicts the club's own plan, a dossier the scouts
//! cannot agree on. The manager can ask and the scouts can argue;
//! ownership still decides.

use crate::club::board::ClubBoard;
use crate::club::board::chairman::ChairmanAmbition;
use crate::club::board::mandate::{FeeEnvelope, SigningMandate};
use crate::club::board::ownership::OwnershipType;
use crate::club::board::strategy::SquadProfile;
use crate::club::board::vision::{FinancialStance, VisionYouthFocus};
use crate::transfers::pipeline::{TransferNeedPriority, TransferNeedReason};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BoardTransferDecision {
    Approved,
    Conditional(BoardTransferConcern),
    Vetoed(BoardTransferConcern),
}

impl BoardTransferDecision {
    pub fn is_approved(self) -> bool {
        matches!(
            self,
            BoardTransferDecision::Approved | BoardTransferDecision::Conditional(_)
        )
    }

    pub fn manager_satisfaction_delta(self, priority: &TransferNeedPriority) -> f32 {
        match self {
            BoardTransferDecision::Approved => match priority {
                TransferNeedPriority::Critical => 0.8,
                TransferNeedPriority::Important => 0.4,
                TransferNeedPriority::Optional => 0.1,
            },
            BoardTransferDecision::Conditional(_) => match priority {
                TransferNeedPriority::Critical => -0.8,
                TransferNeedPriority::Important => -0.4,
                TransferNeedPriority::Optional => 0.0,
            },
            BoardTransferDecision::Vetoed(_) => match priority {
                TransferNeedPriority::Critical => -4.5,
                TransferNeedPriority::Important => -2.75,
                TransferNeedPriority::Optional => -1.0,
            },
        }
    }

    pub fn loyalty_delta(self, priority: &TransferNeedPriority) -> i16 {
        match self {
            BoardTransferDecision::Approved => match priority {
                TransferNeedPriority::Critical => 1,
                _ => 0,
            },
            BoardTransferDecision::Conditional(_) => match priority {
                TransferNeedPriority::Critical => -1,
                _ => 0,
            },
            BoardTransferDecision::Vetoed(_) => match priority {
                TransferNeedPriority::Critical => -5,
                TransferNeedPriority::Important => -3,
                TransferNeedPriority::Optional => -1,
            },
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BoardTransferConcern {
    ExceedsTransferBudget,
    FinancialDiscipline,
    WeakSportingCase,
    ConflictsWithVision,
}

#[derive(Debug, Clone)]
pub struct BoardTransferProposal {
    pub fee: f64,
    pub allocated_budget: f64,
    pub remaining_transfer_budget: f64,
    pub priority: TransferNeedPriority,
    pub reason: TransferNeedReason,
    /// What the club says it is buying him for, and the minutes that
    /// implies. The hearing is about THIS, not about the player's age.
    pub mandate: SigningMandate,
    /// What the board's own doctrine says that purpose is worth.
    pub envelope: FeeEnvelope,
    pub player_age: Option<u8>,
    pub player_ability: Option<u8>,
    pub squad_avg_ability: u8,
    pub shortlist_score: f32,
    /// Optional recruitment-meeting dossier built from scout monitoring
    /// state. The confidence in it has already moved the envelope — what
    /// it still does here is veto a name the scouts openly disagree about.
    pub dossier: Option<BoardDossierSummary>,
    /// Optional financial/profile dossier on the deal. When present the
    /// board applies ownership-archetype governance (wage impact, resale,
    /// risk, manager priority).
    pub economics: Option<BoardTransferEconomics>,
}

/// Financial + profile snapshot of a proposed signing, used by the board's
/// ownership-archetype governance. Mirrors the recruitment dossier pattern
/// — present for pipeline calls, `None` elsewhere.
#[derive(Debug, Clone, Copy, Default)]
pub struct BoardTransferEconomics {
    /// Added annual wage this signing commits the club to.
    pub wage_impact_annual: f64,
    /// Remaining annual wage-budget headroom before the deal.
    pub wage_budget_headroom: f64,
    /// Agent fee on top of the transfer fee.
    pub agent_fee: f64,
    /// Proposed contract length in years.
    pub contract_length_years: u8,
    /// Projected resale value at the end of the deal.
    pub resale_projection: f64,
    /// Off-pitch risk 0..1 (1 = serious professionalism/discipline concern).
    pub professionalism_risk: f32,
    /// True when the player counts as home-grown / domestic for the club.
    pub homegrown_fit: bool,
    /// Injury proneness 0..1.
    pub injury_risk: f32,
    /// Commercial / shirt-sales appeal 0..1.
    pub commercial_value: f32,
    /// True when this is the manager's explicit priority target.
    pub manager_priority: bool,
}

/// Compact, board-facing snapshot of the recruitment dossier. We pull
/// only the fields the board actually reasons about so the board layer
/// stays decoupled from `pipeline::recruitment`.
#[derive(Debug, Clone, Copy, Default)]
pub struct BoardDossierSummary {
    pub scout_votes: u8,
    pub chief_scout_support: bool,
    pub avg_confidence: f32,
    pub avg_role_fit: f32,
    pub risk_flag_count: u8,
    /// Sum of weighted scout votes from the latest meeting on the player.
    pub consensus_score: f32,
    pub data_support: bool,
    pub matches_watched: u16,
}

impl ClubBoard {
    /// Board/chairman review of a proposed incoming transfer.
    ///
    /// The football committee layer: the head coach can ask, the
    /// recruitment team can shortlist, but ownership still weighs the fee
    /// against its own number for this purpose, and then against the
    /// things money cannot settle.
    pub fn hear(&self, proposal: &BoardTransferProposal) -> BoardTransferDecision {
        let remaining_budget = proposal.remaining_transfer_budget.max(0.0);
        if remaining_budget > 0.0 && proposal.fee > remaining_budget * 1.05 {
            return BoardTransferDecision::Vetoed(BoardTransferConcern::ExceedsTransferBudget);
        }

        // The one money question. `walk_away` is what the minutes this
        // mandate promises are worth at this club; `stretch` is how far
        // past its own number this board goes before it says no.
        let ceiling = proposal
            .envelope
            .ceiling(self.stretch(&proposal.priority).value() + self.identity_stretch(proposal));
        if proposal.fee > ceiling {
            return BoardTransferDecision::Vetoed(BoardTransferConcern::FinancialDiscipline);
        }

        if !self.is_sporting_case_credible(proposal) {
            return BoardTransferDecision::Vetoed(BoardTransferConcern::WeakSportingCase);
        }

        if self.transfer_conflicts_with_vision(proposal) {
            return BoardTransferDecision::Conditional(BoardTransferConcern::ConflictsWithVision);
        }

        // "Two scouts watching, consensus near zero" = open disagreement.
        // The board doesn't sign on a flip-coin.
        if let Some(d) = proposal.dossier
            && d.scout_votes >= 2
            && d.consensus_score.abs() < 0.4
            && d.risk_flag_count >= 2
        {
            return BoardTransferDecision::Vetoed(BoardTransferConcern::WeakSportingCase);
        }

        // Ownership-archetype governance: squad-profile fit + deal
        // economics (wage impact, resale, off-pitch risk).
        if let Some(decision) = self.review_governance(proposal) {
            return decision;
        }

        if proposal.fee > proposal.allocated_budget.max(1.0)
            || remaining_budget <= proposal.allocated_budget.max(1.0) * 0.25
        {
            return BoardTransferDecision::Conditional(BoardTransferConcern::FinancialDiscipline);
        }

        BoardTransferDecision::Approved
    }

    /// What the shirt's identity does to the rope.
    ///
    /// A member-owned board answers to people who watch the same players
    /// grow up: a local name earns the deal extra room and an import is
    /// looked at more coolly. No other ownership notices.
    fn identity_stretch(&self, proposal: &BoardTransferProposal) -> f64 {
        if !matches!(self.ownership.ownership_type, OwnershipType::MemberOwned) {
            return 0.0;
        }
        match proposal.economics {
            Some(e) if e.homegrown_fit => 0.20,
            Some(_) => -0.10,
            None => 0.0,
        }
    }

    /// Ownership-archetype governance layered on the base review:
    /// squad-profile fit plus deal economics (wage impact, resale, risk).
    /// Returns `Some` to override the base decision; `None` to defer to it.
    /// A `Balanced` profile with no economics dossier always returns `None`.
    fn review_governance(&self, proposal: &BoardTransferProposal) -> Option<BoardTransferDecision> {
        use BoardTransferConcern::*;

        // ── Squad profile fit ──
        if let Some(age) = proposal.player_age {
            let critical = matches!(proposal.priority, TransferNeedPriority::Critical);
            match self.vision.preferred_squad_profile {
                // Youth project: accept weaker-but-young; block ageing depth
                // outright (a hard veto, not a soft flag).
                SquadProfile::Youth if age >= 29 && !critical => {
                    return Some(BoardTransferDecision::Vetoed(ConflictsWithVision));
                }
                // Resale model: no point buying a player past resale age.
                SquadProfile::ResaleValue if age >= 30 && !critical => {
                    return Some(BoardTransferDecision::Conditional(ConflictsWithVision));
                }
                // Galáctico policy: signings must raise the bar.
                SquadProfile::Stars => {
                    if let Some(ability) = proposal.player_ability
                        && ability + 4 < proposal.squad_avg_ability
                    {
                        return Some(BoardTransferDecision::Vetoed(WeakSportingCase));
                    }
                }
                _ => {}
            }
        }

        // ── Deal economics ──
        let e = proposal.economics?;

        let elite_exception = matches!(
            self.chairman.ambition,
            ChairmanAmbition::Reckless | ChairmanAmbition::Ambitious
        ) && (matches!(proposal.priority, TransferNeedPriority::Critical)
            || proposal
                .player_ability
                .is_some_and(|a| a >= proposal.squad_avg_ability.saturating_add(10)));

        // Wage impact above the remaining headroom.
        if e.wage_impact_annual > e.wage_budget_headroom.max(0.0) {
            let austere = matches!(
                self.vision.financial_stance,
                FinancialStance::Conservative | FinancialStance::Austerity
            );
            if austere && !elite_exception {
                return Some(BoardTransferDecision::Vetoed(FinancialDiscipline));
            }
            if !elite_exception {
                return Some(BoardTransferDecision::Conditional(FinancialDiscipline));
            }
        }

        // Private-equity / resale owners dislike ageing players with weak
        // resale projection.
        if self.ownership.ownership_type.resale_driven() {
            let poor_resale = e.resale_projection < proposal.fee * 0.4;
            let ageing = proposal.player_age.is_some_and(|a| a >= 28);
            if poor_resale && ageing {
                return Some(BoardTransferDecision::Conditional(ConflictsWithVision));
            }
        }

        // Off-pitch risk worries prudent / fan-owned boards.
        if e.professionalism_risk >= 0.7
            && matches!(
                self.vision.financial_stance,
                FinancialStance::Conservative | FinancialStance::Austerity
            )
        {
            return Some(BoardTransferDecision::Conditional(WeakSportingCase));
        }

        None
    }

    fn is_sporting_case_credible(&self, proposal: &BoardTransferProposal) -> bool {
        if matches!(
            proposal.reason,
            TransferNeedReason::DevelopmentSigning
                | TransferNeedReason::CheapReinforcement
                | TransferNeedReason::SquadPadding
                | TransferNeedReason::InjuryCoverLoan
                | TransferNeedReason::LoanToFillSquad
        ) {
            return true;
        }

        let Some(ability) = proposal.player_ability else {
            return true;
        };

        let squad_avg = proposal.squad_avg_ability;
        ability.saturating_add(12) >= squad_avg || proposal.shortlist_score >= 0.95
    }

    fn transfer_conflicts_with_vision(&self, proposal: &BoardTransferProposal) -> bool {
        let Some(age) = proposal.player_age else {
            return false;
        };

        match self.vision.youth_focus {
            VisionYouthFocus::DevelopYouth => {
                age >= 30
                    && matches!(
                        proposal.reason,
                        TransferNeedReason::DevelopmentSigning
                            | TransferNeedReason::SuccessionPlanning
                            | TransferNeedReason::StaffRecommendation
                    )
            }
            VisionYouthFocus::SignExperienced => {
                age <= 20
                    && !matches!(
                        proposal.reason,
                        TransferNeedReason::DevelopmentSigning
                            | TransferNeedReason::SuccessionPlanning
                    )
            }
            VisionYouthFocus::Balanced => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::PlayerFieldPositionGroup;
    use crate::club::board::mandate::{MandateAuthor, MandatePurpose};
    use chrono::NaiveDate;

    fn transfer_proposal(
        fee: f64,
        walk_away: f64,
        priority: TransferNeedPriority,
        reason: TransferNeedReason,
    ) -> BoardTransferProposal {
        BoardTransferProposal {
            fee,
            allocated_budget: 1_000_000.0,
            remaining_transfer_budget: 10_000_000.0,
            priority,
            reason,
            mandate: SigningMandate::new(
                MandatePurpose::Starter,
                PlayerFieldPositionGroup::Midfielder,
                25,
                NaiveDate::from_ymd_opt(2028, 7, 1).unwrap(),
                MandateAuthor::Manager,
            ),
            envelope: FeeEnvelope {
                open: walk_away * 0.7,
                walk_away,
            },
            player_age: Some(25),
            player_ability: Some(65),
            squad_avg_ability: 60,
            shortlist_score: 1.0,
            dossier: None,
            economics: None,
        }
    }

    #[test]
    fn a_fee_past_the_board_s_own_number_is_refused() {
        // Re-pinned from `conservative_board_vetoes_excessive_transfer_overrun`:
        // the same refusal, decided against the doctrine's walk-away rather
        // than against a stack of fifteen tolerance constants.
        let mut board = ClubBoard::new();
        board.vision.financial_stance = FinancialStance::Conservative;
        board.chairman.ambition = ChairmanAmbition::Conservative;

        let proposal = transfer_proposal(
            2_000_000.0,
            1_000_000.0,
            TransferNeedPriority::Important,
            TransferNeedReason::QualityUpgrade,
        );

        assert!(matches!(
            board.hear(&proposal),
            BoardTransferDecision::Vetoed(BoardTransferConcern::FinancialDiscipline)
        ));
    }

    #[test]
    fn ambitious_board_backs_critical_squad_gap_within_its_envelope() {
        let mut board = ClubBoard::new();
        board.vision.financial_stance = FinancialStance::Ambitious;
        board.chairman.ambition = ChairmanAmbition::Ambitious;
        board.confidence.level = 80;

        let proposal = transfer_proposal(
            2_250_000.0,
            2_000_000.0,
            TransferNeedPriority::Critical,
            TransferNeedReason::FormationGap,
        );

        assert!(board.hear(&proposal).is_approved());
    }

    #[test]
    fn a_reckless_owner_stretches_further_than_a_prudent_one() {
        let mut bold = ClubBoard::new();
        bold.chairman.ambition = ChairmanAmbition::Reckless;
        bold.ownership.risk_tolerance = 90;
        let mut prudent = ClubBoard::new();
        prudent.chairman.ambition = ChairmanAmbition::Conservative;
        prudent.ownership.risk_tolerance = 20;

        let proposal = transfer_proposal(
            1_450_000.0,
            1_000_000.0,
            TransferNeedPriority::Important,
            TransferNeedReason::QualityUpgrade,
        );
        assert!(bold.hear(&proposal).is_approved());
        assert!(matches!(
            prudent.hear(&proposal),
            BoardTransferDecision::Vetoed(BoardTransferConcern::FinancialDiscipline)
        ));
    }

    #[test]
    fn a_written_off_window_narrows_the_next_hearing() {
        use crate::club::board::mandate::{MandateExit, MandateOutcome};

        let approved = transfer_proposal(
            1_450_000.0,
            1_000_000.0,
            TransferNeedPriority::Important,
            TransferNeedReason::QualityUpgrade,
        );
        let mut bold = ClubBoard::new();
        bold.chairman.ambition = ChairmanAmbition::Reckless;
        bold.ownership.risk_tolerance = 90;
        assert!(bold.hear(&approved).is_approved());

        // The same board, after writing off most of what it last spent.
        let wasted = SigningMandate::new(
            MandatePurpose::Starter,
            PlayerFieldPositionGroup::Midfielder,
            26,
            NaiveDate::from_ymd_opt(2028, 7, 1).unwrap(),
            MandateAuthor::Manager,
        )
        .with_money(40_000_000.0, 4_000_000.0);
        bold.mandate_ledger.push(MandateOutcome::close(
            9,
            &wasted,
            0.0,
            MandateExit::Released,
            NaiveDate::from_ymd_opt(2029, 7, 1).unwrap(),
        ));
        assert!(matches!(
            bold.hear(&approved),
            BoardTransferDecision::Vetoed(BoardTransferConcern::FinancialDiscipline)
        ));
    }

    #[test]
    fn split_vote_dossier_with_risk_flags_vetoes() {
        // Two scouts watching, split decision, multiple risk flags →
        // board sends it back to recruitment instead of approving.
        let board = ClubBoard::new();
        let mut proposal = transfer_proposal(
            900_000.0,
            1_000_000.0,
            TransferNeedPriority::Important,
            TransferNeedReason::QualityUpgrade,
        );
        proposal.dossier = Some(BoardDossierSummary {
            scout_votes: 3,
            chief_scout_support: false,
            avg_confidence: 0.55,
            avg_role_fit: 0.95,
            risk_flag_count: 3,
            consensus_score: 0.0,
            data_support: false,
            matches_watched: 1,
        });
        let decision = board.hear(&proposal);
        assert!(
            matches!(decision, BoardTransferDecision::Vetoed(_)),
            "split-vote risk-heavy dossier must veto, got {:?}",
            decision
        );
    }

    #[test]
    fn youth_vision_marks_old_development_signing_as_conditional() {
        let mut board = ClubBoard::new();
        board.vision.youth_focus = VisionYouthFocus::DevelopYouth;

        let mut proposal = transfer_proposal(
            750_000.0,
            1_000_000.0,
            TransferNeedPriority::Optional,
            TransferNeedReason::DevelopmentSigning,
        );
        proposal.player_age = Some(31);

        assert!(matches!(
            board.hear(&proposal),
            BoardTransferDecision::Conditional(BoardTransferConcern::ConflictsWithVision)
        ));
    }
}
