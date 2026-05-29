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
    /// Parent club / player is formally pushing to recall the loan
    /// because it is failing. Stronger than `LoanRecallDiscussed`.
    LoanRecallRequested,
    /// Aggregated monthly development warning — the loan is not helping
    /// the player progress (minutes, role, level, training, no progress).
    LoanDevelopmentConcern,
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
            LoanEventKind::LoanRecallRequested => "loan_kind_recall_requested",
            LoanEventKind::LoanDevelopmentConcern => "loan_kind_development_concern",
        }
    }
}

/// Why a parent club / player is pushing to recall a loan. Closed enum so
/// the renderer copy stays bounded. The first implementation focuses on
/// `InsufficientMinutes`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoanConcernReason {
    InsufficientMinutes,
    WrongRole,
    LevelTooHigh,
    LevelTooLow,
    TrainingQuality,
    ParentSquadNeed,
}

impl LoanConcernReason {
    pub fn as_i18n_key(&self) -> &'static str {
        match self {
            LoanConcernReason::InsufficientMinutes => "loan_reason_insufficient_minutes",
            LoanConcernReason::WrongRole => "loan_reason_wrong_role",
            LoanConcernReason::LevelTooHigh => "loan_reason_level_too_high",
            LoanConcernReason::LevelTooLow => "loan_reason_level_too_low",
            LoanConcernReason::TrainingQuality => "loan_reason_training_quality",
            LoanConcernReason::ParentSquadNeed => "loan_reason_parent_squad_need",
        }
    }
}

/// Why a young player's loan is judged to be failing development. Several
/// of these can be present at once — the emit site pushes every signal it
/// observed and the renderer surfaces the strongest.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoanDevelopmentConcernReason {
    InsufficientMinutes,
    WrongRole,
    LevelMismatch,
    PoorTrainingEnvironment,
    NoProgress,
    PoorMatchPerformance,
}

impl LoanDevelopmentConcernReason {
    pub fn as_i18n_key(&self) -> &'static str {
        match self {
            LoanDevelopmentConcernReason::InsufficientMinutes => {
                "loan_dev_reason_insufficient_minutes"
            }
            LoanDevelopmentConcernReason::WrongRole => "loan_dev_reason_wrong_role",
            LoanDevelopmentConcernReason::LevelMismatch => "loan_dev_reason_level_mismatch",
            LoanDevelopmentConcernReason::PoorTrainingEnvironment => {
                "loan_dev_reason_poor_training"
            }
            LoanDevelopmentConcernReason::NoProgress => "loan_dev_reason_no_progress",
            LoanDevelopmentConcernReason::PoorMatchPerformance => {
                "loan_dev_reason_poor_performance"
            }
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
    /// Appearances the loanee was expected to have by now.
    pub expected_apps_by_now: Option<u16>,
    /// Appearances the loanee actually has.
    pub actual_apps: Option<u16>,
    /// Shortfall (`expected - actual`), pre-computed for the renderer.
    pub deficit_apps: Option<u16>,
    /// Days elapsed since the loan started.
    pub loan_days_elapsed: Option<u16>,
    /// Single recall reason (recall-request events).
    pub recall_reason: Option<LoanConcernReason>,
    /// Development-failure reasons (development-concern events).
    pub development_reasons: Vec<LoanDevelopmentConcernReason>,
}

impl LoanEventContext {
    pub fn new(kind: LoanEventKind) -> Self {
        Self {
            kind,
            parent_club_id: None,
            loan_club_id: None,
            minutes_share: None,
            permanent_option_present: false,
            expected_apps_by_now: None,
            actual_apps: None,
            deficit_apps: None,
            loan_days_elapsed: None,
            recall_reason: None,
            development_reasons: Vec::new(),
        }
    }

    pub fn with_parent_club(mut self, id: u32) -> Self {
        self.parent_club_id = Some(id);
        self
    }
    pub fn with_loan_club(mut self, id: u32) -> Self {
        self.loan_club_id = Some(id);
        self
    }
    pub fn with_minutes_share(mut self, share: f32) -> Self {
        self.minutes_share = Some(share);
        self
    }
    pub fn with_permanent_option(mut self, present: bool) -> Self {
        self.permanent_option_present = present;
        self
    }
    pub fn with_expected_apps(mut self, apps: u16) -> Self {
        self.expected_apps_by_now = Some(apps);
        self
    }
    pub fn with_actual_apps(mut self, apps: u16) -> Self {
        self.actual_apps = Some(apps);
        self
    }
    pub fn with_deficit_apps(mut self, deficit: u16) -> Self {
        self.deficit_apps = Some(deficit);
        self
    }
    pub fn with_loan_days_elapsed(mut self, days: u16) -> Self {
        self.loan_days_elapsed = Some(days);
        self
    }
    pub fn with_recall_reason(mut self, reason: LoanConcernReason) -> Self {
        self.recall_reason = Some(reason);
        self
    }
    pub fn with_development_reason(mut self, reason: LoanDevelopmentConcernReason) -> Self {
        if !self.development_reasons.contains(&reason) {
            self.development_reasons.push(reason);
        }
        self
    }
}
