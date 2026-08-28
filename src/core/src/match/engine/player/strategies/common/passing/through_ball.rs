//! **The ball that beats a line.**
//!
//! # Why the engine had never played one
//!
//! Every pass in this engine is aimed at a team-mate's *current
//! position*, and then led along his *current velocity* by
//! `handle_pass_to_event` (see the `lead_ticks` block in
//! `player/events/players.rs`). That is a ball to feet — a good model
//! for the eighty per cent of passes that are balls to feet, and
//! structurally incapable of expressing the other kind: the one played
//! into grass where nobody is standing yet, that the runner arrives onto
//! a second and a half later. The only way to aim a delivery at a POINT
//! rather than at a man is `PassingEventBuilder::with_target_point`, and
//! before this module its three call sites were all crosses.
//!
//! The one function that claimed otherwise — `find_breakthrough_pass_option`
//! in `midfielders/states/passing` — required
//! `would_pass_break_defensive_lines`, which returns **false unless at
//! least two opponents are standing in the passing lane**. So it fired
//! only for a ball played *through a crowd*, never for a ball slid into
//! an empty channel, which is what a through ball is. And the two PPMs
//! written for it, `TriesThroughBalls` and `KillerBallOften`, set
//! `PassingBias::through_ball_bonus`, which nothing in the engine read.
//!
//! # The model
//!
//! A through ball is a **meeting point**, and everything else follows
//! from solving for it:
//!
//! 1. Take the runner's line — his own run if he is already moving, the
//!    line to goal if he is on the shoulder waiting.
//! 2. Solve where he and the ball can meet: iterate the flight time
//!    against the ground he covers in it. This is what makes the ball
//!    land in front of him rather than at his feet, and it means the
//!    pass is weighted by *his* pace, not the passer's optimism.
//! 3. He must be **onside when it is struck** — the whole art of the
//!    pass. The meeting point may be well beyond the last defender; he
//!    may not be.
//! 4. He must **win the race to it**, against the nearest defender and
//!    against the goalkeeper, or it is a pass to the keeper.
//! 5. There must be a **corridor** to play it through, narrowing with
//!    how well the passer strikes a ball.
//!
//! What comes out is scored on how much of the pitch and how many
//! defenders the pass removes, and compared against a bar drawn **once
//! per possession** — see [`Opportunity`]. A player makes his mind up
//! about a ball; he does not re-roll it a hundred times a second.

use crate::club::player::registry::has_risk_tolerant_passing_trait;
use crate::r#match::engine::ball::ball::OffsideLine;
use crate::r#match::midfielders::states::common::Opportunity;
use crate::r#match::player::strategies::players::ops::skill::traits_bias::passing_bias;
use crate::r#match::{MatchPlayerLite, PlayerSide, StateProcessingContext};
use nalgebra::Vector3;

/// Engine units per metre — the pitch is 840u x 545u, i.e. 105 m x 68 m.
const U_PER_M: f32 = 8.0;

/// Per-call-site salt for [`Opportunity`], so declining a through ball
/// is not also declining a shot or a take-on.
const THROUGH_BALL_SALT: u64 = 0x7A1F_9C2D_45B6_E830;

/// What kind of ball this is — for the pass reason, the census and the
/// scoring, which values them differently.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThroughBallKind {
    /// Played beyond the last line, for a runner going in behind.
    InBehind,
    /// Played into the space in front of a runner who is still in front
    /// of the line — the ball down the channel, or into the half-space
    /// between full-back and centre-half.
    IntoSpace,
}

impl ThroughBallKind {
    /// Stable tag for the pass event, so the census can tell the two
    /// apart in `SHOTS BY KIND` and the key-pass ledger.
    pub fn reason(self) -> &'static str {
        match self {
            ThroughBallKind::InBehind => "MID_THROUGH_BALL_IN_BEHIND",
            ThroughBallKind::IntoSpace => "MID_THROUGH_BALL_SPACE",
        }
    }
}

/// A ball worth playing, and where to put it.
#[derive(Debug, Clone, Copy)]
pub struct ThroughBallDecision {
    /// The man it is for. He is forced into the chase by the dispatcher
    /// (`should_force_takeball` reads `pass_target`), which is what makes
    /// a pass into empty grass work at all.
    pub target_id: u32,
    /// Where the ball is going — a point on the pitch, not a player.
    pub aim_point: Vector3<f32>,
    pub kind: ThroughBallKind,
    /// 0..1-ish, how good the ball is. Exposed so a caller can rank it
    /// against its other options rather than taking it on sight.
    pub value: f32,
}

/// Finds the ball that beats a line.
pub struct ThroughBall;

impl ThroughBall {
    /// Furthest a through ball is worth attempting (~45 m) and the
    /// shortest that is meaningfully one (~9 m — below that it is a
    /// square ball and the ordinary evaluator owns it).
    const MAX_RANGE: f32 = 45.0 * U_PER_M;
    const MIN_RANGE: f32 = 9.0 * U_PER_M;

    /// How far in front of the runner the ball may be put, at the two
    /// ends. Below the minimum it is a pass to feet; beyond the maximum
    /// it is a hopeful punt that the covering defender reads all day.
    const MIN_LEAD: f32 = 5.0 * U_PER_M;
    const MAX_LEAD: f32 = 22.0 * U_PER_M;

    /// A runner already at pace is running his own line; one standing
    /// still is going to run at the goal. This is the speed at which the
    /// engine trusts his current direction completely (~1.5 m/s).
    const RUNNING: f32 = 0.12;

    /// Top speed of a runner, in u/tick, at the two ends of the pace
    /// range. Matches the outfield ceiling the movement model actually
    /// produces (measured 0.395 u/tick mean ceiling).
    const SPEED_SLOW: f32 = 0.30;
    const SPEED_FAST: f32 = 0.46;

    /// Half-width of the corridor the ball has to travel down, before
    /// the passer's quality narrows it (~2.6 m). A defender inside it,
    /// between the passer and the meeting point, kills the ball.
    const CORRIDOR: f32 = 2.6 * U_PER_M;

    /// How much of the corridor an elite striker of a ball can do
    /// without — he bends it round the man, or drives it past him before
    /// he can turn.
    const CORRIDOR_RELIEF: f32 = 0.45;

    /// The meeting point has to be this much closer to goal than the
    /// runner already is (~6 m), or the pass has not advanced anything.
    const MIN_GAIN: f32 = 6.0 * U_PER_M;

    /// …and the runner has to beat the nearest defender to it by this
    /// margin in ticks (a third of a second). Below it the ball is a
    /// 50-50, which is not what this pass is for.
    const RACE_MARGIN: f32 = 33.0;

    /// Keep the meeting point out of the goalkeeper's easy reach: a ball
    /// played inside this radius of him is a pass to the keeper however
    /// good it looks on paper (~9 m).
    const KEEPER_KEEPOUT: f32 = 9.0 * U_PER_M;

    /// Bar the value is compared against, and how much of it the
    /// player's licence buys back. Base sits high because this is the
    /// pass that loses possession when it is wrong.
    ///
    /// Fitted on `dev_match stats 140 14 14`, reading the on-ball
    /// census. At 0.46 the ball fired **6 832 times over 140 matches —
    /// 24 a team a match**, which is not a through ball, it is the
    /// midfield's default pass; a real side plays a handful. 0.60 lands
    /// it where the football is: the ball that beats a line is a thing
    /// that happens a few times in a half and changes the game when it
    /// does.
    const BAR_BASE: f32 = 0.60;
    const BAR_SPREAD: f32 = 0.30;
    const LICENCE_RELIEF: f32 = 0.30;

    /// The best ball on, if there is one.
    ///
    /// `licence` is the caller's view of how much this player is
    /// *supposed* to be looking for it — a role licence (see
    /// `MidfieldRole::creation`), not a skill. Skill is priced inside,
    /// in the corridor and the weight of the pass.
    pub fn find(ctx: &StateProcessingContext, licence: f32) -> Option<ThroughBallDecision> {
        let side = ctx.player.side?;
        let from = ctx.player.position;
        let goal = ctx.player().opponent_goal_position();
        let my_goal_distance = (goal - from).magnitude();

        // **The line, read once.** A property of the defence, not of the
        // man being considered — and the same line the referee will use,
        // which is the only way "he was onside when it was played" can
        // mean anything. `None` on a restart that exempts the receiver.
        let offside_line = if ctx
            .tick_context
            .ball
            .pass_origin_restart
            .is_offside_exempt()
        {
            None
        } else {
            OffsideLine::second_last(ctx.players().opponents().all().map(|o| o.position.x), side)
        };
        let ball_x = ctx.tick_context.positions.ball.position.x;

        // How cleanly he strikes it — narrows the corridor he needs and
        // lengthens the ball he can hit. Traits enter HERE, which is the
        // first time `through_ball_bonus` has reached anything.
        let s = &ctx.player.skills;
        let strike = ((s.technical.passing / 20.0) * 0.45
            + (s.technical.technique / 20.0) * 0.25
            + (s.mental.vision / 20.0) * 0.30)
            .clamp(0.0, 1.0);
        let bias = passing_bias(ctx.player);
        let appetite =
            (licence + bias.through_ball_bonus + bias.risky_central_pass_bonus).clamp(0.0, 1.0);
        let corridor = Self::CORRIDOR * (1.0 - strike * Self::CORRIDOR_RELIEF);

        let keeper = ctx
            .players()
            .opponents()
            .all()
            .find(|o| o.tactical_positions.is_goalkeeper())
            .map(|o| o.position);

        let mut best: Option<ThroughBallDecision> = None;
        for runner in ctx.players().teammates().nearby(Self::MAX_RANGE) {
            if runner.tactical_positions.is_goalkeeper() {
                continue;
            }
            let Some(candidate) = Self::solve(
                ctx,
                side,
                from,
                goal,
                my_goal_distance,
                &runner,
                offside_line,
                ball_x,
                corridor,
                keeper,
            ) else {
                continue;
            };
            if best.is_none_or(|b| candidate.value > b.value) {
                best = Some(candidate);
            }
        }

        let candidate = best?;
        // One draw per possession, salted away from every other decision
        // on the tree — a player who declines this ball does not re-ask
        // on the next tick and walk his own bar down.
        let spread = Opportunity::draw(ctx, THROUGH_BALL_SALT);
        let bar = Self::BAR_BASE + spread * Self::BAR_SPREAD - appetite * Self::LICENCE_RELIEF;
        (candidate.value >= bar).then_some(candidate)
    }

    /// Solve the meeting point for one runner and score it, or reject
    /// him. Every rejection below is a football reason, in the order a
    /// passer would run out of patience with them.
    #[allow(clippy::too_many_arguments)]
    fn solve(
        ctx: &StateProcessingContext,
        side: PlayerSide,
        from: Vector3<f32>,
        goal: Vector3<f32>,
        my_goal_distance: f32,
        runner: &MatchPlayerLite,
        offside_line: Option<f32>,
        ball_x: f32,
        corridor: f32,
        keeper: Option<Vector3<f32>>,
    ) -> Option<ThroughBallDecision> {
        let runner_pos = runner.position;
        let to_runner = runner_pos - from;
        let range = to_runner.magnitude();
        if !(Self::MIN_RANGE..=Self::MAX_RANGE).contains(&range) {
            return None;
        }

        // **Onside when it is struck.** He may run beyond the line; he
        // may not already be beyond it. Rejecting here rather than
        // shortening the ball is deliberate — a man who has strayed
        // offside is not an option, he is a problem for him to solve.
        if let Some(line_x) = offside_line {
            if OffsideLine::is_beyond(side, runner_pos.x, ball_x, line_x) {
                return None;
            }
        }

        // He must be ahead of the passer to begin with. A through ball
        // played backwards is a contradiction.
        let runner_goal_distance = (goal - runner_pos).magnitude();
        if runner_goal_distance >= my_goal_distance {
            return None;
        }

        // ── His line ─────────────────────────────────────────────────
        // A man at pace is running his own line and the ball is played
        // to where that line is going. A man on the shoulder is going to
        // run at the goal, and the ball is played into the space in
        // front of him. Blended on his actual speed so neither case is a
        // special case.
        let velocity = ctx.tick_context.positions.players.velocity(runner.id);
        let speed = velocity.magnitude();
        let to_goal = (goal - runner_pos)
            .try_normalize(1.0e-4)
            .unwrap_or_else(|| Vector3::new(side.forward_dir_x(), 0.0, 0.0));
        let run_dir = if speed > 1.0e-3 {
            let w = (speed / Self::RUNNING).clamp(0.0, 1.0);
            let v = velocity / speed;
            (v * w + to_goal * (1.0 - w))
                .try_normalize(1.0e-4)
                .unwrap_or(to_goal)
        } else {
            to_goal
        };
        // Only a forward run is a through-ball run.
        if run_dir.dot(&to_goal) < 0.30 {
            return None;
        }

        // ── The meeting point ────────────────────────────────────────
        // Two passes of the fixed point: guess the flight time to his
        // feet, see how far he travels in it, re-time the longer ball.
        // Converges immediately at these distances and costs two square
        // roots.
        let runner_pace = (ctx.player().skills(runner.id).physical.pace / 20.0).clamp(0.0, 1.0);
        let runner_speed = Self::SPEED_SLOW + runner_pace * (Self::SPEED_FAST - Self::SPEED_SLOW);
        let mut aim = runner_pos;
        for _ in 0..2 {
            let flight = Self::flight_ticks((aim - from).magnitude());
            let lead = (runner_speed * flight).clamp(Self::MIN_LEAD, Self::MAX_LEAD);
            aim = runner_pos + run_dir * lead;
        }
        // Keep it on the pitch, and off the goal line — a ball played
        // through the back of the six-yard box is a goal kick.
        let field_width = ctx.context.field_size.width as f32;
        let field_height = ctx.context.field_size.height as f32;
        let margin = 3.0 * U_PER_M;
        aim.x = aim.x.clamp(margin, field_width - margin);
        aim.y = aim.y.clamp(margin, field_height - margin);
        aim.z = 0.0;

        // ── Is it worth playing? ─────────────────────────────────────
        let aim_goal_distance = (goal - aim).magnitude();
        if runner_goal_distance - aim_goal_distance < Self::MIN_GAIN {
            return None;
        }

        // A ball into the keeper's arms is not a through ball.
        if let Some(gk) = keeper {
            if (aim - gk).magnitude() < Self::KEEPER_KEEPOUT {
                return None;
            }
        }

        // ── The race ─────────────────────────────────────────────────
        // He has to get there first, against the nearest defender who
        // can also see it. Their speed is not read individually — this
        // is the passer's judgement of a race, and a passer judges it
        // off distance.
        let runner_travel = (aim - runner_pos).magnitude() / runner_speed;
        let mut defender_travel = f32::INFINITY;
        let mut cut_off = 0usize;
        for opponent in ctx.players().opponents().all() {
            if opponent.tactical_positions.is_goalkeeper() {
                continue;
            }
            let d = (aim - opponent.position).magnitude();
            let t = d / Self::SPEED_FAST;
            if t < defender_travel {
                defender_travel = t;
            }
            // How many of them the ball takes out of the game: goal-side
            // of the passer now, and behind the meeting point after.
            let theirs = (goal - opponent.position).magnitude();
            if theirs < my_goal_distance && theirs > aim_goal_distance {
                cut_off += 1;
            }
        }
        if runner_travel > defender_travel - Self::RACE_MARGIN {
            return None;
        }

        // ── The corridor ─────────────────────────────────────────────
        // Nobody standing in the way of it. Measured to the meeting
        // point, not to the runner, because that is the line the ball
        // takes.
        let lane = aim - from;
        let lane_len = lane.magnitude();
        let lane_dir = lane.try_normalize(1.0e-4)?;
        for opponent in ctx.players().opponents().all() {
            let offset = opponent.position - from;
            let along = offset.dot(&lane_dir);
            if along <= 0.0 || along >= lane_len {
                continue;
            }
            let lateral = (offset - lane_dir * along).magnitude();
            // A defender only has to move a step to cut the ball out, so
            // the corridor he blocks widens with how long the ball is in
            // front of him.
            let reach = corridor * (1.0 + (along / lane_len) * 0.35);
            if lateral < reach {
                return None;
            }
        }

        // ── Score it ─────────────────────────────────────────────────
        // Ground gained toward goal, defenders removed, and how much
        // room the meeting point has around it — the three things that
        // make one of these balls better than another.
        let gained =
            ((my_goal_distance - aim_goal_distance) / (field_width * 0.45)).clamp(0.0, 1.0);
        let removed = (cut_off as f32 / 4.0).clamp(0.0, 1.0);
        let crowding = ctx
            .players()
            .opponents()
            .nearby_at(aim, 8.0 * U_PER_M)
            .count() as f32;
        let room = 1.0 / (1.0 + crowding);
        // Arriving in a shooting position is what separates a good ball
        // from a merely progressive one.
        let arrival = 1.0 - (aim_goal_distance / (field_width * 0.32)).clamp(0.0, 1.0);

        let value = (gained * 0.34 + removed * 0.26 + room * 0.22 + arrival * 0.18).clamp(0.0, 1.0);

        let kind = match offside_line {
            Some(line_x) if OffsideLine::is_beyond(side, aim.x, ball_x, line_x) => {
                ThroughBallKind::InBehind
            }
            _ => ThroughBallKind::IntoSpace,
        };

        Some(ThroughBallDecision {
            target_id: runner.id,
            aim_point: aim,
            kind,
            value,
        })
    }

    /// Ticks a ground pass of `distance` units takes to arrive.
    ///
    /// Mirrors `PlayerEvents::calculate_horizontal_velocity`, which is
    /// what will actually strike the ball: delivery speed rises with
    /// distance and rails at both ends. Kept as its own function so the
    /// two can be compared by reading — if the strike model changes and
    /// this does not, the meeting point silently stops meeting.
    #[inline]
    fn flight_ticks(distance: f32) -> f32 {
        const BASE_SPEED: f32 = 0.55;
        const SPEED_PER_UNIT: f32 = 0.0028;
        let speed = (BASE_SPEED + distance * SPEED_PER_UNIT).clamp(0.50, 2.20);
        (distance / speed).max(1.0)
    }

    /// Does this player have a risk-tolerant passing PPM? Exposed for
    /// callers that want to widen a bar for the killer-ball merchants
    /// without re-deriving the registry lookup.
    #[inline]
    pub fn is_risk_tolerant(ctx: &StateProcessingContext) -> bool {
        has_risk_tolerant_passing_trait(&ctx.player.traits)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The meeting point must be *ahead* of the runner and the flight
    /// time must be the thing that decides how far ahead. A fixed lead
    /// is what makes a through ball either a pass to feet (short) or a
    /// goal kick (long), and both were the engine's only two options
    /// before this.
    #[test]
    fn a_longer_ball_is_led_further() {
        let near = ThroughBall::flight_ticks(10.0 * U_PER_M);
        let far = ThroughBall::flight_ticks(35.0 * U_PER_M);
        assert!(far > near, "{far} vs {near}");
        // …and the lead each implies stays inside the rails.
        for flight in [near, far] {
            let lead = (0.38 * flight).clamp(ThroughBall::MIN_LEAD, ThroughBall::MAX_LEAD);
            assert!((ThroughBall::MIN_LEAD..=ThroughBall::MAX_LEAD).contains(&lead));
        }
    }

    /// The flight model has to agree with the strike model it is
    /// predicting, or the ball never arrives where the solve put it.
    #[test]
    fn flight_matches_the_strike_model() {
        for d in [20.0f32, 80.0, 160.0, 320.0] {
            let speed = (0.55 + d * 0.0028).clamp(0.50, 2.20);
            assert!((ThroughBall::flight_ticks(d) - d / speed).abs() < 1e-3);
        }
    }

    /// Rails: a through ball is neither a square ball nor a punt.
    #[test]
    fn the_range_is_a_footballing_one() {
        assert!(ThroughBall::MIN_RANGE / U_PER_M >= 6.0);
        assert!(ThroughBall::MAX_RANGE / U_PER_M <= 55.0);
        assert!(ThroughBall::MIN_LEAD < ThroughBall::MAX_LEAD);
    }
}
