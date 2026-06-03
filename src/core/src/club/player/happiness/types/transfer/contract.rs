use crate::club::player::contract::PlayerSquadStatus;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContractEventKind {
    OfferReceived,
    TalksOpened,
    TalksStalled,
    Renewed,
    Terminated,
    SalaryShock,
    SalaryBoost,
    LoyaltyDiscountAccepted,
    AgentPushingForBetterTerms,
    WagePromiseFrustration,
    AcceptedReducedRoleContract,
    RejectedLowStatusOffer,
    /// Player / agent explicitly demanded a release clause in the next
    /// contract. Distinct from the softer `AgentPushingForBetterTerms`
    /// so the renderer can frame the exit-path demand specifically.
    ReleaseClauseDemanded,
    /// Player formally rejected the club's offered contract. Distinct
    /// from `RejectedLowStatusOffer` (which keys on a specific status
    /// downgrade) — this is the generic rejected-the-deal verdict,
    /// regardless of reason. The reason rides on the evidence list.
    OfferRejectedByPlayer,
}

impl ContractEventKind {
    pub fn as_i18n_key(&self) -> &'static str {
        match self {
            ContractEventKind::OfferReceived => "contract_kind_offer_received",
            ContractEventKind::TalksOpened => "contract_kind_talks_opened",
            ContractEventKind::TalksStalled => "contract_kind_talks_stalled",
            ContractEventKind::Renewed => "contract_kind_renewed",
            ContractEventKind::Terminated => "contract_kind_terminated",
            ContractEventKind::SalaryShock => "contract_kind_salary_shock",
            ContractEventKind::SalaryBoost => "contract_kind_salary_boost",
            ContractEventKind::LoyaltyDiscountAccepted => "contract_kind_loyalty_discount_accepted",
            ContractEventKind::AgentPushingForBetterTerms => "contract_kind_agent_pushing",
            ContractEventKind::WagePromiseFrustration => "contract_kind_wage_promise_frustration",
            ContractEventKind::AcceptedReducedRoleContract => "contract_kind_accepted_reduced_role",
            ContractEventKind::RejectedLowStatusOffer => "contract_kind_rejected_low_status",
            ContractEventKind::ReleaseClauseDemanded => "contract_kind_release_clause_demanded",
            ContractEventKind::OfferRejectedByPlayer => "contract_kind_offer_rejected_by_player",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContractEventEvidence {
    AgentPressure,
    HighLoyalty,
    LowLoyalty,
    HighAmbition,
    UnderpaidVsPeers,
    OverpaidVsExpectation,
    SquadStatusUpgrade,
    SquadStatusDowngrade,
    UsedExternalInterestAsLeverage,
    ContractExpiring,
    HasOtherInterest,
    ClubInFinancialDistress,
    /// Player asked for a release clause in the next deal.
    ReleaseClauseDemanded,
    /// Dispute over the promised squad role / status in the renewal.
    RoleExpectationGap,
    /// Dispute over the length of the offered contract.
    ContractLengthDispute,
    /// Player rejected the deal specifically over wage (insufficient
    /// vs market / peers).
    RejectedOverWage,
    /// Player rejected the deal specifically over offered role / status.
    RejectedOverRole,
    /// Player rejected the deal specifically over the missing release
    /// clause (the player asked for one but the club refused).
    RejectedOverReleaseClause,
    /// Player rejected the deal over its length (too short for security,
    /// or too long when wanting to keep options open).
    RejectedOverLength,
    /// Player rejected the deal over ambition / project mismatch — the
    /// club's direction doesn't match what he wants from his career.
    RejectedOverAmbition,
}

impl ContractEventEvidence {
    pub fn as_i18n_key(&self) -> &'static str {
        match self {
            ContractEventEvidence::AgentPressure => "contract_evidence_agent_pressure",
            ContractEventEvidence::HighLoyalty => "contract_evidence_high_loyalty",
            ContractEventEvidence::LowLoyalty => "contract_evidence_low_loyalty",
            ContractEventEvidence::HighAmbition => "contract_evidence_high_ambition",
            ContractEventEvidence::UnderpaidVsPeers => "contract_evidence_underpaid_vs_peers",
            ContractEventEvidence::OverpaidVsExpectation => {
                "contract_evidence_overpaid_vs_expectation"
            }
            ContractEventEvidence::SquadStatusUpgrade => "contract_evidence_squad_status_upgrade",
            ContractEventEvidence::SquadStatusDowngrade => {
                "contract_evidence_squad_status_downgrade"
            }
            ContractEventEvidence::UsedExternalInterestAsLeverage => {
                "contract_evidence_used_external_interest"
            }
            ContractEventEvidence::ContractExpiring => "contract_evidence_contract_expiring",
            ContractEventEvidence::HasOtherInterest => "contract_evidence_has_other_interest",
            ContractEventEvidence::ClubInFinancialDistress => {
                "contract_evidence_club_financial_distress"
            }
            ContractEventEvidence::ReleaseClauseDemanded => {
                "contract_evidence_release_clause_demanded"
            }
            ContractEventEvidence::RoleExpectationGap => "contract_evidence_role_expectation_gap",
            ContractEventEvidence::ContractLengthDispute => {
                "contract_evidence_contract_length_dispute"
            }
            ContractEventEvidence::RejectedOverWage => "contract_evidence_rejected_over_wage",
            ContractEventEvidence::RejectedOverRole => "contract_evidence_rejected_over_role",
            ContractEventEvidence::RejectedOverReleaseClause => {
                "contract_evidence_rejected_over_release_clause"
            }
            ContractEventEvidence::RejectedOverLength => "contract_evidence_rejected_over_length",
            ContractEventEvidence::RejectedOverAmbition => {
                "contract_evidence_rejected_over_ambition"
            }
        }
    }
}

#[derive(Debug, Clone)]
pub struct ContractEventContext {
    pub kind: ContractEventKind,
    pub interested_club_id: Option<u32>,
    pub wage_ratio_vs_previous: Option<f32>,
    pub wage_ratio_vs_peers: Option<f32>,
    pub promised_status: Option<PlayerSquadStatus>,
    pub agent_pressure: Option<f32>,
    pub years_remaining: Option<u8>,
    /// Release-clause value the player / agent demanded, when known.
    pub demanded_release_clause: Option<u64>,
    pub evidence: Vec<ContractEventEvidence>,
}

impl ContractEventContext {
    pub fn new(kind: ContractEventKind) -> Self {
        Self {
            kind,
            interested_club_id: None,
            wage_ratio_vs_previous: None,
            wage_ratio_vs_peers: None,
            promised_status: None,
            agent_pressure: None,
            years_remaining: None,
            demanded_release_clause: None,
            evidence: Vec::new(),
        }
    }

    pub fn with_demanded_release_clause(mut self, value: u64) -> Self {
        self.demanded_release_clause = Some(value);
        self
    }

    pub fn with_wage_vs_previous(mut self, ratio: f32) -> Self {
        self.wage_ratio_vs_previous = Some(ratio);
        self
    }
    pub fn with_wage_vs_peers(mut self, ratio: f32) -> Self {
        self.wage_ratio_vs_peers = Some(ratio);
        self
    }
    pub fn with_promised_status(mut self, status: PlayerSquadStatus) -> Self {
        self.promised_status = Some(status);
        self
    }
    pub fn with_agent_pressure(mut self, pressure: f32) -> Self {
        self.agent_pressure = Some(pressure);
        self
    }
    pub fn with_years_remaining(mut self, years: u8) -> Self {
        self.years_remaining = Some(years);
        self
    }
    pub fn with_interested_club(mut self, club_id: u32) -> Self {
        self.interested_club_id = Some(club_id);
        self
    }

    pub fn with_evidence(mut self, evidence: ContractEventEvidence) -> Self {
        if !self.evidence.contains(&evidence) {
            self.evidence.push(evidence);
        }
        self
    }
}
