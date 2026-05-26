use crate::club::player::contract::PlayerSquadStatus;

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
            TransferInterestStage::AgentSoundingOut => "transfer_interest_stage_agent_sounding_out",
            TransferInterestStage::LooseRumour => "transfer_interest_stage_loose_rumour",
            TransferInterestStage::ConcreteInterest => "transfer_interest_stage_concrete_interest",
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
    /// Approach concretely offers European competition football and
    /// the player's current club cannot. Drives the stronger
    /// `Excited` / `WantsTalks` reaction when the matching desire
    /// mood is active.
    EuropeanCompetitionOpportunity,
    /// Approach concretely offers Copa Libertadores football for a
    /// South American heritage player.
    CopaLibertadoresOpportunity,
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
            TransferInterestKind::FormerClubReturn => "transfer_interest_kind_former_club_return",
            TransferInterestKind::FavoriteClubInterest => {
                "transfer_interest_kind_favorite_club_interest"
            }
            TransferInterestKind::BigLeagueOpportunity => {
                "transfer_interest_kind_big_league_opportunity"
            }
            TransferInterestKind::LoanDevelopment => "transfer_interest_kind_loan_development",
            TransferInterestKind::EscapeRoute => "transfer_interest_kind_escape_route",
            TransferInterestKind::Speculative => "transfer_interest_kind_speculative",
            TransferInterestKind::EuropeanCompetitionOpportunity => {
                "transfer_interest_kind_european_competition_opportunity"
            }
            TransferInterestKind::CopaLibertadoresOpportunity => {
                "transfer_interest_kind_copa_libertadores_opportunity"
            }
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
            TransferSportingFit::BetterPlayingTime => "transfer_sporting_fit_better_playing_time",
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
    /// Approach offers a path to European competition (UCL / UEL /
    /// UECL). Stronger than a generic `BiggerLeague` for ambitious
    /// players whose current club has no continental qualification.
    EuropeanCompetitionOpportunity,
    /// Approach offers a path to Copa Libertadores football. Specific
    /// to South-American heritage / South-American clubs.
    CopaLibertadoresOpportunity,
    /// Move would relieve a documented return-home / homesickness
    /// desire on the player.
    ReturnHomeRelief,
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
            TransferInterestEvidence::EuropeanCompetitionOpportunity => {
                "transfer_interest_evidence_european_competition_opportunity"
            }
            TransferInterestEvidence::CopaLibertadoresOpportunity => {
                "transfer_interest_evidence_copa_libertadores_opportunity"
            }
            TransferInterestEvidence::ReturnHomeRelief => {
                "transfer_interest_evidence_return_home_relief"
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

    pub fn with_playing_time_change(mut self, improve: bool, reduce: bool) -> Self {
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
