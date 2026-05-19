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
}
