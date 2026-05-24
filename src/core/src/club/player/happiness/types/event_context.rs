use super::{
    CareerDesireEventContext, ContractEventContext, InjuryRecoveryEventContext,
    LeadershipEventContext, LifeSimulationDesireContext, LoanEventContext,
    ManagerInteractionEventContext, MatchPerformanceEventContext, MatchSelectionContext,
    MediaFanEventContext, NationalTeamEventContext, PersonalAdaptationEventContext,
    RecognitionEventContext, RegulationEventContext, RoleStatusEventContext, SeasonOutcomeContext,
    SupportEventContext, TeammateConflictContext, TrainingEventContext, TransferInterestContext,
};

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
    // ── Transfer-environment evidence (weak↔elite, star↔weak) ───
    /// Player just joined a top-tier club (dest rep ≥ 7500). Pairs
    /// with the `TopClubOpportunity` / `OverawedByEliteClub` events.
    JoinedEliteClub,
    /// Player's current ability sits visibly below the destination
    /// squad's expected level. Drives `OverawedByEliteClub` framing.
    BelowSquadStandard,
    /// Player's current ability sits visibly above the destination
    /// squad's expected level. Drives `TooGoodForLevel` framing.
    AboveSquadStandard,
    /// Position depth at the destination blocks the new arrival's
    /// minutes path. Pairs with `RolePathBlockedAtEliteClub`.
    BlockedByDepth,
    /// Sub-standard coaching / training facilities at the new club —
    /// a star arriving from a richer setup notices the gap.
    TrainingLevelGap,
    /// Large transfer fee or high reputation creates fan-pressure
    /// expectations. Pairs with `FanExpectationBurden`.
    HighFeePressure,
}

impl HappinessEventEvidence {
    pub fn as_i18n_key(&self) -> &'static str {
        match self {
            HappinessEventEvidence::StrongExistingBond => "evidence_strong_existing_bond",
            HappinessEventEvidence::AlreadyStrainedRelationship => {
                "evidence_already_strained_relationship"
            }
            HappinessEventEvidence::WeakExistingBond => "evidence_weak_existing_bond",
            HappinessEventEvidence::SamePositionCompetition => "evidence_same_position_competition",
            HappinessEventEvidence::SimilarSquadStatusCompetition => {
                "evidence_similar_squad_status_competition"
            }
            HappinessEventEvidence::LowTrust => "evidence_low_trust",
            HappinessEventEvidence::LowFriendship => "evidence_low_friendship",
            HappinessEventEvidence::LowProfessionalRespect => "evidence_low_professional_respect",
            HappinessEventEvidence::HighProfessionalRespect => "evidence_high_professional_respect",
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
            HappinessEventEvidence::JoinedEliteClub => "evidence_joined_elite_club",
            HappinessEventEvidence::BelowSquadStandard => "evidence_below_squad_standard",
            HappinessEventEvidence::AboveSquadStandard => "evidence_above_squad_standard",
            HappinessEventEvidence::BlockedByDepth => "evidence_blocked_by_depth",
            HappinessEventEvidence::TrainingLevelGap => "evidence_training_level_gap",
            HappinessEventEvidence::HighFeePressure => "evidence_high_fee_pressure",
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
    /// Teammate-conflict explanation payload. Drives
    /// `ConflictWithTeammate` rendering with a concrete reason +
    /// location so the headline reads as a specific football moment.
    pub teammate_conflict_context: Option<TeammateConflictContext>,
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
    /// Career-desire payload — return-home, European-competition or
    /// Copa-Libertadores ambition mood (and their positive
    /// counterparts). Drives the renderer for `WantsReturnHome` /
    /// `WantsEuropeanCompetition` / `WantsCopaLibertadores` /
    /// `HomeReturnOpportunity` / `ContinentalAmbitionSatisfied`.
    pub career_desire_context: Option<CareerDesireEventContext>,
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
    /// Life-simulation desire payload — family events, leave requests,
    /// role / pressure preferences, NT visibility, loyalty refusals.
    /// Renderer keys off `kind` to pick the right narrative.
    pub life_simulation_desire_context: Option<LifeSimulationDesireContext>,
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
            teammate_conflict_context: None,
            contract_context: None,
            injury_context: None,
            match_performance_context: None,
            role_status_context: None,
            national_team_context: None,
            leadership_context: None,
            media_fan_context: None,
            personal_adaptation_context: None,
            career_desire_context: None,
            loan_context: None,
            recognition_context: None,
            season_outcome_context: None,
            regulation_context: None,
            life_simulation_desire_context: None,
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

    pub fn with_teammate_conflict_context(mut self, ctx: TeammateConflictContext) -> Self {
        self.teammate_conflict_context = Some(ctx);
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

    pub fn with_career_desire_context(mut self, ctx: CareerDesireEventContext) -> Self {
        self.career_desire_context = Some(ctx);
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

    pub fn with_life_simulation_desire_context(mut self, ctx: LifeSimulationDesireContext) -> Self {
        self.life_simulation_desire_context = Some(ctx);
        self
    }

    /// Returns the number of specialized payload contexts attached to
    /// this event. Specialized payloads are mutually exclusive at the
    /// modelling level — an event is *either* a selection event, *or*
    /// a transfer-interest event, etc. — so this should never exceed 1.
    /// Used by tests as a soft invariant on emit-site code.
    pub fn specialized_payload_count(&self) -> usize {
        let mut n = 0;
        if self.selection_context.is_some() {
            n += 1;
        }
        if self.support_context.is_some() {
            n += 1;
        }
        if self.transfer_interest_context.is_some() {
            n += 1;
        }
        if self.training_context.is_some() {
            n += 1;
        }
        if self.manager_interaction_context.is_some() {
            n += 1;
        }
        if self.teammate_conflict_context.is_some() {
            n += 1;
        }
        if self.contract_context.is_some() {
            n += 1;
        }
        if self.injury_context.is_some() {
            n += 1;
        }
        if self.match_performance_context.is_some() {
            n += 1;
        }
        if self.role_status_context.is_some() {
            n += 1;
        }
        if self.national_team_context.is_some() {
            n += 1;
        }
        if self.leadership_context.is_some() {
            n += 1;
        }
        if self.media_fan_context.is_some() {
            n += 1;
        }
        if self.personal_adaptation_context.is_some() {
            n += 1;
        }
        if self.career_desire_context.is_some() {
            n += 1;
        }
        if self.loan_context.is_some() {
            n += 1;
        }
        if self.recognition_context.is_some() {
            n += 1;
        }
        if self.season_outcome_context.is_some() {
            n += 1;
        }
        if self.regulation_context.is_some() {
            n += 1;
        }
        n
    }
}
