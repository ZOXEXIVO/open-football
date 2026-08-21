use crate::r#match::{PlayerSide, StateProcessingContext, SteeringBehavior};
use nalgebra::Vector3;

/// **The ball is in the goalkeeper's hands — get out of his area.**
///
/// # Why this exists
///
/// Nothing in the engine moved a player because the opposing keeper had
/// picked the ball up. The pressing states stood down (see
/// `BallOperationsImpl::carrier_id`), which stopped them running AT him,
/// but standing down is not the same as backing off: whoever was in the
/// box when he claimed it simply stayed there for the whole hold.
/// Measured over 12 matches, on the ticks the ball was in a keeper's
/// gloves there were **3.2 opponents inside his penalty area on average,
/// at least one on 97% of those ticks, and one within 5u — 62 cm — of him
/// on 22%**. On screen that is a forward standing over a keeper who is
/// holding the ball, which reads as trying to take it off him whether or
/// not the ownership layer would ever allow it.
///
/// It is also the football. A keeper in possession of the ball with his
/// hands cannot be challenged — Law 12 makes even attempting to kick it
/// while he is releasing it an indirect free kick — so there is nothing
/// for an attacker to win by staying, and every attacking side in the
/// world turns and jogs out to press the distribution instead.
///
/// # Where it is applied
///
/// At the single point every state's movement converges on
/// (`StateProcessor::process_inner`), and DELIBERATELY ahead of
/// `ShapeDiscipline`: the attacking plan's box slots are inside the very
/// area he has to leave, so shaping him would pull him straight back in.
/// It is a velocity override rather than a state, because it must reach
/// the states that stand still as well as the ones that run — a striker
/// idling on the six-yard line is the exact case being fixed.
pub struct KeeperReleaseSpace;

impl KeeperReleaseSpace {
    /// How far beyond the edge of the area he keeps going, so he is not
    /// hovering on the line waiting to step back in. 16u = 2 m.
    const CLEAR_MARGIN: f32 = 16.0;
    /// Share of `Arrive`'s own output to use. `Arrive` already caps itself
    /// at roughly `max_speed * agility * 0.7` and tapers to nothing over
    /// the last `slowing_distance`, so this lands at a jog and needs no
    /// separate "don't overshoot" term.
    ///
    /// ⚠ If the retreat looks too slow, RAISE THE EFFORT FLOOR, NOT THIS.
    /// The realised speed is `min(this * Arrive, effort * max_speed)` and
    /// the second term is the binding one — pushing this from 0.62 to 1.0
    /// moved the measured box occupancy by 0.05 of a player, because the
    /// request was already above the ceiling. (What was actually wrong at
    /// the time was the side lookup below, and it is worth knowing that a
    /// tuning knob can be turned twice for nothing while a bug is hiding
    /// underneath it.)
    const PACE: f32 = 0.85;
    /// Nobody stands this close to a keeper who is holding the ball. 26u
    /// ≈ 3.25 m — outside his spread, which is the distance the Laws are
    /// really about.
    const PERSONAL_SPACE: f32 = 26.0;
    /// Effort floor for the walk out at the edge of the area, on
    /// `MovementEffort::speed_fraction`'s scale — between `Low` (0.25, a
    /// stroll) and `Moderate` (0.52, a jog into space). He has a couple of
    /// metres left at this point.
    const JOG_EFFORT: f32 = 0.40;
    /// …rising to this deep inside it, and for backing off a keeper you
    /// are standing on top of. A man on the six-yard line has 16 m to
    /// cover and about three and a half seconds of hold to do it in, so a
    /// stroll never gets him out; `Arrive` tapers him back down to a walk
    /// as he reaches the line, which is the shape a real jog out of the
    /// box has.
    const BACK_OFF_EFFORT: f32 = 0.65;

    /// The velocity that takes this player out of the area of a keeper
    /// holding the ball and the effort floor to serve it at, or `None`
    /// when there is nothing to do — which is almost always.
    pub fn retreat(ctx: &StateProcessingContext) -> Option<(Vector3<f32>, f32)> {
        if !ctx.tick_context.ball.held_in_hands {
            return None;
        }
        // Which end the holder defends, read off the LIVE position store
        // rather than `context.players`. That collection is a pre-kickoff
        // snapshot, and `side` on it is an `Option` that is not reliably
        // populated — taking it as `None` resolves `penalty_area(false)`,
        // the RIGHT-hand box, for everybody. Which is exactly what the
        // instrumentation showed: the retreat fired on 3,383 player-ticks
        // a match against the ~7,300 opponent-in-area ticks it should have,
        // i.e. one team's keeper was protected and the other's was not.
        let holder_id = ctx.tick_context.ball.current_owner?;
        let holder_side = ctx
            .tick_context
            .positions
            .players
            .as_slice()
            .iter()
            .find(|e| e.player_id == holder_id)
            .map(|e| e.side)?;
        if Some(holder_side) == ctx.player.side {
            return None;
        }
        let area = ctx.context.penalty_area(holder_side == PlayerSide::Left);
        let me = ctx.player.position;
        if !(area.min.x..=area.max.x).contains(&me.x) || !(area.min.y..=area.max.y).contains(&me.y)
        {
            return None;
        }

        // Out along the goal-to-goal axis, up the pitch. Leaving sideways
        // would put him level with the goal line by the corner flag, which
        // is not where anybody goes — the press re-forms in front of the
        // area, facing the keeper.
        let out_x = if holder_side == PlayerSide::Left {
            area.max.x + Self::CLEAR_MARGIN
        } else {
            area.min.x - Self::CLEAR_MARGIN
        };
        let mut velocity = SteeringBehavior::Arrive {
            target: Vector3::new(out_x, me.y, 0.0),
            slowing_distance: 24.0,
        }
        .calculate(ctx.player)
        .velocity
            * Self::PACE;
        // How deep he is, as a share of the area's depth. The man on the
        // goal line has the furthest to go and the least time to do it.
        let depth = ((out_x - me.x).abs() / (area.max.x - area.min.x).max(1.0)).clamp(0.0, 1.0);
        let mut effort = Self::JOG_EFFORT + (Self::BACK_OFF_EFFORT - Self::JOG_EFFORT) * depth;

        // Crossing the box takes several seconds and a hold lasts three
        // and a half, so the walk out on its own cannot answer the worst
        // version of this: a forward standing ON the keeper, which was 22%
        // of all hand-ticks. That one is two metres of movement, and it is
        // the one an official would actually intervene over — so it gets
        // its own direct term and its own urgency.
        let gap = ctx.player.position - ctx.tick_context.positions.ball.position;
        let dist = gap.magnitude();
        if dist < Self::PERSONAL_SPACE {
            let away = gap
                .try_normalize(1.0e-3)
                .unwrap_or_else(|| Vector3::new(-holder_side.forward_dir_x(), 0.0, 0.0));
            let urgency = 1.0 - dist / Self::PERSONAL_SPACE;
            velocity += away * ctx.player.max_speed_with_condition_cached() * urgency * 0.6;
            effort = Self::BACK_OFF_EFFORT;
        }
        #[cfg(feature = "match-logs")]
        {
            use crate::r#match::engine::ball::ball::ownership::reception_diag as d;
            d::keeper_ball_note(20);
            d::keeper_ball_add(21, (velocity.magnitude() * 1000.0) as u64);
        }
        Some((velocity, effort))
    }
}
