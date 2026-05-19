#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NationalTeamEventKind {
    FirstCallup,
    Recall,
    EmergencyCallup,
    YouthToSeniorJump,
    DroppedDueToForm,
    DroppedDueToInjury,
    DroppedDueToCompetition,
    TournamentSquadOmitted,
    InternationalPlaceUnderThreat,
    FirstCapPride,
    NationalTeamRoleGrowing,
}

impl NationalTeamEventKind {
    pub fn as_i18n_key(&self) -> &'static str {
        match self {
            NationalTeamEventKind::FirstCallup => "national_kind_first_callup",
            NationalTeamEventKind::Recall => "national_kind_recall",
            NationalTeamEventKind::EmergencyCallup => "national_kind_emergency_callup",
            NationalTeamEventKind::YouthToSeniorJump => "national_kind_youth_to_senior",
            NationalTeamEventKind::DroppedDueToForm => "national_kind_dropped_form",
            NationalTeamEventKind::DroppedDueToInjury => "national_kind_dropped_injury",
            NationalTeamEventKind::DroppedDueToCompetition => "national_kind_dropped_competition",
            NationalTeamEventKind::TournamentSquadOmitted => {
                "national_kind_tournament_squad_omitted"
            }
            NationalTeamEventKind::InternationalPlaceUnderThreat => {
                "national_kind_place_under_threat"
            }
            NationalTeamEventKind::FirstCapPride => "national_kind_first_cap_pride",
            NationalTeamEventKind::NationalTeamRoleGrowing => "national_kind_role_growing",
        }
    }
}

#[derive(Debug, Clone)]
pub struct NationalTeamEventContext {
    pub kind: NationalTeamEventKind,
    pub country_id: Option<u32>,
    pub previous_caps: u16,
    pub recent_club_form: Option<f32>,
    pub competition_window: bool,
}

impl NationalTeamEventContext {
    pub fn new(kind: NationalTeamEventKind) -> Self {
        Self {
            kind,
            country_id: None,
            previous_caps: 0,
            recent_club_form: None,
            competition_window: false,
        }
    }

    pub fn with_country(mut self, country_id: u32) -> Self {
        self.country_id = Some(country_id);
        self
    }
    pub fn with_previous_caps(mut self, caps: u16) -> Self {
        self.previous_caps = caps;
        self
    }
    pub fn with_recent_club_form(mut self, form: f32) -> Self {
        self.recent_club_form = Some(form);
        self
    }
    pub fn with_competition_window(mut self, in_window: bool) -> Self {
        self.competition_window = in_window;
        self
    }
}
