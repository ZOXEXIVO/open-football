//! The goal catalog — what a player can want, as data rather than
//! control flow.
//!
//! This replaces three parallel enums that all described the same thing.
//! `TransferRequestReason` (12), `CareerDesireKind` (11) and
//! `LifeSimulationDesireKind` (18) each carried their own detector,
//! cooldown and escalation rule, so adding a forty-second want meant
//! touching all three layers. Here a want is one [`GoalSpec`] row: how
//! fast it fades, what strength it takes before he says it out loud,
//! what it takes before he demands, what it competes with, and which way
//! it points if he acts on it.
//!
//! Adding a goal is a variant, a row, an i18n key and a test. That is
//! the extensibility contract, made structural.

use super::evidence::GoalDomain;

/// What a player wants. Closed catalog so every renderer, escalation
/// rule and test keys off the variant by name.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum GoalKind {
    /// Placeholder for a default-constructed slot. Never stored.
    #[default]
    None,

    // ── Where he plays: moving up, out, or on ───────────────────
    /// He has outgrown this club and wants a bigger one.
    StepUpToABiggerClub,
    /// His league is a ceiling, whatever his club's local standing.
    PlayInAStrongerLeague,
    /// European nights.
    PlayContinentalFootball,
    /// The South American equivalent — a distinct ambition with its own
    /// route and its own narrative, not a regional flavour of the above.
    PlayInLibertadores,
    /// He wants to be somewhere that actually wins the league.
    CompeteForTheTitle,
    /// He wants better players around him.
    PlayWithBetterPlayers,
    /// Nothing is wrong; he has simply been here long enough.
    FindANewChallenge,
    /// The club went down and he does not intend to go with it.
    KeepPlayingAtThisLevel,
    /// The terminus of unresolved unhappiness — out, anywhere.
    LeaveThisClub,
    /// The club he grew up supporting.
    PlayForMyBoyhoodClub,

    // ── Playing ─────────────────────────────────────────────────
    /// Seasons have gone by without a first-team shirt. The one goal
    /// that will take him to a *smaller* club to get one.
    PlayFirstTeamFootball,
    /// He has lost his place and means to take it back.
    WinBackMyPlace,
    /// Minutes are not the problem — he is being played out of position.
    PlayInMyBestRole,
    /// The loanee who wants his chance at the club that owns him.
    ProveMyselfAtMyParentClub,
    /// The loanee who would rather stay where he is playing.
    StayAtThisLoanClub,
    /// He needs games and knows he will not get them here.
    GoOutOnLoan,
    /// The man in possession, with somebody coming for the shirt. The
    /// mirror of [`GoalKind::WinBackMyPlace`], and a different state:
    /// defending a place is not the same as chasing one, and the two
    /// resolve on opposite results.
    HoldOntoMyPlace,

    // ── Getting better ──────────────────────────────────────────
    /// He has stopped improving and means to start again. Points
    /// nowhere on its own — it is answered by a better coach, a role
    /// that stretches him, or a training focus, all of which the club
    /// can give him without him going anywhere.
    KeepImproving,
    /// The coaching here has taken him as far as it can. The rung above
    /// [`GoalKind::KeepImproving`] and below wanting a bigger club — it
    /// is *still* satisfiable in place, by the club hiring somebody
    /// better, which is why it is a separate want rather than a stage of
    /// wanting out.
    WorkWithABetterCoach,

    // ── The manager, and standing ───────────────────────────────
    WinTheManagersTrust,
    BeCaptain,
    /// Not a move — permission. He wants it understood he may go if the
    /// right offer arrives.
    BeAllowedToLeave,
    /// The counter-goal. A club legend actively wants to stay, and it
    /// pushes back against every goal above.
    StayAtThisClub,
    /// The long server's want. Not to stay — he already means to stay —
    /// but to be *treated* like what he has become: the armband, terms
    /// offered before he has to ask, being spoken about as part of the
    /// place. Refused, it is one of the sharpest grievances in the game,
    /// because it cannot be bought off with money.
    BecomeAClubLegend,

    // ── Money ───────────────────────────────────────────────────
    BePaidWhatImWorth,
    /// Length and stability rather than headline wage.
    SecureMyFuture,
    GetAReleaseClause,
    /// He has decided not to re-sign. Every month he plays on is
    /// leverage, and at the end of it he walks for nothing and picks
    /// where he goes. A want that looks like inaction from outside and
    /// is in fact the most deliberate decision a player ever makes.
    RunDownMyContract,

    // ── Life ────────────────────────────────────────────────────
    GoHome,
    SettleMyFamily,
    LearnTheLanguage,
    FindAMentor,
    /// The cauldron is too hot; he would drop down to get out of it.
    EscapeThePressure,

    // ── Achievement and the end of it ───────────────────────────
    WinATrophy,
    GetIntoTheNationalSquad,
    EndTheDrought,
    RetireOnMyTerms,
    MoveIntoCoaching,

    // ═══ What a manager wants ═══════════════════════════════════
    // Same ladder, same meaning: `Active` is the rung where a want
    // silently shapes every decision, `Voiced` is where the press
    // hears about it.

    // ── This job ────────────────────────────────────────────────
    /// Survival. Its urgency comes from board trust, not the calendar.
    KeepThisJob,
    WinSomethingHere,
    /// The relegation-fight variant of [`GoalKind::KeepThisJob`].
    SurviveTheSeason,
    /// A manager's [`GoalKind::StepUpToABiggerClub`].
    GetABiggerJob,
    /// The terminus — a manager's [`GoalKind::LeaveThisClub`].
    GetOutOfHere,
    TakeANationalJob,
    /// Formed by a sacking, and pointed at the club that did it. The
    /// one goal in the catalog that only exists because memory does.
    ProveThemWrong,

    // ── The people above him ────────────────────────────────────
    /// Voiced, this is the classic public plea for signings.
    BeBackedInTheMarket,
    BeGivenTime,

    // ── The squad ───────────────────────────────────────────────
    /// Fires when the board starts listening to offers.
    KeepMyBestPlayer,
    /// One target, held across a window.
    SignThePlayerIWant,
    /// A new manager wanting to replace a team he inherited.
    GetMyOwnSquad,
    RestoreOrderInTheRoom,

    // ── Himself ─────────────────────────────────────────────────
    RetireFromTheGame,
}

/// Which way a goal points if he acts on it. Read by the deliberation
/// layer, and by [`GoalStack::wants_to_leave`].
///
/// [`GoalStack::wants_to_leave`]: super::stack::GoalStack::wants_to_leave
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GoalDirection {
    /// Satisfying it means leaving.
    Leave,
    /// Satisfying it means staying.
    Stay,
    /// It can be satisfied either way.
    Neutral,
}

/// Bit index of a [`GoalKind`] in a [`GoalMask`]. `None` is not
/// representable — masks describe real goals only.
type GoalBit = u64;

/// A set of goals, for the mutual-exclusion rules.
///
/// One bit per [`GoalKind`], so the catalog cannot grow past 64 kinds
/// without widening this. `every_kind_fits_the_mask` is the guard —
/// without it the sixty-fifth want would silently alias the first.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct GoalMask(GoalBit);

impl GoalMask {
    pub const EMPTY: GoalMask = GoalMask(0);

    pub const fn of(kinds: &[GoalKind]) -> Self {
        let mut bits: GoalBit = 0;
        let mut index = 0;
        while index < kinds.len() {
            bits |= 1 << kinds[index].bit();
            index += 1;
        }
        GoalMask(bits)
    }

    #[inline]
    pub fn contains(self, kind: GoalKind) -> bool {
        self.0 & (1 << kind.bit()) != 0
    }

    #[inline]
    pub fn is_empty(self) -> bool {
        self.0 == 0
    }
}

/// How one goal behaves: how it fades, what it takes to voice or press
/// it, when he gives up on it, and what it rules out.
#[derive(Debug, Clone, Copy)]
pub struct GoalSpec {
    pub domain: GoalDomain,
    /// Which way it points.
    pub direction: GoalDirection,
    /// Fraction of remaining strength shed per month with no
    /// reinforcement. A want nobody feeds fades; how fast is the
    /// difference between a mood and a conviction.
    pub decay_per_month: f32,
    /// Pressure at which he stops keeping it to himself. Below this the
    /// goal still shapes every decision he makes — it is simply silent,
    /// which is what `big_stage_inclination` did for one want and this
    /// does for all of them.
    pub voice_at: f32,
    /// Pressure at which he formally demands — a transfer request, an
    /// ultimatum. Always above `voice_at`.
    pub press_at: f32,
    /// Months of no progress after which he lets it go. `None` for the
    /// goals a player carries until they resolve.
    pub abandon_after_months: Option<u16>,
    /// Goals that cannot be held alongside this one. Asserting a goal
    /// weakens everything in its mask — a man who has decided to stay
    /// is not simultaneously agitating to leave.
    pub competes_with: GoalMask,
}

impl GoalSpec {
    /// An ordinary want: fades at a moderate rate, voiced when it gets
    /// strong, pressed when it dominates.
    pub const fn ordinary(domain: GoalDomain, direction: GoalDirection) -> Self {
        GoalSpec {
            domain,
            direction,
            decay_per_month: 0.10,
            voice_at: 0.55,
            press_at: 0.80,
            abandon_after_months: None,
            competes_with: GoalMask::EMPTY,
        }
    }

    /// A want he is slow to voice and slower to demand — the ones a
    /// professional keeps to himself for a long time.
    pub const fn private(domain: GoalDomain, direction: GoalDirection) -> Self {
        GoalSpec {
            voice_at: 0.70,
            press_at: 0.90,
            ..Self::ordinary(domain, direction)
        }
    }

    /// A grievance: it fades slowly and he says it sooner. Being wronged
    /// does not work like wanting something.
    pub const fn grievance(domain: GoalDomain, direction: GoalDirection) -> Self {
        GoalSpec {
            decay_per_month: 0.05,
            voice_at: 0.45,
            press_at: 0.72,
            ..Self::ordinary(domain, direction)
        }
    }

    /// A passing want — fades fast and he gives up on it inside a couple
    /// of seasons.
    pub const fn fleeting(domain: GoalDomain, direction: GoalDirection) -> Self {
        GoalSpec {
            decay_per_month: 0.22,
            abandon_after_months: Some(18),
            ..Self::ordinary(domain, direction)
        }
    }

    pub const fn competing(self, competes_with: GoalMask) -> Self {
        GoalSpec {
            competes_with,
            ..self
        }
    }

    pub const fn abandoned_after(self, months: u16) -> Self {
        GoalSpec {
            abandon_after_months: Some(months),
            ..self
        }
    }
}

/// Everything that points a player out of his club. Held by the
/// stay-goals so a decision to stay pushes back on all of them at once.
const WANTS_OUT: GoalMask = GoalMask::of(&[
    GoalKind::StepUpToABiggerClub,
    GoalKind::PlayInAStrongerLeague,
    GoalKind::PlayContinentalFootball,
    GoalKind::PlayInLibertadores,
    GoalKind::CompeteForTheTitle,
    GoalKind::FindANewChallenge,
    GoalKind::KeepPlayingAtThisLevel,
    GoalKind::LeaveThisClub,
    GoalKind::PlayFirstTeamFootball,
    GoalKind::GoHome,
    GoalKind::EscapeThePressure,
    GoalKind::RunDownMyContract,
]);

/// Everything that keeps him where he is.
const WANTS_TO_STAY: GoalMask = GoalMask::of(&[
    GoalKind::StayAtThisClub,
    GoalKind::BecomeAClubLegend,
    GoalKind::WinBackMyPlace,
    GoalKind::HoldOntoMyPlace,
    GoalKind::WinTheManagersTrust,
    GoalKind::BeCaptain,
]);

/// The three rungs of wanting to get better, in order. Each one
/// **supersedes** the one below rather than competing with it: a man who
/// has concluded the coaching here cannot help him has stopped simply
/// wanting to improve, and a man who wants out has stopped waiting for a
/// new coach. Holding two at once would double-count the same
/// frustration.
const WANTS_A_BETTER_COACH: GoalMask = GoalMask::of(&[GoalKind::WorkWithABetterCoach]);
const JUST_WANTS_TO_IMPROVE: GoalMask = GoalMask::of(&[GoalKind::KeepImproving]);

/// Everything that points a manager out of his job.
const MANAGER_WANTS_OUT: GoalMask = GoalMask::of(&[
    GoalKind::GetABiggerJob,
    GoalKind::GetOutOfHere,
    GoalKind::TakeANationalJob,
    GoalKind::RetireFromTheGame,
]);

/// Everything that keeps him in it. Left alone, ambition churns
/// managers every season; these are the counterweight, and the reason a
/// manager mid-build resists an approach he would have taken a year
/// earlier.
const MANAGER_WANTS_TO_STAY: GoalMask = GoalMask::of(&[
    GoalKind::KeepThisJob,
    GoalKind::WinSomethingHere,
    GoalKind::SurviveTheSeason,
    GoalKind::GetMyOwnSquad,
    GoalKind::BeGivenTime,
    GoalKind::RestoreOrderInTheRoom,
]);

impl GoalKind {
    /// Bit index for [`GoalMask`]. Stable, dense, and distinct from the
    /// discriminant only in that `None` is excluded.
    pub const fn bit(self) -> u32 {
        self as u32
    }

    /// The catalog. One row per kind.
    pub fn spec(self) -> GoalSpec {
        use GoalDirection as Dir;
        use GoalDomain as D;
        use GoalSpec as S;

        match self {
            GoalKind::None => S::ordinary(D::Career, Dir::Neutral),

            // Where he plays
            GoalKind::StepUpToABiggerClub => {
                S::ordinary(D::Career, Dir::Leave).competing(WANTS_TO_STAY)
            }
            GoalKind::PlayInAStrongerLeague => {
                S::ordinary(D::Career, Dir::Leave).competing(WANTS_TO_STAY)
            }
            GoalKind::PlayContinentalFootball => {
                S::ordinary(D::Career, Dir::Leave).competing(WANTS_TO_STAY)
            }
            GoalKind::PlayInLibertadores => {
                S::ordinary(D::Career, Dir::Leave).competing(WANTS_TO_STAY)
            }
            GoalKind::CompeteForTheTitle => {
                S::private(D::Career, Dir::Leave).competing(WANTS_TO_STAY)
            }
            GoalKind::PlayWithBetterPlayers => S::private(D::Career, Dir::Neutral),
            // A restlessness he has to sit with for a while before it
            // becomes anything — and which he abandons if it passes.
            GoalKind::FindANewChallenge => S::private(D::Career, Dir::Leave)
                .competing(WANTS_TO_STAY)
                .abandoned_after(36),
            GoalKind::KeepPlayingAtThisLevel => {
                S::grievance(D::Career, Dir::Leave).competing(WANTS_TO_STAY)
            }
            // The terminus of unresolved unhappiness. Slowest to fade of
            // anything in the catalog: a man who has decided he wants out
            // does not quietly stop wanting it.
            GoalKind::LeaveThisClub => GoalSpec {
                decay_per_month: 0.04,
                ..S::grievance(D::Career, Dir::Leave).competing(WANTS_TO_STAY)
            },
            GoalKind::PlayForMyBoyhoodClub => GoalSpec {
                decay_per_month: 0.02,
                ..S::private(D::Social, Dir::Leave)
            },

            // Playing
            GoalKind::PlayFirstTeamFootball => {
                S::grievance(D::Competitive, Dir::Leave).competing(WANTS_TO_STAY)
            }
            GoalKind::WinBackMyPlace => S::ordinary(D::Competitive, Dir::Stay).competing(WANTS_OUT),
            GoalKind::PlayInMyBestRole => S::grievance(D::Professional, Dir::Neutral),
            GoalKind::ProveMyselfAtMyParentClub => S::ordinary(D::Career, Dir::Neutral),
            GoalKind::StayAtThisLoanClub => S::ordinary(D::Career, Dir::Neutral),
            GoalKind::GoOutOnLoan => S::fleeting(D::Career, Dir::Leave),
            // Defending a shirt is quieter than chasing one. He does not
            // announce it and he never demands anything over it — he
            // just trains harder and plays like a man who can hear
            // footsteps.
            GoalKind::HoldOntoMyPlace => GoalSpec {
                voice_at: 0.80,
                press_at: 0.99,
                ..S::ordinary(D::Competitive, Dir::Stay).competing(WANTS_OUT)
            },

            // Getting better. Neither of these is a grievance and
            // neither points anywhere on its own, which is the whole
            // point of having them: they are the two rungs a club still
            // gets to answer before a player starts looking at the door.
            GoalKind::KeepImproving => GoalSpec {
                decay_per_month: 0.08,
                ..S::private(D::Career, Dir::Neutral).competing(WANTS_A_BETTER_COACH)
            },
            GoalKind::WorkWithABetterCoach => GoalSpec {
                decay_per_month: 0.06,
                ..S::private(D::Career, Dir::Neutral).competing(JUST_WANTS_TO_IMPROVE)
            },

            // The manager, and standing
            GoalKind::WinTheManagersTrust => {
                S::ordinary(D::Professional, Dir::Stay).competing(WANTS_OUT)
            }
            GoalKind::BeCaptain => S::private(D::Professional, Dir::Stay),
            GoalKind::BeAllowedToLeave => S::ordinary(D::Professional, Dir::Leave),
            // The loyalist's anchor. Never fades on its own.
            GoalKind::StayAtThisClub => GoalSpec {
                decay_per_month: 0.02,
                ..S::private(D::Social, Dir::Stay).competing(WANTS_OUT)
            },

            // The long server's want. It fades even slower than the
            // decision to stay that produced it, and he says it out loud
            // sooner than he would say anything about money — because
            // what he is asking for is not money.
            GoalKind::BecomeAClubLegend => GoalSpec {
                decay_per_month: 0.02,
                voice_at: 0.50,
                press_at: 0.85,
                ..S::ordinary(D::Social, Dir::Stay).competing(WANTS_OUT)
            },

            // Money
            GoalKind::BePaidWhatImWorth => S::grievance(D::Financial, Dir::Neutral),
            GoalKind::SecureMyFuture => S::private(D::Financial, Dir::Neutral),
            GoalKind::GetAReleaseClause => S::fleeting(D::Financial, Dir::Neutral),
            // A decision, not a grievance: it hardly fades, it is never
            // said out loud until it is a fact, and it is answered only
            // by the calendar. `press_at` above 1.0 makes that structural
            // — there is no demand to make, because he already has what
            // he wants simply by waiting.
            GoalKind::RunDownMyContract => GoalSpec {
                decay_per_month: 0.03,
                voice_at: 0.75,
                press_at: 0.95,
                ..S::private(D::Financial, Dir::Leave).competing(WANTS_TO_STAY)
            },

            // Life
            GoalKind::GoHome => GoalSpec {
                decay_per_month: 0.06,
                ..S::private(D::Social, Dir::Leave).competing(WANTS_TO_STAY)
            },
            GoalKind::SettleMyFamily => S::ordinary(D::Social, Dir::Neutral),
            GoalKind::LearnTheLanguage => S::fleeting(D::Social, Dir::Stay),
            GoalKind::FindAMentor => S::fleeting(D::Social, Dir::Stay),
            GoalKind::EscapeThePressure => {
                S::private(D::Social, Dir::Leave).competing(WANTS_TO_STAY)
            }

            // Achievement, and the end of it
            GoalKind::WinATrophy => S::private(D::Competitive, Dir::Neutral),
            GoalKind::GetIntoTheNationalSquad => S::private(D::Competitive, Dir::Neutral),
            GoalKind::EndTheDrought => S::fleeting(D::Competitive, Dir::Stay),
            GoalKind::RetireOnMyTerms => S::private(D::Career, Dir::Neutral),
            GoalKind::MoveIntoCoaching => S::private(D::Career, Dir::Neutral),

            // This job. A manager keeps almost all of this to himself —
            // `private` is the default here rather than the exception,
            // because saying any of it out loud is itself an event.
            GoalKind::KeepThisJob => GoalSpec {
                decay_per_month: 0.06,
                ..S::private(D::Management, Dir::Stay).competing(MANAGER_WANTS_OUT)
            },
            GoalKind::WinSomethingHere => {
                S::private(D::Management, Dir::Stay).competing(MANAGER_WANTS_OUT)
            }
            // Season-bounded by construction: it is answered, one way or
            // the other, by May.
            GoalKind::SurviveTheSeason => S::ordinary(D::Management, Dir::Stay)
                .competing(MANAGER_WANTS_OUT)
                .abandoned_after(12),
            GoalKind::GetABiggerJob => {
                S::private(D::Management, Dir::Leave).competing(MANAGER_WANTS_TO_STAY)
            }
            GoalKind::GetOutOfHere => GoalSpec {
                decay_per_month: 0.04,
                ..S::grievance(D::Management, Dir::Leave).competing(MANAGER_WANTS_TO_STAY)
            },
            GoalKind::TakeANationalJob => GoalSpec {
                decay_per_month: 0.04,
                ..S::private(D::Management, Dir::Leave).competing(MANAGER_WANTS_TO_STAY)
            },
            // Slowest fade in the manager rows. Being sacked is not
            // something a man quietly stops minding.
            GoalKind::ProveThemWrong => GoalSpec {
                decay_per_month: 0.03,
                ..S::grievance(D::Management, Dir::Neutral)
            },

            // The people above him
            GoalKind::BeBackedInTheMarket => S::grievance(D::Boardroom, Dir::Neutral),
            GoalKind::BeGivenTime => {
                S::private(D::Boardroom, Dir::Stay).competing(MANAGER_WANTS_OUT)
            }

            // The squad
            GoalKind::KeepMyBestPlayer => S::grievance(D::Squad, Dir::Neutral).abandoned_after(9),
            GoalKind::SignThePlayerIWant => S::fleeting(D::Squad, Dir::Neutral).abandoned_after(12),
            GoalKind::GetMyOwnSquad => S::ordinary(D::Squad, Dir::Stay)
                .competing(MANAGER_WANTS_OUT)
                .abandoned_after(36),
            GoalKind::RestoreOrderInTheRoom => {
                S::grievance(D::Squad, Dir::Stay).competing(MANAGER_WANTS_OUT)
            }

            // Himself
            GoalKind::RetireFromTheGame => GoalSpec {
                decay_per_month: 0.03,
                ..S::private(D::Welfare, Dir::Neutral)
            },
        }
    }

    #[inline]
    pub fn direction(self) -> GoalDirection {
        self.spec().direction
    }

    #[inline]
    pub fn domain(self) -> GoalDomain {
        self.spec().domain
    }

    /// True when acting on this goal means leaving the club.
    #[inline]
    pub fn points_away(self) -> bool {
        self.direction() == GoalDirection::Leave
    }

    pub fn as_i18n_key(self) -> &'static str {
        match self {
            GoalKind::None => "mind_goal_none",
            GoalKind::StepUpToABiggerClub => "mind_goal_step_up",
            GoalKind::PlayInAStrongerLeague => "mind_goal_stronger_league",
            GoalKind::PlayContinentalFootball => "mind_goal_continental",
            GoalKind::PlayInLibertadores => "mind_goal_libertadores",
            GoalKind::CompeteForTheTitle => "mind_goal_title_challenge",
            GoalKind::PlayWithBetterPlayers => "mind_goal_better_squad",
            GoalKind::FindANewChallenge => "mind_goal_new_challenge",
            GoalKind::KeepPlayingAtThisLevel => "mind_goal_keep_level",
            GoalKind::LeaveThisClub => "mind_goal_leave_club",
            GoalKind::PlayForMyBoyhoodClub => "mind_goal_boyhood_club",
            GoalKind::PlayFirstTeamFootball => "mind_goal_first_team_football",
            GoalKind::WinBackMyPlace => "mind_goal_win_back_place",
            GoalKind::PlayInMyBestRole => "mind_goal_best_role",
            GoalKind::ProveMyselfAtMyParentClub => "mind_goal_prove_at_parent",
            GoalKind::StayAtThisLoanClub => "mind_goal_stay_on_loan",
            GoalKind::GoOutOnLoan => "mind_goal_go_on_loan",
            GoalKind::HoldOntoMyPlace => "mind_goal_hold_onto_my_place",
            GoalKind::KeepImproving => "mind_goal_keep_improving",
            GoalKind::WorkWithABetterCoach => "mind_goal_better_coach",
            GoalKind::WinTheManagersTrust => "mind_goal_managers_trust",
            GoalKind::BeCaptain => "mind_goal_captaincy",
            GoalKind::BeAllowedToLeave => "mind_goal_permission_to_leave",
            GoalKind::StayAtThisClub => "mind_goal_stay",
            GoalKind::BecomeAClubLegend => "mind_goal_club_legend",
            GoalKind::BePaidWhatImWorth => "mind_goal_fair_wage",
            GoalKind::SecureMyFuture => "mind_goal_secure_future",
            GoalKind::GetAReleaseClause => "mind_goal_release_clause",
            GoalKind::RunDownMyContract => "mind_goal_run_down_contract",
            GoalKind::GoHome => "mind_goal_go_home",
            GoalKind::SettleMyFamily => "mind_goal_settle_family",
            GoalKind::LearnTheLanguage => "mind_goal_learn_language",
            GoalKind::FindAMentor => "mind_goal_find_mentor",
            GoalKind::EscapeThePressure => "mind_goal_escape_pressure",
            GoalKind::WinATrophy => "mind_goal_win_trophy",
            GoalKind::GetIntoTheNationalSquad => "mind_goal_national_squad",
            GoalKind::EndTheDrought => "mind_goal_end_drought",
            GoalKind::RetireOnMyTerms => "mind_goal_retire",
            GoalKind::MoveIntoCoaching => "mind_goal_coaching",
            GoalKind::KeepThisJob => "mind_goal_keep_this_job",
            GoalKind::WinSomethingHere => "mind_goal_win_something_here",
            GoalKind::SurviveTheSeason => "mind_goal_survive_the_season",
            GoalKind::GetABiggerJob => "mind_goal_bigger_job",
            GoalKind::GetOutOfHere => "mind_goal_get_out_of_here",
            GoalKind::TakeANationalJob => "mind_goal_national_job",
            GoalKind::ProveThemWrong => "mind_goal_prove_them_wrong",
            GoalKind::BeBackedInTheMarket => "mind_goal_backed_in_the_market",
            GoalKind::BeGivenTime => "mind_goal_be_given_time",
            GoalKind::KeepMyBestPlayer => "mind_goal_keep_my_best_player",
            GoalKind::SignThePlayerIWant => "mind_goal_sign_the_player_i_want",
            GoalKind::GetMyOwnSquad => "mind_goal_get_my_own_squad",
            GoalKind::RestoreOrderInTheRoom => "mind_goal_restore_order",
            GoalKind::RetireFromTheGame => "mind_goal_retire_from_the_game",
        }
    }

    /// Every real kind. Held in lockstep with the enum by
    /// `all_lists_every_variant`.
    pub const ALL: &'static [GoalKind] = &[
        GoalKind::StepUpToABiggerClub,
        GoalKind::PlayInAStrongerLeague,
        GoalKind::PlayContinentalFootball,
        GoalKind::PlayInLibertadores,
        GoalKind::CompeteForTheTitle,
        GoalKind::PlayWithBetterPlayers,
        GoalKind::FindANewChallenge,
        GoalKind::KeepPlayingAtThisLevel,
        GoalKind::LeaveThisClub,
        GoalKind::PlayForMyBoyhoodClub,
        GoalKind::PlayFirstTeamFootball,
        GoalKind::WinBackMyPlace,
        GoalKind::PlayInMyBestRole,
        GoalKind::ProveMyselfAtMyParentClub,
        GoalKind::StayAtThisLoanClub,
        GoalKind::GoOutOnLoan,
        GoalKind::HoldOntoMyPlace,
        GoalKind::KeepImproving,
        GoalKind::WorkWithABetterCoach,
        GoalKind::WinTheManagersTrust,
        GoalKind::BeCaptain,
        GoalKind::BeAllowedToLeave,
        GoalKind::StayAtThisClub,
        GoalKind::BecomeAClubLegend,
        GoalKind::BePaidWhatImWorth,
        GoalKind::SecureMyFuture,
        GoalKind::GetAReleaseClause,
        GoalKind::RunDownMyContract,
        GoalKind::GoHome,
        GoalKind::SettleMyFamily,
        GoalKind::LearnTheLanguage,
        GoalKind::FindAMentor,
        GoalKind::EscapeThePressure,
        GoalKind::WinATrophy,
        GoalKind::GetIntoTheNationalSquad,
        GoalKind::EndTheDrought,
        GoalKind::RetireOnMyTerms,
        GoalKind::MoveIntoCoaching,
        GoalKind::KeepThisJob,
        GoalKind::WinSomethingHere,
        GoalKind::SurviveTheSeason,
        GoalKind::GetABiggerJob,
        GoalKind::GetOutOfHere,
        GoalKind::TakeANationalJob,
        GoalKind::ProveThemWrong,
        GoalKind::BeBackedInTheMarket,
        GoalKind::BeGivenTime,
        GoalKind::KeepMyBestPlayer,
        GoalKind::SignThePlayerIWant,
        GoalKind::GetMyOwnSquad,
        GoalKind::RestoreOrderInTheRoom,
        GoalKind::RetireFromTheGame,
    ];
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_lists_every_variant() {
        assert_eq!(
            GoalKind::ALL.len(),
            52,
            "GoalKind::ALL is out of sync with the enum"
        );
    }

    #[test]
    fn every_kind_has_a_unique_key() {
        let mut keys: Vec<&str> = GoalKind::ALL.iter().map(|k| k.as_i18n_key()).collect();
        keys.sort_unstable();
        let before = keys.len();
        keys.dedup();
        assert_eq!(before, keys.len());
    }

    #[test]
    fn every_bit_fits_the_mask() {
        for kind in GoalKind::ALL {
            assert!(
                kind.bit() < 64,
                "{kind:?} at bit {} does not fit a GoalMask",
                kind.bit()
            );
        }
    }

    #[test]
    fn pressing_always_takes_more_than_voicing() {
        for kind in GoalKind::ALL {
            let spec = kind.spec();
            assert!(
                spec.press_at > spec.voice_at,
                "{kind:?}: he must say it before he demands it"
            );
            assert!((0.0..=1.0).contains(&spec.voice_at));
            assert!((0.0..=1.0).contains(&spec.press_at));
            assert!(spec.decay_per_month > 0.0 && spec.decay_per_month < 1.0);
        }
    }

    #[test]
    fn competition_is_never_self_referential() {
        for kind in GoalKind::ALL {
            assert!(
                !kind.spec().competes_with.contains(*kind),
                "{kind:?} competes with itself"
            );
        }
    }

    #[test]
    fn leaving_and_staying_goals_compete_with_each_other() {
        // The structural guarantee behind `wants_to_leave`: a decision to
        // stay must push back on the goals that point out, and vice versa.
        assert!(
            GoalKind::StayAtThisClub
                .spec()
                .competes_with
                .contains(GoalKind::LeaveThisClub)
        );
        assert!(
            GoalKind::LeaveThisClub
                .spec()
                .competes_with
                .contains(GoalKind::StayAtThisClub)
        );
    }

    #[test]
    fn a_grievance_outlives_a_passing_want() {
        assert!(
            GoalKind::LeaveThisClub.spec().decay_per_month
                < GoalKind::GetAReleaseClause.spec().decay_per_month,
            "a decision to get out does not evaporate the way a contract wish does"
        );
    }

    #[test]
    fn direction_matches_the_narrative() {
        assert!(GoalKind::StepUpToABiggerClub.points_away());
        assert!(GoalKind::PlayFirstTeamFootball.points_away());
        assert!(!GoalKind::StayAtThisClub.points_away());
        assert!(!GoalKind::WinBackMyPlace.points_away());
        assert!(!GoalKind::BePaidWhatImWorth.points_away());
    }

    #[test]
    fn mask_membership_round_trips() {
        let mask = GoalMask::of(&[GoalKind::GoHome, GoalKind::BeCaptain]);
        assert!(mask.contains(GoalKind::GoHome));
        assert!(mask.contains(GoalKind::BeCaptain));
        assert!(!mask.contains(GoalKind::WinATrophy));
        assert!(GoalMask::EMPTY.is_empty());
    }
}
