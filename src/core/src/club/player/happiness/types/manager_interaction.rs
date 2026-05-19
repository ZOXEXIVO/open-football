#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ManagerInteractionTopic {
    PlayingTime,
    Performance,
    Tactical,
    Discipline,
    Attitude,
    PromiseFollowUp,
    RoleClarification,
    Other,
}

impl ManagerInteractionTopic {
    pub fn as_i18n_key(&self) -> &'static str {
        match self {
            ManagerInteractionTopic::PlayingTime => "manager_topic_playing_time",
            ManagerInteractionTopic::Performance => "manager_topic_performance",
            ManagerInteractionTopic::Tactical => "manager_topic_tactical",
            ManagerInteractionTopic::Discipline => "manager_topic_discipline",
            ManagerInteractionTopic::Attitude => "manager_topic_attitude",
            ManagerInteractionTopic::PromiseFollowUp => "manager_topic_promise_follow_up",
            ManagerInteractionTopic::RoleClarification => "manager_topic_role_clarification",
            ManagerInteractionTopic::Other => "manager_topic_other",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ManagerInteractionTone {
    Calm,
    Honest,
    Demanding,
    Supportive,
    Stern,
    Praising,
}

impl ManagerInteractionTone {
    pub fn as_i18n_key(&self) -> &'static str {
        match self {
            ManagerInteractionTone::Calm => "manager_tone_calm",
            ManagerInteractionTone::Honest => "manager_tone_honest",
            ManagerInteractionTone::Demanding => "manager_tone_demanding",
            ManagerInteractionTone::Supportive => "manager_tone_supportive",
            ManagerInteractionTone::Stern => "manager_tone_stern",
            ManagerInteractionTone::Praising => "manager_tone_praising",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlayerAcceptance {
    Accepted,
    Resented,
    Ambivalent,
    Motivated,
    Discouraged,
}

impl PlayerAcceptance {
    pub fn as_i18n_key(&self) -> &'static str {
        match self {
            PlayerAcceptance::Accepted => "manager_acceptance_accepted",
            PlayerAcceptance::Resented => "manager_acceptance_resented",
            PlayerAcceptance::Ambivalent => "manager_acceptance_ambivalent",
            PlayerAcceptance::Motivated => "manager_acceptance_motivated",
            PlayerAcceptance::Discouraged => "manager_acceptance_discouraged",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PromiseKind {
    PlayingTime,
    SquadStatus,
    NewSigning,
    ContractRenewal,
    TacticalRole,
    Other,
}

impl PromiseKind {
    pub fn as_i18n_key(&self) -> &'static str {
        match self {
            PromiseKind::PlayingTime => "promise_kind_playing_time",
            PromiseKind::SquadStatus => "promise_kind_squad_status",
            PromiseKind::NewSigning => "promise_kind_new_signing",
            PromiseKind::ContractRenewal => "promise_kind_contract_renewal",
            PromiseKind::TacticalRole => "promise_kind_tactical_role",
            PromiseKind::Other => "promise_kind_other",
        }
    }
}

/// Concrete football reason a manager singled the player out for
/// criticism. Closed enum so the renderer can pick a stable, translated
/// "what specifically did the manager focus on" sentence per variant.
/// `None` (i.e. legacy emit sites that haven't picked one) keeps the
/// renderer on the topic + tone fallback.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ManagerCriticismReason {
    /// Player ignored a specific tactical assignment (didn't track a
    /// runner, drifted out of position).
    MissedAssignment,
    /// Pressing triggers were skipped or recovery runs were too slow.
    PoorPressing,
    /// A direct mistake changed the match state (gave the ball away in a
    /// dangerous area, missed a clearance, gave away a penalty).
    CostlyError,
    /// Training-ground intensity dropped below squad standards.
    LowTrainingIntensity,
    /// Body language / reaction when things went wrong was the issue,
    /// not the ability.
    PoorBodyLanguage,
    /// Player publicly complained about the manager / team.
    PublicComplaint,
    /// Late arrival, dressing-room standards, off-field discipline.
    LateArrival,
    /// Player ignored a specific tactical instruction the manager had
    /// drilled in pre-match.
    IgnoredTacticalInstruction,
    /// Repeat occurrence — manager has already addressed this.
    RepeatedIncident,
    /// Generic catch-all — renderer falls back to topic/tone copy.
    Other,
}

impl ManagerCriticismReason {
    pub fn as_i18n_key(&self) -> &'static str {
        match self {
            ManagerCriticismReason::MissedAssignment => {
                "manager_criticism_reason_missed_assignment"
            }
            ManagerCriticismReason::PoorPressing => "manager_criticism_reason_poor_pressing",
            ManagerCriticismReason::CostlyError => "manager_criticism_reason_costly_error",
            ManagerCriticismReason::LowTrainingIntensity => {
                "manager_criticism_reason_low_intensity"
            }
            ManagerCriticismReason::PoorBodyLanguage => "manager_criticism_reason_body_language",
            ManagerCriticismReason::PublicComplaint => "manager_criticism_reason_public_complaint",
            ManagerCriticismReason::LateArrival => "manager_criticism_reason_late_arrival",
            ManagerCriticismReason::IgnoredTacticalInstruction => {
                "manager_criticism_reason_ignored_instruction"
            }
            ManagerCriticismReason::RepeatedIncident => "manager_criticism_reason_repeated",
            ManagerCriticismReason::Other => "manager_criticism_reason_other",
        }
    }

    /// Renderer headline-token suffix. Must match the `event_manager_criticism_<token>`
    /// key family in the locale bundles.
    pub fn as_headline_token(&self) -> &'static str {
        match self {
            ManagerCriticismReason::MissedAssignment => "missed_assignment",
            ManagerCriticismReason::PoorPressing => "pressing",
            ManagerCriticismReason::CostlyError => "costly_error",
            ManagerCriticismReason::LowTrainingIntensity => "training_intensity",
            ManagerCriticismReason::PoorBodyLanguage => "body_language",
            ManagerCriticismReason::PublicComplaint => "public_complaint",
            ManagerCriticismReason::LateArrival => "late_arrival",
            ManagerCriticismReason::IgnoredTacticalInstruction => "tactical_discipline",
            ManagerCriticismReason::RepeatedIncident => "repeated",
            ManagerCriticismReason::Other => "other",
        }
    }
}

#[derive(Debug, Clone)]
pub struct ManagerInteractionEventContext {
    pub topic: ManagerInteractionTopic,
    pub tone: ManagerInteractionTone,
    pub acceptance: PlayerAcceptance,
    pub manager_staff_id: Option<u32>,
    pub trust_in_manager: Option<f32>,
    pub promise_kind: Option<PromiseKind>,
    pub promise_credibility: Option<f32>,
    /// Concrete reason behind a `ManagerCriticism` emit. Optional so
    /// other interaction types (praise, tactical instruction, promise
    /// kept/broken) leave it `None` without forcing a meaningless value.
    pub criticism_reason: Option<ManagerCriticismReason>,
    /// Match rating that triggered the interaction, if known. Lets the
    /// renderer surface the concrete number alongside the headline.
    pub match_rating: Option<f32>,
    /// True when the manager has flagged the same issue inside the last
    /// month — escalates the outlook copy.
    pub repeated_recently: bool,
}

impl ManagerInteractionEventContext {
    pub fn new(
        topic: ManagerInteractionTopic,
        tone: ManagerInteractionTone,
        acceptance: PlayerAcceptance,
    ) -> Self {
        Self {
            topic,
            tone,
            acceptance,
            manager_staff_id: None,
            trust_in_manager: None,
            promise_kind: None,
            promise_credibility: None,
            criticism_reason: None,
            match_rating: None,
            repeated_recently: false,
        }
    }

    pub fn with_manager_staff_id(mut self, id: u32) -> Self {
        self.manager_staff_id = Some(id);
        self
    }
    pub fn with_trust(mut self, trust: f32) -> Self {
        self.trust_in_manager = Some(trust);
        self
    }
    pub fn with_promise(mut self, kind: PromiseKind, credibility: f32) -> Self {
        self.promise_kind = Some(kind);
        self.promise_credibility = Some(credibility);
        self
    }
    pub fn with_criticism_reason(mut self, reason: ManagerCriticismReason) -> Self {
        self.criticism_reason = Some(reason);
        self
    }
    pub fn with_match_rating(mut self, rating: f32) -> Self {
        self.match_rating = Some(rating);
        self
    }
    pub fn with_repeated_recently(mut self, repeated: bool) -> Self {
        self.repeated_recently = repeated;
        self
    }
}
