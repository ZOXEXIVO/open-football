//! End-to-end tests for the staff mind.
//!
//! Unit tests live beside each faculty. What is here is the behaviour
//! nobody owns on their own: a whole career through one mind, and the
//! question `docs/staff_mind.md` §4.3 asks — a manager offered his old
//! club after a decade, and what he actually remembers about the place.

use super::*;
use crate::club::mind::organs::goals::GoalStatus;
use chrono::Duration;

/// Fixture builders for these tests. Grouped on a type rather than left
/// loose so the file reads as `Fixture::season()` at every call site.
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
            loyalty: 11.0,
            pressure: 12.0,
            professionalism: 14.0,
            sportsmanship: 12.0,
            temperament: 10.0,
            consistency: 12.0,
            important_matches: 13.0,
            dirtiness: 4.0,
        }
    }

    fn context(date: NaiveDate, club: u32) -> StaffTickContext {
        StaffTickContext::new(date, club, &Self::attributes(), 50.0)
    }

    /// A season of weekly thinks.
    fn season(mind: &mut StaffMind, from: NaiveDate, club: u32, situation: &StaffSituation) {
        for week in 0..38 {
            let date = from + Duration::days(week * 7);
            let mut current = *situation;
            current.season_progress = week as f32 / 38.0;
            current.months_in_the_job = situation.months_in_the_job + (week as u16 / 4);
            mind.tick_with(&Self::context(date, club), &current);
        }
    }

    /// A manager a year or two into a job that is going to plan.
    fn settled_at(club_standing: f32, board_trust: f32) -> StaffSituation {
        let mut situation = StaffSituation::neutral();
        situation.age = 44.0;
        situation.standing = 0.5;
        situation.club_standing = club_standing;
        situation.board_trust = board_trust;
        situation.board_backing = 0.5;
        situation.league_size = 20;
        situation.expected_position = 10;
        situation.league_position = 10;
        situation.squad_is_his = 0.5;
        situation.dressing_room = 0.6;
        situation.terraces = 0.6;
        situation
    }
}

#[test]
fn the_ten_year_return() {
    // The question §4.3 poses: a manager is offered his old club a
    // decade after they sacked him. What does he remember?
    let mut mind = StaffMind::new();
    let old_club = 7u32;

    let arrival = Fixture::date(2030, 6, 1);
    mind.remember(
        EpisodeKind::AppointedManager,
        ActorRef::club(old_club),
        &Fixture::context(arrival, old_club),
    );

    // Two good seasons: a promotion and a cup.
    let mut good = Fixture::settled_at(0.30, 0.75);
    good.league_position = 3;
    Fixture::season(&mut mind, Fixture::date(2030, 8, 1), old_club, &good);
    mind.remember(
        EpisodeKind::Promoted,
        ActorRef::club(old_club),
        &Fixture::context(Fixture::date(2031, 5, 20), old_club),
    );
    Fixture::season(&mut mind, Fixture::date(2031, 8, 1), old_club, &good);
    mind.remember(
        EpisodeKind::WonDomesticCup,
        ActorRef::club(old_club),
        &Fixture::context(Fixture::date(2032, 5, 20), old_club),
    );

    // Then four windows of being told no, and two promises broken.
    let mut starved = Fixture::settled_at(0.30, 0.45);
    starved.board_backing = 0.1;
    starved.league_position = 15;
    for window in 0..4 {
        mind.remember(
            EpisodeKind::BoardRefusedMyTarget,
            ActorRef::board(old_club),
            &Fixture::context(
                Fixture::date(2032, 7, 1) + Duration::days(window * 60),
                old_club,
            ),
        );
    }
    for promise in 0..2 {
        mind.remember(
            EpisodeKind::BoardBrokeItsPromise,
            ActorRef::board(old_club),
            &Fixture::context(
                Fixture::date(2032, 7, 10) + Duration::days(promise * 10),
                old_club,
            ),
        );
    }
    Fixture::season(&mut mind, Fixture::date(2032, 8, 1), old_club, &starved);

    // And the sacking.
    let sacking = Fixture::date(2033, 4, 12);
    mind.remember(
        EpisodeKind::SackedByClub,
        ActorRef::club(old_club),
        &Fixture::context(sacking, old_club),
    );
    mind.on_club_change(old_club);

    // Ten years elsewhere. Consolidation keeps running; the episodes of
    // that spell fade to nothing.
    let elsewhere = 9u32;
    mind.remember(
        EpisodeKind::AppointedManager,
        ActorRef::club(elsewhere),
        &Fixture::context(Fixture::date(2033, 7, 1), elsewhere),
    );
    let quiet = Fixture::settled_at(0.5, 0.6);
    for year in 0..10 {
        Fixture::season(
            &mut mind,
            Fixture::date(2033, 8, 1) + Duration::days(year * 365),
            elsewhere,
            &quiet,
        );
    }

    // The approach comes in.
    let today = Fixture::context(Fixture::date(2043, 5, 30), elsewhere);
    let club = ActorRef::club(old_club);

    assert!(
        mind.believes(FactClaim::IBuiltSomethingThere, club) > 0.0,
        "he took them up and won a cup"
    );
    assert!(
        mind.believes(FactClaim::TheyNeverBackedMe, club) > 0.0,
        "four windows of being told no is what he carries"
    );
    assert!(
        mind.believes(FactClaim::TheySackedMe, club) > 0.0,
        "and being sacked needs to happen exactly once"
    );

    // The board is judged separately from the badge — which is what
    // makes a change of chairman a reason to look at a place again.
    let board = ActorRef::board(old_club);
    assert!(
        mind.believes(FactClaim::TheirWordIsWorthless, board) > 0.0,
        "two broken promises is what he holds against the people, not the club"
    );

    // And the account with them is still open a decade on, because the
    // conviction holds it against the drift back to neutral.
    let standing = mind.standing_with(board, &today);
    assert!(
        standing < -0.2,
        "the balance with that boardroom is still sour after ten years: {standing}"
    );

    // And a decision made through it has something to say — which is the
    // whole gap this closes. Today `candidate_accepts_terms` decides
    // this with three booleans and no memory at all.
    let verdict = mind.deliberate(MindOption::TakeTheJob(old_club));
    assert!(!verdict.is_empty());
}

#[test]
fn a_manager_offered_the_club_that_backed_him_reads_it_differently() {
    let mut backed = StaffMind::new();
    let mut starved = StaffMind::new();
    let club = 7u32;

    let arrival = Fixture::date(2030, 6, 1);
    for mind in [&mut backed, &mut starved] {
        mind.remember(
            EpisodeKind::AppointedManager,
            ActorRef::club(club),
            &Fixture::context(arrival, club),
        );
    }
    for window in 0..4 {
        let date = Fixture::date(2030, 7, 1) + Duration::days(window * 60);
        backed.remember(
            EpisodeKind::BoardBackedMeInTheWindow,
            ActorRef::board(club),
            &Fixture::context(date, club),
        );
        starved.remember(
            EpisodeKind::BoardRefusedMyTarget,
            ActorRef::board(club),
            &Fixture::context(date, club),
        );
    }

    let settled = Fixture::settled_at(0.4, 0.6);
    for mind in [&mut backed, &mut starved] {
        Fixture::season(mind, Fixture::date(2032, 8, 1), club, &settled);
        mind.on_club_change(club);
    }

    let backed_view = backed.deliberate(MindOption::TakeTheJob(club));
    let starved_view = starved.deliberate(MindOption::TakeTheJob(club));

    assert!(
        backed_view.net() > starved_view.net(),
        "the same badge, two very different jobs: {} vs {}",
        backed_view.net(),
        starved_view.net()
    );
}

#[test]
fn ambition_alone_never_makes_a_manager_walk_out() {
    // The staff-side mirror of the player-side property: a want with no
    // grievance behind it and no time accrued does not reach a formal
    // demand. A manager doing well at a small club is restless, not
    // agitating.
    let mut mind = StaffMind::new();
    let mut thriving = Fixture::settled_at(0.25, 0.9);
    thriving.standing = 0.8;
    thriving.league_position = 3;
    thriving.expected_position = 14;

    mind.remember(
        EpisodeKind::AppointedManager,
        ActorRef::club(7),
        &Fixture::context(Fixture::date(2030, 6, 1), 7),
    );
    for year in 0..3 {
        Fixture::season(
            &mut mind,
            Fixture::date(2030, 8, 1) + Duration::days(year * 365),
            7,
            &thriving,
        );
    }

    assert!(
        mind.pressure_of(GoalKind::GetABiggerJob) > 0.0,
        "he is looking"
    );
    assert!(
        !mind.is_pressing(),
        "but nothing has been formally demanded: {:?}",
        mind.strongest_goal().map(|g| (g.kind, g.status))
    );
}

#[test]
fn the_stay_goals_are_a_real_counterweight() {
    // Left alone, ambition churns managers every season. A manager
    // mid-build should resist an approach he would have taken with an
    // inherited squad.
    let mut mid_build = StaffMind::new();
    let mut caretaker = StaffMind::new();

    let mut situation = Fixture::settled_at(0.3, 0.8);
    situation.standing = 0.75;
    situation.league_position = 4;
    situation.expected_position = 13;

    let mut his_own = situation;
    his_own.squad_is_his = 0.95;
    let mut inherited = situation;
    inherited.squad_is_his = 0.05;

    for year in 0..3 {
        let from = Fixture::date(2030, 8, 1) + Duration::days(year * 365);
        Fixture::season(&mut mid_build, from, 7, &his_own);
        Fixture::season(&mut caretaker, from, 7, &inherited);
    }

    let stays = mid_build.deliberate(MindOption::TakeTheJob(31));
    let goes = caretaker.deliberate(MindOption::TakeTheJob(31));

    assert!(
        stays.net() < goes.net(),
        "a man in the middle of something is harder to move: {} vs {}",
        stays.net(),
        goes.net()
    );
}

#[test]
fn a_whole_career_stays_inside_the_footprint() {
    // Twenty years, four clubs, a full judgement store. Nothing
    // allocates and nothing grows.
    let mut mind = StaffMind::new();
    let situation = Fixture::settled_at(0.5, 0.6);

    for job in 0..4u32 {
        let club = 7 + job;
        let arrival = Fixture::date(2030, 6, 1) + Duration::days(job as i64 * 5 * 365);
        mind.remember(
            EpisodeKind::AppointedManager,
            ActorRef::club(club),
            &Fixture::context(arrival, club),
        );

        for squad_player in 0..30u32 {
            let player = ActorRef::player(job * 100 + squad_player);
            mind.form_judgement(player, 0.5, 0.7, &Fixture::context(arrival, club));
            mind.watched(player, 6.8, false, &Fixture::context(arrival, club));
        }

        for year in 0..5 {
            Fixture::season(
                &mut mind,
                arrival + Duration::days(60 + year * 365),
                club,
                &situation,
            );
        }
        mind.remember(
            EpisodeKind::SackedByClub,
            ActorRef::club(club),
            &Fixture::context(arrival + Duration::days(5 * 365), club),
        );
        mind.on_club_change(club);
    }

    let census = mind.census();
    assert!(census.episodes <= 32, "the episode store is bounded");
    assert!(census.facts <= 24, "and so is the semantic store");
    assert!(
        mind.organs.judgements.len() <= 48,
        "and so is the judgement store"
    );
    assert!(
        census.facts > 0,
        "but twenty years taught him something: {census:?}"
    );
}

#[test]
fn a_goal_climbs_the_ladder_one_rung_at_a_time() {
    let mut mind = StaffMind::new();
    let mut sinking = Fixture::settled_at(0.5, 0.1);
    sinking.league_position = 19;
    sinking.expected_position = 8;
    sinking.board_pressure = 0.9;

    mind.remember(
        EpisodeKind::AppointedManager,
        ActorRef::club(7),
        &Fixture::context(Fixture::date(2030, 6, 1), 7),
    );

    let mut rungs = Vec::new();
    for week in 0..30 {
        let date = Fixture::date(2030, 8, 1) + Duration::days(week * 7);
        let mut current = sinking;
        current.season_progress = (week as f32 / 38.0).min(1.0);
        mind.tick_with(&Fixture::context(date, 7), &current);
        rungs.push(mind.goals().status_of(GoalKind::KeepThisJob).rung());
    }

    // Never more than one rung between consecutive reviews. A mind that
    // flips state every week is not a mind.
    for pair in rungs.windows(2) {
        assert!(
            pair[1].abs_diff(pair[0]) <= 1,
            "a want moved {} rungs in one review",
            pair[1].abs_diff(pair[0])
        );
    }
    assert_ne!(
        mind.goals().status_of(GoalKind::KeepThisJob),
        GoalStatus::Latent,
        "and it did climb"
    );
}
