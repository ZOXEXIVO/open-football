/// Concrete football reason behind a `ConflictWithTeammate` emit.
/// Closed enum — the renderer maps each to a localised headline + cause
/// sentence so the user reads "Clashed with Edwards over training
/// standards" instead of the generic "Had a disagreement with a
/// teammate" line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TeammateConflictReason {
    /// Difference in how seriously each player took training.
    TrainingStandards,
    /// Both compete for the same shirt / minutes; small spark carried
    /// extra weight.
    PositionalRivalry,
    /// One player resented the other's wage / squad-status differential.
    WageJealousy,
    /// On-pitch breakdown each blamed on the other.
    TacticalBlame,
    /// Personality friction — temperament, ego, public profile.
    PersonalityClash,
    /// Foreign signing struggling to integrate with a senior teammate.
    LanguageBarrier,
    /// Senior figure challenged the player's standards in the dressing
    /// room.
    LeadershipChallenge,
    /// Public / media comments (player or partner aired the friction).
    MediaComments,
    /// Generic catch-all — renderer uses the legacy headline copy.
    Other,
}

impl TeammateConflictReason {
    pub fn as_i18n_key(&self) -> &'static str {
        match self {
            TeammateConflictReason::TrainingStandards => {
                "teammate_conflict_reason_training_standards"
            }
            TeammateConflictReason::PositionalRivalry => {
                "teammate_conflict_reason_positional_rivalry"
            }
            TeammateConflictReason::WageJealousy => "teammate_conflict_reason_wage_jealousy",
            TeammateConflictReason::TacticalBlame => "teammate_conflict_reason_tactical_blame",
            TeammateConflictReason::PersonalityClash => {
                "teammate_conflict_reason_personality_clash"
            }
            TeammateConflictReason::LanguageBarrier => "teammate_conflict_reason_language_barrier",
            TeammateConflictReason::LeadershipChallenge => {
                "teammate_conflict_reason_leadership_challenge"
            }
            TeammateConflictReason::MediaComments => "teammate_conflict_reason_media_comments",
            TeammateConflictReason::Other => "teammate_conflict_reason_other",
        }
    }

    /// Token used to compose the partner-aware headline key
    /// (`event_teammate_conflict_<token>_named`).
    pub fn as_headline_token(&self) -> &'static str {
        match self {
            TeammateConflictReason::TrainingStandards => "training_standards",
            TeammateConflictReason::PositionalRivalry => "positional_rivalry",
            TeammateConflictReason::WageJealousy => "wage_jealousy",
            TeammateConflictReason::TacticalBlame => "tactical_blame",
            TeammateConflictReason::PersonalityClash => "personality_clash",
            TeammateConflictReason::LanguageBarrier => "language_barrier",
            TeammateConflictReason::LeadershipChallenge => "leadership_challenge",
            TeammateConflictReason::MediaComments => "media_comments",
            TeammateConflictReason::Other => "other",
        }
    }
}

/// Where the conflict played out. Drives the "in the dressing room",
/// "on the training ground", "in front of the cameras" copy variant so
/// the same reason reads differently depending on the setting.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConflictLocation {
    TrainingGround,
    DressingRoom,
    Match,
    Media,
    TeamMeeting,
}

impl ConflictLocation {
    pub fn as_i18n_key(&self) -> &'static str {
        match self {
            ConflictLocation::TrainingGround => "conflict_location_training_ground",
            ConflictLocation::DressingRoom => "conflict_location_dressing_room",
            ConflictLocation::Match => "conflict_location_match",
            ConflictLocation::Media => "conflict_location_media",
            ConflictLocation::TeamMeeting => "conflict_location_team_meeting",
        }
    }
}

/// Structured payload for `ConflictWithTeammate` emits. The closed
/// `reason` + `location` axes drive the headline + cause + outlook copy
/// so the rendered row always reads as a specific football moment, not
/// the generic "argued with a teammate" filler. Optional fields are
/// captured by the emit site when known and skipped otherwise — the
/// renderer hides whatever is missing rather than fabricating it.
#[derive(Debug, Clone)]
pub struct TeammateConflictContext {
    pub reason: TeammateConflictReason,
    pub location: ConflictLocation,
}

impl TeammateConflictContext {
    pub fn new(reason: TeammateConflictReason, location: ConflictLocation) -> Self {
        Self { reason, location }
    }
}
