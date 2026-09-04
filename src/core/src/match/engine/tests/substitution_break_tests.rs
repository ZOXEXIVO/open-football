//! A substitution, end to end: the board going up at a dead ball, the men
//! coming on standing still for the close-up before any of them moves, the
//! twenty standing still while they do it, and the roster that changed before
//! any of it started.
//!
//! Everything asserted here is about POSITION OVER TIME, because everything
//! that was wrong before was. The roster half of a substitution has always
//! worked and has its own tests next to the scorer; what did not exist was
//! any answer to "where were those two men while it happened", and a test
//! that only checked the ledger would have passed against a body swap.
//!
//! The properties worth guarding, in order:
//!
//! 1. **The whole change is one shot, and nobody moves until it is over.**
//!    The camera stands off the middle of the row the substitutes are waiting
//!    in and pans across them, so every man of the change is released on the
//!    same tick — the end of a close-up that grew by
//!    [`SubstitutionBreak::PAN_MS`] for each of them. See
//!    [`SubstitutionBreak::portrait_ms`].
//! 2. **Neither walker teleports.** The window closes on its own beats and
//!    hands both men over wherever they have got to, so there is nothing left
//!    for a tidy-up to close and nothing is landed — which is exactly the
//!    condition the step assertions catch if it ever changes back.
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
use crate::r#match::engine::substitutions::Substitutions;
use crate::r#match::engine::touchline::{Bench, SubstitutionBreak, advance_substitution_break};
use crate::r#match::result::ResultMatchPositionData;
use crate::r#match::squad::squad::MatchSquad;
use crate::r#match::{MatchContext, MatchField, MatchPlayer, MatchPlayerCollection, MatchRng};
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
    // Pin the seed: the substitution timing model draws each side's manager
    // from it (see `SubstitutionUrgency::temperament`), and an entropy seed
    // would decide from run to run whether a discretionary change joins the
    // forced one on the same stoppage.
    context.rng = MatchRng::from_seed(0x5EED_0F17);
    (field, context)
}

/// Put one home outfielder on the floor so the forced-injury branch has to
/// take him off, and stand him on the touchline OPPOSITE the dugouts — the
/// longest walk the engine can ask anybody for.
///
/// Returns his id.
fn injure_the_furthest_man(field: &mut MatchField) -> u32 {
    injure(field, PlayerPositionType::MidfielderLeft, 120.0)
}

/// The same, for a named position and a place along the far touchline — two
/// of them is a double change, which is what the beats have to stagger.
fn injure(field: &mut MatchField, position: PlayerPositionType, along: f32) -> u32 {
    let victim = field
        .players
        .iter_mut()
        .find(|p| p.team_id == 1 && p.tactical_position.current_position == position)
        .expect("the home side fields this position");
    victim.player_attributes.condition = 1200;
    victim.position = Vector3::new(along, 541.0, 0.0);
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
    Substitutions::process(&mut field, &mut context, 3, today);

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
    // One shot for the whole change, a little longer for every man past the
    // first: a manager who is already stopping the game for an injury sends on
    // whoever else was going on, and the window holds them all. Counting the
    // staged changes rather than assuming one is what makes this an assertion
    // about the geometry instead of about how many men happened to cross the
    // line.
    //
    // ⚠ Which of them this one is no longer matters, and that is the point of
    // the 2026-09-04 change: the pan takes in the row, so every pair is let go
    // on the same tick however the pass order happened to stage them.
    let staged = context
        .substitution_break
        .as_ref()
        .expect("the window is open")
        .changes()
        .len();
    assert_eq!(
        context.dead_ball_until_ms,
        opened_at + SubstitutionBreak::window_ms(staged),
        "play must stop for exactly the shot the picture spends on the men \
         coming on, and for nothing else"
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

    // **The two of them go at once, and not before the camera is done with
    // him.** It used to be sequential — the man coming off had to be over the
    // line before the substitute moved — because the shot that opened a change
    // was of HIM, standing on the pitch. The shot is of the man coming on now,
    // so what gates the pair is his own close-up and nothing else.
    let walk_on = &tracks[&in_id];
    let walk_off = &tracks[&out_id];
    let steps_on = walk_on
        .iter()
        .position(|p| *p != entered_at)
        .expect("the substitute never moved");
    let leaves = walk_off
        .iter()
        .position(|p| *p != left_at)
        .expect("the man coming off never moved");
    assert_eq!(
        steps_on, leaves,
        "the substitute set off on tick {steps_on} and the man he was \
         replacing on tick {leaves} — they cross at the gate, so they go \
         together"
    );
    assert_eq!(
        steps_on as u64 * TICK_MS,
        SubstitutionBreak::portrait_ms(staged),
        "the pair moved somewhere other than the end of the close-up"
    );

    // Neither of them jumped. The bounds are the two speeds plus a whisker
    // for float drift.
    //
    // ⚠ **This is the assertion that guards the 2026-08-26 change.** The
    // window no longer waits for either walker, so `land` closing the ground
    // one of them had left would be a twenty-metre teleport rather than the
    // few centimetres it used to be — and `land` no longer writes a position
    // at all. If it ever does again, it shows up here first.
    assert!(
        longest_step(walk_on) <= 0.67,
        "the substitute jumped {:.2} units in one tick — something landed him",
        longest_step(walk_on)
    );
    // ⚠ **His bound has to allow for the HANDOVER.** Inside the window the man
    // coming off is walked by `advance` once per TICK; past it he is walked by
    // `settle_touchline` once per RECORDED FRAME, which is three ticks' worth
    // of ground in one go. Both are the same 8.75 m/s in the recording — and
    // the recording is all the replay ever sees, so it interpolates a smooth
    // walk either way — but this track is sampled every tick, where the second
    // of them looks like a step three times the size.
    let handover = SubstitutionBreak::OFF * (30 / TICK_MS) as f32;
    assert!(
        longest_step(walk_off) <= handover + 0.01,
        "the man coming off jumped {:.2} units in one tick, against {handover:.2} \
         for a recorder's step",
        longest_step(walk_off)
    );

    // **He is on the pitch and still going.** He is NOT on his slot: the
    // window stops two seconds into his run and his own AI takes him the rest
    // of the way once play is live, which is what a substitute jogging into
    // position looks like.
    let arriving = field.players.iter().find(|p| p.id == in_id).unwrap();
    assert!(
        !Bench::is_over_the_line(arriving.position),
        "the substitute is still out in the run-off at {:?}",
        arriving.position
    );
    let covered = (arriving.position - entered_at).norm();
    assert!(
        covered > 100.0,
        "he only covered {covered:.1} units of his run — two seconds at ON is \
         about a hundred and thirty"
    );
    assert!(
        arriving.position != slot,
        "he is standing on his slot — something landed him there"
    );

    // The window ran for exactly its beats and nothing else, and stamped how
    // long it took on the ledger.
    let spent = context.total_match_time - opened_at;
    assert_eq!(
        spent,
        SubstitutionBreak::window_ms(staged),
        "the window ran for {spent} ms instead of its own beats — it is \
         waiting for somebody again"
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
    Substitutions::process(&mut field, &mut context, 3, today);
    let waiting = field
        .players
        .iter()
        .find(|p| Bench::is_over_the_line(p.position) && p.team_id == 1)
        .expect("the substitute is waiting off the pitch")
        .position;

    let tracks = play_out_the_window(&mut field, &mut context);
    let walk = &tracks[&out_id];

    // He does NOT leave by the nearest point on the line — he heads for where
    // the man replacing him is standing, which is what makes the exchange one
    // picture instead of two. He started 289 units along from there.
    //
    // ⚠ **Measured as a HEADING rather than as a crossing**, because since
    // 2026-08-26 the window stops two seconds into the substitute's run and
    // hands him over well short of the line — see
    // `SubstitutionBreak::beats_ms`. Where he actually crosses is
    // `settle_touchline`'s business by then, and it is steering him at the
    // dugout; what the window still owes the picture is that he set off
    // ALONG the pitch toward the gate rather than straight across it.
    let ended = *walk.last().expect("he was sampled");
    assert!(
        (ended.x - started_at.x).abs() > 40.0,
        "he moved {:.0} units along the pitch — a man leaving by the nearest \
         point on the line would move none at all",
        (ended.x - started_at.x).abs()
    );
    let closed = (started_at - waiting).norm() - (ended - waiting).norm();
    assert!(
        closed > 100.0,
        "he closed only {closed:.0} units of the ground to the substitute at \
         x={:.0} — he is not walking at him",
        waiting.x
    );
}

#[test]
fn nobody_moves_while_the_picture_is_on_the_men_coming_on() {
    // ⚠ **A man cannot be shown standing at the fourth official's shoulder and
    // be running onto the pitch at the same time.** The replay opens the
    // change on the faces of the men coming on, pans along the row and comes
    // round behind them to the names across their shoulders, and only then
    // lets them go; without the hold they are away at `ON` (8.25 m/s) on the
    // very first tick and the shot is a close-up of the grass they were
    // standing on.
    let (mut field, mut context) = kickoff();
    let out_id = injure_the_furthest_man(&mut field);
    let today = context.today;
    Substitutions::process(&mut field, &mut context, 3, today);

    let opened_at = context.total_match_time;
    let stood_at = field
        .departed
        .iter()
        .find(|p| p.id == out_id)
        .expect("he is on the touchline list from the swap")
        .touchline
        .unwrap()
        .at;
    let window = context
        .substitution_break
        .as_ref()
        .expect("a substitution must open a window");
    let staged = window.changes().len();
    let change = *window
        .changes()
        .iter()
        .find(|c| c.player_out_id == out_id)
        .expect("the injured man's change is in the window");
    let waited_at = field
        .get_player(change.player_in_id)
        .expect("the substitute is one of the eleven already")
        .position;

    // Right up to the last tick of the close-up, neither of them has moved a
    // millimetre. The tick it expires ON is the tick they are released, so the
    // loop stops one short of it — and it is the close-up the whole window
    // got, pan and all, not a single man's share of one.
    while context.total_match_time + TICK_MS < opened_at + SubstitutionBreak::portrait_ms(staged) {
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

    // And then it lets go, and the pair set off.
    //
    // ⚠ Not "and he is over the line": the window stops two seconds into the
    // substitute's run now and neither man is expected to have arrived
    // anywhere — see `SubstitutionBreak::beats_ms`. What the beat owes is that
    // he was still on his spot the tick before it ended and off it afterwards.
    play_out_the_window(&mut field, &mut context);
    let ended_at = field
        .departed
        .iter()
        .find(|p| p.id == out_id)
        .and_then(|p| p.touchline)
        .map(|stand| stand.at);
    assert!(
        ended_at.is_none_or(|at| at != stood_at),
        "he never left his spot once the beat was over"
    );
}

#[test]
fn a_double_change_is_one_pan_and_the_men_go_together() {
    // ⚠ **The hold is per WINDOW, not per man**, and that is the 2026-09-04
    // reversal (maintainer: *"if several players are coming on, the camera
    // doesn't need to show each player's entrance — position the camera
    // between the players entering the field and have the camera pan over the
    // group"*). The old build gave every man a beat of his own and held the
    // rest of the row at the gate through it, which cost a triple change 16.2
    // s of match clock; one pan across the row costs 7.2 and everybody in it
    // sets off on the same tick.
    let (mut field, mut context) = kickoff();
    injure_the_furthest_man(&mut field);
    injure(&mut field, PlayerPositionType::MidfielderRight, 700.0);

    let opened_at = context.total_match_time;
    let today = context.today;
    Substitutions::process(&mut field, &mut context, 3, today);

    let window = context
        .substitution_break
        .as_ref()
        .expect("two injuries must open a window");
    assert!(
        window.changes().len() >= 2,
        "the fixture no longer stages a double change, so this test cannot \
         tell a stagger from a hold"
    );
    // The order the window holds them in no longer decides anything about
    // when they move — the pan takes in all of them — so all it is read for
    // here is where each man was standing before he set off.
    let order: Vec<(u32, Vector3<f32>)> = window
        .changes()
        .iter()
        .map(|change| {
            let waiting = field
                .get_player(change.player_in_id)
                .expect("the substitute is one of the eleven already")
                .position;
            (change.player_in_id, waiting)
        })
        .collect();

    let tracks = play_out_the_window(&mut field, &mut context);

    for (man, (in_id, waited_at)) in order.iter().enumerate() {
        let set_off = tracks[in_id]
            .iter()
            .position(|at| at != waited_at)
            .expect("a substitute never moved") as u64;
        assert_eq!(
            set_off * TICK_MS,
            SubstitutionBreak::portrait_ms(order.len()),
            "man {man} of the change set off {} ms in, on his own",
            set_off * TICK_MS
        );
    }

    // And the window outlasted the picture: it cannot close while the pan is
    // still running, however little ground anybody had to cover.
    let spent = context.total_match_time - opened_at;
    assert!(
        spent >= SubstitutionBreak::window_ms(order.len()),
        "the window closed after {spent} ms with the shot still on"
    );
}

#[test]
fn the_man_who_came_off_walks_out_of_the_picture_and_then_stops_being_drawn() {
    let (mut field, mut context) = kickoff();
    let out_id = injure_the_furthest_man(&mut field);
    let today = context.today;
    Substitutions::process(&mut field, &mut context, 3, today);
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
