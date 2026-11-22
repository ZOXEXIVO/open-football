use crate::club::player::skills::GoalkeeperSpeedContext;
use crate::r#match::MatchContext;
use crate::r#match::engine::ball::ball::contest::save::SaveModel;
use crate::r#match::engine::ball::ball::{
    AerialReach, BallRoll, DeadBall, GRAVITY_PER_TICK, MAX_OWNER_TRACK_DISTANCE, ShotTarget,
};
use crate::r#match::engine::goal::{GOAL_HEIGHT, GOAL_WIDTH};
use crate::r#match::engine::player::strategies::shared::{
    ActivityIntensityConfig, ConditionProcessor, GOALKEEPER_JADEDNESS_INCREMENT,
    GOALKEEPER_JADEDNESS_INTERVAL, GOALKEEPER_LOW_CONDITION_THRESHOLD,
};
use crate::r#match::events::Event;
use crate::r#match::goalkeepers::states::GoalkeeperPunchingState;
use crate::r#match::goalkeepers::states::state::GoalkeeperState;
#[cfg(feature = "match-logs")]
use crate::r#match::player::diagnostics::keeper_census::{ClaimAction, KeeperCensus};
use crate::r#match::player::events::gk_claim::{ClaimSituation, GkClaimContest};
use crate::r#match::player::events::{FoulCommitted, FoulSeverity, FoulSource, PlayerEvent};
use crate::r#match::player::strategies::players::ops::goalkeeper_skill::GoalkeeperSkillProfile;
use crate::r#match::player::strategies::players::ops::skill_composites as sc;
use crate::r#match::{
    ClaimIntent, GameTickContext, MatchPlayer, MatchPlayerLite, PlayerSide, StateChangeResult,
    StateProcessingContext,
};
#[cfg(feature = "match-logs")]
use crate::mid_run_diag::{KeeperActionDiag, KeeperDiveDiag};
use nalgebra::Vector3;

mod goal_kick;
mod mind;
mod punt;
mod read;
mod release;
mod set_piece;
mod split_step;
pub use goal_kick::*;
pub use mind::*;
pub use punt::*;
pub use read::*;
pub use release::*;
pub use set_piece::*;
pub use split_step::*;

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

/// Diagnostic switch: with `OF_KEEPER_CALM_OFF` set, the keeper goes back
/// to tracking the ball continuously — the resting tolerance stops opening
/// out with distance, `PreparingForSave` stands down on a one-tick
/// possession flag again and holds no set position, and `Standing` re-arms
/// the save posture on his own side's passes.
///
/// This is the A/B control for the "he runs around instead of goalkeeping"
/// work. Those four sites decide how much a keeper moves on every tick of
/// every match, so their effect on save rate and goals cannot be read off
/// the diff — and it must not be answered by checking out an older
/// revision either, because the working tree moves under you. Same pattern
/// and purpose as `OF_SHAPE_OFF` and `OF_KEEPER_SERVO`; read once per
/// process. Debug infrastructure — do not remove.
///
/// Deliberately does NOT gate the two-cycle repairs in `Catching` and
/// `Clearing` that shipped alongside: a state entered and left inside
/// 45 ms, a hundred and thirty times a match, is not a behaviour anyone
/// chose, so both arms should have those. What this isolates is the
/// judgement call — how still a keeper ought to be.
pub struct KeeperDebug;

impl KeeperDebug {
    pub fn calm_off() -> bool {
        use std::sync::OnceLock;
        static OFF: OnceLock<bool> = OnceLock::new();
        *OFF.get_or_init(|| std::env::var("OF_KEEPER_CALM_OFF").is_ok())
    }

    /// `OF_KEEPER_SET_PACE=off`: `PreparingForSave` without a shot back on
    /// the explosive band at sprint pace, for the A/B.
    pub fn set_pace_off() -> bool {
        use std::sync::OnceLock;
        static OFF: OnceLock<bool> = OnceLock::new();
        *OFF.get_or_init(|| Self::off("OF_KEEPER_SET_PACE"))
    }

    /// `OF_KEEPER_STRIKE_SET=off`: a keeper moving at the strike reacts as
    /// quickly as a set one.
    pub fn strike_set_off() -> bool {
        use std::sync::OnceLock;
        static OFF: OnceLock<bool> = OnceLock::new();
        *OFF.get_or_init(|| Self::off("OF_KEEPER_STRIKE_SET"))
    }

    /// `OF_KEEPER_BISECTOR=off`: the rest and set points back on the line
    /// from the ball to the middle of his goal.
    pub fn bisector_off() -> bool {
        use std::sync::OnceLock;
        static OFF: OnceLock<bool> = OnceLock::new();
        *OFF.get_or_init(|| Self::off("OF_KEEPER_BISECTOR"))
    }

    fn off(switch: &str) -> bool {
        std::env::var(switch).is_ok_and(|value| value.eq_ignore_ascii_case("off"))
    }
}

/// **A goalkeeper does not chase the ball he has just played.**
///
/// # The report, and what it measured
///
/// *"Sometimes he kicks the ball around with his hands and runs after it
/// himself, and sometimes the ball even rolls out of bounds."*
///
/// Read off a recorded match rather than off a counter — every mechanism
/// in this file emits an event and none of them showed anything wrong.
/// What the replay shows is unambiguous: of **22 keeper deliveries in one
/// match, 16 (73%) were followed by the keeper sprinting after his own
/// ball**, at a peak of 8-9 m/s, for one and a half to three seconds,
/// covering a mean of **12 m up the pitch** before turning round and
/// running back. One of them, traced tick by tick: he throws it out to a
/// full-back at 804.06 s and by 804.12 s he is in `Catching`, a metre and
/// a half behind his own throw, at eight metres a second, for the next
/// thirty metres — `Catching` → `Standing` → `Clearing` → `Standing` →
/// `Catching` → `ComingOut` → `Catching`, seven state changes in three
/// seconds.
///
/// # Why every gate let it through
///
/// Each of them asks `!ctx.ball().is_owned()`, and an untouched throw
/// satisfies that: nobody is on it yet. Two of the doors do carry
/// `blocked_from_recollecting`, but that bar is about a ball which has NOT
/// travelled — it lifts the moment the ball is five metres clear of him,
/// which for a delivery is immediately and is exactly backwards. So
/// `Standing`'s close-ball and sweep branches, both of `Walking`'s
/// loose-ball branches, its `should_come_out_advanced`,
/// `ReturningToGoal`'s claim branch and the dispatcher's loose-ball
/// override all opened on his own pass.
///
/// The same bug class this engine keeps producing — an outer gate that
/// does not ask the question the decision under it is about — and the same
/// fix that worked for `Interception::is_available`: **one shared
/// predicate, applied at every door.** Patching the doors individually
/// does not hold; there are eight of them and the first pass would simply
/// move the entry.
///
/// # The rule
///
/// A ball a player has just deliberately played, which nobody has touched
/// since, is not a loose ball to him. For a keeper it is also the Laws:
/// a ball released from his hands may not be handled again until somebody
/// else plays it, and `Ball::record_touch` clears the release marker at
/// exactly that moment — so the football rule and the engine's own
/// bookkeeping are already the same statement.
pub struct KeeperDelivery;

impl KeeperDelivery {
    /// Diagnostic switch: with `OF_KEEPER_FOLLOW` set the keeper chases
    /// his own delivery again, exactly as he did before this existed.
    ///
    /// The A/B control for it. Sixteen chases a match, each 12 m out and
    /// 12 m back, is a large share of every metre he covers and of every
    /// state transition he makes, so what removing it costs cannot be read
    /// off the diff. Same pattern and purpose as `OF_KEEPER_CALM_OFF` and
    /// `OF_KEEPER_SERVO`; read once per process. Debug infrastructure — do
    /// not remove.
    pub fn follow_allowed() -> bool {
        use std::sync::OnceLock;
        static ON: OnceLock<bool> = OnceLock::new();
        *ON.get_or_init(|| std::env::var("OF_KEEPER_FOLLOW").is_ok())
    }

    /// Is this ball still the one HE put into play?
    ///
    /// Every keeper door that asks "is there a loose ball for me" should
    /// ask this first and decline.
    pub fn is_his(ctx: &StateProcessingContext) -> bool {
        // ⚠ Never for the taker of a restart. A dead ball is not a
        // delivery, he is the only man allowed to touch it, and walking to
        // it is the whole of what he is doing — the same exemption
        // `GoalkeeperTakeBallState` carries twice, and for the same reason.
        !Self::follow_allowed()
            && ctx.tick_context.ball.restart_taker != Some(ctx.player.id)
            && ctx.ball().is_my_own_delivery()
    }
}

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
    /// 280u = 35 m — the ceiling of the curve, not a place he often
    /// stands: the line tether below almost always takes it first.
    const SWEEP_DEPTH: f32 = 280.0;
    /// How sharply depth falls away as the ball comes toward him.
    ///
    /// This was 2.0, and the square is what kept the keeper on his line.
    /// The ball's mean distance from a goal is 46% of the pitch, and
    /// `0.46² = 0.21`, so the TYPICAL depth was a fifth of the range —
    /// measured over 10 × 90 min with `dev_match heat`, a mean of 6.7 m
    /// off his line with sd 3.1 m and 99.3% of the match inside his own
    /// box. The model's own reference table above asks for 12-18 m with
    /// the ball in midfield, and the square could not produce 8.
    ///
    /// 1.35 reproduces all four reference rows: ~3.5 m with the ball in
    /// the box, ~6.5 in the final third, ~14 in midfield, ~21 with the
    /// ball in the opponent's half. The asymmetry the square was reaching
    /// for — dropping back is urgent, pushing up is not — is a property
    /// of how fast he travels, not of where he wants to be, and it is
    /// already there: the same `SET_DEADZONE` that stops him gliding
    /// after the ball makes the push-up gradual, while a ball arriving
    /// puts him in a save state that has no deadzone at all.
    const DEPTH_FALLOFF: f32 = 1.35;
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

    /// Where a keeper of this `read` plays the ball: [`Self::LEAD_TICKS`]
    /// × read ahead of it along its travel. Every point he stands on reads
    /// the ball through this, so the set and the one-on-one anticipate the
    /// play as the rest does.
    pub fn anticipated(ball: Vector3<f32>, velocity: Vector3<f32>, read: f32) -> Vector3<f32> {
        ball + velocity * (Self::LEAD_TICKS * read.clamp(0.0, 1.0))
    }

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
        let ball = Self::anticipated(ball_now, ball_velocity, read);
        let to_ball = ball - own_goal;
        let ball_distance = to_ball.magnitude();

        // Depth rises with the ball's distance, biased toward the line —
        // see [`Self::DEPTH_FALLOFF`] for why the exponent is what it is
        // and what the square it replaced actually produced.
        let far = (ball_distance / field_width).clamp(0.0, 1.0);
        let (sweep, falloff) = if MatchContext::shape_fix_off() {
            (220.0, 2.0)
        } else {
            (Self::SWEEP_DEPTH, Self::DEPTH_FALLOFF)
        };
        let mut depth = Self::NEAR_DEPTH + (sweep - Self::NEAR_DEPTH) * far.powf(falloff);
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

        // At that depth, on the line he guards: the bisector of the angle
        // the posts make at the ball (`KeeperSetPosition::guard_y`). A
        // better-positioned keeper sits truer on it; a poorer one hedges
        // centrally and can be beaten at his near post.
        let toward_ball = if ball_distance > 1.0 {
            to_ball / ball_distance
        } else {
            Vector3::new(side.forward_dir_x(), 0.0, 0.0)
        };
        let on_angle = own_goal + toward_ball * depth;
        let guard_y = KeeperSetPosition::guard_y(own_goal, ball, on_angle.x);
        let fidelity = 0.55 + read * 0.45;
        Vector3::new(
            on_angle.x,
            own_goal.y + (guard_y - own_goal.y) * fidelity,
            0.0,
        )
    }

    /// Where this keeper wants to stand, right now — the ONE call every
    /// resting state makes.
    ///
    /// `Standing`, `Walking` and `ReturningToGoal` each assembled the
    /// eight arguments to [`Self::point`] themselves, and two of the three
    /// differences between those copies mattered. All three read
    /// `skills.goalkeeping.command_of_area / 20.0` raw, bypassing the
    /// fatigue band and the [`MatchStandard`] shift that every other
    /// keeper read goes through; and only `Standing` bounded the result,
    /// with a lateral clamp of its own to the width of the penalty area —
    /// so a keeper walking or recovering had no lateral bound at all and
    /// the same keeper wanted two different places depending on his state,
    /// which is the defect `KeeperRestPosition` was created to end.
    ///
    /// Contained to [`KeeperSweepLimit`], which is the same territory the
    /// sweep gates use. Resting outside the ground you are willing to
    /// defend is a contradiction, and while it was possible the rest model
    /// could hand him a target the sweep model would immediately send him
    /// back from.
    pub fn for_keeper(ctx: &StateProcessingContext) -> Vector3<f32> {
        let tick = ctx.current_tick();
        let vitals = (ctx.player.player_attributes.condition as u16 as u32)
            | (ctx.player.player_attributes.jadedness as u16 as u32) << 16;
        let cached = ctx
            .tick_context
            .player_agg_cache
            .borrow_mut()
            .slot_mut(ctx.player.id, tick)
            .keeper_rest
            .filter(|(cached_vitals, _)| *cached_vitals == vitals)
            .map(|(_, point)| point);
        if let Some(point) = cached {
            debug_assert_eq!(
                point,
                Self::for_keeper_uncached(ctx),
                "keeper-rest memo mismatch: player={} tick={}",
                ctx.player.id,
                tick
            );
            return point;
        }
        let point = Self::for_keeper_uncached(ctx);
        ctx.tick_context
            .player_agg_cache
            .borrow_mut()
            .slot_mut(ctx.player.id, tick)
            .keeper_rest = Some((vitals, point));
        point
    }

    fn for_keeper_uncached(ctx: &StateProcessingContext) -> Vector3<f32> {
        // A dead ball at his goal has its own place to stand, and it is
        // not a function of the ball's distance. See [`KeeperSetPieceStance`].
        if let Some(mark) = KeeperSetPieceStance::point(ctx) {
            return mark;
        }
        let prof = GoalkeeperSkillProfile::from_ctx(ctx);
        let point = Self::point(
            ctx.ball().direction_to_own_goal(),
            ctx.tick_context.positions.ball.position,
            ctx.tick_context.positions.ball.velocity,
            ctx.player.side.unwrap_or(PlayerSide::Left),
            ctx.team().tactical().defensive_line_x,
            ctx.context.field_size.width as f32,
            prof.command_of_area,
            prof.positioning,
        );
        KeeperSweepLimit::contain(ctx, point, &prof)
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

    /// How much slacker both tolerances get because the ball is a long way
    /// away — the term that decides whether he is goalkeeping or fidgeting.
    ///
    /// # Why a fixed tolerance was wrong
    ///
    /// [`Self::LATERAL_DEADZONE`] is 0.75 m and it was applied at every
    /// distance, so the keeper held his angle to within three quarters of a
    /// metre **while the ball was sixty metres away in the other penalty
    /// area**. Two things make that far more movement than it sounds:
    /// the lateral term of the rest point is `depth × sin(angle to ball)`,
    /// and `depth` is at its LARGEST when the ball is far (he has pushed up
    /// to sweep) — so the target swings widest exactly when it matters
    /// least. A ball passed across midfield moved his target six metres
    /// sideways, and a 0.75 m deadzone made him track every metre of it.
    ///
    /// Measured over 60 matches: **71% of his ticks had the ball more than
    /// 40 m away, and he covered 4778 m in them** — over half a match's
    /// mileage, at a jog, with nothing to defend. Total 9405 m against a
    /// real keeper's ~5000.
    ///
    /// # The model
    ///
    /// A keeper's positional precision requirement is set by how long a
    /// shot would take to arrive, and that is distance. Inside the box a
    /// metre is the difference between a save and a goal; from 60 m he has
    /// well over a second to step across, and standing still is not
    /// sloppiness, it is what keepers do. So the tolerance is anchored at
    /// its tight value where the shot comes from and opens out with the
    /// square of distance — the same shape (and the same reasoning about
    /// which direction is urgent) as the depth curve in [`Self::point`].
    ///
    /// 1.0 at the goal line, 8.0 at the far end of the pitch: 0.75 m of
    /// lateral tolerance in the six-yard box, 6 m with play at the other
    /// end, and no threshold anywhere in between.
    const REST_SLACK_MAX: f32 = 8.0;

    pub fn distance_slack(ball_distance: f32, field_width: f32) -> f32 {
        if KeeperDebug::calm_off() {
            return 1.0;
        }
        let far = (ball_distance / field_width).clamp(0.0, 1.0);
        1.0 + (Self::REST_SLACK_MAX - 1.0) * far * far
    }

    /// Is he set, for a keeper of this concentration, with the ball this
    /// far from his goal? Moving or still is this question alone, asked in
    /// `velocity`: relaxed or alert is the phase's (`KeeperRead::phase`), so
    /// arriving on his spot never changes his state.
    pub fn is_set_with(
        keeper: Vector3<f32>,
        target: Vector3<f32>,
        concentration: f32,
        ball_distance: f32,
        field_width: f32,
    ) -> bool {
        let slack = Self::distance_slack(ball_distance, field_width);
        (keeper.y - target.y).abs() < Self::lateral_tolerance(concentration) * slack
            && (keeper.x - target.x).abs() < Self::SET_DEADZONE * slack
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
/// it the same way: `distance_from_position()` against a constant of
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
/// # The model, and why one axis was not enough
///
/// An excursion is bounded by how far this keeper dares leave his goal:
///   * a line-keeper defends to about the penalty spot — 15 m;
///   * a sweeper defends the space behind his back four — 32 m.
///
/// Both are strictly beyond any distance at which a keeper *enters* a
/// sweep, and strictly beyond the depth `KeeperRestPosition` gives him
/// while his goal is under threat (measured at 4.7 m), so neither the
/// entry nor the resting position can trip it.
///
/// ⚠ **THAT BOUND USED TO BE MEASURED ALONG THE GOAL-TO-GOAL AXIS ALONE**,
/// on the reasoning that the axis an excursion happens on is the axis it
/// should be measured on. It is not, and the omission has a name: a keeper
/// standing on his own touchline level with his six-yard box was
/// **nought metres out of position** by this test, and a keeper who had
/// chased a loose ball into the channel spent exactly the same allowance
/// as one who had come five yards to the edge of his area. Nothing
/// anywhere in the keeper's state machine bounded him ACROSS the pitch:
/// `Standing` clamped its resting target to the width of the penalty area
/// and every other state — `ComingOut`, `TakeBall`, `Catching`,
/// `PreparingForSave` — was free to take him to the corner flag.
///
/// Measured over 200 matches (`KEEPER EXCURSION CENSUS`, added with this
/// fix because no existing diagnostic could see it — every one of them
/// measures the depth axis too): he was more than 25 m from his own goal
/// for **1.8 seconds a minute**, reached **80.9 m** from it at the
/// extreme and **36.5 m** across — wider than the pitch is half-wide, so
/// off it — and spent **34.7 seconds a match beyond 25 m AND wider than
/// his own penalty area**, which has no legitimate reading at all. That
/// is the reported behaviour: he comes out far too far, and chases the
/// ball almost to the corner.
///
/// # The territory
///
/// A keeper defends the space in FRONT of his goal, and that space is
/// longer than it is wide: the angles that matter converge on the goal, so
/// the further upfield he goes the narrower the ground worth owning. A
/// sweeper-keeper 25 m out is central, covering the space behind his back
/// line; he is never 25 m out *and* 25 m across, because a ball out there
/// cannot be shot into the net he has left.
///
/// So the territory is an ELLIPSE centred on his goal — [`Self::off_line`]
/// deep, [`Self::off_centre`] wide — and [`Self::is_within`] is one test
/// against it rather than two independent ones. Both semi-axes are the
/// keeper's own: how far he comes is `rushing_out`, how wide he ranges is
/// `command_of_area`, which is the attribute whose entire meaning is the
/// size of the area he claims.
pub struct KeeperSweepLimit;

impl KeeperSweepLimit {
    /// A keeper who stays home. 120u = 15 m — the penalty spot.
    const LINE_KEEPER: f32 = 120.0;
    /// …and how much further a sweeper goes, out to 32 m.
    const SWEEPER: f32 = 136.0;

    /// ACROSS the goal, for a keeper who owns nothing. 88u = 11 m — a
    /// stride wider than his six-yard box, which is as far as a keeper who
    /// does not command his area ever gets from the middle of his goal.
    const LINE_KEEPER_WIDTH: f32 = 88.0;
    /// …and how much wider one who commands his box ranges: out to 168u
    /// (21 m), the corner of his own penalty area.
    ///
    /// NB this is the semi-axis, so it is only reached at zero depth. At
    /// the depth of the penalty spot the same keeper is held to about
    /// 17 m, which is what makes a corner — deep AND wide — unreachable
    /// for every keeper in the game while leaving the corner of the
    /// six-yard box comfortably inside for one who comes for crosses.
    const SWEEPER_WIDTH: f32 = 80.0;

    /// How far off his line this keeper is willing to be.
    pub fn off_line(rushing_out_profile: f32) -> f32 {
        Self::LINE_KEEPER + rushing_out_profile.clamp(0.0, 1.0) * Self::SWEEPER
    }

    /// …and how far across it, which is a different question and a
    /// different attribute. See the note on the struct.
    pub fn off_centre(command_of_area: f32) -> f32 {
        Self::LINE_KEEPER_WIDTH + command_of_area.clamp(0.0, 1.0) * Self::SWEEPER_WIDTH
    }

    /// How far off his line he currently is, along the goal-to-goal axis.
    pub fn distance_off_line(ctx: &StateProcessingContext) -> f32 {
        (ctx.player.position.x - ctx.ball().direction_to_own_goal().x).abs()
    }

    /// The two semi-axes of this keeper's territory: how deep, how wide.
    pub fn axes(prof: &GoalkeeperSkillProfile) -> (f32, f32) {
        (
            Self::off_line(prof.rushing_out_profile),
            Self::off_centre(prof.command_of_area),
        )
    }

    /// The widest territory ANY keeper in the game has.
    pub fn outermost() -> (f32, f32) {
        (Self::off_line(1.0), Self::off_centre(1.0))
    }

    /// …and the narrowest, which is the one an outside caller wants.
    ///
    /// The dispatcher's loose-ball override runs before a
    /// `StateProcessingContext` exists, so it cannot read the keeper's
    /// profile, and it FORCES a state — `GoalkeeperTakeBallState`, which
    /// then applies the keeper's own territory and hands him straight back
    /// if the ball is outside it.
    ///
    /// ⚠ So the bound here has to be inside every keeper's own, not
    /// outside it: `COMMIT < DISENGAGE` for the seventh time. Measured
    /// with [`Self::outermost`] here instead, the override forced him into
    /// `TakeBall` 338 times a match and **99% of those reversed inside
    /// 300 ms** — the two gates disagreeing about the same ball.
    ///
    /// A keeper who genuinely wants a ball in the outer part of his own
    /// ground does not need this override to get it; the doors in
    /// `Standing` and `ComingOut` are his, and they read his attributes.
    /// This is the safety net for a ball nobody is going for, and a safety
    /// net should be the conservative one.
    pub fn innermost() -> (f32, f32) {
        (Self::off_line(0.0), Self::off_centre(0.0))
    }

    /// How far out of the territory a point is, as a fraction of the way
    /// to its edge: 0 at the centre of his goal, 1 exactly on the
    /// boundary, above 1 outside it. The continuous form, so a caller can
    /// ask *how far* out he is rather than only whether he is.
    pub fn strain(goal: Vector3<f32>, at: Vector3<f32>, axes: (f32, f32)) -> f32 {
        let along = (at.x - goal.x) / axes.0.max(1.0);
        let across = (at.y - goal.y) / axes.1.max(1.0);
        (along * along + across * across).sqrt()
    }

    /// Has the ball left the ground he defends? The override that forces him
    /// onto a loose ball and the give-up that lets him go both ask this. A
    /// ground ball's landing point is where it is now, so a pass played to
    /// him reads as outside his ground until it arrives: the man it is meant
    /// for is meeting it wherever he stands.
    pub fn ball_has_left(
        player_id: u32,
        tick_context: &GameTickContext,
        goal: Vector3<f32>,
        axes: (f32, f32),
    ) -> bool {
        tick_context.ball.pass_target != Some(player_id)
            && Self::strain(goal, tick_context.positions.ball.landing_position, axes) > 1.0
    }

    /// Is he still inside the space he is prepared to defend?
    pub fn is_within(ctx: &StateProcessingContext, prof: &GoalkeeperSkillProfile) -> bool {
        Self::covers(ctx, ctx.player.position, prof)
    }

    /// …and the same question about a point he is thinking of going to.
    pub fn covers(
        ctx: &StateProcessingContext,
        point: Vector3<f32>,
        prof: &GoalkeeperSkillProfile,
    ) -> bool {
        Self::strain(ctx.ball().direction_to_own_goal(), point, Self::axes(prof)) <= 1.0
    }

    /// `point`, pulled back onto the boundary of his territory if it lies
    /// outside — the nearest place he is prepared to stand that is still
    /// on the line to where he wanted to go.
    ///
    /// This is what makes the give-up test a bound rather than a
    /// two-cycle: a keeper who is merely FORBIDDEN to be somewhere sets
    /// off for it anyway, trips the test, turns round, and does it again.
    /// One who is steered to the edge of his own ground simply stops
    /// there — which is also what a keeper looks like.
    pub fn contain(
        ctx: &StateProcessingContext,
        point: Vector3<f32>,
        prof: &GoalkeeperSkillProfile,
    ) -> Vector3<f32> {
        let goal = ctx.ball().direction_to_own_goal();
        let _ = goal;
        let strain = Self::strain(goal, point, Self::axes(prof));
        if strain <= 1.0 {
            return point;
        }
        goal + (point - goal) / strain
    }
}

/// Is the man on the ball actually THROUGH, or is there a defender
/// goal-side of him?
///
/// # Why this exists
///
/// A carrier 18 m out with two centre-backs in front of him is not a
/// keeper's problem — he is the defence's — and charging at him leaves an
/// empty goal to chip. The come-out trigger this replaced fired on any
/// carrier inside 18.75 m, and in doing so made `PreparingForSave`
/// unreachable for a carrier inside 12.5 m: the keeper committed, hit his
/// leash, ran home and committed again.
///
/// A keeper comes for the ball when there is nobody between him and it.
/// That is the whole of the real rule, and it is what this asks, for the
/// duel (`KeeperOneOnOne`) as for the smother.
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

    /// To end a duel the keeper is already in, a defender has to be in the
    /// carrier's path: 12u = 1.5 m. Narrower than the corridor that keeps
    /// the keeper home, so a man tracking the run alongside does not switch
    /// the duel off and on as he drifts across its edge.
    const IN_HIS_PATH: f32 = 12.0;

    /// Nobody goal-side of him: this one is the keeper's.
    pub fn is_through(ctx: &StateProcessingContext, carrier: &MatchPlayerLite) -> bool {
        Self::clear_within(ctx, carrier, Self::COVER_CORRIDOR)
    }

    /// Still through, for a duel already on: only a defender in his path
    /// ends it.
    pub fn still_through(ctx: &StateProcessingContext, carrier: &MatchPlayerLite) -> bool {
        Self::clear_within(ctx, carrier, Self::IN_HIS_PATH)
    }

    fn clear_within(
        ctx: &StateProcessingContext,
        carrier: &MatchPlayerLite,
        corridor: f32,
    ) -> bool {
        let goal = ctx.ball().direction_to_own_goal();
        let _ = goal;
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
            (rel - lane * along).magnitude() <= corridor
        })
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
    pub fn into_pitch(own_goal: Vector3<f32>, field_width: f32) -> f32 {
        if own_goal.x <= field_width * 0.5 {
            1.0
        } else {
            -1.0
        }
    }

    /// The `y`, at depth `at_x`, of the line he guards against a ball at
    /// `ball`: the bisector of the angle the two posts make at the ball.
    /// It lies nearer the near post than the line to the middle of the
    /// goal, and the gap widens as the ball goes wide; a keeper on the
    /// middle line leaves his near post open.
    pub fn guard_y(own_goal: Vector3<f32>, ball: Vector3<f32>, at_x: f32) -> f32 {
        let toward = |y: f32| {
            Vector3::new(own_goal.x - ball.x, y - ball.y, 0.0)
                .try_normalize(1e-3)
                .unwrap_or_else(Vector3::zeros)
        };
        let aim = if KeeperDebug::bisector_off() {
            toward(own_goal.y)
        } else {
            toward(own_goal.y - GOAL_WIDTH) + toward(own_goal.y + GOAL_WIDTH)
        };
        if aim.x.abs() > 1e-3 {
            ball.y + aim.y * (at_x - ball.x) / aim.x
        } else {
            own_goal.y
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
    /// `guard_y`. `read` is the keeper's positioning composite.
    ///
    /// ⚠ **`guard_y` is the `y` at HIS OWN DEPTH, not at the goal line.**
    /// This took the crossing at the line, which is the same point for a
    /// keeper standing on it and a different one for every keeper who is
    /// not — and both save paths adjudicate him at his own plane
    /// (`SaveModel::contact`), so a keeper who had come out was steered at
    /// one point and scored against another. [`KeeperShotDive::crossing_at`]
    /// is the projection, and its own note says why: aiming at the goal
    /// line "sends him backwards into his own net whenever he has come out
    /// to narrow the angle". It was wired into the dive and into
    /// `KeeperShotReaction::ticks_left` and left out of the two states he
    /// spends the flight ON HIS FEET in — which is where the error shows:
    /// measured over 200 matches, lateral error at the arrival was
    /// **1.88 m** when the ball found him still standing against 1.21 m
    /// when it found him already diving.
    ///
    /// Both callers project his BELIEF ([`KeeperShotReaction::crossing_y`])
    /// rather than the truth, so this changes where a keeper's read takes
    /// him and not how good the read is.
    /// **…and `keeper_depth` is a FLOOR on it, never a target to come back
    /// to.** He steps out to it; he does not step back.
    ///
    /// This depth is a pre-shot idea — where to stand to defend a strike
    /// from `shot_distance` — and it was being applied to a ball already in
    /// the air. A keeper who has come to meet a one-on-one is 10-16 m off
    /// his line, so the moment the shot left the foot he was handed a
    /// target 2-5 m off it and turned round: `KeeperShotReaction::on_foot`
    /// caps the whole steering VECTOR at a set keeper's shuffle, so almost
    /// all of the little he has left went backwards along the goal axis
    /// instead of across the ball's line. Measured, that is the difference
    /// between the two ways an arrival can find him — lateral error
    /// **1.74 m still on his feet against 1.13 m already diving** — and it
    /// grows with depth, which is the signature: 1.20 m at 4-7 m out,
    /// 1.51 m at 7-11 m, 1.61 m beyond that.
    ///
    /// A keeper set for a shot plants and covers across. Back-pedalling
    /// into your own goal at a struck ball is a mistake in football and it
    /// was a mistake here, and he could never complete it anyway — the
    /// retreat is 6-12 m and a shot's flight is a third of a second.
    ///
    /// The floor is the right shape rather than "hold whatever depth you
    /// have": a keeper ON his line must still step out to `MIN_DEPTH`, or
    /// the ball the physics save snaps onto him ends up inside his own
    /// goal frame, which is what this depth was introduced for.
    pub fn set_point(
        own_goal: Vector3<f32>,
        guard_y: f32,
        shot_distance: f32,
        field_width: f32,
        read: f32,
        keeper_depth: f32,
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
                .max(Self::MIN_DEPTH * 0.5)
                .max(keeper_depth.max(0.0));
        Vector3::new(
            own_goal.x + Self::into_pitch(own_goal, field_width) * depth,
            guard_y,
            0.0,
        )
    }

    /// The set point with no shot in flight, for a ball at `ball`: the set
    /// depth for a strike from there, on the bisector at that depth. Read
    /// at the depth it sends him to rather than at his own, so the point
    /// does not move as he steps to it.
    pub fn guard_point(
        own_goal: Vector3<f32>,
        ball: Vector3<f32>,
        field_width: f32,
        read: f32,
    ) -> Vector3<f32> {
        let mut point = Self::set_point(
            own_goal,
            own_goal.y,
            (ball - own_goal).magnitude(),
            field_width,
            read,
            0.0,
        );
        point.y = Self::guard_y(own_goal, ball, point.x);
        point
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
    /// The odds he gathers a ball he has got to.
    ///
    /// A ball he already owns is not a claim he can fumble: bending down to
    /// one under his own boot is near certain. For one he is meeting, what
    /// makes it hard is a man on him and pace on the ball, against his
    /// hands. Unpressured and slow it is essentially certain; pressed, with
    /// the ball coming at him quickly and poor hands, nearer four in five.
    /// (A flat 0.9 once had a keeper strolling onto a ball trickling across
    /// an empty six-yard box fumble one in ten.)
    pub fn gather_odds(ctx: &StateProcessingContext) -> f32 {
        let handling = (ctx.player.skills.goalkeeping.handling / 20.0).clamp(0.0, 1.0);
        if ctx.player.has_ball(ctx) {
            return (0.97 + handling * 0.03).min(1.0);
        }
        let press = KeeperFeetDecision::pressure(ctx);
        let pace = (ctx.tick_context.positions.ball.velocity.norm() / 2.0).clamp(0.0, 1.0);
        (0.995 - press * 0.10 - pace * 0.08 - (1.0 - handling) * 0.06).clamp(0.70, 0.999)
    }

    /// One tick of a gather he is trying on every tick the ball is in his
    /// reach: one attempt at [`Self::gather_odds`], spread over the ticks
    /// his gloves are on it, so trying again each tick does not compound it
    /// into a certainty.
    pub fn gathers_this_tick(ctx: &StateProcessingContext) -> bool {
        let prof = GoalkeeperSkillProfile::from_ctx(ctx);
        let per_tick = prof.per_tick_save(Self::gather_odds(ctx), Self::CONTACT_TICKS);
        ctx.context.rng.unit_f32() < per_tick
    }

    /// How long his gloves are on a ball he is gathering, in AI ticks.
    pub const CONTACT_TICKS: f32 = 14.0;

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

    /// …and how much of that edge his VOICE is worth.
    ///
    /// The claim was two fixed distances and **no keeper attribute at
    /// all**: the loudest, most commanding goalkeeper in the game took a
    /// ball in front of his own goal on exactly the same terms as a
    /// seventeen-year-old who says nothing. That is the single most
    /// visible thing communication does on a pitch — a keeper who shouts
    /// "keeper's!" gets a ball his defenders would otherwise have reached
    /// first, and a quiet one lets them run into each other over it.
    ///
    /// `GoalkeeperSkillProfile::communication` is the composite for it
    /// (`communication` 0.45 + `command_of_area` 0.30 + concentration +
    /// positioning), and until this it reached only the punch decision.
    /// `sc::gk_communication`, written whole for the same job, had **no
    /// live caller in the match engine at all**.
    ///
    /// ⚠ CENTRED on the measured population mean of that composite, and
    /// MULTIPLICATIVE, for the two reasons this file keeps repeating: an
    /// uncentred quality term silently recalibrates the claim radius
    /// rather than adding an axis to it, and an additive one into a
    /// distance that has to stay positive is rectified at the floor. Read
    /// the mean off the `KEEPER VOICE` block in `dev_match stats`, which
    /// prints it beside the raw attribute for this purpose.
    const POPULATION_VOICE: f32 = 0.390;
    /// How far the claim radius opens between the quietest keeper in the
    /// game and the most commanding. 0.70 is about ±10% for the realistic
    /// spread of the composite — a metre on a cross, which is the
    /// difference between taking it and watching a defender head it.
    const VOICE_SPREAD: f32 = 0.70;

    /// Is this keeper favourite for the loose ball in front of him?
    pub fn is_favourite(ctx: &StateProcessingContext) -> bool {
        // A DEAD ball is not a loose ball and he is not favourite for it —
        // nobody is, it belongs to whoever was awarded the restart. See
        // [`DeadBall`](crate::r#match::engine::ball::ball::DeadBall).
        //
        // The dispatcher refuses the touch either way, so this is not what
        // stops the ball moving; it is what stops him WALKING AT IT. All
        // three doors into a claim ask this question (`Standing`,
        // `PreparingForSave` and `Catching`'s own gate), and without it he
        // shuttles `Catching` → `Clearing` → `Standing` → `Catching` beside
        // a ball he can never have, at sprint pace, for as long as the
        // restart waits. Measured on a throw-in by the corner flag: nine
        // round trips in 120 ticks.
        //
        // The taker is not exempt: `tick_awaited_restart` hands him the
        // ball when he arrives, and a keeper who "claims" it on the way
        // would drag it to wherever he had got to.
        if DeadBall::is_dead(ctx.tick_context.ball.restart_taker) {
            return false;
        }
        let ball = ctx.tick_context.positions.ball.position;
        let aerial = (ball.z / AerialReach::STANDING).clamp(0.0, 1.0);
        // `from_ctx` is memoized per (player, tick-key), so the composite
        // costs a hash lookup on this per-tick path rather than a fresh
        // `SkillBands` build.
        let prof = GoalkeeperSkillProfile::from_ctx(ctx);
        let voice =
            (1.0 + (prof.communication - Self::POPULATION_VOICE) * Self::VOICE_SPREAD).max(0.2);
        let edge = (Self::HANDS_ADVANTAGE + Self::AERIAL_ADVANTAGE * aerial)
            * voice
            * KeeperAppetite::claim_reach(KeeperAppetite::of(ctx));
        let mine = (ball - ctx.player.position).magnitude() - edge;
        let favourite = !ctx
            .players()
            .opponents()
            .all()
            .any(|opp| (ball - opp.position).magnitude() < mine);
        // Only counted with the ball somewhere he could plausibly claim it
        // — the question is asked every tick from three different doors,
        // and a ball on the halfway line says nothing about his voice.
        #[cfg(feature = "match-logs")]
        if (ball - ctx.player.position).magnitude() < 120.0 {
            crate::mid_run_diag::KeeperVoiceDiag::note(
                ctx.player.skills.goalkeeping.communication,
                prof.communication,
                ball.z > KeeperAerialClaim::MIN_HEIGHT,
                favourite,
            );
        }
        favourite
    }
}

/// **What a keeper does with a ball at his FEET.**
///
/// # Why this exists
///
/// The engine had no answer to the commonest thing a goalkeeper does. Every
/// route that left him owning the ball on the floor — a save he came out
/// and smothered, a loose ball he won, a shot the physics handed him — went
/// straight to `Distributing`, which stands perfectly still for up to
/// twenty ticks hunting a pass and then hoofs it. Nothing anywhere asked the
/// question a real keeper answers in half a second: **pick it up.**
///
/// That matters because a ball at a keeper's feet is the most vulnerable
/// ball on the pitch. `check_ball_ownership` awards a contested ball to
/// whichever player within 5u has the better `calculate_tackling_score`,
/// and the goalkeeper is the worst tackler in the match by a distance — so
/// any forward who is standing next to him simply takes it, six yards from
/// an open goal. Measured over 12 matches: he was robbed off his feet 1.5
/// times a match, and on 84% of his foot-possession ticks the Laws would
/// have let him pick the ball up.
///
/// # The model
///
/// The Laws decide which pair of options he has — gather-or-play with the
/// gloves available, play-or-clear without — and continuous scores decide
/// between them, so a keeper slides from building out to picking it up as
/// the press arrives rather than flipping at a hard boundary. The inputs
/// are the ones he actually reads: how closed down he is, whether he has
/// the technique to play out, and how patient his side has been told to be.
pub struct KeeperFeetDecision;

/// What to do with a ball on the floor that is already his.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FeetChoice {
    /// Bend down and take it in his hands. Ends the phase: nobody can
    /// touch it after that. Only ever offered when handling is legal.
    Gather,
    /// Keep it on the floor and build — the modern keeper's first choice
    /// when nobody is near him and he can pass.
    PlayOut,
    /// Get rid. Under pressure with the hands barred there is nothing else
    /// to do, and it is what a keeper receiving a back-pass with a forward
    /// closing actually does.
    Clear,
}

impl KeeperFeetDecision {
    /// How near an opponent has to be to count as full pressure. 40u = 5 m
    /// — one stride and he is on you, which is exactly when a keeper stops
    /// trying to play and picks the ball up.
    const PRESS_RANGE: f32 = 40.0;
    /// How wide he looks for bodies committing into his area.
    const CROWD_RANGE: f32 = 130.0;

    /// The decision, for a ball this keeper already owns on the floor.
    pub fn choose(ctx: &StateProcessingContext) -> FeetChoice {
        if !ctx.ball().handling_verdict().is_legal() {
            return Self::without_hands_choice(ctx);
        }

        let gk = &ctx.player.skills.goalkeeping;
        let short_skill = ((gk.passing + gk.first_touch) / 40.0).clamp(0.0, 1.0);
        let composure = (ctx.player.skills.mental.composure / 20.0).clamp(0.0, 1.0);
        let press = Self::pressure(ctx);
        let patience = ctx.team().build_up_patience().clamp(0.0, 1.0);

        // Picking it up is the safe, always-available option, so it is the
        // baseline the other one has to beat. Pressure is the dominant
        // term by design — that IS the reason a keeper stops playing and
        // takes it in his hands — and a keeper who cannot pass reaches for
        // it sooner.
        let gather = 0.55 + press * 1.05 + (1.0 - short_skill) * 0.35 - patience * 0.30;
        // Playing out needs space AND the technique to use it.
        let play_out =
            0.45 + short_skill * 0.55 + composure * 0.20 + patience * 0.35 - press * 1.30;

        if play_out > gather {
            FeetChoice::PlayOut
        } else {
            FeetChoice::Gather
        }
    }

    /// The same decision with the gloves barred — a back-pass, a second
    /// touch, or a ball that has rolled out of his area. Play or clear.
    fn without_hands_choice(ctx: &StateProcessingContext) -> FeetChoice {
        let gk = &ctx.player.skills.goalkeeping;
        let short_skill = ((gk.passing + gk.first_touch) / 40.0).clamp(0.0, 1.0);
        let composure = (ctx.player.skills.mental.composure / 20.0).clamp(0.0, 1.0);
        let press = Self::pressure(ctx);
        let play_out = 0.55 + short_skill * 0.50 + composure * 0.25 - press * 1.35;
        if play_out > 0.5 {
            FeetChoice::PlayOut
        } else {
            FeetChoice::Clear
        }
    }

    /// The state to hand a foot possession to when the hands are barred.
    /// Split out because `GoalkeeperHoldingState` needs the answer as a
    /// state rather than as a choice. A goal kick he has been handed is
    /// not a choice at all: it is the kick he placed it for.
    pub fn without_hands(ctx: &StateProcessingContext) -> GoalkeeperState {
        if let Some(kick) = KeeperGoalKick::release_state(ctx) {
            return kick;
        }
        match Self::without_hands_choice(ctx) {
            FeetChoice::Clear => GoalkeeperState::Clearing,
            _ => GoalkeeperState::Distributing,
        }
    }

    /// The whole decision as a state, for the sites that just want to be
    /// told where to go next.
    pub fn state_for(ctx: &StateProcessingContext) -> GoalkeeperState {
        if let Some(kick) = KeeperGoalKick::release_state(ctx) {
            return kick;
        }
        match Self::choose(ctx) {
            FeetChoice::Gather => GoalkeeperState::PickingUpBall,
            FeetChoice::PlayOut => GoalkeeperState::Distributing,
            FeetChoice::Clear => GoalkeeperState::Clearing,
        }
    }

    /// How closed down he is, 0..1.
    ///
    /// The nearest man dominates — one striker two metres away is the
    /// whole of the pressure a keeper feels — with a smaller term for a
    /// box filling up behind him.
    pub fn pressure(ctx: &StateProcessingContext) -> f32 {
        let nearest = ctx
            .players()
            .opponents()
            .nearby(Self::CROWD_RANGE)
            .map(|o| (o.position - ctx.player.position).magnitude())
            .fold(f32::MAX, f32::min);
        let close = if nearest.is_finite() {
            (1.0 - nearest / Self::PRESS_RANGE).clamp(0.0, 1.0)
        } else {
            0.0
        };
        let crowd = (ctx.players().opponents().nearby(Self::CROWD_RANGE).count() as f32 / 3.0)
            .clamp(0.0, 1.0);
        (close * 0.80 + crowd * 0.35).clamp(0.0, 1.0)
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
    ///
    /// Public because it is the ONE line between the two models, and the
    /// loose-ball dispatcher has to read it too: below it a keeper may be
    /// forced into the ground chase, above it the ball is this claim's and
    /// nothing else's. Two numbers for one boundary is how a ball ends up
    /// belonging to both models or to neither.
    pub const MIN_HEIGHT: f32 = 1.35;
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

    /// Catch it or punch it, chosen as he commits: on the odds of holding it
    /// with his hands and nerve in the traffic around it, and the pace on it
    /// — by a keeper with the command of his area to punch at all, at a ball
    /// his fist can play.
    pub fn intent(ctx: &StateProcessingContext, claim: &AerialClaim) -> ClaimIntent {
        let prof = GoalkeeperSkillProfile::from_ctx(ctx);
        let speed = ctx.tick_context.positions.ball.velocity.norm();
        let situation = ClaimSituation::of(
            claim.height,
            claim.traffic,
            speed,
            ctx.tick_context.ball.pending_pass_was_cross,
        );
        let minute = sc::minute_from_ms(ctx.context.total_match_time);
        let hold =
            1.0 - GkClaimContest::failure(ctx.player, ClaimIntent::Catch, &situation, minute);
        let power = ((speed - 1.2) / 2.0).clamp(0.0, 1.0);
        if claim.height >= GoalkeeperPunchingState::FLOOR
            && prof.should_punch(hold, claim.traffic, power)
        {
            ClaimIntent::Punch
        } else {
            ClaimIntent::Catch
        }
    }

    /// The reach the aerial contest grants him: a stride away, and up to the
    /// leap he commits to. One reach for the commit and the contest, or he
    /// comes for a ball the contest then says he cannot touch.
    pub fn contest_reach(keeper: &MatchPlayer, ball: Vector3<f32>) -> bool {
        (ball - keeper.position).xy().norm() <= MAX_OWNER_TRACK_DISTANCE
            && ball.z <= Self::leap_ceiling(keeper.skills.physical.jumping)
    }

    /// Can he play the ball where it is now? A step and his arms across the
    /// grass, and up to his leap off the ground or his reach standing on it:
    /// the envelope he committed to coming for it, and the one the aerial
    /// contest lets him play it in.
    pub fn within_reach(keeper: &MatchPlayer, ball: Vector3<f32>) -> bool {
        let ceiling = if keeper.is_airborne() {
            Self::leap_ceiling(keeper.skills.physical.jumping)
        } else {
            Self::standing_ceiling()
        };
        (ball - keeper.position).xy().norm() <= AerialClaim::CONTACT_RANGE && ball.z <= ceiling
    }

    /// Book a claim the keeper has just decided to go for, and what he means
    /// to do with it. The read calls it at every door onto a claim, so a
    /// claim he runs out to and then meets passes it twice: it counts once,
    /// when the census opens it.
    #[allow(unused_variables)]
    pub fn note_start(ctx: &StateProcessingContext, claim: &AerialClaim, intent: ClaimIntent) {
        #[cfg(feature = "match-logs")]
        if KeeperCensus::open_claim_in(ctx) {
            KeeperCensus::claim_action_in(
                ctx,
                match intent {
                    ClaimIntent::Catch => ClaimAction::Catch,
                    ClaimIntent::Punch => ClaimAction::Punch,
                },
            );
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
        // A delivery an engine-level aerial contest has awarded to
        // somebody else is not his to come for: the keeper's claim chance was PRICED
        // INTO that contest (`gk_command` shrinks the attacker's win,
        // and the cross contest's own GK branch can take the ball off
        // both duellists). Coming for it anyway re-rolled his part —
        // the flight census showed corner-won deliveries being touched
        // out of the air before arrival while the winner's header
        // vanished. He stays for the strike; the save pipeline is where
        // he answers it.
        if ctx
            .tick_context
            .ball
            .aerial_contest_winner
            .is_some_and(|winner| winner != ctx.player.id)
        {
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

/// What happened when the keeper went down at a striker's feet.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SmotherOutcome {
    /// Both gloves on it. The ball is his and he curls up around it.
    Gathered,
    /// He got something in the way — a hand, a shin, his chest — and the
    /// ball has gone. `Vector3` is the velocity he knocked it away with.
    Blocked(Vector3<f32>),
    /// He missed the ball and took the man. Inside his own area that is a
    /// penalty, which is what makes coming out a decision.
    Fouled(FoulSeverity),
    /// The striker went round him. The keeper is on the floor and out of
    /// the move, which is the price of committing.
    Beaten,
}

/// The moment a keeper commits at a carrier's feet, and what it costs him.
#[derive(Debug, Clone, Copy)]
pub struct SmotherAttempt {
    /// The man he is going through.
    pub carrier_id: u32,
    /// Where the ball is — where he is throwing himself.
    pub ball: Vector3<f32>,
    /// How far out at the end of his reach the ball is, 0..1. A ball
    /// under his own body is a smother; one at fingertip range is a
    /// gamble, and the contest is priced accordingly.
    pub stretch: f32,
    /// Whether he may use his hands here. Outside his own area he can
    /// still block the shot with his body — he simply cannot gather it.
    pub hands: bool,
}

/// **A keeper does not stand and watch a striker walk the ball round him.**
///
/// # Why this exists
///
/// The engine had no mechanism for the single most-watched moment in
/// football. A forward carrying the ball into the box was met by a keeper
/// who did exactly one thing: `ComingOut` saw an opponent "with control and
/// very close" and handed him to `PreparingForSave`, i.e. he stopped
/// running, stood up, and set himself for a shot from six yards.
///
/// Nothing else could fire either. `ComingOut::should_dive` returns `false`
/// below `LOOSE` (0.7 u/tick) and a dribbled ball travels at the DRIBBLER'S
/// pace — a striker sprinting at 7 m/s moves it at ~0.56 u/tick — so the
/// dive gate was shut for every carrier in the game by construction. And
/// both claim gates refuse an owned ball on purpose ("he does not tackle
/// with his hands", see [`KeeperBallClaim`]), which is right for a ball
/// somebody is shielding twenty metres out and wrong for one at a striker's
/// feet in the six-yard box, where taking it off him is the entire job.
///
/// # The model
///
/// The same shape as a defender's challenge (`attempt_block_tackle`), for
/// the same reason: it is a duel, it has a cost, and it can be a foul.
///   * He commits when the ball is inside his own spread and the carrier is
///     in front of him — never from behind, which is a foul and nothing
///     else.
///   * The contest is `one_v_one` and his nerve against the carrier's
///     `dribble_attack`, worsened by how far out at the end of his reach
///     the ball is.
///   * Winning splits into GATHERING it (hands, if they are legal here) and
///     KNOCKING IT AWAY, on `handling`. Both are what the report asked for
///     and they are genuinely different outcomes: one ends the move, the
///     other leaves a loose ball in the box.
///   * Losing puts him on the floor with the striker past him — and
///     occasionally brings the man down.
pub struct KeeperSmother;

impl KeeperSmother {
    /// Where a ball he knocks away comes to rest, in units: 2 m for a keeper
    /// who cannot direct a parry, 5 m for one who can.
    const KNOCK_REST: (f32, f32) = (16.0, 40.0);
    /// …rising off him at this, in m/tick: it has come off a body, not a boot.
    const KNOCK_LIFT: f32 = 0.02;
    /// How far a keeper's spread reaches, before skill. 12u = 1.5 m —
    /// arms, chest and the ground he covers going down.
    pub const SPREAD_BASE: f32 = 12.0;
    /// …and what agility and one-on-one technique add, up to what the ball
    /// can be granted at (see [`Self::reach`]).
    pub const SPREAD_SKILL: f32 = 12.0;
    /// A ball higher than this is not at anybody's feet — that is a
    /// [`KeeperAerialClaim`], and diving under it achieves nothing. 1.1 m.
    const FEET_HEIGHT: f32 = 1.1;
    /// He has to be goal-side of the carrier, or he is diving in from
    /// behind. Measured as the dot of (goal − carrier) against
    /// (keeper − carrier), so this is a cosine.
    const IN_FRONT: f32 = 0.15;
    /// How far inside his own spread the ball has to be before he commits,
    /// as a fraction of it. See the note at the gate itself: firing at the
    /// boundary makes every smother a fingertip lunge.
    const COMMIT_SHARE: f32 = 0.72;
    /// …and how close it has to be for everything else to stop mattering.
    /// 10u = 1.25 m — touching distance. A ball this near a goalkeeper is
    /// his whether or not a defender is goal-side of the man on it and
    /// whichever way that man happens to be running.
    const MINE_REGARDLESS: f32 = 10.0;
    /// How near his own goal the ball has to be before he will go to
    /// ground for it at all. 130u ≈ 16 m — the penalty area.
    ///
    /// Not the same question as "is the carrier close to ME". A keeper who
    /// has swept thirty metres upfield and finds himself alongside a man
    /// on the ball does not spread himself at his feet; he stays up,
    /// because going to ground out there leaves an empty net behind him.
    pub const DANGER_DEPTH: f32 = 130.0;
    /// **The keeper's structural edge in this particular duel.**
    ///
    /// Not a population offset between two composites (that is what
    /// `SaveModel::CONTEST_BALANCE` is, and it is 0.08). This is a real
    /// advantage neither composite encodes: the keeper may use his HANDS
    /// and he only has to touch the ball, while the striker has to keep
    /// it and put it somewhere. A 1-v-1 is a chance, not a certainty —
    /// real conversion from a genuine one-on-one runs about 35-40%, so the
    /// keeper wins the moment rather more often than he loses it.
    ///
    /// Derived, not chosen. With the two composites left to fight it out
    /// on their own the measured win rate over 200 matches was **23%**,
    /// which had keepers being dribbled round three times out of four;
    /// 0.26 took it to 45% and 0.34 to 52%. Re-derive it from the
    /// GOALKEEPER ACTION CENSUS
    /// (`gathered + knocked away` against `smothers`) whenever either
    /// composite moves — the win rate is a clean statistic there, ~1900
    /// duels over 200 matches, and far less noisy than the save rate it
    /// feeds into.
    const HANDS_ADVANTAGE: f32 = 0.34;
    /// How much being at the end of his reach costs him. A ball under his
    /// own body is his; one he is fingertipping at is a gamble.
    const STRETCH_COST: f32 = 0.20;
    /// How sharply the duel separates the two. Matches the defender's
    /// tackle sigmoid, and the clamp keeps even the worst mismatch from
    /// becoming a certainty either way.
    const CONTEST_SPREAD: f32 = 2.6;

    /// Is this the moment? `None` means he keeps coming (or keeps
    /// standing) — it is not his ball to go to ground for yet.
    pub fn assess(ctx: &StateProcessingContext) -> Option<SmotherAttempt> {
        // Nobody smothers a dead ball. See [`KeeperSetPieceStance`].
        if KeeperSetPieceStance::pending(ctx).is_some() {
            return None;
        }
        // Somebody else's ball, in somebody else's feet.
        let carrier = ctx.players().opponents().with_ball().next()?;
        let ball = ctx.tick_context.positions.ball.position;
        if ball.z > Self::FEET_HEIGHT {
            return None;
        }
        // Near his own goal. A keeper spreading himself at the halfway
        // line is not defending anything, and one who does it having swept
        // twenty-five metres out leaves an empty net behind him.
        let goal = ctx.ball().direction_to_own_goal();
        let _ = goal;
        if (ball.x - goal.x).abs() > Self::DANGER_DEPTH {
            return None;
        }

        // ONE COMMITMENT PER ATTACK. A defender's challenge needs a
        // cooldown because `Tackling` can roll again on the very next
        // tick; a smother needs one for a subtler reason — resolving it
        // puts the keeper in `Diving`, he is up again 0.44 s later, and if
        // the striker still has the ball beside him he commits again on
        // that tick. Measured without it: **12 smothers per keeper per
        // match** against a real one or two, most of them the same 1-v-1
        // being fought three or four times over.
        //
        // The keeper's own five seconds, NOT the defender's thirty — see
        // `MatchPlayer::start_keeper_cooldown`.
        if !ctx.player.can_attempt_tackle() {
            return None;
        }

        let prof = GoalkeeperSkillProfile::from_ctx(ctx);
        let reach = Self::reach(&prof);
        let range = (ball - ctx.player.position).magnitude();
        if range > reach {
            return None;
        }

        // ⚠ HE DOES NOT GO ON THE FIRST TICK IT IS IN RANGE.
        //
        // This is checked every tick, so firing the moment the ball crosses
        // the edge of his spread means every smother in the game is taken
        // at FULL STRETCH — `stretch` was pinned at ~1.0 across the whole
        // population, the duel carried its maximum penalty every time, and
        // the keeper lost three of every four. A keeper picks his moment:
        // he goes when the ball is properly inside his reach…
        let committed = range <= reach * Self::COMMIT_SHARE;
        // …or when it is now or never, because the man is level with him
        // and one touch from being past. Waiting for a better moment that
        // is not coming is how a keeper gets rounded.
        let keeper_depth = (ctx.player.position.x - goal.x).abs();
        let last_chance = (carrier.position.x - goal.x).abs() <= keeper_depth;
        if !committed && !last_chance {
            return None;
        }

        // A man ATTACKING the goal. A striker shielding the ball with his
        // back to it, or knocking it square in the corner of the area, is
        // not a 1-v-1 — going to ground at him wins nothing and costs the
        // goal behind. At touching distance it stops mattering: the ball is
        // his whatever he is doing with it.
        let carrier_run = ctx.tick_context.positions.players.velocity(carrier.id);
        let bearing_down = range <= Self::MINE_REGARDLESS
            || (goal - carrier.position)
                .try_normalize(1e-3)
                .zip(carrier_run.try_normalize(1e-3))
                .is_none_or(|(lane, run)| lane.dot(&run) > 0.0);
        if !bearing_down {
            return None;
        }

        // Somebody else's job. A striker with the ball in a crowded box and
        // a defender goal-side of him belongs to the defender: a keeper who
        // dives in there takes his own man out and leaves an empty net. The
        // exception is a ball at touching distance, which is his whatever
        // the traffic — the same distinction `KeeperCarrierThreat` draws
        // for a defender who has dropped onto his own line.
        if range > Self::MINE_REGARDLESS && !KeeperCarrierThreat::is_through(ctx, &carrier) {
            return None;
        }

        // In front of him, not behind. A keeper who has been beaten does
        // not get to reach back through the man and take the ball; that
        // is a penalty in every stadium in the world.
        let to_goal = goal - carrier.position;
        let to_keeper = ctx.player.position - carrier.position;
        if let (Some(lane), Some(toward)) =
            (to_goal.try_normalize(1e-3), to_keeper.try_normalize(1e-3))
            && lane.dot(&toward) < Self::IN_FRONT
        {
            return None;
        }

        Some(SmotherAttempt {
            carrier_id: carrier.id,
            ball,
            stretch: (range / reach.max(1e-3)).clamp(0.0, 1.0),
            hands: ctx.ball().handling_verdict().is_legal(),
        })
    }

    /// How far this keeper spreads himself, in game units: no further than
    /// the ball can be granted to him, or a smother he wins at full stretch
    /// is one the possession grant refuses.
    pub fn reach(prof: &GoalkeeperSkillProfile) -> f32 {
        (Self::SPREAD_BASE
            + (prof.dive_reach * 0.6 + prof.one_v_one * 0.4).clamp(0.0, 1.0) * Self::SPREAD_SKILL)
            .min(MAX_OWNER_TRACK_DISTANCE)
    }

    /// Resolve the attempt and hand back the transition it produces.
    ///
    /// One place, because both states that can arrive at a 1-v-1 have to
    /// end it the same way — and because the state he ends in is part of
    /// the outcome: whatever happens to the ball, the keeper is on the
    /// floor afterwards, which is the cost of having come.
    pub fn commit(ctx: &StateProcessingContext, attempt: &SmotherAttempt) -> StateChangeResult {
        let outcome = Self::resolve(ctx, attempt);
        #[cfg(feature = "match-logs")]
        {
            KeeperActionDiag::note(11);
            KeeperActionDiag::note(match outcome {
                SmotherOutcome::Gathered => 12,
                SmotherOutcome::Blocked(_) => 13,
                SmotherOutcome::Fouled(_) => 14,
                SmotherOutcome::Beaten => usize::MAX,
            });
        }
        let mut result = match outcome {
            SmotherOutcome::Gathered => StateChangeResult::with_goalkeeper_state_and_event(
                GoalkeeperState::Diving,
                Event::PlayerEvent(PlayerEvent::CaughtBall(ctx.player.id)),
            ),
            SmotherOutcome::Blocked(away) => StateChangeResult::with_goalkeeper_state_and_event(
                GoalkeeperState::Diving,
                Event::PlayerEvent(PlayerEvent::ClearBall(ctx.player.id, away)),
            ),
            SmotherOutcome::Fouled(severity) => StateChangeResult::with_goalkeeper_state_and_event(
                GoalkeeperState::Diving,
                Event::PlayerEvent(PlayerEvent::CommitFoul(FoulCommitted {
                    fouler: ctx.player.id,
                    fouled: attempt.carrier_id,
                    severity,
                    source: FoulSource::KeeperSmother,
                    challenge: None,
                })),
            ),
            SmotherOutcome::Beaten => {
                StateChangeResult::with_goalkeeper_state(GoalkeeperState::Diving)
            }
        };
        result.start_keeper_cooldown = true;
        result
    }

    /// Play it out. Consumes RNG — call once, on the tick he commits.
    pub fn resolve(ctx: &StateProcessingContext, attempt: &SmotherAttempt) -> SmotherOutcome {
        let prof = GoalkeeperSkillProfile::from_ctx(ctx);
        let rng = &ctx.context.rng;
        let minute = sc::minute_from_ms(ctx.context.total_match_time);

        // The carrier's side of the duel — the same composite the
        // defenders' challenge is scored against, so a forward who is hard
        // to tackle is hard to smother.
        let carry = match ctx.context.players.by_id(attempt.carrier_id) {
            Some(att) => sc::dribble_attack(att, minute),
            None => {
                let players = ctx.player();
                let s = players.skills(attempt.carrier_id);
                (sc::n(s.technical.dribbling) + sc::n(s.physical.agility)) * 0.5
            }
        };

        // And the keeper's. `one_v_one` leads it — this is the situation
        // that attribute is named after and, before this, the only thing
        // that ever read it was a probability curve applied to shots.
        let nerve = (ctx.player.skills.mental.bravery / 20.0).clamp(0.0, 1.0);
        let keeper = (prof.one_v_one * 0.44
            + prof.dive_reach * 0.18
            + nerve * 0.14
            + prof.positioning * 0.12
            + prof.handling_profile * 0.12)
            .clamp(0.0, 1.0);

        let edge = keeper - carry + Self::HANDS_ADVANTAGE - attempt.stretch * Self::STRETCH_COST;
        let win = (0.5 + edge * Self::CONTEST_SPREAD * 0.5).clamp(0.10, 0.86);

        if rng.random::<f32>() < win {
            // He got there. Whether it stays there is `handling` — and
            // outside his own area it never does, because the only thing
            // he is allowed to put in the way is his body.
            let hold = if attempt.hands {
                (0.26 + prof.handling_profile * 0.48 - attempt.stretch * 0.22).clamp(0.05, 0.80)
            } else {
                0.0
            };
            if rng.random::<f32>() < hold {
                return SmotherOutcome::Gathered;
            }
            return SmotherOutcome::Blocked(Self::knock_away(ctx, attempt, &prof));
        }

        // Beaten. Sometimes he takes the man with him — RARELY.
        //
        // ⚠ The rate here is a penalty rate, and it has to be read against
        // the whole match rather than against this duel. The engine gives
        // 0.25 penalties a match without a keeper ever conceding one, which
        // is already the top of the real 0.25-0.30 band; at the first
        // plausible-looking per-loss rate (~9%) the keepers alone added
        // **0.28 a match** and took the total to 0.53. A keeper going down
        // at a striker's feet leads with his hands on the ball and mostly
        // gets something or nothing; the penalty is the exception, and the
        // constant has to be sized as one.
        let composure = (ctx.player.skills.mental.composure / 20.0).clamp(0.0, 1.0);
        let foul = (0.006 + (1.0 - composure) * 0.010 + attempt.stretch * 0.009)
            * (1.0 - prof.one_v_one * 0.5);
        if rng.random::<f32>() < foul.clamp(0.002, 0.022) {
            // A keeper's mistimed dive is a trip, not violence. Reckless
            // only when he has gone through the man at pace.
            let severity = if attempt.stretch > 0.7 && rng.random::<f32>() < 0.10 {
                FoulSeverity::Reckless
            } else {
                FoulSeverity::Normal
            };
            return SmotherOutcome::Fouled(severity);
        }

        SmotherOutcome::Beaten
    }

    /// Where a blocked ball goes: off him, away from the goal, and to one
    /// side. A keeper's block is not a clearance — it comes off whatever
    /// he got in the way, so it is neither hard nor aimed, and the loose
    /// ball it leaves in the box is part of the outcome.
    fn knock_away(
        ctx: &StateProcessingContext,
        attempt: &SmotherAttempt,
        prof: &GoalkeeperSkillProfile,
    ) -> Vector3<f32> {
        let goal = ctx.ball().direction_to_own_goal();
        let _ = goal;
        let out = (attempt.ball - goal)
            .try_normalize(1e-3)
            .unwrap_or_else(|| {
                Vector3::new(ctx.player.side.map_or(1.0, |s| s.forward_dir_x()), 0.0, 0.0)
            });
        // A keeper who can direct a parry puts it wide rather than back
        // into the middle; one who cannot leaves it where it fell.
        let sideways =
            ctx.context.rng.range_f32(-1.0, 1.0).signum() * (0.35 + prof.parry_control * 0.75);
        let lateral = Vector3::new(-out.y, out.x, 0.0) * sideways;
        // …and it comes to rest within his own few metres, where the man
        // he went through can still chase it; at the 0.8-1.7 u/tick it used
        // to leave at it rolled 60-130 m.
        let rest =
            Self::KNOCK_REST.0 + prof.parry_control * (Self::KNOCK_REST.1 - Self::KNOCK_REST.0);
        let speed = BallRoll::speed_to_rest_at_from(rest, attempt.ball.z, Self::KNOCK_LIFT);
        let mut away = (out + lateral).try_normalize(1e-3).unwrap_or(out) * speed;
        away.z = Self::KNOCK_LIFT;
        away
    }
}

/// **A man running at him with the ball, and nobody in the way.**
///
/// # Why this exists
///
/// [`KeeperSmother`] gave the engine a way to take the ball off a carrier's
/// feet, and it fires far less often than the situation occurs, for a reason
/// that has nothing to do with the smother itself: **the keeper never gets
/// close enough to try.** Measured off a recording, over 73 strict 1-v-1s in
/// one match — a central carrier inside the box, running at goal, with no
/// defender nearer the ball than he is:
///
/// | | |
/// |---|---|
/// | keeper→ball at the start | median **13.6 m** |
/// | closest he got in the next 2.5 s | median **9.7 m** |
/// | inside 3 m, which is his own spread | 21 / 73 |
/// | left the ground for it | **3 / 73** |
///
/// The mechanism is [`GoalkeeperPreparingForSaveState::velocity`]'s
/// no-shot-cached branch: with nothing struck it steers to a point on the
/// goal→ball line `18-32 u` out, i.e. **two to four metres off his line
/// whatever the carrier does**. Traced through one episode, a striker
/// dribbles from 8.7 m to 2.8 m from goal and the keeper holds station
/// 5.22 m away for two full seconds, watching. `KeeperSmother::assess`
/// wants the ball inside 1.5-3 m; it was never going to see it.
///
/// # The model
///
/// He stands off the BALL, not off his line. The gap is his own spread plus
/// a stride — near enough that one step is a smother — and it is bounded by
/// how far he is prepared to come at all, so a carrier still twenty-five
/// metres out brings him to the edge of his area rather than out to meet
/// him. As the striker closes, the gap closes with him, and the moment the
/// ball is inside the spread the duel resolves.
pub struct KeeperOneOnOne;

/// Why a one-on-one he is in has lapsed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DuelLapse {
    /// Nobody of theirs is on the ball: a pass, a tackle, a dead ball.
    OffTheBall,
    /// A defender has got into the man's path.
    Covered,
    /// The man has turned away from goal.
    TurnedAway,
}

impl KeeperOneOnOne {
    /// How near his own goal the duel has to be before he plays it as one.
    /// The same 130u the smother uses — the penalty area — and for the same
    /// reason: a keeper who comes to meet a man at the halfway line has left
    /// an empty net behind him.
    pub const DANGER_DEPTH: f32 = KeeperSmother::DANGER_DEPTH;
    /// How much further off the ball he stands than he can reach.
    ///
    /// **Zero, and that is the point.** The gap he holds and the spread he
    /// can cover are ONE budget: stand further off than he can reach and
    /// the duel he came out for can never resolve, which is exactly the
    /// 5.22 m stand-off this replaces. He sets himself at the edge of his
    /// own reach and the striker's next touch decides it — `assess` wants
    /// the ball properly inside the spread (`COMMIT_SHARE`) before he goes,
    /// so a keeper at the edge of it is one push away from a smother and no
    /// closer.
    const MARGIN: f32 = 0.0;
    /// How far toward the ball he will come, as a share of its distance from
    /// his own goal. Without it the stand-off alone sends him out to meet a
    /// breakaway from the halfway line.
    ///
    /// The gap he holds is `max(spread, (1 − ADVANCE) × distance)`, so it
    /// stops being the share and starts being the spread — the point at
    /// which the duel becomes reachable at all — once the ball is inside
    /// `spread / (1 − ADVANCE)`, about 5.9 m of goal. That is the six-yard
    /// box, and it is where a keeper spreads himself.
    ///
    /// **Measured, `stats 200 14 14`, against the `OF_KEEPER_HOLD` control
    /// on the same binary.** The share is the one number in this model with
    /// a scoreline attached, so it was swept rather than chosen:
    ///
    /// | share | goals/match | saves/on-target | penalties | smothers/keeper |
    /// |---|---|---|---|---|
    /// | control | 5.32, 5.24 | 57.7, 57.1 | 0.31, 0.23 | 4.09, 4.26 |
    /// | 0.50 | 4.66, 4.88 | 57.9, 58.3 | 0.32, 0.32 | 4.56, 4.75 |
    /// | **0.62** | **4.55, 4.64** | **59.0, 58.4** | 0.25, 0.27 | 4.85 |
    /// | 0.75 | 4.58 | 57.6 | 0.25 | — |
    ///
    /// It flattens at 0.62 and the save rate turns over past it, which is
    /// the point at which coming further starts costing him more than it
    /// buys. ⚠ The save rate is a DENOMINATOR here as much as a skill —
    /// chances that end as a duel at his feet leave the shot sample
    /// entirely — so read the goals column first.
    pub const ADVANCE: f32 = 0.62;
    /// And he never backs onto his own line to hold the gap. 8u = 1 m.
    const MIN_DEPTH: f32 = 8.0;

    /// The A/B control for the whole model, in the pattern
    /// [`KeeperShotReaction::servo`] documents: coming out changes both the
    /// chances he stops and the ones he lets in, so "was it worth it?" cannot
    /// be read off the diff, and it must not be answered by checking out an
    /// older revision — the working tree moves under you. Debug
    /// infrastructure; do not remove.
    pub fn held_back() -> bool {
        use std::sync::OnceLock;
        static HELD: OnceLock<bool> = OnceLock::new();
        *HELD.get_or_init(|| std::env::var("OF_KEEPER_HOLD").is_ok())
    }

    /// How far from him a man through on goal is met, before his appetite
    /// for risk widens or narrows it. 150u ≈ 19 m: the carrier is into the
    /// ground the keeper is responsible for.
    const MEETING_RANGE: f32 = 150.0;
    /// He stays in the duel while the man is not running away from goal
    /// by more than this (a cosine). Looser than the zero the duel is
    /// entered on, so a stutter in the carrier's run does not end it.
    const STILL_COMING: f32 = -0.5;

    /// The man he is in a duel with, if he is in one: a carrier running at
    /// goal with nobody between them, inside the penalty area's depth or
    /// close enough to be met.
    pub fn duel(ctx: &StateProcessingContext) -> Option<MatchPlayerLite> {
        if Self::held_back() {
            return None;
        }
        // A taker standing over a dead ball is not a man running at him.
        // The `bearing_down` test below is vacuously TRUE of a body with
        // no velocity, so a penalty taker placing the ball read as a
        // breakaway and pulled the keeper 6.8 m off a line Law 14 says
        // he must stay on. See [`KeeperSetPieceStance`].
        if KeeperSetPieceStance::pending(ctx).is_some() {
            return None;
        }
        let carrier = ctx.players().opponents().with_ball().next()?;
        if ctx.ball().is_held_by_opponent_goalkeeper() {
            return None;
        }
        let ball = ctx.tick_context.positions.ball.position;
        let goal = ctx.ball().direction_to_own_goal();
        let _ = goal;
        let risk = GoalkeeperSkillProfile::from_ctx(ctx)
            .eccentricity
            .clamp(0.0, 1.0)
            - 0.5
            + KeeperAppetite::rush_risk(KeeperAppetite::of(ctx));
        let in_range = (ball.x - goal.x).abs() <= Self::DANGER_DEPTH
            || ctx.ball().distance() < Self::MEETING_RANGE * (1.0 + risk * 0.4);
        if !in_range || Self::run_at_goal(ctx, &carrier) <= 0.0 {
            return None;
        }
        // …and nobody between the two of them. The same question the sweep
        // asks, so a covered carrier stays the defence's problem.
        KeeperCarrierThreat::is_through(ctx, &carrier).then_some(carrier)
    }

    /// Why a duel he is already in has lapsed, or `None` while it stands:
    /// the man still on the ball, still through, and not running away from
    /// goal. Looser than [`Self::duel`] by construction, so the pair cannot
    /// cycle.
    pub fn lapse(ctx: &StateProcessingContext) -> Option<DuelLapse> {
        if Self::held_back() || KeeperSetPieceStance::pending(ctx).is_some() {
            return Some(DuelLapse::OffTheBall);
        }
        let Some(carrier) = Self::on_the_ball(ctx) else {
            return Some(DuelLapse::OffTheBall);
        };
        if Self::run_at_goal(ctx, &carrier) <= Self::STILL_COMING {
            return Some(DuelLapse::TurnedAway);
        }
        (!KeeperCarrierThreat::still_through(ctx, &carrier)).then_some(DuelLapse::Covered)
    }

    /// The opponent on the ball: its owner, or, while it is nobody's, the
    /// opponent who had it last, as long as it is still at his feet's reach.
    /// A man pushing it ahead of him leaves it nobody's between touches, and
    /// that is not the end of his run.
    fn on_the_ball(ctx: &StateProcessingContext) -> Option<MatchPlayerLite> {
        if let Some(carrier) = ctx.players().opponents().with_ball().next() {
            return Some(carrier);
        }
        if ctx.ball().is_owned() {
            return None;
        }
        let last = ctx.ball().previous_owner_id()?;
        let ball = ctx.tick_context.positions.ball.position;
        ctx.players().opponents().all().find(|carrier| {
            carrier.id == last
                && (Vector3::new(ball.x, ball.y, 0.0) - carrier.position).magnitude()
                    < Self::TOUCH_AHEAD
        })
    }

    /// How far ahead of a running man his own touch still counts as his:
    /// 24u = 3 m.
    const TOUCH_AHEAD: f32 = 24.0;

    /// Running AT the goal, as a cosine. A man knocking it square along the
    /// edge of the area is not a 1-v-1 and coming to meet him wins nothing.
    /// A man standing still reads as coming.
    fn run_at_goal(ctx: &StateProcessingContext, carrier: &MatchPlayerLite) -> f32 {
        let goal = ctx.ball().direction_to_own_goal();
        let _ = goal;
        let run = ctx.tick_context.positions.players.velocity(carrier.id);
        (goal - carrier.position)
            .try_normalize(1e-3)
            .zip(run.try_normalize(1e-3))
            .map_or(1.0, |(lane, way)| lane.dot(&way))
    }

    /// How far off the ball he holds, in units.
    pub fn standoff(prof: &GoalkeeperSkillProfile) -> f32 {
        KeeperSmother::reach(prof) + Self::MARGIN
    }

    /// Where he stands: on the goal→ball line, a stand-off short of where
    /// the ball is going ([`KeeperRestPosition::anticipated`]).
    ///
    /// Bounded twice, and the two bounds are different shapes: `stand`
    /// caps the DEPTH of the closing point, and [`KeeperSweepLimit`] then
    /// contains it inside the whole territory. A man running at goal down
    /// the touchline is still a duel, and without the second bound the
    /// keeper came to meet him out by the corner flag.
    pub fn point(ctx: &StateProcessingContext, prof: &GoalkeeperSkillProfile) -> Vector3<f32> {
        let ball = &ctx.tick_context.positions.ball;
        KeeperSweepLimit::contain(
            ctx,
            Self::stand(
                ctx.ball().direction_to_own_goal(),
                KeeperRestPosition::anticipated(ball.position, ball.velocity, prof.positioning),
                Self::standoff(prof),
                KeeperSweepLimit::off_line(prof.rushing_out_profile),
            ),
            prof,
        )
    }

    /// The geometry on its own, so the invariants it has to satisfy can be
    /// pinned without a match around them.
    pub fn stand(
        goal: Vector3<f32>,
        ball: Vector3<f32>,
        standoff: f32,
        sweep_limit: f32,
    ) -> Vector3<f32> {
        let out = ball - goal;
        let Some(lane) = out.try_normalize(1e-3) else {
            return goal;
        };
        let range = out.magnitude();
        let depth = (range - standoff)
            .min(range * Self::ADVANCE)
            .clamp(Self::MIN_DEPTH, sweep_limit.max(Self::MIN_DEPTH));
        goal + lane * depth
    }
}

/// **What a keeper can physically do about a ball that is already on its
/// way** — how long before he moves at all, and how fast he can move on
/// his feet once he does.
///
/// # Why this exists: the keeper could RUN faster than he could DIVE
///
/// Measured on a real recording (`dev_match record`, 30 airborne episodes
/// across one match): **22 of them started with the ball already inside
/// 3 m of the keeper**, most of them inside 2 m of a ball travelling at
/// 35 m/s — i.e. 50 ms before it reached him. Not one dive began while the
/// ball was more than 6 m away. The viewer draws that as the ball stopping
/// dead at a standing man who then falls over, which is exactly the report:
/// *"he doesn't dive for the ball at all, for any shot"*.
///
/// [`KeeperShotDive`] below was already asking the right question — can he
/// get there on his feet? — but it asked it against
/// `GoalkeeperSpeedContext::Explosive`, which is `1.0 + agility*0.5 +
/// acceleration*0.5` times a base of 0.36-0.63 u/tick: **8 to 13 m/s**,
/// sideways, for the whole flight. A keeper who can cover the width of his
/// own goal in two thirds of a second never has to dive, because the answer
/// to "can he walk it?" is always yes. The gate was not mis-tuned; the
/// thing it was measuring against was not a goalkeeper.
///
/// # The model
///
/// A shot leaves a keeper with two resources and no others:
///
/// * **A reaction.** 160 ms for an elite shot-stopper, 280 ms for a poor
///   one — the standard human simple-reaction band, and the reason a
///   penalty is a coin flip. He is motionless for it. Measured from the
///   strike, which is recoverable without a new field: `ShotTarget` carries
///   `struck_from`, so the distance already flown divided by the ball's
///   speed IS the elapsed flight.
/// * **A shuffle.** From a set stance — knees bent, weight forward, feet
///   apart — a keeper side-steps at 2.4 to 4.0 m/s. He is not running; a
///   man who runs at a struck ball arrives unset and saves nothing, which
///   is why keepers are coached to be still at the moment of contact.
///
/// Everything else has to come out of the dive. That is the whole point:
/// the dive is not decoration on top of a save the keeper was going to make
/// anyway, it is the only way he covers ground once the ball is struck.
pub struct KeeperShotReaction;

/// The reach model for a dive: how far across and up the ball is, and
/// whether it is worth leaving his feet for.
struct LaunchModel {
    base_reach: f32,
    #[cfg_attr(not(feature = "match-logs"), allow(dead_code))]
    climb: f32,
    gap: f32,
    walkable: f32,
    #[cfg_attr(not(feature = "match-logs"), allow(dead_code))]
    range_band: usize,
}

impl LaunchModel {
    fn read(
        ctx: &StateProcessingContext,
        target: &ShotTarget,
        goal: Vector3<f32>,
        ticks: f32,
        prof: &GoalkeeperSkillProfile,
        range_band: usize,
    ) -> Option<Self> {
        let ball = &ctx.tick_context.positions.ball;
        // How far across he actually has to travel. Asked through the SAME
        // wedge the save itself is scored with, so the dive and the save can
        // never disagree about the geometry.
        let base_reach = KeeperShotSave::base_reach(prof);
        let (lateral_error, reach) = SaveModel::wedge(
            target.struck_from,
            ball.velocity.norm(),
            ctx.player.position,
            base_reach,
            goal.x,
            target.goal_line_y,
        );
        if reach <= 1e-3 {
            return None;
        }
        #[cfg(feature = "match-logs")]
        KeeperDiveDiag::note(3);

        // `wedge` works in goal-line space, where both terms are magnified
        // by the projection; their RATIO scaled back through his real reach
        // gives the units he has to cover from where he is standing.
        //
        // Deliberately NOT clamped at 1.0 the way the save model's
        // `stretch` is: past full stretch is exactly the shot he has to go
        // for and will not get, and clamping it there is what made the
        // corner the one place he never dived. See `DESPAIR_REACH`.
        let across = lateral_error * base_reach / reach;
        // …and HOW HIGH it is going, which nothing here could see. A ball
        // over his head is a ball he has to leave his feet for however
        // straight at him it is — see [`SaveModel::climb_gap`], which is the
        // whole of why a top-corner shot from range used to be the one he
        // never moved for.
        //
        // ⚠ Read off `ShotTarget::goal_line_z` and NOT re-projected from
        // the ball's live vertical state. Re-projecting was tried first and
        // reported the mean long shot arriving **15 cm off the deck**: the
        // gravity term over a 45-tick window is a whole metre, and a shot
        // that has already flattened out clamps straight to zero.
        let climb = SaveModel::climb_gap(target.goal_line_z, SaveModel::set_ceiling(prof));
        let gap = (across * across + climb * climb).sqrt();
        #[cfg(feature = "match-logs")]
        {
            use crate::mid_run_diag::KeeperRangeDiag as R;
            R::add(range_band, 23, (target.goal_line_z * 100.0).max(0.0) as u64);
            if climb > 0.0 {
                R::note(range_band, 16);
                R::add(range_band, 19, (climb * 10.0) as u64);
            }
        }
        let walkable = KeeperShotReaction::ground_left(ctx, prof, ticks);
        #[cfg(feature = "match-logs")]
        {
            use crate::mid_run_diag::KeeperRangeDiag as R;
            KeeperDiveDiag::add(8, (gap.max(0.0) * 10.0) as u64);
            KeeperDiveDiag::add(9, (walkable.max(0.0) * 10.0) as u64);
            R::add(range_band, 14, (gap.max(0.0) * 10.0) as u64);
            R::add(range_band, 15, (walkable.max(0.0) * 10.0) as u64);
        }
        Some(Self {
            base_reach,
            climb,
            gap,
            walkable,
            range_band,
        })
    }

    fn is_dive(&self, ctx: &StateProcessingContext) -> bool {
        if self.gap < KeeperShotDive::STANDING_GAP {
            #[cfg(feature = "match-logs")]
            {
                crate::mid_run_diag::KeeperRangeDiag::note(self.range_band, 10);
                if self.climb > 0.0 {
                    crate::mid_run_diag::KeeperRangeDiag::note(self.range_band, 18);
                }
            }
            return false;
        }
        #[cfg(feature = "match-logs")]
        KeeperDiveDiag::note(4);
        if self.gap > self.base_reach * KeeperShotDive::DESPAIR_REACH {
            #[cfg(feature = "match-logs")]
            crate::mid_run_diag::KeeperRangeDiag::note(self.range_band, 11);
            return false;
        }
        #[cfg(feature = "match-logs")]
        KeeperDiveDiag::note(5);

        if self.gap <= self.walkable * KeeperShotDive::ON_FOOT_SHARE {
            // He can step to this one. Staying on his feet is the better
            // save and the more common picture.
            #[cfg(feature = "match-logs")]
            crate::mid_run_diag::KeeperRangeDiag::note(self.range_band, 12);
            return false;
        }
        #[cfg(feature = "match-logs")]
        {
            KeeperDiveDiag::note(6);
            KeeperActionDiag::note(15);
            use crate::mid_run_diag::{KeeperQualityDiag as Q, KeeperRangeDiag as R};
            R::note(self.range_band, 5);
            R::note(self.range_band, 13);
            if self.climb > 0.0 {
                R::note(self.range_band, 17);
            }
            // …and by the keeper, on the same composite `try_save_shot`
            // bands the ARRIVAL by, so "did quality decide whether he went
            // for it" and "did quality decide whether he stopped it" are
            // answered on one axis. See `KeeperQualityDiag`.
            Q::note(
                Q::band(sc::gk_shot_stopping(
                    ctx.player,
                    sc::minute_from_ms(ctx.context.total_match_time),
                )),
                5,
            );
        }
        true
    }

    #[cfg(feature = "match-logs")]
    fn note_launch(&self, ctx: &StateProcessingContext, target: &ShotTarget, goal: Vector3<f32>) {
        use crate::mid_run_diag::KeeperCommitDiag as C;
        let b = &ctx.tick_context.positions.ball;
        let to_plane = (ctx.player.position.x - b.position.x) / b.velocity.x;
        if to_plane.is_finite() && to_plane > 0.0 {
            let true_y = b.position.y + b.velocity.y * to_plane;
            let aim = KeeperShotDive::crossing_at(
                ctx.player.position.x,
                target.struck_from,
                goal.x,
                target.goal_line_y,
            );
            C::note_launch(
                self.range_band,
                aim.y - true_y,
                aim.y - ctx.player.position.y,
                true_y - ctx.player.position.y,
            );
        }
    }

    #[cfg(not(feature = "match-logs"))]
    fn note_launch(
        &self,
        _ctx: &StateProcessingContext,
        _target: &ShotTarget,
        _goal: Vector3<f32>,
    ) {
    }
}

impl KeeperShotReaction {
    /// Reaction time, in ENGINE ticks (10 ms), for a keeper at the bottom
    /// and the top of the shot-stopping range.
    ///
    /// ⚠ **NOT a simple-reaction time**, and that distinction is worth a
    /// paragraph because getting it wrong costs the population save rate.
    /// A simple reaction — a light comes on, press the button — is 150-280
    /// ms and a keeper does not beat it. But a keeper facing a shot is not
    /// doing that task: he is set, he has watched the striker's hips and
    /// standing foot, and he is already loading one leg as the boot comes
    /// through. What is measured here is the delay between the ball leaving
    /// the foot and his weight going the other way, and for a prepared
    /// keeper that runs 120-240 ms. `anticipation` is what buys the
    /// difference, so it is scored through the positioning composite as
    /// well as through pure reflexes.
    const SLOW_REACTION: f32 = 24.0 * Self::SHOT_TEMPO;
    const FAST_REACTION: f32 = 12.0 * Self::SHOT_TEMPO;

    /// **This engine's shots arrive faster than football's, so every keeper
    /// time and speed has to be expressed in the engine's own time base.**
    ///
    /// `SaveModel::ORDINARY_STRIKE` is 2.63 u/tick — **32.9 m/s** — and the
    /// action census measures the mean shot reaching the save roll at 2.70.
    /// A real shot on target averages about 23 m/s. So a flight that takes a
    /// real keeper 0.7 s takes this one 0.5, and a wall-clock human reaction
    /// dropped into it charges him 40% more of the flight than it charges a
    /// human. Measured, that is worth about ten points of save rate.
    ///
    /// The honest fix is not to make the keeper superhuman, it is to state
    /// the conversion once and put every duration and speed through it. If
    /// the shot-speed calibration is ever brought back to real, this goes to
    /// 1.0 and the keeper's numbers are already right.
    ///
    /// `pub(crate)` because `SaveModel::FULL_STRETCH_TICKS` — the clock
    /// that decides whether he is credited with being able to reach the
    /// ball at all — is a keeper duration like any other and was the one
    /// left out of this conversion. See its note.
    pub(crate) const SHOT_TEMPO: f32 = 23.0 / 32.9;

    /// Lateral speed from a set stance, in u/tick (1 u = 0.125 m, 1 tick =
    /// 10 ms). 0.19 → 2.4 m/s, 0.32 → 4.0 m/s in wall-clock terms, divided
    /// by [`Self::SHOT_TEMPO`] because he has to cover the same share of his
    /// goal in the shorter flight this engine gives him.
    const HEAVY_FEET: f32 = 0.19 / Self::SHOT_TEMPO;
    const QUICK_FEET: f32 = 0.32 / Self::SHOT_TEMPO;

    /// Remaining flight, in engine ticks, over which he stops running and
    /// gets his feet set.
    ///
    /// Outside 0.6 s he is still a footballer covering ground and moves at
    /// his own running pace; inside it he is a goalkeeper being shot at,
    /// and every metre has to come out of the dive. Continuous, because
    /// there is no instant at which a keeper stops running and starts
    /// setting — he decelerates into it.
    const SETTLING_TICKS: f32 = 60.0 * Self::SHOT_TEMPO;

    /// Diagnostic switch: with `OF_KEEPER_SERVO` set, the keeper goes back
    /// to being a tracking servo — no reaction, no shuffle cap, no read
    /// lag, and [`KeeperShotDive::should_launch`] weighed against a sprint.
    ///
    /// The A/B control for this whole model. Its effects reach every shot
    /// in the game, so "did the reaction work cost the save rate?" cannot
    /// be answered by reading the diff, and it must not be answered by
    /// checking out an older revision either — the working tree moves under
    /// you. Same pattern and purpose as `MatchContext::shape_off`; read once
    /// per process. Debug infrastructure — do not remove.
    pub fn servo() -> bool {
        use std::sync::OnceLock;
        static SERVO: OnceLock<bool> = OnceLock::new();
        *SERVO.get_or_init(|| std::env::var("OF_KEEPER_SERVO").is_ok())
    }

    /// Ticks of flight already elapsed, from the geometry of the shot.
    ///
    /// `struck_from` is on the target and the ball's speed is known, so
    /// the strike needs no timestamp of its own — which matters, because
    /// `ShotTarget` is copied into every player's frozen `tick_context`
    /// and a clock on it would have to be right in all of them.
    pub fn since_strike(ctx: &StateProcessingContext) -> f32 {
        let ball = &ctx.tick_context.positions.ball;
        let Some(target) = ctx.tick_context.ball.cached_shot_target.as_ref() else {
            return f32::MAX;
        };
        let speed = ball.velocity.norm();
        if speed < 1e-3 {
            return f32::MAX;
        }
        // Seen from where he first sees it: a free kick is behind its wall
        // for the first of its flight.
        ((ball.position - target.struck_from).magnitude() - target.screened()).max(0.0) / speed
    }

    /// His own reaction time, in engine ticks. Two thirds reflexes, one
    /// third reading the striker — see the constants above.
    pub fn reaction_ticks(prof: &GoalkeeperSkillProfile) -> f32 {
        let sharpness = (prof.shot_stopping.clamp(0.0, 1.0) * 0.65
            + prof.positioning.clamp(0.0, 1.0) * 0.35)
            .clamp(0.0, 1.0);
        Self::SLOW_REACTION - (Self::SLOW_REACTION - Self::FAST_REACTION) * sharpness
    }

    /// The most a keeper still moving at the strike adds to his reaction:
    /// 40%, reached at twice his set shuffle. His weight is going the wrong
    /// way, and he has to stop it before he can push for the ball.
    const CAUGHT_MOVING: f32 = 0.40;

    /// The share of his reaction he adds for being on the move as the shot
    /// was struck — his ground speed then, past his set shuffle.
    pub fn moving_penalty(keeper: &MatchPlayer) -> f32 {
        if KeeperDebug::strike_set_off() {
            return 0.0;
        }
        let shuffle = Self::set_shuffle(keeper.skills.physical.agility);
        Self::CAUGHT_MOVING * ((keeper.strike_speed - shuffle) / shuffle).clamp(0.0, 1.0)
    }

    /// His reaction to this shot, in engine ticks: his own, and later for
    /// being caught moving.
    fn reaction(ctx: &StateProcessingContext, prof: &GoalkeeperSkillProfile) -> f32 {
        Self::reaction_ticks(prof) * (1.0 + Self::moving_penalty(ctx.player))
    }

    /// Has he moved yet? Before this he is a statue, and that is not a
    /// stylistic choice — it is where a well-placed shot gets its value.
    pub fn has_reacted(ctx: &StateProcessingContext, prof: &GoalkeeperSkillProfile) -> bool {
        Self::servo() || Self::since_strike(ctx) >= Self::reaction(ctx, prof)
    }

    /// His set-stance shuffle, in u/tick — the floor of the band below.
    fn shuffle(prof: &GoalkeeperSkillProfile) -> f32 {
        Self::HEAVY_FEET + (Self::QUICK_FEET - Self::HEAVY_FEET) * prof.dive_reach.clamp(0.0, 1.0)
    }

    /// The same shuffle with no shot in flight, in u/tick: 2.4–4.0 m/s by
    /// agility, without the shot-tempo conversion, because there is no
    /// compressed flight to keep up with.
    pub fn set_shuffle(agility: f32) -> f32 {
        let quick = (agility / 20.0).clamp(0.0, 1.0);
        (Self::HEAVY_FEET + (Self::QUICK_FEET - Self::HEAVY_FEET) * quick) * Self::SHOT_TEMPO
    }

    /// Top speed on his feet with the ball `ticks_left` from reaching him,
    /// in u/tick: his running pace while it is still a long way off,
    /// tapering to the set shuffle as it closes.
    pub fn step_speed(
        ctx: &StateProcessingContext,
        prof: &GoalkeeperSkillProfile,
        ticks_left: f32,
    ) -> f32 {
        let shuffle = Self::shuffle(prof);
        // His own legs, with no goalkeeper multiplier on them. The
        // `Explosive` and `Active` bands exist to model reaching for a ball,
        // not to make a keeper the fastest man on the pitch.
        let running = ctx
            .player
            .skills
            .max_speed_with_condition(ctx.player.player_attributes.condition)
            .max(shuffle);
        shuffle + (running - shuffle) * (ticks_left / Self::SETTLING_TICKS).clamp(0.0, 1.0)
    }

    /// Integral of the band above from `to` up to `from` ticks remaining,
    /// i.e. the ground covered while the clock runs down between them.
    fn travel(shuffle: f32, running: f32, from: f32, to: f32) -> f32 {
        if from <= to {
            return 0.0;
        }
        let s = Self::SETTLING_TICKS;
        // ∫ clamp(t/s, 0, 1) dt over [to, from]
        let ramp = |t: f32| {
            if t <= s {
                t * t / (2.0 * s)
            } else {
                s / 2.0 + (t - s)
            }
        };
        shuffle * (from - to) + (running - shuffle) * (ramp(from) - ramp(to))
    }

    /// The last of the flight, in engine ticks, during which nothing he
    /// does on his feet counts.
    ///
    /// A keeper who is still travelling when the ball arrives has not made
    /// a save — he has to plant, and that costs about the last fifth of a
    /// second. It is the honest form of the arbitrary "share of the gap"
    /// factor this replaced: what stops him walking to a shot into the
    /// corner is not that he covers 70% of the ground, it is that the last
    /// stretch of a flight is not walkable at all.
    const PLANT_TICKS: f32 = 20.0 * Self::SHOT_TEMPO;

    /// Ground he can still cover on his feet in `ticks_left`, in units —
    /// the quantity [`KeeperShotDive::should_launch`] weighs the gap
    /// against, and the reason a shot into the corner is a dive rather
    /// than a walk.
    pub fn ground_left(
        ctx: &StateProcessingContext,
        prof: &GoalkeeperSkillProfile,
        ticks_left: f32,
    ) -> f32 {
        let stalled =
            (Self::reaction(ctx, prof) - Self::since_strike(ctx)).clamp(0.0, ticks_left.max(0.0));
        if Self::servo() {
            // The comparison this model replaced: a flat sprint, all the
            // way to the moment of contact.
            return ctx
                .player
                .skills
                .goalkeeper_max_speed(
                    ctx.player.player_attributes.condition,
                    GoalkeeperSpeedContext::Explosive,
                )
                .max(0.1)
                * ticks_left.max(0.0);
        }
        let shuffle = Self::shuffle(prof);
        let running = ctx
            .player
            .skills
            .max_speed_with_condition(ctx.player.player_attributes.condition)
            .max(shuffle);
        Self::travel(
            shuffle,
            running,
            (ticks_left - stalled).max(0.0),
            Self::PLANT_TICKS,
        )
    }

    /// Fraction of the flight by which he has read where the ball is
    /// actually going — 0.30 for a keeper who reads the game, 0.58 for one
    /// who does not, both measured from the strike and therefore including
    /// the reaction above.
    const SHARP_READ: f32 = 0.30;
    const SLOW_READ: f32 = 0.58;

    /// **Where he currently believes the shot will cross the goal line.**
    ///
    /// This is the second half of why the keeper never dived, and it is the
    /// deeper half. Both save states steered him straight at
    /// `ShotTarget::goal_line_y` — the true crossing point — from the tick
    /// the ball left the boot. A tracking servo with the answer in hand
    /// never has a gap to close, so `KeeperShotDive` correctly concluded he
    /// could walk to every shot in the game, and the reported symptom
    /// ("*he doesn't dive for the ball at all if it's a long shot*") is the
    /// direct consequence: the longer the flight, the more completely
    /// perfect tracking removes the need to leave his feet.
    ///
    /// A keeper does not know where a shot is going when it is struck. He
    /// holds the position he set himself in and commits as he reads the
    /// flight. Placement beats a keeper because he reads it late, not
    /// because he decides to stand still.
    ///
    /// **The anchor is where he already IS, not the middle of the goal.**
    /// Hedging to the centre was tried first and is worse in both
    /// directions: against a shot from a wide angle it sends him AWAY from
    /// the ball before it brings him back, which is ground he never gets
    /// again, and it is not what a keeper does — he does not abandon a
    /// near post he has just taken up to stand in the middle. Anchoring on
    /// himself also means confidence 0 costs him nothing at all.
    ///
    /// Continuous in elapsed flight, so there is no instant at which he
    /// snaps onto the answer, and scaled by his positioning composite, so
    /// reading a shot early is finally worth something.
    pub fn crossing_y(
        ctx: &StateProcessingContext,
        prof: &GoalkeeperSkillProfile,
        own_goal: Vector3<f32>,
        target: &ShotTarget,
    ) -> f32 {
        let ball = ctx.tick_context.positions.ball.position;
        // Elapsed share of the flight, measured as GROUND rather than as
        // time: a shot decelerates, so the same fraction of the clock is
        // not the same fraction of the journey, and what he is reading is
        // the ball moving across his eye-line.
        let arrives_at = Vector3::new(own_goal.x, target.goal_line_y, 0.0);
        let flown = (ball - target.struck_from).magnitude();
        let left = (ball - arrives_at).magnitude();
        let elapsed = flown / (flown + left).max(1e-3);
        let read_by = Self::SLOW_READ
            - (Self::SLOW_READ - Self::SHARP_READ) * prof.positioning.clamp(0.0, 1.0);
        let confidence = if Self::servo() {
            1.0
        } else {
            (elapsed / read_by.max(1e-3)).clamp(0.0, 1.0)
        };
        let held = ctx.player.position.y;
        let belief = held + (target.goal_line_y - held) * confidence;
        // Which way that pulls him, against which way the ball is really
        // going. This is the site the commitment is actually made at — the
        // dive only ratifies it — so it is the only place a keeper walking
        // away from the ball can be counted. See `KeeperCommitDiag`.
        #[cfg(feature = "match-logs")]
        {
            let b = &ctx.tick_context.positions.ball;
            let to_line = (own_goal.x - b.position.x) / b.velocity.x;
            if to_line.is_finite() && to_line > 0.0 {
                let true_line = b.position.y + b.velocity.y * to_line;
                // ⚠ ON-FRAME ONLY. Unfiltered, the wide shots he correctly
                // ignores dominate every tick counted here and the mean
                // reads nineteen metres across him.
                if (true_line - own_goal.y).abs() <= GOAL_WIDTH {
                    crate::mid_run_diag::KeeperCommitDiag::note_steer(
                        belief - held,
                        true_line - held,
                    );
                }
            }
        }
        belief
    }

    /// Hold a steering vector to what a set keeper can actually do.
    ///
    /// Returns the vector unchanged when there is no shot of his in
    /// flight, so open play — sweeping, coming for a cross, getting back
    /// to his line — keeps every bit of the pace it had.
    pub fn on_foot(
        ctx: &StateProcessingContext,
        prof: &GoalkeeperSkillProfile,
        velocity: Vector3<f32>,
    ) -> Vector3<f32> {
        let live = !Self::servo()
            && ctx
                .tick_context
                .ball
                .cached_shot_target
                .as_ref()
                .is_some_and(|t| Some(t.defending_side) == ctx.player.side);
        if !live {
            return velocity;
        }
        if !Self::has_reacted(ctx, prof) {
            return Vector3::zeros();
        }
        let cap = Self::step_speed(ctx, prof, Self::ticks_left(ctx));
        match velocity.try_normalize(1e-4) {
            Some(dir) => dir * velocity.magnitude().min(cap),
            None => Vector3::zeros(),
        }
    }

    /// Engine ticks until the shot reaches the keeper's own depth.
    ///
    /// ⚠ Measured to the point it crosses HIM, along the ball's own line of
    /// flight — **not** as `x`-distance over `x`-velocity to the goal line.
    /// That form divides by a component that goes to zero as a shot flattens
    /// across the face of goal, and it did: measured, the mean window it
    /// handed out was **4.5 seconds**, so `ground_left` credited the keeper
    /// with walking 14 m and "can he get there on his feet?" answered yes
    /// for every shot in the game.
    pub fn ticks_left(ctx: &StateProcessingContext) -> f32 {
        let Some(target) = ctx.tick_context.ball.cached_shot_target.as_ref() else {
            return 0.0;
        };
        let ball = &ctx.tick_context.positions.ball;
        let speed = ball.velocity.norm();
        if speed < 0.05 {
            return f32::MAX;
        }
        let goal = ctx.ball().direction_to_own_goal();
        let _ = goal;
        let crossing = KeeperShotDive::crossing_at(
            ctx.player.position.x,
            target.struck_from,
            goal.x,
            target.goal_line_y,
        );
        (crossing - ball.position).magnitude() / speed
    }
}

/// **When a keeper has to leave his feet to reach the shot.**
///
/// # Why this exists
///
/// A keeper's dive was drawn AFTER the save. `Ball::try_save_shot` resolves
/// a shot at the goal line and `apply_pending_save_credit` then puts him
/// into [`GoalkeeperState::Diving`](super::state::GoalkeeperState::Diving)
/// — so the leap fired on the tick the ball had already been stopped, and
/// the whole flight before it was spent on his feet. From the stands the
/// ball flew at the corner, stopped dead at a standing man, and the keeper
/// then fell over: "he doesn't dive into the corners".
///
/// The state machine could not have produced the dive either.
/// `PreparingForSave::should_dive` requires the ball within `DIVE_DISTANCE`
/// — 40u, **5 metres**, about a seventh of a second of flight — and
/// `Catching` never asks the question at all: it commits to the intercept
/// point and shuffles there.
///
/// # The model
///
/// A dive is a decision made DURING the flight, and it has exactly one
/// input: can he get there on his feet? Project the shot onto his own
/// depth, measure the gap across, and compare it with the ground he can
/// cover before the ball arrives. If he can walk it, he stays up — that is
/// what a keeper who has read the shot properly does, and it is why good
/// positioning looks like doing nothing. If he cannot, he goes, and he
/// goes early enough to be at full stretch when the ball gets there.
///
/// It deliberately does NOT decide whether the save is MADE. That stays
/// with `SaveModel` and the shared roll in [`KeeperShotSave`], so putting
/// the keeper in the air changes where he is and what he looks like doing
/// it, and changes no calibrated number.
pub struct KeeperShotDive;

impl KeeperShotDive {
    /// How long a keeper is off the ground on his way to a save, in engine
    /// ticks (10 ms). Measured off `.dev/match` recordings: 390-660 ms
    /// airborne with a median of 450. He leaves the ground this far ahead
    /// of the ball so that the apex of the dive and the ball arrive
    /// together — the same reasoning as [`AerialClaim::TAKEOFF_TICKS`],
    /// and the same failure if it is wrong (take off too early and he
    /// lands before it gets there).
    const FLIGHT_TICKS: f32 = 45.0;
    /// …and the share of the ground he could still walk that he will
    /// actually try to walk, before he accepts that the only way there is
    /// through the air. Below 1.0 because a keeper would rather go early
    /// and be wrong than go late and be short; the physical part of "he
    /// cannot walk it" lives in `KeeperShotReaction::PLANT_TICKS`, and this
    /// is only the appetite on top of it.
    ///
    /// ⚠ **This is weighed against [`KeeperShotReaction::ground_left`], not
    /// against a sprint.** It used to be `goalkeeper_max_speed(Explosive)`
    /// — 8-13 m/s — against a gap that can never exceed his own reach of
    /// 2-3 m, so the comparison was decided before it was made and the
    /// answer was always "he can walk it". Measured: not one dive in a
    /// recorded match began with the ball more than 6 m away.
    const ON_FOOT_SHARE: f32 = 0.85;
    /// Under this he is not diving, he is stepping. 6u = 75 cm.
    const STANDING_GAP: f32 = 6.0;
    /// How far past his own reach he will still throw himself, as a
    /// multiple of it.
    ///
    /// **A keeper dives at shots he does not save.** That is most of them,
    /// and the full-stretch miss is the most recognisable image in the
    /// sport. This gate used to be `lateral_error > reach → don't go`,
    /// borrowed from the save model — so the one ball he was certain not to
    /// reach, the one into the far corner, was also the one he never moved
    /// for. Beyond this he is nearer the other post than the ball and going
    /// is not a save attempt, it is falling over.
    ///
    /// Measured at 1.9 and again at 2.4, this gate was still throwing away
    /// **58% of every tick on which the keeper had more than a step to
    /// cover** — the whole population of shots he was beaten by. 3.5 × a
    /// 2.15 m reach is 7.5 m, one goal width, so with the post-width gate
    /// above it now says only: *he goes for anything inside his own goal
    /// that he cannot step to.* Which is what a goalkeeper does.
    const DESPAIR_REACH: f32 = 3.5;

    /// **How far over the bar a shot has to be before the keeper writes it
    /// off entirely.**
    ///
    /// The width gate below already carries `SHOULDER` — "the ball that
    /// clips the outside of the post is one he still has to be seen to go
    /// for" — and the height gate carried nothing at all, which is an
    /// asymmetry with no argument behind it. Worse, it disagreed with the
    /// save: `Ball::try_save_shot` retires a shot as off-frame at **2.8 m**,
    /// so between the bar and 2.8 the physics save would still resolve a
    /// shot the keeper had already decided not to move for. Two models for
    /// one question, and the stricter one owned the behaviour while the
    /// looser one owned the outcome.
    ///
    /// Measured, that gate wrote off **32% of every live-shot tick from
    /// 18-28 m**, and the recordings show why it matters: those shots cross
    /// the line at 2.1-2.4 m, i.e. UNDER the bar. `goal_line_z` is a clean
    /// arc taken at the moment of the strike and the real ball carries
    /// drag, so it over-predicts — and the keeper was writing off savable
    /// shots on a prediction the save itself did not believe.
    const CROSSBAR_MARGIN: f32 = 0.36;

    /// Does he have to leave his feet for the shot in flight, right now?
    ///
    /// A predicate rather than a description: the two callers only need to
    /// know whether to hand him to `Diving`, and `Diving` recomputes the
    /// point it is aiming at every tick through [`Self::crossing_at`] —
    /// carrying a snapshot of it across the transition would be a second
    /// copy that goes stale the moment the projection updates.
    pub fn should_launch(ctx: &StateProcessingContext) -> bool {
        let Some(target) = ctx.tick_context.ball.cached_shot_target.as_ref() else {
            return false;
        };
        if Some(target.defending_side) != ctx.player.side {
            return false;
        }
        #[cfg(feature = "match-logs")]
        KeeperDiveDiag::note(0);

        let goal: Vector3<f32> = ctx.ball().direction_to_own_goal();
        #[cfg(feature = "match-logs")]
        {
            use crate::mid_run_diag::KeeperRangeDiag as R;
            let strike = (target.struck_from - Vector3::new(goal.x, goal.y, target.struck_from.z))
                .magnitude();
            let band = R::band(strike);
            R::note(band, 20);
        }

        if !Self::in_frame(target, goal) {
            return false;
        }
        let Some((ticks, range_band, prof)) = Self::reaction(ctx) else {
            return false;
        };
        let Some(model) = LaunchModel::read(ctx, target, goal, ticks, &prof, range_band) else {
            return false;
        };
        if !model.is_dive(ctx) {
            return false;
        }
        model.note_launch(ctx, target, goal);
        true
    }

    /// Over the bar, or wide of the post, is a ball he never moves for.
    fn in_frame(target: &ShotTarget, goal: Vector3<f32>) -> bool {
        // `SHOULDER` is one dive's worth of reach — the ball that clips the
        // outside of the post is one he still has to be seen to go for.
        const SHOULDER: f32 = 12.0;
        if target.goal_line_z > GOAL_HEIGHT + Self::CROSSBAR_MARGIN {
            #[cfg(feature = "match-logs")]
            {
                use crate::mid_run_diag::KeeperRangeDiag as R;
                let strike = (target.struck_from
                    - Vector3::new(goal.x, goal.y, target.struck_from.z))
                .magnitude();
                R::note(R::band(strike), 7);
            }
            return false;
        }
        #[cfg(feature = "match-logs")]
        {
            use crate::mid_run_diag::KeeperRangeDiag as R;
            let strike = (target.struck_from - Vector3::new(goal.x, goal.y, target.struck_from.z))
                .magnitude();
            R::note(R::band(strike), 6);
        }
        if (target.goal_line_y - goal.y).abs() > GOAL_WIDTH + SHOULDER {
            #[cfg(feature = "match-logs")]
            {
                use crate::mid_run_diag::KeeperRangeDiag as R;
                let strike = (target.struck_from
                    - Vector3::new(goal.x, goal.y, target.struck_from.z))
                .magnitude();
                R::note(R::band(strike), 8);
            }
            return false;
        }
        #[cfg(feature = "match-logs")]
        {
            use crate::mid_run_diag::KeeperRangeDiag as R;
            let strike = (target.struck_from - Vector3::new(goal.x, goal.y, target.struck_from.z))
                .magnitude();
            R::note(R::band(strike), 7);
        }
        true
    }

    /// The reaction window: is the ball still in flight long enough for
    /// him to have seen it strike?
    fn reaction(ctx: &StateProcessingContext) -> Option<(f32, usize, GoalkeeperSkillProfile)> {
        let ticks = KeeperShotReaction::ticks_left(ctx);
        if !ticks.is_finite() || ticks > Self::FLIGHT_TICKS {
            #[cfg(feature = "match-logs")]
            KeeperDiveDiag::note(1);
            return None;
        }
        let goal = ctx.ball().direction_to_own_goal();
        let _ = goal;
        #[cfg(feature = "match-logs")]
        let range_band = {
            use crate::mid_run_diag::KeeperRangeDiag as R;
            let target = ctx.tick_context.ball.cached_shot_target.as_ref()?;
            let strike = (target.struck_from - Vector3::new(goal.x, goal.y, target.struck_from.z))
                .magnitude();
            R::band(strike)
        };
        #[cfg(not(feature = "match-logs"))]
        let range_band = 0;
        let prof = GoalkeeperSkillProfile::from_ctx(ctx);
        // He has not seen it yet. A keeper who leaves his feet inside his
        // own reaction time is reading the shot before it is struck, and
        // that is where a well-placed strike gets its value — see
        // [`KeeperShotReaction`].
        if !KeeperShotReaction::has_reacted(ctx, &prof) {
            #[cfg(feature = "match-logs")]
            crate::mid_run_diag::KeeperRangeDiag::note(range_band, 9);
            return None;
        }
        #[cfg(feature = "match-logs")]
        KeeperDiveDiag::note(2);
        Some((ticks, range_band, prof))
    }
    /// The `y` the dive is aimed at on the goal line: the crossing point
    /// as he read it, unless he has COMMITTED to a side — a penalty guess
    /// — in which case that side, for the life of the dive. See
    /// [`KeeperPenaltyStance`] and `MatchPlayer::dive_aim`.
    pub fn aim_y(ctx: &StateProcessingContext, target: &ShotTarget) -> f32 {
        ctx.player.dive_aim.unwrap_or(target.goal_line_y)
    }

    /// Where the shot crosses the keeper's OWN depth — the point he is
    /// diving at. Diving at the goal line instead sends him backwards into
    /// his own net whenever he has come out to narrow the angle.
    pub fn crossing_at(
        keeper_x: f32,
        struck_from: Vector3<f32>,
        goal_x: f32,
        goal_line_y: f32,
    ) -> Vector3<f32> {
        let span = struck_from.x - goal_x;
        if span.abs() < 1.0 {
            return Vector3::new(keeper_x, goal_line_y, 0.0);
        }
        let travelled = ((struck_from.x - keeper_x) / span).clamp(0.0, 1.0);
        Vector3::new(
            keeper_x,
            struck_from.y + (goal_line_y - struck_from.y) * travelled,
            0.0,
        )
    }
}

/// How far a keeper can get a glove from where he stands, which is what the
/// dive decision reads. The save itself is decided once, on the ball
/// (`Ball::try_save_shot`), and he carries out what it decides.
pub struct KeeperShotSave;

impl KeeperShotSave {
    /// Effective reach in game units: weak ~14u, elite ~30u.
    pub fn base_reach(prof: &GoalkeeperSkillProfile) -> f32 {
        10.0 + prof.dive_reach * 12.0 + prof.shot_stopping * 4.0
    }
}

/// Goalkeeper condition processor (type alias for clarity)
pub type GoalkeeperCondition = ConditionProcessor<GoalkeeperConfig>;

// Re-export for convenience
pub use crate::r#match::engine::player::strategies::shared::ActivityIntensity;

#[cfg(test)]
mod tests {
    use super::*;

    /// **COMMIT < DISENGAGE, or the pair is a two-cycle.** This engine has
    /// broken that invariant five times — the shot bar, the keeper's
    /// come-out scan, `MAX_COMING_OUT_DISTANCE`, the full-back overlap, and
    /// the kickoff-slot leash this constant replaced, which killed 83% of
    /// every sweep the keeper committed to.
    ///
    /// `KeeperOneOnOne::duel` meets a carrier out to `150 * (1 + risk * 0.4)`
    /// = 180u at the extreme, so the excursion limit has to clear that for
    /// EVERY keeper, including the one least willing to leave his line.
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

    /// **The corner flag is not in any keeper's territory, and his own
    /// six-yard box is in every keeper's.**
    ///
    /// The reported bug, as an assertion. Before the territory had a
    /// second axis, `is_within` asked `|keeper.x − goal.x| <= off_line`,
    /// which the corner flag satisfies exactly — it is ON the goal line.
    #[test]
    fn no_keeper_defends_the_corner_flag_and_every_keeper_defends_his_six_yard_box() {
        let goal = Vector3::new(0.0, 272.5, 0.0);
        // Both flags at his end, and the two "deep and wide" points that
        // the census caught him standing at.
        let outside = [
            Vector3::new(0.0, 0.0, 0.0),
            Vector3::new(0.0, 545.0, 0.0),
            Vector3::new(200.0, 60.0, 0.0),
            Vector3::new(240.0, 480.0, 0.0),
        ];
        // …and the corners of his own six-yard box, which he must always
        // be able to come to: 5.5 m deep, 9.16 m either side.
        let inside = [
            Vector3::new(44.0, 272.5 - 73.3, 0.0),
            Vector3::new(44.0, 272.5 + 73.3, 0.0),
        ];

        for rushing in [0.0f32, 0.5, 1.0] {
            for command in [0.0f32, 0.5, 1.0] {
                let axes = (
                    KeeperSweepLimit::off_line(rushing),
                    KeeperSweepLimit::off_centre(command),
                );
                for point in outside {
                    let strain = KeeperSweepLimit::strain(goal, point, axes);
                    assert!(
                        strain > 1.0,
                        "a keeper at rushing {rushing} / command {command} defends \
                         ({}, {}) — strain {strain:.2}, which is the corner of the pitch",
                        point.x,
                        point.y
                    );
                }
                for point in inside {
                    let strain = KeeperSweepLimit::strain(goal, point, axes);
                    assert!(
                        strain <= 1.0,
                        "a keeper at rushing {rushing} / command {command} will not come \
                         to the corner of his own six-yard box — strain {strain:.2}"
                    );
                }
            }
        }
    }

    /// A sweeper's ground strictly contains a line-keeper's, on both axes,
    /// and containment lands a point on the boundary rather than somewhere
    /// else on the pitch.
    #[test]
    fn the_territory_grows_with_the_keeper_and_containment_lands_on_its_edge() {
        let goal = Vector3::new(0.0, 272.5, 0.0);
        assert!(KeeperSweepLimit::off_line(1.0) > KeeperSweepLimit::off_line(0.0));
        assert!(KeeperSweepLimit::off_centre(1.0) > KeeperSweepLimit::off_centre(0.0));

        let axes = (
            KeeperSweepLimit::off_line(0.4),
            KeeperSweepLimit::off_centre(0.4),
        );
        for point in [
            Vector3::new(400.0, 272.5, 0.0),
            Vector3::new(0.0, 40.0, 0.0),
            Vector3::new(300.0, 500.0, 0.0),
        ] {
            let strain = KeeperSweepLimit::strain(goal, point, axes);
            assert!(strain > 1.0, "test point is already inside");
            let held = goal + (point - goal) / strain;
            assert!(
                (KeeperSweepLimit::strain(goal, held, axes) - 1.0).abs() < 1e-3,
                "containment did not land on the boundary"
            );
            // On the same line out of the goal, and never further than the
            // point it is holding him back from.
            let out = (point - goal).normalize();
            let back = (held - goal).normalize();
            assert!(out.dot(&back) > 0.999, "containment changed his bearing");
            assert!((held - goal).magnitude() < (point - goal).magnitude());
        }
    }

    /// Across the goal and along it are not the same tolerance. The goal
    /// is 58u wide and a keeper's dive reaches 20-32u, so a lateral
    /// tolerance anywhere near the depth one lets him stand a post-width
    /// out of position and call it set.
    #[test]
    fn the_set_tolerance_is_tight_across_the_goal_and_slack_in_depth() {
        const {
            assert!(
                KeeperRestPosition::LATERAL_DEADZONE * 3.0 < KeeperRestPosition::SET_DEADZONE,
                "the two axes must not share a scale"
            )
        };
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

    /// **A keeper dives at the point the ball crosses HIS depth, not at the
    /// goal line.**
    ///
    /// The difference is the whole value of coming off your line. A keeper
    /// three metres out facing a shot into the far corner has a much smaller
    /// gap to cover than the one measured at the line — that is what the
    /// wedge prices — and aiming him at the goal line would send him
    /// BACKWARDS into his own net to defend a point the ball reaches after
    /// it has already gone past him.
    #[test]
    fn the_dive_goes_to_where_the_ball_crosses_him() {
        // Left-hand goal at x = 0, shot struck from 160u out and aimed 20u
        // wide of centre; keeper 24u off his line.
        let struck_from = Vector3::new(160.0, 250.0, 0.0);
        let (goal_x, goal_line_y) = (0.0, 290.0);
        let at_line = KeeperShotDive::crossing_at(0.0, struck_from, goal_x, goal_line_y);
        let at_keeper = KeeperShotDive::crossing_at(24.0, struck_from, goal_x, goal_line_y);
        assert!(
            (at_line.y - goal_line_y).abs() < 1e-3,
            "on the line it has to BE the line: {at_line:?}"
        );
        assert!(
            at_keeper.y < at_line.y && at_keeper.y > struck_from.y,
            "coming out has to shorten the gap, not lengthen it: {at_keeper:?}"
        );
        assert!(
            (at_keeper.x - 24.0).abs() < 1e-3,
            "and it is a LATERAL dive — his depth is his own: {at_keeper:?}"
        );
    }

    /// **A KEEPER MUST NOT BE ABLE TO WALK TO THE CORNER.**
    ///
    /// The invariant the whole dive rests on, and the one that was broken:
    /// `should_launch` weighed the gap against
    /// `goalkeeper_max_speed(Explosive)` — 8-13 m/s — while the gap itself
    /// can never exceed his own reach of two or three metres. A budget
    /// bigger than the quantity it bounds decides the comparison before it
    /// is made, so the answer to "can he get there on his feet?" was
    /// always yes and the keeper never dived at anything.
    ///
    /// Same shape as the `COMMIT < DISENGAGE` invariant above: whenever an
    /// outer bound is looser than the decision it guards, the decision is
    /// dead.
    #[test]
    fn a_set_keeper_cannot_walk_to_the_corner_of_his_own_goal() {
        // The whole launch window, for the quickest feet in the game and no
        // reaction to pay for.
        let quick = KeeperShotReaction::QUICK_FEET;
        // 7.9 m/s, an outright sprint, as the far end of the ramp.
        let walkable = KeeperShotReaction::travel(
            quick,
            0.63,
            KeeperShotDive::FLIGHT_TICKS,
            KeeperShotReaction::PLANT_TICKS,
        );
        // `GOAL_WIDTH` is the HALF-width, so this is "he cannot step from
        // the middle of his goal to a post while the ball is on its way".
        assert!(
            walkable < GOAL_WIDTH * 0.8,
            "he can step {walkable:.0}u inside the launch window and a post is {GOAL_WIDTH:.0}u \
             away — the dive decision is dead"
        );
        assert!(
            walkable > KeeperShotDive::STANDING_GAP,
            "…but a step has to be worth something, or he dives at balls coming straight at him"
        );
    }

    /// **A ball he cannot reach standing is one he has to leave his feet
    /// for, however straight at him it is.**
    ///
    /// The gap `should_launch` weighs was purely lateral, so a shot into
    /// the top corner from twenty-five yards — dead in line with him and
    /// two and a half metres up — read as "less than a step" and he stood
    /// and watched it go in with his arms by his sides. Measured off
    /// recordings, that was four of the twenty-three on-frame goals from
    /// 18 m+ in twelve matches, at heights of 1.95-2.55 m and lateral gaps
    /// of **7 to 26 centimetres**. See [`SaveModel::climb_gap`].
    #[test]
    fn a_ball_over_his_head_is_a_dive_however_straight_at_him_it_is() {
        let ceiling = SaveModel::SET_CEILING;
        // Chest height, dead at him: he takes it standing, and the whole
        // calibrated population of ordinary shots must be untouched.
        assert_eq!(SaveModel::climb_gap(1.2, ceiling), 0.0);
        assert_eq!(SaveModel::climb_gap(ceiling, ceiling), 0.0);
        assert_eq!(SaveModel::climb_gap(0.9, ceiling), 0.0);
        // …and a ball skidding along the floor is a dive too, which is the
        // more common half of the same defect: eleven of the twenty-nine
        // long-range goals measured arrived under 0.4 m.
        let low = SaveModel::climb_gap(0.0, ceiling);
        assert!(
            low > KeeperShotDive::STANDING_GAP,
            "a shot along the floor reads as {low:.0}u and a step is {:.0}u — he stands up \
             straight while it goes in",
            KeeperShotDive::STANDING_GAP
        );
        // …but going down is cheaper than going up.
        assert!(low < SaveModel::climb_gap(ceiling + 0.45, ceiling));
        // …under the bar is not.
        let top = SaveModel::climb_gap(GOAL_HEIGHT, ceiling);
        assert!(
            top > KeeperShotDive::STANDING_GAP,
            "a shot under his own bar reads as {top:.0}u of gap and a step is \
             {:.0}u — he will stand and watch it",
            KeeperShotDive::STANDING_GAP
        );
        // …and it must not be so expensive that it reads as hopeless: he
        // goes for the top corner, he does not give up on it.
        assert!(top < 20.0 * KeeperShotDive::DESPAIR_REACH);
        // Monotone in height, or a keeper is more willing to go for a ball
        // at head height than one under the bar.
        assert!(SaveModel::climb_gap(2.44, ceiling) > SaveModel::climb_gap(2.0, ceiling));
    }

    /// The last of a flight is not walkable at all, so a gap that opens up
    /// then can only be closed by leaving his feet. Without this the
    /// keeper decelerates into the ball on his feet and the dive is always
    /// one tick too late to be worth taking.
    #[test]
    fn nothing_is_walkable_in_the_last_moments_of_a_flight() {
        assert_eq!(
            KeeperShotReaction::travel(
                0.3,
                0.63,
                KeeperShotReaction::PLANT_TICKS,
                KeeperShotReaction::PLANT_TICKS
            ),
            0.0
        );
        assert_eq!(
            KeeperShotReaction::travel(
                0.3,
                0.63,
                KeeperShotReaction::PLANT_TICKS * 0.5,
                KeeperShotReaction::PLANT_TICKS
            ),
            0.0
        );
    }

    /// A dive is worth taking only if it covers more ground than staying up
    /// does. If these ever cross, the honest answer to every shot is to
    /// stand still, which is precisely the behaviour being fixed.
    #[test]
    fn the_dive_beats_the_step() {
        let window = KeeperShotDive::FLIGHT_TICKS;
        let walked = KeeperShotReaction::travel(
            KeeperShotReaction::QUICK_FEET,
            0.63,
            window,
            KeeperShotReaction::PLANT_TICKS,
        );
        // `GoalkeeperSpeedContext::Dive` on an ordinary keeper: 1.0-1.45 x
        // a base around 0.50 u/tick.
        let dived = 0.50 * 1.2 * window;
        assert!(
            dived > walked * 1.5,
            "a dive covers {dived:.0}u against {walked:.0}u on his feet — not worth going"
        );
    }

    /// He always reads it eventually, and a keeper who reads the game reads
    /// it sooner. A `read_by` at or past 1.0 would mean a keeper who never
    /// commits at all.
    #[test]
    fn every_keeper_reads_the_shot_before_it_arrives() {
        const { assert!(KeeperShotReaction::SLOW_READ < 0.85) };
        const { assert!(KeeperShotReaction::SHARP_READ < KeeperShotReaction::SLOW_READ) };
        const { assert!(KeeperShotReaction::SHARP_READ > 0.0) };
        // …and reading it cannot happen before he has reacted to it, or the
        // two models disagree about what he is doing with the first tenth
        // of a second.
        const { assert!(KeeperShotReaction::FAST_REACTION < KeeperShotReaction::SLOW_REACTION) };
        const { assert!(KeeperShotReaction::FAST_REACTION > 0.0) };
    }

    /// Degenerate geometry must not produce a dive into the corner flag. A
    /// shot struck from ON the goal line has no line of flight to project,
    /// and the keeper should simply defend the point it is aimed at.
    #[test]
    fn a_shot_from_the_goal_line_still_aims_at_the_goal() {
        let struck_from = Vector3::new(0.4, 250.0, 0.0);
        let point = KeeperShotDive::crossing_at(18.0, struck_from, 0.0, 290.0);
        assert_eq!(point, Vector3::new(18.0, 290.0, 0.0));
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

    /// **THE GAP HE HOLDS AND THE SPREAD HE CAN COVER ARE ONE BUDGET.**
    ///
    /// The same invariant as `COMMIT < DISENGAGE` and as
    /// `a_set_keeper_cannot_walk_to_the_corner_of_his_own_goal`, in its
    /// third form: a keeper who stands further off the ball than he can
    /// reach has committed to a duel he cannot resolve. Measured before
    /// [`KeeperOneOnOne`] existed, that is exactly what he did — 5.22 m off
    /// the ball for two full seconds while a striker dribbled from 8.7 m
    /// out to 2.8 m from goal, with `KeeperSmother` wanting it inside
    /// 1.5-3 m the whole time.
    #[test]
    fn coming_out_brings_the_ball_inside_his_own_spread() {
        let goal = Vector3::new(0.0, 272.5, 0.0);
        // An ordinary keeper's spread, and the stand-off it earns.
        let spread = KeeperSmother::SPREAD_BASE + KeeperSmother::SPREAD_SKILL * 0.5;
        let standoff = spread + KeeperOneOnOne::MARGIN;
        let limit = KeeperSweepLimit::off_line(0.5);
        // The two bounds cross at `standoff / (1 - ADVANCE)`: inside that
        // the gap IS the spread and the duel is one touch away, outside it
        // the advance cap owns the gap, which is what stops a breakaway
        // from the halfway line dragging him out to meet it.
        let crossover = standoff / (1.0 - KeeperOneOnOne::ADVANCE);
        assert!(
            crossover < KeeperOneOnOne::DANGER_DEPTH * 0.5,
            "he only reaches his own duel inside {crossover:.0}u — the far half of \
             his own area, so the smother is decided by the striker's touch and \
             never by him"
        );
        for out in [18.0_f32, 26.0, crossover] {
            let ball = Vector3::new(out, 272.5, 0.0);
            let gap = (ball - KeeperOneOnOne::stand(goal, ball, standoff, limit)).magnitude();
            assert!(
                gap <= spread + 1e-3,
                "the ball is {out}u out and he is standing {gap:.1}u off it, \
                 against a spread of {spread:.1}u — he cannot reach his own duel"
            );
        }
        // …and it always closes. A gap that stays a fixed fraction of a
        // shrinking distance still shrinks; a fixed DEPTH, which is what
        // this replaces, does not.
        let far = Vector3::new(90.0, 272.5, 0.0);
        let near = Vector3::new(50.0, 272.5, 0.0);
        let gap = |ball: Vector3<f32>| {
            (ball - KeeperOneOnOne::stand(goal, ball, standoff, limit)).magnitude()
        };
        assert!(
            gap(near) < gap(far) - 5.0,
            "the gap does not close as the man comes on: {:.1}u at 90u, {:.1}u at 50u",
            gap(far),
            gap(near)
        );
    }

    /// …and he neither backs into his own goal to hold that gap nor comes
    /// out past the space he is prepared to defend. Both bounds bind, at
    /// opposite ends of the same line.
    #[test]
    fn the_closing_point_stays_between_his_line_and_his_limit() {
        let goal = Vector3::new(0.0, 272.5, 0.0);
        let standoff = 24.0;
        let limit = KeeperSweepLimit::off_line(0.0);
        let mut previous = 0.0;
        for out in [12.0_f32, 30.0, 60.0, 120.0, 240.0, 400.0] {
            let ball = Vector3::new(out, 272.5, 0.0);
            let depth = KeeperOneOnOne::stand(goal, ball, standoff, limit).x;
            assert!(
                depth >= KeeperOneOnOne::MIN_DEPTH - 1e-3,
                "a ball {out}u out puts him {depth:.1}u off his line — inside his own goal"
            );
            assert!(
                depth <= limit + 1e-3,
                "a ball {out}u out draws him {depth:.1}u out, past his own limit of {limit:.1}u"
            );
            // And he only ever comes further as the ball comes further:
            // a target that retreats while the man advances is the defect
            // this replaces.
            assert!(
                depth >= previous - 1e-3,
                "the closing point went BACKWARDS as the ball came on: \
                 {previous:.1}u then {depth:.1}u"
            );
            previous = depth;
        }
    }
}
