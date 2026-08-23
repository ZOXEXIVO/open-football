//! End-to-end tests over a real [`Player`], exercising the wired emit
//! sites rather than the memory organ in isolation.
//!
//! The unit tests under `organs/memory/` prove each mechanism; these
//! prove the mechanisms are actually reached by the simulation, and that
//! the story they tell together is the right one.

#![cfg(test)]

use crate::club::player::builder::PlayerBuilder;
use crate::club::player::core::player::{
    ManagerPromise, ManagerPromiseKind, TransferRequestReason,
};
use crate::club::player::mind::{
    ActorRef, EpisodeKind, FactClaim, GoalBridge, GoalEvidence, GoalKind, GoalOrigin, GoalStatus,
    MindClock, MindSituation, RecallCue,
};
use crate::shared::fullname::FullName;
use crate::{
    PersonAttributes, Player, PlayerAttributes, PlayerClubContract, PlayerPosition,
    PlayerPositionType, PlayerPositions, PlayerSkills, PlayerSquadStatus, PlayerStatusType,
};
use chrono::{Duration, NaiveDate};

const CLUB: u32 = 7;
const COACH: u32 = 412;

/// Fixture builders for these tests. Grouped on a type rather than left
/// loose so the file reads as `Fixture::player()` at every call site.
struct Fixture;

impl Fixture {
    fn date(year: i32, month: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(year, month, day).expect("valid fixture date")
    }

    fn attributes() -> PersonAttributes {
        PersonAttributes {
            adaptability: 12.0,
            ambition: 14.0,
            controversy: 5.0,
            loyalty: 12.0,
            pressure: 12.0,
            professionalism: 12.0,
            sportsmanship: 12.0,
            temperament: 10.0,
            consistency: 12.0,
            important_matches: 12.0,
            dirtiness: 5.0,
        }
    }

    /// A settled first-team regular with an empty mind.
    fn player() -> Player {
        let mut contract = PlayerClubContract::new(50_000, Self::date(2032, 6, 30));
        contract.squad_status = PlayerSquadStatus::FirstTeamRegular;

        PlayerBuilder::new()
            .id(1)
            .full_name(FullName::new("Test".into(), "Player".into()))
            .birth_date(Self::date(2002, 1, 1))
            .country_id(1)
            .attributes(Self::attributes())
            .skills(PlayerSkills::default())
            .positions(PlayerPositions {
                positions: vec![PlayerPosition {
                    position: PlayerPositionType::Striker,
                    level: 20,
                }],
            })
            .player_attributes(PlayerAttributes::default())
            .contract(Some(contract))
            .build()
            .unwrap()
    }

    /// A starting-role promise whose deadline has passed. Judged broken
    /// unless the player has the starts to meet `target_value`.
    fn overdue_promise(made_on: NaiveDate, importance: u8, public: bool) -> ManagerPromise {
        ManagerPromise {
            kind: ManagerPromiseKind::StartingRole,
            made_on,
            deadline: made_on + Duration::days(90),
            baseline_apps: 0,
            baseline_starts: 0,
            target_value: 60,
            made_by_staff_id: Some(COACH),
            credibility_at_creation: 80,
            importance_to_player: importance,
            is_public: public,
        }
    }
}

#[test]
fn a_broken_promise_is_filed_against_the_man_who_made_it() {
    let mut p = Fixture::player();
    p.promises.push(Fixture::overdue_promise(
        Fixture::date(2026, 1, 1),
        80,
        false,
    ));

    p.verify_promises(Fixture::date(2026, 6, 1), Some(CLUB));

    let census = p.mind.census();
    assert_eq!(census.episodes, 1, "the promise left a memory");
    assert_eq!(census.accounts, 1, "and an account with the coach");

    let ctx = p.mind_context(Fixture::date(2026, 6, 1), Some(CLUB));
    assert!(
        p.mind.standing_with(ActorRef::staff(COACH), &ctx) < 0.0,
        "he thinks less of the man"
    );
}

#[test]
fn a_kept_promise_files_the_other_way() {
    let mut p = Fixture::player();
    // Met the starting-role target outright.
    p.statistics.played = 20;
    p.promises.push(ManagerPromise {
        target_value: 50,
        ..Fixture::overdue_promise(Fixture::date(2026, 1, 1), 80, false)
    });

    p.verify_promises(Fixture::date(2026, 6, 1), Some(CLUB));

    let ctx = p.mind_context(Fixture::date(2026, 6, 1), Some(CLUB));
    assert!(
        p.mind.standing_with(ActorRef::staff(COACH), &ctx) > 0.0,
        "keeping your word counts for something"
    );
    assert!(
        p.mind
            .memory()
            .episodes
            .find(|e| e.kind == EpisodeKind::ManagerPromiseKept)
            .is_some()
    );
}

#[test]
fn how_much_the_promise_mattered_decides_how_deeply_it_lands() {
    let encode = |importance: u8, public: bool| -> f32 {
        let mut p = Fixture::player();
        p.promises.push(Fixture::overdue_promise(
            Fixture::date(2026, 1, 1),
            importance,
            public,
        ));
        p.verify_promises(Fixture::date(2026, 6, 1), Some(CLUB));
        p.mind
            .memory()
            .episodes
            .find(|e| e.kind == EpisodeKind::ManagerPromiseBroken)
            .map(|e| e.encoding())
            .unwrap_or(0.0)
    };

    let trivial = encode(20, false);
    let important = encode(90, false);
    let public_and_important = encode(90, true);

    assert!(
        important > trivial,
        "a promise he cared about brands deeper: {important} vs {trivial}"
    );
    assert!(
        public_and_important >= important,
        "and one made in public deeper still"
    );
}

#[test]
fn a_promise_from_nobody_in_particular_still_registers_but_files_against_no_one() {
    let mut p = Fixture::player();
    p.promises.push(ManagerPromise {
        made_by_staff_id: None,
        ..Fixture::overdue_promise(Fixture::date(2026, 1, 1), 80, false)
    });

    p.verify_promises(Fixture::date(2026, 6, 1), Some(CLUB));

    assert_eq!(
        p.mind.census().episodes,
        0,
        "with no one to blame there is nothing to remember about anyone"
    );
    assert!(
        p.happiness.recent_events.iter().any(|e| e.magnitude < 0.0),
        "the morale hit still lands — memory is additive, not a replacement"
    );
}

#[test]
fn a_pattern_of_broken_promises_becomes_a_judgement_that_outlives_them() {
    // The headline behaviour, driven entirely through the public API a
    // real simulation uses.
    let mut p = Fixture::player();

    // Three promises, three failures, over a season and a half.
    for (index, made_on) in [
        Fixture::date(2026, 1, 1),
        Fixture::date(2026, 8, 1),
        Fixture::date(2027, 2, 1),
    ]
    .into_iter()
    .enumerate()
    {
        p.promises
            .push(Fixture::overdue_promise(made_on, 85, index == 2));
        p.verify_promises(made_on + Duration::days(120), Some(CLUB));
    }

    // The monthly think banks what it meant.
    let after = Fixture::date(2027, 7, 1);
    p.mind.tick(&p.mind_context(after, Some(CLUB)));

    assert!(
        p.mind
            .memory()
            .believes(FactClaim::HisWordIsWorthless, ActorRef::staff(COACH))
            > 0.0,
        "three broken promises is not three bad weeks — it is a conclusion about a man"
    );

    // A decade on, at another club entirely. Every episode has long since
    // been pushed out by a career's worth of newer memories.
    p.mind.on_club_change(CLUB);
    let much_later = Fixture::date(2037, 7, 1);
    let late_ctx = p.mind_context(much_later, Some(99));

    assert!(
        p.mind.standing_with(ActorRef::staff(COACH), &late_ctx) < -0.2,
        "ten years later he still would not play for him, got {}",
        p.mind.standing_with(ActorRef::staff(COACH), &late_ctx)
    );
}

#[test]
fn the_club_and_the_coach_are_remembered_separately() {
    let mut p = Fixture::player();

    // Good things at the club, a bad man in the dugout.
    let ctx = p.mind_context(Fixture::date(2026, 1, 10), Some(CLUB));
    p.mind
        .remember(EpisodeKind::SeniorDebut, ActorRef::NONE, &ctx);
    p.mind
        .remember(EpisodeKind::WonLeagueTitle, ActorRef::NONE, &ctx);

    p.promises.push(Fixture::overdue_promise(
        Fixture::date(2026, 2, 1),
        85,
        false,
    ));
    p.verify_promises(Fixture::date(2026, 7, 1), Some(CLUB));

    p.mind
        .tick(&p.mind_context(Fixture::date(2026, 8, 1), Some(CLUB)));

    let later = p.mind_context(Fixture::date(2034, 8, 1), Some(99));
    assert!(
        p.mind.club_sentiment(CLUB, &later) > 0.0,
        "the club is still where he broke through"
    );
    assert!(
        p.mind.standing_with(ActorRef::staff(COACH), &later) < 0.0,
        "the manager is still the manager"
    );
}

#[test]
fn a_return_to_the_old_club_brings_it_back_and_makes_it_vivid_again() {
    let mut p = Fixture::player();
    let early = p.mind_context(Fixture::date(2026, 1, 10), Some(CLUB));
    p.mind
        .remember(EpisodeKind::SeniorDebut, ActorRef::NONE, &early);
    p.mind.on_club_change(CLUB);

    let much_later = p.mind_context(Fixture::date(2036, 1, 10), Some(99));
    let faded = p
        .mind
        .memory()
        .episodes
        .find(|e| e.kind == EpisodeKind::SeniorDebut)
        .unwrap()
        .encoding();

    let recalled = p.mind.recall(RecallCue::Club(CLUB), &much_later);
    assert!(!recalled.is_empty(), "the place comes back");

    let after = p
        .mind
        .memory()
        .episodes
        .find(|e| e.kind == EpisodeKind::SeniorDebut)
        .unwrap();
    assert!(
        after.encoding() > faded,
        "and walking back in makes it vivid again: {faded} → {}",
        after.encoding()
    );
    assert_eq!(after.recall_count, 1);
}

#[test]
fn an_ordinary_career_never_fills_the_stores() {
    // Budget guard at the `Player` level: a full career of promise
    // traffic must not push any store past its cap.
    let mut p = Fixture::player();
    let mut made_on = Fixture::date(2026, 1, 1);
    for _ in 0..60 {
        p.promises
            .push(Fixture::overdue_promise(made_on, 60, false));
        made_on += Duration::days(100);
        p.verify_promises(made_on + Duration::days(1), Some(CLUB));
        p.mind
            .tick(&p.mind_context(made_on + Duration::days(2), Some(CLUB)));
    }

    let census = p.mind.census();
    assert!(census.episodes <= 32, "episodes: {}", census.episodes);
    assert!(census.facts <= 24, "facts: {}", census.facts);
    assert!(census.accounts <= 32, "accounts: {}", census.accounts);
}

// ── Goals (phase 3) ─────────────────────────────────────────────

/// Drive the goal stack the way the weekly desire tick does: the same
/// reason, seen again and again, week after week.
struct DesireRun;

impl DesireRun {
    /// Feed `reason` for `weeks`, reviewing each week, and return the day
    /// it finished on.
    fn sustain(
        player: &mut Player,
        reason: TransferRequestReason,
        weeks: u16,
        from: NaiveDate,
    ) -> NaiveDate {
        let mapping = GoalBridge::from_transfer_request_reason(reason);
        let mut day = from;
        for _ in 0..weeks {
            day += Duration::days(7);
            let ctx = player.mind_context(day, Some(CLUB));
            player.mind.pursue(
                mapping.goal,
                mapping.origin,
                mapping.evidence,
                mapping.weight,
                &ctx,
            );
            player.mind.tick(&ctx);
        }
        day
    }

    /// Review only — nothing feeds the want any more.
    fn silence(player: &mut Player, weeks: u16, from: NaiveDate) -> NaiveDate {
        let mut day = from;
        for _ in 0..weeks {
            day += Duration::days(7);
            let ctx = player.mind_context(day, Some(CLUB));
            player.mind.tick(&ctx);
        }
        day
    }
}

#[test]
fn a_want_climbs_the_ladder_instead_of_being_rediscovered_every_week() {
    // The headline behaviour of phase 3, through the public API.
    let mut p = Fixture::player();
    let start = Fixture::date(2026, 1, 1);

    DesireRun::sustain(&mut p, TransferRequestReason::LongUnhappiness, 2, start);
    let early = p.mind.goals().status_of(GoalKind::LeaveThisClub);
    assert!(
        !early.is_public(),
        "a fortnight in, it colours what he decides and nobody has heard him say it: {early:?}"
    );

    let day = DesireRun::sustain(
        &mut p,
        TransferRequestReason::LongUnhappiness,
        6,
        start + Duration::days(14),
    );
    assert!(
        p.mind
            .goals()
            .status_of(GoalKind::LeaveThisClub)
            .shapes_decisions(),
        "by two months it colours everything he decides"
    );

    DesireRun::sustain(&mut p, TransferRequestReason::LongUnhappiness, 12, day);
    assert!(
        p.mind.is_pressing(),
        "and by half a season it is a formal demand"
    );

    let goal = p.mind.goals().get(GoalKind::LeaveThisClub).unwrap();
    assert!(
        goal.reinforcements > 15,
        "one want, carried — not twenty separate rediscoveries"
    );
}

#[test]
fn a_grievance_that_resolves_fades_instead_of_vanishing() {
    // The designed disagreement with the legacy path, asserted. `Req`
    // clears the day the last reason stops firing; the want subsides.
    let mut p = Fixture::player();
    let start = Fixture::date(2026, 1, 1);
    let day = DesireRun::sustain(&mut p, TransferRequestReason::LongUnhappiness, 20, start);
    assert!(p.mind.is_pressing());
    let at_peak = p.mind.pressure_of(GoalKind::LeaveThisClub);

    // Two weeks of quiet: still there, and still wanted.
    let day = DesireRun::silence(&mut p, 2, day);
    assert!(
        p.mind.pressure_of(GoalKind::LeaveThisClub) > at_peak * 0.8,
        "a fortnight does not undo half a season"
    );

    // A year of quiet: no longer a demand, but he has not forgotten.
    // `LeaveThisClub` carries the slowest decay in the catalog on
    // purpose — a man who has decided he wants out does not quietly stop
    // wanting it — so a year takes roughly a third off, not all of it.
    let day = DesireRun::silence(&mut p, 52, day);
    let after_a_year = p.mind.pressure_of(GoalKind::LeaveThisClub);
    assert!(after_a_year < at_peak * 0.8, "a year takes the edge off");
    assert!(
        !p.mind.is_pressing(),
        "and he is no longer demanding anything"
    );
    assert!(
        after_a_year > at_peak * 0.4,
        "but it is still there: {after_a_year} of {at_peak}"
    );

    // Three more, and it is gone.
    DesireRun::silence(&mut p, 156, day);
    assert!(
        p.mind.pressure_of(GoalKind::LeaveThisClub) < at_peak * 0.25,
        "four years on, he has genuinely moved on"
    );
}

#[test]
fn he_gives_it_until_the_window_before_he_demands() {
    // A player who has just started wanting first-team football does not
    // hand in a request that afternoon. The current model has no way to
    // represent this at all.
    let mut p = Fixture::player();
    let start = Fixture::date(2026, 1, 1);

    let ctx = p.mind_context(start, Some(CLUB));
    let mapping =
        GoalBridge::from_transfer_request_reason(TransferRequestReason::WantsFirstTeamFootball);
    p.mind.pursue(
        mapping.goal,
        mapping.origin,
        mapping.evidence,
        mapping.weight,
        &ctx,
    );
    p.mind
        .goals_mut()
        .commit_until(mapping.goal, MindClock::day(start) + 180);

    // Six months of wanting it without it hardening into a demand.
    let day = DesireRun::sustain(
        &mut p,
        TransferRequestReason::WantsFirstTeamFootball,
        24,
        start,
    );
    assert!(
        p.mind
            .goals()
            .get(GoalKind::PlayFirstTeamFootball)
            .is_some_and(|g| g.has_deadline()),
        "he is still waiting on the date he set"
    );

    // The window comes and goes with nothing changed.
    DesireRun::silence(&mut p, 4, day + Duration::days(180));
    assert_eq!(
        p.mind.goals().status_of(GoalKind::PlayFirstTeamFootball),
        GoalStatus::Abandoned,
        "the date he gave himself passed unmet and the want resolved"
    );
}

#[test]
fn what_he_wants_decides_what_he_remembers() {
    // The two organs, coupled. Same event, same player, different mind.
    let encode_after = |goal_weeks: u16| -> f32 {
        let mut p = Fixture::player();
        let start = Fixture::date(2026, 1, 1);
        let day = if goal_weeks > 0 {
            DesireRun::sustain(
                &mut p,
                TransferRequestReason::WantsFirstTeamFootball,
                goal_weeks,
                start,
            )
        } else {
            start
        };

        let ctx = p.mind_context(day, Some(CLUB));
        p.mind
            .remember(EpisodeKind::LeftOutOfBigMatch, ActorRef::NONE, &ctx);
        p.mind
            .memory()
            .episodes
            .find(|e| e.kind == EpisodeKind::LeftOutOfBigMatch)
            .map(|e| e.encoding())
            .unwrap_or(0.0)
    };

    let settled = encode_after(0);
    let desperate = encode_after(20);
    assert!(
        desperate > settled * 1.2,
        "being left out brands itself on a man who wants to play: {settled} vs {desperate}"
    );
}

#[test]
fn moving_answers_what_he_wanted_out_of_but_not_what_he_is_owed() {
    let mut p = Fixture::player();
    let start = Fixture::date(2026, 1, 1);
    DesireRun::sustain(&mut p, TransferRequestReason::LongUnhappiness, 16, start);
    DesireRun::sustain(&mut p, TransferRequestReason::SalaryUnresolved, 16, start);
    assert!(p.mind.goals().get(GoalKind::LeaveThisClub).is_some());

    p.mind.on_club_change(CLUB);

    assert!(
        p.mind.goals().get(GoalKind::LeaveThisClub).is_none(),
        "he got out"
    );
    assert!(
        p.mind.goals().get(GoalKind::BePaidWhatImWorth).is_some(),
        "being underpaid is not settled by changing employer"
    );
}

#[test]
fn the_desire_tick_feeds_goals_without_touching_the_legacy_verdict() {
    // The parallel run itself: the legacy path still owns `Req`, and the
    // stack accumulates alongside it.
    let mut p = Fixture::player();
    let start = Fixture::date(2026, 1, 1);
    DesireRun::sustain(
        &mut p,
        TransferRequestReason::WantsStrongerLeague,
        10,
        start,
    );

    assert!(
        p.mind.pressure_of(GoalKind::PlayInAStrongerLeague) > 0.0,
        "the stack heard it"
    );
    assert!(
        p.transfer_request_reasons.is_empty(),
        "and the legacy set is untouched by the stack"
    );
    assert!(
        !p.statuses.has(PlayerStatusType::Req),
        "nothing downstream has changed hands yet"
    );
}

#[test]
fn a_career_of_wanting_things_never_fills_the_stack() {
    let mut p = Fixture::player();
    let mut day = Fixture::date(2026, 1, 1);
    for (index, reason) in GoalBridge::ALL_REASONS.iter().cycle().take(120).enumerate() {
        day += Duration::days(7);
        let mapping = GoalBridge::from_transfer_request_reason(*reason);
        let ctx = p.mind_context(day, Some(CLUB));
        p.mind.pursue(
            mapping.goal,
            mapping.origin,
            mapping.evidence,
            mapping.weight,
            &ctx,
        );
        p.mind.tick(&ctx);
        if index % 30 == 29 {
            p.mind.on_club_change(CLUB);
        }
    }

    assert!(
        p.mind.goals().len() <= 12,
        "goals: {}",
        p.mind.goals().len()
    );
    assert!(p.mind.census().episodes <= 32);
}

// ── Sub-minds (phase 4) ─────────────────────────────────────────

impl Fixture {
    /// A settled first-teamer under a named manager.
    fn situation() -> MindSituation {
        MindSituation {
            days_at_club: 500,
            starter_ratio: 0.7,
            expected_start_share: 0.6,
            manager: ActorRef::staff(COACH),
            ..MindSituation::neutral()
        }
    }

    /// The same man, out of the side.
    fn benched() -> MindSituation {
        MindSituation {
            starter_ratio: 0.05,
            expected_start_share: 0.70,
            age: 29,
            ..Self::situation()
        }
    }
}

/// Run the full weekly think against a real situation.
struct Weeks;

impl Weeks {
    fn run(
        player: &mut Player,
        situation: &MindSituation,
        count: u16,
        from: NaiveDate,
    ) -> NaiveDate {
        let mut day = from;
        for _ in 0..count {
            day += Duration::days(7);
            let ctx = player.mind_context(day, Some(CLUB));
            player.mind.tick_with(&ctx, situation);
        }
        day
    }
}

#[test]
fn an_episode_reaches_the_faculty_that_cares_about_it() {
    let mut p = Fixture::player();
    let ctx = p.mind_context(Fixture::date(2026, 1, 1), Some(CLUB));

    p.mind
        .remember(EpisodeKind::WonLeagueTitle, ActorRef::NONE, &ctx);
    assert_eq!(p.mind.career.honours, 1, "a title is a career event");

    p.mind
        .remember(EpisodeKind::TeammateConflict, ActorRef::player(9), &ctx);
    assert_eq!(p.mind.social.friction, 1, "a fallout is a social one");

    p.mind.remember(
        EpisodeKind::ManagerPromiseBroken,
        ActorRef::staff(COACH),
        &ctx,
    );
    assert!(
        p.mind.professional.feels_rated() < 0.0,
        "a broken promise is a professional one"
    );

    p.mind
        .remember(EpisodeKind::WageBelowPeers, ActorRef::club(CLUB), &ctx);
    assert!(p.mind.financial.envy() > 0.0, "a wage is a financial one");

    p.mind
        .remember(EpisodeKind::CostlyError, ActorRef::NONE, &ctx);
    assert!(
        p.mind.competitive.self_belief() < 0.0,
        "a mistake is a competitive one"
    );
}

#[test]
fn the_body_and_life_reach_the_faculties_they_actually_land_on() {
    let mut p = Fixture::player();
    let ctx = p.mind_context(Fixture::date(2026, 1, 1), Some(CLUB));

    p.mind
        .remember(EpisodeKind::CareerThreateningInjury, ActorRef::NONE, &ctx);
    assert!(
        p.mind.competitive.self_belief() < 0.0,
        "an injury is a blow to a player's belief, not only his fitness"
    );

    p.mind
        .remember(EpisodeKind::ChildBorn, ActorRef::NONE, &ctx);
    assert!(
        p.mind.social.belonging() > 0.0,
        "life outside the game reaches whether he feels at home"
    );
}

#[test]
fn a_tick_with_no_situation_does_not_let_the_faculties_think() {
    // The trap this guards: a neutral situation is not a neutral input.
    // To the competitive mind it reads as a man getting the minutes his
    // role implies, which would quietly satisfy wants the caller knows
    // nothing about.
    let mut p = Fixture::player();
    let ctx = p.mind_context(Fixture::date(2026, 1, 1), Some(CLUB));
    p.mind.pursue(
        GoalKind::PlayFirstTeamFootball,
        GoalOrigin::Survival,
        GoalEvidence::EMPTY,
        0.8,
        &ctx,
    );
    let before = p
        .mind
        .goals()
        .get(GoalKind::PlayFirstTeamFootball)
        .unwrap()
        .progress();

    for week in 1..20u16 {
        let ctx = p.mind_context(
            Fixture::date(2026, 1, 1) + Duration::days(week as i64 * 7),
            Some(CLUB),
        );
        p.mind.tick(&ctx);
    }

    let after = p
        .mind
        .goals()
        .get(GoalKind::PlayFirstTeamFootball)
        .unwrap()
        .progress();
    assert_eq!(after, before, "no situation, no thinking");
}

#[test]
fn the_faculties_form_wants_from_the_world_rather_than_from_a_bridge() {
    // Phase 3 could only produce wants the legacy desire path handed it.
    // The faculties read the situation themselves.
    let mut p = Fixture::player();
    Weeks::run(&mut p, &Fixture::benched(), 12, Fixture::date(2026, 1, 1));

    assert!(
        p.mind.pressure_of(GoalKind::WinBackMyPlace) > 0.0,
        "nothing fed this in — the competitive mind saw it"
    );
    assert!(
        p.transfer_request_reasons.is_empty(),
        "and the legacy path was never involved"
    );
}

#[test]
fn a_player_tries_to_win_his_place_back_before_he_gives_up_on_it() {
    let mut p = Fixture::player();
    Weeks::run(&mut p, &Fixture::benched(), 6, Fixture::date(2026, 1, 1));
    assert!(p.mind.pressure_of(GoalKind::WinBackMyPlace) > 0.0);
    assert_eq!(
        p.mind.pressure_of(GoalKind::PlayFirstTeamFootball),
        0.0,
        "he has not stopped believing yet"
    );

    // Dropped, repeatedly, until he has.
    let start = Fixture::date(2027, 1, 1);
    for week in 0..8u16 {
        let ctx = p.mind_context(start + Duration::days(week as i64 * 7), Some(CLUB));
        p.mind
            .remember(EpisodeKind::DroppedToBench, ActorRef::staff(COACH), &ctx);
    }
    Weeks::run(&mut p, &Fixture::benched(), 6, start + Duration::days(60));

    assert!(
        p.mind.pressure_of(GoalKind::PlayFirstTeamFootball) > 0.0,
        "now he wants to go somewhere he would play"
    );
}

#[test]
fn a_change_of_manager_can_rescue_a_career() {
    let mut p = Fixture::player();
    let start = Fixture::date(2026, 1, 1);

    // Frozen out, repeatedly, by one man.
    for week in 0..8u16 {
        let ctx = p.mind_context(start + Duration::days(week as i64 * 7), Some(CLUB));
        p.mind
            .remember(EpisodeKind::ManagerFrozenOut, ActorRef::staff(COACH), &ctx);
    }
    let day = Weeks::run(&mut p, &Fixture::benched(), 8, start + Duration::days(60));
    assert!(p.mind.professional.feels_rated() < 0.0);

    // Somebody else takes over.
    let under_someone_else = MindSituation {
        manager: ActorRef::staff(777),
        ..Fixture::benched()
    };
    Weeks::run(&mut p, &under_someone_else, 1, day);

    assert_eq!(
        p.mind.professional.feels_rated(),
        0.0,
        "the read of the new man starts from nothing"
    );
    assert!(
        p.mind.pressure_of(GoalKind::WinTheManagersTrust) > 0.0,
        "and a fresh start is a reason to fight for it"
    );
}

#[test]
fn a_grudge_stays_with_the_man_it_belongs_to() {
    let mut p = Fixture::player();
    let start = Fixture::date(2026, 1, 1);
    for week in 0..6u16 {
        let ctx = p.mind_context(start + Duration::days(week as i64 * 7), Some(CLUB));
        p.mind.remember(
            EpisodeKind::ManagerPromiseBroken,
            ActorRef::staff(COACH),
            &ctx,
        );
    }
    let day = Weeks::run(&mut p, &Fixture::situation(), 8, start + Duration::days(50));

    let later = p.mind_context(day, Some(CLUB));
    assert!(
        p.mind.standing_with(ActorRef::staff(COACH), &later) < 0.0,
        "the man who broke his word is still that man"
    );

    // A successor arrives and inherits nothing.
    let successor = MindSituation {
        manager: ActorRef::staff(777),
        ..Fixture::situation()
    };
    Weeks::run(&mut p, &successor, 4, day);
    assert!(
        p.mind.standing_with(ActorRef::staff(777), &later) >= 0.0,
        "and his successor is not paying for it"
    );
}

#[test]
fn wanting_to_stay_pushes_back_on_wanting_to_go() {
    // Career and social faculties in genuine opposition, through the
    // real fan-out.
    let mut p = Fixture::player();
    let start = Fixture::date(2026, 1, 1);

    // A decade at a club he loves, with the memories to match.
    for week in 0..6u16 {
        let ctx = p.mind_context(start + Duration::days(week as i64 * 7), Some(CLUB));
        p.mind
            .remember(EpisodeKind::FansAdoration, ActorRef::fans(CLUB), &ctx);
        p.mind
            .remember(EpisodeKind::SquadBackedHim, ActorRef::NONE, &ctx);
    }
    let ctx = p.mind_context(start, Some(CLUB));
    p.mind
        .remember(EpisodeKind::SeniorDebut, ActorRef::NONE, &ctx);
    p.mind
        .remember(EpisodeKind::WonLeagueTitle, ActorRef::NONE, &ctx);

    // An ambitious man, a long time at a small club: the career mind
    // wants out, the social mind wants to stay.
    let torn = MindSituation {
        days_at_club: 3000,
        ambition: 18.0,
        club_reputation: 0.35,
        ..Fixture::situation()
    };
    Weeks::run(&mut p, &torn, 20, start + Duration::days(60));

    assert!(
        p.mind.pressure_of(GoalKind::StepUpToABiggerClub) > 0.0,
        "the ambition is real"
    );
    assert!(
        p.mind.pressure_of(GoalKind::StayAtThisClub) > 0.0,
        "and so is the attachment"
    );
    assert!(
        p.mind.wants_to_leave() < p.mind.pressure_of(GoalKind::StepUpToABiggerClub),
        "the pull out is netted off by the reason to stay"
    );
}

#[test]
fn a_move_resets_belonging_and_the_manager_read_but_not_the_career() {
    let mut p = Fixture::player();
    let start = Fixture::date(2026, 1, 1);
    let ctx = p.mind_context(start, Some(CLUB));
    p.mind
        .remember(EpisodeKind::WonLeagueTitle, ActorRef::NONE, &ctx);
    p.mind
        .remember(EpisodeKind::SquadBackedHim, ActorRef::NONE, &ctx);
    p.mind.remember(
        EpisodeKind::ManagerPrivateBacking,
        ActorRef::staff(COACH),
        &ctx,
    );

    assert!(p.mind.social.belonging() > 0.0);
    assert!(p.mind.professional.feels_rated() > 0.0);
    assert_eq!(p.mind.career.honours, 1);

    p.mind.on_club_change(CLUB);

    assert_eq!(p.mind.social.belonging(), 0.0, "belonging is about a place");
    assert_eq!(
        p.mind.professional.feels_rated(),
        0.0,
        "and a read of a manager is about a person"
    );
    assert_eq!(
        p.mind.career.honours, 1,
        "a career is continuous — he still won that"
    );
}

#[test]
fn the_appraisal_covers_five_axes_and_says_how_much_it_knows() {
    let mut p = Fixture::player();
    let empty = p.mind.appraise();
    assert!(
        empty.coverage() < 0.5,
        "a player with no history is barely readable, and the model should say so"
    );

    let start = Fixture::date(2026, 1, 1);
    let ctx = p.mind_context(start, Some(CLUB));
    for _ in 0..4 {
        p.mind
            .remember(EpisodeKind::ManagerFrozenOut, ActorRef::staff(COACH), &ctx);
        p.mind
            .remember(EpisodeKind::FeltIsolated, ActorRef::NONE, &ctx);
    }
    // Give the professional mind a manager to have a view about.
    Weeks::run(&mut p, &Fixture::benched(), 2, start);

    let profile = p.mind.appraise();
    assert!(profile.coverage() > empty.coverage());
    assert!(profile.net() < 0.0, "this is not a happy man");

    let worst = profile.heaviest_concern().expect("something is weighing");
    assert!(worst.weighted() < 0.0);
}

#[test]
fn the_appraisal_runs_alongside_the_existing_morale_rather_than_replacing_it() {
    let mut p = Fixture::player();
    let start = Fixture::date(2026, 1, 1);
    let ctx = p.mind_context(start, Some(CLUB));
    for _ in 0..5 {
        p.mind
            .remember(EpisodeKind::ManagerFrozenOut, ActorRef::staff(COACH), &ctx);
    }
    Weeks::run(&mut p, &Fixture::benched(), 4, start);

    // The faculties read him as unhappy...
    assert!(p.mind.appraise().net() < 0.0);
    // ...and `PlayerHappiness` is untouched by any of it.
    assert_eq!(
        p.happiness.morale,
        crate::club::player::behaviour_config::HappinessConfig::default().default_morale,
        "phase 4 builds the replacement; it does not switch anything over"
    );
}

#[test]
fn what_he_wants_still_decides_what_brands_itself_on_him() {
    // The phase-3 coupling, re-checked now that the faculties are the
    // ones forming the wants.
    let mut settled = Fixture::player();
    let mut desperate = Fixture::player();
    let start = Fixture::date(2026, 1, 1);

    Weeks::run(&mut settled, &Fixture::situation(), 16, start);
    let day = Weeks::run(&mut desperate, &Fixture::benched(), 16, start);

    let encode = |p: &mut Player| {
        let ctx = p.mind_context(day, Some(CLUB));
        p.mind
            .remember(EpisodeKind::LeftOutOfBigMatch, ActorRef::NONE, &ctx);
        p.mind
            .memory()
            .episodes
            .find(|e| e.kind == EpisodeKind::LeftOutOfBigMatch)
            .map(|e| e.encoding())
            .unwrap_or(0.0)
    };

    assert!(
        encode(&mut desperate) > encode(&mut settled),
        "being left out means more to a man fighting for his place"
    );
}

#[test]
fn a_full_career_of_thinking_never_grows_the_mind() {
    let mut p = Fixture::player();
    let mut day = Fixture::date(2026, 1, 1);
    let situations = [Fixture::situation(), Fixture::benched()];

    for season in 0..15u16 {
        let situation = &situations[(season % 2) as usize];
        day = Weeks::run(&mut p, situation, 40, day);
        if season % 3 == 2 {
            p.mind.on_club_change(CLUB);
        }
    }

    assert!(
        p.mind.goals().len() <= 12,
        "goals: {}",
        p.mind.goals().len()
    );
    assert!(p.mind.census().episodes <= 32);
    assert!(p.mind.census().facts <= 24);
}
