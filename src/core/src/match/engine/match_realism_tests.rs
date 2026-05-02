//! Cross-module regression tests for match-realism additions
//! (environment, referee, set pieces, psychology, chemistry, game
//! management). The single-module unit tests live next to each module —
//! these tests deliberately exercise *interactions* between modules so
//! that future refactors don't drift the combined behavior.

#![cfg(test)]

use crate::r#match::engine::chemistry::{
    ChemistryInputs, ChemistryMap, Lane, Role, chemistry_modifiers, initial_chemistry,
};
use crate::r#match::engine::environment::{MatchEnvironment, Pitch, Weather};
use crate::r#match::engine::game_management::{
    TimeWastingRestart, home_advantage_deltas, time_wasting_delay_ms, time_wasting_yellow_prob,
};
use crate::r#match::engine::psychology::{
    NegativeEvent, PositiveEvent, PsychState, PsychologyState, leadership_damped_momentum,
    pressure_load, skill_modifiers, team_leadership_score,
};
use crate::r#match::engine::referee::{ContactLocation, FoulCallContext, RefereeProfile};
use crate::r#match::engine::set_pieces::{
    CornerRoutine, FreeKickBand, SetPieceHistory, penalty_conversion_prob, pick_corner_routine,
    score_corner_routines, score_free_kick_choices, score_keeper_save, score_penalty_taker,
    wall_block_prob, wall_size_for,
};

#[test]
fn rainy_match_drops_first_touch_and_gk_handling() {
    let env = MatchEnvironment {
        weather: Weather::HeavyRain,
        ..Default::default()
    };
    let m = env.modifiers();
    assert!(m.first_touch <= -0.10);
    assert!(m.goalkeeper_handling <= -0.05);
}

#[test]
fn wind_reduces_far_pass_and_cross_accuracy_only() {
    let env = MatchEnvironment {
        weather: Weather::Wind,
        ..Default::default()
    };
    let m = env.modifiers();
    assert!(m.cross_accuracy < 0.0);
    assert!(m.long_pass_accuracy < 0.0);
    // Short passes unaffected by wind.
    assert_eq!(m.pass_accuracy, 0.0);
}

#[test]
fn high_strictness_increases_foul_call_versus_low() {
    let env = MatchEnvironment::default();
    let strict = RefereeProfile {
        strictness: 0.9,
        leniency: 0.1,
        ..Default::default()
    };
    let lenient = RefereeProfile {
        strictness: 0.15,
        leniency: 0.85,
        ..Default::default()
    };
    let ctx = FoulCallContext {
        contact_severity: 0.5,
        match_temperature: 0.2,
        fouled_team_is_home: false,
        location: ContactLocation::Normal,
    };
    assert!(strict.foul_call_prob(&env, ctx) > lenient.foul_call_prob(&env, ctx));
}

#[test]
fn advantage_window_grows_with_patience_and_violent_blocks_advantage() {
    let patient = RefereeProfile {
        advantage_patience: 1.0,
        ..Default::default()
    };
    let impatient = RefereeProfile {
        advantage_patience: 0.0,
        ..Default::default()
    };
    assert!(patient.advantage_window_ticks() > impatient.advantage_window_ticks());

    // Violent fouls always whistled regardless of attack value.
    let r = RefereeProfile::default();
    assert!(!r.should_play_advantage(0.95, true, 0.95));
    // Reasonable foul + good attack → play advantage.
    assert!(r.should_play_advantage(0.55, true, 0.3));
    // Lost possession → no advantage.
    assert!(!r.should_play_advantage(0.55, false, 0.3));
}

#[test]
fn elite_taker_vs_weak_keeper_floors_within_band() {
    let taker = score_penalty_taker(20.0, 20.0, 20.0, 20.0, 20.0, 1.0);
    let keeper = score_keeper_save(2.0, 2.0, 2.0, 2.0, 2.0, 2.0);
    let p = penalty_conversion_prob(taker, keeper, 0.0, false);
    assert!((0.58..=0.90).contains(&p));
    assert!(p > 0.80);
}

#[test]
fn wall_size_decreases_with_distance_band() {
    assert!(wall_size_for(FreeKickBand::Close, false) > wall_size_for(FreeKickBand::Far, false));
    let p_close = wall_block_prob(0.6, 14.0, 0.4, FreeKickBand::Close);
    let p_far = wall_block_prob(0.6, 14.0, 0.4, FreeKickBand::Far);
    assert!(p_close > p_far);
    assert!((0.08..=0.34).contains(&p_close));
    assert!((0.08..=0.34).contains(&p_far));
}

#[test]
fn fk_indirect_zero_shot_and_distance_zero_shot() {
    let env = MatchEnvironment::default();
    // Indirect — never a shot.
    let s = score_free_kick_choices(FreeKickBand::Close, true, 18.0, 14.0, 0.5, false, false, &env);
    assert_eq!(s.direct_shot, 0.0);
    // Far distance — also no shot.
    let s = score_free_kick_choices(FreeKickBand::Far, false, 18.0, 14.0, 0.5, false, false, &env);
    assert_eq!(s.direct_shot, 0.0);
}

#[test]
fn corner_winner_blocked_after_consecutive_failures() {
    let mut hist = SetPieceHistory::default();
    hist.record_corner(true, CornerRoutine::PenaltySpot, 0.02);
    hist.record_corner(true, CornerRoutine::PenaltySpot, 0.04);
    let env = MatchEnvironment::default();
    let scores = score_corner_routines(15.0, 15.0, 0.55, 0.50, false, false, &env);
    // PenaltySpot is normally the winner per spec base probs, but two
    // failed reps in a row → blocked.
    let pick = pick_corner_routine(&scores, &hist, true);
    assert_ne!(pick, CornerRoutine::PenaltySpot);
}

#[test]
fn leadership_dampens_negative_momentum() {
    let raw = -0.5;
    let captain_leadership = team_leadership_score(18.0, 16.0, 17.0, 17.0, 14.0, 14.0);
    let damped = leadership_damped_momentum(raw, captain_leadership);
    assert!(damped > raw);
    assert!(damped < 0.0);
}

#[test]
fn captain_leadership_dampens_pressure_load() {
    let env = MatchEnvironment {
        match_importance: 0.8,
        derby_intensity: 0.5,
        crowd_intensity: 0.85,
        ..Default::default()
    };
    let no_leader = pressure_load(&env, 0.7, 0.4, 0.0);
    let strong_leader = pressure_load(&env, 0.7, 0.4, 1.0);
    assert!(strong_leader < no_leader);
}

#[test]
fn confidence_swing_after_goal_then_error() {
    let mut p = PsychologyState::default();
    p.record_positive(1, PositiveEvent::Goal, 1000);
    let after_goal = p.get(1).unwrap().confidence;
    p.record_negative(1, NegativeEvent::ErrorLeadingToGoal, 1100);
    let after_error = p.get(1).unwrap().confidence;
    assert!(after_goal > 0.0);
    assert!(after_error < after_goal);
}

#[test]
fn yellow_card_raises_low_temperament_player_more_than_high() {
    let mut p = PsychologyState::default();
    p.apply_yellow_card(1, 18.0); // high temperament — calmer
    p.apply_yellow_card(2, 4.0); // low temperament — rattled
    let cool = p.get(1).cloned().unwrap_or_default();
    let rattled = p.get(2).cloned().unwrap_or_default();
    assert!(rattled.nervousness > cool.nervousness);
}

#[test]
fn very_high_nervousness_state_lifts_foul_risk_modifier() {
    let s = PsychState {
        nervousness: 0.7,
        ..Default::default()
    };
    let m = skill_modifiers(&s);
    assert!(m.foul_risk_add > 0.0);
    assert!(m.miscontrol_add > 0.0);
}

#[test]
fn chemistry_high_pair_passes_one_touch_better() {
    let pair = ChemistryInputs {
        role_a: Role::FullBack,
        lane_a: Lane::Left,
        role_b: Role::Winger,
        lane_b: Lane::Left,
        teamwork_a_0_20: 18.0,
        teamwork_b_0_20: 18.0,
        either_is_new: false,
    };
    let chem = initial_chemistry(pair);
    assert!(chem > 0.65);
    let m = chemistry_modifiers(chem);
    assert!(m.one_touch_pass_bonus > 0.0);
    assert!(m.give_and_go_selection_bonus > 0.0);
}

#[test]
fn chemistry_map_caches_pair_score_symmetric() {
    let mut m = ChemistryMap::default();
    m.set(7, 12, 0.78);
    assert_eq!(m.get(12, 7), Some(0.78));
}

#[test]
fn home_advantage_increases_with_crowd_but_stays_modest() {
    let neutral = MatchEnvironment::default();
    let big = MatchEnvironment {
        crowd_intensity: 1.0,
        home_advantage: 1.0,
        ..Default::default()
    };
    let nd = home_advantage_deltas(&neutral);
    let bd = home_advantage_deltas(&big);
    assert!(bd.referee_marginal_call_home_bias > nd.referee_marginal_call_home_bias);
    // Spec: home buffs stay modest (no arcade-tier additive deltas).
    assert!(bd.home_confidence_bonus <= 0.05);
    assert!(bd.home_press_intensity_bonus <= 0.04);
}

#[test]
fn time_wasting_kicks_in_when_leading_late_and_can_book() {
    // No delay before 75'.
    assert_eq!(time_wasting_delay_ms(1, 60, TimeWastingRestart::ThrowIn, 12.0), 0);
    // Substitution late while leading produces the longest delay.
    let sub = time_wasting_delay_ms(1, 88, TimeWastingRestart::Substitution, 12.0);
    let throw = time_wasting_delay_ms(1, 88, TimeWastingRestart::ThrowIn, 12.0);
    assert!(sub > throw);
    // Past 45s cumulative + strict referee → non-zero yellow chance.
    assert!(time_wasting_yellow_prob(60_000, 0.9, 1) > 0.0);
}

#[test]
fn muddy_pitch_plus_heavy_rain_compounds_first_touch_loss() {
    let env = MatchEnvironment {
        weather: Weather::HeavyRain,
        pitch: Pitch::Muddy,
        ..Default::default()
    };
    let m = env.modifiers();
    // Heavy rain alone is -0.11 first_touch; muddy doesn't add to first_touch
    // directly but should compound dribbling/fatigue/ball roll.
    assert!(m.first_touch <= -0.11);
    assert!(m.dribble_success < 0.0);
    assert!(m.fatigue_rate > 0.0);
}

#[test]
fn psych_neutral_state_is_neutral_modifiers() {
    let s = PsychState::default();
    let m = skill_modifiers(&s);
    assert_eq!(m.composure_mul, 1.0);
    assert_eq!(m.first_touch_mul, 1.0);
    assert_eq!(m.foul_risk_add, 0.0);
}
