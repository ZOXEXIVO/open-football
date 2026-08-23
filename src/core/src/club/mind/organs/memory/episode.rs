//! Episodic memory — specific things that happened, to him, on a date,
//! involving someone.
//!
//! An episode is 16 bytes of packed POD. The store holds 32 of them, six
//! slots of which are reserved for flashbulb memories that never get
//! evicted. That is not a lot, and it is not meant to be: a career is
//! remembered as a handful of vivid moments plus what they added up to
//! (see [`semantic`]). Consolidation banks the meaning before the
//! episodes fade, which is what lets a thirty-four-year-old carry a
//! whole career in about a kilobyte.
//!
//! [`semantic`]: super::semantic

use super::actor::ActorRef;
use super::epoch::EpochDay;

/// What happened. A closed catalog so every renderer, consolidation rule
/// and test can key off the variant by name — and so adding a new
/// remembered event is one variant plus one spec row, per the
/// extensibility contract.
///
/// Grouped by domain. The groups matter: [`EpisodeKind::domain`] drives
/// which sub-mind claims an episode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum EpisodeKind {
    /// Placeholder for a default-constructed slot. Never stored.
    #[default]
    None,

    // ── Career landmarks ────────────────────────────────────────
    SeniorDebut,
    FirstGoalForClub,
    WonLeagueTitle,
    WonDomesticCup,
    WonContinentalTrophy,
    Relegated,
    Promoted,
    SignedForClub,
    SoldAgainstWill,
    ReleasedByClub,
    LoanedOut,
    ContractRenewed,
    CaptaincyAwarded,
    CaptaincyRemoved,
    ClubServantMilestone,

    // ── The manager ─────────────────────────────────────────────
    ManagerPromiseKept,
    ManagerPromiseBroken,
    ManagerPublicPraise,
    ManagerPublicCriticism,
    ManagerPrivateBacking,
    ManagerFrozenOut,
    ManagerSignedARival,
    ManagerLeftClub,
    ManagerArrived,

    // ── Selection and role ──────────────────────────────────────
    StartedBigMatch,
    LeftOutOfBigMatch,
    DroppedToBench,
    WonStartingPlace,
    LostStartingPlace,
    SubbedOffEarly,
    RoleDowngraded,
    RoleUpgraded,

    // ── On the pitch ────────────────────────────────────────────
    DecisiveGoal,
    ManOfTheMatch,
    CostlyError,
    SentOff,
    MissedDecisivePenalty,
    HeavyDefeat,
    DerbyWin,
    DerbyDefeat,

    // ── The dressing room ───────────────────────────────────────
    TeammateBefriended,
    TeammateConflict,
    MentorSupport,
    FeltIsolated,
    WelcomedBySquad,
    SquadBackedHim,
    SquadTurnedOnHim,

    // ── Fans and media ──────────────────────────────────────────
    FansAdoration,
    FansHostility,
    MediaPraise,
    MediaAttack,

    // ── Money ───────────────────────────────────────────────────
    BigPayRise,
    WageBelowPeers,
    ClubRefusedTerms,
    ClubBrokeWagePromise,

    // ── The body ────────────────────────────────────────────────
    SeriousInjury,
    CareerThreateningInjury,
    ReturnedFromLongInjury,

    // ── International ───────────────────────────────────────────
    FirstCap,
    NationalSnub,
    MajorTournamentSquad,
    NationalTeamGlory,

    // ── Life ────────────────────────────────────────────────────
    FamilySettled,
    FamilyUnsettled,
    Bereavement,
    ChildBorn,

    // ═══ The dugout ═════════════════════════════════════════════
    // A manager remembers his own career, not a player's. The kinds
    // above that are genuinely shared — `Relegated`, `WonLeagueTitle`,
    // `FansHostility`, `Bereavement` — are recorded by both minds.

    // ── His career ──────────────────────────────────────────────
    AppointedManager,
    SackedByClub,
    ResignedFromClub,
    CaretakerSpell,
    PromotedFromWithin,
    WonManagerOfTheMonth,
    SurvivedARelegationFight,
    FailedToSurviveIt,

    // ── The board ───────────────────────────────────────────────
    BoardKeptItsPromise,
    BoardBrokeItsPromise,
    BoardRefusedMyTarget,
    BoardBackedMeInTheWindow,
    BoardSoldMyBestPlayer,
    GivenAVoteOfConfidence,
    ChairmanUndercutMePublicly,

    // ── The squad he is responsible for ─────────────────────────
    LostTheDressingRoom,
    SquadFoughtForMe,
    PlayerRefusedToPlayForMe,
    PlayerRepaidMyFaith,
    SignedAPlayerIWanted,
    SignedAPlayerIDidNotWant,

    // ── His football ────────────────────────────────────────────
    MyGambleCameOff,
    MyGambleBackfired,

    // ── Standing outside the club ───────────────────────────────
    SupportersTurnedOnMe,
    SupportersSangMyName,
    MediaWroteMeOff,
}

/// Which faculty an episode primarily belongs to. Used to route
/// `observe` to the sub-mind that cares, and to keep recall results
/// grouped sensibly for the UI.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EpisodeDomain {
    Career,
    Professional,
    Competitive,
    Social,
    Financial,
    Body,
    Life,

    // ── Staff-side domains ──────────────────────────────────────
    /// A manager's own career: appointed, sacked, promoted from
    /// within, a relegation fight survived.
    Management,
    /// Dealings with the people above him.
    Boardroom,
    /// The squad as something he is responsible for, rather than
    /// something he belongs to.
    Squad,
    /// His football, and whether it worked.
    Philosophy,
}

/// How an episode is laid down: its intrinsic emotional weight, its
/// sign, and whether it is the kind of moment a career is remembered by.
#[derive(Debug, Clone, Copy)]
pub struct EpisodeSpec {
    /// Intrinsic emotional intensity, 0..1, before relevance and
    /// surprise scale it. The catalog anchor.
    pub intensity: f32,
    /// Natural valence, -1..+1. Emit sites may override for episodes
    /// whose sign genuinely depends on context.
    pub valence: f32,
    /// Career-defining: eligible for a protected flashbulb slot and the
    /// retention floor.
    pub flashbulb: bool,
    /// A broken commitment by someone who made one. Betrayals hit the
    /// attribution ledger's `trust` axis rather than just `warmth`, and
    /// they resist the ledger's drift back to neutral.
    pub betrayal: bool,
    pub domain: EpisodeDomain,
}

impl EpisodeSpec {
    /// An ordinary episode: something that happened and will be
    /// remembered for as long as the curve keeps it.
    pub const fn ordinary(intensity: f32, valence: f32, domain: EpisodeDomain) -> Self {
        EpisodeSpec {
            intensity,
            valence,
            flashbulb: false,
            betrayal: false,
            domain,
        }
    }

    /// A career-defining moment. Holds a protected slot and a retention
    /// floor — the handful of things a player can still tell you about
    /// decades later.
    pub const fn defining(intensity: f32, valence: f32, domain: EpisodeDomain) -> Self {
        EpisodeSpec {
            flashbulb: true,
            ..Self::ordinary(intensity, valence, domain)
        }
    }

    /// Someone gave his word and broke it. Hits the ledger's trust axis
    /// rather than merely souring the mood, and resists the drift back
    /// to neutral.
    pub const fn betrayal(intensity: f32, valence: f32, domain: EpisodeDomain) -> Self {
        EpisodeSpec {
            betrayal: true,
            ..Self::ordinary(intensity, valence, domain)
        }
    }
}

impl EpisodeKind {
    /// The catalog. One row per kind — adding a remembered event means
    /// adding a variant and a row here, nothing else.
    pub fn spec(self) -> EpisodeSpec {
        use EpisodeDomain as D;
        use EpisodeSpec as S;

        match self {
            EpisodeKind::None => S::ordinary(0.0, 0.0, D::Career),

            // Career landmarks
            EpisodeKind::SeniorDebut => S::defining(0.95, 0.90, D::Career),
            EpisodeKind::FirstGoalForClub => S::ordinary(0.70, 0.75, D::Career),
            EpisodeKind::WonLeagueTitle => S::defining(1.00, 1.00, D::Career),
            EpisodeKind::WonDomesticCup => S::ordinary(0.75, 0.80, D::Career),
            EpisodeKind::WonContinentalTrophy => S::defining(1.00, 1.00, D::Career),
            EpisodeKind::Relegated => S::defining(0.90, -0.90, D::Career),
            EpisodeKind::Promoted => S::ordinary(0.80, 0.80, D::Career),
            EpisodeKind::SignedForClub => S::ordinary(0.55, 0.35, D::Career),
            EpisodeKind::SoldAgainstWill => S::defining(0.90, -0.85, D::Career),
            EpisodeKind::ReleasedByClub => S::ordinary(0.75, -0.70, D::Career),
            EpisodeKind::LoanedOut => S::ordinary(0.50, -0.25, D::Career),
            EpisodeKind::ContractRenewed => S::ordinary(0.45, 0.50, D::Career),
            EpisodeKind::CaptaincyAwarded => S::defining(0.80, 0.85, D::Career),
            EpisodeKind::CaptaincyRemoved => S::ordinary(0.70, -0.70, D::Career),
            EpisodeKind::ClubServantMilestone => S::defining(0.75, 0.80, D::Career),

            // The manager
            EpisodeKind::ManagerPromiseKept => S::ordinary(0.55, 0.65, D::Professional),
            EpisodeKind::ManagerPromiseBroken => S::betrayal(0.85, -0.85, D::Professional),
            EpisodeKind::ManagerPublicPraise => S::ordinary(0.45, 0.55, D::Professional),
            EpisodeKind::ManagerPublicCriticism => S::ordinary(0.60, -0.65, D::Professional),
            EpisodeKind::ManagerPrivateBacking => S::ordinary(0.50, 0.60, D::Professional),
            EpisodeKind::ManagerFrozenOut => S::ordinary(0.80, -0.80, D::Professional),
            EpisodeKind::ManagerSignedARival => S::ordinary(0.50, -0.40, D::Professional),
            EpisodeKind::ManagerLeftClub => S::ordinary(0.45, 0.00, D::Professional),
            EpisodeKind::ManagerArrived => S::ordinary(0.40, 0.00, D::Professional),

            // Selection and role
            EpisodeKind::StartedBigMatch => S::ordinary(0.55, 0.60, D::Competitive),
            EpisodeKind::LeftOutOfBigMatch => S::ordinary(0.70, -0.70, D::Competitive),
            EpisodeKind::DroppedToBench => S::ordinary(0.55, -0.55, D::Competitive),
            EpisodeKind::WonStartingPlace => S::ordinary(0.65, 0.70, D::Competitive),
            EpisodeKind::LostStartingPlace => S::ordinary(0.65, -0.65, D::Competitive),
            EpisodeKind::SubbedOffEarly => S::ordinary(0.40, -0.45, D::Competitive),
            EpisodeKind::RoleDowngraded => S::ordinary(0.60, -0.60, D::Professional),
            EpisodeKind::RoleUpgraded => S::ordinary(0.55, 0.60, D::Professional),

            // On the pitch
            EpisodeKind::DecisiveGoal => S::ordinary(0.75, 0.85, D::Competitive),
            EpisodeKind::ManOfTheMatch => S::ordinary(0.50, 0.60, D::Competitive),
            EpisodeKind::CostlyError => S::ordinary(0.70, -0.75, D::Competitive),
            EpisodeKind::SentOff => S::ordinary(0.65, -0.70, D::Competitive),
            EpisodeKind::MissedDecisivePenalty => S::defining(0.85, -0.85, D::Competitive),
            EpisodeKind::HeavyDefeat => S::ordinary(0.45, -0.50, D::Competitive),
            EpisodeKind::DerbyWin => S::ordinary(0.65, 0.75, D::Competitive),
            EpisodeKind::DerbyDefeat => S::ordinary(0.60, -0.65, D::Competitive),

            // The dressing room
            EpisodeKind::TeammateBefriended => S::ordinary(0.45, 0.55, D::Social),
            EpisodeKind::TeammateConflict => S::ordinary(0.60, -0.60, D::Social),
            EpisodeKind::MentorSupport => S::ordinary(0.55, 0.65, D::Social),
            EpisodeKind::FeltIsolated => S::ordinary(0.55, -0.60, D::Social),
            EpisodeKind::WelcomedBySquad => S::ordinary(0.50, 0.60, D::Social),
            EpisodeKind::SquadBackedHim => S::ordinary(0.60, 0.70, D::Social),
            EpisodeKind::SquadTurnedOnHim => S::ordinary(0.75, -0.80, D::Social),

            // Fans and media
            EpisodeKind::FansAdoration => S::ordinary(0.60, 0.70, D::Social),
            EpisodeKind::FansHostility => S::ordinary(0.65, -0.70, D::Social),
            EpisodeKind::MediaPraise => S::ordinary(0.35, 0.40, D::Social),
            EpisodeKind::MediaAttack => S::ordinary(0.50, -0.55, D::Social),

            // Money
            EpisodeKind::BigPayRise => S::ordinary(0.50, 0.60, D::Financial),
            EpisodeKind::WageBelowPeers => S::ordinary(0.55, -0.55, D::Financial),
            EpisodeKind::ClubRefusedTerms => S::ordinary(0.60, -0.60, D::Financial),
            EpisodeKind::ClubBrokeWagePromise => S::betrayal(0.80, -0.80, D::Financial),

            // The body
            EpisodeKind::SeriousInjury => S::ordinary(0.70, -0.70, D::Body),
            EpisodeKind::CareerThreateningInjury => S::defining(0.95, -0.95, D::Body),
            EpisodeKind::ReturnedFromLongInjury => S::ordinary(0.65, 0.70, D::Body),

            // International
            EpisodeKind::FirstCap => S::defining(0.95, 0.90, D::Career),
            EpisodeKind::NationalSnub => S::ordinary(0.60, -0.60, D::Career),
            EpisodeKind::MajorTournamentSquad => S::ordinary(0.70, 0.75, D::Career),
            EpisodeKind::NationalTeamGlory => S::defining(1.00, 1.00, D::Career),

            // Life
            EpisodeKind::FamilySettled => S::ordinary(0.50, 0.55, D::Life),
            EpisodeKind::FamilyUnsettled => S::ordinary(0.60, -0.60, D::Life),
            EpisodeKind::Bereavement => S::defining(0.95, -0.90, D::Life),
            EpisodeKind::ChildBorn => S::defining(0.90, 0.95, D::Life),

            // His career
            EpisodeKind::AppointedManager => S::defining(0.85, 0.80, D::Management),
            EpisodeKind::SackedByClub => S::defining(0.95, -0.90, D::Management),
            EpisodeKind::ResignedFromClub => S::ordinary(0.70, -0.30, D::Management),
            EpisodeKind::CaretakerSpell => S::ordinary(0.45, 0.10, D::Management),
            EpisodeKind::PromotedFromWithin => S::defining(0.80, 0.85, D::Management),
            EpisodeKind::WonManagerOfTheMonth => S::ordinary(0.40, 0.55, D::Management),
            EpisodeKind::SurvivedARelegationFight => S::defining(0.85, 0.85, D::Management),
            EpisodeKind::FailedToSurviveIt => S::defining(0.90, -0.90, D::Management),

            // The board. Three of these are betrayals rather than
            // setbacks: a refused target is a decision he disagrees
            // with, a broken promise is a different thing entirely.
            EpisodeKind::BoardKeptItsPromise => S::ordinary(0.55, 0.65, D::Boardroom),
            EpisodeKind::BoardBrokeItsPromise => S::betrayal(0.85, -0.85, D::Boardroom),
            EpisodeKind::BoardRefusedMyTarget => S::ordinary(0.55, -0.50, D::Boardroom),
            EpisodeKind::BoardBackedMeInTheWindow => S::ordinary(0.60, 0.70, D::Boardroom),
            EpisodeKind::BoardSoldMyBestPlayer => S::betrayal(0.80, -0.80, D::Boardroom),
            // The public vote of confidence. Faintly negative on
            // purpose — in football it is what a board says on the way
            // to sacking someone, and every manager knows it.
            EpisodeKind::GivenAVoteOfConfidence => S::ordinary(0.50, -0.10, D::Boardroom),
            EpisodeKind::ChairmanUndercutMePublicly => S::ordinary(0.75, -0.75, D::Boardroom),

            // The squad
            EpisodeKind::LostTheDressingRoom => S::defining(0.90, -0.90, D::Squad),
            EpisodeKind::SquadFoughtForMe => S::ordinary(0.70, 0.80, D::Squad),
            EpisodeKind::PlayerRefusedToPlayForMe => S::betrayal(0.80, -0.80, D::Squad),
            EpisodeKind::PlayerRepaidMyFaith => S::ordinary(0.60, 0.75, D::Squad),
            EpisodeKind::SignedAPlayerIWanted => S::ordinary(0.50, 0.55, D::Squad),
            EpisodeKind::SignedAPlayerIDidNotWant => S::ordinary(0.55, -0.50, D::Squad),

            // His football
            EpisodeKind::MyGambleCameOff => S::ordinary(0.65, 0.75, D::Philosophy),
            EpisodeKind::MyGambleBackfired => S::ordinary(0.70, -0.70, D::Philosophy),

            // Standing. Shares the social domain with the player rows —
            // supporters and the press treat both the same way.
            EpisodeKind::SupportersTurnedOnMe => S::ordinary(0.75, -0.80, D::Social),
            EpisodeKind::SupportersSangMyName => S::ordinary(0.65, 0.80, D::Social),
            EpisodeKind::MediaWroteMeOff => S::ordinary(0.50, -0.55, D::Social),
        }
    }

    #[inline]
    pub fn domain(self) -> EpisodeDomain {
        self.spec().domain
    }

    #[inline]
    pub fn is_flashbulb(self) -> bool {
        self.spec().flashbulb
    }

    #[inline]
    pub fn is_betrayal(self) -> bool {
        self.spec().betrayal
    }

    /// Localisation key. Every kind must have one; the i18n parity test
    /// walks [`EpisodeKind::ALL`] to enforce it.
    pub fn as_i18n_key(self) -> &'static str {
        match self {
            EpisodeKind::None => "mind_episode_none",
            EpisodeKind::SeniorDebut => "mind_episode_senior_debut",
            EpisodeKind::FirstGoalForClub => "mind_episode_first_goal",
            EpisodeKind::WonLeagueTitle => "mind_episode_won_league",
            EpisodeKind::WonDomesticCup => "mind_episode_won_cup",
            EpisodeKind::WonContinentalTrophy => "mind_episode_won_continental",
            EpisodeKind::Relegated => "mind_episode_relegated",
            EpisodeKind::Promoted => "mind_episode_promoted",
            EpisodeKind::SignedForClub => "mind_episode_signed",
            EpisodeKind::SoldAgainstWill => "mind_episode_sold_against_will",
            EpisodeKind::ReleasedByClub => "mind_episode_released",
            EpisodeKind::LoanedOut => "mind_episode_loaned_out",
            EpisodeKind::ContractRenewed => "mind_episode_contract_renewed",
            EpisodeKind::CaptaincyAwarded => "mind_episode_captaincy_awarded",
            EpisodeKind::CaptaincyRemoved => "mind_episode_captaincy_removed",
            EpisodeKind::ClubServantMilestone => "mind_episode_club_servant",
            EpisodeKind::ManagerPromiseKept => "mind_episode_promise_kept",
            EpisodeKind::ManagerPromiseBroken => "mind_episode_promise_broken",
            EpisodeKind::ManagerPublicPraise => "mind_episode_manager_praise",
            EpisodeKind::ManagerPublicCriticism => "mind_episode_manager_criticism",
            EpisodeKind::ManagerPrivateBacking => "mind_episode_manager_backing",
            EpisodeKind::ManagerFrozenOut => "mind_episode_frozen_out",
            EpisodeKind::ManagerSignedARival => "mind_episode_signed_rival",
            EpisodeKind::ManagerLeftClub => "mind_episode_manager_left",
            EpisodeKind::ManagerArrived => "mind_episode_manager_arrived",
            EpisodeKind::StartedBigMatch => "mind_episode_started_big_match",
            EpisodeKind::LeftOutOfBigMatch => "mind_episode_left_out_big_match",
            EpisodeKind::DroppedToBench => "mind_episode_dropped",
            EpisodeKind::WonStartingPlace => "mind_episode_won_place",
            EpisodeKind::LostStartingPlace => "mind_episode_lost_place",
            EpisodeKind::SubbedOffEarly => "mind_episode_subbed_early",
            EpisodeKind::RoleDowngraded => "mind_episode_role_downgraded",
            EpisodeKind::RoleUpgraded => "mind_episode_role_upgraded",
            EpisodeKind::DecisiveGoal => "mind_episode_decisive_goal",
            EpisodeKind::ManOfTheMatch => "mind_episode_motm",
            EpisodeKind::CostlyError => "mind_episode_costly_error",
            EpisodeKind::SentOff => "mind_episode_sent_off",
            EpisodeKind::MissedDecisivePenalty => "mind_episode_missed_penalty",
            EpisodeKind::HeavyDefeat => "mind_episode_heavy_defeat",
            EpisodeKind::DerbyWin => "mind_episode_derby_win",
            EpisodeKind::DerbyDefeat => "mind_episode_derby_defeat",
            EpisodeKind::TeammateBefriended => "mind_episode_befriended",
            EpisodeKind::TeammateConflict => "mind_episode_teammate_conflict",
            EpisodeKind::MentorSupport => "mind_episode_mentor_support",
            EpisodeKind::FeltIsolated => "mind_episode_isolated",
            EpisodeKind::WelcomedBySquad => "mind_episode_welcomed",
            EpisodeKind::SquadBackedHim => "mind_episode_squad_backed",
            EpisodeKind::SquadTurnedOnHim => "mind_episode_squad_turned",
            EpisodeKind::FansAdoration => "mind_episode_fans_adoration",
            EpisodeKind::FansHostility => "mind_episode_fans_hostility",
            EpisodeKind::MediaPraise => "mind_episode_media_praise",
            EpisodeKind::MediaAttack => "mind_episode_media_attack",
            EpisodeKind::BigPayRise => "mind_episode_pay_rise",
            EpisodeKind::WageBelowPeers => "mind_episode_wage_below_peers",
            EpisodeKind::ClubRefusedTerms => "mind_episode_refused_terms",
            EpisodeKind::ClubBrokeWagePromise => "mind_episode_broke_wage_promise",
            EpisodeKind::SeriousInjury => "mind_episode_serious_injury",
            EpisodeKind::CareerThreateningInjury => "mind_episode_career_injury",
            EpisodeKind::ReturnedFromLongInjury => "mind_episode_returned_from_injury",
            EpisodeKind::FirstCap => "mind_episode_first_cap",
            EpisodeKind::NationalSnub => "mind_episode_national_snub",
            EpisodeKind::MajorTournamentSquad => "mind_episode_tournament_squad",
            EpisodeKind::NationalTeamGlory => "mind_episode_national_glory",
            EpisodeKind::FamilySettled => "mind_episode_family_settled",
            EpisodeKind::FamilyUnsettled => "mind_episode_family_unsettled",
            EpisodeKind::Bereavement => "mind_episode_bereavement",
            EpisodeKind::ChildBorn => "mind_episode_child_born",
            EpisodeKind::AppointedManager => "mind_episode_appointed_manager",
            EpisodeKind::SackedByClub => "mind_episode_sacked",
            EpisodeKind::ResignedFromClub => "mind_episode_resigned",
            EpisodeKind::CaretakerSpell => "mind_episode_caretaker_spell",
            EpisodeKind::PromotedFromWithin => "mind_episode_promoted_from_within",
            EpisodeKind::WonManagerOfTheMonth => "mind_episode_manager_of_the_month",
            EpisodeKind::SurvivedARelegationFight => "mind_episode_survived_relegation_fight",
            EpisodeKind::FailedToSurviveIt => "mind_episode_failed_relegation_fight",
            EpisodeKind::BoardKeptItsPromise => "mind_episode_board_kept_promise",
            EpisodeKind::BoardBrokeItsPromise => "mind_episode_board_broke_promise",
            EpisodeKind::BoardRefusedMyTarget => "mind_episode_board_refused_target",
            EpisodeKind::BoardBackedMeInTheWindow => "mind_episode_board_backed_me",
            EpisodeKind::BoardSoldMyBestPlayer => "mind_episode_board_sold_best_player",
            EpisodeKind::GivenAVoteOfConfidence => "mind_episode_vote_of_confidence",
            EpisodeKind::ChairmanUndercutMePublicly => "mind_episode_chairman_undercut_me",
            EpisodeKind::LostTheDressingRoom => "mind_episode_lost_dressing_room",
            EpisodeKind::SquadFoughtForMe => "mind_episode_squad_fought_for_me",
            EpisodeKind::PlayerRefusedToPlayForMe => "mind_episode_player_refused_to_play",
            EpisodeKind::PlayerRepaidMyFaith => "mind_episode_player_repaid_faith",
            EpisodeKind::SignedAPlayerIWanted => "mind_episode_signed_player_i_wanted",
            EpisodeKind::SignedAPlayerIDidNotWant => "mind_episode_signed_player_i_did_not_want",
            EpisodeKind::MyGambleCameOff => "mind_episode_gamble_came_off",
            EpisodeKind::MyGambleBackfired => "mind_episode_gamble_backfired",
            EpisodeKind::SupportersTurnedOnMe => "mind_episode_supporters_turned",
            EpisodeKind::SupportersSangMyName => "mind_episode_supporters_sang_my_name",
            EpisodeKind::MediaWroteMeOff => "mind_episode_media_wrote_me_off",
        }
    }

    /// Every real kind (excluding [`EpisodeKind::None`]). Kept in
    /// lockstep with the enum by `catalog_covers_every_kind`.
    pub const ALL: &'static [EpisodeKind] = &[
        EpisodeKind::SeniorDebut,
        EpisodeKind::FirstGoalForClub,
        EpisodeKind::WonLeagueTitle,
        EpisodeKind::WonDomesticCup,
        EpisodeKind::WonContinentalTrophy,
        EpisodeKind::Relegated,
        EpisodeKind::Promoted,
        EpisodeKind::SignedForClub,
        EpisodeKind::SoldAgainstWill,
        EpisodeKind::ReleasedByClub,
        EpisodeKind::LoanedOut,
        EpisodeKind::ContractRenewed,
        EpisodeKind::CaptaincyAwarded,
        EpisodeKind::CaptaincyRemoved,
        EpisodeKind::ClubServantMilestone,
        EpisodeKind::ManagerPromiseKept,
        EpisodeKind::ManagerPromiseBroken,
        EpisodeKind::ManagerPublicPraise,
        EpisodeKind::ManagerPublicCriticism,
        EpisodeKind::ManagerPrivateBacking,
        EpisodeKind::ManagerFrozenOut,
        EpisodeKind::ManagerSignedARival,
        EpisodeKind::ManagerLeftClub,
        EpisodeKind::ManagerArrived,
        EpisodeKind::StartedBigMatch,
        EpisodeKind::LeftOutOfBigMatch,
        EpisodeKind::DroppedToBench,
        EpisodeKind::WonStartingPlace,
        EpisodeKind::LostStartingPlace,
        EpisodeKind::SubbedOffEarly,
        EpisodeKind::RoleDowngraded,
        EpisodeKind::RoleUpgraded,
        EpisodeKind::DecisiveGoal,
        EpisodeKind::ManOfTheMatch,
        EpisodeKind::CostlyError,
        EpisodeKind::SentOff,
        EpisodeKind::MissedDecisivePenalty,
        EpisodeKind::HeavyDefeat,
        EpisodeKind::DerbyWin,
        EpisodeKind::DerbyDefeat,
        EpisodeKind::TeammateBefriended,
        EpisodeKind::TeammateConflict,
        EpisodeKind::MentorSupport,
        EpisodeKind::FeltIsolated,
        EpisodeKind::WelcomedBySquad,
        EpisodeKind::SquadBackedHim,
        EpisodeKind::SquadTurnedOnHim,
        EpisodeKind::FansAdoration,
        EpisodeKind::FansHostility,
        EpisodeKind::MediaPraise,
        EpisodeKind::MediaAttack,
        EpisodeKind::BigPayRise,
        EpisodeKind::WageBelowPeers,
        EpisodeKind::ClubRefusedTerms,
        EpisodeKind::ClubBrokeWagePromise,
        EpisodeKind::SeriousInjury,
        EpisodeKind::CareerThreateningInjury,
        EpisodeKind::ReturnedFromLongInjury,
        EpisodeKind::FirstCap,
        EpisodeKind::NationalSnub,
        EpisodeKind::MajorTournamentSquad,
        EpisodeKind::NationalTeamGlory,
        EpisodeKind::FamilySettled,
        EpisodeKind::FamilyUnsettled,
        EpisodeKind::Bereavement,
        EpisodeKind::ChildBorn,
        EpisodeKind::AppointedManager,
        EpisodeKind::SackedByClub,
        EpisodeKind::ResignedFromClub,
        EpisodeKind::CaretakerSpell,
        EpisodeKind::PromotedFromWithin,
        EpisodeKind::WonManagerOfTheMonth,
        EpisodeKind::SurvivedARelegationFight,
        EpisodeKind::FailedToSurviveIt,
        EpisodeKind::BoardKeptItsPromise,
        EpisodeKind::BoardBrokeItsPromise,
        EpisodeKind::BoardRefusedMyTarget,
        EpisodeKind::BoardBackedMeInTheWindow,
        EpisodeKind::BoardSoldMyBestPlayer,
        EpisodeKind::GivenAVoteOfConfidence,
        EpisodeKind::ChairmanUndercutMePublicly,
        EpisodeKind::LostTheDressingRoom,
        EpisodeKind::SquadFoughtForMe,
        EpisodeKind::PlayerRefusedToPlayForMe,
        EpisodeKind::PlayerRepaidMyFaith,
        EpisodeKind::SignedAPlayerIWanted,
        EpisodeKind::SignedAPlayerIDidNotWant,
        EpisodeKind::MyGambleCameOff,
        EpisodeKind::MyGambleBackfired,
        EpisodeKind::SupportersTurnedOnMe,
        EpisodeKind::SupportersSangMyName,
        EpisodeKind::MediaWroteMeOff,
    ];
}

/// Packed per-episode flags.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct EpisodeFlags(u8);

impl EpisodeFlags {
    /// Career-defining. Holds a retention floor and a protected slot.
    pub const FLASHBULB: u8 = 1 << 0;
    /// Someone broke a commitment. Drives the ledger's trust axis.
    pub const BETRAYAL: u8 = 1 << 1;
    /// Consolidation has already banked this episode's meaning into a
    /// semantic fact — it is free to fade without losing anything.
    pub const CONSOLIDATED: u8 = 1 << 2;
    /// Happened while the player was at a club he has since left. Set at
    /// club-change time; makes "the place I used to be" cheap to find.
    pub const FORMER_CLUB: u8 = 1 << 3;
    /// The player is still waiting on the outcome (a promise not yet due,
    /// a talk with no answer). Unresolved episodes resist consolidation.
    pub const UNRESOLVED: u8 = 1 << 4;

    #[inline]
    pub fn contains(self, flag: u8) -> bool {
        self.0 & flag != 0
    }

    #[inline]
    pub fn insert(&mut self, flag: u8) {
        self.0 |= flag;
    }

    #[inline]
    pub fn remove(&mut self, flag: u8) {
        self.0 &= !flag;
    }

    #[inline]
    pub fn bits(self) -> u8 {
        self.0
    }
}

/// One remembered event. 16 bytes, `Copy`.
///
/// `valence` and `encoding` are stored as `i8`/`u8` percentages rather
/// than `f32` — the accessors convert. That halves the record and costs
/// nothing: no downstream consumer needs more than 1% resolution on how
/// good or how vivid a memory is.
#[derive(Debug, Clone, Copy, Default)]
pub struct MindEpisode {
    pub kind: EpisodeKind,
    /// Who it was about. [`ActorRef::NONE`] for situational memories.
    pub who: ActorRef,
    /// The club he was at when it happened — the cue that brings a whole
    /// spell back a decade later. 0 when clubless.
    pub where_club: u32,
    /// When it happened.
    pub when: EpochDay,
    /// Later of `when` and the last time it was recalled. Rehearsal
    /// resets the forgetting clock; this is the field that carries it.
    pub last_touched: EpochDay,
    /// -100..=100.
    valence_pct: i8,
    /// 0..=100 — strength at encoding, then bumped by rehearsal.
    encoding_pct: u8,
    /// Times recalled. Saturates.
    pub recall_count: u8,
    pub flags: EpisodeFlags,
}

impl MindEpisode {
    /// Build an episode from its catalog spec, with encoding already
    /// computed by the caller (see [`EncodingInputs`]).
    pub fn new(
        kind: EpisodeKind,
        who: ActorRef,
        where_club: u32,
        when: EpochDay,
        valence: f32,
        encoding: f32,
    ) -> Self {
        let spec = kind.spec();
        let mut flags = EpisodeFlags::default();
        if spec.flashbulb {
            flags.insert(EpisodeFlags::FLASHBULB);
        }
        if spec.betrayal {
            flags.insert(EpisodeFlags::BETRAYAL);
        }

        MindEpisode {
            kind,
            who,
            where_club,
            when,
            last_touched: when,
            valence_pct: (valence.clamp(-1.0, 1.0) * 100.0).round() as i8,
            encoding_pct: (encoding.clamp(0.0, 1.0) * 100.0).round() as u8,
            recall_count: 0,
            flags,
        }
    }

    #[inline]
    pub fn valence(&self) -> f32 {
        self.valence_pct as f32 / 100.0
    }

    #[inline]
    pub fn set_valence(&mut self, valence: f32) {
        self.valence_pct = (valence.clamp(-1.0, 1.0) * 100.0).round() as i8;
    }

    #[inline]
    pub fn encoding(&self) -> f32 {
        self.encoding_pct as f32 / 100.0
    }

    #[inline]
    pub fn set_encoding(&mut self, encoding: f32) {
        self.encoding_pct = (encoding.clamp(0.0, 1.0) * 100.0).round() as u8;
    }

    #[inline]
    pub fn is_flashbulb(&self) -> bool {
        self.flags.contains(EpisodeFlags::FLASHBULB)
    }

    #[inline]
    pub fn is_betrayal(&self) -> bool {
        self.flags.contains(EpisodeFlags::BETRAYAL)
    }

    #[inline]
    pub fn is_consolidated(&self) -> bool {
        self.flags.contains(EpisodeFlags::CONSOLIDATED)
    }

    #[inline]
    pub fn is_positive(&self) -> bool {
        self.valence_pct > 0
    }
}

/// The three factors that decide how strongly an event is laid down.
///
/// `encoding = intensity × relevance × (0.5 + surprise)`
///
/// This is the formula that makes two players remember the same season
/// differently, and it is the single place expectation enters memory. It
/// also fixes, once, what the emit sites currently hand-code over and
/// over: being left out matters enormously to a man whose whole goal is
/// first-team football, and barely registers for a settled veteran.
#[derive(Debug, Clone, Copy)]
pub struct EncodingInputs {
    /// The catalog's intrinsic weight for this kind of event, 0..1.
    pub intensity: f32,
    /// How much this touches something he currently wants, 0..1.
    /// Neutral is 0.5 — an event unconnected to any active goal still
    /// registers, it just does not brand itself on him.
    pub relevance: f32,
    /// Prediction error against what he believed would happen, 0..1.
    /// Getting dropped when you thought the manager rated you is what
    /// you remember; getting dropped when you expected it is not.
    pub surprise: f32,
}

impl EncodingInputs {
    /// Relevance and surprise both neutral — for emit sites that have no
    /// goal or belief context to offer yet. Yields `intensity × 0.5 × 0.5`
    /// scaled back up so a neutral encode lands at the intrinsic weight.
    pub fn neutral(intensity: f32) -> Self {
        EncodingInputs {
            intensity,
            relevance: 0.5,
            surprise: 0.5,
        }
    }

    /// The encoding strength, 0..1.
    ///
    /// Both modifiers are bounded multipliers centred on 1.0 at neutral
    /// input, so a site that supplies no context lands exactly on the
    /// catalog intensity and supplying context moves it up or down from
    /// there. Relevance is given the wider span (×0.5..×1.5) because
    /// wanting something is the stronger determinant of what sticks;
    /// surprise modulates within ×0.75..×1.25.
    ///
    /// Deliberately *not* a raw ratio: an unbounded multiplier saturates
    /// against the 0..1 clamp, which would collapse "he wanted this
    /// badly" and "he wanted this badly and never saw it coming" into
    /// the same memory.
    pub fn strength(&self) -> f32 {
        let relevance = 0.5 + self.relevance.clamp(0.0, 1.0);
        let surprise = 0.75 + self.surprise.clamp(0.0, 1.0) * 0.5;
        (self.intensity.clamp(0.0, 1.0) * relevance * surprise).clamp(0.0, 1.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalog_covers_every_kind() {
        // Every kind in ALL must have a distinct i18n key and a spec
        // with a sane intensity. If a variant is added to the enum but
        // not to ALL, the coverage count below catches it.
        let mut keys: Vec<&str> = EpisodeKind::ALL.iter().map(|k| k.as_i18n_key()).collect();
        keys.sort_unstable();
        let before = keys.len();
        keys.dedup();
        assert_eq!(before, keys.len(), "every episode kind needs a unique key");

        for kind in EpisodeKind::ALL {
            let spec = kind.spec();
            assert!(
                spec.intensity > 0.0 && spec.intensity <= 1.0,
                "{kind:?} has an out-of-band intensity {}",
                spec.intensity
            );
            assert!(
                (-1.0..=1.0).contains(&spec.valence),
                "{kind:?} has an out-of-band valence"
            );
        }
    }

    #[test]
    fn all_lists_every_variant() {
        // Cheap structural guard: the count must match the enum. Bump
        // both together when adding a kind.
        assert_eq!(
            EpisodeKind::ALL.len(),
            92,
            "EpisodeKind::ALL is out of sync with the enum"
        );
    }

    #[test]
    fn neutral_encoding_reproduces_catalog_intensity() {
        let spec = EpisodeKind::DecisiveGoal.spec();
        let strength = EncodingInputs::neutral(spec.intensity).strength();
        assert!(
            (strength - spec.intensity).abs() < 1e-5,
            "a context-free encode must land on the anchor: {strength} vs {}",
            spec.intensity
        );
    }

    #[test]
    fn relevance_and_surprise_move_encoding_the_right_way() {
        let base = EncodingInputs::neutral(0.5).strength();

        let wanted_it_badly = EncodingInputs {
            intensity: 0.5,
            relevance: 1.0,
            surprise: 0.5,
        }
        .strength();
        assert!(wanted_it_badly > base);

        let saw_it_coming = EncodingInputs {
            intensity: 0.5,
            relevance: 0.5,
            surprise: 0.0,
        }
        .strength();
        assert!(saw_it_coming < base, "expected events brand less deeply");

        let blindsided = EncodingInputs {
            intensity: 0.5,
            relevance: 1.0,
            surprise: 1.0,
        }
        .strength();
        assert!(blindsided > wanted_it_badly);
    }

    #[test]
    fn flashbulb_and_betrayal_flags_come_from_the_catalog() {
        let debut = MindEpisode::new(EpisodeKind::SeniorDebut, ActorRef::NONE, 7, 100, 0.9, 0.9);
        assert!(debut.is_flashbulb());
        assert!(!debut.is_betrayal());

        let broken = MindEpisode::new(
            EpisodeKind::ManagerPromiseBroken,
            ActorRef::staff(412),
            7,
            100,
            -0.85,
            0.8,
        );
        assert!(broken.is_betrayal());
        assert!(!broken.is_flashbulb());
    }

    #[test]
    fn packed_accessors_round_trip_within_one_percent() {
        let mut ep = MindEpisode::new(EpisodeKind::DerbyWin, ActorRef::NONE, 1, 10, 0.75, 0.62);
        assert!((ep.valence() - 0.75).abs() <= 0.01);
        assert!((ep.encoding() - 0.62).abs() <= 0.01);
        ep.set_valence(-0.33);
        ep.set_encoding(0.99);
        assert!((ep.valence() + 0.33).abs() <= 0.01);
        assert!((ep.encoding() - 0.99).abs() <= 0.01);
    }

    #[test]
    fn an_episode_stays_within_its_budget() {
        // The footprint claim, asserted. 24 bytes × 32 slots = 768 B of
        // episodic memory per player, inline, no allocation. If this
        // grows, the budget in docs/player_mind.md needs revisiting.
        assert!(
            size_of::<MindEpisode>() <= 24,
            "MindEpisode grew to {} bytes",
            size_of::<MindEpisode>()
        );
    }
}
