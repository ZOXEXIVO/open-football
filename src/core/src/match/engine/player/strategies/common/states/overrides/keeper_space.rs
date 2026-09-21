use crate::r#match::common_states::TackleEngagement;
use crate::r#match::player::state::PlayerState;
use crate::r#match::{PassOriginRestart, PlayerSide, StateProcessingContext, SteeringBehavior};
use nalgebra::Vector3;

/// Give a goalkeeper room to distribute: teammates offer outlets, while
/// opponents leave the area unless they are engaging a live foot possession.
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
/// Restricting this to opponents and hand possession left teammates
/// crowding his feet, and surplus pressers waiting beside him for a
/// challenge the engagement election would never allow. The same movement
/// override now supplies outlets and respects that election on live balls.
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

    /// Teammates inside 10 m open out to offer a pass instead of waiting
    /// beside the keeper. Targets are further away so they cross this
    /// boundary at a jog rather than stopping inside it.
    const SUPPORT_SPACE: f32 = 80.0;
    const SUPPORT_DEPTH: f32 = 96.0;
    const SUPPORT_WIDTH: f32 = 64.0;

    /// The outlet/retreat velocity and the effort floor to serve it at,
    /// or `None` when the player's normal movement should apply.
    pub fn retreat(ctx: &StateProcessingContext) -> Option<(Vector3<f32>, f32)> {
        if matches!(ctx.player.state, PlayerState::Injured)
            || ctx
                .player
                .tactical_position
                .current_position
                .is_goalkeeper()
        {
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
        let ball = &ctx.tick_context.ball;
        let goal_kick =
            ball.pass_origin_restart == PassOriginRestart::GoalKick && ball.restart_taker.is_some();
        // During the run-up the ball has no owner. The awarded taker
        // still needs room, and nobody may press this dead ball.
        let holder_id = if goal_kick {
            ball.restart_taker?
        } else {
            ball.current_owner?
        };
        // ⚠ **And he has to be a KEEPER.** `held_in_hands` used to be a
        // goalkeeper's flag and nothing else, so the owner check above was
        // the whole test. A throw-in's taker holds the ball in his hands
        // too now (see [`ThrowInDelivery`](super::ThrowInDelivery)), and
        // without this a throw taken deep in a side's own corner backed
        // the opposition out of a penalty area nobody was standing in.
        if !ctx
            .context
            .players
            .by_id(holder_id)
            .is_some_and(|p| p.tactical_position.current_position.is_goalkeeper())
        {
            return None;
        }
        let holder = ctx
            .tick_context
            .positions
            .players
            .as_slice()
            .iter()
            .find(|e| e.player_id == holder_id)?;
        let holder_side = holder.side;
        if Some(holder_side) == ctx.player.side {
            return Self::offer_outlet(ctx, holder.position, holder_side);
        }
        let area = ctx.context.penalty_area(holder_side == PlayerSide::Left);
        // A keeper playing as an outfielder does not clear his own box
        // of opponents at the other end of the field.
        if !ball.held_in_hands
            && !goal_kick
            && (!(area.min.x..=area.max.x).contains(&holder.position.x)
                || !(area.min.y..=area.max.y).contains(&holder.position.y))
        {
            return None;
        }
        let me = ctx.player.position;
        if !(area.min.x..=area.max.x).contains(&me.x) || !(area.min.y..=area.max.y).contains(&me.y)
        {
            return None;
        }

        // Use the same election as the tackle states. Calling off every
        // opponent here would make back-passes impossible to press;
        // letting the others pursue leaves them queued beside the keeper
        // even though the engagement gate refuses their challenges.
        if !ball.held_in_hands && !goal_kick && TackleEngagement::may_engage_carrier(ctx) {
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
        let gap = ctx.player.position - holder.position;
        let dist = gap.magnitude();
        if dist < Self::PERSONAL_SPACE {
            let away = gap
                .try_normalize(1.0e-3)
                .unwrap_or_else(|| Vector3::new(holder_side.forward_dir_x(), 0.0, 0.0));
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

    fn offer_outlet(
        ctx: &StateProcessingContext,
        keeper: Vector3<f32>,
        side: PlayerSide,
    ) -> Option<(Vector3<f32>, f32)> {
        if (ctx.player.position - keeper).norm_squared() >= Self::SUPPORT_SPACE.powi(2) {
            return None;
        }
        // The formation lane is stable even when players overlap. Using
        // the current gap alone would send an entire cluster the same way
        // (and has no direction at all for coincident positions).
        let wide = if ctx.player.start_position.y < ctx.context.field_size.height as f32 * 0.5 {
            -1.0
        } else {
            1.0
        };
        let target = Vector3::new(
            (keeper.x + side.forward_dir_x() * Self::SUPPORT_DEPTH)
                .clamp(8.0, ctx.context.field_size.width as f32 - 8.0),
            (keeper.y + wide * Self::SUPPORT_WIDTH)
                .clamp(8.0, ctx.context.field_size.height as f32 - 8.0),
            0.0,
        );
        Some((
            SteeringBehavior::Arrive {
                target,
                slowing_distance: 24.0,
            }
            .calculate(ctx.player)
            .velocity
                * Self::PACE,
            Self::BACK_OFF_EFFORT,
        ))
    }
}
