use super::CareerDesireEvidence;

/// Specific flavour of life-simulation request / mood. Kept closed so
/// renderers can localise each category and tests can assert which
/// bucket a particular detector emits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LifeSimulationDesireKind {
    /// Family hasn't settled at the new country (schools, social
    /// network, isolation). Player asks for support / time off / move.
    FamilyUnsettledAbroad,
    /// Schooling-age children: lack of suitable school in the language
    /// the family uses raises home-country preference.
    PartnerSchoolingConcern,
    /// Bereavement leave — recent close-family loss.
    BereavementLeave,
    /// Family-birth leave — pregnancy / new arrival in the household.
    FamilyBirthLeave,
    /// Divorce / separation impact on focus and morale.
    DivorceImpact,
    /// Player asks for a language tutor / cultural integration support.
    WantsLanguageTutor,
    /// Player asks for a mentor or seeks compatriot support in the
    /// dressing room.
    WantsMentorSupport,
    /// Player asks for a tactical role / position change rather than
    /// mere minutes — minutes alone don't fix a misused asset.
    WantsPreferredTacticalRole,
    /// Player wants out of a media / fan-abuse cauldron and is open to
    /// a less-prestigious lower-pressure side.
    WantsLowerPressureClub,
    /// Wants a release clause baked into the next contract extension.
    WantsReleaseClause,
    /// Wants a verbal promise that the club will entertain offers from
    /// a specific competition / club tier.
    WantsPromiseToSell,
    /// Prefers loan to a permanent move (development, family ties,
    /// short-term escape).
    WantsLoanNotPermanent,
    /// Wants more national-team visibility ahead of a major tournament
    /// (World Cup / Copa America / Euros).
    WantsNationalTeamVisibility,
    /// Wants a league with a national-team selection bias toward its
    /// own players (e.g. NT selectors who lean on Premier League /
    /// Bundesliga form).
    WantsLeagueWithNtBias,
    /// Prefers a culturally / linguistically familiar country even if
    /// it's not the player's home country (Argentine to Spain,
    /// Portuguese to Brazil, …).
    PrefersCulturalFamiliarity,
    /// Veteran asks for a final homecoming season at his boyhood / home
    /// club before retirement.
    VeteranHomecomingSeason,
    /// Long-tenured club legend resists a move out unless the club has
    /// disrespected him.
    ClubLegendRefusesLeave,
    /// Player turns down a rival-club approach despite a clear career
    /// upgrade — loyalty trumps progression.
    RefusesRivalMoveDespiteUpgrade,
}

impl LifeSimulationDesireKind {
    pub fn as_i18n_key(&self) -> &'static str {
        match self {
            LifeSimulationDesireKind::FamilyUnsettledAbroad => "life_sim_kind_family_unsettled",
            LifeSimulationDesireKind::PartnerSchoolingConcern => "life_sim_kind_schooling",
            LifeSimulationDesireKind::BereavementLeave => "life_sim_kind_bereavement",
            LifeSimulationDesireKind::FamilyBirthLeave => "life_sim_kind_family_birth",
            LifeSimulationDesireKind::DivorceImpact => "life_sim_kind_divorce",
            LifeSimulationDesireKind::WantsLanguageTutor => "life_sim_kind_language_tutor",
            LifeSimulationDesireKind::WantsMentorSupport => "life_sim_kind_mentor",
            LifeSimulationDesireKind::WantsPreferredTacticalRole => "life_sim_kind_tactical_role",
            LifeSimulationDesireKind::WantsLowerPressureClub => "life_sim_kind_lower_pressure",
            LifeSimulationDesireKind::WantsReleaseClause => "life_sim_kind_release_clause",
            LifeSimulationDesireKind::WantsPromiseToSell => "life_sim_kind_promise_to_sell",
            LifeSimulationDesireKind::WantsLoanNotPermanent => "life_sim_kind_loan_preferred",
            LifeSimulationDesireKind::WantsNationalTeamVisibility => "life_sim_kind_nt_visibility",
            LifeSimulationDesireKind::WantsLeagueWithNtBias => "life_sim_kind_nt_bias_league",
            LifeSimulationDesireKind::PrefersCulturalFamiliarity => "life_sim_kind_cultural_fit",
            LifeSimulationDesireKind::VeteranHomecomingSeason => "life_sim_kind_veteran_homecoming",
            LifeSimulationDesireKind::ClubLegendRefusesLeave => "life_sim_kind_legend_refuses",
            LifeSimulationDesireKind::RefusesRivalMoveDespiteUpgrade => {
                "life_sim_kind_rival_refuse"
            }
        }
    }
}

/// Severity tier specific to life-simulation moods. Renderer can
/// translate to Minor/Moderate/Strong/Acute copy independent of the
/// generic HappinessEventSeverity tier.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LifeSimulationSeverity {
    Mild,
    Moderate,
    Strong,
    Acute,
}

impl LifeSimulationSeverity {
    pub fn as_i18n_key(&self) -> &'static str {
        match self {
            LifeSimulationSeverity::Mild => "life_sim_severity_mild",
            LifeSimulationSeverity::Moderate => "life_sim_severity_moderate",
            LifeSimulationSeverity::Strong => "life_sim_severity_strong",
            LifeSimulationSeverity::Acute => "life_sim_severity_acute",
        }
    }
}

/// What concretely triggered the desire/mood. Closed enum so emit
/// sites pick the football-realistic cause. Renderer uses this for the
/// "why now" framing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LifeSimulationTrigger {
    FamilyAbroadStress,
    SchoolingProblem,
    PersonalLossInFamily,
    NewbornInFamily,
    SeparationOrDivorce,
    LanguageBarrier,
    LackOfMentor,
    TacticalMisuse,
    MediaAbuseOrFanCriticism,
    ContractRenewalDiscussion,
    ApproachFromSpecificClub,
    NationalTeamSquadDeadline,
    ApproachFromRivalClub,
    LongTenureMilestone,
    LateCareerWindow,
    CulturalFitRecognition,
}

impl LifeSimulationTrigger {
    pub fn as_i18n_key(&self) -> &'static str {
        match self {
            LifeSimulationTrigger::FamilyAbroadStress => "life_sim_trigger_family_abroad",
            LifeSimulationTrigger::SchoolingProblem => "life_sim_trigger_schooling",
            LifeSimulationTrigger::PersonalLossInFamily => "life_sim_trigger_loss",
            LifeSimulationTrigger::NewbornInFamily => "life_sim_trigger_newborn",
            LifeSimulationTrigger::SeparationOrDivorce => "life_sim_trigger_separation",
            LifeSimulationTrigger::LanguageBarrier => "life_sim_trigger_language",
            LifeSimulationTrigger::LackOfMentor => "life_sim_trigger_no_mentor",
            LifeSimulationTrigger::TacticalMisuse => "life_sim_trigger_tactical_misuse",
            LifeSimulationTrigger::MediaAbuseOrFanCriticism => "life_sim_trigger_media_abuse",
            LifeSimulationTrigger::ContractRenewalDiscussion => "life_sim_trigger_contract",
            LifeSimulationTrigger::ApproachFromSpecificClub => "life_sim_trigger_specific_approach",
            LifeSimulationTrigger::NationalTeamSquadDeadline => "life_sim_trigger_nt_deadline",
            LifeSimulationTrigger::ApproachFromRivalClub => "life_sim_trigger_rival_approach",
            LifeSimulationTrigger::LongTenureMilestone => "life_sim_trigger_long_tenure",
            LifeSimulationTrigger::LateCareerWindow => "life_sim_trigger_late_career",
            LifeSimulationTrigger::CulturalFitRecognition => "life_sim_trigger_cultural_fit",
        }
    }
}

/// Structured payload for any [`LifeSimulationDesireKind`] event. The
/// renderer reads `kind` first, then severity / trigger / evidence to
/// fill in the headline and reason copy.
#[derive(Debug, Clone)]
pub struct LifeSimulationDesireContext {
    pub kind: LifeSimulationDesireKind,
    pub severity: LifeSimulationSeverity,
    pub trigger: Option<LifeSimulationTrigger>,
    /// Reuses the closed `CareerDesireEvidence` set — adaptation,
    /// language, ambition signals are the same atoms.
    pub evidence: Vec<CareerDesireEvidence>,
}

impl LifeSimulationDesireContext {
    pub fn new(kind: LifeSimulationDesireKind, severity: LifeSimulationSeverity) -> Self {
        Self {
            kind,
            severity,
            trigger: None,
            evidence: Vec::new(),
        }
    }

    pub fn with_trigger(mut self, trigger: LifeSimulationTrigger) -> Self {
        self.trigger = Some(trigger);
        self
    }

    pub fn with_evidence(mut self, evidence: CareerDesireEvidence) -> Self {
        if !self.evidence.contains(&evidence) {
            self.evidence.push(evidence);
        }
        self
    }
}
