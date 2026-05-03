//! Regression tests for the player-individuality / match-intelligence
//! rework (Sections 1–9). These pin the cross-cutting behaviours that
//! the unit tests in each helper module check from below — here we
//! check the end-to-end intent: elite first touch wins under pressure,
//! exhausted players degrade, traits change movement / passing, the
//! coach reads xG and territory, the substitution scoring picks the
//! right replacement, and the rating formula penalises errors / lifts
//! GKs by xG-prevented.

#![cfg(test)]

use crate::club::player::builder::PlayerBuilder;
use crate::club::player::traits::PlayerTrait;
use crate::r#match::engine::coach::{
    CoachInstruction, MatchCoach, RollingTeamMetrics, TacticalNeed,
};
use crate::r#match::engine::sub_scoring::sub_in_score;
use crate::r#match::player::strategies::players::ops::dribble_duel::{
    DuelContext, resolve_dribble_duel,
};
use crate::r#match::player::strategies::players::ops::effective_skill::{
    ActionContext, effective_skill,
};
use crate::r#match::player::strategies::players::ops::first_touch::{
    PassContext, ReceiverPressure, resolve_first_touch,
};
use crate::r#match::player::strategies::players::ops::traits_bias::{movement_bias, passing_bias};
use crate::r#match::{MatchPlayer, calculate_match_rating};
use crate::shared::fullname::FullName;
use crate::{
    PersonAttributes, PlayerAttributes, PlayerFieldPositionGroup, PlayerPosition,
    PlayerPositionType, PlayerPositions, PlayerSkills,
};
use chrono::NaiveDate;

fn build(
    pos: PlayerPositionType,
    skills: PlayerSkills,
    condition: i16,
    traits: Vec<PlayerTrait>,
) -> MatchPlayer {
    let mut attrs = PlayerAttributes::default();
    attrs.condition = condition;
    let mut player = PlayerBuilder::new()
        .id(1)
        .full_name(FullName::new("T".to_string(), "P".to_string()))
        .birth_date(NaiveDate::from_ymd_opt(2000, 1, 1).unwrap())
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
        .unwrap();
    player.traits = traits;
    MatchPlayer::from_player(1, &player, pos, false)
}

fn full_skills(value: f32) -> PlayerSkills {
    let mut s = PlayerSkills::default();
    s.technical.first_touch = value;
    s.technical.passing = value;
    s.technical.technique = value;
    s.technical.dribbling = value;
    s.technical.tackling = value;
    s.technical.marking = value;
    s.technical.crossing = value;
    s.technical.finishing = value;
    s.mental.composure = value;
    s.mental.decisions = value;
    s.mental.vision = value;
    s.mental.anticipation = value;
    s.mental.concentration = value;
    s.mental.positioning = value;
    s.mental.flair = value;
    s.mental.off_the_ball = value;
    s.mental.work_rate = value;
    s.mental.determination = value;
    s.mental.aggression = value;
    s.physical.balance = value;
    s.physical.agility = value;
    s.physical.acceleration = value;
    s.physical.pace = value;
    s.physical.strength = value;
    s.physical.stamina = value;
    s.physical.natural_fitness = value;
    s
}

#[test]
fn elite_first_touch_keeps_possession_more_often_than_poor() {
    let elite = build(
        PlayerPositionType::ForwardCenter,
        full_skills(17.0),
        9000,
        vec![],
    );
    let poor = build(
        PlayerPositionType::ForwardCenter,
        full_skills(7.0),
        9000,
        vec![],
    );
    let pass = PassContext {
        distance_units: 80.0,
        driven: true,
        ..Default::default()
    };
    let pressure = ReceiverPressure {
        nearest_defender: 4.0,
        defenders_within_6u: 1,
        ..Default::default()
    };
    let mut elite_kept = 0;
    let mut poor_kept = 0;
    for i in 0..40 {
        let r = (i as f32 + 0.5) / 40.0;
        if resolve_first_touch(&elite, pass, pressure, 30, r)
            .outcome
            .keeps_possession()
        {
            elite_kept += 1;
        }
        if resolve_first_touch(&poor, pass, pressure, 30, r)
            .outcome
            .keeps_possession()
        {
            poor_kept += 1;
        }
    }
    assert!(
        elite_kept > poor_kept + 10,
        "elite={}, poor={}",
        elite_kept,
        poor_kept
    );
}

#[test]
fn exhausted_player_first_touch_degrades() {
    let p_fresh = build(
        PlayerPositionType::ForwardCenter,
        full_skills(15.0),
        9000,
        vec![],
    );
    let p_tired = build(
        PlayerPositionType::ForwardCenter,
        full_skills(15.0),
        2500,
        vec![],
    );
    let pass = PassContext::default();
    let pressure = ReceiverPressure {
        nearest_defender: 4.0,
        defenders_within_6u: 1,
        ..Default::default()
    };
    let r_fresh = resolve_first_touch(&p_fresh, pass, pressure, 80, 0.5);
    let r_tired = resolve_first_touch(&p_tired, pass, pressure, 80, 0.5);
    assert!(r_fresh.control_score > r_tired.control_score);
}

#[test]
fn fatigue_affects_explosive_more_than_technical() {
    let player = build(
        PlayerPositionType::MidfielderCenter,
        full_skills(15.0),
        2800,
        vec![],
    );
    let tech = effective_skill(&player, 15.0, ActionContext::technical(80));
    let expl = effective_skill(&player, 15.0, ActionContext::explosive(80));
    assert!(expl < tech);
}

#[test]
fn runs_with_ball_often_does_not_remove_attempts() {
    // RunsWithBallRarely is the negative trait — make sure it caps
    // forward run propensity (movement_bias.forward_run_cap_delta < 0).
    let rare = build(
        PlayerPositionType::ForwardCenter,
        full_skills(14.0),
        9000,
        vec![PlayerTrait::RunsWithBallRarely],
    );
    let m = movement_bias(&rare);
    assert!(m.forward_run_cap_delta < 0.0);
}

#[test]
fn hugs_line_keeps_winger_wider_than_cuts_inside() {
    let hugs = build(
        PlayerPositionType::ForwardLeft,
        full_skills(14.0),
        9000,
        vec![PlayerTrait::HugsLine],
    );
    let cuts = build(
        PlayerPositionType::ForwardLeft,
        full_skills(14.0),
        9000,
        vec![PlayerTrait::CutsInsideFromBothWings],
    );
    let h = movement_bias(&hugs);
    let c = movement_bias(&cuts);
    // Higher (positive) touchline_offset_units = stays wider.
    assert!(h.touchline_offset_units > c.touchline_offset_units);
}

#[test]
fn dives_into_tackles_increases_foul_risk_in_duels() {
    let attacker = build(
        PlayerPositionType::ForwardCenter,
        full_skills(13.0),
        9000,
        vec![],
    );
    let stays = build(
        PlayerPositionType::DefenderCenter,
        full_skills(13.0),
        9000,
        vec![PlayerTrait::StaysOnFeet],
    );
    let dives = build(
        PlayerPositionType::DefenderCenter,
        full_skills(13.0),
        9000,
        vec![PlayerTrait::DivesIntoTackles],
    );
    let mut s_fouls = 0;
    let mut d_fouls = 0;
    for i in 0..200 {
        let roll = (i as f32 + 0.5) / 200.0;
        let r1 = resolve_dribble_duel(&attacker, &stays, DuelContext::default(), roll);
        let r2 = resolve_dribble_duel(&attacker, &dives, DuelContext::default(), roll);
        if r1.outcome.is_foul() {
            s_fouls += 1;
        }
        if r2.outcome.is_foul() {
            d_fouls += 1;
        }
    }
    assert!(d_fouls > s_fouls);
}

#[test]
fn playmaker_attempts_more_progressive_passes_via_bias() {
    let playmaker = build(
        PlayerPositionType::MidfielderCenter,
        full_skills(15.0),
        9000,
        vec![PlayerTrait::Playmaker, PlayerTrait::TriesThroughBalls],
    );
    let plain = build(
        PlayerPositionType::MidfielderCenter,
        full_skills(15.0),
        9000,
        vec![],
    );
    let pb = passing_bias(&playmaker);
    let plain_pb = passing_bias(&plain);
    assert!(pb.through_ball_bonus > plain_pb.through_ball_bonus);
    assert!(pb.ask_for_ball_bonus > plain_pb.ask_for_ball_bonus);
}

#[test]
fn coach_does_not_go_all_out_when_drawing_but_dominating_xg() {
    let mut coach = MatchCoach::new();
    let metrics = RollingTeamMetrics {
        xg_for_last_15: 1.4,
        xg_against_last_15: 0.3,
        ..Default::default()
    };
    // Drawing 0–0 in the 80th minute. Without metrics, evaluate would
    // pick AllOutAttack. With metrics showing xG dominance, we expect
    // PushForward at most.
    coach.evaluate_with_metrics(0, 0.83, 0.8, 5000, metrics);
    assert!(matches!(
        coach.instruction,
        CoachInstruction::PushForward | CoachInstruction::Normal
    ));
}

#[test]
fn coach_lowers_press_when_press_failing_and_team_tired() {
    let mut coach = MatchCoach::new();
    coach.instruction = CoachInstruction::AllOutAttack;
    let metrics = RollingTeamMetrics {
        press_success_rate_last_10: 0.25,
        ..Default::default()
    };
    // condition < 0.55 + low press success → step down to Normal.
    coach.evaluate_with_metrics(0, 0.5, 0.45, 5000, metrics);
    assert_ne!(coach.instruction, CoachInstruction::AllOutAttack);
}

#[test]
fn chasing_substitution_prefers_attacker() {
    let mut s_fwd = full_skills(14.0);
    s_fwd.technical.finishing = 17.0;
    s_fwd.physical.pace = 17.0;
    let attacker = build(
        PlayerPositionType::ForwardCenter,
        s_fwd,
        9000,
        vec![PlayerTrait::GetsIntoOppositionArea],
    );
    let defender = build(
        PlayerPositionType::DefenderCenter,
        full_skills(14.0),
        9000,
        vec![PlayerTrait::StaysBack],
    );
    let attacker_score = sub_in_score(&attacker, TacticalNeed::Chasing, 1.0, 0.0);
    let defender_score = sub_in_score(&defender, TacticalNeed::Chasing, 1.0, 0.0);
    assert!(
        attacker_score > defender_score,
        "attacker={:.3}, defender={:.3}",
        attacker_score,
        defender_score
    );
}

#[test]
fn protecting_lead_substitution_prefers_defender() {
    let attacker = build(
        PlayerPositionType::ForwardCenter,
        full_skills(14.0),
        9000,
        vec![],
    );
    let mut s_def = full_skills(14.0);
    s_def.technical.tackling = 17.0;
    s_def.mental.positioning = 17.0;
    let defender = build(
        PlayerPositionType::DefenderCenter,
        s_def,
        9000,
        vec![PlayerTrait::StaysBack, PlayerTrait::MarkTightly],
    );
    let attacker_score = sub_in_score(&attacker, TacticalNeed::ProtectingLead, 1.0, 0.0);
    let defender_score = sub_in_score(&defender, TacticalNeed::ProtectingLead, 1.0, 0.0);
    assert!(defender_score > attacker_score);
}

#[test]
fn match_rating_penalises_error_leading_to_goal() {
    use crate::r#match::engine::result::PlayerMatchEndStats;
    let mut clean = PlayerMatchEndStats {
        shots_on_target: 0,
        shots_total: 0,
        passes_attempted: 30,
        passes_completed: 25,
        tackles: 0,
        interceptions: 0,
        saves: 0,
        shots_faced: 0,
        goals: 0,
        assists: 0,
        match_rating: 0.0,
        xg: 0.0,
        position_group: PlayerFieldPositionGroup::Defender,
        fouls: 0,
        yellow_cards: 0,
        red_cards: 0,
        minutes_played: 90,
        key_passes: 0,
        progressive_passes: 0,
        progressive_carries: 0,
        successful_dribbles: 0,
        attempted_dribbles: 0,
        successful_pressures: 0,
        blocks: 0,
        clearances: 0,
        errors_leading_to_shot: 0,
        errors_leading_to_goal: 0,
        xg_prevented: 0.0,
    };
    let clean_rating = calculate_match_rating(&clean, 1, 1);
    clean.errors_leading_to_goal = 1;
    let with_error = calculate_match_rating(&clean, 1, 1);
    assert!(with_error < clean_rating - 0.5);
}

#[test]
fn gk_rating_uses_xg_prevented() {
    use crate::r#match::engine::result::PlayerMatchEndStats;
    let mut base = PlayerMatchEndStats {
        shots_on_target: 0,
        shots_total: 0,
        passes_attempted: 20,
        passes_completed: 18,
        tackles: 0,
        interceptions: 0,
        saves: 5,
        shots_faced: 6,
        goals: 0,
        assists: 0,
        match_rating: 0.0,
        xg: 0.0,
        position_group: PlayerFieldPositionGroup::Goalkeeper,
        fouls: 0,
        yellow_cards: 0,
        red_cards: 0,
        minutes_played: 90,
        key_passes: 0,
        progressive_passes: 0,
        progressive_carries: 0,
        successful_dribbles: 0,
        attempted_dribbles: 0,
        successful_pressures: 0,
        blocks: 0,
        clearances: 0,
        errors_leading_to_shot: 0,
        errors_leading_to_goal: 0,
        xg_prevented: 0.0,
    };
    let baseline = calculate_match_rating(&base, 1, 1);
    base.xg_prevented = 1.0;
    let lifted = calculate_match_rating(&base, 1, 1);
    assert!(lifted > baseline);
    base.xg_prevented = -1.0;
    let dropped = calculate_match_rating(&base, 1, 1);
    assert!(dropped < baseline);
}
