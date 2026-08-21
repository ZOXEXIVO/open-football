use crate::r#match::engine::ball::ball::Ball;
use nalgebra::Vector3;

/// Per-tick rolling-friction decay for a ball on the ground: each tick
/// its horizontal speed is multiplied by `1 - GROUND_FRICTION`.
///
/// Derived from the real figure rather than fitted: a football on grass
/// loses roughly **15% of its speed per second**. At 100 ticks to the
/// second that is `k^100 = 0.85`, so `k = 0.85^(1/100) = 0.998375` and
/// the coefficient is 0.001625.
///
/// It was 0.006 — a 45%/s loss, ~3.7× real. That single number is why
/// `calculate_horizontal_velocity` had to aim every pass 79-157% BEYOND
/// its target (the old `overshoot` table): with the ball dying that fast,
/// a pass weighted to arrive at its man arrived at walking pace or not at
/// all, so the code compensated by hitting it 5-12 m too far. Both halves
/// are fixed together; neither works alone.
///
/// Shared so the physics and the pass-weighting can never disagree again
/// — they were separate literals in `motion.rs` and `players.rs`.
pub const GROUND_FRICTION: f32 = 0.0016;

/// Downward acceleration applied to an airborne ball, in **m/tick²**.
///
/// # The ball's vertical axis is in METRES
///
/// `x` and `y` are in game units (1u = 0.125 m); `z` is in metres. The
/// engine has always said so — `GOAL_HEIGHT` is annotated "crossbar height
/// in meters (z-axis is in meters)", and every reach threshold in the
/// engine (`PLAYER_JUMP_REACH` 3.5, `is_aerial` 2.3, the receiver ceiling
/// 2.8, the heading band 1.4-2.5) is a sane figure in metres and a
/// nonsense one in units. What did NOT honour the convention was the
/// motion: gravity and the launch velocities were written in units, so a
/// ball climbing to "4.0" was climbing four metres' worth of threshold at
/// four units' worth of speed.
///
/// This constant is the reconciliation. At 10 ms a tick,
/// `9.81 m/s² × (0.01 s)² = 9.81e-4 m/tick²`. It replaces `9.81 * 0.016`
/// (= 0.157), which was 160× too strong in metres — the ball fell like a
/// stone, so nothing could hang, so the pass solver had to fire lofted
/// balls at 85 m/s to get them anywhere, and clearances and shots were
/// each hand-fitted to that in their own units.
///
/// Consequences, all of them wanted: hang times become real (a 30 m cross
/// hangs ~2.3 s instead of ~0.5 s), lofted passes come back inside normal
/// pass speeds, and every height threshold in the engine starts meaning
/// what it says.
///
/// Every site that integrates or inverts vertical motion MUST read this
/// (or the helpers below) rather than carry its own literal — the physics,
/// the landing projection, the pass solver, the shot arc, the clearance
/// and the cross-chase all used to hold private copies of `9.81`-something
/// in three different unit systems.
pub const GRAVITY_PER_TICK: f32 = 9.81 * 0.01 * 0.01;

/// Quadratic air drag on an airborne ball: each tick its velocity loses
/// `AIR_DRAG_PER_TICK * |v| * v`.
///
/// The physics has always applied this — `-C·|v|·v / mass · 0.016` with
/// `C = 0.04` and `mass = 0.43` — but as three private literals inside
/// `update_velocity`, so **nothing that solves a trajectory could see
/// it**. Every ballistic solver in the engine (the pass loft, the
/// clearance, the landing projection) therefore inverts gravity alone and
/// assumes the ball keeps its launch speed all the way down.
///
/// It does not, and the error is not small. Integrated against this
/// constant, a ball struck to peak 20 m up travels **297u where the
/// drag-free `distance / hang_ticks` answer promises 404u** — a 26%
/// shortfall at a keeper's kicking speeds, rising past 40% for the
/// hardest-struck long balls. A goalkeeper's hoof "aimed at the halfway
/// line" from his own six-yard box lands around the edge of his own
/// centre circle.
///
/// [`Ball::launch_for_range`] inverts the real thing. Shared as a
/// constant for the same reason [`GRAVITY_PER_TICK`] is: the physics and
/// anything that inverts the physics must not be able to drift apart.
pub const AIR_DRAG_PER_TICK: f32 = 0.04 * 0.016 / 0.43;

/// Below this speed the physics stops applying drag at all — mirrored
/// here so the solver's flight and the real one agree tick for tick.
pub(in crate::r#match::engine::ball::ball) const AIR_DRAG_FLOOR: f32 = 0.1;

impl Ball {
    /// Vertical launch speed (m/tick) that peaks at `apex` metres.
    ///
    /// Apex is the natural way to ask for a trajectory: it is the one
    /// property of a kick a player actually aims at ("clip it over him",
    /// "put it on his head", "row Z"), it reads in metres so it can be
    /// sanity-checked against a human being, and it is unit-clean — the
    /// alternative, a launch angle, cannot be expressed at all when the
    /// horizontal and vertical axes carry different units.
    #[inline]
    pub fn launch_speed_for_apex(apex_metres: f32) -> f32 {
        (2.0 * GRAVITY_PER_TICK * apex_metres.max(0.0)).sqrt()
    }

    /// How long a ball launched at `vertical_speed` (m/tick) stays up, in
    /// ticks, before returning to the height it left from.
    #[inline]
    pub fn hang_ticks(vertical_speed: f32) -> f32 {
        2.0 * vertical_speed.max(0.0) / GRAVITY_PER_TICK
    }

    /// Peak height in metres of a ball launched at `vertical_speed`.
    #[inline]
    pub fn apex_for_launch(vertical_speed: f32) -> f32 {
        vertical_speed * vertical_speed / (2.0 * GRAVITY_PER_TICK)
    }

    /// Ground covered, in units, by a ball struck at `horizontal` u/tick
    /// and `vertical` m/tick from `launch_height` metres up, before it
    /// first comes back down to the turf.
    ///
    /// Integrates the same drag-then-gravity-then-step sequence
    /// `update_velocity` and `apply_movement` run, so the answer is what
    /// the ball will actually do rather than what a drag-free parabola
    /// says it will do. No spin term: the sites that need this solve for
    /// an unspun ball, and a Magnus force that curls the flight would
    /// make "the range" a function of the aim direction.
    pub fn ballistic_range(horizontal: f32, vertical: f32, launch_height: f32) -> f32 {
        /// Long enough for the highest legal ball in football (a 40 m apex
        /// hangs ~5.7 s) and a hard stop on a caller asking for nonsense.
        const MAX_TICKS: u32 = 900;
        let mut vx = horizontal.max(0.0);
        let mut vz = vertical;
        let mut x = 0.0f32;
        let mut z = launch_height.max(0.0);
        for _ in 0..MAX_TICKS {
            let speed = (vx * vx + vz * vz).sqrt();
            if speed > AIR_DRAG_FLOOR {
                let decay = AIR_DRAG_PER_TICK * speed;
                vx -= decay * vx;
                vz -= decay * vz;
            }
            vz -= GRAVITY_PER_TICK;
            x += vx;
            z += vz;
            if z <= 0.0 {
                return x;
            }
        }
        x
    }

    /// Horizontal launch speed (u/tick) that drops the ball `range` units
    /// away, given how high it is going and where it is struck from.
    ///
    /// The inverse of [`Ball::ballistic_range`], found by bisection: range
    /// is monotone in the launch speed, so this needs no derivative and no
    /// starting guess, and it is deterministic — which matters, because
    /// every trajectory in this engine has to replay identically.
    ///
    /// Saturates at [`Self::MAX_BALLISTIC_HORIZONTAL`]. A range nobody can
    /// physically kick that far comes back as the hardest strike available,
    /// which lands short — the honest answer, and the one that keeps a
    /// weak keeper's punt shorter than a strong one's instead of quietly
    /// solving him a rocket.
    pub fn launch_for_range(range: f32, vertical: f32, launch_height: f32) -> f32 {
        let target = range.max(0.0);
        let (mut lo, mut hi) = (0.0f32, Self::MAX_BALLISTIC_HORIZONTAL);
        // 14 halvings of a 4 u/tick bracket resolve to 0.0002 u/tick,
        // three orders of magnitude finer than any speed difference that
        // means anything on the pitch.
        for _ in 0..14 {
            let mid = 0.5 * (lo + hi);
            if Self::ballistic_range(mid, vertical, launch_height) < target {
                lo = mid;
            } else {
                hi = mid;
            }
        }
        0.5 * (lo + hi)
    }

    /// Upper bound of the bisection bracket — 4 u/tick is 50 m/s, harder
    /// than any human strikes a football, so the solver can never be the
    /// thing that limits a realistic kick.
    const MAX_BALLISTIC_HORIZONTAL: f32 = 4.0;

    /// Ground covered, and ticks taken, before a ball struck at
    /// `horizontal` u/tick and `vertical` m/tick from `launch_height`
    /// first comes back **down** through `arrival_height`.
    ///
    /// # Why this and not [`ballistic_range`](Self::ballistic_range)
    ///
    /// `ballistic_range` answers "where does it land", which is the right
    /// question for a clearance and the wrong one for a delivery that is
    /// supposed to meet somebody's head. A cross aimed to land at a
    /// forward's feet passes over his head a stride earlier and arrives
    /// at his boots travelling down hard; a cross aimed to ARRIVE at
    /// 2.5 m is the one he can attack. Same integration, different exit
    /// condition — see [`Self::ballistic_launch_arriving_at`].
    ///
    /// Returns `(range_units, ticks)`. A ball launched below
    /// `arrival_height` and never reaching it returns its full range.
    pub fn ballistic_arrival(
        horizontal: f32,
        vertical: f32,
        launch_height: f32,
        arrival_height: f32,
    ) -> (f32, u32) {
        /// Same bound as `ballistic_range`, and for the same reason.
        const MAX_TICKS: u32 = 900;
        let mut vx = horizontal.max(0.0);
        let mut vz = vertical;
        let mut x = 0.0f32;
        let mut z = launch_height.max(0.0);
        let floor = arrival_height.max(0.0);
        for tick in 0..MAX_TICKS {
            let speed = (vx * vx + vz * vz).sqrt();
            if speed > AIR_DRAG_FLOOR {
                let decay = AIR_DRAG_PER_TICK * speed;
                vx -= decay * vx;
                vz -= decay * vz;
            }
            vz -= GRAVITY_PER_TICK;
            x += vx;
            z += vz;
            // Descending only: a ball climbing THROUGH head height on its
            // way up has not arrived anywhere.
            if vz <= 0.0 && z <= floor {
                return (x, tick + 1);
            }
            if z <= 0.0 {
                return (x, tick + 1);
            }
        }
        (x, MAX_TICKS)
    }

    /// The horizontal speed that puts [`Self::ballistic_arrival`] at
    /// `range`. Bisection over the same bracket as
    /// [`launch_for_range`](Self::launch_for_range).
    pub fn launch_for_arrival(
        range: f32,
        vertical: f32,
        launch_height: f32,
        arrival_height: f32,
    ) -> f32 {
        let target = range.max(0.0);
        let (mut lo, mut hi) = (0.0f32, Self::MAX_BALLISTIC_HORIZONTAL);
        for _ in 0..14 {
            let mid = 0.5 * (lo + hi);
            if Self::ballistic_arrival(mid, vertical, launch_height, arrival_height).0 < target {
                lo = mid;
            } else {
                hi = mid;
            }
        }
        0.5 * (lo + hi)
    }

    /// The launch vector that carries the ball from `origin` to `target`'s
    /// x/y **arriving at `target.z` on the way down**, peaking `apex`
    /// metres above where it was struck — plus how many ticks that flight
    /// takes.
    ///
    /// This is what a resolver should use instead of writing a decided
    /// outcome's position into the ball. The contest still picks the
    /// winner at the instant the delivery is struck; the ball then
    /// actually travels to him, and the header happens when it gets
    /// there. See `FootballEngine::resolve_corner_contest`.
    pub fn ballistic_launch_arriving_at(
        origin: Vector3<f32>,
        target: Vector3<f32>,
        apex: f32,
    ) -> Option<(Vector3<f32>, u32)> {
        let to_target = Vector3::new(target.x - origin.x, target.y - origin.y, 0.0);
        let direction = to_target.try_normalize(1.0e-4)?;
        let vertical = Self::launch_speed_for_apex(apex);
        let range = to_target.norm();
        let horizontal = Self::launch_for_arrival(range, vertical, origin.z, target.z);
        let (_, ticks) = Self::ballistic_arrival(horizontal, vertical, origin.z, target.z);
        Some((
            Vector3::new(direction.x * horizontal, direction.y * horizontal, vertical),
            ticks,
        ))
    }

    /// The whole launch vector that drops the ball on `target`, peaking
    /// `apex` metres up, struck from `launch_height` metres.
    ///
    /// Returns `None` when origin and target coincide — there is no
    /// direction to launch along.
    pub fn ballistic_launch(
        origin: Vector3<f32>,
        target: Vector3<f32>,
        apex: f32,
        launch_height: f32,
    ) -> Option<Vector3<f32>> {
        let to_target = Vector3::new(target.x - origin.x, target.y - origin.y, 0.0);
        let direction = to_target.try_normalize(1.0e-4)?;
        let vertical = Self::launch_speed_for_apex(apex);
        let horizontal = Self::launch_for_range(to_target.norm(), vertical, launch_height);
        Some(Vector3::new(
            direction.x * horizontal,
            direction.y * horizontal,
            vertical,
        ))
    }
}

#[cfg(test)]
mod ballistic_solver_tests {
    use super::*;

    /// A punt-shaped ball: 20 m apex, struck from a keeper's chest.
    const PUNT_APEX: f32 = 20.0;
    const HAND_HEIGHT: f32 = 1.15;

    /// The whole reason the solver exists. `distance / hang_ticks` is the
    /// drag-free answer every ballistic site in the engine used, and the
    /// ball is not drag-free — so the ball landed a quarter short of every
    /// aim point.
    #[test]
    fn ignoring_air_drag_lands_the_ball_a_quarter_short() {
        let vertical = Ball::launch_speed_for_apex(PUNT_APEX);
        let hang = Ball::hang_ticks(vertical);
        // What the old solver would fire to "cover 404u in the hang time".
        let naive_horizontal = 404.0 / hang;
        let actually_travelled = Ball::ballistic_range(naive_horizontal, vertical, HAND_HEIGHT);
        assert!(
            actually_travelled < 404.0 * 0.80,
            "drag-free solve should fall well short, travelled {actually_travelled}u of 404u"
        );
    }

    #[test]
    fn solved_launch_lands_where_it_was_aimed() {
        let vertical = Ball::launch_speed_for_apex(PUNT_APEX);
        for range in [200.0f32, 340.0, 480.0, 540.0] {
            let horizontal = Ball::launch_for_range(range, vertical, HAND_HEIGHT);
            let landed = Ball::ballistic_range(horizontal, vertical, HAND_HEIGHT);
            assert!(
                (landed - range).abs() < 4.0,
                "aimed {range}u, landed {landed}u"
            );
        }
    }

    #[test]
    fn a_range_beyond_any_human_leg_saturates_instead_of_solving_a_rocket() {
        let vertical = Ball::launch_speed_for_apex(PUNT_APEX);
        let horizontal = Ball::launch_for_range(5_000.0, vertical, HAND_HEIGHT);
        assert!(
            horizontal <= Ball::MAX_BALLISTIC_HORIZONTAL,
            "solver must not exceed its own bracket, got {horizontal}"
        );
    }

    /// The solver and the physics are two descriptions of one flight. If
    /// they can disagree, every aim point in the engine is a guess — so
    /// fly a real `Ball` through `update_velocity` / `apply_movement` and
    /// require it to come down where the solver said it would.
    #[test]
    fn the_solver_agrees_with_the_physics_it_inverts() {
        let vertical = Ball::launch_speed_for_apex(PUNT_APEX);
        let horizontal = Ball::launch_for_range(480.0, vertical, HAND_HEIGHT);

        let mut ball = Ball::with_coord(840.0, 545.0);
        ball.position = Vector3::new(100.0, 272.0, HAND_HEIGHT);
        ball.velocity = Vector3::new(horizontal, 0.0, vertical);
        ball.spin = Vector3::zeros();

        let start_x = ball.position.x;
        let mut flown = 0.0;
        for _ in 0..900 {
            ball.update_velocity();
            ball.apply_movement();
            if ball.position.z <= 0.0 {
                flown = ball.position.x - start_x;
                break;
            }
        }
        assert!(
            (flown - 480.0).abs() < 8.0,
            "solver promised 480u, the physics flew {flown}u"
        );
    }

    /// Struck from the hands the ball gets a free fall the same kick off
    /// the deck has to buy back, so a punt out-carries a goal kick.
    #[test]
    fn a_ball_struck_from_the_hands_carries_further_than_one_off_the_floor() {
        let vertical = Ball::launch_speed_for_apex(PUNT_APEX);
        let from_hands = Ball::ballistic_range(1.6, vertical, HAND_HEIGHT);
        let off_the_deck = Ball::ballistic_range(1.6, vertical, 0.0);
        assert!(
            from_hands > off_the_deck,
            "hands {from_hands}u vs deck {off_the_deck}u"
        );
    }
}
