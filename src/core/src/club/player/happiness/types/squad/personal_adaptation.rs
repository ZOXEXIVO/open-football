#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PersonalAdaptationKind {
    HomesicknessConcern,
    FamilySettled,
    FamilyUnsettled,
    LifestyleAdaptation,
    LanguageBarrierConcern,
    LocalCultureSettling,
    CompanionSupport,
    AskedForPersonalLeave,
    LanguageMilestone,
    SettlingIntoSquad,
    StillStrugglingToSettle,
    /// Long-running adaptation failure — player wants to head back home
    /// or to a familiar league/club after an extended struggle.
    ReturnHomeAfterPoorAdaptation,
    /// Career-stage ambition: at a club that can't offer European
    /// competition. Used by the desire pipeline (`WantsEuropeanCompetition`).
    EuropeanCompetitionAmbition,
    /// Career-stage ambition: South American heritage player wants
    /// Copa Libertadores football.
    CopaLibertadoresAmbition,
}

impl PersonalAdaptationKind {
    pub fn as_i18n_key(&self) -> &'static str {
        match self {
            PersonalAdaptationKind::HomesicknessConcern => "adaptation_kind_homesickness",
            PersonalAdaptationKind::FamilySettled => "adaptation_kind_family_settled",
            PersonalAdaptationKind::FamilyUnsettled => "adaptation_kind_family_unsettled",
            PersonalAdaptationKind::LifestyleAdaptation => "adaptation_kind_lifestyle",
            PersonalAdaptationKind::LanguageBarrierConcern => "adaptation_kind_language_barrier",
            PersonalAdaptationKind::LocalCultureSettling => "adaptation_kind_local_culture",
            PersonalAdaptationKind::CompanionSupport => "adaptation_kind_companion_support",
            PersonalAdaptationKind::AskedForPersonalLeave => "adaptation_kind_personal_leave",
            PersonalAdaptationKind::LanguageMilestone => "adaptation_kind_language_milestone",
            PersonalAdaptationKind::SettlingIntoSquad => "adaptation_kind_settling_squad",
            PersonalAdaptationKind::StillStrugglingToSettle => "adaptation_kind_still_struggling",
            PersonalAdaptationKind::ReturnHomeAfterPoorAdaptation => {
                "adaptation_kind_return_home_after_poor_adaptation"
            }
            PersonalAdaptationKind::EuropeanCompetitionAmbition => {
                "adaptation_kind_european_competition_ambition"
            }
            PersonalAdaptationKind::CopaLibertadoresAmbition => {
                "adaptation_kind_copa_libertadores_ambition"
            }
        }
    }
}

#[derive(Debug, Clone)]
pub struct PersonalAdaptationEventContext {
    pub kind: PersonalAdaptationKind,
    pub days_at_club: u32,
    pub adaptability: Option<f32>,
    pub has_compatriot_in_squad: bool,
    pub speaks_local_language: bool,
}

impl PersonalAdaptationEventContext {
    pub fn new(kind: PersonalAdaptationKind, days_at_club: u32) -> Self {
        Self {
            kind,
            days_at_club,
            adaptability: None,
            has_compatriot_in_squad: false,
            speaks_local_language: false,
        }
    }

    pub fn with_adaptability(mut self, attr: f32) -> Self {
        self.adaptability = Some(attr);
        self
    }
    pub fn with_compatriot(mut self, has: bool) -> Self {
        self.has_compatriot_in_squad = has;
        self
    }
    pub fn with_local_language(mut self, speaks: bool) -> Self {
        self.speaks_local_language = speaks;
        self
    }
}
