//! **The goalkeeper's kick from his hands.**
//!
//! Reported as: *when a goalkeeper picks up the ball he doesn't
//! necessarily have to pass it to nearby players — he can and should kick
//! it, flicking it from his hand. This logic isn't implemented.*
//!
//! It was not. Every release a keeper had ended in `PlayerEvent::PassTo`,
//! and the one called `Kicking` scored its candidates in bands written as
//! `distance > 300.0` and annotated "300m+". 300u is 37.5 m. Its long tier
//! was additionally gated behind a distribution composite above 0.62 while
//! the population sits at 0.34, so for almost every goalkeeper in the game
//! the highest-scoring target was a midfielder 100-200u — twelve to
//! twenty-five metres — away. The keeper passed it short, every time,
//! whatever the state was called.
//!
//! Every assertion here is therefore about **where the ball ends up**, and
//! it is measured by flying it through the real physics rather than by
//! reading the launch vector. A test on the event alone passes against a
//! keeper who "punts" the ball nine metres.

#![cfg(test)]

use super::goal_celebration_tests::squad;
use crate::r#match::engine::result::Score;
use crate::r#match::goalkeepers::states::GoalkeeperKickingState;
use crate::r#match::goalkeepers::states::common::KeeperPunt;
use crate::r#match::player::events::{PlayerEvent, PlayerEventDispatcher};
use crate::r#match::result::ResultMatchPositionData;
use crate::r#match::{
    GameTickContext, MatchContext, MatchField, MatchPlayerCollection, PlayerSide,
    StateProcessingContext, StateProcessingHandler, events::Event,
};
use nalgebra::Vector3;

const KICKOFF_MS: u64 = 10 * 60 * 1000;
/// 1u = 12.5 cm.
const HALFWAY_X: f32 = 420.0;
/// Where a keeper walks the ball to before he releases it —
/// `KeeperSetPosition::RELEASE_DEPTH`, 10.6 m off his own line.
const RELEASE_X: f32 = 85.0;
const MID_Y: f32 = 272.0;

fn kickoff() -> (MatchField, MatchContext) {
    let home = squad(1, 100);
    let away = squad(2, 200);
    let players = MatchPlayerCollection::from_squads(&home, &away);
    let field = MatchField::new(840, 545, home, away);
    let mut context = MatchContext::new(&field, players, Score::new(1, 2), false, false);
    context.total_match_time = KICKOFF_MS;
    (field, context)
}

/// The home keeper, stood on his release point with the ball in his
/// gloves — the picture at the end of every `HoldingBall` spell.
///
/// `kicking` is the leg under test; everything else about him is whatever
/// the shared squad builder makes.
fn keeper_holding_the_ball(field: &mut MatchField, context: &MatchContext, kicking: f32) -> u32 {
    let keeper_id = field
        .players
        .iter_mut()
        .find(|p| {
            p.side == Some(PlayerSide::Left) && p.tactical_position.current_position.is_goalkeeper()
        })
        .map(|p| {
            p.position = Vector3::new(RELEASE_X, MID_Y, 0.0);
            p.skills.goalkeeping.kicking = kicking;
            p.id
        })
        .expect("the home side has a goalkeeper");

    let team_id = field
        .get_player(keeper_id)
        .map(|p| p.team_id)
        .expect("the keeper is on the field");

    field.ball.position = Vector3::new(RELEASE_X, MID_Y, 1.15);
    field.ball.velocity = Vector3::zeros();
    field.ball.current_owner = Some(keeper_id);
    field
        .ball
        .gather_in_hands(keeper_id, team_id, context.current_tick());
    keeper_id
}

/// Run `Kicking` for the keeper and hand back whatever it asked for.
fn kick(field: &MatchField, context: &MatchContext, keeper_id: u32) -> Vec<Event> {
    let players = field.players.clone();
    let tick_context = GameTickContext::new(field, &context.players);
    let player = players
        .iter()
        .find(|p| p.id == keeper_id)
        .expect("the keeper is on the field");
    let ctx = StateProcessingContext {
        in_state_time: 0,
        player,
        context,
        tick_context: &tick_context,
    };
    let mut result = GoalkeeperKickingState::default()
        .process(&ctx)
        .expect("a keeper holding the ball always does something with it");
    result.events.drain().collect()
}

/// Apply the events the state asked for, then fly the ball through the
/// real integrator until it comes back down. Returns where it lands.
fn land_it(field: &mut MatchField, context: &mut MatchContext, events: Vec<Event>) -> Vector3<f32> {
    let mut match_data = ResultMatchPositionData::new();
    for event in events {
        if let Event::PlayerEvent(player_event) = event {
            PlayerEventDispatcher::dispatch(player_event, field, context, &mut match_data);
        }
    }
    let mut last = field.ball.position;
    for _ in 0..900 {
        field.ball.update_velocity();
        field.ball.apply_movement();
        last = field.ball.position;
        if last.z <= 0.0 {
            break;
        }
    }
    last
}

/// The report itself. A keeper with the ball in his hands must be able to
/// put it into the opposition half — that is what a punt is for, and it is
/// the thing the engine could not do.
#[test]
fn a_keeper_with_the_ball_in_his_hands_kicks_it_into_the_opposition_half() {
    let (mut field, mut context) = kickoff();
    let keeper_id = keeper_holding_the_ball(&mut field, &context, 12.0);

    let events = kick(&field, &context, keeper_id);
    assert!(
        events
            .iter()
            .any(|e| matches!(e, Event::PlayerEvent(PlayerEvent::ClearBall(_)))),
        "a punt is a ball launched into space, not a pass at a named man"
    );

    let landing = land_it(&mut field, &mut context, events);
    assert!(
        landing.x > HALFWAY_X,
        "the punt came down on {:.0}u, short of the halfway line at {HALFWAY_X}u — \
         that is the reported short pass wearing a kick's name",
        landing.x
    );
    assert!(
        landing.x < 840.0,
        "…and it must not sail out the far end: landed on {:.0}u",
        landing.x
    );
    assert!(
        (0.0..=545.0).contains(&landing.y),
        "a punt aimed at a channel should not be aimed off the pitch: y {:.0}u",
        landing.y
    );
}

/// A punt is his LEG. The whole model rests on how far the ball goes being
/// decided by the man rather than by where the nearest team-mate happens
/// to be standing, so the two ends of the attribute must be visibly
/// different kicks.
///
/// Averaged over draws, deliberately. A single punt carries the keeper's
/// own execution error — ±11% of the range for an ordinary distributor —
/// and the `MatchRng` stream is thread-seeded, so a one-kick comparison
/// passes on its own and fails inside the suite depending on which thread
/// picked it up. Twenty draws cut the spread by √20 and leave the skill
/// signal, which is the thing under test.
#[test]
fn a_better_striker_of_the_ball_punts_it_further() {
    const DRAWS: usize = 20;

    let mean_carry = |kicking: f32| {
        let (mut field, context) = kickoff();
        let keeper_id = keeper_holding_the_ball(&mut field, &context, kicking);
        let players = field.players.clone();
        let tick_context = GameTickContext::new(&field, &context.players);
        let player = players
            .iter()
            .find(|p| p.id == keeper_id)
            .expect("the keeper is on the field");
        let ctx = StateProcessingContext {
            in_state_time: 0,
            player,
            context: &context,
            tick_context: &tick_context,
        };
        let total: f32 = (0..DRAWS)
            .map(|_| {
                let plan = KeeperPunt::plan(&ctx).expect("he is stood on his own release point");
                (plan.target - player.position).norm()
            })
            .sum();
        total / DRAWS as f32
    };

    let weak = mean_carry(3.0);
    let strong = mean_carry(19.0);
    assert!(
        strong > weak + 80.0,
        "a 19/20 kicker averaged {strong:.0}u against a 3/20 kicker's {weak:.0}u — \
         at least 10 m should separate them"
    );
}

/// **A punt is not a defensive clearance.**
///
/// The ball was dead in his gloves; there was nothing to clear. It matters
/// beyond bookkeeping — `clearances` is a rating input, so a keeper who
/// punts ten times a match would saturate his defensive term every week
/// regardless of how he played.
#[test]
fn punting_it_is_not_credited_as_a_clearance() {
    let (mut field, mut context) = kickoff();
    let keeper_id = keeper_holding_the_ball(&mut field, &context, 12.0);
    let events = kick(&field, &context, keeper_id);
    land_it(&mut field, &mut context, events);

    let keeper = field
        .get_player(keeper_id)
        .expect("the keeper is on the field");
    assert_eq!(
        keeper.statistics.clearances, 0,
        "distribution out of the gloves must not be counted as defensive work"
    );
}

/// The ball has to actually leave his hands: `held_in_hands` is what bars
/// every claim, tackle and interception in the engine, so a punt that left
/// the flag up would be a ball nobody on the pitch could contest.
#[test]
fn the_ball_leaves_his_gloves_and_arms_the_second_touch_bar() {
    let (mut field, mut context) = kickoff();
    let keeper_id = keeper_holding_the_ball(&mut field, &context, 12.0);
    assert!(field.ball.held_in_hands, "he starts with it in his gloves");

    let events = kick(&field, &context, keeper_id);
    land_it(&mut field, &mut context, events);

    assert!(!field.ball.held_in_hands, "the punt empties the gloves");
    assert!(
        field.ball.current_owner.is_none(),
        "and it is a loose ball once it is gone"
    );
    assert!(
        field.ball.awaiting_touch_after_release_by(keeper_id),
        "a ball put back into play from the hands arms Law 12's second-touch bar"
    );
    assert!(
        field.ball.pass_target_player_id.is_none(),
        "nobody is the intended receiver of a punt — that privilege is what \
         makes a pass a pass, and it is exactly what a contested drop must not have"
    );
}
