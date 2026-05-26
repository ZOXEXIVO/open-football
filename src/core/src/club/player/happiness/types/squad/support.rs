/// Where the support / approval came from. Drives the renderer's
/// "who reacted" line and the headline variant.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SupportSource {
    Manager,
    /// Reserved for future use — captain / senior pro speech moments.
    DressingRoomLeader,
    /// Reaction from the squad as a whole (post-match huddle, training
    /// ground recognition).
    Squad,
    Supporters,
    Media,
}

impl SupportSource {
    pub fn as_i18n_key(&self) -> &'static str {
        match self {
            SupportSource::Manager => "support_source_manager",
            SupportSource::DressingRoomLeader => "support_source_dressing_room_leader",
            SupportSource::Squad => "support_source_squad",
            SupportSource::Supporters => "support_source_supporters",
            SupportSource::Media => "support_source_media",
        }
    }
}

/// Where the moment played out. Drives setting-aware copy ("private
/// chat", "in front of the home crowd", "in the dressing room").
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SupportSetting {
    PrivateTalk,
    TrainingGround,
    DressingRoom,
    Touchline,
    HomeCrowd,
    AwayEnd,
    PostMatch,
}

impl SupportSetting {
    pub fn as_i18n_key(&self) -> &'static str {
        match self {
            SupportSetting::PrivateTalk => "support_setting_private",
            SupportSetting::TrainingGround => "support_setting_training_ground",
            SupportSetting::DressingRoom => "support_setting_dressing_room",
            SupportSetting::Touchline => "support_setting_touchline",
            SupportSetting::HomeCrowd => "support_setting_home_crowd",
            SupportSetting::AwayEnd => "support_setting_away_end",
            SupportSetting::PostMatch => "support_setting_post_match",
        }
    }
}

/// Why the reaction happened. Closed enum — adding a new trigger means
/// adding renderer copy in every locale, so we want the surface to stay
/// finite.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SupportTrigger {
    HighRating,
    PlayerOfMatch,
    GoalContribution,
    DecisiveMoment,
    PoorMorale,
    PoorFormRecovery,
    BigMatch,
    Derby,
    CupTie,
    LeadershipMoment,
    TeamTrailingAtHalfTime,
    TeamWon,
    YoungPlayerConfidence,
    ReturningFromInjury,
    /// Generic / unknown trigger — renderer falls back to the default
    /// headline copy.
    Generic,
}

impl SupportTrigger {
    pub fn as_i18n_key(&self) -> &'static str {
        match self {
            SupportTrigger::HighRating => "support_trigger_high_rating",
            SupportTrigger::PlayerOfMatch => "support_trigger_pom",
            SupportTrigger::GoalContribution => "support_trigger_goal_contribution",
            SupportTrigger::DecisiveMoment => "support_trigger_decisive_moment",
            SupportTrigger::PoorMorale => "support_trigger_poor_morale",
            SupportTrigger::PoorFormRecovery => "support_trigger_form_recovery",
            SupportTrigger::BigMatch => "support_trigger_big_match",
            SupportTrigger::Derby => "support_trigger_derby",
            SupportTrigger::CupTie => "support_trigger_cup_tie",
            SupportTrigger::LeadershipMoment => "support_trigger_leadership_moment",
            SupportTrigger::TeamTrailingAtHalfTime => "support_trigger_trailing_half_time",
            SupportTrigger::TeamWon => "support_trigger_team_won",
            SupportTrigger::YoungPlayerConfidence => "support_trigger_young_player_confidence",
            SupportTrigger::ReturningFromInjury => "support_trigger_returning_from_injury",
            SupportTrigger::Generic => "support_trigger_generic",
        }
    }
}

/// Render-safe mirror of `team_talks::MatchPhase` — kept here so the
/// support context can carry the phase without dragging the team-talks
/// module into the events / renderer crates.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SupportMatchPhase {
    PreMatch,
    HalfTime,
    FullTime,
    InMatch,
}

impl SupportMatchPhase {
    pub fn as_i18n_key(&self) -> &'static str {
        match self {
            SupportMatchPhase::PreMatch => "support_phase_pre_match",
            SupportMatchPhase::HalfTime => "support_phase_half_time",
            SupportMatchPhase::FullTime => "support_phase_full_time",
            SupportMatchPhase::InMatch => "support_phase_in_match",
        }
    }
}

/// Render-safe mirror of `TeamTalkTone` / `InteractionTone`. Kept as a
/// closed enum so the renderer can pick deterministic copy without
/// importing the team-talks types.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SupportTone {
    Praise,
    Criticise,
    Encourage,
    Passionate,
    Calm,
    Supportive,
    Honest,
    Demanding,
}

impl SupportTone {
    pub fn as_i18n_key(&self) -> &'static str {
        match self {
            SupportTone::Praise => "support_tone_praise",
            SupportTone::Criticise => "support_tone_criticise",
            SupportTone::Encourage => "support_tone_encourage",
            SupportTone::Passionate => "support_tone_passionate",
            SupportTone::Calm => "support_tone_calm",
            SupportTone::Supportive => "support_tone_supportive",
            SupportTone::Honest => "support_tone_honest",
            SupportTone::Demanding => "support_tone_demanding",
        }
    }
}

/// Structured payload for support / approval events
/// (`ManagerEncouragement`, `DressingRoomSpeech`, `FanPraise`,
/// `FansChantPlayerName`). The emit site fills in what it knows; the
/// renderer turns that into a contextual headline + reason + outlook.
///
/// Every field is optional except the three classification axes
/// (`source`, `setting`, `trigger`), so partial information is never a
/// blocker — the renderer only references the fields it has.
#[derive(Debug, Clone)]
pub struct SupportEventContext {
    pub source: SupportSource,
    pub setting: SupportSetting,
    pub trigger: SupportTrigger,
    /// Staff id of the speaker (manager, coach), if known.
    pub speaker_staff_id: Option<u32>,
    /// Player id of the speaker (captain / senior pro), if known.
    pub source_player_id: Option<u32>,
    pub match_rating: Option<f32>,
    pub goals: Option<u8>,
    pub assists: Option<u8>,
    pub team_won: Option<bool>,
    pub is_derby: bool,
    pub is_cup: bool,
    pub phase: Option<SupportMatchPhase>,
    pub tone: Option<SupportTone>,
}

impl SupportEventContext {
    pub fn new(source: SupportSource, setting: SupportSetting, trigger: SupportTrigger) -> Self {
        Self {
            source,
            setting,
            trigger,
            speaker_staff_id: None,
            source_player_id: None,
            match_rating: None,
            goals: None,
            assists: None,
            team_won: None,
            is_derby: false,
            is_cup: false,
            phase: None,
            tone: None,
        }
    }

    pub fn with_speaker_staff_id(mut self, id: u32) -> Self {
        self.speaker_staff_id = Some(id);
        self
    }

    pub fn with_source_player_id(mut self, id: u32) -> Self {
        self.source_player_id = Some(id);
        self
    }

    pub fn with_match_rating(mut self, rating: f32) -> Self {
        self.match_rating = Some(rating);
        self
    }

    pub fn with_goals(mut self, goals: u8) -> Self {
        self.goals = Some(goals);
        self
    }

    pub fn with_assists(mut self, assists: u8) -> Self {
        self.assists = Some(assists);
        self
    }

    pub fn with_team_won(mut self, team_won: bool) -> Self {
        self.team_won = Some(team_won);
        self
    }

    pub fn with_derby(mut self, is_derby: bool) -> Self {
        self.is_derby = is_derby;
        self
    }

    pub fn with_cup(mut self, is_cup: bool) -> Self {
        self.is_cup = is_cup;
        self
    }

    pub fn with_phase(mut self, phase: SupportMatchPhase) -> Self {
        self.phase = Some(phase);
        self
    }

    pub fn with_tone(mut self, tone: SupportTone) -> Self {
        self.tone = Some(tone);
        self
    }
}
