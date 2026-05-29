//! Career-stage context payload — the late-career arc that bridges an
//! active playing career into retirement (and, for leaders, into a future
//! coaching career). Shared by [`RetirementConsidering`],
//! [`RetirementAnnounced`], and [`CoachingCareerInterest`] so the renderer
//! can explain *why now* — age, reduced role, injuries, long free agency,
//! leadership — instead of guessing from the bare event type.
//!
//! [`RetirementConsidering`]: crate::HappinessEventType::RetirementConsidering
//! [`RetirementAnnounced`]: crate::HappinessEventType::RetirementAnnounced
//! [`CoachingCareerInterest`]: crate::HappinessEventType::CoachingCareerInterest

/// Which career-stage moment this payload describes. The renderer keys off
/// this first, then folds in the reason / evidence atoms.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CareerStageEventKind {
    RetirementConsidering,
    RetirementAnnounced,
    CoachingCareerInterest,
}

impl CareerStageEventKind {
    pub fn as_i18n_key(&self) -> &'static str {
        match self {
            CareerStageEventKind::RetirementConsidering => {
                "career_stage_kind_retirement_considering"
            }
            CareerStageEventKind::RetirementAnnounced => "career_stage_kind_retirement_announced",
            CareerStageEventKind::CoachingCareerInterest => "career_stage_kind_coaching_interest",
        }
    }
}

/// Why a retirement happened. Drives the magnitude sign at the emit site
/// (planned / legend → positive; forced / injury → negative) and the
/// renderer's farewell vs. forced-exit framing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RetirementReason {
    /// Long-term free agent gave up looking for a club.
    LongFreeAgency,
    /// Ordinary age-driven retirement.
    Age,
    /// Injury / recurrence forced an early stop.
    Injury,
    /// Reduced playing role pushed the player out.
    ReducedRole,
    /// Planned, dignified farewell — high reputation / long tenure.
    PlannedFarewell,
    /// Club-legend farewell — the strongest planned send-off.
    ClubLegendFarewell,
}

impl RetirementReason {
    pub fn as_i18n_key(&self) -> &'static str {
        match self {
            RetirementReason::LongFreeAgency => "retirement_reason_long_free_agency",
            RetirementReason::Age => "retirement_reason_age",
            RetirementReason::Injury => "retirement_reason_injury",
            RetirementReason::ReducedRole => "retirement_reason_reduced_role",
            RetirementReason::PlannedFarewell => "retirement_reason_planned_farewell",
            RetirementReason::ClubLegendFarewell => "retirement_reason_club_legend_farewell",
        }
    }

    /// True for send-offs the player chose / earned — used by the emit
    /// site to decide whether the announcement reads positive.
    pub fn is_planned(&self) -> bool {
        matches!(
            self,
            RetirementReason::PlannedFarewell
                | RetirementReason::ClubLegendFarewell
                | RetirementReason::Age
        )
    }
}

/// Concrete signals the career-stage detector latched onto. Closed enum so
/// the renderer copy stays bounded; emit sites push the atoms that justified
/// the moment and the renderer surfaces the most informative one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CareerStageEvidence {
    /// Player is in the late-career age window.
    LateCareer,
    /// Repeated / recurring major injuries.
    RepeatedInjuries,
    /// Playing role has shrunk sharply this season.
    ReducedRole,
    /// Long spell without a club (free agency).
    LongFreeAgency,
    /// Current level is visibly declining.
    DecliningLevel,
    /// Player recently emerged as a dressing-room leader.
    LeadershipEmergence,
    /// Player is / was the club captain.
    Captaincy,
    /// Player has been an influential mentor to younger teammates.
    MentorInfluence,
    /// High professionalism — a coaching-temperament signal.
    HighProfessionalism,
    /// High world reputation — a name that still carries weight.
    HighReputation,
    /// A `RetirementConsidering` event fired recently.
    RecentRetirementConsidering,
}

impl CareerStageEvidence {
    pub fn as_i18n_key(&self) -> &'static str {
        match self {
            CareerStageEvidence::LateCareer => "career_stage_evidence_late_career",
            CareerStageEvidence::RepeatedInjuries => "career_stage_evidence_repeated_injuries",
            CareerStageEvidence::ReducedRole => "career_stage_evidence_reduced_role",
            CareerStageEvidence::LongFreeAgency => "career_stage_evidence_long_free_agency",
            CareerStageEvidence::DecliningLevel => "career_stage_evidence_declining_level",
            CareerStageEvidence::LeadershipEmergence => "career_stage_evidence_leadership",
            CareerStageEvidence::Captaincy => "career_stage_evidence_captaincy",
            CareerStageEvidence::MentorInfluence => "career_stage_evidence_mentor_influence",
            CareerStageEvidence::HighProfessionalism => "career_stage_evidence_professionalism",
            CareerStageEvidence::HighReputation => "career_stage_evidence_high_reputation",
            CareerStageEvidence::RecentRetirementConsidering => {
                "career_stage_evidence_recent_considering"
            }
        }
    }
}

/// Structured payload describing a late-career moment. Filled in at emit
/// time so the renderer can compose a contextual headline + reason rather
/// than guessing from the event-type enum alone.
#[derive(Debug, Clone)]
pub struct CareerStageEventContext {
    pub kind: CareerStageEventKind,
    /// Player age at emit time.
    pub age: u8,
    /// Months without a club (free agents only).
    pub months_without_club: Option<u16>,
    /// Competitive appearances this season, if known.
    pub appearances_this_season: Option<u16>,
    /// Approximate injury days over the last year, if known.
    pub injury_days_last_year: Option<u16>,
    /// World reputation at emit time, if known.
    pub world_reputation: Option<u16>,
    /// Last / current club id, for the renderer to link.
    pub last_club_id: Option<u32>,
    /// Why a retirement happened (announcement only).
    pub retirement_reason: Option<RetirementReason>,
    /// Closed-set evidence atoms that justified the moment.
    pub evidence: Vec<CareerStageEvidence>,
}

impl CareerStageEventContext {
    pub fn new(kind: CareerStageEventKind) -> Self {
        Self {
            kind,
            age: 0,
            months_without_club: None,
            appearances_this_season: None,
            injury_days_last_year: None,
            world_reputation: None,
            last_club_id: None,
            retirement_reason: None,
            evidence: Vec::new(),
        }
    }

    pub fn with_age(mut self, age: u8) -> Self {
        self.age = age;
        self
    }

    pub fn with_months_without_club(mut self, months: u16) -> Self {
        self.months_without_club = Some(months);
        self
    }

    pub fn with_appearances_this_season(mut self, apps: u16) -> Self {
        self.appearances_this_season = Some(apps);
        self
    }

    pub fn with_injury_days_last_year(mut self, days: u16) -> Self {
        self.injury_days_last_year = Some(days);
        self
    }

    pub fn with_world_reputation(mut self, rep: u16) -> Self {
        self.world_reputation = Some(rep);
        self
    }

    pub fn with_last_club(mut self, club_id: u32) -> Self {
        self.last_club_id = Some(club_id);
        self
    }

    pub fn with_retirement_reason(mut self, reason: RetirementReason) -> Self {
        self.retirement_reason = Some(reason);
        self
    }

    pub fn with_evidence(mut self, evidence: CareerStageEvidence) -> Self {
        if !self.evidence.contains(&evidence) {
            self.evidence.push(evidence);
        }
        self
    }
}
