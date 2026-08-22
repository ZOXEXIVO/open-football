use crate::PlayerFieldPositionGroup;
use crate::r#match::ball::events::GoalSide;
use crate::r#match::engine::flow::field::ResetReason;
use crate::r#match::field::MatchField;
use crate::r#match::flow::celebration::GoalCelebration;
use crate::r#match::{MatchContext, MatchFieldSize, PlayerSide, TransitionSource};
use nalgebra::Vector3;
use std::cmp::Ordering;

pub const GOAL_WIDTH: f32 = 29.0; // half-width in game units (full goal = 58 units, real = 7.32m)
pub const GOAL_HEIGHT: f32 = 2.44; // Crossbar height in meters (z-axis is in meters)

#[derive(Clone)]
pub struct GoalPosition {
    pub left: Vector3<f32>,
    pub right: Vector3<f32>,
}

impl From<&MatchFieldSize> for GoalPosition {
    fn from(value: &MatchFieldSize) -> Self {
        // Left goal at x = 0, centered on width
        let left_goal = Vector3::new(0.0, value.height as f32 / 2.0, 0.0);

        // Right goal at x = length, centered on width
        let right_goal = Vector3::new(value.width as f32, (value.height / 2usize) as f32, 0.0);

        GoalPosition {
            left: left_goal,
            right: right_goal,
        }
    }
}

impl GoalPosition {
    pub fn is_goal(&self, ball_position: Vector3<f32>) -> Option<GoalSide> {
        if ball_position.z > GOAL_HEIGHT {
            return None;
        }
        self.check_goal_line(ball_position)
    }

    /// Check if ball crossed the goal line within goal width but ABOVE the crossbar.
    /// Returns which side the ball went over (goal kick for the defending team).
    pub fn is_over_goal(&self, ball_position: Vector3<f32>) -> Option<GoalSide> {
        if ball_position.z <= GOAL_HEIGHT {
            return None;
        }
        self.check_goal_line(ball_position)
    }

    fn check_goal_line(&self, ball_position: Vector3<f32>) -> Option<GoalSide> {
        if ball_position.x <= self.left.x {
            if (self.left.y - GOAL_WIDTH..=self.left.y + GOAL_WIDTH).contains(&ball_position.y) {
                return Some(GoalSide::Home);
            }
        }

        if ball_position.x >= self.right.x {
            if (self.right.y - GOAL_WIDTH..=self.right.y + GOAL_WIDTH).contains(&ball_position.y) {
                return Some(GoalSide::Away);
            }
        }

        None
    }
}

/// Place an outfield player from `side` on the centre spot and give
/// them protected possession. Used by every restart that puts the
/// ball on the centre circle — goals, match start, halftime, start of
/// extra time. Without this, `reset_players_positions` leaves the
/// whole squad at formation start and the ball sits with no claimant
/// — once `in_flight_state` expires nobody is close enough to keep
/// it, ownership gets nulled, and the period stalls for ~14 seconds
/// until the emergency chaser-override fires.
/// Could this man take the kickoff for `side`?
///
/// Shared with `GoalCelebration::restart`, which has to answer the same
/// question BEFORE it resets the formation: it leaves the taker where he
/// is, and that is only right if he is going to be the taker. The
/// celebration's retriever often is — he carried the ball to the centre
/// spot — but he can also be the beaten goalkeeper, who cannot, and
/// keeping him in place then strands one man off his formation spot for
/// the kickoff with somebody else on the ball.
pub fn can_take_kickoff(field: &MatchField, side: PlayerSide, id: u32) -> bool {
    field.players.iter().any(|p| {
        p.id == id
            && !p.is_sent_off
            && p.side == Some(side)
            && p.tactical_position.current_position.position_group()
                != PlayerFieldPositionGroup::Goalkeeper
    })
}

pub fn assign_kickoff(field: &mut MatchField, side: PlayerSide, preferred: Option<u32>) {
    let ball_pos = field.ball.position;
    // ⚠ **The man who carried the ball here takes it.**
    //
    // `preferred` is the celebration's retriever: he fetched the ball out
    // of the net, walked it to the centre spot and is standing on it. He
    // is from the conceding side, which is the side kicking off, so he is
    // also the correct taker by the laws. Picking "nearest to the ball"
    // instead sounds equivalent and is not — the position reset runs
    // first, so by the time the search happens he has been sent back to
    // his formation slot and the nearest man is somebody else, who is
    // then teleported onto the ball. Two relocations, 166 m/match between
    // them, to hand the ball to a different player for no reason.
    let kickoff_player_id = preferred
        .filter(|id| can_take_kickoff(field, side, *id))
        .or_else(|| {
            field
                .players
                .iter()
                .filter(|p| p.side == Some(side) && !p.is_sent_off)
                .filter(|p| {
                    p.tactical_position.current_position.position_group()
                        != PlayerFieldPositionGroup::Goalkeeper
                })
                .min_by(|a, b| {
                    let da = (a.position - ball_pos).norm_squared();
                    let db = (b.position - ball_pos).norm_squared();
                    da.partial_cmp(&db).unwrap_or(Ordering::Equal)
                })
                .map(|p| p.id)
        });

    if let Some(player_id) = kickoff_player_id {
        if let Some(kicker) = field.players.iter_mut().find(|p| p.id == player_id) {
            #[cfg(feature = "match-logs")]
            {
                use crate::r#match::engine::ball::ball::teleport as tc;
                tc::PlayerTeleportCensus::note_firing(tc::PSITE_KICKOFF_TAKER);
                tc::PlayerTeleportCensus::note(tc::PSITE_KICKOFF_TAKER, kicker.position, ball_pos);
            }
            kicker.position = ball_pos;
            kicker.velocity = Vector3::zeros();
            kicker.set_default_state(TransitionSource::Reset);
            // `set_default_state` is timer-preserving; the kickoff taker
            // reset its timer explicitly in the pre-refactor engine, so
            // keep that to stay calibration-neutral.
            kicker.in_state_time = 0;
        }
        field.ball.current_owner = Some(player_id);
        // Short ping-pong guard only — the kicker needs to take the
        // ball forward, not hold on to it for 1.2 s while the whole
        // pack watches. A 30-tick cooldown is enough to stop the
        // ownership logic from immediately ripping the ball back out
        // of their feet and falls away by the time the state machine
        // decides to pass.
        field.ball.claim_cooldown = 30;
        field.ball.flags.in_flight_state = 0;
        field.ball.contested_claim_count = 0;
    }
}

/// A goal has gone in: arm the celebration.
///
/// This used to BE the restart — players teleported into formation, ball
/// teleported to the centre spot, all on the tick the ball crossed the line,
/// after which the engine loop skipped 45-75 s of match clock with the world
/// frozen. The restart still happens at exactly the same instant and leaves
/// exactly the same state; what changed is that the window in between is now
/// played out rather than skipped, so the ball goes into the net and the
/// players celebrate instead of the recording holding one frame for a minute.
/// See [`GoalCelebration`] for what happens in there and why none of it can
/// move a calibrated number.
pub fn handle_goal_reset(field: &mut MatchField, context: &mut MatchContext) {
    if !field.ball.goal_scored {
        return;
    }

    let kickoff_side = field.ball.kickoff_team_side;

    field.ball.goal_scored = false;
    field.ball.kickoff_team_side = None;
    context.record_goal_tick();
    // Post-goal dead time: celebration + walk-back + the referee's
    // restart — 45-75 s of match clock during which no ball physics,
    // player AI or events run (see `MatchContext::dead_ball_until_ms`).
    // The pause is load-bearing for realism: it consumes the post-goal
    // window in which the engine's freshly-reset formations were
    // measurably easy to attack (goals begat goals), and it means play
    // always resumes against a fully SET defense.
    //
    // NB the single RNG draw here is the only one the whole post-goal
    // path takes, and it must stay that way — the stream is shared with
    // every calibrated roll in the match.
    context.dead_ball_until_ms = context.total_match_time + context.rng.range_u64(45, 75) * 1000;
    // The side kicking off after a goal IS the side that just conceded.
    // Mark them so the forward shot-decision dampens willingness in the
    // ~1-minute post-concede window — breaks the equalizer cascade that
    // was the dominant source of 2-2 / 3-3 / 4-4 draws in the engine's
    // scoreline distribution.
    if let Some(conceding_side) = kickoff_side {
        context.record_conceded(conceding_side);
    }

    let Some(side) = kickoff_side else {
        // No conceding side could be resolved — nothing to restart toward.
        // Put the world back the way the old path did and move on.
        field.reset_players_positions(ResetReason::Restart { keep: None });
        field.ball.reset();
        return;
    };

    let restart_at_ms = context.dead_ball_until_ms;
    context.goal_celebration = Some(GoalCelebration::arm(field, context, side, restart_at_ms));
}

/// Play one tick of the post-goal window, if there is one.
///
/// Called from the engine loop's dead-ball branch — the only ticks in which
/// it can do anything, because that is the only time a celebration is live.
/// Returns `true` if something moved and the tick is therefore worth
/// recording.
pub fn advance_goal_celebration(field: &mut MatchField, context: &mut MatchContext) -> bool {
    let Some(mut celebration) = context.goal_celebration.take() else {
        return false;
    };
    if celebration.advance(field, context) {
        context.goal_celebration = Some(celebration);
    }
    // The whole tick body is skipped in here, `Ball::update` included, so
    // the celebration hands the woodwork trace its own sample. Without it
    // every trace of a ball that ends up in the goal stops at the line and
    // resumes at the centre spot — which is the single stretch the "it goes
    // to the keeper and then teleports" report is about.
    #[cfg(feature = "match-logs")]
    field
        .ball
        .trace_tick(context.current_tick(), &field.players);
    true
}

/// Force any pending celebration to its restart immediately.
///
/// The whistle for half time can go while the ball is still in the net. Play
/// must never resume — nor a period end — with a goal half-processed, so the
/// period boundary settles it.
pub fn finish_goal_celebration(field: &mut MatchField, context: &mut MatchContext) {
    if let Some(celebration) = context.goal_celebration.take() {
        // A period boundary follows immediately and will re-form both
        // sides anyway, so there is no taker worth keeping in place.
        GoalCelebration::restart(field, celebration.kickoff_side, None);
    }
}
