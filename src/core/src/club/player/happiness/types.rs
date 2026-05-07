use crate::club::player::behaviour_config::HappinessConfig;
use crate::club::player::contract::PlayerSquadStatus;
use chrono::NaiveDate;

#[derive(Debug, Clone)]
pub struct PlayerHappiness {
    pub morale: f32,
    pub factors: HappinessFactors,
    pub recent_events: Vec<HappinessEvent>,
    pub last_salary_negotiation: Option<NaiveDate>,
    /// EMA of "did I start this competitive match?" — updated on every
    /// non-friendly match. Drives the WonStartingPlace / LostStartingPlace
    /// transitions instead of raw season totals so a mid-season turnaround
    /// is felt promptly. Range 0.0..=1.0; 0.5 baseline before first match.
    pub starter_ratio: f32,
    /// Rolling count of recent competitive appearances feeding `starter_ratio`.
    /// Caps at u8::MAX; only the first 5 appearances are required before role
    /// transitions can fire (avoids one good week swinging the verdict).
    pub appearances_tracked: u8,
    /// Sticky flag — true once the player has been recognised as an
    /// established starter, false until they fall back below the bench
    /// threshold. Used to emit one-shot WonStartingPlace / LostStartingPlace
    /// events on the crossing rather than every matchday in range.
    pub is_established_starter: bool,
    /// Competitive appearances since the player's last competitive goal.
    /// Drives `GoalDroughtEnded` / `ScoringDroughtConcern` without an
    /// unbounded per-match history. Saturates at u8::MAX.
    pub apps_since_last_competitive_goal: u8,
    /// Bit-ring of the last 5 competitive appearances: 1 = rating < 6.0,
    /// 0 otherwise. Bit 0 is the most recent appearance. Drives the
    /// "two of the last five" trigger for `MediaPressureMounting` as a
    /// true sliding window — bad games never fall off a block boundary.
    pub recent_low_rating_mask: u8,
    /// Number of appearances currently encoded in `recent_low_rating_mask`,
    /// capped at 5. The trigger only fires once the mask is full, so a
    /// player who has just made his second poor appearance after one good
    /// one doesn't fire on a 1-of-2 ratio.
    pub recent_low_rating_len: u8,
}

#[derive(Debug, Clone, Default)]
pub struct HappinessFactors {
    pub playing_time: f32,
    pub salary_satisfaction: f32,
    pub manager_relationship: f32,
    pub ambition_fit: f32,
    pub injury_frustration: f32,
    pub recent_praise: f32,
    pub recent_discipline: f32,

    // ── Derived "life in the team" factors ──────────────────────
    /// Does the player understand his role and how he's being used?
    /// Drops on RoleMismatch / repeated tactical-role talks; rises
    /// when the player is in his preferred position with consistent
    /// minutes. Range roughly -8..+5.
    pub role_clarity: f32,
    /// Does the player believe the coaching staff is competent enough
    /// to coach him? Reads coach attribute scores against the player's
    /// own ability. A world-class player at a club with weak coaching
    /// loses respect quickly. Range roughly -8..+6.
    pub coach_credibility: f32,
    /// Where does the player sit in the dressing room — respected,
    /// resented, isolated, or influential? Built from leadership,
    /// reputation, and relations. Range roughly -6..+8.
    pub dressing_room_status: f32,
    /// Cultural / structural fit with the club — facilities, league
    /// level, language, lifestyle, ambition match. Range roughly -8..+6.
    pub club_fit: f32,
    /// Pressure load from fans, media, board expectations relative to
    /// the player's `pressure` personality. Range roughly -8..+3.
    pub pressure_load: f32,
    /// Trust the player has in the manager's word — distinct from the
    /// general manager_relationship. Built from kept-vs-broken
    /// promises and recent broken-promise count. Range roughly -10..+6.
    pub promise_trust: f32,
}

#[derive(Debug, Clone)]
pub struct HappinessEvent {
    pub event_type: HappinessEventType,
    pub magnitude: f32,
    pub days_ago: u16,
    /// Optional teammate / partner involved in this event. Lets the UI
    /// link the event description to a specific player (e.g. who the
    /// player bonded with, who the close friend was, who the mentor was).
    /// `None` for events that don't naturally involve a specific peer.
    pub partner_player_id: Option<u32>,
    /// Structured cause/evidence/impact payload attached at emit time.
    /// `None` for legacy events whose emit-site has not been upgraded yet
    /// (renderer falls back to the i18n string for those).
    pub context: Option<HappinessEventContext>,
}

/// Severity tier derived from applied magnitude. Renderers and tests treat
/// these as ordinal — Minor < Moderate < Serious < Major.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HappinessEventSeverity {
    Minor,
    Moderate,
    Serious,
    Major,
}

impl HappinessEventSeverity {
    /// Stable mapping from magnitude (absolute value) to severity.
    /// Thresholds are deliberately conservative — most pair events sit
    /// in [0.5, 3.0] (Minor / Moderate); Serious starts at 4 (training
    /// bust-up scale); Major is reserved for headline blow-ups (≥6).
    pub fn from_magnitude(magnitude: f32) -> Self {
        let m = magnitude.abs();
        if m >= 6.0 {
            HappinessEventSeverity::Major
        } else if m >= 4.0 {
            HappinessEventSeverity::Serious
        } else if m >= 2.0 {
            HappinessEventSeverity::Moderate
        } else {
            HappinessEventSeverity::Minor
        }
    }

    pub fn as_i18n_key(&self) -> &'static str {
        match self {
            HappinessEventSeverity::Minor => "severity_minor",
            HappinessEventSeverity::Moderate => "severity_moderate",
            HappinessEventSeverity::Serious => "severity_serious",
            HappinessEventSeverity::Major => "severity_major",
        }
    }
}

/// Cause category — the football-realistic reason behind the event.
/// Renderer turns this into the "why" sentence; tests assert that emit
/// sites pick the right category for a given `ChangeType` / situation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HappinessEventCause {
    PersonalityClash,
    TrainingFriction,
    PositionalRivalry,
    WageJealousy,
    PoorFormPressure,
    LeadershipDispute,
    TacticalDisagreement,
    AdaptationIsolation,
    MediaPressure,
    MentorDeparture,
    FriendDeparture,
    MatchCooperation,
    NationalityIntegration,
    /// Healthy training-ground partnership — paired bonding off the
    /// pitch (drills, sessions) rather than a match-day moment.
    TrainingPartnership,
    /// Reputation tension: lower-rep player resents an established star
    /// or the star creates friction by his bearing in the dressing room.
    ReputationTension,
    /// Mutual reputation respect — peer-level stars who recognise each
    /// other professionally.
    ReputationAdmiration,
    /// Manager backing — encouragement, praise, public support after a
    /// performance or in private. Used by `ManagerEncouragement` events.
    ManagerSupport,
    /// Supporter appreciation — applause, songs, fan-poll wins directed
    /// at the player after a contribution. Used by `FanPraise` events.
    SupporterAppreciation,
    /// Stadium-wide identification with the player — chants, lasting
    /// connection beyond a single performance. Used by
    /// `FansChantPlayerName` events.
    SupporterIdentification,
    /// Dressing-room talk lift — manager team-talk that landed for this
    /// player. Used by `DressingRoomSpeech` events.
    DressingRoomLift,
    /// Catch-all for unstructured causes; renderer falls back to the
    /// generic i18n line. New emit sites should pick a real category.
    Other,
}

impl HappinessEventCause {
    pub fn as_i18n_key(&self) -> &'static str {
        match self {
            HappinessEventCause::PersonalityClash => "cause_personality_clash",
            HappinessEventCause::TrainingFriction => "cause_training_friction",
            HappinessEventCause::PositionalRivalry => "cause_positional_rivalry",
            HappinessEventCause::WageJealousy => "cause_wage_jealousy",
            HappinessEventCause::PoorFormPressure => "cause_poor_form_pressure",
            HappinessEventCause::LeadershipDispute => "cause_leadership_dispute",
            HappinessEventCause::TacticalDisagreement => "cause_tactical_disagreement",
            HappinessEventCause::AdaptationIsolation => "cause_adaptation_isolation",
            HappinessEventCause::MediaPressure => "cause_media_pressure",
            HappinessEventCause::MentorDeparture => "cause_mentor_departure",
            HappinessEventCause::FriendDeparture => "cause_friend_departure",
            HappinessEventCause::MatchCooperation => "cause_match_cooperation",
            HappinessEventCause::NationalityIntegration => "cause_nationality_integration",
            HappinessEventCause::TrainingPartnership => "cause_training_partnership",
            HappinessEventCause::ReputationTension => "cause_reputation_tension",
            HappinessEventCause::ReputationAdmiration => "cause_reputation_admiration",
            HappinessEventCause::ManagerSupport => "cause_manager_support",
            HappinessEventCause::SupporterAppreciation => "cause_supporter_appreciation",
            HappinessEventCause::SupporterIdentification => "cause_supporter_identification",
            HappinessEventCause::DressingRoomLift => "cause_dressing_room_lift",
            HappinessEventCause::Other => "cause_other",
        }
    }
}

/// Where the fallout lands — a single dressing-room incident, a wider
/// squad-mood ripple, or a public-facing media moment. Used to colour
/// the "what it affected" line in the UI.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HappinessEventScope {
    Personal,
    DressingRoom,
    TrainingGround,
    MatchDay,
    Media,
    Boardroom,
}

impl HappinessEventScope {
    pub fn as_i18n_key(&self) -> &'static str {
        match self {
            HappinessEventScope::Personal => "scope_personal",
            HappinessEventScope::DressingRoom => "scope_dressing_room",
            HappinessEventScope::TrainingGround => "scope_training_ground",
            HappinessEventScope::MatchDay => "scope_match_day",
            HappinessEventScope::Media => "scope_media",
            HappinessEventScope::Boardroom => "scope_boardroom",
        }
    }
}

/// Structured explanation payload carried alongside a `HappinessEvent`.
/// Filled in at emit time so the same simulation tick captures both the
/// magnitude and the football-realistic context — renderers turn this
/// into the "who / why / how serious / what it affected / what's next"
/// sentences instead of guessing from the event type alone.
///
/// `None` evidence fields mean "the emit site didn't know" — the UI
/// hides the corresponding sentence rather than fabricating one.
#[derive(Debug, Clone)]
pub struct HappinessEventContext {
    pub cause: HappinessEventCause,
    pub severity: HappinessEventSeverity,
    pub scope: HappinessEventScope,
    /// Relationship level on the partner side BEFORE the relation update
    /// landed — captured by the emit site so the renderer can describe
    /// the relationship as it existed when the event fired.
    pub relationship_level_before: Option<f32>,
    /// Relationship level AFTER the update — lets the renderer call out
    /// trend direction without recomputing the delta.
    pub relationship_level_after: Option<f32>,
    /// Trust 0..100 at emit time, captured pre-update.
    pub relationship_trust: Option<f32>,
    /// Friendship 0..100 at emit time, captured pre-update.
    pub relationship_friendship: Option<f32>,
    /// Professional respect 0..100 at emit time, captured pre-update.
    pub relationship_professional_respect: Option<f32>,
    /// Render-safe mirror of the `ChangeType` that triggered the event.
    /// Lets the renderer pick cause-specific reason copy without
    /// importing the relations crate's type tree.
    pub change_type: Option<HappinessEventChangeKind>,
    /// Closed-set evidence list — concrete football reasons attached at
    /// emit time so the renderer can produce evidence-driven, varied
    /// explanations instead of one generic sentence per cause.
    pub evidence: Vec<HappinessEventEvidence>,
    /// Hint for the "what may happen next" sentence. The catalog is
    /// closed so renderers can pick stable, translated copy.
    pub follow_up: Option<HappinessEventFollowUp>,
    /// Match-selection explanation payload. Populated by the squad
    /// selector when a player was omitted (left out of XI, dropped to
    /// bench, or off the matchday squad entirely) so the renderer can
    /// describe who was preferred and why instead of falling back to a
    /// generic "Dropped from match squad" line. `None` for non-selection
    /// events.
    pub selection_context: Option<MatchSelectionContext>,
    /// Support / talk / supporter explanation payload. Populated by the
    /// emit sites for `ManagerEncouragement`, `DressingRoomSpeech`,
    /// `FanPraise`, and `FansChantPlayerName` so the renderer can
    /// describe who reacted, where, why, and what may follow. `None` for
    /// other event types.
    pub support_context: Option<SupportEventContext>,
    /// Transfer-interest explanation payload. Populated for events
    /// covering scout attendance, rumours, agent leaks, concrete
    /// interest, rejected bids, talks expected, and interest cooling.
    /// Lets the renderer produce contextual headlines + reasons +
    /// reactions tied to the interested club, stage, and player
    /// personality. `None` for non-interest events.
    pub transfer_interest_context: Option<TransferInterestContext>,
    /// Training-session explanation payload. Populated for
    /// `GoodTraining` / `PoorTraining` so the renderer can describe
    /// why the session swung — fatigue, response to criticism, return
    /// from injury, setting standards — instead of "Performed well /
    /// poorly in training".
    pub training_context: Option<TrainingEventContext>,
    /// Manager-interaction explanation payload. Drives
    /// `ManagerPraise` / `ManagerDiscipline` / `ManagerCriticism` /
    /// `ManagerTacticalInstruction` / `PromiseKept` / `PromiseBroken`
    /// rendering with topic + tone + trust state.
    pub manager_interaction_context: Option<ManagerInteractionEventContext>,
    /// Contract / agent explanation payload — wage delta, promised
    /// status, agent pressure, loyalty discount. Drives
    /// `ContractOffer` / `ContractRenewal` / `ContractTerminated` /
    /// `SalaryShock` / `SalaryBoost` rendering.
    pub contract_context: Option<ContractEventContext>,
    /// Injury-recovery explanation payload — recovery length,
    /// sharpness vs match readiness, setback flag, recurrence concern.
    /// Drives `InjuryReturn` rendering.
    pub injury_context: Option<InjuryRecoveryEventContext>,
    /// Match-performance explanation payload — opponent context,
    /// rating, goals/assists, derby/cup, drought-ending.
    pub match_performance_context: Option<MatchPerformanceEventContext>,
    /// Role / squad-status explanation payload — depth chart,
    /// formation slot, rotation reason, role-clarity drift.
    pub role_status_context: Option<RoleStatusEventContext>,
    /// National-team explanation payload — first-cap vs recall,
    /// emergency call-up, dropped reason, tournament squad.
    pub national_team_context: Option<NationalTeamEventContext>,
    /// Leadership / dressing-room explanation payload — captaincy,
    /// emergence, mentorship, mediation.
    pub leadership_context: Option<LeadershipEventContext>,
    /// Media / fan / public-life explanation payload — narrative
    /// direction, audience, source.
    pub media_fan_context: Option<MediaFanEventContext>,
    /// Personal adaptation payload — settling into a new club /
    /// country, language progress, family / cultural cues.
    pub personal_adaptation_context: Option<PersonalAdaptationEventContext>,
    /// Loan-specific payload — parent-club view, minutes concern,
    /// recall discussion.
    pub loan_context: Option<LoanEventContext>,
    /// Recognition / award explanation payload — POM, POS, top
    /// scorer, world player of year, national-team debut. Lets the
    /// renderer explain margin, season totals, and runner-up instead
    /// of the bare award name.
    pub recognition_context: Option<RecognitionEventContext>,
    /// Season-outcome explanation payload for relegation /
    /// relegation-fear events. Carries final position, points gap to
    /// safety, and matches remaining at the moment the event fired.
    pub season_outcome_context: Option<SeasonOutcomeContext>,
    /// Regulation / squad-registration payload — slot type, slot
    /// counts, replacement player. Lets the renderer explain "left
    /// out to free a non-EU slot for the new signing" rather than a
    /// generic "Squad registration omitted".
    pub regulation_context: Option<RegulationEventContext>,
}

impl HappinessEventContext {
    pub fn new(
        cause: HappinessEventCause,
        severity: HappinessEventSeverity,
        scope: HappinessEventScope,
    ) -> Self {
        Self {
            cause,
            severity,
            scope,
            relationship_level_before: None,
            relationship_level_after: None,
            relationship_trust: None,
            relationship_friendship: None,
            relationship_professional_respect: None,
            change_type: None,
            evidence: Vec::new(),
            follow_up: None,
            selection_context: None,
            support_context: None,
            transfer_interest_context: None,
            training_context: None,
            manager_interaction_context: None,
            contract_context: None,
            injury_context: None,
            match_performance_context: None,
            role_status_context: None,
            national_team_context: None,
            leadership_context: None,
            media_fan_context: None,
            personal_adaptation_context: None,
            loan_context: None,
            recognition_context: None,
            season_outcome_context: None,
            regulation_context: None,
        }
    }

    pub fn with_relationship_level(mut self, level: f32) -> Self {
        self.relationship_level_before = Some(level);
        self
    }

    /// Capture both the pre- and post-update relationship level. Use
    /// this from emit sites that have access to a snapshot taken before
    /// `Relations::update_with_type` was called.
    pub fn with_relationship_levels(mut self, before: f32, after: f32) -> Self {
        self.relationship_level_before = Some(before);
        self.relationship_level_after = Some(after);
        self
    }

    /// Attach the trust / friendship / professional-respect axes from
    /// the partner relation at emit time. These drive the
    /// `LowTrust` / `LowFriendship` / `LowProfessionalRespect` evidence
    /// derivation.
    pub fn with_relationship_axes(
        mut self,
        trust: f32,
        friendship: f32,
        professional_respect: f32,
    ) -> Self {
        self.relationship_trust = Some(trust);
        self.relationship_friendship = Some(friendship);
        self.relationship_professional_respect = Some(professional_respect);
        self
    }

    pub fn with_change_kind(mut self, kind: HappinessEventChangeKind) -> Self {
        self.change_type = Some(kind);
        self
    }

    pub fn with_evidence(mut self, evidence: HappinessEventEvidence) -> Self {
        if !self.evidence.contains(&evidence) {
            self.evidence.push(evidence);
        }
        self
    }

    pub fn with_evidence_iter<I>(mut self, iter: I) -> Self
    where
        I: IntoIterator<Item = HappinessEventEvidence>,
    {
        for ev in iter {
            if !self.evidence.contains(&ev) {
                self.evidence.push(ev);
            }
        }
        self
    }

    pub fn with_follow_up(mut self, follow_up: HappinessEventFollowUp) -> Self {
        self.follow_up = Some(follow_up);
        self
    }

    pub fn with_selection_context(mut self, ctx: MatchSelectionContext) -> Self {
        self.selection_context = Some(ctx);
        self
    }

    pub fn with_support_context(mut self, ctx: SupportEventContext) -> Self {
        self.support_context = Some(ctx);
        self
    }

    pub fn with_transfer_interest_context(mut self, ctx: TransferInterestContext) -> Self {
        self.transfer_interest_context = Some(ctx);
        self
    }

    pub fn with_training_context(mut self, ctx: TrainingEventContext) -> Self {
        self.training_context = Some(ctx);
        self
    }

    pub fn with_manager_interaction_context(mut self, ctx: ManagerInteractionEventContext) -> Self {
        self.manager_interaction_context = Some(ctx);
        self
    }

    pub fn with_contract_context(mut self, ctx: ContractEventContext) -> Self {
        self.contract_context = Some(ctx);
        self
    }

    pub fn with_injury_context(mut self, ctx: InjuryRecoveryEventContext) -> Self {
        self.injury_context = Some(ctx);
        self
    }

    pub fn with_match_performance_context(mut self, ctx: MatchPerformanceEventContext) -> Self {
        self.match_performance_context = Some(ctx);
        self
    }

    pub fn with_role_status_context(mut self, ctx: RoleStatusEventContext) -> Self {
        self.role_status_context = Some(ctx);
        self
    }

    pub fn with_national_team_context(mut self, ctx: NationalTeamEventContext) -> Self {
        self.national_team_context = Some(ctx);
        self
    }

    pub fn with_leadership_context(mut self, ctx: LeadershipEventContext) -> Self {
        self.leadership_context = Some(ctx);
        self
    }

    pub fn with_media_fan_context(mut self, ctx: MediaFanEventContext) -> Self {
        self.media_fan_context = Some(ctx);
        self
    }

    pub fn with_personal_adaptation_context(mut self, ctx: PersonalAdaptationEventContext) -> Self {
        self.personal_adaptation_context = Some(ctx);
        self
    }

    pub fn with_loan_context(mut self, ctx: LoanEventContext) -> Self {
        self.loan_context = Some(ctx);
        self
    }

    pub fn with_recognition_context(mut self, ctx: RecognitionEventContext) -> Self {
        self.recognition_context = Some(ctx);
        self
    }

    pub fn with_season_outcome_context(mut self, ctx: SeasonOutcomeContext) -> Self {
        self.season_outcome_context = Some(ctx);
        self
    }

    pub fn with_regulation_context(mut self, ctx: RegulationEventContext) -> Self {
        self.regulation_context = Some(ctx);
        self
    }

    /// Returns the number of specialized payload contexts attached to
    /// this event. Specialized payloads are mutually exclusive at the
    /// modelling level — an event is *either* a selection event, *or*
    /// a transfer-interest event, etc. — so this should never exceed 1.
    /// Used by tests as a soft invariant on emit-site code.
    pub fn specialized_payload_count(&self) -> usize {
        let mut n = 0;
        if self.selection_context.is_some() { n += 1; }
        if self.support_context.is_some() { n += 1; }
        if self.transfer_interest_context.is_some() { n += 1; }
        if self.training_context.is_some() { n += 1; }
        if self.manager_interaction_context.is_some() { n += 1; }
        if self.contract_context.is_some() { n += 1; }
        if self.injury_context.is_some() { n += 1; }
        if self.match_performance_context.is_some() { n += 1; }
        if self.role_status_context.is_some() { n += 1; }
        if self.national_team_context.is_some() { n += 1; }
        if self.leadership_context.is_some() { n += 1; }
        if self.media_fan_context.is_some() { n += 1; }
        if self.personal_adaptation_context.is_some() { n += 1; }
        if self.loan_context.is_some() { n += 1; }
        if self.recognition_context.is_some() { n += 1; }
        if self.season_outcome_context.is_some() { n += 1; }
        if self.regulation_context.is_some() { n += 1; }
        n
    }
}

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

/// Why a player ended up off the team-sheet for a match. Passed in by
/// the squad selector at emit time so the player-events renderer can
/// describe the decision in football-realistic terms ("rested after
/// heavy minutes", "lost out to a fitter teammate", "no natural role
/// in the current shape") instead of a generic drop line.
///
/// Closed enum, mirrored by the renderer's i18n token list. Adding a
/// new variant means adding a copy line in every locale and a renderer
/// branch — fail loud at compile time rather than show the raw key.
#[derive(Debug, Clone)]
pub struct MatchSelectionContext {
    /// Where in the matchday selection ladder the player ended up —
    /// dropped from XI to bench, left off the matchday squad entirely,
    /// or named to the bench but never came on.
    pub scope: SelectionDecisionScope,
    /// Football-realistic reason the manager picked the chosen player
    /// over this one.
    pub reason: SelectionOmissionReason,
    /// Concrete comparison to the player who took the slot. `None`
    /// when no direct counterpart exists (e.g. left out of squad with
    /// no positional rival).
    pub comparison: Option<SelectionComparison>,
    /// Player's expected role given his squad status / promises. Drives
    /// severity and copy variants — a `KeyPlayer` left out reads
    /// differently from a fringe `MainBackupPlayer`.
    pub role: SelectionRole,
    /// Match importance the selection was made under (0.0–1.0). Lets
    /// the renderer tag low-importance cup nights as "rotation" and
    /// soften the impact line.
    pub match_importance: f32,
    /// True when the omission has happened in consecutive matches.
    /// Drives the "if repeated" outlook and the severity bump.
    pub repeated: bool,
    /// True for friendlies / development matches. Renderer dampens the
    /// outlook ("a friendly snub is rarely held against the manager").
    pub is_friendly: bool,
}

/// Bucket the selection decision falls into. Distinct from
/// `SelectionOmissionReason` (the *why*) — this is the *what*.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SelectionDecisionScope {
    /// Named to the bench but never came on.
    UnusedSubstitute,
    /// Dropped from a starting role to the bench — did not start.
    DroppedToBench,
    /// Left out of the matchday squad entirely.
    LeftOutOfMatchdaySquad,
    /// Explicitly rested by the manager (load-management call).
    Rested,
    /// Available, but not picked for non-injury reasons (discipline,
    /// personal). Distinct from full unavailability (suspension /
    /// injury) which is filtered before selection.
    UnavailableButNotInjured,
    /// Cup / low-importance fixture rotation.
    Rotation,
}

impl SelectionDecisionScope {
    pub fn as_i18n_key(&self) -> &'static str {
        match self {
            SelectionDecisionScope::UnusedSubstitute => "selection_scope_unused_substitute",
            SelectionDecisionScope::DroppedToBench => "selection_scope_dropped_to_bench",
            SelectionDecisionScope::LeftOutOfMatchdaySquad => {
                "selection_scope_left_out_of_matchday_squad"
            }
            SelectionDecisionScope::Rested => "selection_scope_rested",
            SelectionDecisionScope::UnavailableButNotInjured => {
                "selection_scope_unavailable_not_injured"
            }
            SelectionDecisionScope::Rotation => "selection_scope_rotation",
        }
    }
}

/// Football-realistic reason the manager picked someone else. Closed
/// enum, every variant maps to a localised sentence the renderer turns
/// into the "why" line. Multiple reasons can apply at once — the
/// selector picks the dominant one (highest weight in the scoring
/// breakdown).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SelectionOmissionReason {
    /// Chosen player was sharper / fresher.
    LowerMatchReadiness,
    /// Manager protected a fragile player (returning, accumulating risk).
    FitnessProtection,
    /// Recent workload drove rotation.
    FatigueManagement,
    /// Bad recent ratings cost the player his place.
    PoorRecentForm,
    /// Tactical shape demands a different profile.
    TacticalMismatch,
    /// Player's positions don't fit any open slot well.
    PositionFitIssue,
    /// Direct rival was preferred on perceived ability.
    TeammatePreferredOnAbility,
    /// Rival was preferred because his form was stronger.
    TeammatePreferredOnForm,
    /// Rival was preferred on physical readiness.
    TeammatePreferredOnFitness,
    /// Coach trusts the rival more (relationship / professional respect).
    TeammatePreferredOnTrust,
    /// Manager preferred the rival to balance the shape (eg. defensive
    /// reliability against a tough opponent).
    TeammatePreferredForTacticalBalance,
    /// Manager promoted a youth player as part of development plan.
    YouthDevelopmentRotation,
    /// Cup / League Cup rotation call.
    CupRotation,
    /// Low-importance match — manager rotated for managed minutes.
    LowMatchImportanceRotation,
    /// Player's squad status doesn't match the moment (e.g. fringe
    /// player overlooked when the manager could afford his best XI).
    SquadStatusMismatch,
    /// Coach has limited trust in the player despite squad-status label.
    ManagerDoesNotTrustPlayer,
    /// New signing still inside the integration window.
    NewcomerStillIntegrating,
    /// Returning from injury — protected start.
    ReturningFromInjury,
    /// Disciplinary call (training-ground row, public apology pending).
    DisciplinarySelection,
    /// Bench-balance call: the manager wanted a different option for
    /// in-match flexibility.
    BenchBalance,
    /// Formation has no slot anywhere near the player's preferred role.
    NoNaturalRoleInFormation,
}

impl SelectionOmissionReason {
    pub fn as_i18n_key(&self) -> &'static str {
        match self {
            SelectionOmissionReason::LowerMatchReadiness => "selection_reason_lower_match_readiness",
            SelectionOmissionReason::FitnessProtection => "selection_reason_fitness_protection",
            SelectionOmissionReason::FatigueManagement => "selection_reason_fatigue_management",
            SelectionOmissionReason::PoorRecentForm => "selection_reason_poor_recent_form",
            SelectionOmissionReason::TacticalMismatch => "selection_reason_tactical_mismatch",
            SelectionOmissionReason::PositionFitIssue => "selection_reason_position_fit_issue",
            SelectionOmissionReason::TeammatePreferredOnAbility => {
                "selection_reason_teammate_preferred_on_ability"
            }
            SelectionOmissionReason::TeammatePreferredOnForm => {
                "selection_reason_teammate_preferred_on_form"
            }
            SelectionOmissionReason::TeammatePreferredOnFitness => {
                "selection_reason_teammate_preferred_on_fitness"
            }
            SelectionOmissionReason::TeammatePreferredOnTrust => {
                "selection_reason_teammate_preferred_on_trust"
            }
            SelectionOmissionReason::TeammatePreferredForTacticalBalance => {
                "selection_reason_teammate_preferred_for_tactical_balance"
            }
            SelectionOmissionReason::YouthDevelopmentRotation => {
                "selection_reason_youth_development_rotation"
            }
            SelectionOmissionReason::CupRotation => "selection_reason_cup_rotation",
            SelectionOmissionReason::LowMatchImportanceRotation => {
                "selection_reason_low_match_importance_rotation"
            }
            SelectionOmissionReason::SquadStatusMismatch => {
                "selection_reason_squad_status_mismatch"
            }
            SelectionOmissionReason::ManagerDoesNotTrustPlayer => {
                "selection_reason_manager_does_not_trust"
            }
            SelectionOmissionReason::NewcomerStillIntegrating => {
                "selection_reason_newcomer_still_integrating"
            }
            SelectionOmissionReason::ReturningFromInjury => {
                "selection_reason_returning_from_injury"
            }
            SelectionOmissionReason::DisciplinarySelection => {
                "selection_reason_disciplinary"
            }
            SelectionOmissionReason::BenchBalance => "selection_reason_bench_balance",
            SelectionOmissionReason::NoNaturalRoleInFormation => {
                "selection_reason_no_natural_role"
            }
        }
    }
}

/// Concrete comparison to the player who took the omitted player's
/// slot. Stores ids and scores for tests / debugging plus the
/// dominant scoring components so the renderer can produce a
/// "stronger condition / sharper form" sentence rather than guessing.
#[derive(Debug, Clone)]
pub struct SelectionComparison {
    /// Player id that was selected for the slot the omitted player
    /// would naturally have filled.
    pub selected_player_id: u32,
    /// Whether the selected player was a starter or substitute.
    pub selected_was_starter: bool,
    /// Position / slot the selected player took. `None` when the
    /// player's preferred role isn't in the formation at all.
    pub slot: Option<SelectionRole>,
    /// Selected player's total score for that slot.
    pub selected_score: f32,
    /// Omitted player's total score for the same slot.
    pub omitted_score: f32,
    /// Top scoring factors where the selected player edged ahead. Up
    /// to four factors, stored in dominance order so the renderer can
    /// pick the first one or two for the comparison sentence.
    pub top_factors: Vec<SelectionScoreFactor>,
}

/// Coarse positional bucket used in the comparison line. Mirrors the
/// engine's positional groupings — keeping it as a render-safe enum
/// avoids dragging the full `PlayerPositionType` into the events
/// module's i18n surface.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SelectionRole {
    Goalkeeper,
    CentreBack,
    Fullback,
    DefensiveMidfielder,
    CentralMidfielder,
    AttackingMidfielder,
    Winger,
    Striker,
    /// Free-floating / unclassified — fallback for unusual slots.
    Other,
}

impl SelectionRole {
    pub fn as_i18n_key(&self) -> &'static str {
        match self {
            SelectionRole::Goalkeeper => "selection_role_goalkeeper",
            SelectionRole::CentreBack => "selection_role_centre_back",
            SelectionRole::Fullback => "selection_role_fullback",
            SelectionRole::DefensiveMidfielder => "selection_role_defensive_midfielder",
            SelectionRole::CentralMidfielder => "selection_role_central_midfielder",
            SelectionRole::AttackingMidfielder => "selection_role_attacking_midfielder",
            SelectionRole::Winger => "selection_role_winger",
            SelectionRole::Striker => "selection_role_striker",
            SelectionRole::Other => "selection_role_other",
        }
    }
}

/// Single-component breakdown atom from the scoring engine. The
/// selector picks the top few factors where the selected player beat
/// the omitted player and packs them into `SelectionComparison` so
/// the renderer doesn't have to expose raw f32 scores.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SelectionScoreFactor {
    PositionFit,
    PerceivedQuality,
    MatchReadiness,
    Fatigue,
    TacticalFit,
    SideFootFit,
    Reputation,
    CoachRelationship,
    Newcomer,
    YouthPreference,
    TrainingImpression,
    Cohesion,
    SquadStatus,
    ForceSelection,
    ClubPhilosophy,
    InjuryRisk,
    DevelopmentMinutes,
}

impl SelectionScoreFactor {
    pub fn as_i18n_key(&self) -> &'static str {
        match self {
            SelectionScoreFactor::PositionFit => "selection_factor_position_fit",
            SelectionScoreFactor::PerceivedQuality => "selection_factor_perceived_quality",
            SelectionScoreFactor::MatchReadiness => "selection_factor_match_readiness",
            SelectionScoreFactor::Fatigue => "selection_factor_fatigue",
            SelectionScoreFactor::TacticalFit => "selection_factor_tactical_fit",
            SelectionScoreFactor::SideFootFit => "selection_factor_side_foot_fit",
            SelectionScoreFactor::Reputation => "selection_factor_reputation",
            SelectionScoreFactor::CoachRelationship => "selection_factor_coach_relationship",
            SelectionScoreFactor::Newcomer => "selection_factor_newcomer",
            SelectionScoreFactor::YouthPreference => "selection_factor_youth_preference",
            SelectionScoreFactor::TrainingImpression => "selection_factor_training_impression",
            SelectionScoreFactor::Cohesion => "selection_factor_cohesion",
            SelectionScoreFactor::SquadStatus => "selection_factor_squad_status",
            SelectionScoreFactor::ForceSelection => "selection_factor_force_selection",
            SelectionScoreFactor::ClubPhilosophy => "selection_factor_club_philosophy",
            SelectionScoreFactor::InjuryRisk => "selection_factor_injury_risk",
            SelectionScoreFactor::DevelopmentMinutes => "selection_factor_development_minutes",
        }
    }
}

/// Render-safe mirror of `crate::ChangeType`. Stored on
/// `HappinessEventContext` so the renderer can branch on the underlying
/// relationship-change driver without importing the relations module.
/// Closed enum — one variant per ChangeType the events pipeline cares
/// about; the catch-all `Other` keeps adding new ChangeType variants
/// from being a breaking change here.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HappinessEventChangeKind {
    MatchCooperation,
    TrainingBonding,
    ConflictResolution,
    PersonalSupport,
    CoachingSuccess,
    TeamSuccess,
    MentorshipBond,
    CompetitionRivalry,
    TrainingFriction,
    PersonalConflict,
    TacticalDisagreement,
    DisciplinaryAction,
    TeamFailure,
    ReputationAdmiration,
    ReputationTension,
    NaturalProgression,
    Other,
}

impl HappinessEventChangeKind {
    pub fn as_i18n_key(&self) -> &'static str {
        match self {
            HappinessEventChangeKind::MatchCooperation => "change_kind_match_cooperation",
            HappinessEventChangeKind::TrainingBonding => "change_kind_training_bonding",
            HappinessEventChangeKind::ConflictResolution => "change_kind_conflict_resolution",
            HappinessEventChangeKind::PersonalSupport => "change_kind_personal_support",
            HappinessEventChangeKind::CoachingSuccess => "change_kind_coaching_success",
            HappinessEventChangeKind::TeamSuccess => "change_kind_team_success",
            HappinessEventChangeKind::MentorshipBond => "change_kind_mentorship_bond",
            HappinessEventChangeKind::CompetitionRivalry => "change_kind_competition_rivalry",
            HappinessEventChangeKind::TrainingFriction => "change_kind_training_friction",
            HappinessEventChangeKind::PersonalConflict => "change_kind_personal_conflict",
            HappinessEventChangeKind::TacticalDisagreement => "change_kind_tactical_disagreement",
            HappinessEventChangeKind::DisciplinaryAction => "change_kind_disciplinary_action",
            HappinessEventChangeKind::TeamFailure => "change_kind_team_failure",
            HappinessEventChangeKind::ReputationAdmiration => "change_kind_reputation_admiration",
            HappinessEventChangeKind::ReputationTension => "change_kind_reputation_tension",
            HappinessEventChangeKind::NaturalProgression => "change_kind_natural_progression",
            HappinessEventChangeKind::Other => "change_kind_other",
        }
    }

    /// Render-safe mirror of the relations crate's `ChangeType`.
    /// Total mapping — `Other` keeps adding a new ChangeType variant
    /// from being a breaking change in the events crate.
    pub fn from_change_type(change_type: &crate::ChangeType) -> Self {
        use crate::ChangeType as C;
        match change_type {
            C::MatchCooperation => HappinessEventChangeKind::MatchCooperation,
            C::TrainingBonding => HappinessEventChangeKind::TrainingBonding,
            C::ConflictResolution => HappinessEventChangeKind::ConflictResolution,
            C::PersonalSupport => HappinessEventChangeKind::PersonalSupport,
            C::CoachingSuccess => HappinessEventChangeKind::CoachingSuccess,
            C::TeamSuccess => HappinessEventChangeKind::TeamSuccess,
            C::MentorshipBond => HappinessEventChangeKind::MentorshipBond,
            C::CompetitionRivalry => HappinessEventChangeKind::CompetitionRivalry,
            C::TrainingFriction => HappinessEventChangeKind::TrainingFriction,
            C::PersonalConflict => HappinessEventChangeKind::PersonalConflict,
            C::TacticalDisagreement => HappinessEventChangeKind::TacticalDisagreement,
            C::DisciplinaryAction => HappinessEventChangeKind::DisciplinaryAction,
            C::TeamFailure => HappinessEventChangeKind::TeamFailure,
            C::ReputationAdmiration => HappinessEventChangeKind::ReputationAdmiration,
            C::ReputationTension => HappinessEventChangeKind::ReputationTension,
            C::NaturalProgression => HappinessEventChangeKind::NaturalProgression,
        }
    }
}

/// Closed set of evidence atoms. Each variant is a concrete, football-
/// realistic reason an emit site observed and decided was worth carrying
/// to the renderer (e.g. "low trust between this pair", "still a new
/// signing"). The renderer picks at most one or two of these per event
/// — they're inputs to the explanation, not a checklist to dump.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HappinessEventEvidence {
    /// Both players had a strong existing bond before the incident.
    StrongExistingBond,
    /// The relationship was already strained / hostile.
    AlreadyStrainedRelationship,
    /// Bond was weak / neutral — not enough cushion for a small incident.
    WeakExistingBond,
    /// Pair compete for the same shirt / position.
    SamePositionCompetition,
    /// Same squad-status tier (KeyPlayer/FirstTeamRegular) competition.
    SimilarSquadStatusCompetition,
    /// Trust axis was low between the pair.
    LowTrust,
    /// Friendship axis was low.
    LowFriendship,
    /// Professional respect was low.
    LowProfessionalRespect,
    /// Professional respect was high — softens the fallout.
    HighProfessionalRespect,
    /// Player has high ambition; likely to read the incident as a slight.
    HighAmbition,
    /// Offender's temperament was low (volatile).
    LowTemperament,
    /// Offender's controversy was high.
    HighControversy,
    /// Offender's sportsmanship was low.
    LowSportsmanship,
    /// Offender's professionalism was high — usually walks back the fallout.
    HighProfessionalism,
    /// Player joined recently — still inside the settling-in window.
    NewSigningStillSettling,
    /// Foreign player without local-language fluency.
    LanguageBarrier,
    /// Same-nationality teammate alleviated integration friction.
    SharedNationality,
    /// A senior / mentor figure helped or was at the centre of the incident.
    MentorInfluence,
    /// Recent match cooperation between the pair.
    MatchCooperation,
    /// Roles complement each other on the pitch.
    ComplementaryRoles,
    /// Training-ground standards were the trigger.
    TrainingStandardsMismatch,
    /// Pair has had repeat incidents recently.
    RepeatedIncident,
    /// Wage gap between the pair was material.
    WageGap,
    /// Reputation gap — star vs role player tension.
    ReputationGap,
    /// Player has formed no inner-circle bond at the club yet.
    NoInnerCircleYet,
    /// Recent squad turnover left the player without consistent peers.
    SquadTurnover,
    /// Public-facing incident: media noise rather than dressing-room.
    MediaIncident,
    /// Dressing-room row, not a media or training-ground incident.
    DressingRoomRow,
    /// Training-ground confrontation specifically.
    TrainingGroundIncident,
    // ── Support / talk / supporter evidence ─────────────────────
    /// Player produced an excellent post-match rating (≥7.5).
    ExcellentPerformance,
    /// Player was named Player of the Match.
    PlayerOfTheMatch,
    /// Player scored or assisted in this match.
    GoalContribution,
    /// Contribution arrived in a tight, decisive moment (1-goal margin
    /// win, late winner, hat-trick scale moment).
    DecisiveContribution,
    /// Performance came in a derby fixture.
    DerbyPerformance,
    /// Performance came in a cup tie.
    CupPerformance,
    /// Reaction came from the home crowd specifically.
    HomeCrowdMoment,
    /// Player's morale was poor going into the talk — message was about
    /// confidence as much as performance.
    PoorMoraleBeforeTalk,
    /// Player's confidence was visibly low coming in.
    LowConfidence,
    /// Existing manager trust amplified the message.
    ManagerTrust,
    /// Strong rapport with the talking coach lifted reception.
    StrongCoachRapport,
    /// Weak rapport with the talking coach blunted or backfired the
    /// message.
    WeakCoachRapport,
    /// Player has a high-pressure personality (≥15) — handles big
    /// occasions well.
    HighPressurePersonality,
    /// Player has a low-pressure personality (≤7) — shrinks under big
    /// occasions.
    LowPressurePersonality,
    /// Player has high determination (≥15) — converts criticism into
    /// motivation.
    HighDetermination,
    /// Player handles big matches well (`important_matches` ≥ 15).
    ImportantMatchTemperament,
    /// Manager has used the same tone repeatedly — message is dampened.
    RepeatedTalkDampened,
    /// A captain or senior leader figure was at the centre of the moment.
    CaptainOrLeaderInfluence,
    /// Young player needing a confidence boost.
    YoungPlayerNeedingConfidence,
    /// Returning from injury — the gesture lands harder.
    ReturnFromInjuryBoost,
}

impl HappinessEventEvidence {
    pub fn as_i18n_key(&self) -> &'static str {
        match self {
            HappinessEventEvidence::StrongExistingBond => "evidence_strong_existing_bond",
            HappinessEventEvidence::AlreadyStrainedRelationship => {
                "evidence_already_strained_relationship"
            }
            HappinessEventEvidence::WeakExistingBond => "evidence_weak_existing_bond",
            HappinessEventEvidence::SamePositionCompetition => {
                "evidence_same_position_competition"
            }
            HappinessEventEvidence::SimilarSquadStatusCompetition => {
                "evidence_similar_squad_status_competition"
            }
            HappinessEventEvidence::LowTrust => "evidence_low_trust",
            HappinessEventEvidence::LowFriendship => "evidence_low_friendship",
            HappinessEventEvidence::LowProfessionalRespect => "evidence_low_professional_respect",
            HappinessEventEvidence::HighProfessionalRespect => {
                "evidence_high_professional_respect"
            }
            HappinessEventEvidence::HighAmbition => "evidence_high_ambition",
            HappinessEventEvidence::LowTemperament => "evidence_low_temperament",
            HappinessEventEvidence::HighControversy => "evidence_high_controversy",
            HappinessEventEvidence::LowSportsmanship => "evidence_low_sportsmanship",
            HappinessEventEvidence::HighProfessionalism => "evidence_high_professionalism",
            HappinessEventEvidence::NewSigningStillSettling => {
                "evidence_new_signing_still_settling"
            }
            HappinessEventEvidence::LanguageBarrier => "evidence_language_barrier",
            HappinessEventEvidence::SharedNationality => "evidence_shared_nationality",
            HappinessEventEvidence::MentorInfluence => "evidence_mentor_influence",
            HappinessEventEvidence::MatchCooperation => "evidence_match_cooperation",
            HappinessEventEvidence::ComplementaryRoles => "evidence_complementary_roles",
            HappinessEventEvidence::TrainingStandardsMismatch => {
                "evidence_training_standards_mismatch"
            }
            HappinessEventEvidence::RepeatedIncident => "evidence_repeated_incident",
            HappinessEventEvidence::WageGap => "evidence_wage_gap",
            HappinessEventEvidence::ReputationGap => "evidence_reputation_gap",
            HappinessEventEvidence::NoInnerCircleYet => "evidence_no_inner_circle_yet",
            HappinessEventEvidence::SquadTurnover => "evidence_squad_turnover",
            HappinessEventEvidence::MediaIncident => "evidence_media_incident",
            HappinessEventEvidence::DressingRoomRow => "evidence_dressing_room_row",
            HappinessEventEvidence::TrainingGroundIncident => "evidence_training_ground_incident",
            HappinessEventEvidence::ExcellentPerformance => "evidence_excellent_performance",
            HappinessEventEvidence::PlayerOfTheMatch => "evidence_player_of_the_match",
            HappinessEventEvidence::GoalContribution => "evidence_goal_contribution",
            HappinessEventEvidence::DecisiveContribution => "evidence_decisive_contribution",
            HappinessEventEvidence::DerbyPerformance => "evidence_derby_performance",
            HappinessEventEvidence::CupPerformance => "evidence_cup_performance",
            HappinessEventEvidence::HomeCrowdMoment => "evidence_home_crowd_moment",
            HappinessEventEvidence::PoorMoraleBeforeTalk => "evidence_poor_morale_before_talk",
            HappinessEventEvidence::LowConfidence => "evidence_low_confidence",
            HappinessEventEvidence::ManagerTrust => "evidence_manager_trust",
            HappinessEventEvidence::StrongCoachRapport => "evidence_strong_coach_rapport",
            HappinessEventEvidence::WeakCoachRapport => "evidence_weak_coach_rapport",
            HappinessEventEvidence::HighPressurePersonality => "evidence_high_pressure_personality",
            HappinessEventEvidence::LowPressurePersonality => "evidence_low_pressure_personality",
            HappinessEventEvidence::HighDetermination => "evidence_high_determination",
            HappinessEventEvidence::ImportantMatchTemperament => {
                "evidence_important_match_temperament"
            }
            HappinessEventEvidence::RepeatedTalkDampened => "evidence_repeated_talk_dampened",
            HappinessEventEvidence::CaptainOrLeaderInfluence => {
                "evidence_captain_or_leader_influence"
            }
            HappinessEventEvidence::YoungPlayerNeedingConfidence => {
                "evidence_young_player_needing_confidence"
            }
            HappinessEventEvidence::ReturnFromInjuryBoost => "evidence_return_from_injury_boost",
        }
    }
}


/// Closed set of "what's next" hints. Renderer maps each to a localised
/// sentence; storing the variant (rather than free text) keeps the UI
/// stable across re-renders and translatable.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HappinessEventFollowUp {
    /// Likely to settle unless repeated within the next few weeks.
    LikelyToSettle,
    /// Repeated incidents may damage dressing-room status.
    DressingRoomDamageRisk,
    /// Contract request risk increased.
    ContractRequestRisk,
    /// Manager intervention may be required if it persists.
    ManagerInterventionRisk,
    /// Trend is improving — relationship should keep strengthening.
    TrendImproving,
    /// Settling-in period is almost over.
    SettlingInProgress,
    /// Manager trust should improve if this form continues.
    ManagerTrustRising,
    /// Standing with the supporters should improve if this form
    /// continues.
    FanStandingRising,
    /// Pressure / expectations may build after a stand-out moment.
    PressureBuilding,
}

impl HappinessEventFollowUp {
    pub fn as_i18n_key(&self) -> &'static str {
        match self {
            HappinessEventFollowUp::LikelyToSettle => "follow_up_likely_to_settle",
            HappinessEventFollowUp::DressingRoomDamageRisk => "follow_up_dressing_room_damage",
            HappinessEventFollowUp::ContractRequestRisk => "follow_up_contract_request_risk",
            HappinessEventFollowUp::ManagerInterventionRisk => "follow_up_manager_intervention",
            HappinessEventFollowUp::TrendImproving => "follow_up_trend_improving",
            HappinessEventFollowUp::SettlingInProgress => "follow_up_settling_in",
            HappinessEventFollowUp::ManagerTrustRising => "follow_up_manager_trust_rising",
            HappinessEventFollowUp::FanStandingRising => "follow_up_fan_standing_rising",
            HappinessEventFollowUp::PressureBuilding => "follow_up_pressure_building",
        }
    }
}

/// Stage in the transfer-interest funnel — where the rumour or approach
/// stands at the moment the event was emitted. Tracks how serious the
/// interest is, from a single scout sighting to a formal bid being
/// negotiated.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransferInterestStage {
    ScoutWatched,
    Shortlisted,
    AgentSoundingOut,
    LooseRumour,
    ConcreteInterest,
    BidExpected,
    BidSubmitted,
    BidRejected,
    NegotiationsOpened,
    NegotiationsStalled,
    MoveCollapsed,
    InterestCooled,
}

impl TransferInterestStage {
    pub fn as_i18n_key(&self) -> &'static str {
        match self {
            TransferInterestStage::ScoutWatched => "transfer_interest_stage_scout_watched",
            TransferInterestStage::Shortlisted => "transfer_interest_stage_shortlisted",
            TransferInterestStage::AgentSoundingOut => {
                "transfer_interest_stage_agent_sounding_out"
            }
            TransferInterestStage::LooseRumour => "transfer_interest_stage_loose_rumour",
            TransferInterestStage::ConcreteInterest => {
                "transfer_interest_stage_concrete_interest"
            }
            TransferInterestStage::BidExpected => "transfer_interest_stage_bid_expected",
            TransferInterestStage::BidSubmitted => "transfer_interest_stage_bid_submitted",
            TransferInterestStage::BidRejected => "transfer_interest_stage_bid_rejected",
            TransferInterestStage::NegotiationsOpened => {
                "transfer_interest_stage_negotiations_opened"
            }
            TransferInterestStage::NegotiationsStalled => {
                "transfer_interest_stage_negotiations_stalled"
            }
            TransferInterestStage::MoveCollapsed => "transfer_interest_stage_move_collapsed",
            TransferInterestStage::InterestCooled => "transfer_interest_stage_interest_cooled",
        }
    }
}

/// Where the rumour came from. Drives the "how the player heard about it"
/// line — a scout sighting reads differently from an agent leak or a
/// confirmed approach.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransferInterestSource {
    ScoutAttendance,
    AgentLeak,
    LocalPress,
    NationalPress,
    ClubBriefing,
    FanSpeculation,
    ConfirmedApproach,
    InternalRecruitmentMeeting,
    RejectedBid,
    ContractTalk,
}

impl TransferInterestSource {
    pub fn as_i18n_key(&self) -> &'static str {
        match self {
            TransferInterestSource::ScoutAttendance => "transfer_interest_source_scout_attendance",
            TransferInterestSource::AgentLeak => "transfer_interest_source_agent_leak",
            TransferInterestSource::LocalPress => "transfer_interest_source_local_press",
            TransferInterestSource::NationalPress => "transfer_interest_source_national_press",
            TransferInterestSource::ClubBriefing => "transfer_interest_source_club_briefing",
            TransferInterestSource::FanSpeculation => "transfer_interest_source_fan_speculation",
            TransferInterestSource::ConfirmedApproach => {
                "transfer_interest_source_confirmed_approach"
            }
            TransferInterestSource::InternalRecruitmentMeeting => {
                "transfer_interest_source_internal_recruitment_meeting"
            }
            TransferInterestSource::RejectedBid => "transfer_interest_source_rejected_bid",
            TransferInterestSource::ContractTalk => "transfer_interest_source_contract_talk",
        }
    }
}

/// The football meaning of the move from this player's perspective.
/// Drives whether the rumour reads as a step up, a return home, an
/// escape route, or just speculation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransferInterestKind {
    StepUp,
    LateralMove,
    StepDownWithMinutes,
    Homecoming,
    RivalMove,
    FormerClubReturn,
    FavoriteClubInterest,
    BigLeagueOpportunity,
    LoanDevelopment,
    EscapeRoute,
    Speculative,
}

impl TransferInterestKind {
    pub fn as_i18n_key(&self) -> &'static str {
        match self {
            TransferInterestKind::StepUp => "transfer_interest_kind_step_up",
            TransferInterestKind::LateralMove => "transfer_interest_kind_lateral_move",
            TransferInterestKind::StepDownWithMinutes => {
                "transfer_interest_kind_step_down_with_minutes"
            }
            TransferInterestKind::Homecoming => "transfer_interest_kind_homecoming",
            TransferInterestKind::RivalMove => "transfer_interest_kind_rival_move",
            TransferInterestKind::FormerClubReturn => {
                "transfer_interest_kind_former_club_return"
            }
            TransferInterestKind::FavoriteClubInterest => {
                "transfer_interest_kind_favorite_club_interest"
            }
            TransferInterestKind::BigLeagueOpportunity => {
                "transfer_interest_kind_big_league_opportunity"
            }
            TransferInterestKind::LoanDevelopment => "transfer_interest_kind_loan_development",
            TransferInterestKind::EscapeRoute => "transfer_interest_kind_escape_route",
            TransferInterestKind::Speculative => "transfer_interest_kind_speculative",
        }
    }
}

/// How the player reacted privately to the rumour or approach. Tied to
/// personality + context — the same rumour produces different reactions
/// for an ambitious star vs a loyal squad regular.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransferInterestReaction {
    Flattered,
    Focused,
    Distracted,
    Unsettled,
    Excited,
    Cautious,
    AnnoyedBySpeculation,
    LoyalToCurrentClub,
    WantsTalks,
    WantsBidAccepted,
    FearsBeingPushedOut,
    UsesInterestForContractLeverage,
    PubliclyCalmPrivatelyInterested,
}

impl TransferInterestReaction {
    pub fn as_i18n_key(&self) -> &'static str {
        match self {
            TransferInterestReaction::Flattered => "transfer_interest_reaction_flattered",
            TransferInterestReaction::Focused => "transfer_interest_reaction_focused",
            TransferInterestReaction::Distracted => "transfer_interest_reaction_distracted",
            TransferInterestReaction::Unsettled => "transfer_interest_reaction_unsettled",
            TransferInterestReaction::Excited => "transfer_interest_reaction_excited",
            TransferInterestReaction::Cautious => "transfer_interest_reaction_cautious",
            TransferInterestReaction::AnnoyedBySpeculation => {
                "transfer_interest_reaction_annoyed_by_speculation"
            }
            TransferInterestReaction::LoyalToCurrentClub => {
                "transfer_interest_reaction_loyal_to_current_club"
            }
            TransferInterestReaction::WantsTalks => "transfer_interest_reaction_wants_talks",
            TransferInterestReaction::WantsBidAccepted => {
                "transfer_interest_reaction_wants_bid_accepted"
            }
            TransferInterestReaction::FearsBeingPushedOut => {
                "transfer_interest_reaction_fears_being_pushed_out"
            }
            TransferInterestReaction::UsesInterestForContractLeverage => {
                "transfer_interest_reaction_uses_interest_for_contract_leverage"
            }
            TransferInterestReaction::PubliclyCalmPrivatelyInterested => {
                "transfer_interest_reaction_publicly_calm_privately_interested"
            }
        }
    }
}

/// Sporting fit category — how the move would play out on the pitch
/// rather than as a headline. A "bigger club but harder minutes" link
/// produces a meaningfully different reaction from a "better playing
/// time" link.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransferSportingFit {
    ClearUpgrade,
    BiggerClubButHarderMinutes,
    BetterPlayingTime,
    PoorRoleFit,
    TacticalFit,
    BadLeagueFit,
    EmotionalFit,
    FinancialFitOnly,
}

impl TransferSportingFit {
    pub fn as_i18n_key(&self) -> &'static str {
        match self {
            TransferSportingFit::ClearUpgrade => "transfer_sporting_fit_clear_upgrade",
            TransferSportingFit::BiggerClubButHarderMinutes => {
                "transfer_sporting_fit_bigger_club_but_harder_minutes"
            }
            TransferSportingFit::BetterPlayingTime => {
                "transfer_sporting_fit_better_playing_time"
            }
            TransferSportingFit::PoorRoleFit => "transfer_sporting_fit_poor_role_fit",
            TransferSportingFit::TacticalFit => "transfer_sporting_fit_tactical_fit",
            TransferSportingFit::BadLeagueFit => "transfer_sporting_fit_bad_league_fit",
            TransferSportingFit::EmotionalFit => "transfer_sporting_fit_emotional_fit",
            TransferSportingFit::FinancialFitOnly => "transfer_sporting_fit_financial_fit_only",
        }
    }
}

/// Concrete football evidence behind the player's reaction. Closed set;
/// the renderer picks the most informative atom to surface as a
/// supporting sentence next to the main reason.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransferInterestEvidence {
    BiggerClub,
    BiggerLeague,
    ChampionsLeagueOpportunity,
    MoreLikelyStarts,
    LessLikelyStarts,
    CurrentPlayingTimeFrustration,
    CurrentClubAmbitionMismatch,
    CurrentClubLoyalty,
    HighAmbition,
    LowAmbition,
    HighLoyalty,
    LowLoyalty,
    HighProfessionalism,
    HighControversy,
    AgentPushing,
    FanPressure,
    MediaNoise,
    ScoutAtMatch,
    RepeatedRumours,
    RejectedBid,
    RivalClub,
    FormerClub,
    FavoriteClub,
    HomeCountry,
    LanguageCultureFit,
    ContractExpiring,
    Underpaid,
    ManagerPromiseConflict,
    RecentNewSigningThreatensRole,
}

impl TransferInterestEvidence {
    pub fn as_i18n_key(&self) -> &'static str {
        match self {
            TransferInterestEvidence::BiggerClub => "transfer_interest_evidence_bigger_club",
            TransferInterestEvidence::BiggerLeague => "transfer_interest_evidence_bigger_league",
            TransferInterestEvidence::ChampionsLeagueOpportunity => {
                "transfer_interest_evidence_champions_league_opportunity"
            }
            TransferInterestEvidence::MoreLikelyStarts => {
                "transfer_interest_evidence_more_likely_starts"
            }
            TransferInterestEvidence::LessLikelyStarts => {
                "transfer_interest_evidence_less_likely_starts"
            }
            TransferInterestEvidence::CurrentPlayingTimeFrustration => {
                "transfer_interest_evidence_current_playing_time_frustration"
            }
            TransferInterestEvidence::CurrentClubAmbitionMismatch => {
                "transfer_interest_evidence_current_club_ambition_mismatch"
            }
            TransferInterestEvidence::CurrentClubLoyalty => {
                "transfer_interest_evidence_current_club_loyalty"
            }
            TransferInterestEvidence::HighAmbition => "transfer_interest_evidence_high_ambition",
            TransferInterestEvidence::LowAmbition => "transfer_interest_evidence_low_ambition",
            TransferInterestEvidence::HighLoyalty => "transfer_interest_evidence_high_loyalty",
            TransferInterestEvidence::LowLoyalty => "transfer_interest_evidence_low_loyalty",
            TransferInterestEvidence::HighProfessionalism => {
                "transfer_interest_evidence_high_professionalism"
            }
            TransferInterestEvidence::HighControversy => {
                "transfer_interest_evidence_high_controversy"
            }
            TransferInterestEvidence::AgentPushing => "transfer_interest_evidence_agent_pushing",
            TransferInterestEvidence::FanPressure => "transfer_interest_evidence_fan_pressure",
            TransferInterestEvidence::MediaNoise => "transfer_interest_evidence_media_noise",
            TransferInterestEvidence::ScoutAtMatch => "transfer_interest_evidence_scout_at_match",
            TransferInterestEvidence::RepeatedRumours => {
                "transfer_interest_evidence_repeated_rumours"
            }
            TransferInterestEvidence::RejectedBid => "transfer_interest_evidence_rejected_bid",
            TransferInterestEvidence::RivalClub => "transfer_interest_evidence_rival_club",
            TransferInterestEvidence::FormerClub => "transfer_interest_evidence_former_club",
            TransferInterestEvidence::FavoriteClub => "transfer_interest_evidence_favorite_club",
            TransferInterestEvidence::HomeCountry => "transfer_interest_evidence_home_country",
            TransferInterestEvidence::LanguageCultureFit => {
                "transfer_interest_evidence_language_culture_fit"
            }
            TransferInterestEvidence::ContractExpiring => {
                "transfer_interest_evidence_contract_expiring"
            }
            TransferInterestEvidence::Underpaid => "transfer_interest_evidence_underpaid",
            TransferInterestEvidence::ManagerPromiseConflict => {
                "transfer_interest_evidence_manager_promise_conflict"
            }
            TransferInterestEvidence::RecentNewSigningThreatensRole => {
                "transfer_interest_evidence_recent_new_signing_threatens_role"
            }
        }
    }
}

/// Structured payload describing a transfer-interest moment. Filled in
/// at emit time so the renderer can compose a contextual headline +
/// reason + reaction + outlook instead of falling back to a generic
/// "wanted by another club" line.
///
/// Most fields are optional — emit sites populate only what they know.
/// The `interest_stage`, `interest_source`, `interest_kind`, and
/// `player_reaction` axes are required: a transfer-interest event
/// without any of those four would not communicate anything useful.
#[derive(Debug, Clone)]
pub struct TransferInterestContext {
    pub interested_club_id: Option<u32>,
    pub interested_league_id: Option<u32>,
    pub interest_stage: TransferInterestStage,
    pub interest_source: TransferInterestSource,
    pub interest_kind: TransferInterestKind,
    pub player_reaction: TransferInterestReaction,
    pub sporting_fit: Option<TransferSportingFit>,
    pub reputation_gap: i32,
    pub league_reputation_gap: i32,
    pub likely_role: Option<PlayerSquadStatus>,
    pub current_squad_status: Option<PlayerSquadStatus>,
    pub is_rival: bool,
    pub is_former_club: bool,
    pub is_home_country: bool,
    pub is_favorite_club: bool,
    pub would_improve_playing_time: bool,
    pub would_reduce_playing_time: bool,
    pub wage_upside_ratio: Option<f32>,
    pub agent_pressure: Option<f32>,
    pub media_heat: Option<f32>,
    pub evidence: Vec<TransferInterestEvidence>,
}

impl TransferInterestContext {
    pub fn new(
        interest_stage: TransferInterestStage,
        interest_source: TransferInterestSource,
        interest_kind: TransferInterestKind,
        player_reaction: TransferInterestReaction,
    ) -> Self {
        Self {
            interested_club_id: None,
            interested_league_id: None,
            interest_stage,
            interest_source,
            interest_kind,
            player_reaction,
            sporting_fit: None,
            reputation_gap: 0,
            league_reputation_gap: 0,
            likely_role: None,
            current_squad_status: None,
            is_rival: false,
            is_former_club: false,
            is_home_country: false,
            is_favorite_club: false,
            would_improve_playing_time: false,
            would_reduce_playing_time: false,
            wage_upside_ratio: None,
            agent_pressure: None,
            media_heat: None,
            evidence: Vec::new(),
        }
    }

    pub fn with_interested_club(mut self, club_id: u32) -> Self {
        self.interested_club_id = Some(club_id);
        self
    }

    pub fn with_interested_league(mut self, league_id: u32) -> Self {
        self.interested_league_id = Some(league_id);
        self
    }

    pub fn with_sporting_fit(mut self, fit: TransferSportingFit) -> Self {
        self.sporting_fit = Some(fit);
        self
    }

    pub fn with_reputation_gap(mut self, gap: i32) -> Self {
        self.reputation_gap = gap;
        self
    }

    pub fn with_league_reputation_gap(mut self, gap: i32) -> Self {
        self.league_reputation_gap = gap;
        self
    }

    pub fn with_likely_role(mut self, role: PlayerSquadStatus) -> Self {
        self.likely_role = Some(role);
        self
    }

    pub fn with_current_squad_status(mut self, status: PlayerSquadStatus) -> Self {
        self.current_squad_status = Some(status);
        self
    }

    pub fn with_rival(mut self, is_rival: bool) -> Self {
        self.is_rival = is_rival;
        self
    }

    pub fn with_former_club(mut self, is_former: bool) -> Self {
        self.is_former_club = is_former;
        self
    }

    pub fn with_home_country(mut self, is_home: bool) -> Self {
        self.is_home_country = is_home;
        self
    }

    pub fn with_favorite_club(mut self, is_favorite: bool) -> Self {
        self.is_favorite_club = is_favorite;
        self
    }

    pub fn with_playing_time_change(
        mut self,
        improve: bool,
        reduce: bool,
    ) -> Self {
        self.would_improve_playing_time = improve;
        self.would_reduce_playing_time = reduce;
        self
    }

    pub fn with_wage_upside(mut self, ratio: f32) -> Self {
        self.wage_upside_ratio = Some(ratio);
        self
    }

    pub fn with_agent_pressure(mut self, pressure: f32) -> Self {
        self.agent_pressure = Some(pressure);
        self
    }

    pub fn with_media_heat(mut self, heat: f32) -> Self {
        self.media_heat = Some(heat);
        self
    }

    pub fn with_evidence(mut self, evidence: TransferInterestEvidence) -> Self {
        if !self.evidence.contains(&evidence) {
            self.evidence.push(evidence);
        }
        self
    }

    pub fn with_evidence_iter<I>(mut self, iter: I) -> Self
    where
        I: IntoIterator<Item = TransferInterestEvidence>,
    {
        for ev in iter {
            if !self.evidence.contains(&ev) {
                self.evidence.push(ev);
            }
        }
        self
    }
}

// ─────────────────────────────────────────────────────────────────
// Training-event context
// ─────────────────────────────────────────────────────────────────

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
            TrainingEventReason::SharpAfterBeingLeftOut => "training_reason_sharp_after_being_left_out",
            TrainingEventReason::RespondedToCriticism => "training_reason_responded_to_criticism",
            TrainingEventReason::StruggledWithIntensity => "training_reason_struggled_with_intensity",
            TrainingEventReason::DistractedByRumours => "training_reason_distracted_by_rumours",
            TrainingEventReason::PoorAttitude => "training_reason_poor_attitude",
            TrainingEventReason::ReturningFromInjuryNotSharp => "training_reason_returning_from_injury_not_sharp",
            TrainingEventReason::YoungImpressedStaff => "training_reason_young_impressed_staff",
            TrainingEventReason::SettingStandards => "training_reason_setting_standards",
            TrainingEventReason::ExtraWorkAfterSession => "training_reason_extra_work_after_session",
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
}

impl TrainingEventEvidence {
    pub fn as_i18n_key(&self) -> &'static str {
        match self {
            TrainingEventEvidence::HighSessionPerformance => "training_evidence_high_session_performance",
            TrainingEventEvidence::LowSessionPerformance => "training_evidence_low_session_performance",
            TrainingEventEvidence::HighWorkload => "training_evidence_high_workload",
            TrainingEventEvidence::LowCondition => "training_evidence_low_condition",
            TrainingEventEvidence::RecentlyDropped => "training_evidence_recently_dropped",
            TrainingEventEvidence::TransferSpeculation => "training_evidence_transfer_speculation",
            TrainingEventEvidence::InRecoveryPhase => "training_evidence_in_recovery_phase",
            TrainingEventEvidence::HighProfessionalism => "training_evidence_high_professionalism",
            TrainingEventEvidence::LowProfessionalism => "training_evidence_low_professionalism",
            TrainingEventEvidence::YouthDevelopmentTier => "training_evidence_youth_development_tier",
            TrainingEventEvidence::VeteranLeader => "training_evidence_veteran_leader",
            TrainingEventEvidence::StrongRecentForm => "training_evidence_strong_recent_form",
            TrainingEventEvidence::UpcomingBigMatch => "training_evidence_upcoming_big_match",
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
    pub fn new(reason: TrainingEventReason, session_performance: f32, training_performance_ema: f32) -> Self {
        Self { reason, session_performance, training_performance_ema, evidence: Vec::new() }
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

// ─────────────────────────────────────────────────────────────────
// Manager-interaction context
// ─────────────────────────────────────────────────────────────────

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

#[derive(Debug, Clone)]
pub struct ManagerInteractionEventContext {
    pub topic: ManagerInteractionTopic,
    pub tone: ManagerInteractionTone,
    pub acceptance: PlayerAcceptance,
    pub manager_staff_id: Option<u32>,
    pub trust_in_manager: Option<f32>,
    pub promise_kind: Option<PromiseKind>,
    pub promise_credibility: Option<f32>,
}

impl ManagerInteractionEventContext {
    pub fn new(topic: ManagerInteractionTopic, tone: ManagerInteractionTone, acceptance: PlayerAcceptance) -> Self {
        Self {
            topic,
            tone,
            acceptance,
            manager_staff_id: None,
            trust_in_manager: None,
            promise_kind: None,
            promise_credibility: None,
        }
    }

    pub fn with_manager_staff_id(mut self, id: u32) -> Self { self.manager_staff_id = Some(id); self }
    pub fn with_trust(mut self, trust: f32) -> Self { self.trust_in_manager = Some(trust); self }
    pub fn with_promise(mut self, kind: PromiseKind, credibility: f32) -> Self {
        self.promise_kind = Some(kind);
        self.promise_credibility = Some(credibility);
        self
    }
}

// ─────────────────────────────────────────────────────────────────
// Contract / agent context
// ─────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContractEventKind {
    OfferReceived,
    TalksOpened,
    TalksStalled,
    Renewed,
    Terminated,
    SalaryShock,
    SalaryBoost,
    LoyaltyDiscountAccepted,
    AgentPushingForBetterTerms,
    WagePromiseFrustration,
    AcceptedReducedRoleContract,
    RejectedLowStatusOffer,
}

impl ContractEventKind {
    pub fn as_i18n_key(&self) -> &'static str {
        match self {
            ContractEventKind::OfferReceived => "contract_kind_offer_received",
            ContractEventKind::TalksOpened => "contract_kind_talks_opened",
            ContractEventKind::TalksStalled => "contract_kind_talks_stalled",
            ContractEventKind::Renewed => "contract_kind_renewed",
            ContractEventKind::Terminated => "contract_kind_terminated",
            ContractEventKind::SalaryShock => "contract_kind_salary_shock",
            ContractEventKind::SalaryBoost => "contract_kind_salary_boost",
            ContractEventKind::LoyaltyDiscountAccepted => "contract_kind_loyalty_discount_accepted",
            ContractEventKind::AgentPushingForBetterTerms => "contract_kind_agent_pushing",
            ContractEventKind::WagePromiseFrustration => "contract_kind_wage_promise_frustration",
            ContractEventKind::AcceptedReducedRoleContract => "contract_kind_accepted_reduced_role",
            ContractEventKind::RejectedLowStatusOffer => "contract_kind_rejected_low_status",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContractEventEvidence {
    AgentPressure,
    HighLoyalty,
    LowLoyalty,
    HighAmbition,
    UnderpaidVsPeers,
    OverpaidVsExpectation,
    SquadStatusUpgrade,
    SquadStatusDowngrade,
    UsedExternalInterestAsLeverage,
    ContractExpiring,
    HasOtherInterest,
    ClubInFinancialDistress,
}

impl ContractEventEvidence {
    pub fn as_i18n_key(&self) -> &'static str {
        match self {
            ContractEventEvidence::AgentPressure => "contract_evidence_agent_pressure",
            ContractEventEvidence::HighLoyalty => "contract_evidence_high_loyalty",
            ContractEventEvidence::LowLoyalty => "contract_evidence_low_loyalty",
            ContractEventEvidence::HighAmbition => "contract_evidence_high_ambition",
            ContractEventEvidence::UnderpaidVsPeers => "contract_evidence_underpaid_vs_peers",
            ContractEventEvidence::OverpaidVsExpectation => "contract_evidence_overpaid_vs_expectation",
            ContractEventEvidence::SquadStatusUpgrade => "contract_evidence_squad_status_upgrade",
            ContractEventEvidence::SquadStatusDowngrade => "contract_evidence_squad_status_downgrade",
            ContractEventEvidence::UsedExternalInterestAsLeverage => "contract_evidence_used_external_interest",
            ContractEventEvidence::ContractExpiring => "contract_evidence_contract_expiring",
            ContractEventEvidence::HasOtherInterest => "contract_evidence_has_other_interest",
            ContractEventEvidence::ClubInFinancialDistress => "contract_evidence_club_financial_distress",
        }
    }
}

#[derive(Debug, Clone)]
pub struct ContractEventContext {
    pub kind: ContractEventKind,
    pub interested_club_id: Option<u32>,
    pub wage_ratio_vs_previous: Option<f32>,
    pub wage_ratio_vs_peers: Option<f32>,
    pub promised_status: Option<PlayerSquadStatus>,
    pub agent_pressure: Option<f32>,
    pub years_remaining: Option<u8>,
    pub evidence: Vec<ContractEventEvidence>,
}

impl ContractEventContext {
    pub fn new(kind: ContractEventKind) -> Self {
        Self {
            kind,
            interested_club_id: None,
            wage_ratio_vs_previous: None,
            wage_ratio_vs_peers: None,
            promised_status: None,
            agent_pressure: None,
            years_remaining: None,
            evidence: Vec::new(),
        }
    }

    pub fn with_wage_vs_previous(mut self, ratio: f32) -> Self { self.wage_ratio_vs_previous = Some(ratio); self }
    pub fn with_wage_vs_peers(mut self, ratio: f32) -> Self { self.wage_ratio_vs_peers = Some(ratio); self }
    pub fn with_promised_status(mut self, status: PlayerSquadStatus) -> Self { self.promised_status = Some(status); self }
    pub fn with_agent_pressure(mut self, pressure: f32) -> Self { self.agent_pressure = Some(pressure); self }
    pub fn with_years_remaining(mut self, years: u8) -> Self { self.years_remaining = Some(years); self }
    pub fn with_interested_club(mut self, club_id: u32) -> Self { self.interested_club_id = Some(club_id); self }

    pub fn with_evidence(mut self, evidence: ContractEventEvidence) -> Self {
        if !self.evidence.contains(&evidence) {
            self.evidence.push(evidence);
        }
        self
    }
}

// ─────────────────────────────────────────────────────────────────
// Injury / fitness recovery context
// ─────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InjuryRecoveryStage {
    ReturnedToFullTraining,
    FirstMinutesAfterInjury,
    RecoverySetback,
    ProtectedByMedicalStaff,
    InjuryRecurrenceConcern,
    FitnessConfidenceRestored,
}

impl InjuryRecoveryStage {
    pub fn as_i18n_key(&self) -> &'static str {
        match self {
            InjuryRecoveryStage::ReturnedToFullTraining => "injury_stage_returned_full_training",
            InjuryRecoveryStage::FirstMinutesAfterInjury => "injury_stage_first_minutes",
            InjuryRecoveryStage::RecoverySetback => "injury_stage_recovery_setback",
            InjuryRecoveryStage::ProtectedByMedicalStaff => "injury_stage_protected",
            InjuryRecoveryStage::InjuryRecurrenceConcern => "injury_stage_recurrence_concern",
            InjuryRecoveryStage::FitnessConfidenceRestored => "injury_stage_confidence_restored",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InjuryRecoveryEvidence {
    LongTermLayoff,
    ShortTermLayoff,
    MatchSharpnessLow,
    MatchSharpnessRecovering,
    MultipleInjuriesThisSeason,
    PriorRecurringIssue,
    HighProfessionalism,
    FearLosingPlace,
}

impl InjuryRecoveryEvidence {
    pub fn as_i18n_key(&self) -> &'static str {
        match self {
            InjuryRecoveryEvidence::LongTermLayoff => "injury_evidence_long_term_layoff",
            InjuryRecoveryEvidence::ShortTermLayoff => "injury_evidence_short_term_layoff",
            InjuryRecoveryEvidence::MatchSharpnessLow => "injury_evidence_sharpness_low",
            InjuryRecoveryEvidence::MatchSharpnessRecovering => "injury_evidence_sharpness_recovering",
            InjuryRecoveryEvidence::MultipleInjuriesThisSeason => "injury_evidence_multiple_injuries_season",
            InjuryRecoveryEvidence::PriorRecurringIssue => "injury_evidence_prior_recurring_issue",
            InjuryRecoveryEvidence::HighProfessionalism => "injury_evidence_high_professionalism",
            InjuryRecoveryEvidence::FearLosingPlace => "injury_evidence_fear_losing_place",
        }
    }
}

#[derive(Debug, Clone)]
pub struct InjuryRecoveryEventContext {
    pub stage: InjuryRecoveryStage,
    pub recovery_days_total: u16,
    pub match_readiness: f32,
    pub evidence: Vec<InjuryRecoveryEvidence>,
}

impl InjuryRecoveryEventContext {
    pub fn new(stage: InjuryRecoveryStage, recovery_days_total: u16, match_readiness: f32) -> Self {
        Self { stage, recovery_days_total, match_readiness, evidence: Vec::new() }
    }

    pub fn with_evidence(mut self, evidence: InjuryRecoveryEvidence) -> Self {
        if !self.evidence.contains(&evidence) {
            self.evidence.push(evidence);
        }
        self
    }
}

// ─────────────────────────────────────────────────────────────────
// Match-performance context
// ─────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MatchPerformanceKind {
    AnsweredCriticsWithPerformance,
    CostlyErrorUnderPressure,
    SavedResultLate,
    ChangedGameFromBench,
    DefensiveLeaderPerformance,
    WastefulFinishingConcern,
    ComposurePraised,
    BigMatchNerves,
    StandoutDisplay,
    FirstClubGoalMoment,
    DroughtEnded,
    HatTrickFire,
}

impl MatchPerformanceKind {
    pub fn as_i18n_key(&self) -> &'static str {
        match self {
            MatchPerformanceKind::AnsweredCriticsWithPerformance => "match_perf_kind_answered_critics",
            MatchPerformanceKind::CostlyErrorUnderPressure => "match_perf_kind_costly_error_pressure",
            MatchPerformanceKind::SavedResultLate => "match_perf_kind_saved_result_late",
            MatchPerformanceKind::ChangedGameFromBench => "match_perf_kind_changed_game_from_bench",
            MatchPerformanceKind::DefensiveLeaderPerformance => "match_perf_kind_defensive_leader",
            MatchPerformanceKind::WastefulFinishingConcern => "match_perf_kind_wasteful_finishing",
            MatchPerformanceKind::ComposurePraised => "match_perf_kind_composure_praised",
            MatchPerformanceKind::BigMatchNerves => "match_perf_kind_big_match_nerves",
            MatchPerformanceKind::StandoutDisplay => "match_perf_kind_standout",
            MatchPerformanceKind::FirstClubGoalMoment => "match_perf_kind_first_club_goal",
            MatchPerformanceKind::DroughtEnded => "match_perf_kind_drought_ended",
            MatchPerformanceKind::HatTrickFire => "match_perf_kind_hat_trick",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MatchPerformanceEvidence {
    HighRating,
    LowRating,
    GoalContribution,
    DecisiveContribution,
    DerbyFixture,
    CupTie,
    LeagueDecider,
    SubstituteAppearance,
    PlayedFullMinutes,
    PlayedAfterCriticism,
    HighPressurePersonality,
    LowPressurePersonality,
    ImportantMatchTemperament,
}

impl MatchPerformanceEvidence {
    pub fn as_i18n_key(&self) -> &'static str {
        match self {
            MatchPerformanceEvidence::HighRating => "match_perf_evidence_high_rating",
            MatchPerformanceEvidence::LowRating => "match_perf_evidence_low_rating",
            MatchPerformanceEvidence::GoalContribution => "match_perf_evidence_goal_contribution",
            MatchPerformanceEvidence::DecisiveContribution => "match_perf_evidence_decisive_contribution",
            MatchPerformanceEvidence::DerbyFixture => "match_perf_evidence_derby",
            MatchPerformanceEvidence::CupTie => "match_perf_evidence_cup_tie",
            MatchPerformanceEvidence::LeagueDecider => "match_perf_evidence_league_decider",
            MatchPerformanceEvidence::SubstituteAppearance => "match_perf_evidence_substitute",
            MatchPerformanceEvidence::PlayedFullMinutes => "match_perf_evidence_full_minutes",
            MatchPerformanceEvidence::PlayedAfterCriticism => "match_perf_evidence_after_criticism",
            MatchPerformanceEvidence::HighPressurePersonality => "match_perf_evidence_high_pressure_personality",
            MatchPerformanceEvidence::LowPressurePersonality => "match_perf_evidence_low_pressure_personality",
            MatchPerformanceEvidence::ImportantMatchTemperament => "match_perf_evidence_important_match_temperament",
        }
    }
}

#[derive(Debug, Clone)]
pub struct MatchPerformanceEventContext {
    pub kind: MatchPerformanceKind,
    pub rating: Option<f32>,
    pub goals: u8,
    pub assists: u8,
    pub minutes: u16,
    pub team_won: Option<bool>,
    pub goal_margin: Option<i8>,
    pub is_derby: bool,
    pub is_cup: bool,
    pub opponent_club_id: Option<u32>,
    pub evidence: Vec<MatchPerformanceEvidence>,
}

impl MatchPerformanceEventContext {
    pub fn new(kind: MatchPerformanceKind) -> Self {
        Self {
            kind,
            rating: None,
            goals: 0,
            assists: 0,
            minutes: 0,
            team_won: None,
            goal_margin: None,
            is_derby: false,
            is_cup: false,
            opponent_club_id: None,
            evidence: Vec::new(),
        }
    }

    pub fn with_rating(mut self, rating: f32) -> Self { self.rating = Some(rating); self }
    pub fn with_goals(mut self, goals: u8) -> Self { self.goals = goals; self }
    pub fn with_assists(mut self, assists: u8) -> Self { self.assists = assists; self }
    pub fn with_minutes(mut self, minutes: u16) -> Self { self.minutes = minutes; self }
    pub fn with_team_won(mut self, won: bool) -> Self { self.team_won = Some(won); self }
    pub fn with_goal_margin(mut self, margin: i8) -> Self { self.goal_margin = Some(margin); self }
    pub fn with_derby(mut self, is_derby: bool) -> Self { self.is_derby = is_derby; self }
    pub fn with_cup(mut self, is_cup: bool) -> Self { self.is_cup = is_cup; self }
    pub fn with_opponent(mut self, club_id: u32) -> Self { self.opponent_club_id = Some(club_id); self }

    pub fn with_evidence(mut self, evidence: MatchPerformanceEvidence) -> Self {
        if !self.evidence.contains(&evidence) {
            self.evidence.push(evidence);
        }
        self
    }
}

// ─────────────────────────────────────────────────────────────────
// Role / squad-status context
// ─────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RoleStatusKind {
    RoleClarifiedByManager,
    RoleUnclear,
    DepthChartPressure,
    DirectRivalPreferred,
    TacticalRoleChanged,
    BenchedForBalance,
    RestedForWorkload,
    SquadStatusUpgrade,
    SquadStatusDowngrade,
    NoNaturalRoleInFormation,
    EstablishedStarter,
    SlippedOutOfStartingXI,
}

impl RoleStatusKind {
    pub fn as_i18n_key(&self) -> &'static str {
        match self {
            RoleStatusKind::RoleClarifiedByManager => "role_status_kind_role_clarified",
            RoleStatusKind::RoleUnclear => "role_status_kind_role_unclear",
            RoleStatusKind::DepthChartPressure => "role_status_kind_depth_chart_pressure",
            RoleStatusKind::DirectRivalPreferred => "role_status_kind_direct_rival_preferred",
            RoleStatusKind::TacticalRoleChanged => "role_status_kind_tactical_role_changed",
            RoleStatusKind::BenchedForBalance => "role_status_kind_benched_for_balance",
            RoleStatusKind::RestedForWorkload => "role_status_kind_rested_for_workload",
            RoleStatusKind::SquadStatusUpgrade => "role_status_kind_squad_status_upgrade",
            RoleStatusKind::SquadStatusDowngrade => "role_status_kind_squad_status_downgrade",
            RoleStatusKind::NoNaturalRoleInFormation => "role_status_kind_no_natural_role",
            RoleStatusKind::EstablishedStarter => "role_status_kind_established_starter",
            RoleStatusKind::SlippedOutOfStartingXI => "role_status_kind_slipped_out_xi",
        }
    }
}

#[derive(Debug, Clone)]
pub struct RoleStatusEventContext {
    pub kind: RoleStatusKind,
    pub previous_status: Option<PlayerSquadStatus>,
    pub new_status: Option<PlayerSquadStatus>,
    pub formation_slot: Option<SelectionRole>,
    pub starter_ratio: Option<f32>,
    pub repeated_omissions: u8,
    pub direct_rival_id: Option<u32>,
}

impl RoleStatusEventContext {
    pub fn new(kind: RoleStatusKind) -> Self {
        Self {
            kind,
            previous_status: None,
            new_status: None,
            formation_slot: None,
            starter_ratio: None,
            repeated_omissions: 0,
            direct_rival_id: None,
        }
    }

    pub fn with_status_change(mut self, prev: PlayerSquadStatus, new: PlayerSquadStatus) -> Self {
        self.previous_status = Some(prev);
        self.new_status = Some(new);
        self
    }
    pub fn with_formation_slot(mut self, slot: SelectionRole) -> Self { self.formation_slot = Some(slot); self }
    pub fn with_starter_ratio(mut self, ratio: f32) -> Self { self.starter_ratio = Some(ratio); self }
    pub fn with_repeated_omissions(mut self, n: u8) -> Self { self.repeated_omissions = n; self }
    pub fn with_direct_rival(mut self, id: u32) -> Self { self.direct_rival_id = Some(id); self }
}

// ─────────────────────────────────────────────────────────────────
// National-team context
// ─────────────────────────────────────────────────────────────────

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
            NationalTeamEventKind::TournamentSquadOmitted => "national_kind_tournament_squad_omitted",
            NationalTeamEventKind::InternationalPlaceUnderThreat => "national_kind_place_under_threat",
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

    pub fn with_country(mut self, country_id: u32) -> Self { self.country_id = Some(country_id); self }
    pub fn with_previous_caps(mut self, caps: u16) -> Self { self.previous_caps = caps; self }
    pub fn with_recent_club_form(mut self, form: f32) -> Self { self.recent_club_form = Some(form); self }
    pub fn with_competition_window(mut self, in_window: bool) -> Self { self.competition_window = in_window; self }
}

// ─────────────────────────────────────────────────────────────────
// Leadership / dressing-room context
// ─────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LeadershipEventKind {
    CaptaincyAwarded,
    CaptaincyRemoved,
    LeadershipEmergence,
    SeniorPlayerMediates,
    BackedBySeniorPlayers,
    ChallengedTrainingStandards,
    InfluenceInDressingRoomRising,
    InfluenceInDressingRoomFalling,
    MentorshipStarted,
    MentorshipStrained,
    SquadLeadershipQuestioned,
}

impl LeadershipEventKind {
    pub fn as_i18n_key(&self) -> &'static str {
        match self {
            LeadershipEventKind::CaptaincyAwarded => "leadership_kind_captaincy_awarded",
            LeadershipEventKind::CaptaincyRemoved => "leadership_kind_captaincy_removed",
            LeadershipEventKind::LeadershipEmergence => "leadership_kind_emergence",
            LeadershipEventKind::SeniorPlayerMediates => "leadership_kind_senior_mediates",
            LeadershipEventKind::BackedBySeniorPlayers => "leadership_kind_backed_seniors",
            LeadershipEventKind::ChallengedTrainingStandards => "leadership_kind_challenged_standards",
            LeadershipEventKind::InfluenceInDressingRoomRising => "leadership_kind_influence_rising",
            LeadershipEventKind::InfluenceInDressingRoomFalling => "leadership_kind_influence_falling",
            LeadershipEventKind::MentorshipStarted => "leadership_kind_mentorship_started",
            LeadershipEventKind::MentorshipStrained => "leadership_kind_mentorship_strained",
            LeadershipEventKind::SquadLeadershipQuestioned => "leadership_kind_squad_leadership_questioned",
        }
    }
}

#[derive(Debug, Clone)]
pub struct LeadershipEventContext {
    pub kind: LeadershipEventKind,
    pub partner_player_id: Option<u32>,
    pub leadership_attribute: Option<f32>,
    pub influence_change: Option<f32>,
}

impl LeadershipEventContext {
    pub fn new(kind: LeadershipEventKind) -> Self {
        Self {
            kind,
            partner_player_id: None,
            leadership_attribute: None,
            influence_change: None,
        }
    }

    pub fn with_partner(mut self, id: u32) -> Self { self.partner_player_id = Some(id); self }
    pub fn with_leadership_attribute(mut self, attr: f32) -> Self { self.leadership_attribute = Some(attr); self }
    pub fn with_influence_change(mut self, change: f32) -> Self { self.influence_change = Some(change); self }
}

// ─────────────────────────────────────────────────────────────────
// Media / fans context
// ─────────────────────────────────────────────────────────────────

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

    pub fn with_form_trigger(mut self) -> Self { self.trigger_due_to_form = true; self }
    pub fn with_transfer_trigger(mut self) -> Self { self.trigger_due_to_transfer = true; self }
    pub fn with_discipline_trigger(mut self) -> Self { self.trigger_due_to_discipline = true; self }
    pub fn with_big_match_trigger(mut self) -> Self { self.trigger_due_to_big_match = true; self }
}

// ─────────────────────────────────────────────────────────────────
// Personal-adaptation context
// ─────────────────────────────────────────────────────────────────

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

    pub fn with_adaptability(mut self, attr: f32) -> Self { self.adaptability = Some(attr); self }
    pub fn with_compatriot(mut self, has: bool) -> Self { self.has_compatriot_in_squad = has; self }
    pub fn with_local_language(mut self, speaks: bool) -> Self { self.speaks_local_language = speaks; self }
}

// ─────────────────────────────────────────────────────────────────
// Loan-specific context
// ─────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoanEventKind {
    LoanListingAccepted,
    LoanDevelopmentProgress,
    LoanMinutesConcern,
    LoanRecallDiscussed,
    SettledOnLoan,
    LoanMovePermanentInterest,
    LoanRoleBroken,
    ParentClubSatisfied,
    ParentClubConcerned,
}

impl LoanEventKind {
    pub fn as_i18n_key(&self) -> &'static str {
        match self {
            LoanEventKind::LoanListingAccepted => "loan_kind_listing_accepted",
            LoanEventKind::LoanDevelopmentProgress => "loan_kind_development_progress",
            LoanEventKind::LoanMinutesConcern => "loan_kind_minutes_concern",
            LoanEventKind::LoanRecallDiscussed => "loan_kind_recall_discussed",
            LoanEventKind::SettledOnLoan => "loan_kind_settled",
            LoanEventKind::LoanMovePermanentInterest => "loan_kind_permanent_interest",
            LoanEventKind::LoanRoleBroken => "loan_kind_role_broken",
            LoanEventKind::ParentClubSatisfied => "loan_kind_parent_satisfied",
            LoanEventKind::ParentClubConcerned => "loan_kind_parent_concerned",
        }
    }
}

#[derive(Debug, Clone)]
pub struct LoanEventContext {
    pub kind: LoanEventKind,
    pub parent_club_id: Option<u32>,
    pub loan_club_id: Option<u32>,
    pub minutes_share: Option<f32>,
    pub permanent_option_present: bool,
}

impl LoanEventContext {
    pub fn new(kind: LoanEventKind) -> Self {
        Self {
            kind,
            parent_club_id: None,
            loan_club_id: None,
            minutes_share: None,
            permanent_option_present: false,
        }
    }

    pub fn with_parent_club(mut self, id: u32) -> Self { self.parent_club_id = Some(id); self }
    pub fn with_loan_club(mut self, id: u32) -> Self { self.loan_club_id = Some(id); self }
    pub fn with_minutes_share(mut self, share: f32) -> Self { self.minutes_share = Some(share); self }
    pub fn with_permanent_option(mut self, present: bool) -> Self { self.permanent_option_present = present; self }
}

// ─────────────────────────────────────────────────────────────────
// Recognition / award context
// ─────────────────────────────────────────────────────────────────

/// Identifies which trophy / public recognition the event represents.
/// Maps 1:1 to the relevant `HappinessEventType` award variants and lets
/// the renderer pick recognition-specific copy without re-deriving the
/// kind from the event-type enum at render time.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecognitionEventKind {
    PlayerOfTheWeek,
    YoungPlayerOfTheWeek,
    PlayerOfTheMonth,
    YoungPlayerOfTheMonth,
    PlayerOfTheSeason,
    YoungPlayerOfTheSeason,
    TeamOfTheSeasonSelection,
    TeamOfTheYearSelection,
    LeagueTopScorer,
    LeagueTopAssists,
    LeagueGoldenGlove,
    WorldPlayerOfYear,
    WorldPlayerOfYearNomination,
    NationalTeamDebut,
}

impl RecognitionEventKind {
    pub fn as_token(&self) -> &'static str {
        match self {
            RecognitionEventKind::PlayerOfTheWeek => "player_of_the_week",
            RecognitionEventKind::YoungPlayerOfTheWeek => "young_player_of_the_week",
            RecognitionEventKind::PlayerOfTheMonth => "player_of_the_month",
            RecognitionEventKind::YoungPlayerOfTheMonth => "young_player_of_the_month",
            RecognitionEventKind::PlayerOfTheSeason => "player_of_the_season",
            RecognitionEventKind::YoungPlayerOfTheSeason => "young_player_of_the_season",
            RecognitionEventKind::TeamOfTheSeasonSelection => "team_of_the_season",
            RecognitionEventKind::TeamOfTheYearSelection => "team_of_the_year",
            RecognitionEventKind::LeagueTopScorer => "league_top_scorer",
            RecognitionEventKind::LeagueTopAssists => "league_top_assists",
            RecognitionEventKind::LeagueGoldenGlove => "league_golden_glove",
            RecognitionEventKind::WorldPlayerOfYear => "world_player_of_year",
            RecognitionEventKind::WorldPlayerOfYearNomination => "world_player_nominee",
            RecognitionEventKind::NationalTeamDebut => "national_team_debut",
        }
    }
}

/// Recognition / award explanation payload — captured at emit time so the
/// renderer can describe what was won, the season totals or vote
/// margin behind the award, and who the closest contender was.
/// All quantitative fields are `Option` so emit sites can attach what's
/// available without forcing missing-data placeholders.
#[derive(Debug, Clone)]
pub struct RecognitionEventContext {
    pub kind: RecognitionEventKind,
    pub league_id: Option<u32>,
    pub country_id: Option<u32>,
    pub season_goals: Option<u16>,
    pub season_assists: Option<u16>,
    pub season_clean_sheets: Option<u16>,
    pub avg_rating: Option<f32>,
    /// Quantitative gap to the closest contender — vote share for POM/POS,
    /// goals lead for top scorer, ratings gap for season selections.
    /// Renderer interprets this together with `kind` so the unit doesn't
    /// have to be encoded in the type system.
    pub margin: Option<f32>,
    pub runner_up_player_id: Option<u32>,
    pub matches_played: Option<u16>,
    pub previous_caps: Option<u16>,
    /// True for first-time achievements (first POM, first cap, etc.). Lets
    /// the renderer surface "first" framing when relevant.
    pub first_time: bool,
}

impl RecognitionEventContext {
    pub fn new(kind: RecognitionEventKind) -> Self {
        Self {
            kind,
            league_id: None,
            country_id: None,
            season_goals: None,
            season_assists: None,
            season_clean_sheets: None,
            avg_rating: None,
            margin: None,
            runner_up_player_id: None,
            matches_played: None,
            previous_caps: None,
            first_time: false,
        }
    }

    pub fn with_league(mut self, id: u32) -> Self { self.league_id = Some(id); self }
    pub fn with_country(mut self, id: u32) -> Self { self.country_id = Some(id); self }
    pub fn with_season_goals(mut self, goals: u16) -> Self { self.season_goals = Some(goals); self }
    pub fn with_season_assists(mut self, assists: u16) -> Self { self.season_assists = Some(assists); self }
    pub fn with_clean_sheets(mut self, cs: u16) -> Self { self.season_clean_sheets = Some(cs); self }
    pub fn with_avg_rating(mut self, rating: f32) -> Self { self.avg_rating = Some(rating); self }
    pub fn with_margin(mut self, margin: f32) -> Self { self.margin = Some(margin); self }
    pub fn with_runner_up(mut self, id: u32) -> Self { self.runner_up_player_id = Some(id); self }
    pub fn with_matches_played(mut self, m: u16) -> Self { self.matches_played = Some(m); self }
    pub fn with_previous_caps(mut self, caps: u16) -> Self { self.previous_caps = Some(caps); self }
    pub fn with_first_time(mut self, first: bool) -> Self { self.first_time = first; self }
}

// ─────────────────────────────────────────────────────────────────
// Season-outcome context — relegation / relegation-fear narrative
// ─────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SeasonOutcomeKind {
    Relegated,
    RelegationFear,
    SurvivedRelegation,
}

impl SeasonOutcomeKind {
    pub fn as_token(&self) -> &'static str {
        match self {
            SeasonOutcomeKind::Relegated => "relegated",
            SeasonOutcomeKind::RelegationFear => "relegation_fear",
            SeasonOutcomeKind::SurvivedRelegation => "survived_relegation",
        }
    }
}

/// Season-outcome context for relegation-adjacent events. Carries the
/// final / current league standing, points gap to safety, and matches
/// remaining when the worry crystallised, so the renderer can describe
/// "10th in the table, 4 points clear of the drop with 6 to play"
/// instead of a generic "Relegation fear".
#[derive(Debug, Clone)]
pub struct SeasonOutcomeContext {
    pub kind: SeasonOutcomeKind,
    pub league_id: Option<u32>,
    pub final_position: Option<u8>,
    pub points: Option<u16>,
    /// Points gap to the safety line (positive = clear of the drop,
    /// negative = below it). For `Relegated`, captures how far short.
    pub points_to_safety: Option<i16>,
    pub matches_remaining: Option<u8>,
    pub season_participation: Option<f32>,
}

impl SeasonOutcomeContext {
    pub fn new(kind: SeasonOutcomeKind) -> Self {
        Self {
            kind,
            league_id: None,
            final_position: None,
            points: None,
            points_to_safety: None,
            matches_remaining: None,
            season_participation: None,
        }
    }

    pub fn with_league(mut self, id: u32) -> Self { self.league_id = Some(id); self }
    pub fn with_final_position(mut self, pos: u8) -> Self { self.final_position = Some(pos); self }
    pub fn with_points(mut self, points: u16) -> Self { self.points = Some(points); self }
    pub fn with_points_to_safety(mut self, gap: i16) -> Self { self.points_to_safety = Some(gap); self }
    pub fn with_matches_remaining(mut self, n: u8) -> Self { self.matches_remaining = Some(n); self }
    pub fn with_participation(mut self, p: f32) -> Self { self.season_participation = Some(p); self }
}

// ─────────────────────────────────────────────────────────────────
// Regulation context — squad registration / paperwork outcomes
// ─────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RegulationSlotKind {
    HomegrownQuota,
    NonEuQuota,
    SeniorSquadCap,
    YouthSlot,
    InternationalRegistration,
    Other,
}

impl RegulationSlotKind {
    pub fn as_token(&self) -> &'static str {
        match self {
            RegulationSlotKind::HomegrownQuota => "homegrown_quota",
            RegulationSlotKind::NonEuQuota => "non_eu_quota",
            RegulationSlotKind::SeniorSquadCap => "senior_squad_cap",
            RegulationSlotKind::YouthSlot => "youth_slot",
            RegulationSlotKind::InternationalRegistration => "international_registration",
            RegulationSlotKind::Other => "other",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RegulationOutcomeKind {
    Omitted,
    Registered,
    DowngradedToReserves,
}

impl RegulationOutcomeKind {
    pub fn as_token(&self) -> &'static str {
        match self {
            RegulationOutcomeKind::Omitted => "omitted",
            RegulationOutcomeKind::Registered => "registered",
            RegulationOutcomeKind::DowngradedToReserves => "downgraded_reserves",
        }
    }
}

/// Regulation / squad-registration explanation payload. Captures which
/// slot type was contested, who took it, and why this player was the
/// odd one out — so the renderer can say "left out of the senior 25
/// to free a non-EU slot for the new signing" rather than "Squad
/// registration omitted".
#[derive(Debug, Clone)]
pub struct RegulationEventContext {
    pub outcome: RegulationOutcomeKind,
    pub slot_kind: RegulationSlotKind,
    pub competition_name_key: Option<String>,
    pub replacement_player_id: Option<u32>,
    /// How many roster slots were available; renderers may say "1 of 4".
    pub slots_total: Option<u8>,
    pub slots_used: Option<u8>,
}

impl RegulationEventContext {
    pub fn new(outcome: RegulationOutcomeKind, slot_kind: RegulationSlotKind) -> Self {
        Self {
            outcome,
            slot_kind,
            competition_name_key: None,
            replacement_player_id: None,
            slots_total: None,
            slots_used: None,
        }
    }

    pub fn with_competition(mut self, key: impl Into<String>) -> Self {
        self.competition_name_key = Some(key.into());
        self
    }
    pub fn with_replacement(mut self, id: u32) -> Self { self.replacement_player_id = Some(id); self }
    pub fn with_slots(mut self, used: u8, total: u8) -> Self {
        self.slots_used = Some(used);
        self.slots_total = Some(total);
        self
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum HappinessEventType {
    // Manager interactions
    ManagerPraise,
    ManagerDiscipline,
    ManagerPlayingTimePromise,
    ManagerCriticism,
    ManagerEncouragement,
    ManagerTacticalInstruction,
    // Training
    GoodTraining,
    PoorTraining,
    // Match selection
    MatchDropped,
    // Contract & transfers
    ContractOffer,
    ContractRenewal,
    SquadStatusChange,
    LackOfPlayingTime,
    LoanListingAccepted,
    // Injury
    InjuryReturn,
    // Match performance
    PlayerOfTheMatch,
    /// Named the league's Player of the Week — chosen Mondays based on the
    /// previous calendar week's performances. Bigger than POM (which is one
    /// match) and rarer (one player per league per week). Career-visible.
    PlayerOfTheWeek,
    // Team/squad relationship
    TeammateBonding,
    ConflictWithTeammate,
    DressingRoomSpeech,
    SettledIntoSquad,
    FeelingIsolated,
    /// Teammate signed a meaningfully bigger deal and this player noticed —
    /// drags salary_satisfaction. Typically only fires if the friendship
    /// with the newly-signed teammate is low.
    SalaryGapNoticed,
    /// Manager kept a concrete promise (e.g. more playing time).
    PromiseKept,
    /// Manager broke a concrete promise. Big morale hit, erodes trust.
    PromiseBroken,
    /// Fresh transfer landed the player at a club whose reputation sits well
    /// below what his ambition expects. Lingers while the gap exists.
    AmbitionShock,
    /// New contract is dramatically worse than the pre-transfer salary —
    /// e.g. Messi moving to a Maltese club on a 1/100th deal.
    SalaryShock,
    /// Team's primary formation has no slot for the player's preferred
    /// position. Degrades ambition_fit until a compatible role opens.
    RoleMismatch,
    /// Signed for a club well above the player's expectations — an
    /// unambiguous step up (small-club talent joining Barça / Madrid).
    DreamMove,
    /// New contract pays materially more than the previous deal — the
    /// positive counterpart to SalaryShock.
    SalaryBoost,
    /// Joined a genuinely elite club (top-tier reputation). Fires only
    /// when the move is also a step up relative to the player's own
    /// reputation, to avoid stacking with DreamMove at mid-table moves.
    JoiningElite,
    /// Club bought the player out of his contract — a mild blow to pride
    /// softened by the severance payout. Emitted on mutual termination.
    ContractTerminated,
    /// Head coach was replaced. Fires per-player: strongly negative for
    /// players who had a close bond with the outgoing manager, mildly
    /// positive for players whose relationship had soured.
    ManagerDeparture,
    /// Called up to the senior national team. Big prestige moment for
    /// younger players, routine for established internationals.
    NationalTeamCallup,
    /// Dropped from the national team squad after previous caps — hurts
    /// pride more than a non-selection would.
    NationalTeamDropped,
    /// Promoted to a prestigious shirt number (1-11, esp. #10 / #7 / #9).
    /// Small ongoing pride boost while the number holds.
    ShirtNumberPromotion,
    /// Had a controversial incident (media or dressing room) — fallout
    /// tied to `controversy` personality attribute.
    ControversyIncident,

    // ── Match performance ────────────────────────────────────────
    /// First competitive goal scored for this club. Career milestone —
    /// one-shot per club, lingers in memory for the season.
    FirstClubGoal,
    /// Scored or assisted a goal that decided a tight match. Bigger
    /// than a routine goal, smaller than POM unless paired with it.
    DecisiveGoal,
    /// Came on as a substitute and made a clear positive impact —
    /// scored, assisted, or finished with a high rating off the bench.
    SubstituteImpact,
    /// Defender or goalkeeper kept a clean sheet. Position-gated —
    /// strikers don't care about clean sheets.
    CleanSheetPride,
    /// Finished a match with a costly low rating, often paired with
    /// a goal conceded the player was directly responsible for.
    CostlyMistake,
    /// Sent off (direct red or two yellows). Lingers as embarrassment
    /// plus the suspension fallout.
    RedCardFallout,
    /// Standout performer in a derby win — scorer, assister, POM, or
    /// high-rated display. Reserved for players who carried the win;
    /// ordinary participants get the squad-wide [`DerbyWin`] instead.
    DerbyHero,
    /// Squad-wide moderate positive for being on the winning side of a
    /// derby. Distinct from [`DerbyHero`], which is reserved for the
    /// match's standout performers.
    DerbyWin,
    /// Lost a derby — meaningfully bigger blow than a generic defeat.
    /// Lingers; rivalry loss isn't shaken off in a week.
    DerbyDefeat,

    // ── Team season events ──────────────────────────────────────
    /// Team won a trophy (league, cup, continental). Major career moment.
    TrophyWon,
    /// Team lost a cup final. The flip side of TrophyWon — tournament
    /// runs that ended in heartbreak weigh on a squad.
    CupFinalDefeat,
    /// Team confirmed promotion to a higher division.
    PromotionCelebration,
    /// Team is in the relegation fight late in the season — ambient
    /// dread that builds with the season trajectory.
    RelegationFear,
    /// Team was relegated. Major morale hit, particularly for ambitious
    /// players who'll often want a transfer afterwards.
    Relegated,
    /// Team qualified for European competition — a real boost for
    /// ambitious squads who treat continental football as the floor.
    QualifiedForEurope,

    // ── Role / status ───────────────────────────────────────────
    /// Cemented a place in the starting XI after fighting for it. Fires
    /// once per spell — the moment the manager's trust is established.
    WonStartingPlace,
    /// Lost the starting place to a teammate / new signing. Fires once
    /// per spell on the cusp of being benched, not every dropped match.
    LostStartingPlace,
    /// Awarded the captain's armband. Big prestige and trust signal.
    CaptaincyAwarded,
    /// Stripped of the captain's armband. Wounding — rarely forgotten.
    CaptaincyRemoved,
    /// Young player promoted from academy / development squad to senior
    /// matchday duty for the first time. One-shot career milestone.
    YouthBreakthrough,
    /// Left out of the squad registration list for a competition. Frozen
    /// out of matchday minutes for the duration of that registration window.
    ///
    /// **Reserved.** No emit site exists today — the simulation has
    /// `ForeignPlayerLimits` / `YouthRequirements` placeholders in
    /// `continent::regulations::types`, but no per-club registration list
    /// is enforced and `match_squad` picks XI matchday-by-matchday with
    /// no separate roster gate. When a registration window is added
    /// (continental cup squad lists, foreign-player caps), emit this for
    /// `KeyPlayer` / `FirstTeamRegular` who were expected to be included
    /// but weren't. Do **not** infer it from match-day non-selection —
    /// that's a manager call, not a roster lockout, and a different event.
    SquadRegistrationOmitted,

    // ── Transfer / media ────────────────────────────────────────
    /// Confirmed concrete interest from a club meaningfully bigger than
    /// the current one. Flattery for ambitious players, distraction for
    /// settled ones — replaces the old "manager-encouragement" misuse.
    WantedByBiggerClub,
    /// Bid for the player from another club was rejected by the selling
    /// side. Frustrating for an ambitious player who saw the move coming.
    TransferBidRejected,
    /// A transfer the player was set on collapsed at a late stage —
    /// medical, registration, or club back-out. Lingering bitterness.
    DreamMoveCollapsed,
    /// Scout from a meaningful club has been watching the player. Often
    /// invisible, but lands as a small confidence note when it leaks /
    /// repeats / coincides with an ambition trigger.
    ScoutedByClub,
    /// Loose rumour — the player heard the link but the interested club
    /// has not put concrete weight behind it.
    TransferRumour,
    /// Agent / representatives have been actively stirring interest with
    /// other clubs. Distinct from a leaked club briefing.
    AgentStirsInterest,
    /// Concrete interest from a club well above this player's current
    /// level. Distinct from the legacy `WantedByBiggerClub` in that it
    /// carries the full `TransferInterestContext` payload.
    InterestFromBiggerClub,
    /// Concrete interest from a known sporting rival. Even at lateral
    /// rep this raises pressure / fan backlash risk.
    InterestFromRival,
    /// Rumour or approach links the player to a club in their home
    /// country. Emotionally charged regardless of pure rep gap.
    HomecomingRumour,
    /// Approach from a club the player previously played for. Pulls on
    /// loyalty / unfinished-business strings.
    FormerClubInterest,
    /// Approach from a club listed as the player's favourite. Strong
    /// emotional pull; often produces excitement even before a bid.
    FavoriteClubInterest,
    /// Repeated speculation that the player has not yet shaken off —
    /// distracts focus, drags pressure load.
    TransferSpeculationDistracts,
    /// Player publicly dismisses the speculation and reaffirms focus
    /// on the current club. Small positive PR + dressing-room effect.
    TransferInterestDismissed,
    /// Talks with the interested club are imminent / opening — the
    /// player is now in the final stages of a possible move.
    TransferTalksExpected,
    /// Previously concrete interest has cooled — the buying club has
    /// moved on without a bid. Mild disappointment for the player.
    InterestCooled,
    /// Player used external interest as leverage during contract
    /// renewal — produces a small confidence effect plus a follow-up
    /// risk flag.
    UsedInterestForContractLeverage,
    /// Supporter reaction to an active transfer rumour — split between
    /// "stay" and "go" voices, tracked as fan pressure.
    FansReactToTransferRumour,
    /// Praised by the supporters — banners, songs, fan-poll wins.
    FanPraise,
    /// Targeted by fan criticism — bad displays, off-field controversy.
    FanCriticism,
    /// Praised in the media. Reputation-boosting profile pieces, top
    /// pundit ratings.
    MediaPraise,
    /// Targeted by media criticism. Hatchet jobs, tabloid drama.
    MediaCriticism,

    // ── Social / culture ────────────────────────────────────────
    /// A close friend / mentor / linchpin teammate left the club. Players
    /// with strong relationships at the dressing-room core feel this.
    CloseFriendSold,
    /// A compatriot (same primary nationality) joined the club. Big
    /// integration boost for foreign players battling language/culture.
    CompatriotJoined,
    /// Veteran mentor on whom this young player relied departed. Hits
    /// developing players who lost an established guidance figure.
    MentorDeparted,
    /// Made meaningful progress with the local language. Self-reinforcing
    /// integration milestone, only fires for foreign players.
    LanguageProgress,

    // ── Awards / nominations ────────────────────────────────────
    PlayerOfTheMonth,
    YoungPlayerOfTheMonth,
    /// Named the league's Young Player of the Week (age ≤ 20). Career
    /// memory for emerging talent — distinct from the broader Player
    /// of the Week so a 19-year-old who edged the senior award doesn't
    /// suppress the under-20 recognition.
    YoungPlayerOfTheWeek,
    TeamOfTheWeekSelection,
    /// Selected in the Young Team of the Week (age ≤ 20). Recognition
    /// for an under-20 starting in the weekly young XI.
    YoungTeamOfTheWeekSelection,
    TeamOfTheSeasonSelection,
    /// Selected in the league's calendar-year XI (Team of the Year).
    /// Distinct from `TeamOfTheSeasonSelection`, which is per-season.
    TeamOfTheYearSelection,
    PlayerOfTheSeason,
    YoungPlayerOfTheSeason,
    LeagueTopScorer,
    LeagueTopAssists,
    LeagueGoldenGlove,
    ContinentalPlayerOfYearNomination,
    ContinentalPlayerOfYear,
    WorldPlayerOfYearNomination,
    WorldPlayerOfYear,

    // ── Real-life football events ────────────────────────────────
    /// First competitive senior appearance for the current club.
    SeniorDebut,
    /// First international appearance after being capped (transitions
    /// from 0 to >0 international apps).
    NationalTeamDebut,
    /// Three or more goals in a non-friendly match.
    HatTrick,
    /// Three or more assists in a non-friendly match.
    AssistHatTrick,
    /// Returned to scoring after a long competitive drought.
    GoalDroughtEnded,
    /// Forward facing a sustained scoring drought.
    ScoringDroughtConcern,
    /// Reached a competitive appearances milestone.
    AppearanceMilestone,
    /// Reached a competitive goals milestone.
    GoalMilestone,
    /// Reached a competitive clean sheets milestone (GK only).
    CleanSheetMilestone,
    /// High-controversy / low-temperament training-ground confrontation.
    TrainingGroundBustUp,
    /// Public apology following a controversy / bust-up.
    PublicApology,
    /// Supporters chanted the player's name in a strong performance.
    FansChantPlayerName,
    /// Sustained negative media coverage at high-profile reputation.
    MediaPressureMounting,
    /// Veteran captain / senior pro stepping up as dressing-room leader.
    LeadershipEmergence,
}

impl PlayerHappiness {
    pub fn new() -> Self {
        let cfg = HappinessConfig::default();
        PlayerHappiness {
            morale: cfg.default_morale,
            factors: HappinessFactors::default(),
            recent_events: Vec::new(),
            last_salary_negotiation: None,
            starter_ratio: 0.5,
            appearances_tracked: 0,
            is_established_starter: false,
            apps_since_last_competitive_goal: 0,
            recent_low_rating_mask: 0,
            recent_low_rating_len: 0,
        }
    }

    pub fn recalculate_morale(&mut self) {
        let cfg = HappinessConfig::default();
        let core_factor_sum = self.factors.playing_time
            + self.factors.salary_satisfaction
            + self.factors.manager_relationship
            + self.factors.ambition_fit
            + self.factors.injury_frustration
            + self.factors.recent_praise
            + self.factors.recent_discipline;

        // Derived "life in the team" factors. Weighted to 0.6× of their
        // raw range so they enrich morale without dominating the core
        // axes the audit already balances around. Each factor is
        // independently clamped at compute time.
        let derived_sum = (self.factors.role_clarity
            + self.factors.coach_credibility
            + self.factors.dressing_room_status
            + self.factors.club_fit
            + self.factors.pressure_load
            + self.factors.promise_trust)
            * 0.6;

        let event_sum: f32 = self
            .recent_events
            .iter()
            .map(|e| e.magnitude * cfg.event_decay(e.days_ago))
            .sum();

        self.morale =
            cfg.clamp_morale(cfg.default_morale + core_factor_sum + derived_sum + event_sum);
    }

    pub fn adjust_morale(&mut self, delta: f32) {
        let cfg = HappinessConfig::default();
        self.morale = cfg.clamp_morale(self.morale + delta);
    }

    pub fn decay_events(&mut self) {
        let cfg = HappinessConfig::default();
        for event in &mut self.recent_events {
            event.days_ago += cfg.decay_step_days;
        }
        self.recent_events
            .retain(|e| e.days_ago <= cfg.event_retention_days);

        if self.recent_events.len() > cfg.recent_events_cap {
            self.recent_events
                .sort_by(|a, b| a.days_ago.cmp(&b.days_ago));
            self.recent_events.truncate(cfg.recent_events_cap);
        }
    }

    pub fn add_event(&mut self, event_type: HappinessEventType, magnitude: f32) {
        self.add_event_with_partner(event_type, magnitude, None);
    }

    /// Same as `add_event` but tags the event with a teammate / partner
    /// player id so the UI can render an inline link. Use this for events
    /// that naturally involve a specific peer (TeammateBonding,
    /// ConflictWithTeammate, CloseFriendSold, MentorDeparted,
    /// CompatriotJoined). The partner id has no effect on morale — it's
    /// purely informational.
    ///
    /// Enforcement: events listed in `requires_partner_id` MUST be emitted
    /// with a `Some(_)` partner id. Calls that pass `None` for those types
    /// are dropped here — the event would otherwise reach the UI as
    /// orphaned text ("bonded with a teammate" — which one?), be filtered
    /// out at render time, and waste a slot in `recent_events`. Failing
    /// silently at the source forces the emit-site to either supply the
    /// partner id or pick a different event type.
    pub fn add_event_with_partner(
        &mut self,
        event_type: HappinessEventType,
        magnitude: f32,
        partner_player_id: Option<u32>,
    ) {
        self.add_event_full(event_type, magnitude, partner_player_id, None);
    }

    /// Same as [`Self::add_event_with_partner`] but also attaches a
    /// structured [`HappinessEventContext`] for the renderer. Used by
    /// the upgraded emit sites (PlayerBehaviourResult, controversy
    /// pipeline, transfer-social, squad integration) so the UI can
    /// produce a real explanation instead of a static black-box line.
    pub fn add_event_with_context(
        &mut self,
        event_type: HappinessEventType,
        magnitude: f32,
        partner_player_id: Option<u32>,
        context: HappinessEventContext,
    ) {
        self.add_event_full(event_type, magnitude, partner_player_id, Some(context));
    }

    /// Cooldown-gated counterpart of `add_event_with_context`.
    pub fn add_event_with_context_and_cooldown(
        &mut self,
        event_type: HappinessEventType,
        magnitude: f32,
        partner_player_id: Option<u32>,
        context: HappinessEventContext,
        cooldown_days: u16,
    ) -> bool {
        if self.has_recent_event(&event_type, cooldown_days) {
            return false;
        }
        self.add_event_full(event_type, magnitude, partner_player_id, Some(context));
        true
    }

    /// Cooldown-gated, partner-aware counterpart of
    /// `add_event_with_context`. Cooldown is keyed by `(event_type,
    /// partner_id)` so a chronic friction pair doesn't suppress a
    /// different teammate's first incident with the same type.
    pub fn add_event_with_partner_context_and_cooldown(
        &mut self,
        event_type: HappinessEventType,
        magnitude: f32,
        partner_player_id: u32,
        context: HappinessEventContext,
        cooldown_days: u16,
    ) -> bool {
        if self.has_recent_event_with_partner(&event_type, partner_player_id, cooldown_days) {
            return false;
        }
        self.add_event_full(
            event_type,
            magnitude,
            Some(partner_player_id),
            Some(context),
        );
        true
    }

    fn add_event_full(
        &mut self,
        event_type: HappinessEventType,
        magnitude: f32,
        partner_player_id: Option<u32>,
        context: Option<HappinessEventContext>,
    ) {
        if requires_partner_id(&event_type) && partner_player_id.is_none() {
            debug_assert!(
                false,
                "{:?} requires a partner_player_id; use add_event_with_partner",
                event_type
            );
            return;
        }
        if let Some(ctx) = context.as_ref() {
            // Specialized payloads are mutually exclusive — an event is
            // *either* a selection event, *or* a transfer-interest event,
            // etc. Attaching two at the same emit site is a bug that would
            // confuse the renderer's dispatch and produce mixed copy.
            debug_assert!(
                ctx.specialized_payload_count() <= 1,
                "{:?} carries {} specialized payloads (max 1); emit site attached more than one with_*_context",
                event_type,
                ctx.specialized_payload_count()
            );
        }
        let cfg = HappinessConfig::default();
        self.recent_events.push(HappinessEvent {
            event_type,
            magnitude,
            days_ago: 0,
            partner_player_id,
            context,
        });

        if self.recent_events.len() > cfg.recent_events_cap {
            self.recent_events
                .sort_by(|a, b| a.days_ago.cmp(&b.days_ago));
            self.recent_events.truncate(cfg.recent_events_cap);
        }
    }

    /// True if an event of `event_type` was recorded within the last
    /// `days` days (inclusive). Cheap O(n) scan — `recent_events` is
    /// capped, so this is bounded.
    pub fn has_recent_event(&self, event_type: &HappinessEventType, days: u16) -> bool {
        self.recent_events
            .iter()
            .any(|e| e.event_type == *event_type && e.days_ago <= days)
    }

    /// Same as [`Self::has_recent_event`] but filtered to events tagged
    /// with the given partner. Use this for per-pair cooldowns — e.g.
    /// to avoid emitting "ConflictWithTeammate (vs player X)" every
    /// week when the underlying friction is constant.
    pub fn has_recent_event_with_partner(
        &self,
        event_type: &HappinessEventType,
        partner_player_id: u32,
        days: u16,
    ) -> bool {
        self.recent_events.iter().any(|e| {
            e.event_type == *event_type
                && e.partner_player_id == Some(partner_player_id)
                && e.days_ago <= days
        })
    }

    /// Add an event only if no event of this type was emitted in the
    /// last `cooldown_days`. Returns `true` if the event was recorded.
    /// Centralised cooldown gate so emit sites don't reimplement the
    /// "did we already fire this recently" pattern (the audit found
    /// inline copies in `process_contract_jealousy` and
    /// `process_periodic_wage_envy`).
    pub fn add_event_with_cooldown(
        &mut self,
        event_type: HappinessEventType,
        magnitude: f32,
        cooldown_days: u16,
    ) -> bool {
        if self.has_recent_event(&event_type, cooldown_days) {
            return false;
        }
        self.add_event(event_type, magnitude);
        true
    }

    /// Cooldown-gated counterpart of `add_event_with_partner`. Use this
    /// for partner-required events that also want a cooldown — emitting
    /// via the partner-less variant would be silently dropped by the
    /// `requires_partner_id` guard.
    pub fn add_event_with_partner_and_cooldown(
        &mut self,
        event_type: HappinessEventType,
        magnitude: f32,
        partner_player_id: Option<u32>,
        cooldown_days: u16,
    ) -> bool {
        if self.has_recent_event(&event_type, cooldown_days) {
            return false;
        }
        self.add_event_with_partner(event_type, magnitude, partner_player_id);
        true
    }

    /// Catalog-default counterpart of [`add_event_with_cooldown`].
    pub fn add_event_default_with_cooldown(
        &mut self,
        event_type: HappinessEventType,
        cooldown_days: u16,
    ) -> bool {
        if self.has_recent_event(&event_type, cooldown_days) {
            return false;
        }
        self.add_event_default(event_type);
        true
    }

    /// Record an event using the catalog's default magnitude. Equivalent
    /// to `add_event(event_type, catalog.magnitude(event_type))`. Use this
    /// for emit sites whose magnitude is the canonical default — single-
    /// magnitude events that don't depend on context.
    pub fn add_event_default(&mut self, event_type: HappinessEventType) {
        let cfg = HappinessConfig::default();
        let magnitude = cfg.catalog.magnitude(event_type.clone());
        self.add_event(event_type, magnitude);
    }

    /// Record an event with a magnitude scaled relative to the catalog
    /// default. `factor=1.0` is equivalent to `add_event_default`. Use
    /// this for emit sites where the magnitude varies by context (severity,
    /// loan damp, etc.) but the *base* should still come from the catalog.
    pub fn add_event_scaled(&mut self, event_type: HappinessEventType, factor: f32) {
        let cfg = HappinessConfig::default();
        let magnitude = cfg.catalog.magnitude(event_type.clone()) * factor;
        self.add_event(event_type, magnitude);
    }

    /// Reset happiness to neutral state (fresh start at a new club).
    /// `HappinessFactors::default()` zeroes all six derived factors —
    /// they're recomputed on the first weekly tick at the new club.
    pub fn clear(&mut self) {
        let cfg = HappinessConfig::default();
        self.morale = cfg.default_morale;
        self.factors = HappinessFactors::default();
        self.recent_events.clear();
        self.last_salary_negotiation = None;
        self.starter_ratio = 0.5;
        self.appearances_tracked = 0;
        self.is_established_starter = false;
        self.apps_since_last_competitive_goal = 0;
        self.recent_low_rating_mask = 0;
        self.recent_low_rating_len = 0;
    }

    /// Backward compatible: morale >= happy_threshold means happy.
    pub fn is_happy(&self) -> bool {
        self.morale >= HappinessConfig::default().happy_threshold
    }

    /// Backward compatible: push a positive event
    pub fn add_positive(&mut self, _item: PositiveHappiness) {
        self.add_event_default(HappinessEventType::GoodTraining);
    }

    /// Backward compatible: push a negative event
    pub fn add_negative(&mut self, _item: NegativeHappiness) {
        self.add_event_default(HappinessEventType::PoorTraining);
    }
}

/// Event types that name a specific teammate and therefore must carry a
/// `partner_player_id`. Mirrors the web layer's `is_partner_required`
/// gate — kept here as the source of truth so emit-side and render-side
/// agree. Adding a new partner-style event type means listing it both
/// here (to enforce at emit) and in the web filter (to render the link).
fn requires_partner_id(event_type: &HappinessEventType) -> bool {
    matches!(
        event_type,
        HappinessEventType::TeammateBonding
            | HappinessEventType::ConflictWithTeammate
            | HappinessEventType::CloseFriendSold
            | HappinessEventType::MentorDeparted
            | HappinessEventType::CompatriotJoined
    )
}

/// Kept for backward compatibility
#[derive(Debug, Clone)]
pub struct PositiveHappiness {
    pub description: String,
}

/// Kept for backward compatibility
#[derive(Debug, Clone)]
pub struct NegativeHappiness {
    pub description: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fresh_context_carries_no_specialized_payload() {
        let ctx = HappinessEventContext::new(
            HappinessEventCause::Other,
            HappinessEventSeverity::Minor,
            HappinessEventScope::Personal,
        );
        assert_eq!(ctx.specialized_payload_count(), 0);
    }

    #[test]
    fn each_with_specialized_context_yields_payload_count_one() {
        // Spot-check several builders — the renderer dispatch trusts
        // that exactly one specialized payload is set per event, so a
        // single with_*_context call must produce count == 1 (and not,
        // say, count == 2 due to a copy-paste bug in a future builder).
        let base = || {
            HappinessEventContext::new(
                HappinessEventCause::Other,
                HappinessEventSeverity::Minor,
                HappinessEventScope::Personal,
            )
        };
        let recognition = base().with_recognition_context(RecognitionEventContext::new(
            RecognitionEventKind::PlayerOfTheWeek,
        ));
        assert_eq!(recognition.specialized_payload_count(), 1);

        let season = base().with_season_outcome_context(SeasonOutcomeContext::new(
            SeasonOutcomeKind::Relegated,
        ));
        assert_eq!(season.specialized_payload_count(), 1);

        let regulation = base().with_regulation_context(RegulationEventContext::new(
            RegulationOutcomeKind::Omitted,
            RegulationSlotKind::NonEuQuota,
        ));
        assert_eq!(regulation.specialized_payload_count(), 1);
    }

    #[test]
    fn double_attached_specialized_payload_count_exceeds_one() {
        // If a future emit site (or a careless refactor) attaches two
        // specialized contexts, `specialized_payload_count` must return
        // >1 so the debug_assert in `add_event_full` catches it. This
        // test pins that the counter actually counts each payload, so
        // the runtime guard remains effective.
        let ctx = HappinessEventContext::new(
            HappinessEventCause::Other,
            HappinessEventSeverity::Minor,
            HappinessEventScope::Personal,
        )
        .with_recognition_context(RecognitionEventContext::new(
            RecognitionEventKind::PlayerOfTheWeek,
        ))
        .with_season_outcome_context(SeasonOutcomeContext::new(
            SeasonOutcomeKind::Relegated,
        ));
        assert_eq!(ctx.specialized_payload_count(), 2);
    }

    #[test]
    #[should_panic(expected = "specialized payloads")]
    fn add_event_with_double_specialized_payload_panics_in_debug() {
        // The debug_assert in add_event_full fires when an emit site
        // attaches two specialized payloads. Tests run with
        // debug_assertions enabled, so this should panic. In release
        // builds the event is still recorded (best-effort), but the
        // mutually-exclusive contract is enforced under tests.
        let mut h = PlayerHappiness::new();
        let bad_ctx = HappinessEventContext::new(
            HappinessEventCause::Other,
            HappinessEventSeverity::Moderate,
            HappinessEventScope::Media,
        )
        .with_recognition_context(RecognitionEventContext::new(
            RecognitionEventKind::PlayerOfTheMonth,
        ))
        .with_regulation_context(RegulationEventContext::new(
            RegulationOutcomeKind::Omitted,
            RegulationSlotKind::Other,
        ));
        h.add_event_with_context(
            HappinessEventType::PlayerOfTheMonth,
            5.0,
            None,
            bad_ctx,
        );
    }

    #[test]
    fn cooldown_blocks_duplicate_event() {
        let mut h = PlayerHappiness::new();
        let added = h.add_event_with_cooldown(HappinessEventType::DerbyHero, 5.0, 14);
        assert!(added, "first emit should land");
        let second = h.add_event_with_cooldown(HappinessEventType::DerbyHero, 5.0, 14);
        assert!(!second, "second emit inside cooldown should be skipped");
        assert_eq!(
            h.recent_events
                .iter()
                .filter(|e| e.event_type == HappinessEventType::DerbyHero)
                .count(),
            1
        );
    }

    #[test]
    fn cooldown_lapses_after_age() {
        let mut h = PlayerHappiness::new();
        h.add_event_with_cooldown(HappinessEventType::SettledIntoSquad, 2.0, 14);
        // Simulate time passing — bump days_ago past the cooldown window.
        h.recent_events[0].days_ago = 21;
        let added = h.add_event_with_cooldown(HappinessEventType::SettledIntoSquad, 2.0, 14);
        assert!(added, "emit should resume once cooldown has elapsed");
    }

    #[test]
    fn has_recent_event_distinguishes_event_types() {
        let mut h = PlayerHappiness::new();
        h.add_event_default(HappinessEventType::DerbyHero);
        assert!(h.has_recent_event(&HappinessEventType::DerbyHero, 30));
        assert!(!h.has_recent_event(&HappinessEventType::DerbyDefeat, 30));
    }

    #[test]
    fn severity_thresholds_are_stable() {
        // Boundary checks — keep these in lockstep with renderer copy
        // and tests that assert the visible label.
        assert_eq!(
            HappinessEventSeverity::from_magnitude(0.5),
            HappinessEventSeverity::Minor
        );
        assert_eq!(
            HappinessEventSeverity::from_magnitude(1.9),
            HappinessEventSeverity::Minor
        );
        assert_eq!(
            HappinessEventSeverity::from_magnitude(2.0),
            HappinessEventSeverity::Moderate
        );
        assert_eq!(
            HappinessEventSeverity::from_magnitude(-3.5),
            HappinessEventSeverity::Moderate
        );
        assert_eq!(
            HappinessEventSeverity::from_magnitude(4.0),
            HappinessEventSeverity::Serious
        );
        assert_eq!(
            HappinessEventSeverity::from_magnitude(-5.9),
            HappinessEventSeverity::Serious
        );
        assert_eq!(
            HappinessEventSeverity::from_magnitude(6.0),
            HappinessEventSeverity::Major
        );
        assert_eq!(
            HappinessEventSeverity::from_magnitude(-12.0),
            HappinessEventSeverity::Major
        );
    }

    #[test]
    fn legacy_emit_paths_carry_no_context() {
        let mut h = PlayerHappiness::new();
        h.add_event(HappinessEventType::PoorTraining, -1.0);
        let event = h.recent_events.last().unwrap();
        assert!(
            event.context.is_none(),
            "legacy emit must not synthesise a context — None means 'unknown', \
             which the renderer falls back from cleanly"
        );
    }

    #[test]
    fn add_event_with_context_round_trips() {
        let mut h = PlayerHappiness::new();
        let ctx = HappinessEventContext::new(
            HappinessEventCause::PositionalRivalry,
            HappinessEventSeverity::Moderate,
            HappinessEventScope::DressingRoom,
        )
        .with_relationship_level(-30.0)
        .with_follow_up(HappinessEventFollowUp::DressingRoomDamageRisk);
        h.add_event_with_context(
            HappinessEventType::ConflictWithTeammate,
            -2.0,
            Some(99),
            ctx,
        );
        let event = h.recent_events.last().unwrap();
        let stored = event.context.as_ref().expect("context must round-trip");
        assert_eq!(stored.cause, HappinessEventCause::PositionalRivalry);
        assert_eq!(stored.severity, HappinessEventSeverity::Moderate);
        assert_eq!(stored.scope, HappinessEventScope::DressingRoom);
        assert_eq!(stored.relationship_level_before, Some(-30.0));
        assert_eq!(
            stored.follow_up,
            Some(HappinessEventFollowUp::DressingRoomDamageRisk)
        );
        assert_eq!(event.partner_player_id, Some(99));
    }

    #[test]
    fn partner_required_event_without_partner_is_dropped() {
        let mut h = PlayerHappiness::new();
        // debug_assertions panic in test builds — wrap in catch_unwind so
        // we can assert that the event is not silently committed.
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let ctx = HappinessEventContext::new(
                HappinessEventCause::PersonalityClash,
                HappinessEventSeverity::Minor,
                HappinessEventScope::DressingRoom,
            );
            h.add_event_with_context(
                HappinessEventType::ConflictWithTeammate,
                -1.0,
                None,
                ctx,
            );
        }));
        assert!(
            result.is_err() || h.recent_events.is_empty(),
            "partner-required event without partner_id must not land in recent_events"
        );
    }

    #[test]
    fn match_selection_context_round_trips_through_event_context() {
        let mut h = PlayerHappiness::new();
        let sel = MatchSelectionContext {
            scope: SelectionDecisionScope::DroppedToBench,
            reason: SelectionOmissionReason::TeammatePreferredOnFitness,
            comparison: Some(SelectionComparison {
                selected_player_id: 42,
                selected_was_starter: true,
                slot: Some(SelectionRole::Winger),
                selected_score: 14.5,
                omitted_score: 12.0,
                top_factors: vec![SelectionScoreFactor::MatchReadiness],
            }),
            role: SelectionRole::Winger,
            match_importance: 0.8,
            repeated: false,
            is_friendly: false,
        };
        let ctx = HappinessEventContext::new(
            HappinessEventCause::PositionalRivalry,
            HappinessEventSeverity::Moderate,
            HappinessEventScope::MatchDay,
        )
        .with_selection_context(sel);
        h.add_event_with_context(HappinessEventType::MatchDropped, -2.0, None, ctx);

        let event = h.recent_events.last().expect("event must land");
        let stored = event
            .context
            .as_ref()
            .and_then(|c| c.selection_context.as_ref())
            .expect("selection context round-trips");
        assert_eq!(stored.scope, SelectionDecisionScope::DroppedToBench);
        assert_eq!(
            stored.reason,
            SelectionOmissionReason::TeammatePreferredOnFitness
        );
        let comp = stored.comparison.as_ref().expect("comparison present");
        assert_eq!(comp.selected_player_id, 42);
        assert!(comp.selected_was_starter);
        assert_eq!(comp.slot, Some(SelectionRole::Winger));
    }

    #[test]
    fn selection_omission_reason_keys_are_unique_and_non_empty() {
        let reasons = [
            SelectionOmissionReason::LowerMatchReadiness,
            SelectionOmissionReason::FitnessProtection,
            SelectionOmissionReason::FatigueManagement,
            SelectionOmissionReason::PoorRecentForm,
            SelectionOmissionReason::TacticalMismatch,
            SelectionOmissionReason::PositionFitIssue,
            SelectionOmissionReason::TeammatePreferredOnAbility,
            SelectionOmissionReason::TeammatePreferredOnForm,
            SelectionOmissionReason::TeammatePreferredOnFitness,
            SelectionOmissionReason::TeammatePreferredOnTrust,
            SelectionOmissionReason::TeammatePreferredForTacticalBalance,
            SelectionOmissionReason::YouthDevelopmentRotation,
            SelectionOmissionReason::CupRotation,
            SelectionOmissionReason::LowMatchImportanceRotation,
            SelectionOmissionReason::SquadStatusMismatch,
            SelectionOmissionReason::ManagerDoesNotTrustPlayer,
            SelectionOmissionReason::NewcomerStillIntegrating,
            SelectionOmissionReason::ReturningFromInjury,
            SelectionOmissionReason::DisciplinarySelection,
            SelectionOmissionReason::BenchBalance,
            SelectionOmissionReason::NoNaturalRoleInFormation,
        ];
        let mut keys: Vec<&'static str> = reasons.iter().map(|r| r.as_i18n_key()).collect();
        keys.sort();
        let unique = {
            let mut k = keys.clone();
            k.dedup();
            k.len()
        };
        assert_eq!(keys.len(), unique, "reason keys must be unique");
        for k in &keys {
            assert!(!k.is_empty(), "reason i18n key must be non-empty");
            assert!(
                k.starts_with("selection_reason_"),
                "reason key {} must follow the naming convention",
                k
            );
        }
    }

    #[test]
    fn partner_aware_cooldown_is_per_partner() {
        let mut h = PlayerHappiness::new();
        let ctx = HappinessEventContext::new(
            HappinessEventCause::TrainingFriction,
            HappinessEventSeverity::Minor,
            HappinessEventScope::TrainingGround,
        );
        let added_first = h.add_event_with_partner_context_and_cooldown(
            HappinessEventType::ConflictWithTeammate,
            -1.0,
            7,
            ctx.clone(),
            45,
        );
        assert!(added_first);
        // Same partner inside cooldown — blocked.
        let added_again = h.add_event_with_partner_context_and_cooldown(
            HappinessEventType::ConflictWithTeammate,
            -1.0,
            7,
            ctx.clone(),
            45,
        );
        assert!(!added_again, "same partner inside cooldown must be blocked");
        // Different partner — should land.
        let added_other = h.add_event_with_partner_context_and_cooldown(
            HappinessEventType::ConflictWithTeammate,
            -1.0,
            42,
            ctx,
            45,
        );
        assert!(
            added_other,
            "cooldown must be keyed per-partner so a new teammate's first incident is recorded"
        );
    }
}
