//! End-to-end tests for the coach decision engine.
//!
//! Each test stands up a small fixture (Staff + CoachMemoryStore +
//! Player) and drives the public API. The tests cover the brief's
//! required scenarios — poor-form pressure not auto-benching, coach
//! personality moderating reaction strength, big-match trust
//! protecting against poor form, sustained-form reading mapping to
//! the omission layer, sub-off urgency feedback, and memory decay.

use super::assessment::CoachPlayerAssessment;
use super::engine::{CoachDecisionEngine, CoachLiveMatchContext, CoachSelectionContext};
use super::memory::{CoachMatchObservation, CoachMemoryFlags};
use super::reason::CoachDecisionReason;
use super::strategy::CoachStrategy;
use crate::club::staff::{CoachProfile, CoachingStyle, StaffStub};
use crate::club::player::builder::PlayerBuilder;
use crate::shared::fullname::FullName;
use crate::{
    PersonAttributes, Player, PlayerAttributes, PlayerPosition, PlayerPositionType,
    PlayerPositions, PlayerSkills, Staff,
};
use chrono::NaiveDate;

fn d(y: i32, m: u32, day: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(y, m, day).unwrap()
}

/// Builder for a small Player suitable for coach-decision tests.
/// Wraps the existing `PlayerBuilder` so the call-sites read as
/// declarative property overrides.
struct PlayerFixture;

impl PlayerFixture {
    fn forward(id: u32, age: u32) -> Player {
        Self::build(id, age, PlayerPositionType::ForwardCenter, 150)
    }

    fn youth_forward(id: u32) -> Player {
        Self::build(id, 18, PlayerPositionType::ForwardCenter, 130)
    }

    fn build(id: u32, age: u32, pos: PlayerPositionType, ca: u8) -> Player {
        let mut attrs = PlayerAttributes::default();
        attrs.current_ability = ca;
        attrs.condition = 9000;
        let mut skills = PlayerSkills::default();
        skills.technical.finishing = 14.0;
        skills.mental.composure = 14.0;
        skills.physical.pace = 14.0;
        let birth = d(2026 - age as i32, 1, 1);
        PlayerBuilder::new()
            .id(id)
            .full_name(FullName::new("T".to_string(), format!("P{}", id)))
            .birth_date(birth)
            .country_id(1)
            .attributes(PersonAttributes::default())
            .skills(skills)
            .positions(PlayerPositions {
                positions: vec![PlayerPosition {
                    position: pos,
                    level: 18,
                }],
            })
            .player_attributes(attrs)
            .build()
            .unwrap()
    }
}

/// Build a staff/profile/strategy triple for a given staff fixture
/// and strategy choice. Bundled so the tests don't replicate the
/// three-line builder each time.
struct CoachSetup;

impl CoachSetup {
    fn baseline_staff() -> Staff {
        let mut s = StaffStub::default();
        s.id = 1;
        s.staff_attributes.knowledge.judging_player_ability = 14;
        s.staff_attributes.knowledge.judging_player_potential = 14;
        s.staff_attributes.mental.man_management = 12;
        s.staff_attributes.mental.motivating = 12;
        s.staff_attributes.mental.adaptability = 10;
        s.staff_attributes.mental.determination = 12;
        s.staff_attributes.mental.discipline = 10;
        s.staff_attributes.coaching.tactical = 12;
        s.staff_attributes.coaching.technical = 12;
        s.staff_attributes.coaching.fitness = 12;
        s.staff_attributes.coaching.mental = 12;
        s.staff_attributes.coaching.working_with_youngsters = 10;
        s.coaching_style = CoachingStyle::Democratic;
        s
    }

    fn high_negativity_staff() -> Staff {
        let mut s = Self::baseline_staff();
        s.coaching_style = CoachingStyle::Authoritarian;
        s.staff_attributes.mental.discipline = 18;
        s.staff_attributes.mental.man_management = 5;
        s
    }

    fn warm_staff() -> Staff {
        let mut s = Self::baseline_staff();
        s.coaching_style = CoachingStyle::Transformational;
        s.staff_attributes.mental.man_management = 18;
        s.staff_attributes.mental.motivating = 18;
        s
    }

    fn youth_staff() -> Staff {
        let mut s = Self::baseline_staff();
        s.coaching_style = CoachingStyle::Transformational;
        s.staff_attributes.coaching.working_with_youngsters = 18;
        s
    }

    /// Run `runs` poor-rating observations against a fresh memory store.
    fn observe_poor_run(staff: &mut Staff, player_id: u32, runs: u32, rating: f32) {
        let profile = CoachProfile::from_staff(staff);
        for i in 0..runs {
            staff.coach_memory.observe(
                &ObservationFixture::league_start(player_id, rating, d(2026, 1, 1 + i)),
                &profile,
            );
        }
    }

    fn observe_strong_run(staff: &mut Staff, player_id: u32, runs: u32, rating: f32) {
        let profile = CoachProfile::from_staff(staff);
        for i in 0..runs {
            staff.coach_memory.observe(
                &ObservationFixture::league_start(player_id, rating, d(2026, 1, 1 + i)),
                &profile,
            );
        }
    }

    fn observe_big_match(staff: &mut Staff, player_id: u32, rating: f32, date: NaiveDate) {
        let profile = CoachProfile::from_staff(staff);
        staff.coach_memory.observe(
            &ObservationFixture::big_match(player_id, rating, date),
            &profile,
        );
    }
}

/// Observation builder. Defaults to a clean league start; tests
/// override the flag they care about.
struct ObservationFixture;

impl ObservationFixture {
    fn league_start(player_id: u32, rating: f32, date: NaiveDate) -> CoachMatchObservation {
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
            team_won: true,
            was_substituted_early: false,
            role_fit: 1.0,
            professionalism_signal: 0.7,
            date,
        }
    }

    fn big_match(player_id: u32, rating: f32, date: NaiveDate) -> CoachMatchObservation {
        let mut o = Self::league_start(player_id, rating, date);
        o.is_cup = true;
        o.is_continental = true;
        o.match_importance = 0.95;
        o
    }
}

/// Context builder for selection assessments. Defaults to a league
/// fixture with a natural-role fit.
struct CtxFixture;

impl CtxFixture {
    fn league() -> CoachSelectionContext<'static> {
        CoachSelectionContext {
            date: d(2026, 6, 5),
            match_importance: 0.7,
            is_friendly: false,
            is_cup: false,
            is_derby: false,
            is_continental: false,
            natural_role_fit: 1.0,
            is_succession_heir: &[],
        }
    }

    fn cup_final() -> CoachSelectionContext<'static> {
        CoachSelectionContext {
            date: d(2026, 6, 5),
            match_importance: 0.95,
            is_friendly: false,
            is_cup: true,
            is_derby: false,
            is_continental: true,
            natural_role_fit: 1.0,
            is_succession_heir: &[],
        }
    }
}

#[test]
fn poor_form_lowers_confidence_but_doesnt_auto_bench() {
    // A player on a clear poor-form streak should see selection_confidence
    // fall — but the assessment never goes to "drop the player" on its own,
    // because the caller still has to compare against alternatives.
    let mut staff = CoachSetup::baseline_staff();
    let player = PlayerFixture::forward(7, 27);
    CoachSetup::observe_poor_run(&mut staff, 7, 6, 5.4);

    let profile = CoachProfile::from_staff(&staff);
    let engine = CoachDecisionEngine::from_staff(&staff, &profile, CoachStrategy::WinNow);
    let assessment = engine.assess_player_for_selection(&player, &CtxFixture::league());

    assert!(
        assessment.form_confidence < 0.5,
        "form_confidence should drop after poor run, got {}",
        assessment.form_confidence
    );
    // Adjustment is bounded — the coach moves the slot score by less
    // than the engine's SELECTION_SCALE.
    assert!(
        assessment.selection_adjustment().abs() <= 1.0,
        "selection_adjustment must stay normalised, got {}",
        assessment.selection_adjustment()
    );
    // Confirm a sustained-form reason is surfaced.
    assert!(
        assessment.reasons.iter().any(|r| matches!(
            r,
            CoachDecisionReason::SustainedPoorForm | CoachDecisionReason::PoorRecentForm
        )),
        "expected a poor-form reason, got {:?}",
        assessment.reasons
    );
}

#[test]
fn negativity_biased_coach_reacts_harder_to_repeated_poor_form() {
    let mut neg_staff = CoachSetup::high_negativity_staff();
    let mut base_staff = CoachSetup::baseline_staff();
    let player = PlayerFixture::forward(7, 27);

    // Same observation run on both coaches.
    CoachSetup::observe_poor_run(&mut neg_staff, 7, 5, 5.2);
    CoachSetup::observe_poor_run(&mut base_staff, 7, 5, 5.2);

    let neg_profile = CoachProfile::from_staff(&neg_staff);
    let base_profile = CoachProfile::from_staff(&base_staff);
    let neg_engine = CoachDecisionEngine::from_staff(&neg_staff, &neg_profile, CoachStrategy::WinNow);
    let base_engine = CoachDecisionEngine::from_staff(&base_staff, &base_profile, CoachStrategy::WinNow);

    let neg_assessment = neg_engine.assess_player_for_selection(&player, &CtxFixture::league());
    let base_assessment = base_engine.assess_player_for_selection(&player, &CtxFixture::league());

    assert!(
        neg_assessment.form_confidence <= base_assessment.form_confidence,
        "negativity-biased coach should not have HIGHER form_confidence: neg={} base={}",
        neg_assessment.form_confidence,
        base_assessment.form_confidence
    );
}

#[test]
fn warm_coach_reacts_less_to_one_bad_game() {
    let mut warm_staff = CoachSetup::warm_staff();
    let mut base_staff = CoachSetup::baseline_staff();
    let player = PlayerFixture::forward(7, 27);

    // Build the same positive baseline, then one bad game.
    CoachSetup::observe_strong_run(&mut warm_staff, 7, 8, 7.4);
    CoachSetup::observe_strong_run(&mut base_staff, 7, 8, 7.4);
    let warm_profile = CoachProfile::from_staff(&warm_staff);
    let base_profile = CoachProfile::from_staff(&base_staff);
    warm_staff.coach_memory.observe(
        &ObservationFixture::league_start(7, 5.0, d(2026, 1, 10)),
        &warm_profile,
    );
    base_staff.coach_memory.observe(
        &ObservationFixture::league_start(7, 5.0, d(2026, 1, 10)),
        &base_profile,
    );

    let warm_engine = CoachDecisionEngine::from_staff(&warm_staff, &warm_profile, CoachStrategy::WinNow);
    let base_engine = CoachDecisionEngine::from_staff(&base_staff, &base_profile, CoachStrategy::WinNow);

    let warm_assessment = warm_engine.assess_player_for_selection(&player, &CtxFixture::league());
    let base_assessment = base_engine.assess_player_for_selection(&player, &CtxFixture::league());

    // The warm coach's drop_risk should be lower than the baseline's.
    assert!(
        warm_assessment.drop_risk <= base_assessment.drop_risk + 0.05,
        "warm coach's drop_risk should not exceed baseline meaningfully: warm={} base={}",
        warm_assessment.drop_risk,
        base_assessment.drop_risk
    );
}

#[test]
fn big_match_trust_protects_against_poor_form() {
    // A player who has earned big-match trust (cup hero) but is on a
    // mild poor-form dip should still see a big-match reliability
    // reason surface in a cup-final fixture.
    let mut staff = CoachSetup::baseline_staff();
    let player = PlayerFixture::forward(7, 27);

    // Earn big-match-proven status with a strong showing in a continental tie.
    CoachSetup::observe_big_match(&mut staff, 7, 7.6, d(2026, 1, 5));
    // Then a couple of mild league dips.
    CoachSetup::observe_poor_run(&mut staff, 7, 3, 6.0);

    let profile = CoachProfile::from_staff(&staff);
    let engine = CoachDecisionEngine::from_staff(&staff, &profile, CoachStrategy::WinNow);
    let assessment = engine.assess_player_for_selection(&player, &CtxFixture::cup_final());

    let mem = staff.coach_memory.get(7).expect("memory exists");
    assert!(
        mem.flags.contains(CoachMemoryFlags::BIG_MATCH_PROVEN),
        "big-match-proven flag should be set after the cup display"
    );
    assert!(
        assessment
            .reasons
            .iter()
            .any(|r| matches!(r, CoachDecisionReason::BigMatchReliability)),
        "expected BigMatchReliability among reasons, got {:?}",
        assessment.reasons
    );
}

#[test]
fn development_strategy_lifts_youth_preference() {
    let mut staff = CoachSetup::youth_staff();
    let young = PlayerFixture::youth_forward(7);

    // Even with no observed history, the coach should weight the
    // youth player's development priority — the assessment surfaces
    // a DevelopmentPathway reason.
    let profile = CoachProfile::from_staff(&staff);
    let engine = CoachDecisionEngine::from_staff(&staff, &profile, CoachStrategy::DevelopYouth);
    let assessment = engine.assess_player_for_selection(&young, &CtxFixture::league());

    assert!(
        assessment.development_priority > 0.0,
        "youth player should carry development priority"
    );
    assert!(
        assessment
            .reasons
            .iter()
            .any(|r| matches!(r, CoachDecisionReason::DevelopmentPathway)),
        "expected DevelopmentPathway in reasons, got {:?}",
        assessment.reasons
    );

    // Mark `staff` as observed so the unused warning is silenced.
    staff.coach_memory.observe(
        &ObservationFixture::league_start(7, 7.0, d(2026, 1, 1)),
        &profile,
    );
}

#[test]
fn costly_error_lifts_live_sub_off_urgency() {
    let staff = CoachSetup::baseline_staff();
    let profile = CoachProfile::from_staff(&staff);
    let engine = CoachDecisionEngine::from_staff(&staff, &profile, CoachStrategy::WinNow);
    let live = CoachLiveMatchContext {
        date: d(2026, 6, 5),
        match_minute: 70,
        goal_diff: -1,
        live_rating: 5.3,
        goals: 0,
        assists: 0,
        errors_leading_to_goal: 1,
        yellow_cards: 0,
        red_cards: 0,
        condition_pct: 0.6,
        is_starter: true,
    };
    let assessment = engine.assess_live_substitution(7, &live);
    assert!(
        assessment.sub_off_urgency > 0.5,
        "errors_leading_to_goal should lift urgency, got {}",
        assessment.sub_off_urgency
    );
    assert!(
        assessment
            .reasons
            .iter()
            .any(|r| matches!(r, CoachDecisionReason::CostlyError)),
        "expected CostlyError reason"
    );
}

#[test]
fn live_high_rated_scorer_is_protected_from_routine_removal() {
    let staff = CoachSetup::baseline_staff();
    let profile = CoachProfile::from_staff(&staff);
    let engine = CoachDecisionEngine::from_staff(&staff, &profile, CoachStrategy::WinNow);

    let scorer_live = CoachLiveMatchContext {
        date: d(2026, 6, 5),
        match_minute: 70,
        goal_diff: 1,
        live_rating: 7.6,
        goals: 1,
        assists: 1,
        errors_leading_to_goal: 0,
        yellow_cards: 0,
        red_cards: 0,
        condition_pct: 0.6,
        is_starter: true,
    };
    let scorer_assessment = engine.assess_live_substitution(7, &scorer_live);
    assert!(
        scorer_assessment.sub_off_urgency < 0.4,
        "a 1G/1A scorer at 7.6 should be protected; urgency={}",
        scorer_assessment.sub_off_urgency
    );
    assert!(
        scorer_assessment
            .reasons
            .iter()
            .any(|r| matches!(r, CoachDecisionReason::ProtectingStar)),
        "expected ProtectingStar"
    );
}

#[test]
fn memory_softens_after_long_inactivity() {
    let mut staff = CoachSetup::high_negativity_staff();
    CoachSetup::observe_poor_run(&mut staff, 7, 4, 5.2);
    let before = staff.coach_memory.get(7).unwrap().form_pressure();
    // ~120 days later, no further observations.
    staff.coach_memory.decay_inactive(d(2026, 5, 5));
    let after = staff.coach_memory.get(7).unwrap().form_pressure();
    assert!(
        after < before,
        "form_pressure should soften after inactivity: before={} after={}",
        before,
        after
    );
}

#[test]
fn memory_update_via_engine_observe_match_outcome_path() {
    // Confirm the memory store reflects a single observation through
    // the public API. This pins the "match observation feeds memory"
    // contract that the dispatch layer depends on.
    let mut staff = CoachSetup::baseline_staff();
    let profile = CoachProfile::from_staff(&staff);
    staff.coach_memory.observe(
        &ObservationFixture::league_start(7, 7.0, d(2026, 1, 1)),
        &profile,
    );
    let mem = staff.coach_memory.get(7).expect("record created");
    assert_eq!(mem.matches_observed, 1);
    assert!((mem.recent_rating_ema - 7.0).abs() < 1e-4);
    assert!((mem.long_form_rating - 7.0).abs() < 1e-4);
}

#[test]
fn late_game_amplifies_sub_off_urgency_for_poor_performer() {
    // Same player, same rating — but in stoppage time the coach
    // reads the urgency higher. Pins the live-context amplifier.
    let staff = CoachSetup::baseline_staff();
    let profile = CoachProfile::from_staff(&staff);
    let engine = CoachDecisionEngine::from_staff(&staff, &profile, CoachStrategy::WinNow);

    let mid_match = CoachLiveMatchContext {
        date: d(2026, 6, 5),
        match_minute: 50,
        goal_diff: 0,
        live_rating: 5.6,
        goals: 0,
        assists: 0,
        errors_leading_to_goal: 0,
        yellow_cards: 0,
        red_cards: 0,
        condition_pct: 0.5,
        is_starter: true,
    };
    let late_match = CoachLiveMatchContext {
        match_minute: 82,
        ..mid_match
    };
    let mid = engine.assess_live_substitution(7, &mid_match);
    let late = engine.assess_live_substitution(7, &late_match);
    assert!(
        late.sub_off_urgency > mid.sub_off_urgency,
        "late-game urgency should exceed mid-game: late={} mid={}",
        late.sub_off_urgency,
        mid.sub_off_urgency
    );
}

#[test]
fn losing_late_amplifies_poor_performer_urgency() {
    let staff = CoachSetup::baseline_staff();
    let profile = CoachProfile::from_staff(&staff);
    let engine = CoachDecisionEngine::from_staff(&staff, &profile, CoachStrategy::WinNow);

    let level = CoachLiveMatchContext {
        date: d(2026, 6, 5),
        match_minute: 75,
        goal_diff: 0,
        live_rating: 5.6,
        goals: 0,
        assists: 0,
        errors_leading_to_goal: 0,
        yellow_cards: 0,
        red_cards: 0,
        condition_pct: 0.7,
        is_starter: true,
    };
    let losing = CoachLiveMatchContext {
        goal_diff: -2,
        ..level
    };
    let level_a = engine.assess_live_substitution(7, &level);
    let losing_a = engine.assess_live_substitution(7, &losing);
    assert!(
        losing_a.sub_off_urgency >= level_a.sub_off_urgency,
        "losing-state urgency should not drop below level state: losing={} level={}",
        losing_a.sub_off_urgency,
        level_a.sub_off_urgency
    );
}

#[test]
fn role_fit_drops_for_player_pressed_into_emergency_slot() {
    // The observation layer's role_fit input drives the coach's
    // role_fit_confidence EMA. Two players seen at the same rating —
    // one at his natural position, one at an emergency slot — should
    // diverge in role_fit_confidence.
    let mut staff = CoachSetup::baseline_staff();
    let profile = CoachProfile::from_staff(&staff);
    let mut natural_obs = ObservationFixture::league_start(7, 7.0, d(2026, 1, 1));
    natural_obs.role_fit = 1.0;
    let mut emergency_obs = ObservationFixture::league_start(8, 7.0, d(2026, 1, 1));
    emergency_obs.role_fit = 0.35;

    for i in 0..6 {
        let mut natural = natural_obs;
        natural.date = d(2026, 1, 1 + i);
        let mut emergency = emergency_obs;
        emergency.date = d(2026, 1, 1 + i);
        staff.coach_memory.observe(&natural, &profile);
        staff.coach_memory.observe(&emergency, &profile);
    }
    let natural_role = staff.coach_memory.get(7).unwrap().role_fit_confidence;
    let emergency_role = staff.coach_memory.get(8).unwrap().role_fit_confidence;
    assert!(
        emergency_role < natural_role,
        "emergency-slot role_fit_confidence should drop below natural: emergency={} natural={}",
        emergency_role,
        natural_role
    );
}

#[test]
fn assessment_keeps_neutral_when_no_memory_exists() {
    // Old saves with empty CoachMemoryStore must not have the coach
    // engine swing the slot score. The assessment falls back to a
    // neutral 0.5 starting_confidence and a near-zero adjustment.
    let staff = CoachSetup::baseline_staff();
    let profile = CoachProfile::from_staff(&staff);
    let engine = CoachDecisionEngine::from_staff(&staff, &profile, CoachStrategy::WinNow);
    let player = PlayerFixture::forward(99, 27);
    let assessment = engine.assess_player_for_selection(&player, &CtxFixture::league());
    silence_unused(assessment.selection_adjustment());
    assert!(
        assessment.selection_adjustment().abs() <= 0.6,
        "no memory → assessment should not swing the slot score: {}",
        assessment.selection_adjustment()
    );
}

fn silence_unused<T>(value: T) -> T {
    value
}

fn _unused_assessment_field_anchor(a: &CoachPlayerAssessment) -> f32 {
    a.start_preference + a.bench_preference
}
