/// Football-realistic reason a training session swung positively or
/// negatively. Closed enum so renderer copy stays bounded.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrainingEventReason {
    SharpAfterBeingLeftOut,
    RespondedToCriticism,
    StruggledWithIntensity,
    DistractedByRumours,
    PoorAttitude,
    ReturningFromInjuryNotSharp,
    YoungImpressedStaff,
    SettingStandards,
    ExtraWorkAfterSession,
    MatchPreparationFocus,
    RoutineGoodSession,
    RoutineBadSession,
}

impl TrainingEventReason {
    pub fn as_i18n_key(&self) -> &'static str {
        match self {
            TrainingEventReason::SharpAfterBeingLeftOut => {
                "training_reason_sharp_after_being_left_out"
            }
            TrainingEventReason::RespondedToCriticism => "training_reason_responded_to_criticism",
            TrainingEventReason::StruggledWithIntensity => {
                "training_reason_struggled_with_intensity"
            }
            TrainingEventReason::DistractedByRumours => "training_reason_distracted_by_rumours",
            TrainingEventReason::PoorAttitude => "training_reason_poor_attitude",
            TrainingEventReason::ReturningFromInjuryNotSharp => {
                "training_reason_returning_from_injury_not_sharp"
            }
            TrainingEventReason::YoungImpressedStaff => "training_reason_young_impressed_staff",
            TrainingEventReason::SettingStandards => "training_reason_setting_standards",
            TrainingEventReason::ExtraWorkAfterSession => {
                "training_reason_extra_work_after_session"
            }
            TrainingEventReason::MatchPreparationFocus => "training_reason_match_preparation_focus",
            TrainingEventReason::RoutineGoodSession => "training_reason_routine_good_session",
            TrainingEventReason::RoutineBadSession => "training_reason_routine_bad_session",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrainingEventEvidence {
    HighSessionPerformance,
    LowSessionPerformance,
    HighWorkload,
    LowCondition,
    RecentlyDropped,
    TransferSpeculation,
    InRecoveryPhase,
    HighProfessionalism,
    LowProfessionalism,
    YouthDevelopmentTier,
    VeteranLeader,
    StrongRecentForm,
    UpcomingBigMatch,
    LowEffort,
    FatigueLimited,
    RecoveryLimited,
    LowMorale,
    TransferDistraction,
    CoachMismatch,
    HighWorkRate,
    HighDetermination,
    Overloaded,
    StrongBaselineButOffDay,
    YoungPlayerBreakthrough,
    VeteranSetStandard,
    TacticalMismatch,
}

impl TrainingEventEvidence {
    pub fn as_i18n_key(&self) -> &'static str {
        match self {
            TrainingEventEvidence::HighSessionPerformance => {
                "training_evidence_high_session_performance"
            }
            TrainingEventEvidence::LowSessionPerformance => {
                "training_evidence_low_session_performance"
            }
            TrainingEventEvidence::HighWorkload => "training_evidence_high_workload",
            TrainingEventEvidence::LowCondition => "training_evidence_low_condition",
            TrainingEventEvidence::RecentlyDropped => "training_evidence_recently_dropped",
            TrainingEventEvidence::TransferSpeculation => "training_evidence_transfer_speculation",
            TrainingEventEvidence::InRecoveryPhase => "training_evidence_in_recovery_phase",
            TrainingEventEvidence::HighProfessionalism => "training_evidence_high_professionalism",
            TrainingEventEvidence::LowProfessionalism => "training_evidence_low_professionalism",
            TrainingEventEvidence::YouthDevelopmentTier => {
                "training_evidence_youth_development_tier"
            }
            TrainingEventEvidence::VeteranLeader => "training_evidence_veteran_leader",
            TrainingEventEvidence::StrongRecentForm => "training_evidence_strong_recent_form",
            TrainingEventEvidence::UpcomingBigMatch => "training_evidence_upcoming_big_match",
            TrainingEventEvidence::LowEffort => "training_evidence_low_effort",
            TrainingEventEvidence::FatigueLimited => "training_evidence_fatigue_limited",
            TrainingEventEvidence::RecoveryLimited => "training_evidence_recovery_limited",
            TrainingEventEvidence::LowMorale => "training_evidence_low_morale",
            TrainingEventEvidence::TransferDistraction => "training_evidence_transfer_distraction",
            TrainingEventEvidence::CoachMismatch => "training_evidence_coach_mismatch",
            TrainingEventEvidence::HighWorkRate => "training_evidence_high_work_rate",
            TrainingEventEvidence::HighDetermination => "training_evidence_high_determination",
            TrainingEventEvidence::Overloaded => "training_evidence_overloaded",
            TrainingEventEvidence::StrongBaselineButOffDay => {
                "training_evidence_strong_baseline_but_off_day"
            }
            TrainingEventEvidence::YoungPlayerBreakthrough => {
                "training_evidence_young_player_breakthrough"
            }
            TrainingEventEvidence::VeteranSetStandard => "training_evidence_veteran_set_standard",
            TrainingEventEvidence::TacticalMismatch => "training_evidence_tactical_mismatch",
        }
    }
}

#[derive(Debug, Clone)]
pub struct TrainingEventContext {
    pub reason: TrainingEventReason,
    pub session_performance: f32,
    pub training_performance_ema: f32,
    pub evidence: Vec<TrainingEventEvidence>,
}

impl TrainingEventContext {
    pub fn new(
        reason: TrainingEventReason,
        session_performance: f32,
        training_performance_ema: f32,
    ) -> Self {
        Self {
            reason,
            session_performance,
            training_performance_ema,
            evidence: Vec::new(),
        }
    }

    pub fn with_evidence(mut self, evidence: TrainingEventEvidence) -> Self {
        if !self.evidence.contains(&evidence) {
            self.evidence.push(evidence);
        }
        self
    }

    pub fn with_evidence_iter<I>(mut self, iter: I) -> Self
    where
        I: IntoIterator<Item = TrainingEventEvidence>,
    {
        for ev in iter {
            if !self.evidence.contains(&ev) {
                self.evidence.push(ev);
            }
        }
        self
    }
}
