//! Structured contexts for the manager-relationship arc:
//! private-talk requests, club-direction concern / encouragement, big-
//! match selection trust / drops, substitution frustration, and new-
//! signing competition. Each is a small payload tagged onto the
//! `HappinessEventContext` envelope at emit time so the renderer can
//! describe the cause, evidence, and follow-up of the event without
//! guessing from the headline alone.

use super::ManagerInteractionTopic;
use crate::club::player::contract::PlayerSquadStatus;

// ────────────────────────────────────────────────────────────────
// PrivateTalkRequestContext
// ────────────────────────────────────────────────────────────────

/// What drove the player to formally request a private conversation
/// with the manager. Picked at emit time so the renderer can name the
/// core grievance rather than the generic "wanted a chat" line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PrivateTalkReason {
    /// Role / minutes — playing-time frustration is dominant.
    PlayingTime,
    /// Contract — wage / length / status dispute building.
    Contract,
    /// Transfer status — wants out, or wants assurances on staying.
    TransferStatus,
    /// Captaincy or squad status (KeyPlayer demotion, armband stripped).
    CaptaincyOrStatus,
    /// Tactical role — the way he's being used, not pure non-selection.
    TacticalRole,
    /// Manager relationship is in a bad spot — repeated criticism,
    /// broken promises, lost trust.
    ManagerRelationship,
}

impl PrivateTalkReason {
    pub fn as_i18n_key(&self) -> &'static str {
        match self {
            PrivateTalkReason::PlayingTime => "private_talk_reason_playing_time",
            PrivateTalkReason::Contract => "private_talk_reason_contract",
            PrivateTalkReason::TransferStatus => "private_talk_reason_transfer_status",
            PrivateTalkReason::CaptaincyOrStatus => "private_talk_reason_captaincy_or_status",
            PrivateTalkReason::TacticalRole => "private_talk_reason_tactical_role",
            PrivateTalkReason::ManagerRelationship => "private_talk_reason_manager_relationship",
        }
    }

    /// Map a private-talk reason to the analogous manager-interaction
    /// topic — so the rendering layer can reuse the topic copy already
    /// translated for `ManagerCriticism` / `ManagerPraise` exchanges.
    pub fn as_manager_topic(&self) -> ManagerInteractionTopic {
        match self {
            PrivateTalkReason::PlayingTime => ManagerInteractionTopic::PlayingTime,
            PrivateTalkReason::TacticalRole => ManagerInteractionTopic::Tactical,
            PrivateTalkReason::Contract => ManagerInteractionTopic::PromiseFollowUp,
            PrivateTalkReason::CaptaincyOrStatus => ManagerInteractionTopic::RoleClarification,
            PrivateTalkReason::TransferStatus => ManagerInteractionTopic::Other,
            PrivateTalkReason::ManagerRelationship => ManagerInteractionTopic::Attitude,
        }
    }
}

#[derive(Debug, Clone)]
pub struct PrivateTalkRequestContext {
    pub reason: PrivateTalkReason,
    /// Manager trust 0..100 at emit time — drives tone of the headline.
    pub trust_in_manager: Option<f32>,
    /// Player morale at emit time — distinguishes a chronic-unhappy
    /// player's talk from an otherwise content player's one-off concern.
    pub current_morale: Option<f32>,
    /// True when this is the second / further request in a short window.
    /// The renderer escalates the outlook line.
    pub repeated_request: bool,
}

impl PrivateTalkRequestContext {
    pub fn new(reason: PrivateTalkReason) -> Self {
        Self {
            reason,
            trust_in_manager: None,
            current_morale: None,
            repeated_request: false,
        }
    }

    pub fn with_trust(mut self, trust: f32) -> Self {
        self.trust_in_manager = Some(trust);
        self
    }

    pub fn with_morale(mut self, morale: f32) -> Self {
        self.current_morale = Some(morale);
        self
    }

    pub fn with_repeated(mut self, repeated: bool) -> Self {
        self.repeated_request = repeated;
        self
    }
}

// ────────────────────────────────────────────────────────────────
// ClubDirectionContext
// ────────────────────────────────────────────────────────────────

/// Whether the club's direction signal is positive or negative. Both
/// flavours share the same payload — only the polarity differs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClubDirectionKind {
    /// Player is concerned about where the club is heading.
    Concern,
    /// Player is encouraged by the squad investment / direction.
    Encouragement,
}

impl ClubDirectionKind {
    pub fn as_i18n_key(&self) -> &'static str {
        match self {
            ClubDirectionKind::Concern => "club_direction_kind_concern",
            ClubDirectionKind::Encouragement => "club_direction_kind_encouragement",
        }
    }
}

/// Concrete evidence that triggered the club-direction event.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClubDirectionEvidence {
    /// Key player was sold without replacement.
    KeyPlayerSoldUnreplaced,
    /// Squad's effective ability has dropped season-over-season.
    SquadQualityWeakened,
    /// Sustained league underperformance vs stated ambition.
    PoorLeagueForm,
    /// Board missed continental qualification or relegation safety.
    AmbitionGapWithBoard,
    /// New signing arrived who meaningfully improves a unit.
    MeaningfulSigningArrived,
    /// Multiple meaningful signings in the same window.
    MultipleSigningsImproveSquad,
    /// Net spend visibly positive — board is investing.
    BoardInvestmentVisible,
    /// Player's own role / status was elevated alongside the investment.
    PlayerInvolvedInProject,
    /// High personality ambition amplifies the reaction.
    HighAmbition,
    /// High professional / influence weight — senior pro reaction.
    HighInfluence,
}

impl ClubDirectionEvidence {
    pub fn as_i18n_key(&self) -> &'static str {
        match self {
            ClubDirectionEvidence::KeyPlayerSoldUnreplaced => {
                "club_direction_evidence_key_player_sold"
            }
            ClubDirectionEvidence::SquadQualityWeakened => {
                "club_direction_evidence_squad_weakened"
            }
            ClubDirectionEvidence::PoorLeagueForm => "club_direction_evidence_poor_league_form",
            ClubDirectionEvidence::AmbitionGapWithBoard => {
                "club_direction_evidence_ambition_gap"
            }
            ClubDirectionEvidence::MeaningfulSigningArrived => {
                "club_direction_evidence_meaningful_signing"
            }
            ClubDirectionEvidence::MultipleSigningsImproveSquad => {
                "club_direction_evidence_multiple_signings"
            }
            ClubDirectionEvidence::BoardInvestmentVisible => {
                "club_direction_evidence_board_investment"
            }
            ClubDirectionEvidence::PlayerInvolvedInProject => {
                "club_direction_evidence_player_involved"
            }
            ClubDirectionEvidence::HighAmbition => "club_direction_evidence_high_ambition",
            ClubDirectionEvidence::HighInfluence => "club_direction_evidence_high_influence",
        }
    }
}

#[derive(Debug, Clone)]
pub struct ClubDirectionContext {
    pub kind: ClubDirectionKind,
    /// Net signings count this window (positive = investment, negative =
    /// fire sale). `None` when the emit site can't supply a number.
    pub net_signings_window: Option<i8>,
    /// Player id of the most relevant new arrival / departure tied to
    /// the event — lets the renderer name the player when known.
    pub focal_player_id: Option<u32>,
    pub evidence: Vec<ClubDirectionEvidence>,
}

impl ClubDirectionContext {
    pub fn new(kind: ClubDirectionKind) -> Self {
        Self {
            kind,
            net_signings_window: None,
            focal_player_id: None,
            evidence: Vec::new(),
        }
    }

    pub fn with_net_signings(mut self, n: i8) -> Self {
        self.net_signings_window = Some(n);
        self
    }

    pub fn with_focal_player(mut self, id: u32) -> Self {
        self.focal_player_id = Some(id);
        self
    }

    pub fn with_evidence(mut self, ev: ClubDirectionEvidence) -> Self {
        if !self.evidence.contains(&ev) {
            self.evidence.push(ev);
        }
        self
    }
}

// ────────────────────────────────────────────────────────────────
// BigMatchSelectionContext
// ────────────────────────────────────────────────────────────────

/// What flavour of "big match" the selection / drop refers to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BigMatchKind {
    Derby,
    CupFinal,
    TitleDecider,
    PromotionDecider,
    RelegationDecider,
    ContinentalKnockout,
    NationalCupSemiOrLater,
}

impl BigMatchKind {
    pub fn as_i18n_key(&self) -> &'static str {
        match self {
            BigMatchKind::Derby => "big_match_kind_derby",
            BigMatchKind::CupFinal => "big_match_kind_cup_final",
            BigMatchKind::TitleDecider => "big_match_kind_title_decider",
            BigMatchKind::PromotionDecider => "big_match_kind_promotion_decider",
            BigMatchKind::RelegationDecider => "big_match_kind_relegation_decider",
            BigMatchKind::ContinentalKnockout => "big_match_kind_continental_knockout",
            BigMatchKind::NationalCupSemiOrLater => "big_match_kind_national_cup_late",
        }
    }
}

/// Whether the player was trusted with the start or dropped from the
/// expected XI for the big match.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BigMatchDecision {
    StartedTrusted,
    BenchedUnexpectedly,
}

impl BigMatchDecision {
    pub fn as_i18n_key(&self) -> &'static str {
        match self {
            BigMatchDecision::StartedTrusted => "big_match_decision_started_trusted",
            BigMatchDecision::BenchedUnexpectedly => "big_match_decision_benched",
        }
    }
}

#[derive(Debug, Clone)]
pub struct BigMatchSelectionContext {
    pub kind: BigMatchKind,
    pub decision: BigMatchDecision,
    /// Opponent club id when known. Lets the renderer name the rival.
    pub opponent_club_id: Option<u32>,
    /// Player's squad status at the time of the call.
    pub squad_status: Option<PlayerSquadStatus>,
    /// `true` if the player wore the armband for this fixture.
    pub was_captain: bool,
    /// `true` if this is a young or fringe player whose call meant more
    /// because of their stage — the renderer amplifies the headline.
    pub is_young_or_fringe: bool,
    /// `true` when player was on a clear hot streak (excellent form
    /// EMA) heading into the fixture. Amplifies a `BenchedForBigMatch`
    /// hit because the call defied form.
    pub recent_hot_form: bool,
    /// Match importance 0..1 (matches `MatchSelectionContext` scale).
    pub match_importance: f32,
}

impl BigMatchSelectionContext {
    pub fn new(kind: BigMatchKind, decision: BigMatchDecision) -> Self {
        Self {
            kind,
            decision,
            opponent_club_id: None,
            squad_status: None,
            was_captain: false,
            is_young_or_fringe: false,
            recent_hot_form: false,
            match_importance: 1.0,
        }
    }

    pub fn with_opponent(mut self, club_id: u32) -> Self {
        self.opponent_club_id = Some(club_id);
        self
    }

    pub fn with_squad_status(mut self, status: PlayerSquadStatus) -> Self {
        self.squad_status = Some(status);
        self
    }

    pub fn with_captain(mut self, flag: bool) -> Self {
        self.was_captain = flag;
        self
    }

    pub fn with_young_or_fringe(mut self, flag: bool) -> Self {
        self.is_young_or_fringe = flag;
        self
    }

    pub fn with_hot_form(mut self, flag: bool) -> Self {
        self.recent_hot_form = flag;
        self
    }

    pub fn with_match_importance(mut self, importance: f32) -> Self {
        self.match_importance = importance.clamp(0.0, 1.0);
        self
    }
}

// ────────────────────────────────────────────────────────────────
// SubstitutionFrustrationContext
// ────────────────────────────────────────────────────────────────

/// What flavour of substitution frustration drove the event. The match
/// engine knows why a player was hooked; this enum lets the renderer
/// describe it specifically.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SubstitutionFrustrationKind {
    /// Repeated early hooks across several recent matches.
    RepeatedEarlyHook,
    /// Pulled off while playing well (high match rating at the time).
    HookedWhilePlayingWell,
    /// Removed in a big match before expected.
    RemovedInBigMatchEarly,
    /// Tactical positional swap that the player resented.
    TacticalSwapResented,
}

impl SubstitutionFrustrationKind {
    pub fn as_i18n_key(&self) -> &'static str {
        match self {
            SubstitutionFrustrationKind::RepeatedEarlyHook => "sub_frustration_kind_repeated_early",
            SubstitutionFrustrationKind::HookedWhilePlayingWell => {
                "sub_frustration_kind_hooked_while_playing_well"
            }
            SubstitutionFrustrationKind::RemovedInBigMatchEarly => {
                "sub_frustration_kind_removed_big_match"
            }
            SubstitutionFrustrationKind::TacticalSwapResented => {
                "sub_frustration_kind_tactical_swap"
            }
        }
    }
}

#[derive(Debug, Clone)]
pub struct SubstitutionFrustrationContext {
    pub kind: SubstitutionFrustrationKind,
    /// Minute the player came off, 0–120. `None` if unknown.
    pub minute_off: Option<u8>,
    /// Match rating at the time of the substitution.
    pub match_rating_at_sub: Option<f32>,
    /// Number of early substitutions in the recent window (for kind =
    /// RepeatedEarlyHook); 0 when not relevant.
    pub recent_early_hooks: u8,
    /// `true` if the substitution happened in a big match (derby / cup
    /// final / continental knockout).
    pub is_big_match: bool,
}

impl SubstitutionFrustrationContext {
    pub fn new(kind: SubstitutionFrustrationKind) -> Self {
        Self {
            kind,
            minute_off: None,
            match_rating_at_sub: None,
            recent_early_hooks: 0,
            is_big_match: false,
        }
    }

    pub fn with_minute(mut self, m: u8) -> Self {
        self.minute_off = Some(m);
        self
    }

    pub fn with_match_rating(mut self, rating: f32) -> Self {
        self.match_rating_at_sub = Some(rating);
        self
    }

    pub fn with_recent_early_hooks(mut self, n: u8) -> Self {
        self.recent_early_hooks = n;
        self
    }

    pub fn with_big_match(mut self, flag: bool) -> Self {
        self.is_big_match = flag;
        self
    }
}

// ────────────────────────────────────────────────────────────────
// NewSigningThreatContext
// ────────────────────────────────────────────────────────────────

/// Why a new signing is perceived as a threat. Multiple may apply at
/// once; emit site picks the dominant one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NewSigningThreatReason {
    /// New signing plays in the same primary position.
    SamePosition,
    /// New signing was bought with the same intended squad-status tier.
    SimilarSquadStatus,
    /// New signing has visibly higher ability — likely to overtake.
    HigherAbility,
    /// New signing arrived on a much bigger contract — wage threat.
    LargerWageDeal,
    /// New signing is significantly younger — long-term threat.
    YoungerAndHighPotential,
    /// Player was already short on minutes — any rival is a threat.
    AlreadyFringe,
}

impl NewSigningThreatReason {
    pub fn as_i18n_key(&self) -> &'static str {
        match self {
            NewSigningThreatReason::SamePosition => "new_signing_threat_same_position",
            NewSigningThreatReason::SimilarSquadStatus => "new_signing_threat_similar_status",
            NewSigningThreatReason::HigherAbility => "new_signing_threat_higher_ability",
            NewSigningThreatReason::LargerWageDeal => "new_signing_threat_larger_wage",
            NewSigningThreatReason::YoungerAndHighPotential => "new_signing_threat_younger",
            NewSigningThreatReason::AlreadyFringe => "new_signing_threat_already_fringe",
        }
    }
}

#[derive(Debug, Clone)]
pub struct NewSigningThreatContext {
    /// The rival player id — the renderer names him in the headline.
    pub rival_player_id: u32,
    pub primary_reason: NewSigningThreatReason,
    /// Player's own squad status when the signing landed.
    pub player_squad_status: Option<PlayerSquadStatus>,
    /// Rival's intended squad status, if known.
    pub rival_squad_status: Option<PlayerSquadStatus>,
    /// Player age — older players feel positional threat more sharply.
    pub player_age: Option<u8>,
    /// Rival age — youthful threats hit older incumbents harder.
    pub rival_age: Option<u8>,
    /// All applicable reasons (for the detail line).
    pub all_reasons: Vec<NewSigningThreatReason>,
}

impl NewSigningThreatContext {
    pub fn new(rival_player_id: u32, primary_reason: NewSigningThreatReason) -> Self {
        let all_reasons = vec![primary_reason];
        Self {
            rival_player_id,
            primary_reason,
            player_squad_status: None,
            rival_squad_status: None,
            player_age: None,
            rival_age: None,
            all_reasons,
        }
    }

    pub fn with_player_status(mut self, status: PlayerSquadStatus) -> Self {
        self.player_squad_status = Some(status);
        self
    }

    pub fn with_rival_status(mut self, status: PlayerSquadStatus) -> Self {
        self.rival_squad_status = Some(status);
        self
    }

    pub fn with_player_age(mut self, age: u8) -> Self {
        self.player_age = Some(age);
        self
    }

    pub fn with_rival_age(mut self, age: u8) -> Self {
        self.rival_age = Some(age);
        self
    }

    pub fn with_reason(mut self, reason: NewSigningThreatReason) -> Self {
        if !self.all_reasons.contains(&reason) {
            self.all_reasons.push(reason);
        }
        self
    }
}
