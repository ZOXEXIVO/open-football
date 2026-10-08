//! Heavy rain on a muddy pitch: the ball holds up, touches go astray and
//! legs go sooner than on a clear day on a normal pitch.

#![cfg(test)]

use super::substitution_break_tests::kickoff;
use crate::r#match::ConditionContext;
use crate::r#match::defenders::states::common::{ActivityIntensity, DefenderCondition};
use crate::r#match::engine::ball::ball::{Ball, BallRoll};
use crate::r#match::engine::environment::{EnvModifiers, MatchEnvironment, Pitch, Weather};
use crate::r#match::engine::player::events::players::PlayerEventDispatcher;
use nalgebra::Vector3;

fn conditions(weather: Weather, pitch: Pitch) -> EnvModifiers {
    MatchEnvironment {
        weather,
        pitch,
        ..Default::default()
    }
    .modifiers()
}

#[test]
fn a_ground_pass_on_a_heavy_pitch_comes_up_short() {
    let clear = conditions(Weather::Clear, Pitch::Normal);
    let heavy = conditions(Weather::HeavyRain, Pitch::Muddy);
    let distance = 240.0;
    let on_grass = Ball::pass_pace(distance, &clear);
    let in_mud = Ball::pass_pace(distance, &heavy);
    assert!(in_mud < on_grass, "{in_mud} vs {on_grass}");
    assert!(BallRoll::range(in_mud) < BallRoll::range(on_grass));
}

#[test]
fn a_first_touch_goes_astray_more_often_in_the_wet() {
    let clear = conditions(Weather::Clear, Pitch::Normal);
    let heavy = conditions(Weather::HeavyRain, Pitch::Muddy);
    let receiver = 0.62;
    let dry = PlayerEventDispatcher::first_touch_loss_probability(
        (receiver + clear.touch(60)).clamp(0.0, 1.0),
        1,
    );
    let wet = PlayerEventDispatcher::first_touch_loss_probability(
        (receiver + heavy.touch(60)).clamp(0.0, 1.0),
        1,
    );
    assert!(wet > dry, "{wet} vs {dry}");
    // A cold day costs the opening fifteen minutes and nothing after.
    let cold = conditions(Weather::Cold, Pitch::Normal);
    assert!(cold.touch(5) < cold.touch(30));
}

#[test]
fn legs_go_sooner_in_the_mud() {
    let (field, _) = kickoff();
    let clear = conditions(Weather::Clear, Pitch::Normal);
    let heavy = conditions(Weather::HeavyRain, Pitch::Muddy);
    let run = |conditions: &EnvModifiers| {
        let mut player = field.players[3].clone();
        player.player_attributes.condition = 10_000;
        player.velocity = Vector3::new(0.5, 0.0, 0.0);
        for tick in 0..3_000 {
            DefenderCondition::new(ActivityIntensity::High).process(ConditionContext {
                in_state_time: tick,
                player: &mut player,
                conditions,
                match_progress: 0.5,
            });
        }
        player.player_attributes.condition
    };
    let (dry, wet) = (run(&clear), run(&heavy));
    assert!(wet < dry, "{wet} vs {dry}");
}
