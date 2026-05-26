/// What flavour of career-desire mood the player is signalling.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CareerDesireKind {
    ReturnHomeAfterPoorAdaptation,
    EuropeanCompetitionAmbition,
    CopaLibertadoresAmbition,
}

impl CareerDesireKind {
    pub fn as_i18n_key(&self) -> &'static str {
        match self {
            CareerDesireKind::ReturnHomeAfterPoorAdaptation => "career_desire_kind_return_home",
            CareerDesireKind::EuropeanCompetitionAmbition => {
                "career_desire_kind_european_competition"
            }
            CareerDesireKind::CopaLibertadoresAmbition => "career_desire_kind_copa_libertadores",
        }
    }
}

/// Concrete signals the desire detector latched onto. Closed enum so the
/// renderer copy stays bounded; emit sites push the atoms that justified
/// the mood and the renderer surfaces the most informative one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CareerDesireEvidence {
    /// Player is at a club whose country sits on a different continent
    /// from the player's nationality.
    DifferentContinent,
    /// Player does not speak the local language of his current club
    /// country.
    NoLocalLanguage,
    /// Player's `adaptability` personality is low (≤ 8).
    LowAdaptability,
    /// No same-nationality / shared-language teammates in the squad.
    NoCompatriotSupport,
    /// Recent adaptation_score was poor (sub-40 band).
    PoorAdaptationScore,
    /// Personality `ambition` is high (≥ 14).
    HighAmbition,
    /// Current club is not in or near a continental qualification path.
    CurrentClubNotContinental,
    /// A favourite / former / home-country destination is concretely
    /// linked.
    HomeOrFavouriteLink,
    /// Repeated `FeelingIsolated` events over the recent window.
    RepeatedIsolation,
    /// `club_fit` morale axis is meaningfully negative.
    LowClubFit,
}

impl CareerDesireEvidence {
    pub fn as_i18n_key(&self) -> &'static str {
        match self {
            CareerDesireEvidence::DifferentContinent => {
                "career_desire_evidence_different_continent"
            }
            CareerDesireEvidence::NoLocalLanguage => "career_desire_evidence_no_local_language",
            CareerDesireEvidence::LowAdaptability => "career_desire_evidence_low_adaptability",
            CareerDesireEvidence::NoCompatriotSupport => {
                "career_desire_evidence_no_compatriot_support"
            }
            CareerDesireEvidence::PoorAdaptationScore => {
                "career_desire_evidence_poor_adaptation_score"
            }
            CareerDesireEvidence::HighAmbition => "career_desire_evidence_high_ambition",
            CareerDesireEvidence::CurrentClubNotContinental => {
                "career_desire_evidence_current_club_not_continental"
            }
            CareerDesireEvidence::HomeOrFavouriteLink => {
                "career_desire_evidence_home_or_favourite_link"
            }
            CareerDesireEvidence::RepeatedIsolation => "career_desire_evidence_repeated_isolation",
            CareerDesireEvidence::LowClubFit => "career_desire_evidence_low_club_fit",
        }
    }
}

/// Structured payload describing why the player is signalling a
/// career-desire mood (return home / European / Libertadores). Filled
/// in at emit time so the renderer can compose a contextual headline +
/// reason instead of guessing from the event-type enum alone.
#[derive(Debug, Clone)]
pub struct CareerDesireEventContext {
    pub kind: CareerDesireKind,
    /// Days at current club at emit time. 0 if unknown.
    pub days_at_club: u32,
    /// Adaptation score 0..100 if available.
    pub adaptation_score: Option<f32>,
    /// Closed-set evidence atoms that justified the mood.
    pub evidence: Vec<CareerDesireEvidence>,
}

impl CareerDesireEventContext {
    pub fn new(kind: CareerDesireKind) -> Self {
        Self {
            kind,
            days_at_club: 0,
            adaptation_score: None,
            evidence: Vec::new(),
        }
    }

    pub fn with_days_at_club(mut self, days: u32) -> Self {
        self.days_at_club = days;
        self
    }

    pub fn with_adaptation_score(mut self, score: f32) -> Self {
        self.adaptation_score = Some(score);
        self
    }

    pub fn with_evidence(mut self, evidence: CareerDesireEvidence) -> Self {
        if !self.evidence.contains(&evidence) {
            self.evidence.push(evidence);
        }
        self
    }
}
