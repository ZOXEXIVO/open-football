//! The goalkeeping department, end to end.
//!
//! Each test stands up a keeper room by hand and drives the public review,
//! because every claim being made here is about a group rather than about
//! one player: that the pecking order persists, that the succession opens
//! before it has to, and that a boy in the under-eighteens can be handed a
//! senior shirt he has not yet earned on ability alone.

use chrono::NaiveDate;

use super::advice::{KeeperAdvice, KeeperSuccession, KeeperTier};
use super::brief::{KeeperFixture, KeeperSelectionBrief};
use super::department::{GoalkeepingDepartment, KeeperCoachAuthority};
use super::plan::KeeperRoomPlan;
use super::room::{KeeperAgeCurve, KeeperRoom};
use crate::club::player::builder::PlayerBuilder;
use crate::club::staff::StaffStub;
use crate::club::staff::mind::organs::judgements::{CoachMatchObservation, CoachMemoryStore};
use crate::club::staff::perception::CoachProfile;
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
struct KeeperBuild;

impl KeeperBuild {
    fn build(id: u32, age: u8, level: u8) -> Player {
        let attrs = PlayerAttributes {
            current_ability: level,
            condition: 9500,
            ..Default::default()
        };
        // The observable ceiling reads attitude, and a default
        // `PersonAttributes` is a professional with zero professionalism and
        // zero ambition — which credits every fixture keeper with no future
        // at all. Give them an ordinary pro's application so the ceiling
        // means something.
        let person = PersonAttributes {
            professionalism: 15.0,
            ambition: 15.0,
            ..Default::default()
        };
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
            .push((1, TeamType::Main, KeeperBuild::build(id, age, level)));
        self
    }

    /// A senior keeper here on loan.
    fn borrowed(mut self, id: u32, age: u8, level: u8) -> Self {
        let mut player = KeeperBuild::build(id, age, level);
        player.contract_loan = Some(crate::PlayerClubContract::new_loan(
            50_000,
            today() + chrono::Duration::days(300),
            99,
            98,
            1,
        ));
        self.squads.push((1, TeamType::Main, player));
        self
    }

    fn academy(mut self, id: u32, age: u8, level: u8) -> Self {
        self.squads
            .push((2, TeamType::U18, KeeperBuild::build(id, age, level)));
        self
    }

    /// A senior keeper the club BOUGHT to take a named man's gloves.
    fn bought_heir(
        mut self,
        id: u32,
        age: u8,
        level: u8,
        incumbent: u32,
        handover_in_days: i64,
    ) -> Self {
        use crate::club::board::mandate::{MandateAuthor, MandatePurpose, SigningMandate};
        use crate::{PlayerFieldPositionGroup, PlayerPlan};

        let mut player = KeeperBuild::build(id, age, level);
        let signed = today();
        player.plan = Some(PlayerPlan::from_mandate(
            SigningMandate::new(
                MandatePurpose::Heir {
                    incumbent_id: incumbent,
                    handover: signed + chrono::Duration::days(handover_in_days),
                },
                PlayerFieldPositionGroup::Goalkeeper,
                age,
                signed,
                MandateAuthor::Manager,
            )
            .with_money(40_000_000.0, 4_000_000.0),
            signed,
        ));
        self.squads.push((1, TeamType::Main, player));
        self
    }

    fn academy_with_season(mut self, id: u32, age: u8, level: u8, apps: u16, rating: f32) -> Self {
        let mut player = KeeperBuild::build(id, age, level);
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
        cup_reach: CoachProfile::NEUTRAL_CUP_REACH,
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

/// The heir the club PAID for is the heir.
///
/// The department used to infer a successor from age gaps and ceilings —
/// so a twenty-five-year-old the board had just spent forty million on to
/// inherit the gloves was on no handover path at all, and the shirt never
/// moved. A named mandate outranks the inference, and the handover date it
/// carries is what the succession clock reads.
#[test]
fn a_bought_heir_outranks_the_inferred_one() {
    let fixture = RoomFixture::new()
        .senior(1, 33, 145)
        .senior(2, 20, 120)
        .bought_heir(3, 25, 130, 1, 730);
    let plan = review(&fixture.assemble(), &KeeperRoomPlan::new());

    assert_eq!(
        plan.heir(),
        Some(3),
        "the club already answered this question and paid for the answer"
    );

    // …and when the season the club named for the handover arrives, the
    // clock is that date rather than the incumbent's birthday.
    let due = RoomFixture::new()
        .senior(1, 33, 145)
        .senior(2, 20, 120)
        .bought_heir(3, 25, 130, 1, 0);
    assert_eq!(
        review(&due.assemble(), &KeeperRoomPlan::new()).succession(),
        KeeperSuccession::Pressing
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
        nominated: Some(4),
        deputy_cup_reach: None,
        ..settled_brief()
    };
    let all = [1, 2, 3, 4];
    assert_eq!(
        brief.keeper_for(&league(0.20), &all),
        Some(4),
        "in a fixture the club can afford, the nomination is a decision"
    );
    assert_eq!(
        brief.keeper_for(&league(0.90), &all),
        Some(1),
        "and it is worth nothing at all when the result matters"
    );
    // A manager who half-listens does not act on it.
    let unheard = KeeperSelectionBrief {
        authority: 0.3,
        ..brief
    };
    assert_eq!(unheard.keeper_for(&league(0.20), &all), Some(1));
}

#[test]
fn a_silent_brief_names_nobody() {
    let brief = KeeperSelectionBrief::from_plan(&KeeperRoomPlan::new(), today());
    assert!(brief.is_silent());
    assert_eq!(brief.keeper_for(&league(0.5), &[1, 7]), None);
    assert_eq!(brief.next_in_order(&[1, 7]), None);
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

fn match_seen(player_id: u32, errors: u16, week: i64) -> CoachMatchObservation {
    CoachMatchObservation {
        player_id,
        effective_rating: 5.8,
        minutes_played: 90,
        is_starter: true,
        match_importance: 0.6,
        is_cup: false,
        is_derby: false,
        is_continental: false,
        goals: 0,
        assists: 0,
        errors_leading_to_goal: errors,
        errors_leading_to_shot: errors,
        keeper_claims: 1,
        standard_of_football: 0.66,
        yellow_cards: 0,
        red_cards: 0,
        team_won: false,
        was_substituted_early: false,
        role_fit: 1.0,
        professionalism_signal: 0.6,
        date: today() + chrono::Duration::days(week * 7),
    }
}

/// Three matches running with an error that cost a goal, seen by a
/// sharp-eyed goalkeeping coach.
fn shaken_room(room: &mut KeeperRoom, shaken_id: u32) {
    let mut lead = StaffStub::build();
    lead.id = 11;
    lead.staff_attributes.knowledge.judging_player_ability = 17;
    let profile = CoachProfile::from_staff(&lead);
    let mut memory = CoachMemoryStore::new();
    for week in 0..3 {
        memory.observe(&match_seen(shaken_id, 1, week), &profile);
    }
    room.perceive(&lead, 0.66, |_, player_id| (0.66, memory.get(player_id)), today());
}

#[test]
fn a_fixture_a_step_above_him_is_not_a_debut_for_an_unassured_keeper() {
    let brief = KeeperSelectionBrief {
        nominated: Some(4),
        nominee_assurance: Some(0.55),
        deputy_cup_reach: None,
        ..settled_brief()
    };
    let all = [1, 2, 3, 4];
    // A league fixture at the first team's full standard.
    assert_eq!(
        brief
            .with_fixture_standard(0.69)
            .keeper_for(&league(0.33), &all),
        Some(1)
    );
    // An early cup tie against a lower division, close to where he plays.
    assert_eq!(
        brief
            .with_fixture_standard(0.57)
            .keeper_for(&league(0.30), &all),
        Some(4)
    );
}

#[test]
fn a_shaken_number_one_is_taken_out_of_the_firing_line() {
    let mut room = RoomFixture::new()
        .senior(1, 29, 140)
        .senior(2, 27, 128)
        .assemble();
    shaken_room(&mut room, 1);
    let plan = review(&room, &KeeperRoomPlan::new());
    assert_eq!(plan.rested(), Some(1));

    let brief = KeeperSelectionBrief::from_plan(&plan, today());
    assert_eq!(
        brief.keeper_for(&league(0.9), &[1, 2]),
        Some(2),
        "a big night is the deputy's"
    );
    assert_eq!(
        brief.keeper_for(&league(0.4), &[1, 2]),
        Some(1),
        "an ordinary one is still his"
    );
}

#[test]
fn no_protection_without_cover() {
    let mut room = RoomFixture::new().senior(1, 29, 140).assemble();
    shaken_room(&mut room, 1);
    let plan = review(&room, &KeeperRoomPlan::new());
    assert_eq!(plan.rested(), None);
}

/// Four clean matches at `standard`, seen by a sharp-eyed goalkeeping coach
/// at a club whose first team plays at 0.70.
fn read_at(room: &mut KeeperRoom, keeper_id: u32, standard: f32) {
    let mut lead = StaffStub::build();
    lead.id = 11;
    lead.staff_attributes.knowledge.judging_player_ability = 17;
    let profile = CoachProfile::from_staff(&lead);
    let mut memory = CoachMemoryStore::new();
    for week in 0..4 {
        let mut seen = match_seen(keeper_id, 0, week);
        seen.standard_of_football = standard;
        memory.observe(&seen, &profile);
    }
    room.perceive(&lead, 0.70, |_, player_id| (standard, memory.get(player_id)), today());
}

#[test]
fn a_blocked_twenty_year_old_is_loaned_to_the_level_between() {
    let fixture = RoomFixture::new()
        .senior(1, 29, 150)
        .senior(2, 27, 138)
        .senior(3, 33, 125)
        .academy(4, 20, 100);
    let loan_aim = |plan: &KeeperRoomPlan| {
        plan.advice_for(4)
            .find(|r| r.advice == KeeperAdvice::LoanHimOutForMinutes)
            .expect("three keepers ahead of him")
            .target_standard
    };

    let mut room = fixture.assemble();
    read_at(&mut room, 4, 0.58);
    let target = loan_aim(&review(&room, &KeeperRoomPlan::new())).expect("he has been read");
    assert!(target > 0.58 && target < 0.70, "{target}");

    let unread = review(&fixture.assemble(), &KeeperRoomPlan::new());
    assert_eq!(loan_aim(&unread), None, "an unread boy goes out on the club's ordinary terms");
}

// ── The room counts every keeper who can play for the club ──

#[test]
fn a_borrowed_keeper_clearly_better_than_the_clubs_own_is_declared_number_one() {
    let fixture = RoomFixture::new()
        .senior(1, 30, 130)
        .borrowed(2, 25, 150)
        .academy(3, 17, 80);
    let plan = review(&fixture.assemble(), &KeeperRoomPlan::new());
    assert_eq!(plan.number_one(), Some(2), "the borrowed keeper takes his place on his level");
    assert_eq!(plan.deputy(), Some(1), "and the best keeper the club owns is his deputy");
}

#[test]
fn the_succession_looks_past_a_borrowed_keeper() {
    // An ageing owned keeper, a young borrowed one good enough to follow
    // him, and nobody else: the club has no heir, because the borrowed
    // man goes back.
    let fixture = RoomFixture::new().senior(1, 35, 140).borrowed(2, 22, 132);
    let plan = review(&fixture.assemble(), &KeeperRoomPlan::new());
    assert_eq!(plan.heir(), None, "a keeper who goes back in June is nobody's heir");
    assert!(
        plan.succession() >= KeeperSuccession::Pressing,
        "and the clock runs on the keeper the club owns"
    );
    assert!(
        plan.advice_for(1)
            .any(|r| r.advice == KeeperAdvice::OpenTheSuccession),
        "the succession is opened for the owned keeper"
    );

    // The same young keeper, owned, is the heir.
    let owned = RoomFixture::new().senior(1, 35, 140).senior(2, 22, 132);
    assert_eq!(
        review(&owned.assemble(), &KeeperRoomPlan::new()).heir(),
        Some(2)
    );
}

// ── The deputy's competition is declared once ──

fn review_under(room: &KeeperRoom, previous: &KeeperRoomPlan, cup_reach: f32, on: NaiveDate) -> KeeperRoomPlan {
    let outcome = GoalkeepingDepartment::review(
        room,
        previous,
        KeeperCoachAuthority {
            weight: 0.8,
            specialist_focus: 0.8,
            cup_reach,
        },
        on,
    )
    .expect("a room with keepers in it produces a plan");
    let mut plan = previous.clone();
    plan.commit(outcome, on);
    plan
}

fn cup_tie(round: u8, total_rounds: u8) -> KeeperFixture {
    KeeperFixture {
        stakes: 0.5,
        domestic_cup_progress: Some(f32::from(round - 1) / f32::from(total_rounds - 1)),
    }
}

#[test]
fn a_cup_declared_to_the_semi_final_is_the_deputys_to_the_semi_and_the_number_ones_final() {
    let room = RoomFixture::new().senior(1, 29, 150).senior(2, 28, 138).assemble();
    // Five rounds: the semi-final is three quarters of the way through.
    let plan = review_under(&room, &KeeperRoomPlan::new(), 0.75, today());
    let declared = plan.deputy_competition().expect("a ready deputy is given the cup");
    assert_eq!((declared.number_one, declared.deputy), (1, 2));

    let brief = KeeperSelectionBrief::from_plan(&plan, today());
    let both = [1, 2];
    for round in 1..=4 {
        assert_eq!(brief.keeper_for(&cup_tie(round, 5), &both), Some(2), "round {round}");
    }
    assert_eq!(brief.keeper_for(&cup_tie(5, 5), &both), Some(1), "the final");
    assert!(
        plan.advice_for(2)
            .any(|r| r.advice == KeeperAdvice::HandHimTheCup)
    );
}

#[test]
fn a_manager_who_lets_him_finish_starts_the_deputy_in_the_final() {
    let room = RoomFixture::new().senior(1, 29, 150).senior(2, 28, 138).assemble();
    let plan = review_under(&room, &KeeperRoomPlan::new(), 1.0, today());
    let brief = KeeperSelectionBrief::from_plan(&plan, today());
    assert_eq!(brief.keeper_for(&cup_tie(5, 5), &[1, 2]), Some(2));
}

#[test]
fn a_deputy_who_is_not_ready_is_given_no_competition() {
    // A 17-year-old far below first-team standard is the only man behind
    // the number one.
    let room = RoomFixture::new().senior(1, 29, 150).academy(2, 17, 90).assemble();
    let plan = review_under(&room, &KeeperRoomPlan::new(), 1.0, today());
    assert_eq!(plan.deputy(), Some(2));
    assert!(plan.deputy_competition().is_none());
    let brief = KeeperSelectionBrief::from_plan(&plan, today());
    assert_eq!(
        brief.keeper_for(&cup_tie(1, 5), &[1, 2]),
        Some(1),
        "the number one plays the cup"
    );
    assert!(
        !plan
            .advice_for(2)
            .any(|r| r.advice == KeeperAdvice::HandHimTheCup)
    );
}

#[test]
fn the_declaration_stands_through_the_season_and_is_made_again_when_the_order_changes() {
    let room = RoomFixture::new().senior(1, 29, 150).senior(2, 28, 138).assemble();
    let first = review_under(&room, &KeeperRoomPlan::new(), 0.75, today());
    let declared_on = first.deputy_competition().unwrap().declared_on;

    // A new manager's habits do not reopen a declaration made for this
    // order this season.
    let later = today() + chrono::Duration::days(120);
    let second = review_under(&room, &first, 1.0, later);
    let standing = second.deputy_competition().unwrap();
    assert_eq!((standing.reach, standing.declared_on), (0.75, declared_on));

    // A new deputy is a new declaration.
    let changed = RoomFixture::new().senior(1, 29, 150).senior(3, 27, 140).assemble();
    let third = review_under(&changed, &second, 1.0, later);
    let renewed = third.deputy_competition().unwrap();
    assert_eq!((renewed.deputy, renewed.reach, renewed.declared_on), (3, 1.0, later));

    // And so is a new season.
    let next_season = today() + chrono::Duration::days(370);
    let fourth = review_under(&room, &first, 1.0, next_season);
    assert_eq!(fourth.deputy_competition().unwrap().declared_on, next_season);
}

// ── The goalkeeper is the declared one ──

fn settled_brief() -> KeeperSelectionBrief {
    KeeperSelectionBrief {
        number_one: Some(1),
        deputy: Some(2),
        third: Some(3),
        nominated: None,
        authority: 0.85,
        nominee_assurance: None,
        fixture_standard: None,
        rested: None,
        deputy_cup_reach: Some(0.5),
    }
}

fn league(stakes: f32) -> KeeperFixture {
    KeeperFixture {
        stakes,
        domestic_cup_progress: None,
    }
}

#[test]
fn the_number_one_plays_every_fixture_that_is_not_the_deputys() {
    let brief = settled_brief();
    let all = [1, 2, 3];
    for stakes in [0.1, 0.45, 0.66, 0.9, 1.0] {
        assert_eq!(brief.keeper_for(&league(stakes), &all), Some(1), "stakes {stakes}");
    }
    // The cup is the deputy's up to the quarter-final of a five-round cup,
    // whoever the opponent is.
    assert_eq!(brief.keeper_for(&cup_tie(3, 5), &all), Some(2));
    // Past the declared round the number one is back.
    assert_eq!(brief.keeper_for(&cup_tie(4, 5), &all), Some(1));
}

#[test]
fn the_next_keeper_in_the_order_plays_when_the_declared_one_cannot() {
    let brief = settled_brief();
    assert_eq!(brief.keeper_for(&league(0.7), &[2, 3]), Some(2), "the number one is injured");
    assert_eq!(brief.keeper_for(&league(0.7), &[3]), Some(3));
    assert_eq!(brief.keeper_for(&cup_tie(1, 5), &[1, 3]), Some(1), "the deputy is injured");
    assert_eq!(brief.keeper_for(&league(0.7), &[9]), None, "nobody in the order can play");
    assert_eq!(brief.next_in_order(&[2, 3]), Some(2));
}
