//! Structural guards over the three shared catalogs.
//!
//! `EpisodeKind`, `FactClaim` and `GoalKind` are each held by two minds
//! now. That is deliberate — much of the vocabulary is genuinely shared,
//! and `ActorRef` already points both ways — but one enum serving two
//! readers is exactly the shape that goes wrong quietly.
//!
//! These are the checks that make it go wrong loudly instead: every kind
//! carries a domain and a key, no key collides *across* catalogs, and
//! the staff rows land on staff-side domains rather than silently
//! joining the player's.

use super::organs::goals::{GoalDirection, GoalDomain, GoalKind};
use super::organs::memory::{ActorKind, EpisodeDomain, EpisodeKind, FactClaim, MindHolder};

/// The staff-side episode kinds, named once so a new manager row that
/// forgets its domain is caught rather than quietly filed under the
/// player's.
const MANAGER_EPISODES: &[EpisodeKind] = &[
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
];

/// The manager rows of the goal catalog.
const MANAGER_GOALS: &[GoalKind] = &[
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

/// The convictions a manager carries between jobs.
const MANAGER_FACTS: &[FactClaim] = &[
    FactClaim::TheySackedMe,
    FactClaim::TheyNeverBackedMe,
    FactClaim::TheyKeptTheirWord,
    FactClaim::IBuiltSomethingThere,
    FactClaim::TheSquadWasNeverMine,
    FactClaim::ThatPlaceWasAGraveyard,
    FactClaim::TheirWordIsWorthless,
    FactClaim::TheyStoodByMe,
    FactClaim::HeRepaidMyFaith,
    FactClaim::HeLetMeDown,
    FactClaim::HeIsWorthBuildingAround,
    FactClaim::IWasWrongAboutHim,
    FactClaim::IAmATeacherNotAWinner,
    FactClaim::IOnlyWorkWithMySquad,
];

#[test]
fn no_key_collides_across_the_three_catalogs() {
    // One namespace, three enums. A key reused between them would make
    // a renderer show a goal where a memory belongs — and the
    // per-catalog uniqueness tests cannot see it.
    let mut keys: Vec<&str> = Vec::new();
    keys.extend(EpisodeKind::ALL.iter().map(|k| k.as_i18n_key()));
    keys.extend(FactClaim::ALL.iter().map(|c| c.as_i18n_key()));
    keys.extend(GoalKind::ALL.iter().map(|g| g.as_i18n_key()));

    let before = keys.len();
    keys.sort_unstable();
    keys.dedup();
    assert_eq!(
        before,
        keys.len(),
        "an i18n key is used by more than one catalog"
    );
}

#[test]
fn every_key_is_namespaced_by_its_catalog() {
    for kind in EpisodeKind::ALL {
        assert!(
            kind.as_i18n_key().starts_with("mind_episode_"),
            "{kind:?} has an off-namespace key"
        );
    }
    for claim in FactClaim::ALL {
        assert!(
            claim.as_i18n_key().starts_with("mind_fact_"),
            "{claim:?} has an off-namespace key"
        );
    }
    for goal in GoalKind::ALL {
        assert!(
            goal.as_i18n_key().starts_with("mind_goal_"),
            "{goal:?} has an off-namespace key"
        );
    }
}

#[test]
fn every_manager_episode_lands_on_a_staff_side_domain() {
    // The shared rows — `Relegated`, `WonLeagueTitle`, `FansHostility`,
    // `Bereavement` — deliberately keep their player domains and are
    // routed by `StaffMind::dispatch`. The rows that exist *only* for a
    // manager must not: one filed under `Career` would reach a player's
    // career mind if any emit site ever recorded it against a player.
    for kind in MANAGER_EPISODES {
        let domain = kind.domain();
        assert!(
            matches!(
                domain,
                EpisodeDomain::Management
                    | EpisodeDomain::Boardroom
                    | EpisodeDomain::Squad
                    | EpisodeDomain::Philosophy
                    | EpisodeDomain::Social
            ),
            "{kind:?} is a manager row filed under {domain:?}"
        );
    }
}

#[test]
fn every_manager_goal_lands_on_a_staff_side_domain() {
    for goal in MANAGER_GOALS {
        let domain = goal.domain();
        assert!(
            matches!(
                domain,
                GoalDomain::Management
                    | GoalDomain::Boardroom
                    | GoalDomain::Squad
                    | GoalDomain::Philosophy
                    | GoalDomain::Welfare
            ),
            "{goal:?} is a manager want filed under {domain:?}"
        );
    }
}

#[test]
fn the_manager_rows_have_a_counterweight() {
    // The structural guarantee behind manager tenure. Left alone,
    // ambition churns managers every season; the plan's answer is that
    // the Stay-goals compete with the Leave-goals directly.
    let leaving: Vec<GoalKind> = MANAGER_GOALS
        .iter()
        .copied()
        .filter(|g| g.direction() == GoalDirection::Leave)
        .collect();
    let staying: Vec<GoalKind> = MANAGER_GOALS
        .iter()
        .copied()
        .filter(|g| g.direction() == GoalDirection::Stay)
        .collect();

    assert!(!leaving.is_empty() && !staying.is_empty());
    for leave in &leaving {
        assert!(
            staying
                .iter()
                .any(|stay| leave.spec().competes_with.contains(*stay)),
            "{leave:?} points out of the job and nothing pushes back"
        );
    }
    for stay in &staying {
        assert!(
            leaving
                .iter()
                .any(|leave| stay.spec().competes_with.contains(*leave)),
            "{stay:?} keeps him in the job and competes with nothing"
        );
    }
}

#[test]
fn a_managers_convictions_are_about_the_right_kind_of_subject() {
    for claim in MANAGER_FACTS {
        let subject = claim.subject_kind();
        assert!(
            matches!(
                subject,
                ActorKind::Club | ActorKind::Board | ActorKind::Player | ActorKind::None
            ),
            "{claim:?} is held about {subject:?}, which a manager never judges"
        );
    }

    // The board is judged separately from the badge. That separation is
    // what makes a change of chairman a reason to look at a place
    // again, and it is the only thing keeping the two claim sets apart.
    assert_eq!(
        FactClaim::TheirWordIsWorthless.subject_kind(),
        ActorKind::Board
    );
    assert_eq!(FactClaim::TheyNeverBackedMe.subject_kind(), ActorKind::Club);
}

#[test]
fn the_two_readings_of_an_episode_never_form_the_same_claim() {
    // A title is `WonEverythingHere` to the man who played in it and
    // `IBuiltSomethingThere` to the man who picked the side. Where a
    // kind consolidates for both holders, the claims must differ — if
    // they agreed there would be no reason for `MindHolder` to exist.
    let mut shared = 0;
    for kind in EpisodeKind::ALL {
        let (Some(player), Some(staff)) = (
            kind.consolidates_to(MindHolder::Player),
            kind.consolidates_to(MindHolder::Staff),
        ) else {
            continue;
        };
        shared += 1;
        assert_ne!(
            player.claim, staff.claim,
            "{kind:?} means the same thing to a player and a manager"
        );
    }
    assert!(
        shared > 0,
        "no episode is read by both minds — the holder parameter is dead weight"
    );
}
