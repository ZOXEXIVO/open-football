//! What the goalkeeping coach actually says.
//!
//! The backroom in this sim was a set of multipliers: a coach raised
//! training gains, a scout widened a shortlist, and nobody ever said
//! anything to anybody. A goalkeeping coach's job is almost entirely the
//! opposite — he watches four squads' worth of keepers all week and then
//! tells the manager what he thinks, and the manager either acts on it or
//! doesn't.
//!
//! So the department's output is a small closed catalogue of things a real
//! goalkeeping coach says, each attached to the keeper it is about. Nothing
//! here moves a player; consumers read the advice and decide.

/// A place in the goalkeeping hierarchy — declared, not re-derived.
///
/// The point of declaring it is stability. A manager names his number one
/// and then plays him: he does not re-rank his keepers by form and freshness
/// every Saturday, which is what an unconstrained argmax over ability does.
/// Ordered from most to least central so a consumer can compare standing
/// without a lookup table.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum KeeperTier {
    /// The number one. He plays the league.
    NumberOne,
    /// The deputy. The cup is his, he covers without the team dropping,
    /// and at a well-run club he is close enough to push.
    Deputy,
    /// Third choice: emergency cover and, at most clubs, the senior voice
    /// in the goalkeeping group. A real role rather than a stalled career.
    Third,
    /// A young keeper on the senior pathway — training with the first team,
    /// travelling with the squad, taking minutes where the stakes allow.
    Pathway,
    /// An academy keeper the department is watching but not yet promoting.
    Academy,
    /// Out of the picture here.
    Surplus,
}

impl KeeperTier {
    /// Tiers that commit the club to giving the keeper competitive minutes.
    pub fn promises_minutes(self) -> bool {
        matches!(self, Self::NumberOne | Self::Deputy | Self::Pathway)
    }

    /// Tiers that belong to the senior goalkeeping group.
    pub fn is_senior_group(self) -> bool {
        matches!(self, Self::NumberOne | Self::Deputy | Self::Third)
    }

    /// Share of the season's competitive matches the tier is planned to
    /// start. A real keeper room is lopsided on purpose — the number one
    /// plays almost everything, the deputy owns the cup, and the rest is
    /// a handful of games — so the shares are lopsided too.
    pub fn planned_share(self) -> f32 {
        match self {
            Self::NumberOne => 0.76,
            Self::Deputy => 0.18,
            Self::Third => 0.03,
            Self::Pathway => 0.03,
            Self::Academy | Self::Surplus => 0.0,
        }
    }

    /// Stable key for the events feed and the UI.
    pub fn as_i18n_key(self) -> &'static str {
        match self {
            Self::NumberOne => "keeper_tier_number_one",
            Self::Deputy => "keeper_tier_deputy",
            Self::Third => "keeper_tier_third",
            Self::Pathway => "keeper_tier_pathway",
            Self::Academy => "keeper_tier_academy",
            Self::Surplus => "keeper_tier_surplus",
        }
    }
}

/// One thing the goalkeeping coach has to say.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeeperAdvice {
    /// "He should be your number one." Raised when the department wants the
    /// declared order changed, not merely when someone scores higher this
    /// week.
    MakeHimNumberOne,
    /// "Give him the cup." The deputy's competition, decided before the
    /// season rather than weighed tie by tie.
    HandHimTheCup,
    /// "Put him on the bench with the first team." The first rung: a youth
    /// keeper travels and trains with the seniors before he plays for them.
    NameHimOnTheBench,
    /// "Start him." The nomination the manager actually has to weigh —
    /// raised for a keeper the department believes in even when he is not
    /// yet the best keeper at the club. A boy who never plays never
    /// becomes one.
    GiveHimASeniorStart,
    /// "He needs a season of men's football somewhere else."
    LoanHimOutForMinutes,
    /// "Leave him where he is. He is not ready and the games he is getting
    /// are the right games."
    KeepHimDeveloping,
    /// "Keep him. He holds the room together." The third keeper's real job
    /// at most clubs, and the reason a contented veteran is not deadwood.
    KeepHimAsTheSeniorVoice,
    /// "Start planning for the number one's replacement." Raised on the
    /// succession clock, not on one bad afternoon.
    OpenTheSuccession,
    /// "There is nobody behind the number one." Club-scoped.
    SignACredibleDeputy,
    /// "There is nobody coming." Club-scoped.
    SignAKeeperForTheFuture,
    /// "We need an experienced head as third choice." Club-scoped.
    SignAnExperiencedThird,
    /// "He is not going to play here again."
    TimeToMoveHimOn,
}

impl KeeperAdvice {
    /// Advice about the club's shape rather than about one man.
    pub fn is_club_scoped(self) -> bool {
        matches!(
            self,
            Self::SignACredibleDeputy
                | Self::SignAKeeperForTheFuture
                | Self::SignAnExperiencedThird
        )
    }

    /// Stable key for the events feed and the UI.
    pub fn as_i18n_key(self) -> &'static str {
        match self {
            Self::MakeHimNumberOne => "gk_advice_make_him_number_one",
            Self::HandHimTheCup => "gk_advice_hand_him_the_cup",
            Self::NameHimOnTheBench => "gk_advice_name_him_on_the_bench",
            Self::GiveHimASeniorStart => "gk_advice_give_him_a_senior_start",
            Self::LoanHimOutForMinutes => "gk_advice_loan_him_out_for_minutes",
            Self::KeepHimDeveloping => "gk_advice_keep_him_developing",
            Self::KeepHimAsTheSeniorVoice => "gk_advice_keep_him_as_the_senior_voice",
            Self::OpenTheSuccession => "gk_advice_open_the_succession",
            Self::SignACredibleDeputy => "gk_advice_sign_a_credible_deputy",
            Self::SignAKeeperForTheFuture => "gk_advice_sign_a_keeper_for_the_future",
            Self::SignAnExperiencedThird => "gk_advice_sign_an_experienced_third",
            Self::TimeToMoveHimOn => "gk_advice_time_to_move_him_on",
        }
    }
}

/// How hard the department is pushing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum KeeperUrgency {
    /// Worth saying, no more than that.
    Noted,
    /// He wants this dealt with this season.
    Pressing,
    /// He will keep saying it until somebody acts.
    Urgent,
}

/// One recommendation, with the keeper it is about.
#[derive(Debug, Clone, Copy)]
pub struct KeeperRecommendation {
    pub advice: KeeperAdvice,
    /// The keeper concerned. `None` for club-scoped advice about the shape
    /// of the room.
    pub player_id: Option<u32>,
    pub urgency: KeeperUrgency,
}

impl KeeperRecommendation {
    pub fn about(advice: KeeperAdvice, player_id: u32, urgency: KeeperUrgency) -> Self {
        KeeperRecommendation {
            advice,
            player_id: Some(player_id),
            urgency,
        }
    }

    pub fn club(advice: KeeperAdvice, urgency: KeeperUrgency) -> Self {
        KeeperRecommendation {
            advice,
            player_id: None,
            urgency,
        }
    }
}

/// How pressing the succession behind the number one is.
///
/// Read off the incumbent's age against the keeper curve and shifted a
/// step when there is nobody in the building who could replace him. The
/// point is that the question opens years before the answer is needed —
/// which is exactly the part the sim was missing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default)]
pub enum KeeperSuccession {
    /// The number one has years left and the club need not think about it.
    #[default]
    Settled,
    /// Worth having somebody in mind.
    Watch,
    /// The heir should be in the building and getting minutes.
    Pressing,
    /// The club is one injury from a problem it cannot solve.
    Critical,
}

impl KeeperSuccession {
    pub fn escalated(self) -> Self {
        match self {
            Self::Settled => Self::Watch,
            Self::Watch => Self::Pressing,
            _ => Self::Critical,
        }
    }

    pub fn as_i18n_key(self) -> &'static str {
        match self {
            Self::Settled => "keeper_succession_settled",
            Self::Watch => "keeper_succession_watch",
            Self::Pressing => "keeper_succession_pressing",
            Self::Critical => "keeper_succession_critical",
        }
    }
}
