//! The goalkeeping department, end to end.
//!
//! Each test stands up a keeper room by hand and drives the public review,
//! because every claim being made here is about a group rather than about
//! one player: that the pecking order persists, that the succession opens
//! before it has to, and that a boy in the under-eighteens can be handed a
//! senior shirt he has not yet earned on ability alone.

use chrono::NaiveDate;

use super::advice::{KeeperAdvice, KeeperSuccession, KeeperTier};
use super::brief::KeeperSelectionBrief;
use super::department::{GoalkeepingDepartment, KeeperCoachAuthority};
use super::plan::KeeperRoomPlan;
use super::room::{KeeperAgeCurve, KeeperRoom};
use crate::club::player::builder::PlayerBuilder;
use crate::shared::fullname::FullName;
use crate::{
    PersonAttributes, Player, PlayerAttributes, PlayerPosition, PlayerPositionType,
    PlayerPositions, PlayerSkills, TeamType,
};

const TODAY: (i32, u32, u32) = (2026, 8, 1);

fn today() -> NaiveDate {
    NaiveDate::from_ymd_opt(TODAY.0, TODAY.1, TODAY.2).unwrap()
}

/// A goalkeeper at a chosen age and visible level.
struct KeeperFixture;

impl KeeperFixture {
    fn build(id: u32, age: u8, level: u8) -> Player {
        let mut attrs = PlayerAttributes::default();
        attrs.current_ability = level;
        attrs.condition = 9500;
        // The observable ceiling reads attitude, and a default
        // `PersonAttributes` is a professional with zero professionalism and
        // zero ambition — which credits every fixture keeper with no future
        // at all. Give them an ordinary pro's application so the ceiling
        // means something.
        let mut person = PersonAttributes::default();
        person.professionalism = 15.0;
        person.ambition = 15.0;
        PlayerBuilder::new()
            .id(id)
            .full_name(FullName::new("K".to_string(), format!("G{id}")))
            .birth_date(NaiveDate::from_ymd_opt(TODAY.0 - age as i32, 1, 1).unwrap())
            .country_id(1)
            .attributes(person)
            .skills(PlayerSkills::flat_for_ability(level))
            .positions(PlayerPositions {
                positions: vec![PlayerPosition {
                    position: PlayerPositionType::Goalkeeper,
                    level: 20,
                }],
            })
            .player_attributes(attrs)
            .build()
            .unwrap()
    }
}

/// A club's keeper room, described as squads.
struct RoomFixture {
    squads: Vec<(u32, TeamType, Player)>,
}

impl RoomFixture {
    fn new() -> Self {
        RoomFixture { squads: Vec::new() }
    }

    fn senior(mut self, id: u32, age: u8, level: u8) -> Self {
        self.squads
            .push((1, TeamType::Main, KeeperFixture::build(id, age, level)));
        self
    }

    fn academy(mut self, id: u32, age: u8, level: u8) -> Self {
        self.squads
            .push((2, TeamType::U18, KeeperFixture::build(id, age, level)));
        self
    }

    fn academy_with_season(mut self, id: u32, age: u8, level: u8, apps: u16, rating: f32) -> Self {
        let mut player = KeeperFixture::build(id, age, level);
        player.friendly_statistics.played = apps;
        player.friendly_statistics.average_rating = rating;
        self.squads.push((2, TeamType::U18, player));
        self
    }

    fn assemble(&self) -> KeeperRoom {
        KeeperRoom::assemble(self.squads.iter().map(|(id, tt, p)| (*id, *tt, p)), today())
    }
}

/// The department with a competent specialist behind it, without needing a
/// whole staff fixture.
fn authority(weight: f32, specialist_focus: f32) -> KeeperCoachAuthority {
    KeeperCoachAuthority {
        weight,
        specialist_focus,
    }
}

fn review(room: &KeeperRoom, previous: &KeeperRoomPlan) -> KeeperRoomPlan {
    let outcome = GoalkeepingDepartment::review(room, previous, authority(0.8, 0.8), today())
        .expect("a room with keepers in it produces a plan");
    let mut plan = previous.clone();
    plan.commit(outcome, today());
    plan
}

#[test]
fn the_room_is_the_whole_club_not_one_squad() {
    let fixture = RoomFixture::new()
        .senior(1, 29, 140)
        .senior(2, 27, 128)
        .academy(3, 17, 80);
    let room = fixture.assemble();

    assert_eq!(room.len(), 3, "the under-eighteens keeper is in the room");
    assert_eq!(room.seniors().count(), 2);
    assert_eq!(room.pathway().count(), 1);
    assert_eq!(room.best().map(|k| k.player_id), Some(1));
}

#[test]
fn the_number_one_keeps_his_shirt_through_a_marginal_challenger() {
    // A settled number one and a deputy three assessed points behind him:
    // the sort of gap a week's form used to be enough to flip.
    let fixture = RoomFixture::new().senior(1, 30, 140).senior(2, 27, 137);
    let room = fixture.assemble();
    let mut plan = KeeperRoomPlan::new();
    plan = review(&room, &plan);
    assert_eq!(plan.number_one(), Some(1));

    // Now the deputy edges ahead — and still does not take the shirt.
    let fixture = RoomFixture::new().senior(1, 30, 140).senior(2, 27, 143);
    let room = fixture.assemble();
    let plan = review(&room, &plan);
    assert_eq!(
        plan.number_one(),
        Some(1),
        "a three-point edge is not a change of number one"
    );
    assert_eq!(plan.deputy(), Some(2));
}

#[test]
fn a_clearly_better_keeper_does_take_the_shirt() {
    let fixture = RoomFixture::new().senior(1, 30, 140).senior(2, 27, 137);
    let mut plan = KeeperRoomPlan::new();
    plan = review(&fixture.assemble(), &plan);
    assert_eq!(plan.number_one(), Some(1));

    let fixture = RoomFixture::new().senior(1, 30, 140).senior(2, 27, 160);
    let plan = review(&fixture.assemble(), &plan);
    assert_eq!(plan.number_one(), Some(2));
    assert_eq!(plan.deputy(), Some(1));
}

#[test]
fn the_deputy_gets_the_cup_and_the_veteran_third_is_the_senior_voice() {
    let fixture = RoomFixture::new()
        .senior(1, 29, 145)
        .senior(2, 27, 132)
        .senior(3, 36, 110);
    let plan = review(&fixture.assemble(), &KeeperRoomPlan::new());

    assert_eq!(plan.tier_of(1), Some(KeeperTier::NumberOne));
    assert_eq!(plan.tier_of(2), Some(KeeperTier::Deputy));
    assert_eq!(plan.tier_of(3), Some(KeeperTier::Third));

    assert!(
        plan.advice_for(2)
            .any(|r| r.advice == KeeperAdvice::HandHimTheCup),
        "the cup is the deputy's competition"
    );
    assert!(
        plan.advice_for(3)
            .any(|r| r.advice == KeeperAdvice::KeepHimAsTheSeniorVoice),
        "an experienced third keeper is a role, not deadwood"
    );
}

#[test]
fn an_ageing_number_one_with_nobody_behind_him_is_a_critical_succession() {
    // Three keepers, all past thirty: exactly the room the sim used to
    // produce and never notice.
    let fixture = RoomFixture::new()
        .senior(1, 38, 140)
        .senior(2, 33, 120)
        .senior(3, 31, 112);
    let plan = review(&fixture.assemble(), &KeeperRoomPlan::new());

    assert_eq!(plan.succession(), KeeperSuccession::Critical);
    assert_eq!(plan.heir(), None);
    assert!(plan.wants(KeeperAdvice::SignAKeeperForTheFuture));
    assert!(
        plan.advice_for(1)
            .any(|r| r.advice == KeeperAdvice::OpenTheSuccession)
    );
}

#[test]
fn a_credible_young_keeper_is_named_the_heir_and_calms_the_clock() {
    let fixture = RoomFixture::new()
        .senior(1, 35, 140)
        .senior(2, 30, 120)
        .senior(3, 22, 124);
    let plan = review(&fixture.assemble(), &KeeperRoomPlan::new());

    assert_eq!(plan.heir(), Some(3), "the twenty-two-year-old is the heir");
    assert_eq!(
        plan.succession(),
        KeeperSuccession::Pressing,
        "past the peak band, but no longer critical with a successor in place"
    );
    assert!(
        plan.advice_for(3)
            .any(|r| r.advice == KeeperAdvice::MakeHimNumberOne)
    );
}

#[test]
fn a_promising_academy_keeper_reaches_the_pathway_and_is_nominated() {
    // The case the user asked for: a boy well short of the first team on
    // ability, playing well for his age group, whom the department wants
    // given a senior start anyway.
    let fixture = RoomFixture::new()
        .senior(1, 29, 150)
        .senior(2, 28, 138)
        .senior(3, 34, 120)
        .academy_with_season(4, 18, 108, 14, 7.4);
    let plan = review(&fixture.assemble(), &KeeperRoomPlan::new());

    assert_eq!(
        plan.tier_of(4),
        Some(KeeperTier::Pathway),
        "he trains and travels with the first team"
    );
    assert_eq!(
        plan.nominated(today()),
        Some(4),
        "and the department asks for him to be played"
    );
    assert!(
        plan.advice_for(4)
            .any(|r| r.advice == KeeperAdvice::GiveHimASeniorStart)
    );
}

#[test]
fn the_nomination_beats_a_better_keeper_in_a_dead_rubber_and_never_in_a_decider() {
    let brief = KeeperSelectionBrief {
        number_one: Some(1),
        deputy: Some(2),
        third: Some(3),
        nominated: Some(4),
        authority: 0.85,
    };

    let dead_rubber = brief.selection_adjustment(4, 0.20);
    let number_one_standing = brief.selection_adjustment(1, 0.20);
    assert!(
        dead_rubber > number_one_standing + 3.0,
        "in a fixture the club can afford, the nomination is a decision: {dead_rubber} vs {number_one_standing}"
    );

    let decider = brief.selection_adjustment(4, 0.90);
    assert_eq!(
        decider, 0.0,
        "and it is worth nothing at all when the result matters"
    );
    assert!(
        brief.selection_adjustment(1, 0.90) > 0.0,
        "the number one keeps his standing in the big games"
    );
}

#[test]
fn a_silent_brief_changes_nothing() {
    let brief = KeeperSelectionBrief::from_plan(&KeeperRoomPlan::new(), today());
    assert!(brief.is_silent());
    assert_eq!(brief.selection_adjustment(1, 0.5), 0.0);
    assert_eq!(brief.selection_adjustment(7, 0.1), 0.0);
}

#[test]
fn a_blocked_twenty_one_year_old_is_sent_out_for_minutes() {
    let fixture = RoomFixture::new()
        .senior(1, 29, 150)
        .senior(2, 28, 140)
        .senior(3, 33, 125)
        .academy(4, 21, 100);
    let plan = review(&fixture.assemble(), &KeeperRoomPlan::new());

    assert!(
        plan.advice_for(4)
            .any(|r| r.advice == KeeperAdvice::LoanHimOutForMinutes),
        "three keepers ahead of him means his season is somewhere else"
    );
}

#[test]
fn without_a_specialist_only_the_obvious_academy_keeper_gets_through() {
    // The same marginal boy, reviewed by a department with nobody watching
    // the academy on Tuesday mornings.
    let fixture = RoomFixture::new()
        .senior(1, 29, 150)
        .senior(2, 28, 138)
        .senior(3, 34, 120)
        .academy(4, 18, 108);
    let room = fixture.assemble();

    let with_coach = {
        let outcome = GoalkeepingDepartment::review(
            &room,
            &KeeperRoomPlan::new(),
            authority(0.8, 0.9),
            today(),
        )
        .unwrap();
        let mut plan = KeeperRoomPlan::new();
        plan.commit(outcome, today());
        plan
    };
    let without_coach = {
        let outcome = GoalkeepingDepartment::review(
            &room,
            &KeeperRoomPlan::new(),
            authority(0.25, 0.0),
            today(),
        )
        .unwrap();
        let mut plan = KeeperRoomPlan::new();
        plan.commit(outcome, today());
        plan
    };

    assert_eq!(with_coach.tier_of(4), Some(KeeperTier::Pathway));
    assert_eq!(
        without_coach.tier_of(4),
        Some(KeeperTier::Academy),
        "twelve points off the third choice is only a case if somebody is making it"
    );
}

#[test]
fn a_thin_room_is_told_to_find_a_deputy() {
    let fixture = RoomFixture::new().senior(1, 28, 155).senior(2, 26, 110);
    let plan = review(&fixture.assemble(), &KeeperRoomPlan::new());
    assert!(
        plan.wants(KeeperAdvice::SignACredibleDeputy),
        "a forty-five point gap is not a deputy"
    );
}

#[test]
fn the_keeper_age_curve_peaks_late_and_falls_slowly() {
    assert!(KeeperAgeCurve::of(19) < KeeperAgeCurve::of(24));
    assert!(KeeperAgeCurve::of(24) < KeeperAgeCurve::of(29));
    assert_eq!(KeeperAgeCurve::of(29), KeeperAgeCurve::of(33));
    assert!(KeeperAgeCurve::of(36) < KeeperAgeCurve::of(33));
    assert!(
        KeeperAgeCurve::of(36) > 0.8,
        "a thirty-six-year-old keeper is still a keeper"
    );
}
