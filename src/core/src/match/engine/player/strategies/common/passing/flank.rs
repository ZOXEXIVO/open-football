//! **What a footballer does with the ball in a wide area** — decided by
//! where he is standing and nothing else.
//!
//! # Why this is positional and carries no role check
//!
//! The team plan ([`wide`](crate::r#match::engine::teamplay::plans::wide))
//! decides who *goes* to a touchline, because nothing else in the engine
//! ever would — every other force pulls a player toward the ball and
//! toward the middle. But what a man *does* once he is there is not an
//! assignment in football and must not be one here: a centre-half who
//! ends up on the right touchline with the ball at his feet in the 89th
//! minute looks for the same three things a winger does.
//!
//! So every test below reads the player's position, the ball's position
//! and the opposition. None of them reads the plan. The plan supplies the
//! bodies; geometry supplies the behaviour.
//!
//! # The ladder
//!
//! In the order a footballer actually considers them once he is wide and
//! high:
//!
//! 1. **Release the man outside you.** Somebody has run past you on the
//!    outside; he is in more space than you and closer to the byline.
//!    Giving it is one touch and it is the best ball on the pitch.
//! 2. **Deliver.** There is a runner attacking the box and a lane to put
//!    the ball in front of him. [`CrossModel`] already owns *which*
//!    delivery — floated, whipped, driven or pulled back — so this rung
//!    only decides *whether*.
//! 3. **Drive the byline.** Nothing is on yet, but the grass between the
//!    full-back and the goal line is. Carrying into it is what creates
//!    rungs 1 and 2 a second later, and it is the difference between a
//!    wide player and a man who happens to be standing wide.
//!
//! Rung 3 is not a state change — it is a change of *aim*, applied to
//! the carry the player was making anyway ([`Self::carry_aim`]). Every
//! ball-carrying state in this engine drives at the goal, which for a man
//! on the touchline means cutting infield into the crowd. That single
//! substitution is most of the difference between "he ran inside and lost
//! it" and "he got to the byline".

use crate::r#match::player::strategies::common::passing::CrossModel;
use crate::r#match::{MatchPlayerLite, StateProcessingContext};
use nalgebra::Vector3;

/// What the wide area is offering the carrier.
#[derive(Debug, Clone, Copy)]
pub enum FlankAction {
    /// Give it to the man who has run outside you.
    ReleaseOutside { target: u32 },
    /// Put it in the box.
    Deliver,
}

/// The wide carrier's decision, and the aim that makes it possible.
pub struct FlankPlay;

impl FlankPlay {
    /// How far up the pitch a delivery becomes the natural next action
    /// for a player who can genuinely cross. 0.64 of the way to goal —
    /// a shade short of the final third, because an early ball from the
    /// edge of it is a real delivery and [`CrossModel`] has a type for
    /// it.
    ///
    /// 0.66 was measured and reverted. Two metres of pitch is nothing to
    /// a winger and everything to a **full-back**, who arrives deeper by
    /// definition and is additionally pushed deeper again by
    /// [`Self::DELIVERY_PATIENCE`] (his crossing is usually the lower of
    /// the two). At 0.66 defenders struck 7% of all deliveries against a
    /// real ~25%, and the early-ball share of the mix collapsed to 2%
    /// against a real 8-12% — the engine had quietly deleted the
    /// full-back's cross, which is the very thing the overlap exists to
    /// produce. Volume is bounded by the box-occupancy and lane rules
    /// below, which are football; depth is not the place to do it.
    const DELIVERY_PROGRESS: f32 = 0.64;

    /// How near the goal a team-mate has to be to count as being IN the
    /// box for the purposes of aiming at him. 170u ≈ 21 m — the penalty
    /// area and the yard or two of approach a runner arrives from.
    const BOX_RANGE: f32 = 170.0;

    /// …and how many of them there have to be.
    ///
    /// This is the throttle, and it is a football rule rather than a
    /// dial: you do not cross into an empty box. A winger who gets to
    /// the touchline with one man in the middle holds it up, comes back
    /// inside, or plays it round the corner — and the reason it is worth
    /// stating is that without it the engine crossed **26.8 times a team
    /// a match against a real 16-18**, because reaching the channel was
    /// treated as sufficient reason to deliver.
    ///
    /// From the byline the arithmetic changes and so does the rule: a
    /// ball pulled back from the goal line beats the whole defence at
    /// once, so it is worth playing to a single runner. That is why the
    /// requirement is depth-dependent rather than a constant.
    const BOX_BODIES: usize = 2;
    const BOX_BODIES_AT_BYLINE: usize = 1;

    /// Depth from the goal line inside which one man in the middle is
    /// enough (~14 m). Kept in step with `CrossModel::pick_type`'s own
    /// byline test, which is what decides the ball is a cutback.
    const BYLINE_DEPTH_GATE: f32 = 110.0;

    /// …and how much further a player who cannot cross has to carry
    /// before he tries anyway (~10% of the pitch, 9 m).
    ///
    /// This is where crossing SKILL enters the decision, and it enters
    /// as depth rather than as a dice roll. The predecessor rolled
    /// `SkillCurve(crossing, 8.0)` every tick the carrier stood in the
    /// channel, which makes the delivery rate a function of how long he
    /// loiters — a statistic, not a perception. Depth is what actually
    /// separates crossers in football: a good one hits it from 40 yards,
    /// a limited full-back has to reach the byline first.
    const DELIVERY_PATIENCE: f32 = 0.10;

    /// …and how far up before driving the byline is worth doing. Lower,
    /// because the drive is what *creates* the delivery position.
    const DRIVE_PROGRESS: f32 = 0.48;

    /// A team-mate this much further toward the goal line than the
    /// carrier counts as having gone past him (~5 m). Below it he is
    /// level, and a square ball to a man level with you on the same
    /// touchline achieves nothing.
    const BEYOND: f32 = 40.0;

    /// …and this much wider (~2 m), so the release is genuinely outside
    /// rather than a pass into the same congestion.
    const OUTSIDE: f32 = 16.0;

    /// How near the goal line the byline drive aims (~5.6 m), and how far
    /// off the goal's centre line (~20.6 m — just outside the corner of
    /// the penalty area). Kept in step with `WideChannel`, which sends
    /// the off-ball runner to the same patch of grass.
    const BYLINE_DEPTH: f32 = 45.0;
    const BYLINE_LATERAL: f32 = 165.0;

    /// Is this player standing in a wide area at all?
    ///
    /// Deliberately the SAME predicate the crossing states guard
    /// themselves with, so this ladder can never propose a cross that
    /// [`CrossModel`] would then refuse — a mismatch of two constants is
    /// how the previous crossing gate quietly became unreachable.
    pub fn in_channel(ctx: &StateProcessingContext) -> bool {
        CrossModel::is_in_wide_position(ctx)
    }

    /// Attacking progress of the carrier, 0 at his own goal line.
    fn progress(ctx: &StateProcessingContext) -> f32 {
        let field_width = ctx.context.field_size.width as f32;
        ctx.player
            .side
            .map(|s| s.attacking_progress_x(ctx.player.position.x, field_width))
            .unwrap_or(0.0)
    }

    /// The wide carrier's decision, or `None` when the wide area is not
    /// offering anything and he should carry on doing whatever he was.
    pub fn decide(ctx: &StateProcessingContext) -> Option<FlankAction> {
        if !ctx.player.has_ball(ctx) || !Self::in_channel(ctx) {
            return None;
        }
        let progress = Self::progress(ctx);

        // 1. The man outside you. No depth bar of its own — a simple
        //    ball to a team-mate in more space is right at any height of
        //    the pitch, and refusing it is how possession dies on a
        //    touchline.
        if progress >= Self::DRIVE_PROGRESS {
            if let Some(runner) = Self::man_outside(ctx) {
                return Some(FlankAction::ReleaseOutside { target: runner.id });
            }
        }

        // 2. The delivery, once he is deep enough for HIS delivery and
        //    there is somebody to cross TO.
        //
        //    `CrossModel::pick` answers *which* ball as a side effect of
        //    choosing a target, so asking it here costs nothing the
        //    crossing state was not about to spend. What it does not ask
        //    is whether the box is worth crossing into at all — its
        //    candidate radius is 260u (32 m), which is most of the final
        //    third — so the occupancy test is made here.
        let crossing = (ctx.player.skills.technical.crossing / 20.0).clamp(0.0, 1.0);
        let bar = Self::DELIVERY_PROGRESS + (1.0 - crossing) * Self::DELIVERY_PATIENCE;
        if progress < bar {
            return None;
        }
        let goal = ctx.player().opponent_goal_position();
        let needed = if (goal.x - ctx.player.position.x).abs() < Self::BYLINE_DEPTH_GATE {
            Self::BOX_BODIES_AT_BYLINE
        } else {
            Self::BOX_BODIES
        };
        let bodies = ctx
            .players()
            .teammates()
            .nearby_at(goal, Self::BOX_RANGE)
            .filter(|t| t.id != ctx.player.id)
            .count();
        if bodies < needed {
            return None;
        }

        // …and he needs the yard to strike it.
        //
        // A cross with a defender stood in front of you is a blocked
        // cross, and a footballer knows that before he swings his leg:
        // he takes a touch, goes outside, or comes back inside. The
        // engine has no such instinct — reaching the channel was
        // sufficient reason to deliver — and the result was **32.8
        // deliveries a team a match against a real 16-18**, with the
        // surplus arriving as tame balls into a keeper who claimed them
        // (his gathers ran 35 a match against a real 8-12).
        //
        // "In front of" is measured toward the near post rather than
        // toward the goal centre, because that is the line the ball
        // actually takes off a wide foot.
        if Self::lane_is_blocked(ctx) {
            return None;
        }

        if CrossModel::pick(ctx).is_some() {
            return Some(FlankAction::Deliver);
        }

        None
    }

    /// How near an opponent has to be to make the delivery his (~2.5 m),
    /// and how much of the striking line he has to occupy.
    const BLOCK_RADIUS: f32 = 20.0;
    const BLOCK_ALIGNMENT: f32 = 0.4;

    /// Is somebody stood in the way of the delivery?
    fn lane_is_blocked(ctx: &StateProcessingContext) -> bool {
        let me = ctx.player.position;
        let goal = ctx.player().opponent_goal_position();
        let field_height = ctx.context.field_size.height as f32;
        // The near post, on the crosser's own side of the goal.
        let outward = if me.y < field_height * 0.5 { -1.0 } else { 1.0 };
        let near_post = Vector3::new(goal.x, goal.y + outward * 29.0, 0.0);
        let Some(line) = (near_post - me).try_normalize(0.01) else {
            return false;
        };
        ctx.players()
            .opponents()
            .nearby(Self::BLOCK_RADIUS)
            .any(|o| {
                let to_opp = o.position - me;
                let along = to_opp.dot(&line);
                along > 0.0
                    && to_opp.magnitude() > 0.01
                    && along / to_opp.magnitude() > Self::BLOCK_ALIGNMENT
            })
    }

    /// The team-mate who has run beyond and outside the carrier on the
    /// same flank.
    ///
    /// Purely geometric: further toward the goal line, wider, on the same
    /// side of the pitch, and with nobody standing on him. That
    /// description is satisfied by the plan's overlapping full-back
    /// without ever naming him, and equally by a midfielder who simply
    /// happened to make the run — which is the point.
    pub fn man_outside(ctx: &StateProcessingContext) -> Option<MatchPlayerLite> {
        let side = ctx.player.side?;
        let field_height = ctx.context.field_size.height as f32;
        let centre_y = field_height * 0.5;
        let me = ctx.player.position;
        // Which touchline am I on? The release has to go further that
        // way, not merely away from the middle.
        let outward = if me.y < centre_y { -1.0 } else { 1.0 };

        ctx.players()
            .teammates()
            .nearby(240.0)
            .filter(|t| {
                let p = t.position;
                // Same half of the pitch laterally as me.
                (p.y - centre_y).signum() == outward
                    && (p.y - me.y) * outward > Self::OUTSIDE
                    && side.forward_delta(me.x, p.x) > Self::BEYOND
            })
            .filter(|t| ctx.tick_context.grid.opponents(t.id, 45.0).count() == 0)
            // Furthest forward wins — the man nearest the byline is the
            // one whose ball is worth most. Ties by id so the choice is
            // reproducible run to run.
            .max_by(|a, b| {
                side.forward_delta(me.x, a.position.x)
                    .partial_cmp(&side.forward_delta(me.x, b.position.x))
                    .unwrap_or(std::cmp::Ordering::Equal)
                    .then(b.id.cmp(&a.id))
            })
    }

    /// Where a wide carrier should be driving, in place of the goal.
    ///
    /// `None` for everybody else, and for a wide carrier whose route to
    /// goal is genuinely open — a winger with the inside lane free cuts
    /// in and shoots, which is also real football and is what the
    /// unmodified carry already does.
    ///
    /// The substitution matters more than it looks. `Arrive` at the goal
    /// from a touchline is a diagonal into the two centre-backs; the same
    /// call aimed at the byline is a run down the outside of the
    /// full-back. Same steering, same speed, opposite football.
    pub fn carry_aim(ctx: &StateProcessingContext, lane_openness: f32) -> Option<Vector3<f32>> {
        if !Self::in_channel(ctx) || Self::progress(ctx) < Self::DRIVE_PROGRESS {
            return None;
        }
        // An open inside lane is a better ball than the byline, and the
        // carry model already takes it. Only a carrier being shown
        // outside goes outside.
        if lane_openness > 0.55 {
            return None;
        }
        let side = ctx.player.side?;
        let field_height = ctx.context.field_size.height as f32;
        let field_width = ctx.context.field_size.width as f32;
        let forward = side.forward_dir_x();
        let goal = ctx.player().opponent_goal_position();
        let outward = if ctx.player.position.y < field_height * 0.5 {
            -1.0
        } else {
            1.0
        };
        Some(Vector3::new(
            (goal.x - forward * Self::BYLINE_DEPTH).clamp(14.0, field_width - 14.0),
            (goal.y + outward * Self::BYLINE_LATERAL).clamp(20.0, field_height - 20.0),
            0.0,
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::FlankPlay;

    /// The ladder only works if the drive comes before the delivery: a
    /// carrier who may cross from further out than he may drive never
    /// drives, and the byline positions that produce cutbacks are never
    /// reached. Their ORDER is the design, not their values.
    #[test]
    fn a_wide_player_may_drive_from_deeper_than_he_may_cross() {
        assert!(
            FlankPlay::DRIVE_PROGRESS < FlankPlay::DELIVERY_PROGRESS,
            "the drive that creates the crossing position is gated behind the cross"
        );
    }

    /// Even the worst crosser in the game must be able to deliver from
    /// somewhere short of the goal line, or the patience term silently
    /// removes the whole behaviour for a band of players.
    #[test]
    fn the_patience_term_never_pushes_the_bar_off_the_pitch() {
        let worst = FlankPlay::DELIVERY_PROGRESS + FlankPlay::DELIVERY_PATIENCE;
        assert!(
            worst < 0.90,
            "a poor crosser has to reach {worst} of the pitch before he may cross"
        );
    }

    /// The byline aim must be a place a footballer would actually stand:
    /// outside the width of the six-yard box (or he is running into the
    /// keeper) and inside the width of the penalty area (or he is on the
    /// paint and the ball goes out).
    #[test]
    fn the_byline_target_is_the_corner_of_the_box() {
        /// Half the width of the six-yard box, in game units.
        const SIX_YARD_HALF: f32 = 73.0;
        /// …and of the penalty area.
        const BOX_HALF: f32 = 161.0;
        assert!(
            FlankPlay::BYLINE_LATERAL > SIX_YARD_HALF,
            "the byline aim is inside the six-yard box"
        );
        assert!(
            FlankPlay::BYLINE_LATERAL > BOX_HALF,
            "the byline aim is inside the penalty area rather than at its corner"
        );
        assert!(
            FlankPlay::BYLINE_DEPTH > 0.0 && FlankPlay::BYLINE_DEPTH < 100.0,
            "the byline aim is not near the byline"
        );
    }

    /// One man in the middle is enough only from the byline. If the two
    /// requirements ever met, "don't cross into an empty box" would stop
    /// being a rule about where he is standing.
    #[test]
    fn the_byline_asks_for_fewer_bodies_than_the_wide_area() {
        assert!(FlankPlay::BOX_BODIES_AT_BYLINE < FlankPlay::BOX_BODIES);
    }
}
