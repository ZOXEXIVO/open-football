//! Where a goalkeeper stands for a dead ball at his own goal, and how he
//! faces a penalty.
//!
//! # Why this exists
//!
//! The rest model ([`KeeperRestPosition`]) is a model of OPEN PLAY: depth
//! rises with the ball's distance because a ball far away can be played
//! in behind and a sweeper is the man to meet it. A dead ball breaks that
//! reasoning. Nothing is coming in behind — the taker is standing over it
//! — and the one delivery that is coming will arrive from a known place
//! at a time nobody can read early. So a keeper does what every keeper
//! does at a set piece: he goes back to his line and sets there.
//!
//! Measured before this, with the ball on the corner arc the rest model
//! put him **8-9 m off his line**, drifting toward the near post, and at a
//! penalty [`KeeperOneOnOne`] read the taker standing over the ball as a
//! carrier bearing down on him — a stationary man has no velocity, and
//! the "running at goal" test is vacuously true of a body that is not
//! running anywhere — and steered him **6.8 m off his line** while the
//! penalty was being placed. `Standing`'s sweep gate could then send him
//! to `ComingOut` after the dead ball itself: a keeper standing beside the
//! penalty spot while the taker walked up.
//!
//! # The model
//!
//! One question, [`KeeperSetPieceStance::pending`]: *is there a dead ball
//! at my goal that has not been struck yet?* While there is, the stance
//! point replaces the rest point in every state that rests, and every
//! gate that would take him off his line for a carrier or a loose ball
//! stands down. The moment the ball moves the stance is over and the
//! ordinary reads — the aerial claim, the shot, the sweep — take over.
//!
//! * **Corner.** A metre off his line, a third of the way toward the far
//!   post: the near post is his defenders' and an inswinger drops at the
//!   far one.
//! * **Direct free kick in range.** A metre and a half off his line on the
//!   side the wall does not cover.
//! * **Penalty.** ON the line, dead centre — Law 14 — and then the dive is
//!   a decision made at the strike rather than a reaction to the flight.
//!   See [`KeeperPenaltyStance`].
//!
//! `OF_KEEPER_SETPIECE=off` is the A/B control: the old rest model
//! everywhere, the old late reaction at penalties.

use crate::r#match::engine::ball::ball::DeadBall;
use crate::r#match::engine::goal::GOAL_WIDTH;
use crate::r#match::goalkeepers::states::common::{
    KeeperRestPosition, KeeperSetPosition, KeeperShotReaction, KeeperShotSave,
};
use crate::r#match::goalkeepers::states::state::GoalkeeperState;
use crate::r#match::player::state::PlayerState;
use crate::r#match::player::strategies::players::ops::goalkeeper_skill::GoalkeeperSkillProfile;
use crate::r#match::{
    PassOriginRestart, StateChangeResult, StateProcessingContext, SteeringBehavior,
};
use nalgebra::Vector3;

/// The dead balls a keeper takes a stance for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SetPiece {
    Corner,
    DirectFreeKick,
    Penalty,
}

pub struct KeeperSetPieceStance;

impl KeeperSetPieceStance {
    /// Off his line at a corner, in units. 8u = 1 m: close enough to
    /// cover the line against a ball that dips under the bar, far enough
    /// that a step takes him to a cross dropping on the six-yard line.
    const CORNER_DEPTH: f32 = 8.0;
    /// …and at a direct free kick. 12u = 1.5 m. He cannot read a dead-ball
    /// strike early and a chip over the wall is the ball that beats a
    /// keeper who has come out, so he sets deep and reacts.
    const FREE_KICK_DEPTH: f32 = 12.0;
    /// …and at a penalty: on the line. Law 14 wants a foot on it until
    /// the ball is kicked. 2u keeps him a quarter of a metre in front of
    /// the goal line so a caught ball cannot be inside the frame.
    const PENALTY_DEPTH: f32 = 2.0;
    /// A free kick further out than this is not a set piece he sets for —
    /// it is a delivery, and the rest model already handles those. 280u =
    /// 35 m, about the range a direct strike is still on.
    const FREE_KICK_RANGE: f32 = 280.0;
    /// How far toward the far post he hedges at a corner, as a share of
    /// the goal's half-width. A third: the middle of the goal is where a
    /// keeper is coached to start and the far post is where an inswinger
    /// ends up.
    const CORNER_FAR_POST_BIAS: f32 = 0.33;
    /// …and at a free kick, where the wall has the near side. A quarter.
    const FREE_KICK_FAR_BIAS: f32 = 0.25;
    /// A ball within this of the goal's centre line is central and has no
    /// far post. 12u = 1.5 m.
    const CENTRAL: f32 = 12.0;
    /// Below this the dead ball has not been struck, in units per tick. A
    /// taker's first touch on a corner leaves at 1.0-2.5.
    const UNSTRUCK: f32 = 0.30;
    /// He is on his mark within this: 4u across the goal (half a metre)
    /// and 6u in depth. Tighter than the rest model's tolerances on
    /// purpose — those open with the ball's distance because a far ball
    /// gives him time, and this is the one far ball that does not.
    const ACROSS_TOLERANCE: f32 = 4.0;
    const DEPTH_TOLERANCE: f32 = 6.0;

    /// False when `OF_KEEPER_SETPIECE=off`.
    pub fn armed() -> bool {
        use std::sync::OnceLock;
        static ON: OnceLock<bool> = OnceLock::new();
        *ON.get_or_init(|| std::env::var("OF_KEEPER_SETPIECE").as_deref() != Ok("off"))
    }

    /// The dead ball at his goal that has not been struck, if there is
    /// one.
    ///
    /// "At his goal" is the restart being the OPPOSITION's and the ball
    /// being in his half; "not struck" is the ball being out of play
    /// ([`DeadBall`]) or standing still at an opponent's feet — the corner
    /// taker waiting for his box to fill, the penalty taker over the
    /// spot. The instant it moves this is `None` and the open-play model
    /// has him back.
    pub fn pending(ctx: &StateProcessingContext) -> Option<SetPiece> {
        if !Self::armed() {
            return None;
        }
        let kind = match ctx.tick_context.ball.pass_origin_restart {
            PassOriginRestart::Corner => SetPiece::Corner,
            PassOriginRestart::DirectFreeKick => SetPiece::DirectFreeKick,
            PassOriginRestart::Penalty => SetPiece::Penalty,
            _ => return None,
        };
        if !ctx.ball().on_own_side() {
            return None;
        }
        let ball = &ctx.tick_context.positions.ball;
        let theirs = |id: u32| ctx.players().opponents().all().any(|p| p.id == id);
        let dead = DeadBall::taker(ctx.tick_context.ball.restart_taker).is_some_and(theirs);
        let over_it = ball.velocity.norm() < Self::UNSTRUCK
            && ctx
                .tick_context
                .ball
                .current_owner
                .is_some_and(theirs);
        if !dead && !over_it {
            return None;
        }
        if kind == SetPiece::DirectFreeKick {
            let goal = ctx.ball().direction_to_own_goal();
            let range = Vector3::new(ball.position.x - goal.x, ball.position.y - goal.y, 0.0);
            if range.norm() > Self::FREE_KICK_RANGE {
                return None;
            }
        }
        Some(kind)
    }

    /// Where he stands for it, or `None` when there is nothing to stand
    /// for.
    pub fn point(ctx: &StateProcessingContext) -> Option<Vector3<f32>> {
        let kind = Self::pending(ctx)?;
        let goal = ctx.ball().direction_to_own_goal();
        let into = KeeperSetPosition::into_pitch(goal, ctx.context.field_size.width as f32);
        let ball = ctx.tick_context.positions.ball.position;
        // Toward the post the ball is NOT on.
        let far_sign = if (ball.y - goal.y).abs() < Self::CENTRAL {
            0.0
        } else if ball.y > goal.y {
            -1.0
        } else {
            1.0
        };
        let (depth, across) = match kind {
            SetPiece::Corner => (
                Self::CORNER_DEPTH,
                far_sign * GOAL_WIDTH * Self::CORNER_FAR_POST_BIAS,
            ),
            SetPiece::DirectFreeKick => (
                Self::FREE_KICK_DEPTH,
                far_sign * GOAL_WIDTH * Self::FREE_KICK_FAR_BIAS,
            ),
            SetPiece::Penalty => (Self::PENALTY_DEPTH, 0.0),
        };
        Some(Vector3::new(goal.x + into * depth, goal.y + across, 0.0))
    }

    /// Is he on his mark?
    pub fn is_set(keeper: Vector3<f32>, mark: Vector3<f32>) -> bool {
        (keeper.y - mark.y).abs() < Self::ACROSS_TOLERANCE
            && (keeper.x - mark.x).abs() < Self::DEPTH_TOLERANCE
    }

    /// The steering to his mark, if there is a dead ball to set for:
    /// nothing once he is on it, otherwise an `Arrive` at the pace the
    /// rest model uses for the same ball distance — a walk to the line at
    /// a corner, a jog at a penalty.
    ///
    /// Every resting state asks this FIRST, ahead of its own rest point,
    /// so the stance cannot be argued with by a tolerance written for a
    /// ball that is moving.
    pub fn steer(ctx: &StateProcessingContext) -> Option<Vector3<f32>> {
        let mark = Self::point(ctx)?;
        if Self::is_set(ctx.player.position, mark) {
            return Some(Vector3::zeros());
        }
        let pace =
            KeeperRestPosition::pace(ctx.ball().distance(), ctx.context.field_size.width as f32);
        Some(
            SteeringBehavior::Arrive {
                target: mark,
                slowing_distance: 6.0,
            }
            .calculate(ctx.player)
            .velocity
                * pace,
        )
    }
}

/// **The penalty is a guess, and he makes it at the strike.**
///
/// # Why this exists
///
/// A penalty is eleven metres and a third of a second. Nothing in a
/// keeper's reaction model survives that: [`KeeperShotReaction`] holds
/// him still for his reaction time and then lets him read the flight over
/// the first third of it, which at a penalty means he moves when the ball
/// is already at the line. Real keepers do not try. They read the run-up
/// and go as the boot comes through — a committed, full-length dive to
/// one corner, right about six times in ten — and that picture is the
/// most recognisable in football. Measured before this: on a penalty the
/// keeper waited out his reaction on his line and then stepped.
///
/// # The model
///
/// At the strike he commits to a side. He is right with probability
/// `0.5 + edge`, where the edge is his positioning composite — reading a
/// taker's hips and plant foot is anticipation, which is the composite's
/// second-heaviest term — centred on the population so an ordinary keeper
/// reads a penalty like a coin with a thumb on it. The side he picks is
/// where he dives, full length ([`Self::CORNER`]), and from then on the
/// save is adjudicated exactly as every other shot is: the physics
/// measures where his body is against where the ball crosses, so a
/// correct guess is a save roll and a wrong one is a goal. Conversion is
/// therefore not a number written here; it is what the geometry gives.
///
/// The guess is rolled ONCE, on the tick he commits, and carried on the
/// player (`MatchPlayer::dive_aim`) for the life of the dive — the dive
/// re-aims every tick and a guess re-rolled every tick is a keeper
/// changing his mind in mid-air.
pub struct KeeperPenaltyStance;

impl KeeperPenaltyStance {
    /// The penalty spot, in units from the goal line. The award writes
    /// 88u (11 m) — see `FoulResolver::award_restart_for_foul`.
    const SPOT_DEPTH: f32 = 88.0;
    /// A strike from further than this off the spot is not a penalty
    /// kick, whatever the origin still says. 12u = 1.5 m.
    const SPOT_TOLERANCE: f32 = 12.0;
    /// How often an ordinary keeper picks the right side. 0.60 is the
    /// published figure for elite keepers guessing the side of the kick;
    /// the population here is the whole game, so a shade under.
    const BASE_READ: f32 = 0.58;
    /// …and how far his positioning composite moves that either way.
    /// ±0.10 at the ends of the range: the best readers of a run-up get
    /// two in three, the worst are a coin.
    const READ_SPREAD: f32 = 0.20;
    /// Where the dive is aimed, as a distance from the centre of the
    /// goal: 24u = 3 m, the inside of the post. A keeper who has guessed
    /// goes to the corner, not to some hedge between the corner and the
    /// middle.
    const CORNER: f32 = 24.0;

    /// Is the shot in flight a penalty kick at his goal?
    pub fn facing(ctx: &StateProcessingContext) -> bool {
        if !KeeperSetPieceStance::armed() {
            return false;
        }
        if ctx.tick_context.ball.pass_origin_restart != PassOriginRestart::Penalty {
            return false;
        }
        let Some(target) = ctx.tick_context.ball.cached_shot_target.as_ref() else {
            return false;
        };
        if Some(target.defending_side) != ctx.player.side {
            return false;
        }
        let goal = ctx.ball().direction_to_own_goal();
        let into = KeeperSetPosition::into_pitch(goal, ctx.context.field_size.width as f32);
        let spot = Vector3::new(goal.x + into * Self::SPOT_DEPTH, goal.y, 0.0);
        let from = Vector3::new(target.struck_from.x, target.struck_from.y, 0.0);
        (from - spot).norm() < Self::SPOT_TOLERANCE
    }

    /// The dive, if this is the tick to go: a transition to `Diving` that
    /// carries the side he has guessed. `None` for every other shot, and
    /// once he has gone.
    pub fn commit(ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        if !Self::facing(ctx) || ctx.player.dive_aim.is_some() {
            return None;
        }
        if matches!(
            ctx.player.state,
            PlayerState::Goalkeeper(GoalkeeperState::Diving)
        ) {
            return None;
        }
        // The strike has to have happened — `facing` only says a penalty
        // is cached. The reaction clock reads the ball's flown distance,
        // so a finite answer is a ball on its way.
        if !KeeperShotReaction::since_strike(ctx).is_finite() {
            return None;
        }
        let goal = ctx.ball().direction_to_own_goal();
        let ball = &ctx.tick_context.positions.ball;
        // Which side it is REALLY going, off the ball's own line — not his
        // read of the crossing point, which is the thing being modelled.
        let truth = if ball.velocity.y.abs() < 1e-3 {
            // Dead centre: whichever way he goes is a guess he loses.
            if ctx.context.rng.unit_f32() < 0.5 { -1.0 } else { 1.0 }
        } else {
            ball.velocity.y.signum()
        };
        let prof = GoalkeeperSkillProfile::from_ctx(ctx);
        let read = Self::BASE_READ
            + (prof.positioning.clamp(0.0, 1.0) - GoalkeeperSkillProfile::POPULATION_READ)
                * Self::READ_SPREAD;
        let side = if ctx.context.rng.unit_f32() < read.clamp(0.5, 0.75) {
            truth
        } else {
            -truth
        };
        // Full length, to the corner — bounded by his own reach so a heavy
        // keeper is not asked to travel further than his dive can.
        let corner = Self::CORNER.min(KeeperShotSave::base_reach(&prof) * 1.4);
        let mut result = StateChangeResult::with_goalkeeper_state(GoalkeeperState::Diving);
        result.dive_aim = Some(goal.y + side * corner);
        #[cfg(feature = "match-logs")]
        crate::mid_run_diag::KeeperActionDiag::note(18);
        Some(result)
    }
}
