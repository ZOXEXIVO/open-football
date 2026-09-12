//! End-to-end tests for the dossier layer.
//!
//! Unit tests live beside each piece. What is here is the behaviour nobody
//! owns on their own — a working relationship from the first match to a
//! reunion years later.

use super::*;
use crate::club::mind::organs::memory::EpochDay;
use crate::club::staff::{DossierTuning, StaffStub};
use crate::club::staff::coach::memory::{CoachMatchObservation, CoachMemoryFlags};
use crate::club::staff::coach::plan::PlannedRole;
use crate::club::staff::coach::standing::{GrievanceFlags, StandingEvidence};
use crate::club::staff::perception::CoachProfile;
use crate::Staff;
use crate::club::mind::organs::memory::MindClock;

const TODAY: EpochDay = 10_000;
const YEAR: EpochDay = 365;

/// Fixture builders, grouped so the tests read as sentences.
struct Fx;

impl Fx {
    fn store() -> CoachDossierStore {
        CoachDossierStore::new()
    }
}

#[test]
fn a_working_relationship_opens_runs_and_closes() {
    let mut store = Fx::store();
    assert_eq!(
        Dossiers::open(&mut store, 11, 3, TODAY - 2 * YEAR),
        SpellOpening::Fresh
    );
    assert!(Dossiers::is_working_with(&store, 11));

    let record = Dossiers::of_mut(&mut store, 11).unwrap();
    record.add_matches(64);
    record.note_role(PlannedRole::Starter);
    record.set_standing(0.5);
    record.set_warmth(0.55);
    record.medals.insert(MedalFlags::CORNERSTONE);
    record.close(SeparationCause::IMovedOn, 3, 29, TODAY);

    assert!(!Dossiers::is_working_with(&store, 11));
    let held = Dossiers::of(&store, 11).unwrap();
    assert_eq!(held.matches_together, 64);
    assert_eq!(held.peak_role(), Some(PlannedRole::Starter));
    assert_eq!(held.parted, SeparationCause::IMovedOn);
}

#[test]
fn the_years_together_survive_every_parting_and_reunion() {
    let mut store = Fx::store();
    Dossiers::open(&mut store, 11, 3, TODAY - 6 * YEAR);
    Dossiers::of_mut(&mut store, 11).unwrap().add_matches(40);
    Dossiers::of_mut(&mut store, 11)
        .unwrap()
        .close(SeparationCause::SoldByBoard, 3, 25, TODAY - 4 * YEAR);

    Dossiers::open(&mut store, 11, 9, TODAY - 2 * YEAR);
    Dossiers::of_mut(&mut store, 11).unwrap().add_matches(30);
    Dossiers::of_mut(&mut store, 11)
        .unwrap()
        .close(SeparationCause::IWasSacked, 9, 27, TODAY);

    let held = Dossiers::of(&store, 11).unwrap();
    assert_eq!(held.spells, 2);
    assert_eq!(held.matches_together, 70);
    assert_eq!(held.last_club, 9);
}

#[test]
fn a_coach_with_no_history_has_nothing_to_say_about_anybody() {
    let store = Fx::store();
    assert!(Dossiers::of(&store, 1).is_none());
    assert!(!Dossiers::is_working_with(&store, 1));
    assert_eq!(store.census(TODAY), DossierCensus::default());
    let warmest: [Option<u32>; 3] = Dossiers::warmest(&store, TODAY);
    assert_eq!(warmest, [None, None, None]);
}

// ── The spell lifecycle ─────────────────────────────────────────

/// Fixtures for the closer, which needs a whole `Staff` rather than a bare
/// store.
struct Spell;

impl Spell {
    fn today() -> chrono::NaiveDate {
        chrono::NaiveDate::from_ymd_opt(2030, 6, 1).expect("valid fixture date")
    }

    fn coach() -> Staff {
        let mut staff = StaffStub::default();
        staff.id = 77;
        staff.staff_attributes.mental.man_management = 12;
        staff.staff_attributes.knowledge.judging_player_ability = 13;
        staff.attributes.loyalty = 12.0;
        staff.attributes.temperament = 10.0;
        staff
    }

    fn observation(player_id: u32, rating: f32, day: i64) -> CoachMatchObservation {
        CoachMatchObservation {
            player_id,
            effective_rating: rating,
            minutes_played: 90,
            is_starter: true,
            match_importance: 0.7,
            is_cup: false,
            is_derby: false,
            is_continental: false,
            goals: 0,
            assists: 0,
            errors_leading_to_goal: 0,
            yellow_cards: 0,
            red_cards: 0,
            team_won: rating >= 6.5,
            was_substituted_early: false,
            role_fit: 1.0,
            professionalism_signal: 0.7,
            date: Self::today() - chrono::Duration::days(400 - day),
        }
    }

    /// A coach who has worked with `player_id` for `matches` at `rating`.
    fn worked_with(player_id: u32, matches: i64, rating: f32) -> Staff {
        let mut coach = Self::coach();
        coach.player_joined(player_id, 3, Self::today() - chrono::Duration::days(400));
        let profile = CoachProfile::from_staff(&coach);
        for day in 0..matches {
            coach
                .coach_memory
                .observe(&Self::observation(player_id, rating, day * 7), &profile);
        }
        coach
    }
}

#[test]
fn selling_a_player_closes_the_spell_and_opens_a_dossier() {
    let mut coach = Spell::worked_with(9, 20, 7.1);
    assert!(coach.coach_memory.get(9).is_some(), "a live record");

    let report = PartingReport::bare(9, 3, 27);
    coach.player_left(&report, SeparationCause::SoldByBoard, Spell::today());

    let held = Dossiers::of(&coach.dossiers, 9).expect("a dossier survives");
    assert!(!held.open);
    assert_eq!(held.parted, SeparationCause::SoldByBoard);
    assert_eq!(held.matches_together, 20);
    assert!(
        held.long_form() > 6.5,
        "he remembers roughly what the man was worth: {}",
        held.long_form()
    );
}

#[test]
fn hot_memory_is_gone_once_the_dossier_is_written() {
    let mut coach = Spell::worked_with(9, 12, 6.8);
    coach.squad_plan.force_role(9, PlannedRole::Starter, Spell::today());
    assert!(coach.squad_plan.role_of(9).is_some());

    let report = PartingReport::bare(9, 3, 27);
    coach.player_left(&report, SeparationCause::SoldOnMyCall, Spell::today());

    assert!(
        coach.coach_memory.get(9).is_none(),
        "a form average for a man who has left is not an opinion"
    );
    assert!(
        coach.squad_plan.role_of(9).is_none(),
        "nor is a plan for him"
    );
    assert!(Dossiers::of(&coach.dossiers, 9).is_some());
}

#[test]
fn a_sacked_coach_carries_every_dossier_out_of_the_door() {
    let mut coach = Spell::coach();
    for player_id in 1..=5u32 {
        coach.player_joined(player_id, 3, Spell::today() - chrono::Duration::days(300));
    }
    assert_eq!(coach.dossiers.open_spells(), 5);

    coach.leave_club_as(3, SeparationCause::IWasSacked, Spell::today());

    assert_eq!(coach.dossiers.len(), 5, "he remembers all of them");
    assert_eq!(coach.dossiers.open_spells(), 0, "and works with none");
    for player_id in 1..=5u32 {
        assert_eq!(
            Dossiers::of(&coach.dossiers, player_id).unwrap().parted,
            SeparationCause::IWasSacked
        );
    }
}

#[test]
fn a_loan_suspends_rather_than_closes() {
    let mut coach = Spell::worked_with(9, 10, 6.4);
    let report = PartingReport::bare(9, 3, 20);
    coach.player_left(&report, SeparationCause::LoanedOutByMe, Spell::today());

    assert!(
        Dossiers::is_working_with(&coach.dossiers, 9),
        "he is still the manager's player"
    );
    assert!(
        coach.coach_memory.get(9).is_some(),
        "and the manager still has a view of him"
    );
    assert!(
        coach
            .coach_memory
            .get(9)
            .unwrap()
            .flags
            .contains(CoachMemoryFlags::AWAY_ON_LOAN)
    );
}

#[test]
fn a_good_loan_comes_home_as_evidence() {
    let mut coach = Spell::worked_with(9, 6, 6.0);
    let before = coach.coach_memory.get(9).unwrap().long_form_rating;
    coach.player_left(
        &PartingReport::bare(9, 3, 20),
        SeparationCause::LoanedOutByMe,
        Spell::today(),
    );

    SpellCloser::resume_from_loan(&mut coach, 9, 28, 7.1, Spell::today());

    let after = coach.coach_memory.get(9).unwrap();
    assert!(
        after.long_form_rating > before,
        "twenty-eight good games move the read: {before} → {}",
        after.long_form_rating
    );
    assert!(
        after.long_form_rating < 7.1,
        "but not as far as if he had watched them"
    );
    assert!(!after.flags.contains(CoachMemoryFlags::AWAY_ON_LOAN));
}

#[test]
fn a_man_he_rated_walking_out_is_remembered_as_a_different_parting() {
    let mut loyal = Spell::worked_with(9, 25, 7.4);
    let mut easy = Spell::worked_with(9, 25, 7.4);
    // Push both to a standing where the coach was picking him.
    for coach in [&mut loyal, &mut easy] {
        coach.coach_memory.standing_of_mut(9).unwrap().score = 0.5;
    }

    loyal.player_left(
        &PartingReport::bare(9, 3, 26),
        SeparationCause::HeRequestedOut,
        Spell::today(),
    );
    easy.player_left(
        &PartingReport::bare(9, 3, 26),
        SeparationCause::SoldByBoard,
        Spell::today(),
    );

    let walked = Dossiers::of(&loyal.dossiers, 9).unwrap();
    let sold = Dossiers::of(&easy.dossiers, 9).unwrap();
    assert!(
        walked.warmth() < sold.warmth(),
        "asking to go costs him: walked={} sold={}",
        walked.warmth(),
        sold.warmth()
    );
    assert!(walked.scars.contains(ScarFlags::WANTED_OUT));
    assert!(sold.scars.is_empty());
}

#[test]
fn a_long_clean_spell_leaves_a_coach_warm_and_a_sour_one_leaves_him_cold() {
    let mut warm = Spell::worked_with(9, 40, 7.3);
    warm.coach_memory.standing_of_mut(9).unwrap().score = 0.6;
    warm.player_left(
        &PartingReport::bare(9, 3, 29),
        SeparationCause::HeRetired,
        Spell::today(),
    );

    let mut cold = Spell::worked_with(8, 40, 5.2);
    {
        let standing = cold.coach_memory.standing_of_mut(8).unwrap();
        standing.score = -0.8;
        standing.grievance.insert(GrievanceFlags::REFUSED);
    }
    cold.player_left(
        &PartingReport::bare(8, 3, 29),
        SeparationCause::SoldOnMyCall,
        Spell::today(),
    );

    assert!(Dossiers::of(&warm.dossiers, 9).unwrap().warmth() > 0.3);
    assert!(Dossiers::of(&cold.dossiers, 8).unwrap().warmth() < -0.2);
    assert!(
        Dossiers::of(&cold.dossiers, 8)
            .unwrap()
            .scars
            .contains(ScarFlags::REFUSED_TO_PLAY)
    );
}

// ── Meeting again ───────────────────────────────────────────────

/// Fixtures for reunions: a coach with a closed record, and a way to put
/// the two of them back together `years` later.
struct Again;

impl Again {
    fn coach_who_parted(
        player_id: u32,
        matches: u16,
        warmth: f32,
        standing: f32,
        long_form: f32,
        age_at_parting: u8,
        years_ago: i64,
    ) -> Staff {
        let mut coach = Spell::coach();
        let parted = Spell::today() - chrono::Duration::days(years_ago * 365);
        coach.player_joined(player_id, 3, parted - chrono::Duration::days(700));
        let record = Dossiers::of_mut(&mut coach.dossiers, player_id).unwrap();
        record.add_matches(matches);
        record.set_warmth(warmth);
        record.set_standing(standing);
        record.set_reads(0.72, 0.78, long_form, 0.7, 0.6, 0.6, 0.7);
        record.note_role(PlannedRole::Starter);
        record.close(
            SeparationCause::IMovedOn,
            3,
            age_at_parting,
            MindClock::day(parted),
        );
        coach
    }
}

#[test]
fn a_reunion_seeds_memory_in_proportion_to_the_prior() {
    let mut recent = Again::coach_who_parted(9, 90, 0.6, 0.5, 7.2, 27, 1);
    let mut distant = Again::coach_who_parted(9, 90, 0.6, 0.5, 7.2, 27, 8);

    recent.player_joined_at(9, 21, 28, Spell::today());
    distant.player_joined_at(9, 21, 35, Spell::today());

    let fresh = recent.coach_memory.get(9).expect("seeded");
    let faded = distant.coach_memory.get(9).expect("seeded");

    assert!(
        fresh.prior_at_seed > faded.prior_at_seed * 2.0,
        "one year against eight: {} vs {}",
        fresh.prior_at_seed,
        faded.prior_at_seed
    );
    assert!(
        fresh.matches_observed > faded.matches_observed,
        "and he is entitled to an opinion sooner"
    );
    assert!(
        fresh.tactical_trust > faded.tactical_trust,
        "trust starts further along for the man he saw recently"
    );
    for memory in [fresh, faded] {
        assert!(memory.flags.contains(CoachMemoryFlags::KNOWN_QUANTITY));
    }
}

#[test]
fn a_manager_brings_his_cornerstone_and_starts_him_on_trust() {
    let mut coach = Again::coach_who_parted(9, 90, 0.65, 0.5, 7.3, 27, 2);
    coach.player_joined_at(9, 21, 29, Spell::today());

    let standing = coach.coach_memory.standing_of(9).expect("seeded");
    assert!(
        standing.rung.is_favoured(),
        "he walks in already in the side: {:?}",
        standing.rung
    );
    assert!(
        coach
            .squad_plan
            .role_of(9)
            .is_some_and(|role| role.is_at_least(PlannedRole::Rotation)),
        "and with a role his history entitles him to"
    );
}

#[test]
fn a_known_quantity_is_re_rated_slower_than_a_stranger() {
    let mut known = Again::coach_who_parted(9, 90, 0.6, 0.5, 7.2, 27, 1);
    known.player_joined_at(9, 21, 28, Spell::today());
    let mut stranger = Spell::coach();
    stranger.player_joined(9, 21, Spell::today());

    let profile = CoachProfile::from_staff(&known);
    // Both watch the same four poor matches.
    for day in 0..4 {
        let observation = Spell::observation(9, 5.2, 380 + day * 7);
        known.coach_memory.observe(&observation, &profile);
        stranger.coach_memory.observe(&observation, &profile);
    }

    let known_read = known.coach_memory.get(9).unwrap().long_form_rating;
    let stranger_read = stranger.coach_memory.get(9).unwrap().long_form_rating;
    assert!(
        known_read > stranger_read,
        "the man he knows is not re-rated on four games: known={known_read} stranger={stranger_read}"
    );
}

#[test]
fn a_reunion_with_a_man_he_froze_out_starts_under_a_cloud() {
    let mut coach = Again::coach_who_parted(9, 40, -0.5, -0.6, 5.8, 26, 3);
    {
        let record = Dossiers::of_mut(&mut coach.dossiers, 9).unwrap();
        record.scars.insert(ScarFlags::REFUSED_TO_PLAY);
        record.refresh_scar_strength(10.0);
    }
    coach.player_joined_at(9, 21, 29, Spell::today());

    let standing = coach.coach_memory.standing_of(9).expect("seeded");
    assert!(
        standing.rung.is_out(),
        "a second chance, and a watched one: {:?}",
        standing.rung
    );
    assert!(standing.grievance.contains(GrievanceFlags::REFUSED));
}

#[test]
fn five_years_and_a_thirtieth_birthday_leave_only_the_warmth() {
    let mut coach = Again::coach_who_parted(9, 90, 0.8, 0.6, 7.4, 27, 5);
    let record = *Dossiers::of(&coach.dossiers, 9).unwrap();
    let day = MindClock::day(Spell::today());

    coach.player_joined_at(9, 21, 32, Spell::today());
    let seeded = coach.coach_memory.get(9).unwrap();

    assert!(
        record.warmth_now(day) > 0.4,
        "he still likes him: {}",
        record.warmth_now(day)
    );
    assert!(
        seeded.prior_at_seed < 0.3,
        "but does not trust the old read: {}",
        seeded.prior_at_seed
    );
    assert!(
        seeded.tactical_trust < 0.6,
        "so the trust axes sit near neutral: {}",
        seeded.tactical_trust
    );
}

#[test]
fn a_first_spell_seeds_nothing() {
    let mut coach = Spell::coach();
    assert_eq!(
        coach.player_joined(9, 3, Spell::today()),
        SpellOpening::Fresh
    );
    assert!(
        coach.coach_memory.get(9).is_none(),
        "a man he has never seen gets no head start"
    );
}

// ── Wanting him again ───────────────────────────────────────────

#[test]
fn a_coach_asks_for_the_player_he_made() {
    let mut coach = Again::coach_who_parted(9, 90, 0.7, 0.6, 7.4, 26, 2);
    {
        let record = Dossiers::of_mut(&mut coach.dossiers, 9).unwrap();
        record.medals.insert(MedalFlags::CORNERSTONE);
        record.medals.insert(MedalFlags::MADE_UNDER_ME);
    }
    let affinity = coach.affinity_for(9, Spell::today());
    assert!(
        affinity >= DossierTuning::AFFINITY_REQUEST_MIN,
        "he would ring him first: {affinity}"
    );
}

#[test]
fn a_loyal_coach_never_signs_a_man_who_refused_to_play_for_him() {
    let mut coach = Again::coach_who_parted(9, 30, -0.8, -0.9, 6.0, 26, 2);
    {
        let record = Dossiers::of_mut(&mut coach.dossiers, 9).unwrap();
        record.scars.insert(ScarFlags::REFUSED_TO_PLAY);
        record.scars.insert(ScarFlags::WENT_PUBLIC);
        record.refresh_scar_strength(14.0);
    }
    let affinity = coach.affinity_for(9, Spell::today());
    assert!(
        affinity <= DossierTuning::AFFINITY_VETO,
        "not at any price: {affinity}"
    );
}

#[test]
fn a_man_he_never_worked_with_is_neither_wanted_nor_refused() {
    let coach = Spell::coach();
    assert_eq!(coach.affinity_for(4321, Spell::today()), 0.0);
}

#[test]
fn time_cools_an_old_grudge_but_never_a_poisonous_one() {
    let mut recent = Again::coach_who_parted(9, 30, -0.6, -0.7, 6.0, 26, 1);
    let mut ancient = Again::coach_who_parted(9, 30, -0.6, -0.7, 6.0, 26, 9);
    for coach in [&mut recent, &mut ancient] {
        let record = Dossiers::of_mut(&mut coach.dossiers, 9).unwrap();
        record.scars.insert(ScarFlags::ERROR_PRONE);
        record.refresh_scar_strength(10.0);
    }
    assert!(
        ancient.affinity_for(9, Spell::today()) > recent.affinity_for(9, Spell::today()),
        "nine years takes the edge off an ordinary failing"
    );

    let mut old_betrayal = Again::coach_who_parted(8, 30, -0.6, -0.7, 6.0, 26, 9);
    {
        let record = Dossiers::of_mut(&mut old_betrayal.dossiers, 8).unwrap();
        record.scars.insert(ScarFlags::REFUSED_TO_PLAY);
        record.refresh_scar_strength(10.0);
    }
    assert!(
        old_betrayal.affinity_for(8, Spell::today()) < ancient.affinity_for(9, Spell::today()),
        "and does not take it off a refusal"
    );
}

// ── Promises ────────────────────────────────────────────────────

#[test]
fn a_plan_that_promises_minutes_is_a_promise_the_player_can_hold_him_to() {
    // The coach's side: two broken assurances leave him owing a start,
    // and a man-manager pays it in a game he can afford to lose.
    let mut coach = Spell::worked_with(9, 10, 6.5);
    let standing = coach.coach_memory.standing_of_mut(9).unwrap();
    assert!(!standing.owes_a_start(0.8, 0.4), "he owes nothing yet");
    let earned = standing.score;

    StandingEvidence::promise_broken(standing);
    StandingEvidence::promise_broken(standing);
    assert!(standing.owes_a_start(0.8, 0.4));
    assert_eq!(
        standing.score, earned,
        "and the player's standing is untouched — it was not his failure"
    );
}

#[test]
fn keeping_his_word_pays_the_debt_down() {
    let mut coach = Spell::worked_with(9, 10, 6.5);
    let standing = coach.coach_memory.standing_of_mut(9).unwrap();
    StandingEvidence::promise_broken(standing);
    StandingEvidence::promise_broken(standing);
    let owed = standing.debt;

    let earned = standing.score;
    StandingEvidence::promise_kept(standing);
    assert!(standing.debt < owed);
    assert!(standing.score > earned, "and it counts for the player too");
}
