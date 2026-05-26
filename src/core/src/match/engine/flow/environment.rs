/// Match environment: weather, pitch, crowd, importance.
///
/// Pure data + clamp helpers. Consumed by passing/shooting/first-touch/
/// fatigue/injury logic via `EnvModifiers`. RNG belongs at event resolution
/// — this module only returns deterministic deltas.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Weather {
    Clear,
    Rain,
    HeavyRain,
    Wind,
    Snow,
    Hot,
    Cold,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pitch {
    Perfect,
    Normal,
    Worn,
    Wet,
    Muddy,
    DryFast,
}

#[derive(Debug, Clone, Copy)]
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

/// Multiplicative/additive deltas the environment applies to specific
/// match-engine quantities. All deltas are *added* to a baseline that
/// callers normally clamp to [0,1] for probabilities or to skill-bounded
/// values for accuracy. Callers are responsible for clamping after combining.
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

impl MatchEnvironment {
    pub fn modifiers(&self) -> EnvModifiers {
        weather_modifiers(self.weather).combine(pitch_modifiers(self.pitch))
    }

    pub fn clamp_inputs(&mut self) {
        self.crowd_intensity = self.crowd_intensity.clamp(0.0, 1.0);
        self.home_advantage = self.home_advantage.clamp(0.0, 1.0);
        self.match_importance = self.match_importance.clamp(0.0, 1.0);
        self.derby_intensity = self.derby_intensity.clamp(0.0, 1.0);
    }
}

fn weather_modifiers(w: Weather) -> EnvModifiers {
    let mut m = EnvModifiers::default();
    match w {
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

fn pitch_modifiers(p: Pitch) -> EnvModifiers {
    let mut m = EnvModifiers::default();
    match p {
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
