//! Professional (tactical) foul decisions: whether a defender cynically
//! stops a counter, and whether the resulting card is yellow or red.
//!
//! Pure helpers; no engine state mutation. Returned values are
//! probabilities the caller folds into the existing referee logic.

use nalgebra::Vector3;

/// Estimate of the post-foul attacking threat. Caller computes from the
/// match state; high values mean the foul prevented a likely chance.
#[derive(Debug, Clone, Copy)]
pub struct CounterAttackThreat {
    /// True if the carrier had a clear lane toward the goal.
    pub lane_open: bool,
    /// Defenders behind the ball, from the foul location.
    pub defenders_behind_ball: u8,
    /// Distance from foul spot to the goal being attacked, in field units.
    pub distance_to_goal_units: f32,
    /// Closest defender within fouling range (field units).
    pub fouler_distance_units: f32,
}

impl CounterAttackThreat {
    /// A defender this far either side of the carrier's line to goal is
    /// in the way. 24u = 3 m.
    const LANE_HALF_WIDTH: f32 = 24.0;

    /// The threat read off the pitch: the carrier running at `goal`, the
    /// man who might bring him down at `fouler`, and every other player of
    /// the defending side, keeper included.
    pub fn from_positions(
        carrier: Vector3<f32>,
        goal: Vector3<f32>,
        fouler: Vector3<f32>,
        defenders: impl Iterator<Item = (Vector3<f32>, bool)>,
    ) -> Self {
        let to_goal = Vector3::new(goal.x - carrier.x, goal.y - carrier.y, 0.0);
        let distance_to_goal_units = to_goal.norm();
        let dir = if distance_to_goal_units > 0.0 {
            to_goal / distance_to_goal_units
        } else {
            to_goal
        };
        let mut behind = 0u8;
        let mut lane_open = true;
        for (at, is_keeper) in defenders {
            let rel = Vector3::new(at.x - carrier.x, at.y - carrier.y, 0.0);
            let along = rel.dot(&dir);
            if along <= 0.0 || along >= distance_to_goal_units {
                continue;
            }
            behind = behind.saturating_add(1);
            let across = (rel - dir * along).norm();
            if !is_keeper && across < Self::LANE_HALF_WIDTH {
                lane_open = false;
            }
        }
        CounterAttackThreat {
            lane_open,
            defenders_behind_ball: behind,
            distance_to_goal_units,
            fouler_distance_units: (fouler - carrier).norm(),
        }
    }

    pub fn is_dogso_zone(&self) -> bool {
        self.distance_to_goal_units < 260.0
    }
}

/// Card outcome for a professional foul.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProfessionalFoulCard {
    Yellow,
    Red,
}

/// Professional (tactical) foul decisions, grouped as associated functions.
pub struct ProfessionalFoul;

impl ProfessionalFoul {
    /// Probability the defender commits a tactical (professional) foul to
    /// stop a counter. Returns 0 outside the trigger conditions.
    ///
    /// Trigger: `lane_open && defenders_behind_ball < 2 && distance < 260u
    /// && fouler_distance < 8u`.
    ///
    /// Probability:
    ///   aggression*0.14 + decisions*0.08 + team_desperation*0.10
    ///   + match_minute_late*0.06 - sportsmanship*0.10 - already_yellow*0.12
    pub fn commit_prob(
        threat: CounterAttackThreat,
        aggression_0_20: f32,
        decisions_0_20: f32,
        sportsmanship_0_20: f32,
        team_desperation_0_1: f32,
        match_minute: u32,
        already_yellow: bool,
    ) -> f32 {
        if !(threat.lane_open
            && threat.defenders_behind_ball < 2
            && threat.distance_to_goal_units < 260.0
            && threat.fouler_distance_units < 8.0)
        {
            return 0.0;
        }
        let n = |x: f32| (x / 20.0).clamp(0.0, 1.0);
        let late = if match_minute >= 75 {
            1.0
        } else if match_minute >= 60 {
            0.5
        } else {
            0.0
        };
        let yellow_bias = if already_yellow { 0.12 } else { 0.0 };

        let raw = n(aggression_0_20) * 0.14
            + n(decisions_0_20) * 0.08
            + team_desperation_0_1.clamp(0.0, 1.0) * 0.10
            + late * 0.06
            - n(sportsmanship_0_20) * 0.10
            - yellow_bias;
        raw.clamp(0.0, 0.65)
    }

    /// Decide whether the professional foul is yellow or red — DOGSO close to
    /// goal can flip a small fraction to red. Returned as the *probability*
    /// of red; caller rolls.
    pub fn red_card_prob(threat: CounterAttackThreat) -> f32 {
        if !threat.is_dogso_zone() {
            return 0.03;
        }
        // Closer to goal + clear lane + no covering defender → DOGSO red.
        if threat.lane_open && threat.defenders_behind_ball == 0 {
            if threat.distance_to_goal_units < 100.0 {
                return 0.12;
            }
            return 0.07;
        }
        0.04
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn open_threat() -> CounterAttackThreat {
        CounterAttackThreat {
            lane_open: true,
            defenders_behind_ball: 1,
            distance_to_goal_units: 220.0,
            fouler_distance_units: 5.0,
        }
    }

    #[test]
    fn a_runner_with_only_the_keeper_to_beat_has_an_open_lane() {
        let carrier = Vector3::new(150.0, 272.0, 0.0);
        let goal = Vector3::new(0.0, 272.0, 0.0);
        let fouler = Vector3::new(156.0, 270.0, 0.0);
        let keeper = (Vector3::new(10.0, 272.0, 0.0), true);
        let beaten = (Vector3::new(200.0, 260.0, 0.0), false);
        let threat =
            CounterAttackThreat::from_positions(carrier, goal, fouler, [keeper, beaten].into_iter());
        assert!(threat.lane_open);
        assert_eq!(threat.defenders_behind_ball, 1);
        assert!(threat.is_dogso_zone());
    }

    #[test]
    fn a_covering_defender_on_the_line_closes_the_lane() {
        let carrier = Vector3::new(150.0, 272.0, 0.0);
        let goal = Vector3::new(0.0, 272.0, 0.0);
        let fouler = Vector3::new(156.0, 270.0, 0.0);
        let keeper = (Vector3::new(10.0, 272.0, 0.0), true);
        let cover = (Vector3::new(80.0, 280.0, 0.0), false);
        let threat =
            CounterAttackThreat::from_positions(carrier, goal, fouler, [keeper, cover].into_iter());
        assert!(!threat.lane_open);
        assert_eq!(threat.defenders_behind_ball, 2);
    }

    #[test]
    fn pro_foul_zero_outside_trigger() {
        let mut t = open_threat();
        t.lane_open = false;
        let p = ProfessionalFoul::commit_prob(t, 14.0, 12.0, 12.0, 0.6, 80, false);
        assert_eq!(p, 0.0);
    }

    #[test]
    fn pro_foul_late_match_more_likely() {
        let early = ProfessionalFoul::commit_prob(open_threat(), 14.0, 12.0, 12.0, 0.6, 30, false);
        let late = ProfessionalFoul::commit_prob(open_threat(), 14.0, 12.0, 12.0, 0.6, 85, false);
        assert!(late > early);
    }

    #[test]
    fn pro_foul_already_yellow_reduces_prob() {
        let no_card =
            ProfessionalFoul::commit_prob(open_threat(), 14.0, 12.0, 12.0, 0.5, 80, false);
        let on_yellow =
            ProfessionalFoul::commit_prob(open_threat(), 14.0, 12.0, 12.0, 0.5, 80, true);
        assert!(on_yellow < no_card);
    }

    #[test]
    fn dogso_close_to_goal_red_prob_higher() {
        let mut t = open_threat();
        t.distance_to_goal_units = 80.0;
        t.defenders_behind_ball = 0;
        let close = ProfessionalFoul::red_card_prob(t);

        let mut far = open_threat();
        far.distance_to_goal_units = 250.0;
        let far_p = ProfessionalFoul::red_card_prob(far);

        assert!(close > far_p);
    }
}
