//! Match environment: weather, pitch, crowd, importance.
//!
//! Drawn once per fixture (`MatchEnvironment::for_fixture`), then pure
//! data: consumed by passing/shooting/first-touch/fatigue/injury logic via
//! `EnvModifiers`, which only returns deterministic deltas.

use crate::r#match::engine::flow::context::rng::MatchRng;
use crate::r#match::FixtureContext;
use chrono::Datelike;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Weather {
    #[default]
    Clear,
    Rain,
    HeavyRain,
    Wind,
    Snow,
    Hot,
    Cold,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Pitch {
    Perfect,
    #[default]
    Normal,
    Worn,
    Wet,
    Muddy,
    DryFast,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MatchEnvironment {
    pub weather: Weather,
    pub pitch: Pitch,
    /// Crowd intensity 0..1. Drives nervousness, GK communication noise,
    /// referee marginal-call bias when combined with home_bias.
    pub crowd_intensity: f32,
    /// Home advantage 0..1. Combined with crowd_intensity to scale
    /// confidence/pressure/referee bias — does NOT directly buff skill.
    pub home_advantage: f32,
    /// Match importance 0..1 (friendly 0.1, league mid-table 0.45,
    /// title decider/cup final 0.9+).
    pub match_importance: f32,
    /// Derby intensity 0..1 — extra cards/pressure on top of importance.
    pub derby_intensity: f32,
}

impl Default for MatchEnvironment {
    fn default() -> Self {
        MatchEnvironment {
            weather: Weather::Clear,
            pitch: Pitch::Normal,
            crowd_intensity: 0.55,
            home_advantage: 0.50,
            match_importance: 0.45,
            derby_intensity: 0.0,
        }
    }
}

/// The deltas the weather and the pitch put on the match, drawn once per
/// match onto `MatchContext::conditions`. Each is added to the baseline it
/// shifts (or to 1 for a rate); callers clamp after combining. Where each
/// one is read:
///   * `pass_accuracy` / `long_pass_accuracy` — the pass evaluator
///   * `first_touch`, `early_touch_penalty_first_15min` — the reception
///     miscontrol roll
///   * `cross_accuracy` — the cross's execution error
///   * `shot_accuracy_long` — the accuracy of a strike from outside the area
///   * `goalkeeper_handling`, `long_shot_rebound_chance` — the save roll
///   * `goalkeeper_claim_cross` — the keeper's claim in the cross and
///     corner contests
///   * `sliding_tackle_success`, `slide_tackle_range_units` — the slide
///     and the stretch
///   * `injury_risk` — the in-match injury rolls
///   * `fatigue_rate` / `recovery_rate` — the condition processor
///   * `high_press_intensity_cap` — the tactical press ceiling
///   * `dribble_control` / `dribble_success` — the dribble duel
///   * `ball_roll_speed` / `pass_speed` — `Ball::pass_pace`
///   * `acceleration` — the sprint ramp
#[derive(Debug, Clone, Copy, Default)]
pub struct EnvModifiers {
    pub pass_accuracy: f32,
    pub long_pass_accuracy: f32,
    pub first_touch: f32,
    pub cross_accuracy: f32,
    pub shot_accuracy_long: f32,
    pub goalkeeper_handling: f32,
    pub goalkeeper_claim_cross: f32,
    pub sliding_tackle_success: f32,
    pub injury_risk: f32,
    pub long_shot_rebound_chance: f32,
    pub fatigue_rate: f32,
    pub recovery_rate: f32,
    pub high_press_intensity_cap: f32,
    pub dribble_control: f32,
    pub dribble_success: f32,
    pub ball_roll_speed: f32,
    pub pass_speed: f32,
    pub acceleration: f32,
    pub slide_tackle_range_units: f32,
    pub early_touch_penalty_first_15min: f32,
}

impl EnvModifiers {
    /// What the conditions do to a first touch at `minute`: a wet ball
    /// skids off the boot, and on a cold afternoon nobody's touch is
    /// right for the first quarter of an hour.
    pub fn touch(&self, minute: u32) -> f32 {
        let cold_start = if minute < 15 {
            self.early_touch_penalty_first_15min
        } else {
            0.0
        };
        self.first_touch + cold_start
    }

    /// Combine two modifier sets (used to fold weather + pitch together).
    pub fn combine(mut self, other: EnvModifiers) -> EnvModifiers {
        self.pass_accuracy += other.pass_accuracy;
        self.long_pass_accuracy += other.long_pass_accuracy;
        self.first_touch += other.first_touch;
        self.cross_accuracy += other.cross_accuracy;
        self.shot_accuracy_long += other.shot_accuracy_long;
        self.goalkeeper_handling += other.goalkeeper_handling;
        self.goalkeeper_claim_cross += other.goalkeeper_claim_cross;
        self.sliding_tackle_success += other.sliding_tackle_success;
        self.injury_risk += other.injury_risk;
        self.long_shot_rebound_chance += other.long_shot_rebound_chance;
        self.fatigue_rate += other.fatigue_rate;
        self.recovery_rate += other.recovery_rate;
        self.high_press_intensity_cap += other.high_press_intensity_cap;
        self.dribble_control += other.dribble_control;
        self.dribble_success += other.dribble_success;
        self.ball_roll_speed += other.ball_roll_speed;
        self.pass_speed += other.pass_speed;
        self.acceleration += other.acceleration;
        self.slide_tackle_range_units += other.slide_tackle_range_units;
        self.early_touch_penalty_first_15min += other.early_touch_penalty_first_15min;
        self
    }
}

/// The quarter of the football year a fixture falls in, at its ground.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Season {
    Winter,
    Spring,
    Summer,
    Autumn,
}

impl Season {
    pub fn of(fixture: &FixtureContext) -> Self {
        let month = fixture.date.month();
        let month = if fixture.southern { (month + 5) % 12 + 1 } else { month };
        match month {
            12 | 1 | 2 => Season::Winter,
            3..=5 => Season::Spring,
            6..=8 => Season::Summer,
            _ => Season::Autumn,
        }
    }
}

impl MatchEnvironment {
    /// Six in ten of every crowd is the ground itself; the rest is what
    /// the match means.
    const CROWD_FROM_GATE: f32 = 0.6;

    /// The match this fixture is played in: weather for the season at
    /// its ground, a pitch to go with it, a crowd from how full the
    /// ground is and what is at stake, and the rivalry. The same fixture
    /// always draws the same.
    pub fn for_fixture(fixture: &FixtureContext) -> Self {
        let rng = MatchRng::from_seed(fixture.seed);
        let season = Season::of(fixture);
        let weather = Weather::draw(season, rng.unit_f32());
        let pitch = Pitch::draw(weather, season, rng.unit_f32());
        let mut environment = MatchEnvironment {
            weather,
            pitch,
            crowd_intensity: fixture.gate
                * (Self::CROWD_FROM_GATE + (1.0 - Self::CROWD_FROM_GATE) * fixture.importance),
            home_advantage: 0.35 + 0.30 * fixture.gate,
            match_importance: fixture.importance,
            derby_intensity: fixture.rivalry,
        };
        environment.clamp_inputs();
        environment
    }

    pub fn modifiers(&self) -> EnvModifiers {
        self.weather.modifiers().combine(self.pitch.modifiers())
    }

    pub fn clamp_inputs(&mut self) {
        self.crowd_intensity = self.crowd_intensity.clamp(0.0, 1.0);
        self.home_advantage = self.home_advantage.clamp(0.0, 1.0);
        self.match_importance = self.match_importance.clamp(0.0, 1.0);
        self.derby_intensity = self.derby_intensity.clamp(0.0, 1.0);
    }
}

impl Weather {
    /// Cumulative odds per season, in declaration order (clear, rain,
    /// heavy rain, wind, snow, hot, cold).
    const ODDS: [(Season, [f32; 7]); 4] = [
        (Season::Winter, [0.38, 0.22, 0.08, 0.10, 0.07, 0.00, 0.15]),
        (Season::Spring, [0.48, 0.22, 0.06, 0.12, 0.01, 0.06, 0.05]),
        (Season::Summer, [0.55, 0.12, 0.04, 0.07, 0.00, 0.22, 0.00]),
        (Season::Autumn, [0.42, 0.24, 0.08, 0.14, 0.01, 0.04, 0.07]),
    ];
    pub const ALL: [Weather; 7] = [
        Weather::Clear,
        Weather::Rain,
        Weather::HeavyRain,
        Weather::Wind,
        Weather::Snow,
        Weather::Hot,
        Weather::Cold,
    ];

    /// The weather a `roll` in 0..1 draws in `season`.
    pub fn draw(season: Season, roll: f32) -> Self {
        let odds = Self::ODDS
            .iter()
            .find(|(s, _)| *s == season)
            .map(|(_, odds)| odds)
            .unwrap_or(&Self::ODDS[0].1);
        let mut cumulative = 0.0;
        for (weather, chance) in Self::ALL.into_iter().zip(odds) {
            cumulative += chance;
            if roll < cumulative {
                return weather;
            }
        }
        Weather::Clear
    }

    pub fn modifiers(self) -> EnvModifiers {
        let mut m = EnvModifiers::default();
        match self {
        Weather::Clear => {}
        Weather::Rain => {
            m.pass_accuracy = -0.04;
            m.first_touch = -0.06;
            m.sliding_tackle_success = 0.04;
            m.injury_risk = 0.03;
            m.long_shot_rebound_chance = 0.05;
        }
        Weather::HeavyRain => {
            m.pass_accuracy = -0.09;
            m.first_touch = -0.11;
            m.dribble_control = -0.08;
            m.goalkeeper_handling = -0.08;
            m.injury_risk = 0.07;
            m.long_shot_rebound_chance = 0.08;
        }
        Weather::Wind => {
            m.long_pass_accuracy = -0.08;
            m.cross_accuracy = -0.10;
            m.shot_accuracy_long = -0.05;
            m.goalkeeper_claim_cross = -0.05;
        }
        Weather::Snow => {
            // Treated like a heavier-rain + cold blend.
            m.pass_accuracy = -0.07;
            m.first_touch = -0.09;
            m.dribble_control = -0.06;
            m.acceleration = -0.05;
            m.injury_risk = 0.05;
        }
        Weather::Hot => {
            m.fatigue_rate = 0.10;
            m.recovery_rate = -0.08;
            m.high_press_intensity_cap = -0.08;
        }
        Weather::Cold => {
            m.injury_risk = 0.03;
            m.early_touch_penalty_first_15min = -0.03;
        }
    }
    m
    }
}

impl Pitch {
    /// The surface a `roll` in 0..1 draws under `weather` in `season`:
    /// rain makes it wet or muddy, snow muddy or worn, heat dry and fast,
    /// and otherwise it is the groundsman's.
    pub fn draw(weather: Weather, season: Season, roll: f32) -> Self {
        let odds: &[(Pitch, f32)] = match weather {
            Weather::HeavyRain => &[(Pitch::Muddy, 0.55), (Pitch::Wet, 0.45)],
            Weather::Rain => &[(Pitch::Wet, 0.60), (Pitch::Normal, 0.30), (Pitch::Muddy, 0.10)],
            Weather::Snow => &[(Pitch::Muddy, 0.50), (Pitch::Worn, 0.50)],
            Weather::Hot => &[(Pitch::DryFast, 0.60), (Pitch::Normal, 0.30), (Pitch::Worn, 0.10)],
            _ if season == Season::Summer => &[
                (Pitch::Perfect, 0.25),
                (Pitch::Normal, 0.50),
                (Pitch::DryFast, 0.20),
                (Pitch::Worn, 0.05),
            ],
            _ => &[
                (Pitch::Perfect, 0.15),
                (Pitch::Normal, 0.60),
                (Pitch::Worn, 0.20),
                (Pitch::DryFast, 0.05),
            ],
        };
        let mut cumulative = 0.0;
        for (pitch, chance) in odds {
            cumulative += chance;
            if roll < cumulative {
                return *pitch;
            }
        }
        odds[odds.len() - 1].0
    }

    pub fn modifiers(self) -> EnvModifiers {
        let mut m = EnvModifiers::default();
        match self {
        Pitch::Perfect => {
            m.pass_accuracy = 0.02;
            m.first_touch = 0.02;
        }
        Pitch::Normal => {}
        Pitch::Worn => {
            m.pass_accuracy = -0.03;
            m.first_touch = -0.03;
            m.injury_risk = 0.02;
        }
        Pitch::Wet => {
            m.ball_roll_speed = 0.08;
            m.first_touch = -0.05;
            m.slide_tackle_range_units = 1.5;
        }
        Pitch::Muddy => {
            m.ball_roll_speed = -0.10;
            m.acceleration = -0.07;
            m.fatigue_rate = 0.08;
            m.dribble_success = -0.06;
        }
        Pitch::DryFast => {
            m.ball_roll_speed = 0.05;
            m.pass_speed = 0.04;
            // Faster ball makes first touch slightly harder.
            m.first_touch = -0.03;
        }
    }
    m
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_environment_is_neutral() {
        let env = MatchEnvironment::default();
        let m = env.modifiers();
        assert_eq!(m.pass_accuracy, 0.0);
        assert_eq!(m.first_touch, 0.0);
        assert_eq!(m.fatigue_rate, 0.0);
    }

    #[test]
    fn rain_reduces_first_touch_and_handling() {
        let env = MatchEnvironment {
            weather: Weather::HeavyRain,
            ..Default::default()
        };
        let m = env.modifiers();
        assert!(m.first_touch < 0.0);
        assert!(m.goalkeeper_handling < 0.0);
        assert!(m.injury_risk > 0.0);
    }

    #[test]
    fn wind_reduces_far_pass_and_cross_accuracy() {
        let env = MatchEnvironment {
            weather: Weather::Wind,
            ..Default::default()
        };
        let m = env.modifiers();
        assert!(m.long_pass_accuracy < 0.0);
        assert!(m.cross_accuracy < 0.0);
        // Short pass shouldn't be affected by wind alone.
        assert_eq!(m.pass_accuracy, 0.0);
    }

    #[test]
    fn muddy_pitch_slows_ball_and_dribbling() {
        let env = MatchEnvironment {
            pitch: Pitch::Muddy,
            ..Default::default()
        };
        let m = env.modifiers();
        assert!(m.ball_roll_speed < 0.0);
        assert!(m.dribble_success < 0.0);
        assert!(m.fatigue_rate > 0.0);
    }

    #[test]
    fn weather_and_pitch_combine_additively() {
        let env = MatchEnvironment {
            weather: Weather::Rain,
            pitch: Pitch::Wet,
            ..Default::default()
        };
        let m = env.modifiers();
        // Rain: first_touch -0.06; Wet pitch: first_touch -0.05.
        assert!((m.first_touch - (-0.11)).abs() < 1e-5);
        // Rain alone gives sliding_tackle_success bump; pitch adds tackle range units.
        assert!(m.sliding_tackle_success > 0.0);
        assert!(m.slide_tackle_range_units > 0.0);
    }

    #[test]
    fn clamp_inputs_keeps_unit_range() {
        let mut env = MatchEnvironment {
            crowd_intensity: 1.4,
            home_advantage: -0.2,
            match_importance: 2.0,
            derby_intensity: -1.0,
            ..Default::default()
        };
        env.clamp_inputs();
        assert_eq!(env.crowd_intensity, 1.0);
        assert_eq!(env.home_advantage, 0.0);
        assert_eq!(env.match_importance, 1.0);
        assert_eq!(env.derby_intensity, 0.0);
    }
}
