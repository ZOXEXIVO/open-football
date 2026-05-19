#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MediaFanEventKind {
    InterviewCalmsSpeculation,
    InterviewFuelsSpeculation,
    FansSplitOverPlayer,
    SupportersBackPlayerDuringSlump,
    PublicApologyAccepted,
    PublicApologyRejected,
    SocialMediaCriticism,
    MediaNarrativeChanged,
    HomeFansApprove,
    AwayFansHostile,
}

impl MediaFanEventKind {
    pub fn as_i18n_key(&self) -> &'static str {
        match self {
            MediaFanEventKind::InterviewCalmsSpeculation => "media_fan_kind_interview_calms",
            MediaFanEventKind::InterviewFuelsSpeculation => "media_fan_kind_interview_fuels",
            MediaFanEventKind::FansSplitOverPlayer => "media_fan_kind_fans_split",
            MediaFanEventKind::SupportersBackPlayerDuringSlump => "media_fan_kind_supporters_back",
            MediaFanEventKind::PublicApologyAccepted => "media_fan_kind_apology_accepted",
            MediaFanEventKind::PublicApologyRejected => "media_fan_kind_apology_rejected",
            MediaFanEventKind::SocialMediaCriticism => "media_fan_kind_social_media_criticism",
            MediaFanEventKind::MediaNarrativeChanged => "media_fan_kind_narrative_changed",
            MediaFanEventKind::HomeFansApprove => "media_fan_kind_home_fans_approve",
            MediaFanEventKind::AwayFansHostile => "media_fan_kind_away_fans_hostile",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MediaFanSource {
    LocalPress,
    NationalPress,
    SocialMedia,
    HomeSupporters,
    AwaySupporters,
    Pundits,
    PlayerInterview,
}

impl MediaFanSource {
    pub fn as_i18n_key(&self) -> &'static str {
        match self {
            MediaFanSource::LocalPress => "media_fan_source_local_press",
            MediaFanSource::NationalPress => "media_fan_source_national_press",
            MediaFanSource::SocialMedia => "media_fan_source_social_media",
            MediaFanSource::HomeSupporters => "media_fan_source_home_supporters",
            MediaFanSource::AwaySupporters => "media_fan_source_away_supporters",
            MediaFanSource::Pundits => "media_fan_source_pundits",
            MediaFanSource::PlayerInterview => "media_fan_source_player_interview",
        }
    }
}

#[derive(Debug, Clone)]
pub struct MediaFanEventContext {
    pub kind: MediaFanEventKind,
    pub source: MediaFanSource,
    pub trigger_due_to_form: bool,
    pub trigger_due_to_transfer: bool,
    pub trigger_due_to_discipline: bool,
    pub trigger_due_to_big_match: bool,
}

impl MediaFanEventContext {
    pub fn new(kind: MediaFanEventKind, source: MediaFanSource) -> Self {
        Self {
            kind,
            source,
            trigger_due_to_form: false,
            trigger_due_to_transfer: false,
            trigger_due_to_discipline: false,
            trigger_due_to_big_match: false,
        }
    }

    pub fn with_form_trigger(mut self) -> Self {
        self.trigger_due_to_form = true;
        self
    }
    pub fn with_transfer_trigger(mut self) -> Self {
        self.trigger_due_to_transfer = true;
        self
    }
    pub fn with_discipline_trigger(mut self) -> Self {
        self.trigger_due_to_discipline = true;
        self
    }
    pub fn with_big_match_trigger(mut self) -> Self {
        self.trigger_due_to_big_match = true;
        self
    }
}
