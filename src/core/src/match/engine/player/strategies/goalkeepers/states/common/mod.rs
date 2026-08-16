use crate::club::player::skills::GoalkeeperSpeedContext;
use crate::r#match::engine::ball::ball::{AerialReach, GRAVITY_PER_TICK};
use crate::r#match::engine::player::strategies::common::{
    ActivityIntensityConfig, ConditionProcessor, GOALKEEPER_JADEDNESS_INCREMENT,
    GOALKEEPER_JADEDNESS_INTERVAL, GOALKEEPER_LOW_CONDITION_THRESHOLD,
};
use crate::r#match::player::strategies::players::ops::goalkeeper_skill::GoalkeeperSkillProfile;
use crate::r#match::{MatchPlayerLite, PlayerSide, StateProcessingContext};
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
    /// Inside this he is where he wants to be and stands set, measured
    /// ALONG the goal-to-goal axis. A keeper standing still, set, is a
    /// real and common thing; without a deadzone he chases a target that
    /// moves every tick with the ball and covers more ground than a
    /// midfielder.
    ///
    /// 26u = 3.25 m. A keeper repositions in STEPS — he shuffles, then
    /// sets, then shuffles again — rather than gliding continuously after
    /// the ball. At 10u he tracked it every tick and covered 10.2 km
    /// against a real ~5 km.
    pub const SET_DEADZONE: f32 = 26.0;

    /// …and the same tolerance ACROSS the goal, which is a completely
    /// different quantity.
    ///
    /// # Why the two axes cannot share a number
    ///
    /// The deadzone was one scalar tested against the whole 2-D gap. But
    /// the two axes of a keeper's position are not comparable: **depth has
    /// a 200-unit working range** (18u on his line to 220u sweeping) while
    /// **lateral has about ±15u**, because the lateral term is only
    /// `depth × sin(angle to the ball)`. A single 26u threshold is
    /// therefore slack in depth and *larger than the entire lateral range*
    /// — so the keeper repositioned for depth and, in practice, never once
    /// stepped sideways to stay on the angle. He was frozen laterally at
    /// wherever the last depth-driven step had left him.
    ///
    /// 26u is also 3.25 m on a goal that is 7.32 m wide, against a dive
    /// that reaches 2.5-4.0 m. In other words the tolerance permitted him
    /// to stand a full post-width off the angle, which is not a tolerance
    /// at all — it is the difference between a save and a goal. Measured:
    /// he was motionless on **53% of every tick the ball was live within
    /// 37.5 m of his goal**, and **22% of the shots that arrived on frame
    /// found him beyond his own reach of them, a mean 6.10 m away** — the
    /// keeper standing at one post while the ball crosses the line at the
    /// other, which is exactly the reported complaint.
    ///
    /// 6u = 0.75 m: a shuffle. Sideways adjustment is the most continuous
    /// thing a goalkeeper does, and the one his whole game is built on.
    pub const LATERAL_DEADZONE: f32 = 6.0;

    /// Is he set? Tight across the goal, slack in depth — see
    /// [`Self::LATERAL_DEADZONE`].
    pub fn is_set(keeper: Vector3<f32>, target: Vector3<f32>) -> bool {
        (keeper.y - target.y).abs() < Self::LATERAL_DEADZONE
            && (keeper.x - target.x).abs() < Self::SET_DEADZONE
    }

    /// How far ahead of the ball a keeper with perfect anticipation
    /// plays, in ticks of its current travel. 24 ticks ≈ a quarter of a
    /// second of flight.
    ///
    /// # Why a keeper needs this at all
    ///
    /// The model positioned him against the ball's CURRENT position, so
    /// every keeper in the game read the play identically and none of them
    /// read it early — he shuffled to where the ball was, arriving as it
    /// left. Nothing in the resting model distinguished a keeper who sees
    /// the cross coming from one who watches it land, which is most of
    /// what separates keepers who are good at this from keepers who are
    /// not.
    ///
    /// Leading the ball is the mechanism, and `anticipation` is the
    /// attribute: it is already the second-heaviest term in the
    /// positioning composite and, before this, it changed nothing about
    /// where he stood.
    const LEAD_TICKS: f32 = 24.0;

    /// The spot, for a keeper defending `own_goal`, given where the ball
    /// is and where his side's defensive line is sitting.
    ///
    /// `read` is the keeper's positioning composite (`positioning`,
    /// `anticipation`, `decisions`, `concentration`, `command_of_area`…),
    /// and it is what makes one keeper better at this than another: it
    /// sets how far ahead of the ball he plays and how truly he sits on
    /// the angle rather than hedging toward the middle and hoping.
    pub fn point(
        own_goal: Vector3<f32>,
        ball_now: Vector3<f32>,
        ball_velocity: Vector3<f32>,
        side: PlayerSide,
        defensive_line_x: f32,
        field_width: f32,
        command_of_area: f32,
        read: f32,
    ) -> Vector3<f32> {
        let read = read.clamp(0.0, 1.0);
        // Where the ball is GOING. A keeper who reads the game is already
        // moving as the ball is struck; one who does not is still watching
        // where it was.
        let ball = ball_now + ball_velocity * (Self::LEAD_TICKS * read);
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
        let fidelity = 0.55 + read * 0.45;
        Vector3::new(
            on_angle.x,
            own_goal.y + (on_angle.y - own_goal.y) * fidelity,
            0.0,
        )
    }

    /// How far out of position he will tolerate being before he does
    /// something about it, across the goal.
    ///
    /// Holding a set position is CONCENTRATION, and nothing in the keeper
    /// model read it: every keeper re-set at exactly the same tolerance,
    /// so ball-watching — the commonest way a real keeper is caught at the
    /// wrong post — could not happen to a bad one or be avoided by a good
    /// one. A switched-on keeper adjusts for three quarters of a metre; a
    /// distracted one lets it go to nearly two. Centred so the average
    /// keeper keeps exactly `LATERAL_DEADZONE` and this is a spread, not a
    /// loosening of every keeper in the game.
    pub fn lateral_tolerance(concentration: f32) -> f32 {
        Self::LATERAL_DEADZONE
            * (1.0
                + (GoalkeeperSkillProfile::POPULATION_READ - concentration.clamp(0.0, 1.0)) * 1.2)
    }

    /// Is he set, for a keeper of this concentration?
    pub fn is_set_with(keeper: Vector3<f32>, target: Vector3<f32>, concentration: f32) -> bool {
        (keeper.y - target.y).abs() < Self::lateral_tolerance(concentration)
            && (keeper.x - target.x).abs() < Self::SET_DEADZONE
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

/// How far off his own goal line a keeper is allowed to be — the one
/// place that answers it, for every state that has to ask.
///
/// # Why this exists
///
/// Five states bounded the keeper's excursions, and every one of them did
/// it the same way: `distance_from_start_position()` against a constant of
/// 40 or 50 units. That distance is measured from his **kickoff dot** —
/// (20, 275), two and a half metres in front of the middle of the goal —
/// and 40-50u is **5 to 6 metres**, shallower than the six-yard box. It is
/// also a RADIUS, so the metres he spends shuffling across his line to
/// cover the angle are charged against the same allowance as the metres he
/// spends coming forward: a keeper who has correctly moved 5 m toward his
/// near post has already spent it all and may not come out at all.
///
/// The effect was total, and it is the reported bug. `ComingOut` commits
/// to a carrier up to 18.75 m away and then abandoned the sweep the moment
/// he was 5 m from the dot. Measured over 60 matches: **83% of every sweep
/// the keeper committed to died on this leash** — 175 abandons a match
/// against 211 commitments — and `ComingOut` held 2.4% of his ticks while
/// `ReturningToGoal` held 5.1%. With an opponent carrying the ball inside
/// 25 m he spent **18.6% of those ticks running BACK to his line and only
/// 13.5% coming to meet it.** From the stands that is a keeper who dances
/// on his line while a striker walks in.
///
/// It is also the `COMMIT < DISENGAGE` invariant broken for the fifth time
/// in this engine (see `MAX_COMING_OUT_DISTANCE`, which was widened to
/// 240u to fix precisely this — the fix never reached the second, tighter
/// gate sitting underneath it). An entry condition three times wider than
/// its own give-up condition is a two-cycle by construction.
///
/// # The model
///
/// An excursion is measured **along the goal-to-goal axis**, because that
/// is the axis it happens on, and it is bounded by how far this keeper
/// dares leave his goal:
///   * a line-keeper defends to about the penalty spot — 15 m;
///   * a sweeper defends the space behind his back four — 32 m.
///
/// Both are strictly beyond any distance at which a keeper *enters* a
/// sweep, and strictly beyond the depth `KeeperRestPosition` gives him
/// while his goal is under threat (measured at 4.7 m), so neither the
/// entry nor the resting position can trip it.
pub struct KeeperSweepLimit;

impl KeeperSweepLimit {
    /// A keeper who stays home. 120u = 15 m — the penalty spot.
    const LINE_KEEPER: f32 = 120.0;
    /// …and how much further a sweeper goes, out to 32 m.
    const SWEEPER: f32 = 136.0;

    /// How far off his line this keeper is willing to be.
    pub fn off_line(rushing_out_profile: f32) -> f32 {
        Self::LINE_KEEPER + rushing_out_profile.clamp(0.0, 1.0) * Self::SWEEPER
    }

    /// How far off his line he currently is.
    pub fn distance_off_line(ctx: &StateProcessingContext) -> f32 {
        (ctx.player.position.x - ctx.ball().direction_to_own_goal().x).abs()
    }

    /// Is he still inside the space he is prepared to defend?
    pub fn is_within(ctx: &StateProcessingContext, rushing_out_profile: f32) -> bool {
        Self::distance_off_line(ctx) <= Self::off_line(rushing_out_profile)
    }
}

/// Is the man on the ball actually THROUGH, or is there a defender
/// goal-side of him?
///
/// # Why this exists
///
/// `should_rush_out_for_ball` short-circuits to "come out" for any carrier
/// inside 18.75 m, deliberately: it was added because a forward with the
/// ball at his feet could otherwise carry it to within a couple of feet of
/// a keeper who never moved. That is the right instinct and the wrong
/// trigger, because a carrier 18 m out with two centre-backs in front of
/// him is not a keeper's problem — he is the defence's — and charging at
/// him leaves an empty goal to chip.
///
/// It also swallowed the branch beneath it whole: `Standing` asks the
/// come-out question first and only then asks whether to *set* himself,
/// so a trigger that fires at 18.75 m made `PreparingForSave` — the state
/// that actually tracks the angle continuously — **unreachable for any
/// carrier inside 12.5 m**, the one situation it exists for. The keeper
/// committed, hit the leash above, turned round, ran home, and committed
/// again.
///
/// A keeper comes for the ball when there is nobody between him and it.
/// That is the whole of the real rule, and it is what this asks.
pub struct KeeperCarrierThreat;

impl KeeperCarrierThreat {
    /// How far to either side of the carrier's route to goal a defender
    /// still counts as being in the way. 24u = 3 m — a stride and a leg.
    ///
    /// At 7 m this test read as "covered" on essentially every carrier in
    /// the game (commitments fell 211/match → 1.4) because a seven-metre
    /// corridor over an eighteen-metre run sweeps up half the defensive
    /// third. Cover means somebody who can actually make the challenge,
    /// not somebody in the same postcode.
    const COVER_CORRIDOR: f32 = 24.0;

    /// A defender inside this of his own goal is covering the GOAL, not
    /// the man — he is on the line waiting for the shot, and the keeper
    /// coming to meet the ball is still the right call. 44u = 5.5 m, the
    /// six-yard box.
    const ON_THE_LINE: f32 = 44.0;

    /// Nobody goal-side of him: this one is the keeper's.
    pub fn is_through(ctx: &StateProcessingContext, carrier: &MatchPlayerLite) -> bool {
        let goal = ctx.ball().direction_to_own_goal();
        let to_goal = goal - carrier.position;
        let run = to_goal.magnitude();
        if run < 1.0 {
            return true;
        }
        let lane = to_goal / run;

        !ctx.players().teammates().all().any(|mate| {
            if mate.id == ctx.player.id {
                return false;
            }
            let rel = mate.position - carrier.position;
            // Goal-side of the carrier…
            let along = rel.dot(&lane);
            if along <= 0.0 || along >= run {
                return false;
            }
            // …not already back on his own line…
            if (mate.position - goal).magnitude() < Self::ON_THE_LINE {
                return false;
            }
            // …and inside the corridor the carrier is running down.
            (rel - lane * along).magnitude() <= Self::COVER_CORRIDOR
        })
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

    /// How much further off his line a keeper who reads the game sets
    /// himself. 26u = 3.25 m on top of the distance term, so the band runs
    /// 1.25 m (a poor keeper against a point-blank strike) to 6.75 m (a
    /// good one facing a long-range effort).
    ///
    /// # Why this exists
    ///
    /// `set_point` took no keeper at all. Every goalkeeper in the game,
    /// from an international to a 17-year-old third choice, set himself in
    /// **exactly the same place** for the same shot — and since this is
    /// the position both save paths measure him from, position selection
    /// was worth precisely nothing. All keeper quality lived in the save
    /// roll: a good keeper was a better shot-stopper standing in the same
    /// spot as a bad one, never a keeper who was in a better spot.
    ///
    /// Coming off the line to cut the angle is the single most legible
    /// thing good positioning buys, and `SaveModel::wedge` now prices it
    /// honestly (more of the mouth covered, less time to extend), so the
    /// trade-off is real in both directions rather than a free bonus.
    const READ_DEPTH: f32 = 26.0;

    /// The spot to defend a strike from `shot_distance` away, guarding
    /// `goal_line_y`. `read` is the keeper's positioning composite.
    pub fn set_point(
        own_goal: Vector3<f32>,
        goal_line_y: f32,
        shot_distance: f32,
        field_width: f32,
        read: f32,
    ) -> Vector3<f32> {
        let opened = (shot_distance / Self::DEPTH_RANGE).clamp(0.0, 1.0);
        // CENTRED on the population, so the average keeper sets exactly
        // where he always did and this is a spread rather than a quiet
        // deepening of every keeper in the game — see
        // `GoalkeeperSkillProfile::POPULATION_READ`.
        let read_gain =
            (read.clamp(0.0, 1.0) - GoalkeeperSkillProfile::POPULATION_READ) * Self::READ_DEPTH;
        let depth =
            (Self::MIN_DEPTH + (Self::MAX_DEPTH - Self::MIN_DEPTH) * opened + read_gain * opened)
                .max(Self::MIN_DEPTH * 0.5);
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

    /// **COMMIT < DISENGAGE, or the pair is a two-cycle.** This engine has
    /// broken that invariant five times — the shot bar, the keeper's
    /// come-out scan, `MAX_COMING_OUT_DISTANCE`, the full-back overlap, and
    /// the kickoff-slot leash this constant replaced, which killed 83% of
    /// every sweep the keeper committed to.
    ///
    /// `GoalkeeperStandingState::should_rush_out_for_ball` commits to a
    /// carrier out to `150 * (1 + risk * 0.4)` = 180u at the extreme, so
    /// the excursion limit has to clear that for EVERY keeper, including
    /// the one least willing to leave his line.
    #[test]
    fn a_keeper_may_always_go_further_than_the_distance_he_commits_at() {
        const WIDEST_ENTRY: f32 = 180.0;
        for rushing in [0.0f32, 0.25, 0.5, 0.75, 1.0] {
            let limit = KeeperSweepLimit::off_line(rushing);
            assert!(
                limit > WIDEST_ENTRY * 0.65,
                "a keeper at rushing_out {rushing} gives up at {limit:.0}u, which is not a \
                 sweep — he would abandon on the tick he commits"
            );
        }
        assert!(
            KeeperSweepLimit::off_line(1.0) > WIDEST_ENTRY,
            "a sweeper must be able to go further than the furthest carrier he sets off for"
        );
        assert!(
            KeeperSweepLimit::off_line(1.0) > KeeperSweepLimit::off_line(0.0),
            "and a sweeper must go further than a line-keeper"
        );
    }

    /// Across the goal and along it are not the same tolerance. The goal
    /// is 58u wide and a keeper's dive reaches 20-32u, so a lateral
    /// tolerance anywhere near the depth one lets him stand a post-width
    /// out of position and call it set.
    #[test]
    fn the_set_tolerance_is_tight_across_the_goal_and_slack_in_depth() {
        assert!(
            KeeperRestPosition::LATERAL_DEADZONE * 3.0 < KeeperRestPosition::SET_DEADZONE,
            "the two axes must not share a scale"
        );
        let target = Vector3::new(20.0, 270.0, 0.0);
        assert!(
            KeeperRestPosition::is_set(Vector3::new(40.0, 270.0, 0.0), target),
            "2.5 m deeper than ideal is set"
        );
        assert!(
            !KeeperRestPosition::is_set(Vector3::new(20.0, 285.0, 0.0), target),
            "…but 1.9 m across the goal is a post-width, and is not"
        );
    }

    /// Concentration is what holds a set position, and a keeper who has
    /// switched off is the one caught at the wrong post. If these collapse
    /// to the same number, ball-watching cannot happen to a bad keeper or
    /// be avoided by a good one.
    #[test]
    fn a_switched_off_keeper_tolerates_being_further_out_of_position() {
        let sharp = KeeperRestPosition::lateral_tolerance(1.0);
        let dull = KeeperRestPosition::lateral_tolerance(0.0);
        assert!(dull > sharp * 1.5, "sharp {sharp:.2} vs dull {dull:.2}");
        assert!(
            sharp > 0.0,
            "even an elite keeper needs a deadzone or he glides"
        );
    }

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
        assert!(
            claim(10.0, here).at_contact(here),
            "arriving, and he is there"
        );
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
