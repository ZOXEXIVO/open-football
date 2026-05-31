use crate::PlayerFieldPositionGroup;

/// What flavour of career-desire mood the player is signalling.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CareerDesireKind {
    ReturnHomeAfterPoorAdaptation,
    EuropeanCompetitionAmbition,
    CopaLibertadoresAmbition,
    /// Ambitious star wants the board to strengthen the squad before
    /// committing his future. Drives `WantsStrongerSquad`.
    StrongerSquadAmbition,
    /// Elite player wants to play for a genuine title challenger. Drives
    /// `WantsTitleChallenge`.
    TitleChallengeAmbition,
    /// Sentimental move to a favourite/boyhood club that does not also
    /// clear the source-aware DreamMove gates. Distinct from
    /// `ReturnHomeAfterPoorAdaptation` because the player isn't escaping
    /// a failed adaptation — they're answering a heritage pull. Drives
    /// the `HomeReturnOpportunity` event when emitted by the favourite-
    /// club permanent-signing branch.
    FavoriteClubHomecoming,
}

impl CareerDesireKind {
    pub fn as_i18n_key(&self) -> &'static str {
        match self {
            CareerDesireKind::ReturnHomeAfterPoorAdaptation => "career_desire_kind_return_home",
            CareerDesireKind::EuropeanCompetitionAmbition => {
                "career_desire_kind_european_competition"
            }
            CareerDesireKind::CopaLibertadoresAmbition => "career_desire_kind_copa_libertadores",
            CareerDesireKind::StrongerSquadAmbition => "career_desire_kind_stronger_squad",
            CareerDesireKind::TitleChallengeAmbition => "career_desire_kind_title_challenge",
            CareerDesireKind::FavoriteClubHomecoming => "career_desire_kind_favorite_club_homecoming",
        }
    }
}

/// Concrete signals the desire detector latched onto. Closed enum so the
/// renderer copy stays bounded; emit sites push the atoms that justified
/// the mood and the renderer surfaces the most informative one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CareerDesireEvidence {
    /// Player is at a club whose country sits on a different continent
    /// from the player's nationality.
    DifferentContinent,
    /// Player does not speak the local language of his current club
    /// country.
    NoLocalLanguage,
    /// Player's `adaptability` personality is low (≤ 8).
    LowAdaptability,
    /// No same-nationality / shared-language teammates in the squad.
    NoCompatriotSupport,
    /// Recent adaptation_score was poor (sub-40 band).
    PoorAdaptationScore,
    /// Personality `ambition` is high (≥ 14).
    HighAmbition,
    /// Current club is not in or near a continental qualification path.
    CurrentClubNotContinental,
    /// A favourite / former / home-country destination is concretely
    /// linked.
    HomeOrFavouriteLink,
    /// Repeated `FeelingIsolated` events over the recent window.
    RepeatedIsolation,
    /// `club_fit` morale axis is meaningfully negative.
    LowClubFit,
    // ── Squad-ambition (WantsStrongerSquad) ─────────────────────
    /// Squad's average ability sits below the player's own level.
    SquadQualityBelowPlayerLevel,
    /// A top squad player was sold recently and not replaced.
    KeyPlayerSold,
    /// The unit around the player (his position group) is thin / weak.
    WeakDepthInPlayerUnit,
    /// Concern that the board's ambition does not match the player's.
    BoardAmbitionConcern,
    // ── Title-challenge (WantsTitleChallenge) ───────────────────
    /// Current club is not a realistic title contender this season.
    CurrentClubNotTitleContender,
    /// Repeated top-four finishes without ever challenging for the title.
    RepeatedTopFourWithoutTitleChallenge,
    /// Player is clearly above the club's overall level.
    PlayerAboveClubLevel,
    /// Player is in the prime of his career — the window to win matters.
    PrimeCareerWindow,
}

impl CareerDesireEvidence {
    pub fn as_i18n_key(&self) -> &'static str {
        match self {
            CareerDesireEvidence::DifferentContinent => {
                "career_desire_evidence_different_continent"
            }
            CareerDesireEvidence::NoLocalLanguage => "career_desire_evidence_no_local_language",
            CareerDesireEvidence::LowAdaptability => "career_desire_evidence_low_adaptability",
            CareerDesireEvidence::NoCompatriotSupport => {
                "career_desire_evidence_no_compatriot_support"
            }
            CareerDesireEvidence::PoorAdaptationScore => {
                "career_desire_evidence_poor_adaptation_score"
            }
            CareerDesireEvidence::HighAmbition => "career_desire_evidence_high_ambition",
            CareerDesireEvidence::CurrentClubNotContinental => {
                "career_desire_evidence_current_club_not_continental"
            }
            CareerDesireEvidence::HomeOrFavouriteLink => {
                "career_desire_evidence_home_or_favourite_link"
            }
            CareerDesireEvidence::RepeatedIsolation => "career_desire_evidence_repeated_isolation",
            CareerDesireEvidence::LowClubFit => "career_desire_evidence_low_club_fit",
            CareerDesireEvidence::SquadQualityBelowPlayerLevel => {
                "career_desire_evidence_squad_quality_below_player_level"
            }
            CareerDesireEvidence::KeyPlayerSold => "career_desire_evidence_key_player_sold",
            CareerDesireEvidence::WeakDepthInPlayerUnit => "career_desire_evidence_weak_depth",
            CareerDesireEvidence::BoardAmbitionConcern => {
                "career_desire_evidence_board_ambition_concern"
            }
            CareerDesireEvidence::CurrentClubNotTitleContender => {
                "career_desire_evidence_current_club_not_title_contender"
            }
            CareerDesireEvidence::RepeatedTopFourWithoutTitleChallenge => {
                "career_desire_evidence_repeated_top_four"
            }
            CareerDesireEvidence::PlayerAboveClubLevel => {
                "career_desire_evidence_player_above_club_level"
            }
            CareerDesireEvidence::PrimeCareerWindow => "career_desire_evidence_prime_career_window",
        }
    }
}

/// Structured payload describing why the player is signalling a
/// career-desire mood (return home / European / Libertadores). Filled
/// in at emit time so the renderer can compose a contextual headline +
/// reason instead of guessing from the event-type enum alone.
#[derive(Debug, Clone)]
pub struct CareerDesireEventContext {
    pub kind: CareerDesireKind,
    /// Days at current club at emit time. 0 if unknown.
    pub days_at_club: u32,
    /// Adaptation score 0..100 if available.
    pub adaptation_score: Option<f32>,
    /// Closed-set evidence atoms that justified the mood.
    pub evidence: Vec<CareerDesireEvidence>,
    // ── Squad-ambition fields (WantsStrongerSquad) ──────────────
    /// Squad's average current ability, if measured.
    pub squad_average_ability: Option<u8>,
    /// Player's own current ability, for the comparison.
    pub player_ability: Option<u8>,
    /// The weakest unit around the player, if identified.
    pub weakest_unit: Option<PlayerFieldPositionGroup>,
    // ── Title-challenge fields (WantsTitleChallenge) ────────────
    /// League position at emit time.
    pub league_position: Option<u8>,
    /// Points behind the leader (negative = ahead).
    pub points_off_leader: Option<i16>,
    /// Club reputation at emit time.
    pub club_reputation: Option<u16>,
}

impl CareerDesireEventContext {
    pub fn new(kind: CareerDesireKind) -> Self {
        Self {
            kind,
            days_at_club: 0,
            adaptation_score: None,
            evidence: Vec::new(),
            squad_average_ability: None,
            player_ability: None,
            weakest_unit: None,
            league_position: None,
            points_off_leader: None,
            club_reputation: None,
        }
    }

    pub fn with_days_at_club(mut self, days: u32) -> Self {
        self.days_at_club = days;
        self
    }

    pub fn with_adaptation_score(mut self, score: f32) -> Self {
        self.adaptation_score = Some(score);
        self
    }

    pub fn with_evidence(mut self, evidence: CareerDesireEvidence) -> Self {
        if !self.evidence.contains(&evidence) {
            self.evidence.push(evidence);
        }
        self
    }

    pub fn with_squad_average_ability(mut self, ability: u8) -> Self {
        self.squad_average_ability = Some(ability);
        self
    }

    pub fn with_player_ability(mut self, ability: u8) -> Self {
        self.player_ability = Some(ability);
        self
    }

    pub fn with_weakest_unit(mut self, unit: PlayerFieldPositionGroup) -> Self {
        self.weakest_unit = Some(unit);
        self
    }

    pub fn with_league_position(mut self, position: u8) -> Self {
        self.league_position = Some(position);
        self
    }

    pub fn with_points_off_leader(mut self, points: i16) -> Self {
        self.points_off_leader = Some(points);
        self
    }

    pub fn with_club_reputation(mut self, reputation: u16) -> Self {
        self.club_reputation = Some(reputation);
        self
    }
}
