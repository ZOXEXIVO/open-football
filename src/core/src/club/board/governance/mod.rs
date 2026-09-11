//! The transfer hearing: what the board does when the recruitment team
//! puts a name in front of it.
//!
//! Everything the boardroom weighs on an incoming signing lives here — the
//! proposal it is shown, the money and the profile behind it, the verdict
//! it returns, and the tolerance arithmetic that produces that verdict. The
//! manager can ask and the scouts can argue; ownership still decides.

use crate::club::board::ClubBoard;
use crate::club::board::chairman::ChairmanAmbition;
use crate::club::board::ownership::OwnershipType;
use crate::club::board::strategy::{ManagerAutonomy, SquadProfile};
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
    pub player_age: Option<u8>,
    pub player_ability: Option<u8>,
    pub squad_avg_ability: u8,
    pub shortlist_score: f32,
    /// Optional recruitment-meeting dossier built from scout monitoring
    /// state. When present, the board uses it to relax or tighten its
    /// tolerance — strong consensus + chief scout backing earn extra
    /// rope; thin discussion or risk-heavy dossiers get less.
    /// When `None` the board falls back to the legacy decision path
    /// (preserves behaviour for non-pipeline call sites and tests).
    pub dossier: Option<BoardDossierSummary>,
    /// Optional financial/profile dossier on the deal. When present the
    /// board applies ownership-archetype governance (wage impact, resale,
    /// risk, manager priority). `None` keeps the legacy path for tests and
    /// call sites that don't build it yet.
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

impl BoardTransferProposal {
    /// Whether the need behind this proposal is one the board treats as
    /// pressing enough to stretch for — a hole in the shape, an injury to
    /// cover, a squad too thin to field. Earns a tolerance bump at the
    /// hearing.
    pub fn has_urgent_reason(&self) -> bool {
        matches!(
            self.reason,
            TransferNeedReason::FormationGap
                | TransferNeedReason::QualityUpgrade
                | TransferNeedReason::DepthCover
                | TransferNeedReason::LoanToFillSquad
                | TransferNeedReason::SquadPadding
                | TransferNeedReason::InjuryCoverLoan
                | TransferNeedReason::OpportunisticLoanUpgrade
        )
    }
}

impl ClubBoard {
    /// Board/chairman review of a proposed incoming transfer. This is the
    /// football committee layer: the head coach can ask, the recruitment team
    /// can shortlist, but ownership still weighs budget, urgency, squad level,
    /// chairman temperament, and club vision before negotiations start.
    pub fn review_transfer_proposal(
        &self,
        proposal: &BoardTransferProposal,
    ) -> BoardTransferDecision {
        let allocated_budget = proposal.allocated_budget.max(1.0);
        let over_allocated = proposal.fee / allocated_budget;
        let remaining_budget = proposal.remaining_transfer_budget.max(0.0);

        if remaining_budget > 0.0 && proposal.fee > remaining_budget * 1.05 {
            return BoardTransferDecision::Vetoed(BoardTransferConcern::ExceedsTransferBudget);
        }

        let mut tolerance: f64 = match self.vision.financial_stance {
            FinancialStance::Austerity => 0.90,
            FinancialStance::Conservative => 1.25,
            FinancialStance::Balanced => 1.75,
            FinancialStance::Ambitious => 2.35,
        };

        tolerance += match self.chairman.ambition {
            ChairmanAmbition::Reckless => 0.45,
            ChairmanAmbition::Ambitious => 0.20,
            ChairmanAmbition::Balanced => 0.0,
            ChairmanAmbition::Conservative => -0.15,
        };

        // Ownership archetype risk appetite. Neutral owners (risk 50,
        // LocalBusiness) contribute exactly 0 so legacy call sites and
        // tests are unaffected.
        tolerance += (self.ownership.risk_tolerance as f64 - 50.0) / 100.0 * 0.5;
        tolerance += match self.ownership.ownership_type {
            OwnershipType::StateBacked => 0.20,
            OwnershipType::MemberOwned => -0.10,
            OwnershipType::PrivateEquity => -0.05,
            _ => 0.0,
        };

        // Member-owned boards prize local identity: a homegrown target earns
        // extra rope, an import is viewed more coolly. Reads the economics
        // dossier's homegrown flag when one is present.
        if matches!(self.ownership.ownership_type, OwnershipType::MemberOwned) {
            if let Some(e) = proposal.economics {
                tolerance += if e.homegrown_fit { 0.20 } else { -0.10 };
            }
        }

        tolerance += match proposal.priority {
            TransferNeedPriority::Critical => 0.35,
            TransferNeedPriority::Important => 0.15,
            TransferNeedPriority::Optional => 0.0,
        };

        if self.confidence.level >= 75 {
            tolerance += 0.15;
        } else if self.confidence.level < 35 {
            tolerance -= 0.25;
        }

        // Low-autonomy boards under sliding confidence let the director of
        // football intervene and tighten tolerance on the manager's asks.
        if matches!(self.vision.manager_autonomy, ManagerAutonomy::Low)
            && self.confidence.level < self.vision.manager_autonomy.dof_override_threshold()
        {
            tolerance -= 0.20;
        }

        if proposal.has_urgent_reason() {
            tolerance += 0.20;
        }

        if proposal.shortlist_score >= 1.15 {
            tolerance += 0.10;
        } else if proposal.shortlist_score < 0.75 {
            tolerance -= 0.15;
        }

        // Dossier-driven tolerance shift. Strong consensus + chief
        // scout backing + plenty of confidence earn extra board rope;
        // thin or risk-heavy dossiers tighten tolerance. Done before
        // the over-allocation gate so a well-supported target can
        // survive a slightly higher fee, and a poorly-supported one
        // can fall short even if the fee is close to budget.
        if let Some(d) = proposal.dossier {
            if d.consensus_score >= 2.5 && d.chief_scout_support {
                tolerance += 0.20;
            } else if d.consensus_score >= 1.5 {
                tolerance += 0.10;
            } else if d.consensus_score <= 0.5 && d.scout_votes >= 2 {
                tolerance -= 0.15;
            }
            if d.avg_confidence >= 0.8 {
                tolerance += 0.05;
            } else if d.avg_confidence < 0.5 {
                tolerance -= 0.10;
            }
            if d.risk_flag_count >= 3 {
                tolerance -= 0.15;
            }
            if d.data_support {
                tolerance += 0.05;
            }
            if d.avg_role_fit < 0.85 {
                tolerance -= 0.10;
            }
        }

        // Asset term: the same fee is a different proposition depending on
        // what the club still owns at the end of the contract.
        //
        // A board weighing a big fee does not only ask "can we afford it?"
        // — it asks "what is left when we are done?". A 22-year-old is a
        // resaleable asset the club can recover most of its money from; a
        // 30-year-old at the same price is consumption. Without this the
        // model priced both identically, so the deals real boards find
        // easiest to sign off — a young standout at a transformative fee —
        // faced exactly the same discipline gate as a veteran punt.
        //
        // Continuous in age, so there is no cliff at which a player stops
        // being an asset, and centred on `ASSET_NEUTRAL_AGE` so an
        // ordinary prime-age signing is unaffected.
        if let Some(age) = proposal.player_age {
            tolerance += Self::asset_tolerance(age);
        }

        if over_allocated > tolerance.max(0.50) {
            return BoardTransferDecision::Vetoed(BoardTransferConcern::FinancialDiscipline);
        }

        if !self.is_sporting_case_credible(proposal) {
            return BoardTransferDecision::Vetoed(BoardTransferConcern::WeakSportingCase);
        }

        if self.transfer_conflicts_with_vision(proposal) {
            return BoardTransferDecision::Conditional(BoardTransferConcern::ConflictsWithVision);
        }

        // Dossier-driven veto: if the dossier shows a serious red flag
        // (split votes / no role fit / multiple risks) the board sends
        // it back to the recruitment team rather than approving.
        if let Some(d) = proposal.dossier {
            // "Two scouts watching, consensus near zero" = open
            // disagreement. The board doesn't sign on a flip-coin.
            if d.scout_votes >= 2 && d.consensus_score.abs() < 0.4 && d.risk_flag_count >= 2 {
                return BoardTransferDecision::Vetoed(BoardTransferConcern::WeakSportingCase);
            }
        }

        // Ownership-archetype governance: squad-profile fit + deal
        // economics (wage impact, resale, off-pitch risk). No-op for a
        // Balanced profile with no economics dossier.
        if let Some(decision) = self.review_governance(proposal) {
            return decision;
        }

        if over_allocated > 1.0 || remaining_budget <= allocated_budget * 0.25 {
            return BoardTransferDecision::Conditional(BoardTransferConcern::FinancialDiscipline);
        }

        BoardTransferDecision::Approved
    }

    /// Ownership-archetype governance layered on the base review:
    /// squad-profile fit plus deal economics (wage impact, resale, risk).
    /// Returns `Some` to override the base decision; `None` to defer to it.
    /// A `Balanced` profile with no economics dossier always returns `None`.
    fn asset_tolerance(age: u8) -> f64 {
        /// Age at which a signing is neither an asset nor consumption — the
        /// tolerance shift crosses zero here.
        const ASSET_NEUTRAL_AGE: f64 = 26.0;
        /// Tolerance the board grants per year of resale life below
        /// `ASSET_NEUTRAL_AGE`, and takes back per year above it.
        const PER_YEAR: f64 = 0.06;
        /// Bound on the whole term, so age can shade a decision but never
        /// decide it on its own.
        const CAP: f64 = 0.30;
        ((ASSET_NEUTRAL_AGE - age as f64) * PER_YEAR).clamp(-CAP, CAP)
    }

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
                    if let Some(ability) = proposal.player_ability {
                        if ability + 4 < proposal.squad_avg_ability {
                            return Some(BoardTransferDecision::Vetoed(WeakSportingCase));
                        }
                    }
                }
                _ => {}
            }
        }

        // ── Deal economics ──
        let Some(e) = proposal.economics else {
            return None;
        };

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

    fn transfer_proposal(
        fee: f64,
        allocated_budget: f64,
        priority: TransferNeedPriority,
        reason: TransferNeedReason,
    ) -> BoardTransferProposal {
        BoardTransferProposal {
            fee,
            allocated_budget,
            remaining_transfer_budget: 10_000_000.0,
            priority,
            reason,
            player_age: Some(25),
            player_ability: Some(65),
            squad_avg_ability: 60,
            shortlist_score: 1.0,
            dossier: None,
            economics: None,
        }
    }

    #[test]
    fn conservative_board_vetoes_excessive_transfer_overrun() {
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
            board.review_transfer_proposal(&proposal),
            BoardTransferDecision::Vetoed(BoardTransferConcern::FinancialDiscipline)
        ));
    }

    #[test]
    fn ambitious_board_backs_critical_squad_gap_within_cash_limit() {
        let mut board = ClubBoard::new();
        board.vision.financial_stance = FinancialStance::Ambitious;
        board.chairman.ambition = ChairmanAmbition::Ambitious;
        board.confidence.level = 80;

        let proposal = transfer_proposal(
            2_250_000.0,
            1_000_000.0,
            TransferNeedPriority::Critical,
            TransferNeedReason::FormationGap,
        );

        assert!(board.review_transfer_proposal(&proposal).is_approved());
    }

    #[test]
    fn strong_dossier_relaxes_board_tolerance() {
        // A proposal that's borderline on budget normally gets flagged
        // financial-discipline. With a strong dossier (consensus + chief
        // scout backing + high confidence) the board approves anyway.
        let mut board = ClubBoard::new();
        board.vision.financial_stance = FinancialStance::Balanced;
        let mut proposal = transfer_proposal(
            1_700_000.0,
            1_000_000.0,
            TransferNeedPriority::Important,
            TransferNeedReason::QualityUpgrade,
        );
        // Without dossier — borderline.
        let baseline = board.review_transfer_proposal(&proposal);
        // With strong dossier — should approve.
        proposal.dossier = Some(BoardDossierSummary {
            scout_votes: 3,
            chief_scout_support: true,
            avg_confidence: 0.85,
            avg_role_fit: 1.10,
            risk_flag_count: 0,
            consensus_score: 3.0,
            data_support: true,
            matches_watched: 4,
        });
        let with_dossier = board.review_transfer_proposal(&proposal);
        // Dossier-backed should be at least as approved as the baseline.
        // Specifically: a strong dossier should never downgrade an
        // Approved into a Vetoed.
        if matches!(baseline, BoardTransferDecision::Vetoed(_)) {
            assert!(
                with_dossier.is_approved(),
                "strong dossier should rescue a borderline veto, got {:?}",
                with_dossier
            );
        } else {
            assert!(with_dossier.is_approved());
        }
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
        let decision = board.review_transfer_proposal(&proposal);
        assert!(
            matches!(decision, BoardTransferDecision::Vetoed(_)),
            "split-vote risk-heavy dossier must veto, got {:?}",
            decision
        );
    }

    #[test]
    fn dossier_is_optional_legacy_path_unchanged() {
        // Ensure the no-dossier path produces exactly the same result
        // as the pre-recruitment-meeting baseline. The whole point of
        // the optional field is backwards compatibility.
        let mut board = ClubBoard::new();
        board.vision.financial_stance = FinancialStance::Conservative;
        let proposal = transfer_proposal(
            2_000_000.0,
            1_000_000.0,
            TransferNeedPriority::Important,
            TransferNeedReason::QualityUpgrade,
        );
        let decision = board.review_transfer_proposal(&proposal);
        assert!(matches!(
            decision,
            BoardTransferDecision::Vetoed(BoardTransferConcern::FinancialDiscipline)
        ));
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
            board.review_transfer_proposal(&proposal),
            BoardTransferDecision::Conditional(BoardTransferConcern::ConflictsWithVision)
        ));
    }

    #[test]
    fn a_young_signing_earns_rope_and_an_old_one_loses_it() {
        // The same fee, three ages. A board weighing a big number asks what
        // is left at the end of the contract, and the model had no way to
        // express that at all.
        let young = ClubBoard::asset_tolerance(21);
        let neutral = ClubBoard::asset_tolerance(26);
        let veteran = ClubBoard::asset_tolerance(31);
        assert!(young > 0.0, "{young}");
        assert_eq!(neutral, 0.0);
        assert!(veteran < 0.0, "{veteran}");
        assert!(young > veteran);
    }

    #[test]
    fn the_term_stays_bounded_at_both_ends() {
        // Age shades a decision; it must never decide one.
        assert!(ClubBoard::asset_tolerance(16) <= 0.30);
        assert!(ClubBoard::asset_tolerance(40) >= -0.30);
    }
}
