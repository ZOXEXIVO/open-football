use crate::club::player::skills::GoalkeeperSpeedContext;
use crate::r#match::engine::ball::ball::{AerialReach, GRAVITY_PER_TICK};
use crate::r#match::engine::player::strategies::common::{
    ActivityIntensityConfig, ConditionProcessor, GOALKEEPER_JADEDNESS_INCREMENT,
    GOALKEEPER_JADEDNESS_INTERVAL, GOALKEEPER_LOW_CONDITION_THRESHOLD,
};
use crate::r#match::player::strategies::players::ops::goalkeeper_skill::GoalkeeperSkillProfile;
use crate::r#match::{PlayerSide, StateProcessingContext};
use nalgebra::Vector3;

/// Goalkeeper-specific activity intensity configuration
pub struct GoalkeeperConfig;

impl ActivityIntensityConfig for GoalkeeperConfig {
    fn very_high_fatigue() -> f32 {
        7.0 // Lower than outfield players - explosive but infrequent
    }

    fn high_fatigue() -> f32 {
        4.5 // Lower than outfield players
    }

    fn moderate_fatigue() -> f32 {
        2.5
    }

    fn low_fatigue() -> f32 {
        0.8
    }

    fn recovery_rate() -> f32 {
        -4.0 // Better recovery than outfield players
    }

    fn sprint_multiplier() -> f32 {
        1.3 // Sprinting (less demanding than outfield players)
    }

    fn jogging_multiplier() -> f32 {
        0.5
    }

    fn walking_multiplier() -> f32 {
        0.2
    }

    fn low_condition_threshold() -> i16 {
        GOALKEEPER_LOW_CONDITION_THRESHOLD
    }

    fn jadedness_interval() -> u64 {
        GOALKEEPER_JADEDNESS_INTERVAL
    }

    fn jadedness_increment() -> i16 {
        GOALKEEPER_JADEDNESS_INCREMENT
    }
}

/// Where a keeper sets his feet to face a shot, and where he takes the
/// ball once he has gathered it.
///
/// Both save states used to steer to `(own_goal.x, goal_line_y)` — the
/// goal LINE itself. No keeper faces a shot standing on his line; he sets
/// a yard or so off it so he can attack the ball rather than carry it
/// back over the line behind him. It also had a very visible
/// consequence: the physics save snaps the ball to the keeper's
/// position, so every catch parked the ball at **x ≈ 839 of 840** at
/// glove height — hanging inside the goal frame, on the same spot, about
/// 160 times a match. Measured from a replay dump: the ball sat at
/// (839.3, 223.6, 1.20) without moving for 3.5 seconds.
///
/// UNITS: 1 unit = 0.125 m.
/// Where the keeper stands during OPEN PLAY — as distinct from
/// [`KeeperSetPosition`], which is where he sets himself to face a strike.
///
/// # Why this exists
///
/// Two states owned a keeper's resting position — `Standing` and
/// `Walking` — and each had its own copy of the model with different
/// constants, so the same keeper wanted to be in two different places
/// depending on which state he happened to be in. Both copies were also
/// wrong in the same way: their whole depth range came to about 1-4 m, so
/// a keeper never left his line. He stood motionless for **88% of the
/// match**, which is the "only the GK stays in the goal" report.
///
/// # The model
///
/// Depth is set by **where the ball is** and **how high his own defence
/// is**; lateral position by **the angle** — he stands on the line from
/// the middle of his goal to the ball, which is what narrowing the angle
/// physically means, and which gives him his side-to-side movement for
/// free.
///
/// Real reference points, which the constants reproduce:
///   * ball in the opponent's half behind a high line — 20-28 m out,
///     effectively a sweeper;
///   * ball in midfield — 12-18 m;
///   * ball entering our final third — 6-10 m;
///   * ball in the box — 2-5 m, on the angle.
pub struct KeeperRestPosition;

impl KeeperRestPosition {
    /// Off his line with the ball at the far end and the line high.
    /// 220u = 27.5 m.
    const SWEEP_DEPTH: f32 = 220.0;
    /// …and with the ball on top of him. 18u = 2.25 m.
    const NEAR_DEPTH: f32 = 18.0;
    /// He never closes to within this of his own back line — the space
    /// in behind is his to cover, not a free run for a striker.
    const BEHIND_LINE_GAP: f32 = 150.0;
    /// Inside this he is where he wants to be and stands set. A keeper
    /// standing still, set, is a real and common thing; without a
    /// deadzone he chases a target that moves every tick with the ball
    /// and covers more ground than a midfielder.
    ///
    /// 26u = 3.25 m. A keeper repositions in STEPS — he shuffles, then
    /// sets, then shuffles again — rather than gliding continuously after
    /// the ball. At 10u he tracked it every tick and covered 10.2 km
    /// against a real ~5 km.
    pub const SET_DEADZONE: f32 = 26.0;

    /// The spot, for a keeper defending `own_goal`, given where the ball
    /// is and where his side's defensive line is sitting.
    pub fn point(
        own_goal: Vector3<f32>,
        ball: Vector3<f32>,
        side: PlayerSide,
        defensive_line_x: f32,
        field_width: f32,
        command_of_area: f32,
        positioning: f32,
    ) -> Vector3<f32> {
        let to_ball = ball - own_goal;
        let ball_distance = to_ball.magnitude();

        // Depth rises with the ball's distance. SQUARED, so he drops onto
        // his line quickly as the ball comes into the final third and only
        // drifts back up slowly once it has gone — the asymmetry a keeper
        // actually plays with: getting back is urgent, pushing up is not.
        let far = (ball_distance / field_width).clamp(0.0, 1.0);
        let mut depth = Self::NEAR_DEPTH + (Self::SWEEP_DEPTH - Self::NEAR_DEPTH) * far * far;
        // Temperament: a commanding keeper sweeps, a line-keeper does not.
        depth *= 0.65 + command_of_area.clamp(0.0, 1.0) * 0.55;

        // Tethered to his own defensive line. A keeper 30 m off his line
        // behind a deep block is lost, not brave; behind a high line, one
        // on his goal line leaves 40 m of grass nobody covers.
        // `defensive_line_x` is the same reference the back four uses, so
        // keeper and defence agree where the space in behind is.
        let line_progress = side.attacking_progress_x(defensive_line_x, field_width);
        let line_depth =
            (line_progress * field_width - Self::BEHIND_LINE_GAP).max(Self::NEAR_DEPTH);
        depth = depth.min(line_depth).min(field_width * 0.5 - 20.0);

        // On the goal-to-ball line. A better-positioned keeper sits truer
        // on the angle; a poorer one hedges centrally and can be beaten at
        // his near post.
        let toward_ball = if ball_distance > 1.0 {
            to_ball / ball_distance
        } else {
            Vector3::new(side.forward_dir_x(), 0.0, 0.0)
        };
        let on_angle = own_goal + toward_ball * depth;
        let fidelity = 0.55 + positioning.clamp(0.0, 1.0) * 0.45;
        Vector3::new(
            on_angle.x,
            own_goal.y + (on_angle.y - own_goal.y) * fidelity,
            0.0,
        )
    }

    /// How fast he travels to it — set by how near the BALL is, not by how
    /// far he has to go. That is the difference between a keeper strolling
    /// to the edge of his box while play is at the other end and the same
    /// keeper sprinting to the same spot with a striker running through.
    pub fn pace(ball_distance: f32, field_width: f32) -> f32 {
        let urgency = (1.0 - ball_distance / (field_width * 0.55)).clamp(0.0, 1.0);
        0.18 + urgency * urgency * 1.12
    }
}

pub struct KeeperSetPosition;

impl KeeperSetPosition {
    /// Set position for a point-blank strike — 1.25 m off the line.
    /// Tight, because there is no time to come and meet it.
    const MIN_DEPTH: f32 = 10.0;
    /// Set position against a long-range effort — 3.5 m off, where a
    /// keeper stands when he can see it coming.
    const MAX_DEPTH: f32 = 28.0;
    /// Shot range over which the depth opens up (~11 m).
    const DEPTH_RANGE: f32 = 90.0;
    /// Where he takes the ball to release it — 10.6 m out, around the
    /// edge of the six-yard box.
    const RELEASE_DEPTH: f32 = 85.0;

    /// `+1` when the goal being defended is the left one, so "out of the
    /// goal" is `+x`; `-1` for the right-hand goal.
    fn into_pitch(own_goal: Vector3<f32>, field_width: f32) -> f32 {
        if own_goal.x <= field_width * 0.5 {
            1.0
        } else {
            -1.0
        }
    }

    /// The spot to defend a strike from `shot_distance` away, guarding
    /// `goal_line_y`.
    pub fn set_point(
        own_goal: Vector3<f32>,
        goal_line_y: f32,
        shot_distance: f32,
        field_width: f32,
    ) -> Vector3<f32> {
        let opened = (shot_distance / Self::DEPTH_RANGE).clamp(0.0, 1.0);
        let depth = Self::MIN_DEPTH + (Self::MAX_DEPTH - Self::MIN_DEPTH) * opened;
        Vector3::new(
            own_goal.x + Self::into_pitch(own_goal, field_width) * depth,
            goal_line_y,
            0.0,
        )
    }

    /// Where a keeper walks the ball once it is in his gloves. Real
    /// keepers get up and carry it out to the edge of their area to
    /// release it; this engine's stood exactly where it caught it for up
    /// to five and a half seconds.
    ///
    /// He walks OUT from his line while holding most of his lateral
    /// position, drifting gently back towards the middle. Keeping it
    /// continuous in where he gathered the ball matters: an absolute
    /// target would just move the single point everyone converges on
    /// further off the line rather than removing it.
    pub fn release_point(
        own_goal: Vector3<f32>,
        keeper: Vector3<f32>,
        field_width: f32,
    ) -> Vector3<f32> {
        Vector3::new(
            own_goal.x + Self::into_pitch(own_goal, field_width) * Self::RELEASE_DEPTH,
            keeper.y * 0.65 + own_goal.y * 0.35,
            0.0,
        )
    }
}

/// Whether a loose ball is the keeper's to claim, or somebody else's.
///
/// Every claim gate asked only about the BALL — is it loose, is it near,
/// is it on our side — and never about the people standing round it. In
/// a crowded six-yard box that is always true of something, because
/// possession there flickers constantly: a touch, a challenge or an
/// owner drifting past the tracking cutoff leaves the ball momentarily
/// unowned, and the keeper is by definition the man standing next to it.
/// So he collected it, distributed it, an attacker took it two metres
/// away, and the next stray touch handed it straight back to him — the
/// ball ping-ponging between the keeper and the players in front of him
/// on one spot, which is the reported symptom. Measured: 47.7 gathers a
/// match against a real 8-12.
///
/// A keeper does not grab a ball an opponent is on. He comes for the one
/// he can get to first, and lets his defenders deal with the rest. That
/// is the question this asks, and it is the same question the attacker
/// is implicitly asking, so the two cannot both claim it.
pub struct KeeperBallClaim;

impl KeeperBallClaim {
    /// How much nearer the keeper is allowed to let an opponent be while
    /// still going for a ball ON THE FLOOR. He has hands and a dive, so he
    /// wins one he is marginally further from — but only marginally.
    /// 8u = 1 m.
    const HANDS_ADVANTAGE: f32 = 8.0;

    /// …and how much that edge grows as the ball climbs. Above head height
    /// nobody else on the pitch can put a hand on it, which is the whole
    /// reason a keeper comes for a cross through four people — so the
    /// advantage there is not "marginal", it is decisive. 26u = 3.25 m at
    /// full height, on top of the ground advantage.
    ///
    /// Without this the test was purely horizontal, so the same 1 m
    /// allowance decided a ball rolling at his feet and one dropping over
    /// a crowd at 2.5 m. That is what made [`KeeperAerialClaim`] send him
    /// for a cross he had correctly judged was his, only for the catch
    /// gate in `GoalkeeperCatchingState` to hand it back to whichever
    /// attacker happened to be standing a metre nearer.
    const AERIAL_ADVANTAGE: f32 = 26.0;

    /// Is this keeper favourite for the loose ball in front of him?
    pub fn is_favourite(ctx: &StateProcessingContext) -> bool {
        let ball = ctx.tick_context.positions.ball.position;
        let aerial = (ball.z / AerialReach::STANDING).clamp(0.0, 1.0);
        let edge = Self::HANDS_ADVANTAGE + Self::AERIAL_ADVANTAGE * aerial;
        let mine = (ball - ctx.player.position).magnitude() - edge;
        !ctx.players()
            .opponents()
            .all()
            .any(|opp| (ball - opp.position).magnitude() < mine)
    }
}

/// A high ball the keeper has decided to come and take.
#[derive(Debug, Clone, Copy)]
pub struct AerialClaim {
    /// Where he has to be to meet it.
    pub meeting_point: Vector3<f32>,
    /// Height of the ball there, in metres.
    pub height: f32,
    /// Ticks until it gets there.
    pub ticks: f32,
    /// 0..1 — how much traffic he has to come through.
    pub traffic: f32,
    /// True when he can take it with both feet on the floor; false means
    /// the claim needs a leap.
    pub standing: bool,
}

impl AerialClaim {
    /// A keeper takes off to meet the ball, he does not jump and wait for
    /// it. His hang time for the apexes `PlayerMatchState::leap_apex` asks
    /// for is ~55-75 ticks, so leaving the ground with the ball ~30 ticks
    /// away puts the top of the leap and the ball in the same place.
    pub const TAKEOFF_TICKS: f32 = 30.0;

    /// Close enough to play it from where he is standing (1.5 m).
    pub const CONTACT_RANGE: f32 = 12.0;

    /// Is he at the meeting point with the ball arriving now?
    pub fn at_contact(&self, keeper: Vector3<f32>) -> bool {
        self.ticks <= Self::TAKEOFF_TICKS
            && (self.meeting_point - keeper).magnitude() <= Self::CONTACT_RANGE
    }
}

/// Whether a ball in the air over the keeper's own box is HIS.
///
/// # Why this exists
///
/// `GoalkeeperState::Jumping` had exactly one inbound transition in the
/// whole engine — from `Punching`, on the branch that fires when the ball
/// is already out of punching range. So a keeper never once left the
/// ground to attack a cross, a corner or a chipped through-ball, and
/// `aerial_reach`, `command_of_area`, `punching`, `jumping` and `bravery`
/// were decorative: the only thing that ever read them was the post-hoc
/// flap contest in `gk_claim.rs`, which runs *after* the ball has already
/// arrived at a keeper who happened to be standing there.
///
/// From the stands that is most of what "he isn't in the game" means. A
/// real keeper's most visible act of authority is coming through a crowd
/// to take a ball off somebody's head, and this engine had no mechanism
/// for it at all — deliveries into the six-yard box were contested by the
/// outfield players alone while the keeper watched from his line.
///
/// # The model
///
/// Project the flight; find the first moment the ball is inside his own
/// penalty area, within the envelope his arms and his leap can reach, and
/// he can be there. Then ask whether he is FAVOURITE for it the same way
/// [`KeeperBallClaim`] does for a ground ball — a keeper who comes for
/// everything gets lobbed and beaten to it, which is why
/// `command_of_area` sets how far he comes and `bravery` how much traffic
/// he will come through, rather than either of them simply making him
/// better.
pub struct KeeperAerialClaim;

impl KeeperAerialClaim {
    /// Below this the ball is not in the air in any meaningful sense and
    /// the ground-ball claim owns it.
    const MIN_HEIGHT: f32 = 1.35;
    /// How much higher than an outfielder a keeper plays the ball — he
    /// has his hands above his head and is allowed to use them.
    const ARMS: f32 = 0.45;
    /// How far he comes for one at all (6 m), before `command_of_area`.
    const RANGE_BASE: f32 = 48.0;
    /// …and how much of the box `command_of_area` adds (up to ~20 m).
    const RANGE_COMMAND: f32 = 112.0;
    /// How far ahead the flight is projected (1.5 s) and at what
    /// resolution. Coarse on purpose: this runs every tick for both
    /// keepers, and the answer only has to be good enough to start
    /// running — he re-reads it on the way.
    const LOOKAHEAD_TICKS: f32 = 150.0;
    const STEP_TICKS: f32 = 10.0;
    /// Radius around the meeting point that counts as traffic (3.5 m).
    const TRAFFIC_RADIUS: f32 = 28.0;

    /// The highest ball this keeper can take standing / at full stretch.
    pub fn standing_ceiling() -> f32 {
        AerialReach::STANDING + Self::ARMS
    }

    pub fn leap_ceiling(jumping: f32) -> f32 {
        AerialReach::ceiling(jumping) + Self::ARMS
    }

    /// Book a claim the keeper has just decided to go for. Called only at
    /// the two states a claim can START from, so a claim he spends a
    /// second running to counts once rather than once per tick.
    #[allow(unused_variables)]
    pub fn note_start(ctx: &StateProcessingContext, claim: &AerialClaim) {
        #[cfg(feature = "match-logs")]
        {
            crate::mid_run_diag::KeeperActionDiag::note(3);
            let range = (claim.meeting_point - ctx.player.position).magnitude();
            crate::mid_run_diag::KeeperActionDiag::add(8, (range * 100.0).max(0.0) as u64);
        }
    }

    /// Read the flight and decide. `None` means it is not his ball —
    /// either it is not an aerial claim at all, or somebody else is
    /// better placed for it.
    pub fn assess(ctx: &StateProcessingContext) -> Option<AerialClaim> {
        // ── Cheap rejects, in the order that kills the most ticks ─────
        let ball = &ctx.tick_context.positions.ball;
        // Airborne now, or on its way up. A ball rolling along the floor
        // is `KeeperBallClaim`'s.
        if ball.position.z < Self::MIN_HEIGHT && ball.velocity.z <= 0.0 {
            return None;
        }
        if ctx.ball().is_owned() || ctx.ball().blocked_from_recollecting() {
            return None;
        }
        if !ctx.ball().on_own_side() {
            return None;
        }

        let prof = GoalkeeperSkillProfile::from_ctx(ctx);
        let keeper = ctx.player.position;
        // How far he is willing to come. A commanding keeper defends the
        // whole six-yard box and most of the penalty area; a line-keeper
        // comes for what lands on him.
        let range = Self::RANGE_BASE + prof.aerial_command * Self::RANGE_COMMAND;
        // Nothing beyond that plus the distance the ball can still travel
        // is worth projecting.
        if (ball.position - keeper).magnitude() > range + Self::LOOKAHEAD_TICKS * 2.0 {
            return None;
        }

        let penalty_area = ctx
            .context
            .penalty_area(ctx.player.side == Some(PlayerSide::Left));
        let leap_ceiling = Self::leap_ceiling(ctx.player.skills.physical.jumping);
        let standing_ceiling = Self::standing_ceiling();
        // His travel speed for the race. `Explosive` is the band the
        // engine gives him in `Jumping` / `ComingOut`, so the race he
        // thinks he can win is the one he actually runs.
        let speed = ctx
            .player
            .skills
            .goalkeeper_max_speed(
                ctx.player.player_attributes.condition,
                GoalkeeperSpeedContext::Explosive,
            )
            .max(0.1);

        // ── Project the flight ────────────────────────────────────────
        let mut t = Self::STEP_TICKS;
        while t <= Self::LOOKAHEAD_TICKS {
            let at = t;
            t += Self::STEP_TICKS;
            let z = ball.position.z + ball.velocity.z * at - 0.5 * GRAVITY_PER_TICK * at * at;
            if z <= 0.0 {
                break; // it has landed; anything after this is a bounce
            }
            let point = Vector3::new(
                ball.position.x + ball.velocity.x * at,
                ball.position.y + ball.velocity.y * at,
                0.0,
            );

            if z < Self::MIN_HEIGHT || z > leap_ceiling {
                continue;
            }
            // Hands are only legal in his own area, and a keeper who
            // leaves it to punch a cross is not commanding his box, he is
            // lost. The `Clearing` path owns everything outside.
            if !(penalty_area.min.x..=penalty_area.max.x).contains(&point.x)
                || !(penalty_area.min.y..=penalty_area.max.y).contains(&point.y)
            {
                continue;
            }
            let travel = (point - keeper).magnitude();
            if travel > range {
                continue;
            }
            // Can he be there? `at` is measured from now, and he is
            // already moving, so this is deliberately a plain foot-race
            // rather than a full arrival model. He wants to be set half a
            // take-off before the ball arrives.
            let arrive = travel / speed;
            if arrive > at - AerialClaim::TAKEOFF_TICKS * 0.5 {
                continue;
            }

            // ── Is it his? ────────────────────────────────────────────
            let mut traffic = 0.0f32;
            let mut opponent_beats_him = false;
            for opponent in ctx.players().opponents().nearby_at(point, 64.0) {
                let d = (point - opponent.position).magnitude();
                if d <= Self::TRAFFIC_RADIUS {
                    traffic += 1.0;
                }
                // Hands and a leap win him a ball he is marginally
                // further from — the same allowance `KeeperBallClaim`
                // makes, widened by how far up the ball is (nobody else
                // can put a hand on it).
                let aerial_edge = 1.15 + prof.aerial_command * 0.35;
                if d * aerial_edge < travel {
                    opponent_beats_him = true;
                    break;
                }
            }
            if opponent_beats_him {
                return None;
            }
            let traffic = 1.0 - (-traffic / 1.5).exp();
            // Coming through a crowd is a matter of nerve, and a keeper
            // who has none stays on his line and lets his defenders head
            // it. `eccentricity` is appetite, not quality, so it only
            // moves how speculative a claim he will take on.
            let nerve = (ctx.player.skills.mental.bravery / 20.0).clamp(0.0, 1.0) * 0.7
                + prof.eccentricity * 0.3;
            if traffic > 0.25 + nerve * 0.70 {
                return None;
            }

            return Some(AerialClaim {
                meeting_point: point,
                height: z,
                ticks: at,
                traffic,
                standing: z <= standing_ceiling,
            });
        }

        None
    }
}

/// Goalkeeper condition processor (type alias for clarity)
pub type GoalkeeperCondition = ConditionProcessor<GoalkeeperConfig>;

#[cfg(test)]
mod tests {
    use super::*;

    /// A keeper plays the ball higher than anyone else on the pitch,
    /// standing or jumping — that is what the gloves are for. If these
    /// ever invert, `KeeperAerialClaim` sends him for balls he cannot
    /// reach and `GoalkeeperJumpingState` refuses ones he can.
    #[test]
    fn a_keeper_reaches_higher_than_an_outfielder() {
        assert!(KeeperAerialClaim::standing_ceiling() > AerialReach::STANDING);
        for jumping in [1.0f32, 10.0, 20.0] {
            let leap = KeeperAerialClaim::leap_ceiling(jumping);
            assert!(
                leap > KeeperAerialClaim::standing_ceiling(),
                "leaping must beat standing at jumping {jumping}: {leap:.2}"
            );
            assert!(
                leap > AerialReach::ceiling(jumping),
                "a keeper's hands must beat an outfielder's head at jumping {jumping}"
            );
        }
        assert!(
            KeeperAerialClaim::leap_ceiling(20.0) > KeeperAerialClaim::leap_ceiling(1.0),
            "a better leaper must reach higher"
        );
    }

    /// `Jumping` fires `PlayerMatchState::leap_apex` on ENTRY, so entering
    /// it early means landing before the ball arrives. The take-off window
    /// has to be inside the leap's own hang time, and the contact test has
    /// to require he is actually at the meeting point.
    #[test]
    fn he_only_leaves_the_ground_when_the_ball_is_arriving() {
        let here = Vector3::new(100.0, 100.0, 0.0);
        let claim = |ticks: f32, point: Vector3<f32>| AerialClaim {
            meeting_point: point,
            height: 2.5,
            ticks,
            traffic: 0.0,
            standing: false,
        };
        assert!(claim(10.0, here).at_contact(here), "arriving, and he is there");
        assert!(
            !claim(90.0, here).at_contact(here),
            "still a second away — he must run to it, not jump early"
        );
        assert!(
            !claim(10.0, Vector3::new(140.0, 100.0, 0.0)).at_contact(here),
            "arriving, but 5 m away — jumping on the spot achieves nothing"
        );
    }
}

// Re-export for convenience
pub use crate::r#match::engine::player::strategies::common::ActivityIntensity;
