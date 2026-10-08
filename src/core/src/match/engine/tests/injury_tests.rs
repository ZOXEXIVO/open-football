//! In-match injuries, end to end: how one happens, what it does to the
//! player, how the referee stops play for it, who replaces him, and what he
//! carries out of the match.

#![cfg(test)]

use super::substitution_break_tests::kickoff;
use crate::PlayerPositionType;
use crate::r#match::engine::ball::ball::PassOriginRestart;
use crate::r#match::engine::flow::arena::formation_variant::FormationLine;
use crate::r#match::engine::flow::result::SubstitutionReason;
use crate::r#match::engine::player::events::players::FoulSeverity;
use crate::r#match::engine::player::injury::{InjuryCause, InjuryGrade, InjuryRisk, MatchInjury};
use crate::r#match::engine::result::{DeadTime, PlayingTime};
use crate::r#match::engine::substitutions::Substitutions;
use crate::r#match::player::state::PlayerState;
use crate::r#match::player::strategies::players::ops::effective_skill::{
    ActionContext, effective_skill,
};
use crate::r#match::{MatchField, MatchPlayer, MatchState, PlayerSide};
use nalgebra::Vector3;

fn home_player(field: &MatchField, position: PlayerPositionType) -> u32 {
    field
        .players
        .iter()
        .find(|p| p.team_id == 1 && p.tactical_position.current_position == position)
        .map(|p| p.id)
        .expect("the home side fields this position")
}

fn with_body(player: &MatchPlayer, proneness: u8, natural_fitness: f32) -> MatchPlayer {
    let mut body = player.clone();
    body.player_attributes.injury_proneness = proneness;
    body.skills.physical.natural_fitness = natural_fitness;
    body
}

#[test]
fn a_reckless_foul_is_likelier_to_injure_than_a_clean_tackle() {
    let (field, _) = kickoff();
    let carrier = &field.players[5];
    let clean = InjuryRisk::contact_chance(carrier, InjuryRisk::CLEAN_TACKLE_IMPULSE, 0.0);
    let reckless = InjuryRisk::contact_chance(
        carrier,
        InjuryRisk::foul_impulse(FoulSeverity::Reckless),
        0.0,
    );
    assert!(reckless > clean * 4.0, "clean {clean}, reckless {reckless}");
    // Heavy rain on a muddy pitch makes the same contact worse.
    let wet = InjuryRisk::contact_chance(carrier, InjuryRisk::CLEAN_TACKLE_IMPULSE, 0.07);
    assert!(wet > clean);
}

#[test]
fn a_fragile_body_is_hurt_worse_by_the_same_contact() {
    let (field, _) = kickoff();
    let robust = with_body(&field.players[5], 1, 20.0);
    let fragile = with_body(&field.players[5], 20, 2.0);
    let impulse = InjuryRisk::foul_impulse(FoulSeverity::Normal);
    let roll = 0.10;
    assert_eq!(
        InjuryRisk::severity(InjuryCause::Contact, impulse, &fragile, roll),
        InjuryGrade::Serious
    );
    assert_ne!(
        InjuryRisk::severity(InjuryCause::Contact, impulse, &robust, roll),
        InjuryGrade::Serious
    );
}

#[test]
fn a_knock_is_shaken_off_and_a_serious_injury_is_not() {
    let (mut field, context) = kickoff();
    let now = context.total_match_time;
    let player = field
        .get_player_mut(home_player(&field, PlayerPositionType::MidfielderLeft))
        .unwrap();

    player.on_injury(InjuryGrade::Knock, now);
    assert_eq!(player.state, PlayerState::Injured);
    assert!(!player.is_treated(now + 1_000));
    assert!(player.is_treated(now + InjuryRisk::KNOCK_MS));
    assert_eq!(player.injury_handicap(), 1.0);

    player.on_injury(InjuryGrade::Serious, now);
    assert!(!player.is_treated(now + 10 * InjuryRisk::SERIOUS_MS));
    assert!(player.needs_replacing());
}

#[test]
fn a_hurt_player_plays_worse_for_the_rest_of_the_match() {
    let (mut field, context) = kickoff();
    let id = home_player(&field, PlayerPositionType::MidfielderLeft);
    let ctx = ActionContext::technical(60);
    let fit = effective_skill(field.get_player(id).unwrap(), 15.0, ctx);

    let player = field.get_player_mut(id).unwrap();
    player.on_injury(InjuryGrade::Hurt, context.total_match_time);
    player.refresh_skill_reads(60);
    let hurt = effective_skill(player, 15.0, ctx);

    assert!(hurt < fit * 0.95, "fit {fit}, hurt {hurt}");
}

#[test]
fn a_serious_injury_in_open_play_stops_it_for_a_drop_ball() {
    let (mut field, mut context) = kickoff();
    context.state.set(MatchState::SecondHalf);
    let carrier = home_player(&field, PlayerPositionType::MidfielderCenterLeft);
    let victim = home_player(&field, PlayerPositionType::MidfielderLeft);
    field.ball.position = Vector3::new(420.0, 272.0, 0.0);
    field.ball.current_owner = Some(carrier);

    MatchInjury::befall(
        &mut field,
        &mut context,
        victim,
        InjuryCause::Contact,
        InjuryGrade::Serious,
    );

    let restart = field.ball.awaiting_restart.expect("play has stopped");
    assert_eq!(restart.origin, PassOriginRestart::DropBall);
    assert_eq!(field.get_player(restart.taker_id).unwrap().team_id, 1);
    let now = context.current_tick();
    assert_eq!(
        field.ball.playing_time(now),
        PlayingTime::Dead(DeadTime::Treatment)
    );
    assert_eq!(context.tally.injury_count(), 1);

    // The treatment is dead time the referee gives back.
    for _ in 0..1_000 {
        context.note_tick(field.ball.playing_time(context.current_tick()));
        context.total_match_time += 10;
    }
    assert!(context.period_stoppage_time_ms > 8_000);
}

#[test]
fn a_drop_ball_in_the_box_goes_to_the_keeper() {
    let (mut field, mut context) = kickoff();
    let keeper = field
        .players
        .iter()
        .find(|p| {
            p.side == Some(PlayerSide::Left) && p.tactical_position.current_position.is_goalkeeper()
        })
        .map(|p| p.id)
        .unwrap();
    let striker = field
        .players
        .iter()
        .find(|p| {
            p.side == Some(PlayerSide::Right)
                && !p.tactical_position.current_position.is_goalkeeper()
        })
        .map(|p| p.id)
        .unwrap();
    field.ball.position = Vector3::new(40.0, 272.0, 0.0);
    field.ball.current_owner = Some(striker);

    MatchInjury::befall(
        &mut field,
        &mut context,
        striker,
        InjuryCause::Load,
        InjuryGrade::Hurt,
    );

    assert_eq!(field.ball.awaiting_restart.unwrap().taker_id, keeper);
}

#[test]
fn a_knock_does_not_stop_play() {
    let (mut field, mut context) = kickoff();
    let victim = home_player(&field, PlayerPositionType::MidfielderLeft);
    MatchInjury::befall(
        &mut field,
        &mut context,
        victim,
        InjuryCause::Contact,
        InjuryGrade::Knock,
    );
    assert!(field.ball.awaiting_restart.is_none());
}

#[test]
fn a_seriously_injured_man_is_replaced_for_an_injury() {
    let (mut field, mut context) = kickoff();
    let victim = home_player(&field, PlayerPositionType::MidfielderLeft);
    field
        .get_player_mut(victim)
        .unwrap()
        .on_injury(InjuryGrade::Serious, context.total_match_time);

    Substitutions::process_medical(&mut field, &mut context);

    let change = context
        .substitutions
        .iter()
        .find(|s| s.player_out_id == victim)
        .expect("he was replaced");
    assert_eq!(change.reason, SubstitutionReason::CriticalInjury);
}

#[test]
fn a_spent_man_is_replaced_for_exhaustion_not_injury() {
    let (mut field, mut context) = kickoff();
    let spent = home_player(&field, PlayerPositionType::MidfielderLeft);
    field
        .get_player_mut(spent)
        .unwrap()
        .player_attributes
        .condition = 1200;

    Substitutions::process_medical(&mut field, &mut context);

    let change = context
        .substitutions
        .iter()
        .find(|s| s.player_out_id == spent)
        .expect("he was replaced");
    assert_eq!(change.reason, SubstitutionReason::Exhaustion);
}

#[test]
fn with_no_change_left_a_serious_injury_leaves_the_side_a_man_short() {
    let (mut field, mut context) = kickoff();
    field.substitutes.retain(|p| p.team_id != 1);
    let victim = home_player(&field, PlayerPositionType::MidfielderLeft);
    field
        .get_player_mut(victim)
        .unwrap()
        .on_injury(InjuryGrade::Serious, context.total_match_time);

    Substitutions::process_medical(&mut field, &mut context);

    assert!(field.get_player(victim).unwrap().off_pitch);
    let on_pitch = field
        .players
        .iter()
        .filter(|p| p.team_id == 1 && !p.off_pitch)
        .count();
    assert_eq!(on_pitch, 10);
}

#[test]
fn a_sent_off_man_is_never_replaced() {
    let (mut field, mut context) = kickoff();
    let dismissed = home_player(&field, PlayerPositionType::MidfielderLeft);
    field.take_off(dismissed, FormationLine::Attack);
    field
        .get_player_mut(dismissed)
        .unwrap()
        .player_attributes
        .condition = 1200;

    Substitutions::process_medical(&mut field, &mut context);

    assert!(
        context
            .substitutions
            .iter()
            .all(|s| s.player_out_id != dismissed)
    );
}

#[test]
fn the_injury_a_player_carries_out_is_the_one_he_got() {
    let (field, _) = kickoff();
    let snapshot = {
        let mut player = field.players[5].clone();
        player.on_injury(InjuryGrade::Serious, 30 * 60_000);
        player.to_physical_snapshot(60 * 60_000)
    };
    assert_eq!(snapshot.injury, Some(InjuryGrade::Serious));
}
