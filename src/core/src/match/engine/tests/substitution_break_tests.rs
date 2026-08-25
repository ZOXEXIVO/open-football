//! A substitution, end to end: the board going up at a dead ball, one man
//! running off before the other comes on, the twenty standing still while
//! they do it, and the roster that changed before any of it started.
//!
//! Everything asserted here is about POSITION OVER TIME, because everything
//! that was wrong before was. The roster half of a substitution has always
//! worked and has its own tests next to the scorer; what did not exist was
//! any answer to "where were those two men while it happened", and a test
//! that only checked the ledger would have passed against a body swap.
//!
//! The properties worth guarding, in order:
//!
//! 1. **The exchange is sequential.** The man coming on does not move until
//!    the man coming off is over the line. Simultaneous runs were the first
//!    thing reported back when this was watched.
//! 2. **Neither walker teleports.** The window closes when the last man is on
//!    rather than on a clock, so the only way the tidy-up at the end becomes
//!    the whole-pitch write it replaced is if a walk runs out of
//!    [`SubstitutionBreak::BREAK_MS`] first — which is what the step
//!    assertions catch.
//! 3. **Nobody else moves**, and nobody who is not part of the change is
//!    drawn beside the pitch at all.

#![cfg(test)]

use crate::Tactics;
use crate::club::player::builder::PlayerBuilder;
use crate::club::player::{PlayerPosition, PlayerPositions};
use crate::club::team::tactics::MatchTacticType;
use crate::r#match::engine::ball::ball::{AwaitedRestart, PassOriginRestart};
use crate::r#match::engine::engine::FootballEngine;
use crate::r#match::engine::result::Score;
use crate::r#match::engine::substitutions::{Substitutions, process_substitutions};
use crate::r#match::engine::touchline::{Bench, SubstitutionBreak, advance_substitution_break};
use crate::r#match::result::ResultMatchPositionData;
use crate::r#match::squad::squad::MatchSquad;
use crate::r#match::{MatchContext, MatchField, MatchPlayer, MatchPlayerCollection};
use crate::shared::fullname::FullName;
use crate::{PersonAttributes, PlayerAttributes, PlayerPositionType, PlayerSkills};
use chrono::NaiveDate;
use nalgebra::Vector3;
use std::collections::HashMap;

const XI: [PlayerPositionType; 11] = [
    PlayerPositionType::Goalkeeper,
    PlayerPositionType::DefenderLeft,
    PlayerPositionType::DefenderCenterLeft,
    PlayerPositionType::DefenderCenterRight,
    PlayerPositionType::DefenderRight,
    PlayerPositionType::MidfielderLeft,
    PlayerPositionType::MidfielderCenterLeft,
    PlayerPositionType::MidfielderCenterRight,
    PlayerPositionType::MidfielderRight,
    PlayerPositionType::ForwardLeft,
    PlayerPositionType::ForwardRight,
];

const BENCH: [PlayerPositionType; 7] = [
    PlayerPositionType::Goalkeeper,
    PlayerPositionType::DefenderCenterLeft,
    PlayerPositionType::DefenderRight,
    PlayerPositionType::MidfielderCenterLeft,
    PlayerPositionType::MidfielderRight,
    PlayerPositionType::ForwardLeft,
    PlayerPositionType::ForwardRight,
];

const TICK_MS: u64 = 10;

fn player(team_id: u32, id: u32, position: PlayerPositionType) -> MatchPlayer {
    let mut attributes = PlayerAttributes::default();
    attributes.condition = 9000;
    attributes.current_ability = 150;
    let built = PlayerBuilder::new()
        .id(id)
        .full_name(FullName::new("T".to_string(), format!("P{id}")))
        .birth_date(NaiveDate::from_ymd_opt(1998, 5, 1).unwrap())
        .country_id(1)
        .attributes(PersonAttributes::default())
        .skills(PlayerSkills::default())
        .positions(PlayerPositions {
            positions: vec![PlayerPosition {
                position,
                level: 18,
            }],
        })
        .player_attributes(attributes)
        .build()
        .unwrap();
    MatchPlayer::from_player(team_id, &built, position, false, None)
}

fn squad(team_id: u32, base_id: u32) -> MatchSquad {
    MatchSquad {
        team_id,
        team_name: format!("Team{team_id}"),
        tactics: Tactics::new(MatchTacticType::T442),
        main_squad: XI
            .iter()
            .enumerate()
            .map(|(index, position)| player(team_id, base_id + index as u32, *position))
            .collect(),
        substitutes: BENCH
            .iter()
            .enumerate()
            .map(|(index, position)| player(team_id, base_id + 50 + index as u32, *position))
            .collect(),
        captain_id: None,
        vice_captain_id: None,
        penalty_taker_id: None,
        free_kick_taker_id: None,
        selection_omissions: vec![],
        coach_snapshot: None,
    }
}

/// Two sides with a bench each, an hour into the match.
fn kickoff() -> (MatchField, MatchContext) {
    let home = squad(1, 100);
    let away = squad(2, 300);
    let players = MatchPlayerCollection::from_squads(&home, &away);
    let field = MatchField::new(840, 545, home, away);
    let mut context = MatchContext::new(&field, players, Score::new(1, 2), false, false);
    context.total_match_time = 60 * 60_000;
    (field, context)
}

/// Put one home outfielder on the floor so the forced-injury branch has to
/// take him off, and stand him on the touchline OPPOSITE the dugouts — the
/// longest walk the engine can ask anybody for.
///
/// Returns his id.
fn injure_the_furthest_man(field: &mut MatchField) -> u32 {
    let victim = field
        .players
        .iter_mut()
        .find(|p| {
            p.team_id == 1
                && p.tactical_position.current_position == PlayerPositionType::MidfielderLeft
        })
        .expect("a home midfielder exists");
    victim.player_attributes.condition = 1200;
    victim.position = Vector3::new(120.0, 541.0, 0.0);
    victim.id
}

/// One tick's sample of everybody who is drawn: on-pitch men from their own
/// coordinate, off-pitch men from their touchline stand, which is exactly the
/// split the recorder writes.
fn sample(field: &MatchField, tracks: &mut HashMap<u32, Vec<Vector3<f32>>>) {
    for p in field.players.iter() {
        tracks.entry(p.id).or_default().push(p.position);
    }
    for p in field.off_pitch() {
        if let Some(stand) = p.touchline {
            tracks.entry(p.id).or_default().push(stand.at);
        }
    }
}

/// Run the window to its end, sampling every walker every tick.
///
/// ⚠ **The recorder has to run too, on its own cadence.** It is not a passive
/// reader: `write_match_positions` walks everybody off the pitch a step
/// toward his seat while it is there. Leave it out and the walk measured here
/// is not the walk the replay shows — which is exactly how the man coming off
/// came to be moved twice a frame, by the window and by the recorder, in two
/// different directions.
fn play_out_the_window(
    field: &mut MatchField,
    context: &mut MatchContext,
) -> HashMap<u32, Vec<Vector3<f32>>> {
    let mut recording = ResultMatchPositionData::new();
    let mut tracks: HashMap<u32, Vec<Vector3<f32>>> = HashMap::new();

    sample(field, &mut tracks);
    let mut guard = 0;
    while context.substitution_break.is_some() {
        context.total_match_time += TICK_MS;
        advance_substitution_break(field, context);
        let now = context.total_match_time;
        if now % 30 == 0 {
            FootballEngine::<840, 545>::write_match_positions(field, now, &mut recording);
        }
        sample(field, &mut tracks);
        guard += 1;
        assert!(guard < 10_000, "the window never closed");
    }
    tracks
}

/// Longest single-tick move on a track.
fn longest_step(track: &[Vector3<f32>]) -> f32 {
    track
        .windows(2)
        .map(|pair| {
            let delta = pair[1] - pair[0];
            (delta.x * delta.x + delta.y * delta.y).sqrt()
        })
        .fold(0.0f32, f32::max)
}

#[test]
fn a_substitute_who_is_not_coming_on_is_not_drawn_at_all() {
    let (mut field, _context) = kickoff();

    assert_eq!(field.substitutes.len(), BENCH.len() * 2);
    for sub in &field.substitutes {
        assert!(
            sub.touchline.is_none(),
            "substitute {} is being drawn beside the pitch before he is needed",
            sub.id
        );
        // …and his own coordinate is still the sentinel every proximity scan
        // in the engine has to see him at.
        assert_eq!(sub.position, Vector3::new(-500.0, -500.0, 0.0));
    }

    let mut data = ResultMatchPositionData::new();
    FootballEngine::<840, 545>::write_match_positions(&mut field, 30, &mut data);
    for sub in &field.substitutes {
        assert!(
            data.get_player_position_at(sub.id, 30).is_none(),
            "the recorder drew substitute {}, who is on the bench",
            sub.id
        );
    }
}

#[test]
fn a_substitution_stops_the_match_and_walks_both_men() {
    let (mut field, mut context) = kickoff();
    let out_id = injure_the_furthest_man(&mut field);
    let left_at = field.get_player(out_id).unwrap().position;

    let opened_at = context.total_match_time;
    let today = context.today;
    process_substitutions(&mut field, &mut context, 3, today);

    // The roster changed on this tick — that half is unconditional, and the
    // decision layer depends on it (see the `changeover` module note).
    assert!(
        field.players.iter().all(|p| p.id != out_id),
        "the man taken off is still one of the eleven"
    );
    let change = *context
        .substitution_break
        .as_ref()
        .expect("a substitution must open a window")
        .changes()
        .iter()
        .find(|c| c.player_out_id == out_id)
        .expect("the injured man's change is in the window");
    let in_id = change.player_in_id;
    assert_eq!(
        context.dead_ball_until_ms,
        opened_at + SubstitutionBreak::BREAK_MS + SubstitutionBreak::PORTRAIT_MS,
        "play must stop for at most the ceiling, plus the beat the picture \
         spends on each man being replaced"
    );

    // Both men start where they really are: him on the pitch, the substitute
    // at the fourth official's shoulder.
    let departed = field
        .departed
        .iter()
        .find(|p| p.id == out_id)
        .expect("he is on the touchline list now");
    assert_eq!(departed.touchline.unwrap().at, left_at);
    let coming_on = field.players.iter().find(|p| p.id == in_id).unwrap();
    assert!(
        Bench::is_over_the_line(coming_on.position),
        "the substitute is already on the pitch: {:?}",
        coming_on.position
    );
    let slot = coming_on.start_position;
    let entered_at = coming_on.position;

    // Everybody else, before.
    let before: HashMap<u32, Vector3<f32>> = field
        .players
        .iter()
        .filter(|p| p.id != in_id)
        .map(|p| (p.id, p.position))
        .collect();

    let tracks = play_out_the_window(&mut field, &mut context);

    // Nobody else moved a millimetre.
    for player in field.players.iter().filter(|p| p.id != in_id) {
        let was = before[&player.id];
        assert_eq!(
            player.position, was,
            "player {} moved during a substitution",
            player.id
        );
    }

    // **The exchange is sequential.** He does not leave his spot until the
    // other man is over the line, which is the whole shape of the thing.
    let walk_on = &tracks[&in_id];
    let walk_off = &tracks[&out_id];
    let steps_on = walk_on
        .iter()
        .position(|p| *p != entered_at)
        .expect("the substitute never moved");
    let is_off = walk_off
        .iter()
        .position(|p| Bench::is_over_the_line(*p))
        .expect("the man coming off never crossed the line");
    assert!(
        steps_on >= is_off,
        "the substitute set off on tick {steps_on}, before the man he was \
         replacing was off on tick {is_off}"
    );

    // Neither of them jumped. The bounds are the two speeds plus a whisker
    // for float drift; anything above them is `land` closing ground a walk
    // ran out of window to cover.
    assert!(
        longest_step(walk_on) <= 0.67,
        "the substitute jumped {:.2} units in one tick — the window ran out \
         before his walk did",
        longest_step(walk_on)
    );
    assert!(
        longest_step(walk_off) <= 0.71,
        "the man coming off jumped {:.2} units in one tick",
        longest_step(walk_off)
    );

    // He finished on the slot he was brought on for.
    let arrived = field.players.iter().find(|p| p.id == in_id).unwrap();
    assert_eq!(
        arrived.position, slot,
        "the substitute did not reach his slot"
    );

    // The window closed as soon as he got there rather than at the ceiling,
    // and stamped how long it took on the ledger.
    let spent = context.total_match_time - opened_at;
    assert!(
        spent < SubstitutionBreak::BREAK_MS + SubstitutionBreak::PORTRAIT_MS,
        "the window ran to its ceiling ({spent} ms) instead of ending on the \
         last man"
    );
    assert!(
        spent > SubstitutionBreak::PORTRAIT_MS,
        "the window closed inside the beat nobody is allowed to move in"
    );
    assert_eq!(
        context.dead_ball_until_ms, context.total_match_time,
        "play did not resume when the window closed"
    );
    let record = context
        .substitutions
        .iter()
        .find(|r| r.player_in_id == in_id)
        .expect("the change is on the ledger");
    assert_eq!(
        record.break_ms, spent,
        "the ledger did not learn how long it took"
    );
}

#[test]
fn the_two_men_cross_at_the_halfway_line() {
    let (mut field, mut context) = kickoff();
    let out_id = injure_the_furthest_man(&mut field);
    let started_at = field.get_player(out_id).unwrap().position;

    let today = context.today;
    process_substitutions(&mut field, &mut context, 3, today);
    let waiting = field
        .players
        .iter()
        .find(|p| Bench::is_over_the_line(p.position) && p.team_id == 1)
        .expect("the substitute is waiting off the pitch")
        .position;

    let tracks = play_out_the_window(&mut field, &mut context);
    let walk = &tracks[&out_id];

    // He does NOT leave by the nearest point on the line — he crosses where
    // the man replacing him is standing, which is what makes the exchange one
    // picture instead of two. He started 289 units along from there.
    let crossing = *walk
        .iter()
        .find(|p| Bench::is_over_the_line(**p))
        .expect("he must cross the line");
    assert!(
        (crossing.x - waiting.x).abs() < 20.0,
        "he crossed at x={:.0}, {:.0} units from the substitute at x={:.0}",
        crossing.x,
        (crossing.x - waiting.x).abs(),
        waiting.x
    );
    assert!(
        (crossing.x - started_at.x).abs() > 100.0,
        "the fixture no longer starts him anywhere near the halfway line, so \
         this test cannot tell the two rules apart"
    );
}

#[test]
fn nobody_moves_while_the_picture_is_on_the_men_being_replaced() {
    // ⚠ **A man cannot be shown standing on the pitch and be running off it
    // at the same time.** The replay opens a change with a beat on the back of
    // each man coming off, where he stood when the board went up; without the
    // hold he is away at `OFF` (8.75 m/s) on the very first tick and is
    // thirty-five metres from that spot before the camera reaches him.
    let (mut field, mut context) = kickoff();
    let out_id = injure_the_furthest_man(&mut field);
    let today = context.today;
    process_substitutions(&mut field, &mut context, 3, today);

    let opened_at = context.total_match_time;
    let stood_at = field
        .departed
        .iter()
        .find(|p| p.id == out_id)
        .expect("he is on the touchline list from the swap")
        .touchline
        .unwrap()
        .at;
    let change = *context
        .substitution_break
        .as_ref()
        .expect("a substitution must open a window")
        .changes()
        .iter()
        .find(|c| c.player_out_id == out_id)
        .expect("the injured man's change is in the window");
    let waited_at = field
        .get_player(change.player_in_id)
        .expect("the substitute is one of the eleven already")
        .position;

    // Right up to the last tick of the beat, neither of them has moved a
    // millimetre. The tick the beat expires ON is the tick he is released, so
    // the loop stops one short of it.
    while context.total_match_time + TICK_MS < opened_at + SubstitutionBreak::PORTRAIT_MS {
        context.total_match_time += TICK_MS;
        advance_substitution_break(&mut field, &mut context);
        let at = field
            .departed
            .iter()
            .find(|p| p.id == out_id)
            .and_then(|p| p.touchline)
            .expect("he is still being drawn")
            .at;
        assert_eq!(
            at,
            stood_at,
            "the man coming off left his spot {} ms into the beat",
            context.total_match_time - opened_at
        );
        assert_eq!(
            field.get_player(change.player_in_id).unwrap().position,
            waited_at,
            "the substitute set off during the beat"
        );
    }

    // And then it lets go: the exchange runs exactly as it always did.
    play_out_the_window(&mut field, &mut context);
    assert!(
        field
            .departed
            .iter()
            .find(|p| p.id == out_id)
            .and_then(|p| p.touchline)
            .is_none_or(|stand| Bench::is_over_the_line(stand.at)),
        "he never crossed the line once the beat was over"
    );
}

#[test]
fn the_man_who_came_off_walks_out_of_the_picture_and_then_stops_being_drawn() {
    let (mut field, mut context) = kickoff();
    let out_id = injure_the_furthest_man(&mut field);
    let today = context.today;
    process_substitutions(&mut field, &mut context, 3, today);
    let dugout = field
        .departed
        .iter()
        .find(|p| p.id == out_id)
        .expect("he is on the touchline list from the swap")
        .touchline
        .unwrap()
        .seat;
    assert!(Bench::is_over_the_line(dugout));

    // The window walks him to the line; the recorder walks him the rest of the
    // way and drops him the moment he gets there — there is no bench drawn for
    // him to stand beside.
    play_out_the_window(&mut field, &mut context);
    let mut recording = ResultMatchPositionData::new();
    let mut guard = 0;
    while field.departed.iter().any(|p| p.id == out_id) {
        guard += 1;
        FootballEngine::<840, 545>::write_match_positions(&mut field, 30 * guard, &mut recording);
        assert!(guard < 20_000, "he never got there");
    }
    let last = recording
        .get_player_position_at(out_id, 30 * guard)
        .expect("he was drawn on the way");
    assert!(
        (last.x - dugout.x).abs() < 2.0 && (last.y - dugout.y).abs() < 2.0,
        "his last frame is at {last:?}, not at {dugout:?}"
    );

    // ⚠ **And the walk has to outlast the shot watching it.** He is dropped
    // where he stands, so if he gets there while the substitution camera is
    // still pointed at the halfway line he vanishes in the middle of the
    // frame. The viewer's shot runs `break_ms` plus `LINGER_MS` (1.2 s) and
    // then ramps out over `CLOSE_TIME` (0.7 s), so it is gone about two
    // seconds after the window closes; three is the bar, and part of his walk
    // is already spent inside the window. See `Bench::DUGOUT_ALONG`, which is
    // sized for this and nothing else.
    let walked_for = 30 * guard;
    assert!(
        walked_for >= 3_000,
        "he was gone {walked_for} ms after the window closed, before the \
         camera had cut away"
    );
}

#[test]
fn a_change_waits_for_the_ball_to_be_dead() {
    let (mut field, _context) = kickoff();

    assert!(
        !Substitutions::play_is_stopped(&field),
        "a live ball is not a stoppage"
    );

    // A restart that has been AWARDED but whose ball is still rolling out is
    // not one either — the window would open on a ball in motion.
    field.ball.awaiting_restart = Some(AwaitedRestart {
        taker_id: field.players[3].id,
        spot: Vector3::new(400.0, 550.0, 0.0),
        take_from: None,
        settled: false,
        carrying: false,
        origin: PassOriginRestart::ThrowIn,
        awarded_tick: 0,
        patience_ticks: 500,
        settled_tick: None,
    });
    assert!(
        !Substitutions::play_is_stopped(&field),
        "the ball is still running out of play"
    );
    field.ball.awaiting_restart.as_mut().unwrap().settled = true;
    assert!(Substitutions::play_is_stopped(&field));
}
