use super::distances::TackleEngagement;
use crate::r#match::player::strategies::common::passing::FlankPlay;
use crate::r#match::{MatchPlayerLite, StateProcessingContext};
use nalgebra::Vector3;

/// Where a man closing the ball carrier stands: a stride off him,
/// goal-side, across the line from the carrier to the danger in our box,
/// leaning toward the carrier's run only as far as that run is a threat.
///
/// The goal centre alone is the wrong reference near the byline. From
/// the corner of the box its line runs along the goal line, where neither
/// a cross nor a cutback goes, so a closer aimed there was stood between
/// the winger and the byline and every delivery went past him. Through
/// the middle the goal, the runners and the run all lie ahead, and the
/// point is the goal-side jockey it always was.
#[derive(Debug, Clone, Copy)]
pub struct ClosingPoint {
    carrier: Vector3<f32>,
    carrier_velocity: Vector3<f32>,
    own_goal: Vector3<f32>,
    /// The carrier's top speed over the closer's — a ratio, so equally
    /// matched men read the same at every level.
    race: f32,
    field_width: f32,
    field_height: f32,
}

impl ClosingPoint {
    /// The jockey distance (~1 m), inside the charge-down window and
    /// short of [`TackleEngagement::COMMIT`], so the challenge cadence
    /// still sees a man in range.
    pub const STAND_OFF: f32 = TackleEngagement::CONTACT * 0.8;

    /// How far off the carrier the closer stands. `OF_CLOSE_STAND_OFF`
    /// overrides for titration.
    ///
    /// **Measured, not chosen** (`stats 300 14 14`, always across the
    /// line): 8u → goals 2.70, tackles 9.8 a team, corners 5.7; 16u →
    /// goals 3.42, tackles 8.3, successful pressures 12.0 → 9.6, corners
    /// 6.9. Two metres off is a press that never arrives, everywhere on
    /// the pitch, so the corners it buys are paid for in goals.
    pub fn stand_off() -> f32 {
        use std::sync::OnceLock;
        static V: OnceLock<f32> = OnceLock::new();
        *V.get_or_init(|| Self::titrated("OF_CLOSE_STAND_OFF", Self::STAND_OFF))
    }

    /// Grass ahead of the carrier at which his run is fully a threat
    /// (~10 m, twice the room the crosser reads for his own run).
    /// `OF_CLOSE_ROOM_SCALE` overrides for titration. Taking the run
    /// weight to zero (always across) moved the closer onto the line on
    /// 19% of struck deliveries against 17%, and cross blocks and corners
    /// not at all, so the run keeps its weight.
    fn room_scale() -> f32 {
        use std::sync::OnceLock;
        static V: OnceLock<f32> = OnceLock::new();
        *V.get_or_init(|| Self::titrated("OF_CLOSE_ROOM_SCALE", 80.0))
    }

    /// How fast a pace edge fills the run weight: with the grass there,
    /// a carrier 20% quicker than his closer takes it all.
    /// `OF_CLOSE_RACE_GAIN` overrides for titration.
    fn race_gain() -> f32 {
        use std::sync::OnceLock;
        static V: OnceLock<f32> = OnceLock::new();
        *V.get_or_init(|| Self::titrated("OF_CLOSE_RACE_GAIN", 2.5))
    }

    fn titrated(var: &str, default: f32) -> f32 {
        std::env::var(var)
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(default)
    }
    /// Carrier speed, u/tick, below which the run he threatens is the
    /// straight line at the goal line rather than his heading.
    const HEADING_SPEED: f32 = 0.05;

    pub fn new(
        carrier: Vector3<f32>,
        carrier_velocity: Vector3<f32>,
        own_goal: Vector3<f32>,
        carrier_speed: f32,
        closer_speed: f32,
        field_width: f32,
        field_height: f32,
    ) -> Self {
        ClosingPoint {
            carrier,
            carrier_velocity,
            own_goal,
            race: carrier_speed / closer_speed.max(1.0e-3),
            field_width,
            field_height,
        }
    }

    /// The point `ctx.player` steers for while closing `carrier`.
    pub fn for_carrier(ctx: &StateProcessingContext, carrier: &MatchPlayerLite) -> Vector3<f32> {
        let players = &ctx.tick_context.positions.players;
        let own_goal = ctx.ball().direction_to_own_goal();
        let closing = Self::new(
            carrier.position,
            players.velocity(carrier.id),
            own_goal,
            players.max_speed(carrier.id),
            players.max_speed(ctx.player.id),
            ctx.context.field_size.width as f32,
            ctx.context.field_size.height as f32,
        );
        closing.point(
            ctx.players()
                .opponents()
                .nearby_at(own_goal, FlankPlay::BOX_RANGE)
                .filter(|mate| mate.id != carrier.id)
                .map(|mate| mate.position),
        )
    }

    /// The point, given where the carrier's team-mates in our box stand.
    pub fn point(&self, box_mates: impl Iterator<Item = Vector3<f32>>) -> Vector3<f32> {
        let to_box = self.box_dir(box_mates);
        let run = self.run_dir();
        let weight = self.run_weight(run);
        let heading = ((1.0 - weight) * to_box + weight * run)
            .try_normalize(1.0e-4)
            .unwrap_or(to_box);
        self.carrier + heading * Self::stand_off()
    }

    /// Mean bearing from the carrier to our goal and to each of his
    /// team-mates in our box. Bearings rather than offsets, so a runner
    /// at the far post pulls no harder than one at the near post.
    fn box_dir(&self, box_mates: impl Iterator<Item = Vector3<f32>>) -> Vector3<f32> {
        let to_goal = self.bearing(self.own_goal);
        box_mates
            .fold(to_goal, |sum, mate| sum + self.bearing(mate))
            .try_normalize(1.0e-4)
            .unwrap_or(to_goal)
    }

    /// The run he threatens: his heading, falling back to the straight
    /// line at our goal line as he slows to a stand.
    fn run_dir(&self) -> Vector3<f32> {
        let at_goal_line = Vector3::new((self.own_goal.x - self.carrier.x).signum(), 0.0, 0.0);
        let heading = self.carrier_velocity + at_goal_line * Self::HEADING_SPEED;
        Vector3::new(heading.x, heading.y, 0.0)
            .try_normalize(1.0e-6)
            .unwrap_or(at_goal_line)
    }

    /// 0 stands the closer fully across the line to the box, 1 fully on
    /// the run: the grass the carrier has along it, times his pace edge.
    fn run_weight(&self, run: Vector3<f32>) -> f32 {
        let room = (self.grass_along(run) / Self::room_scale()).clamp(0.0, 1.0);
        let edge = (0.5 + Self::race_gain() * (self.race - 1.0)).clamp(0.0, 1.0);
        room * edge
    }

    /// Grass ahead of the carrier along `heading` before he runs out of
    /// pitch at a goal line or a touchline.
    fn grass_along(&self, heading: Vector3<f32>) -> f32 {
        let exit = |at: f32, step: f32, edge: f32| {
            if step > 1.0e-6 {
                (edge - at) / step
            } else if step < -1.0e-6 {
                -at / step
            } else {
                f32::INFINITY
            }
        };
        exit(self.carrier.x, heading.x, self.field_width)
            .min(exit(self.carrier.y, heading.y, self.field_height))
            .max(0.0)
    }

    fn bearing(&self, to: Vector3<f32>) -> Vector3<f32> {
        Vector3::new(to.x - self.carrier.x, to.y - self.carrier.y, 0.0)
            .try_normalize(1.0e-3)
            .unwrap_or_else(Vector3::zeros)
    }

    /// The `OF_CROSS_LINE_OFF` control arm: a point on the carrier's line
    /// to the goal centre, `stand_off` toward it, as every closer aimed
    /// before this rule.
    pub fn goal_side(
        ctx: &StateProcessingContext,
        carrier: Vector3<f32>,
        stand_off: f32,
    ) -> Vector3<f32> {
        let to_goal = (ctx.ball().direction_to_own_goal() - carrier)
            .try_normalize(0.01)
            .unwrap_or_else(|| Vector3::new(1.0, 0.0, 0.0));
        carrier + to_goal * stand_off
    }
}

#[cfg(test)]
mod tests {
    use super::ClosingPoint;
    use nalgebra::Vector3;

    const W: f32 = 840.0;
    const H: f32 = 545.0;

    /// Defending the right-hand goal, against a carrier attacking it.
    fn goal() -> Vector3<f32> {
        Vector3::new(W, H * 0.5, 0.0)
    }

    fn runners() -> [Vector3<f32>; 2] {
        [
            Vector3::new(790.0, 262.0, 0.0),
            Vector3::new(812.0, 290.0, 0.0),
        ]
    }

    fn closing(carrier: Vector3<f32>, velocity: Vector3<f32>, race: f32) -> ClosingPoint {
        ClosingPoint::new(carrier, velocity, goal(), 0.5 * race, 0.5, W, H)
    }

    fn heading(point: Vector3<f32>, from: Vector3<f32>) -> Vector3<f32> {
        (point - from).normalize()
    }

    #[test]
    fn at_the_byline_he_stands_across_the_line_to_the_box_whatever_the_race() {
        let carrier = Vector3::new(838.0, 455.0, 0.0);
        let running_at_it = Vector3::new(0.5, 0.0, 0.0);
        let to_box = closing(carrier, running_at_it, 1.0).box_dir(runners().into_iter());
        for race in [0.8, 1.0, 1.5] {
            let point = closing(carrier, running_at_it, race).point(runners().into_iter());
            let lean = heading(point, carrier).dot(&to_box);
            assert!(
                lean > 0.995,
                "race {race}: the closer leans off the line to the box ({lean})"
            );
            assert!(
                point.y < carrier.y,
                "race {race}: the closer is not inside the winger"
            );
        }
    }

    #[test]
    fn with_grass_ahead_a_quicker_carrier_pulls_him_toward_the_run() {
        let carrier = Vector3::new(700.0, 470.0, 0.0);
        let down_the_line = Vector3::new(0.5, 0.0, 0.0);
        let run = Vector3::new(1.0, 0.0, 0.0);
        let toward_run = |race: f32| {
            let point = closing(carrier, down_the_line, race).point(runners().into_iter());
            heading(point, carrier).dot(&run)
        };
        assert!(
            toward_run(1.2) > toward_run(0.9) + 0.05,
            "quick {} vs slow {}",
            toward_run(1.2),
            toward_run(0.9)
        );
    }

    #[test]
    fn through_the_middle_he_stands_goal_side_on_the_line_to_goal() {
        let carrier = Vector3::new(600.0, H * 0.5, 0.0);
        let at_goal = Vector3::new(0.5, 0.0, 0.0);
        let level_runners = [
            Vector3::new(780.0, H * 0.5 - 25.0, 0.0),
            Vector3::new(780.0, H * 0.5 + 25.0, 0.0),
        ];
        let point = closing(carrier, at_goal, 1.1).point(level_runners.into_iter());
        assert!(
            (point.y - carrier.y).abs() < 1.0e-3,
            "lateral shift {}",
            point.y - carrier.y
        );
        assert!(
            (point.x - carrier.x - ClosingPoint::stand_off()).abs() < 1.0e-3,
            "not a stand-off goal-side: {point:?}"
        );
    }

    #[test]
    fn with_nobody_in_the_box_he_stands_on_the_line_to_goal() {
        let carrier = Vector3::new(740.0, 120.0, 0.0);
        let standing = Vector3::zeros();
        let point = closing(carrier, standing, 0.9).point(std::iter::empty());
        let to_goal = heading(goal(), carrier);
        let run = Vector3::new(1.0, 0.0, 0.0);
        let along = heading(point, carrier);
        assert!(
            along.dot(&to_goal) > 0.9 && along.dot(&run) > 0.0,
            "{along:?}"
        );
    }

    #[test]
    fn equally_matched_men_read_the_same_at_any_speed() {
        let carrier = Vector3::new(700.0, 470.0, 0.0);
        let velocity = Vector3::new(0.4, 0.05, 0.0);
        let slow = ClosingPoint::new(carrier, velocity, goal(), 0.40, 0.40, W, H);
        let fast = ClosingPoint::new(carrier, velocity, goal(), 0.62, 0.62, W, H);
        let (a, b) = (
            slow.point(runners().into_iter()),
            fast.point(runners().into_iter()),
        );
        assert!((a - b).norm() < 1.0e-4, "{a:?} vs {b:?}");
    }

    #[test]
    fn the_point_is_a_stand_off_from_the_carrier() {
        let carrier = Vector3::new(650.0, 400.0, 0.0);
        let point =
            closing(carrier, Vector3::new(0.3, -0.2, 0.0), 1.05).point(runners().into_iter());
        assert!(((point - carrier).norm() - ClosingPoint::stand_off()).abs() < 1.0e-3);
    }
}
