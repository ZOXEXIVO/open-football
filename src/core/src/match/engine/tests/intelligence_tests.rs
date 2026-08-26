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
use crate::r#match::engine::sub_scoring::SubScoring;
use crate::r#match::player::strategies::players::ops::dribble_duel::{
    DribbleDuelResolver, DuelContext,
};
use crate::r#match::player::strategies::players::ops::effective_skill::{
    ActionContext, effective_skill,
};
use crate::r#match::player::strategies::players::ops::skill_composites as sc;
use crate::r#match::player::strategies::players::ops::traits_bias::{movement_bias, passing_bias};
use crate::r#match::{MatchPlayer, RatingContext};
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
    MatchPlayer::from_player(1, &player, pos, false, None)
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

/// The reception composite the LIVE model rolls against
/// (`Ball::roll_first_touch` scales `(1.15 - quality)^1.6` by it, and
/// `maybe_record_first_touch_loss` books the stat off the same idea).
///
/// Rewritten 2026-08-26: this used to drive `resolve_first_touch`, a
/// parallel reception resolver with no match callers at all, so it was
/// pinning behaviour the engine never executed. Deleted with it — the
/// property is real, so it is now asserted where the football happens.
#[test]
fn elite_first_touch_controls_better_than_poor() {
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
    let elite_q = sc::receiving_first_touch(&elite, 30);
    let poor_q = sc::receiving_first_touch(&poor, 30);
    assert!(elite_q > poor_q + 0.15, "elite={elite_q}, poor={poor_q}");

    // …and the gap has to survive the curve the live roll applies, or a
    // better receiver is better on paper and identical on the pitch.
    let loss = |q: f32| (0.006 + 0.5 * 0.060) * (1.15 - q).powf(1.6);
    assert!(
        loss(poor_q) > loss(elite_q) * 1.5,
        "poor={}, elite={}",
        loss(poor_q),
        loss(elite_q)
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
    assert!(sc::receiving_first_touch(&p_fresh, 80) > sc::receiving_first_touch(&p_tired, 80));
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
        let r1 = DribbleDuelResolver::resolve(&attacker, &stays, DuelContext::default(), roll);
        let r2 = DribbleDuelResolver::resolve(&attacker, &dives, DuelContext::default(), roll);
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
    let attacker_score = SubScoring::sub_in_score(&attacker, TacticalNeed::Chasing, 1.0, 0.0);
    let defender_score = SubScoring::sub_in_score(&defender, TacticalNeed::Chasing, 1.0, 0.0);
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
    let attacker_score =
        SubScoring::sub_in_score(&attacker, TacticalNeed::ProtectingLead, 1.0, 0.0);
    let defender_score =
        SubScoring::sub_in_score(&defender, TacticalNeed::ProtectingLead, 1.0, 0.0);
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
        raw_match_rating: 0.0,
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
        pressures: 0,
        blocks: 0,
        clearances: 0,
        passes_into_box: 0,
        crosses_attempted: 0,
        crosses_completed: 0,
        xg_chain: 0.0,
        xg_buildup: 0.0,
        miscontrols: 0,
        heavy_touches: 0,
        carry_distance: 0,
        errors_leading_to_shot: 0,
        errors_leading_to_goal: 0,
        xg_prevented: 0.0,
        xg_faced: 0.0,
        offsides: 0,
        own_goals: 0,
        zone_stats: Default::default(),
    };
    let clean_rating = RatingContext::new(&clean, 1, 1).calculate();
    clean.errors_leading_to_goal = 1;
    let with_error = RatingContext::new(&clean, 1, 1).calculate();
    assert!(with_error < clean_rating - 0.5);
}

#[test]
fn gk_rating_reads_the_difficulty_of_the_shots_faced() {
    use crate::r#match::engine::result::PlayerMatchEndStats;
    // Mirrors `keeper::REF_XGOT_PER_ON_TARGET` — the engine population
    // mean POST-shot expected goal of a shot on target, i.e. what a
    // league-average keeper concedes from an ordinary strike. Pre-shot xG
    // (the 0.1136 this used to carry) scores the chance the defence
    // conceded, not the strike the keeper had to stop.
    const ORDINARY_CHANCE_XGOT: f32 = 0.562;
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
        raw_match_rating: 0.0,
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
        pressures: 0,
        blocks: 0,
        clearances: 0,
        passes_into_box: 0,
        crosses_attempted: 0,
        crosses_completed: 0,
        xg_chain: 0.0,
        xg_buildup: 0.0,
        miscontrols: 0,
        heavy_touches: 0,
        carry_distance: 0,
        errors_leading_to_shot: 0,
        errors_leading_to_goal: 0,
        xg_prevented: 0.0,
        xg_faced: 0.0,
        offsides: 0,
        own_goals: 0,
        zone_stats: Default::default(),
    };
    // `xg_faced` is what a league-average keeper would have conceded from
    // the strikes this one dealt with, and it sets the bar his outcome is
    // measured against. Same saves, same goals, harder strikes ⇒ a better
    // afternoon.
    //
    // Multipliers stay inside what `expected_goal_on_target` can actually
    // produce: a single strike is bounded to [1 − MAX_SAVE, 1 − MIN_SAVE]
    // = [0.08, 0.92], so the per-shot mean cannot exceed 1.64× the
    // reference. Feeding 2× (as this fixture used to, in pre-shot units
    // where it was reachable) asks the model a question the engine can
    // never pose, and the answer — a keeper credited with an expectation
    // above one goal per shot on target — is meaningless.
    const HARD: f32 = 1.5; // per-shot 0.843, just under the 0.92 ceiling
    const EASY: f32 = 0.45; // per-shot 0.253, a tame afternoon
    base.xg_faced = 6.0 * ORDINARY_CHANCE_XGOT;
    let ordinary = RatingContext::new(&base, 1, 1).calculate();
    base.xg_faced = 6.0 * ORDINARY_CHANCE_XGOT * HARD;
    let hard = RatingContext::new(&base, 1, 1).calculate();
    base.xg_faced = 6.0 * ORDINARY_CHANCE_XGOT * EASY;
    let easy = RatingContext::new(&base, 1, 1).calculate();
    assert!(
        hard > ordinary && ordinary > easy,
        "chance difficulty must order the same stat line: easy {easy:.3} \
         < ordinary {ordinary:.3} < hard {hard:.3}"
    );
    // And it conditions the expectation without ever overturning the
    // outcome: facing the hardest strikes the model can produce does not
    // buy a keeper who shipped three the rating of one who shipped one
    // from the tamest. `DEFENSIVE_OUTCOME` is what guarantees this — it
    // reads goals conceded against a league-average night, and no
    // difficulty multiplier touches it.
    base.xg_faced = 6.0 * ORDINARY_CHANCE_XGOT * EASY;
    base.saves = 5;
    let one_conceded_easy = RatingContext::new(&base, 1, 1).calculate();
    base.xg_faced = 6.0 * ORDINARY_CHANCE_XGOT * HARD;
    base.saves = 3;
    let three_conceded_hard = RatingContext::new(&base, 1, 3).calculate();
    assert!(
        one_conceded_easy > three_conceded_hard,
        "chance difficulty must not overturn the outcome: one conceded \
         from easy chances {one_conceded_easy:.3} must beat three from \
         hard ones {three_conceded_hard:.3}"
    );
}
