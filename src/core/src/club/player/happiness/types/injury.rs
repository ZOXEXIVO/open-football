#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InjuryRecoveryStage {
    ReturnedToFullTraining,
    FirstMinutesAfterInjury,
    RecoverySetback,
    ProtectedByMedicalStaff,
    InjuryRecurrenceConcern,
    FitnessConfidenceRestored,
}

impl InjuryRecoveryStage {
    pub fn as_i18n_key(&self) -> &'static str {
        match self {
            InjuryRecoveryStage::ReturnedToFullTraining => "injury_stage_returned_full_training",
            InjuryRecoveryStage::FirstMinutesAfterInjury => "injury_stage_first_minutes",
            InjuryRecoveryStage::RecoverySetback => "injury_stage_recovery_setback",
            InjuryRecoveryStage::ProtectedByMedicalStaff => "injury_stage_protected",
            InjuryRecoveryStage::InjuryRecurrenceConcern => "injury_stage_recurrence_concern",
            InjuryRecoveryStage::FitnessConfidenceRestored => "injury_stage_confidence_restored",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InjuryRecoveryEvidence {
    LongTermLayoff,
    ShortTermLayoff,
    MatchSharpnessLow,
    MatchSharpnessRecovering,
    MultipleInjuriesThisSeason,
    PriorRecurringIssue,
    HighProfessionalism,
    FearLosingPlace,
}

impl InjuryRecoveryEvidence {
    pub fn as_i18n_key(&self) -> &'static str {
        match self {
            InjuryRecoveryEvidence::LongTermLayoff => "injury_evidence_long_term_layoff",
            InjuryRecoveryEvidence::ShortTermLayoff => "injury_evidence_short_term_layoff",
            InjuryRecoveryEvidence::MatchSharpnessLow => "injury_evidence_sharpness_low",
            InjuryRecoveryEvidence::MatchSharpnessRecovering => {
                "injury_evidence_sharpness_recovering"
            }
            InjuryRecoveryEvidence::MultipleInjuriesThisSeason => {
                "injury_evidence_multiple_injuries_season"
            }
            InjuryRecoveryEvidence::PriorRecurringIssue => "injury_evidence_prior_recurring_issue",
            InjuryRecoveryEvidence::HighProfessionalism => "injury_evidence_high_professionalism",
            InjuryRecoveryEvidence::FearLosingPlace => "injury_evidence_fear_losing_place",
        }
    }
}

#[derive(Debug, Clone)]
pub struct InjuryRecoveryEventContext {
    pub stage: InjuryRecoveryStage,
    pub recovery_days_total: u16,
    pub match_readiness: f32,
    pub evidence: Vec<InjuryRecoveryEvidence>,
}

impl InjuryRecoveryEventContext {
    pub fn new(stage: InjuryRecoveryStage, recovery_days_total: u16, match_readiness: f32) -> Self {
        Self {
            stage,
            recovery_days_total,
            match_readiness,
            evidence: Vec::new(),
        }
    }

    pub fn with_evidence(mut self, evidence: InjuryRecoveryEvidence) -> Self {
        if !self.evidence.contains(&evidence) {
            self.evidence.push(evidence);
        }
        self
    }
}
