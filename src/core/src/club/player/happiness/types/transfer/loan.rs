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
    /// The spell is over and the parent club has read the record: how
    /// many games he played, how he played in them, and what that is
    /// worth. Emitted once, on the way home. Carries the whole spell —
    /// see [`LoanSpellVerdict`].
    LoanSpellReviewed,
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
            LoanEventKind::LoanSpellReviewed => "loan_kind_spell_reviewed",
        }
    }
}

/// What the parent club makes of a finished loan spell.
///
/// Two things decide it, and they are the two things a loan is for:
/// **did he play**, and **how did he play**. Neither is read as a raw
/// count — a fortnight's cover and a full season are not the same
/// denominator — so the minutes axis is the share of the borrower's
/// matches he actually started, and the form axis is his rating against
/// the level he was playing at. A spell too short to judge says so
/// rather than inventing a verdict from three appearances.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoanSpellVerdict {
    /// Played nearly every week and played well. The loan did exactly
    /// what it was sent to do.
    Standout,
    /// A proper run of games at a decent standard. The season the loan
    /// was arranged for.
    Successful,
    /// He played, and the football was unremarkable either way.
    Steady,
    /// He was there, but he was not in the team. Whatever the reason,
    /// the club got no football out of it.
    Peripheral,
    /// A real sample of matches, and he was visibly below the standard
    /// of them.
    Struggled,
    /// Too little of it happened to draw a conclusion — a short spell,
    /// an injury, a January recall.
    Inconclusive,
}

impl LoanSpellVerdict {
    pub fn as_i18n_key(&self) -> &'static str {
        match self {
            LoanSpellVerdict::Standout => "loan_verdict_standout",
            LoanSpellVerdict::Successful => "loan_verdict_successful",
            LoanSpellVerdict::Steady => "loan_verdict_steady",
            LoanSpellVerdict::Peripheral => "loan_verdict_peripheral",
            LoanSpellVerdict::Struggled => "loan_verdict_struggled",
            LoanSpellVerdict::Inconclusive => "loan_verdict_inconclusive",
        }
    }

    pub fn as_token(&self) -> &'static str {
        match self {
            LoanSpellVerdict::Standout => "standout",
            LoanSpellVerdict::Successful => "successful",
            LoanSpellVerdict::Steady => "steady",
            LoanSpellVerdict::Peripheral => "peripheral",
            LoanSpellVerdict::Struggled => "struggled",
            LoanSpellVerdict::Inconclusive => "inconclusive",
        }
    }

    /// True for the verdicts a club reads as the loan having worked.
    /// Used by callers that want the judgement without the wording.
    pub fn is_positive(&self) -> bool {
        matches!(
            self,
            LoanSpellVerdict::Standout | LoanSpellVerdict::Successful
        )
    }

    /// A spell of at least this many days is long enough to be judged on
    /// its football rather than dismissed as too short to read. Below it,
    /// only a genuinely heavy match load earns a verdict — a month's
    /// emergency cover with six starts in it did tell the club something.
    const JUDGEABLE_DAYS: i64 = 90;
    /// Appearances that make even a short spell readable.
    const JUDGEABLE_APPS: u16 = 6;
    /// Starts per month of the spell that count as "in the team". A
    /// league season is roughly 38 games over ten months, so a side
    /// plays a shade under four a month: 2.6 of them is a player who
    /// starts most weeks, and under 1.0 is a player who does not get in.
    const REGULAR_STARTS_PER_MONTH: f32 = 2.6;
    const FRINGE_STARTS_PER_MONTH: f32 = 1.0;

    /// Read a finished spell.
    ///
    /// `rating` is the realistic per-position average the statistics
    /// layer already computes, so a 6.9 keeper and a 6.9 striker mean the
    /// same thing here. `days` is the length of the spell, which is what
    /// turns an appearance count into a rate.
    pub fn classify(starts: u16, apps: u16, rating: f32, days: i64) -> Self {
        // Nothing to read: he was barely there, or he was there and never
        // played enough for anyone to form a view.
        if apps < Self::JUDGEABLE_APPS && days < Self::JUDGEABLE_DAYS {
            return LoanSpellVerdict::Inconclusive;
        }

        // Starts per month, which is what "did he play" actually means
        // once the spell length is divided out. A one-month spell is
        // floored at a month so a fortnight's cover cannot look like a
        // season's worth of football off two starts.
        let months = (days as f32 / 30.4).max(1.0);
        let starts_rate = starts as f32 / months;

        if starts_rate < Self::FRINGE_STARTS_PER_MONTH {
            // He did not get in the team. A long spell of that is a
            // wasted year; a short one is simply unreadable.
            return if days >= Self::JUDGEABLE_DAYS {
                LoanSpellVerdict::Peripheral
            } else {
                LoanSpellVerdict::Inconclusive
            };
        }

        // A rating is only worth reading off a real sample. Below that,
        // the minutes are the whole story.
        if apps < Self::JUDGEABLE_APPS {
            return LoanSpellVerdict::Steady;
        }

        let regular = starts_rate >= Self::REGULAR_STARTS_PER_MONTH;
        if rating >= 7.05 && regular {
            LoanSpellVerdict::Standout
        } else if rating >= 6.75 {
            LoanSpellVerdict::Successful
        } else if rating < 6.30 {
            LoanSpellVerdict::Struggled
        } else {
            LoanSpellVerdict::Steady
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
    /// The finished spell, read on the way home. Only
    /// [`LoanEventKind::LoanSpellReviewed`] carries it — every other kind
    /// is reporting on a loan still in progress.
    ///
    /// Captured at the moment of return, because the borrower-season
    /// statistics it is read off are frozen and reset seconds later; the
    /// numbers cannot be recovered from the player afterwards.
    pub spell: Option<LoanSpellRecord>,
}

/// What a player did while he was away. Plain counts plus the verdict
/// they produced, so the renderer never has to re-derive the judgement
/// from the numbers and disagree with the event that carried it.
#[derive(Debug, Clone, Copy)]
pub struct LoanSpellRecord {
    pub verdict: LoanSpellVerdict,
    pub starts: u16,
    pub appearances: u16,
    pub goals: u16,
    pub assists: u16,
    /// Realistic per-position average rating over the spell. `None` when
    /// he never played, which is not the same as having played badly.
    pub average_rating: Option<f32>,
    pub days: u16,
}

impl LoanSpellRecord {
    pub fn new(starts: u16, appearances: u16, goals: u16, assists: u16, rating: f32, days: i64) -> Self {
        let days = days.clamp(0, u16::MAX as i64) as u16;
        LoanSpellRecord {
            verdict: LoanSpellVerdict::classify(starts, appearances, rating, days as i64),
            starts,
            appearances,
            goals,
            assists,
            // A rating attached to nobody having played is a 0.0 that
            // renders as a damning "0.00 average" — there is no rating
            // to report, so report none.
            average_rating: (appearances > 0 && rating > 0.0).then_some(rating),
            days,
        }
    }
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
            spell: None,
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
    pub fn with_spell(mut self, spell: LoanSpellRecord) -> Self {
        self.spell = Some(spell);
        self
    }
}

#[cfg(test)]
mod tests {
    use super::LoanSpellVerdict;

    /// A season on loan playing every week and playing well is the
    /// verdict the whole loan was arranged to produce.
    #[test]
    fn a_full_season_played_well_reads_as_a_standout_spell() {
        assert_eq!(
            LoanSpellVerdict::classify(32, 36, 7.21, 320),
            LoanSpellVerdict::Standout
        );
    }

    /// The same rating off a bench career is not a good loan. The club
    /// sent him out to play, and he did not play.
    #[test]
    fn a_good_rating_off_three_starts_is_not_a_successful_loan() {
        assert_eq!(
            LoanSpellVerdict::classify(3, 11, 7.30, 320),
            LoanSpellVerdict::Peripheral
        );
    }

    /// A real run of games clearly below the standard of them.
    #[test]
    fn a_regular_who_played_badly_reads_as_a_struggle() {
        assert_eq!(
            LoanSpellVerdict::classify(24, 28, 6.05, 300),
            LoanSpellVerdict::Struggled
        );
    }

    /// Six weeks of emergency cover tells nobody anything, however the
    /// six weeks went — the sample is the problem, not the football.
    #[test]
    fn a_spell_too_short_to_read_says_so() {
        assert_eq!(
            LoanSpellVerdict::classify(4, 4, 7.40, 40),
            LoanSpellVerdict::Inconclusive
        );
        assert_eq!(
            LoanSpellVerdict::classify(1, 2, 5.90, 45),
            LoanSpellVerdict::Inconclusive
        );
    }

    /// …but a short spell he played every game of did tell them
    /// something, and the appearance count is what says so.
    #[test]
    fn a_short_spell_he_played_every_game_of_is_still_readable() {
        assert_eq!(
            LoanSpellVerdict::classify(8, 8, 7.10, 60),
            LoanSpellVerdict::Standout
        );
    }

    /// The rate is per month, so a half-season regular and a full-season
    /// regular reach the same verdict off very different raw counts.
    #[test]
    fn the_minutes_axis_is_a_rate_not_a_count() {
        let half = LoanSpellVerdict::classify(15, 17, 6.85, 150);
        let full = LoanSpellVerdict::classify(30, 34, 6.85, 300);

        assert_eq!(half, LoanSpellVerdict::Successful);
        assert_eq!(half, full);
    }
}
