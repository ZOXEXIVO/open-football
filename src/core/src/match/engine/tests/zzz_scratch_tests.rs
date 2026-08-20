//! scratch
#![cfg(test)]

use super::goal_celebration_tests::squad;
use crate::r#match::engine::result::Score;
use crate::r#match::{
    MatchContext, MatchField, MatchPlayerCollection, PlayerSide, events::EventCollection,
};
use nalgebra::Vector3;

fn run(pin: Vector3<f32>, label: &str) {
    let home = squad(1, 100);
    let away = squad(2, 200);
    let players = MatchPlayerCollection::from_squads(&home, &away);
    let mut field = MatchField::new(840, 545, home, away);
    let mut context = MatchContext::new(&field, players, Score::new(1, 2), false, false);
    context.total_match_time = 10 * 60 * 1000;

    let cid = field
        .players
        .iter()
        .find(|p| {
            p.side == Some(PlayerSide::Left) && !p.tactical_position.current_position.is_goalkeeper()
        })
        .map(|p| p.id)
        .unwrap();

    field.ball.position = pin;
    field.ball.velocity = Vector3::zeros();
    field.ball.current_owner = Some(cid);
    field.ball.previous_owner = Some(cid);
    field.ball.last_touch_player_id = Some(cid);

    println!("=== {label} pin={:?}", pin);
    for t in 0..24 {
        if let Some(p) = field.players.iter_mut().find(|p| p.id == cid) {
            p.position = pin;
        }
        let ps = field.players.clone();
        let mut events = EventCollection::with_capacity(8);
        context.increment_time();
        field.ball.update_light(&mut context, &ps, &mut events);
        println!(
            "  t{t:02} ball=({:.2},{:.2},{:.2}) owner={:?} restart={:?}",
            field.ball.position.x,
            field.ball.position.y,
            field.ball.position.z,
            field.ball.current_owner,
            field.ball.awaiting_restart.map(|r| r.origin)
        );
    }
}

#[test]
fn scratch_touchline() {
    run(Vector3::new(400.0, 545.5, 0.0), "touchline y=545.5");
}

#[test]
fn scratch_byline_outside_posts() {
    run(Vector3::new(840.5, 100.0, 0.0), "byline x=840.5 outside posts");
}

#[test]
fn scratch_touchline_exact() {
    run(Vector3::new(400.0, 545.0, 0.0), "touchline y=545.0 exact");
}

// ── whole-match reachability probe ───────────────────────────────────
use crate::r#match::engine::engine::FootballEngine;
use crate::r#match::{GameTickContext, ResultMatchPositionData};

#[test]
fn scratch_real_match_owned_ball_out_of_rect() {
    const W: usize = 840;
    const H: usize = 545;
    let mut home = squad(1, 100);
    let mut away = squad(2, 200);
    for s in [&mut home, &mut away] {
        for p in s.main_squad.iter_mut() {
            p.skills.physical.acceleration = 14.0;
            p.skills.physical.agility = 14.0;
            p.skills.physical.stamina = 14.0;
        }
    }
    let players = MatchPlayerCollection::from_squads(&home, &away);
    let mut field = MatchField::new(W, H, home, away);
    let mut context = MatchContext::new(&field, players, Score::new(1, 2), false, false);
    context.total_match_time = 0;
    let mut tick_context = GameTickContext::new(&field, &context.players);
    let mut recording = ResultMatchPositionData::empty();

    let mut owned_out = 0u32;
    let mut owned_out_episodes = 0u32;
    let mut prev_owned_out = false;
    let mut pinned_players = 0u32;
    let mut clamps = 0u32;
    const TICKS: usize = 30_000;
    for _ in 0..TICKS {
        context.increment_time();
        FootballEngine::<W, H>::game_tick(
            &mut field,
            &mut context,
            &mut recording,
            &mut tick_context,
        );
        let b = field.ball.position;
        let out = b.x <= 0.0 || b.x >= W as f32 || b.y <= 0.0 || b.y >= H as f32;
        let owned = field.ball.current_owner.is_some();
        if owned && out {
            owned_out += 1;
            if !prev_owned_out {
                owned_out_episodes += 1;
                println!(
                    "  episode at tick {} ball=({:.2},{:.2}) owner={:?} restart={:?}",
                    context.current_tick(),
                    b.x,
                    b.y,
                    field.ball.current_owner,
                    field.ball.awaiting_restart.map(|r| r.origin)
                );
            }
            prev_owned_out = true;
        } else {
            prev_owned_out = false;
        }
        // a non-taker sitting in the slack band
        let taker = field.ball.awaiting_restart.map(|r| r.taker_id);
        for p in field.players.iter() {
            if Some(p.id) == taker || p.is_sent_off {
                continue;
            }
            if p.position.x >= W as f32
                || p.position.x <= 0.0
                || p.position.y >= H as f32
                || p.position.y <= 0.0
            {
                pinned_players += 1;
            }
        }
        if owned && field.ball.current_owner.is_some() {
            if let Some(o) = field
                .players
                .iter()
                .find(|p| Some(p.id) == field.ball.current_owner)
            {
                if (o.position - field.ball.position).norm() > 5.0
                    && field.ball.awaiting_restart.is_none()
                {
                    clamps += 1;
                }
            }
        }
    }
    println!(
        "TICKS={TICKS} owned_out_ticks={owned_out} episodes={owned_out_episodes} pinned_player_ticks={pinned_players} owner_far_ticks={clamps}"
    );
}

#[test]
fn scratch_long_match_owned_ball_near_lines() {
    const W: usize = 840;
    const H: usize = 545;
    let mut home = squad(1, 100);
    let mut away = squad(2, 200);
    for s in [&mut home, &mut away] {
        for p in s.main_squad.iter_mut() {
            p.skills.physical.acceleration = 14.0;
            p.skills.physical.agility = 14.0;
            p.skills.physical.stamina = 14.0;
        }
    }
    let players = MatchPlayerCollection::from_squads(&home, &away);
    let mut field = MatchField::new(W, H, home, away);
    let mut context = MatchContext::new(&field, players, Score::new(1, 2), false, false);
    context.total_match_time = 0;
    let mut tick_context = GameTickContext::new(&field, &context.players);
    let mut recording = ResultMatchPositionData::empty();

    let mut owned_out = 0u32;
    let mut max_owned_y = 0.0f32;
    let mut min_owned_y = 1e9f32;
    let mut max_owned_x = 0.0f32;
    let mut min_owned_x = 1e9f32;
    let mut owned_within_2u = 0u32;
    let mut owner_out_of_rect = 0u32; // the OWNER himself out, whatever the ball
    const TICKS: usize = 540_000;
    for _ in 0..TICKS {
        context.increment_time();
        FootballEngine::<W, H>::game_tick(
            &mut field,
            &mut context,
            &mut recording,
            &mut tick_context,
        );
        if field.ball.current_owner.is_none() {
            continue;
        }
        let b = field.ball.position;
        max_owned_y = max_owned_y.max(b.y);
        min_owned_y = min_owned_y.min(b.y);
        max_owned_x = max_owned_x.max(b.x);
        min_owned_x = min_owned_x.min(b.x);
        if b.x <= 0.0 || b.x >= W as f32 || b.y <= 0.0 || b.y >= H as f32 {
            owned_out += 1;
        } else if b.x <= 2.0 || b.x >= W as f32 - 2.0 || b.y <= 2.0 || b.y >= H as f32 - 2.0 {
            owned_within_2u += 1;
        }
        let taker = field.ball.awaiting_restart.map(|r| r.taker_id);
        if let Some(o) = field
            .players
            .iter()
            .find(|p| Some(p.id) == field.ball.current_owner)
        {
            if Some(o.id) != taker
                && (o.position.x <= 0.0
                    || o.position.x >= W as f32
                    || o.position.y <= 0.0
                    || o.position.y >= H as f32)
            {
                owner_out_of_rect += 1;
            }
        }
    }
    println!(
        "TICKS={TICKS} owned_out={owned_out} owned_within_2u={owned_within_2u} owner_out_of_rect={owner_out_of_rect} owned_x=[{min_owned_x:.2},{max_owned_x:.2}] owned_y=[{min_owned_y:.2},{max_owned_y:.2}]"
    );
}
