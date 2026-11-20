//! How far ahead of or behind his age a player matures.

use crate::PlayerPositionType;
use crate::club::player::maturation::SkillMaturation;
use crate::utils::random::engine::RandomEngine;

/// An offset in years added to a player's age on every curve his
/// development reads. It fades to nothing between 22 and 26: timing changes
/// when a player arrives, not where.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct MaturationTiming {
    years: f32,
}

impl MaturationTiming {
    pub const BOUND: f32 = 1.5;
    const FADE_FROM: f32 = 22.0;
    const FADE_TO: f32 = 26.0;
    /// Spread of a drawn timing: two players in three mature within half a
    /// year of their age.
    const DRAWN_SD: f32 = 0.5;

    pub fn new(years: f32) -> Self {
        MaturationTiming {
            years: years.clamp(-Self::BOUND, Self::BOUND),
        }
    }

    /// A timing drawn for a player created during play.
    pub fn draw() -> Self {
        let u1 = RandomEngine::gen_f32().max(1e-6);
        let u2 = RandomEngine::gen_f32();
        let normal = (-2.0 * u1.ln()).sqrt() * (std::f32::consts::TAU * u2).cos();
        Self::new(normal * Self::DRAWN_SD)
    }

    /// The timing that puts a recorded ability share on the player's own
    /// curve: the age at which his position's families, grown without
    /// football, would hold `ratio` of his potential, within the bound. A share above
    /// the curve's furthest reach reads as the earliest timing, one below
    /// its nearest as the latest.
    pub fn of_record(age: f32, ratio: f32, position: PlayerPositionType) -> Self {
        let fade = Self::fade(age);
        if fade <= 0.0 {
            return Self::default();
        }
        let share =
            |years: f32| SkillMaturation::unplayed_ability_share(age + years * fade, position);
        let (mut lo, mut hi) = (-Self::BOUND, Self::BOUND);
        if ratio >= share(hi) {
            return Self::new(hi);
        }
        if ratio <= share(lo) {
            return Self::new(lo);
        }
        for _ in 0..20 {
            let mid = (lo + hi) * 0.5;
            if share(mid) < ratio {
                lo = mid;
            } else {
                hi = mid;
            }
        }
        Self::new((lo + hi) * 0.5)
    }

    pub fn years(&self) -> f32 {
        self.years
    }

    /// His age as his curves read it.
    pub fn age(&self, age: f32) -> f32 {
        age + self.years * Self::fade(age)
    }

    fn fade(age: f32) -> f32 {
        ((Self::FADE_TO - age) / (Self::FADE_TO - Self::FADE_FROM)).clamp(0.0, 1.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timing_stays_within_a_year_and_a_half() {
        assert_eq!(MaturationTiming::new(4.0).years(), 1.5);
        assert_eq!(MaturationTiming::new(-4.0).years(), -1.5);
        for _ in 0..2000 {
            assert!(MaturationTiming::draw().years().abs() <= 1.5);
        }
    }

    #[test]
    fn timing_fades_from_22_to_26() {
        let early = MaturationTiming::new(1.0);
        assert_eq!(early.age(18.0), 19.0);
        assert_eq!(early.age(22.0), 23.0);
        assert!((early.age(24.0) - 24.5).abs() < 1e-5);
        assert_eq!(early.age(26.0), 26.0);
        assert_eq!(early.age(30.0), 30.0);
    }

    #[test]
    fn a_recorded_prodigy_is_an_early_developer() {
        let timing = MaturationTiming::of_record(18.5, 0.75, PlayerPositionType::MidfielderCenter);
        assert!(
            timing.years() > 0.0,
            "0.75 at 18 read as {}",
            timing.years()
        );
    }

    #[test]
    fn a_record_on_the_curve_reads_as_on_time() {
        let position = PlayerPositionType::DefenderCenter;
        let on_curve = SkillMaturation::unplayed_ability_share(19.0, position);
        let timing = MaturationTiming::of_record(19.0, on_curve, position);
        assert!(timing.years().abs() < 0.01, "{}", timing.years());
    }
}
