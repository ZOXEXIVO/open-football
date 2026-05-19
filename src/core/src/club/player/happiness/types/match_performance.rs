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
            MatchPerformanceKind::AnsweredCriticsWithPerformance => {
                "match_perf_kind_answered_critics"
            }
            MatchPerformanceKind::CostlyErrorUnderPressure => {
                "match_perf_kind_costly_error_pressure"
            }
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
            MatchPerformanceEvidence::DecisiveContribution => {
                "match_perf_evidence_decisive_contribution"
            }
            MatchPerformanceEvidence::DerbyFixture => "match_perf_evidence_derby",
            MatchPerformanceEvidence::CupTie => "match_perf_evidence_cup_tie",
            MatchPerformanceEvidence::LeagueDecider => "match_perf_evidence_league_decider",
            MatchPerformanceEvidence::SubstituteAppearance => "match_perf_evidence_substitute",
            MatchPerformanceEvidence::PlayedFullMinutes => "match_perf_evidence_full_minutes",
            MatchPerformanceEvidence::PlayedAfterCriticism => "match_perf_evidence_after_criticism",
            MatchPerformanceEvidence::HighPressurePersonality => {
                "match_perf_evidence_high_pressure_personality"
            }
            MatchPerformanceEvidence::LowPressurePersonality => {
                "match_perf_evidence_low_pressure_personality"
            }
            MatchPerformanceEvidence::ImportantMatchTemperament => {
                "match_perf_evidence_important_match_temperament"
            }
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

    pub fn with_rating(mut self, rating: f32) -> Self {
        self.rating = Some(rating);
        self
    }
    pub fn with_goals(mut self, goals: u8) -> Self {
        self.goals = goals;
        self
    }
    pub fn with_assists(mut self, assists: u8) -> Self {
        self.assists = assists;
        self
    }
    pub fn with_minutes(mut self, minutes: u16) -> Self {
        self.minutes = minutes;
        self
    }
    pub fn with_team_won(mut self, won: bool) -> Self {
        self.team_won = Some(won);
        self
    }
    pub fn with_goal_margin(mut self, margin: i8) -> Self {
        self.goal_margin = Some(margin);
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
    pub fn with_opponent(mut self, club_id: u32) -> Self {
        self.opponent_club_id = Some(club_id);
        self
    }

    pub fn with_evidence(mut self, evidence: MatchPerformanceEvidence) -> Self {
        if !self.evidence.contains(&evidence) {
            self.evidence.push(evidence);
        }
        self
    }
}
