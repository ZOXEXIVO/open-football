//! **One read of the game for every forward without the ball.**
//!
//! A forward is on one of two ladders. Out of possession: the shape, the
//! press, the duel, the recovery. In possession: the shape, a job in the
//! attack, the ball. Each state sits on one rung and asks this read for
//! every door above its own level; its own work at its level stays in the
//! state. So the same situation gets the same answer from whatever state it
//! finds him in, and no state hands him to an idle one for the question to
//! be asked again.

use crate::PlayerFieldPositionGroup;
use crate::r#match::common_states::{LooseBallChase, RecoveryChallenge, TackleEngagement};
use crate::r#match::forwarders::states::ForwardState;
use crate::r#match::forwarders::states::common::RunLine;
use crate::r#match::player::state::PlayerState;
use crate::r#match::player::strategies::RestartHold;
use crate::r#match::player::strategies::common::players::ops::box_movement::BoxMovement;
use crate::r#match::player::strategies::common::team::{ShapeStation, WideChannel};
use crate::r#match::{
    DefensiveDuty, MatchPlayerLite, PlayerSide, StateChangeResult, StateProcessingContext,
};

/// The rungs of the read, highest door first.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum ReadLevel {
    /// The ball is his and at his feet.
    Possession = 1,
    /// A high ball is his to head.
    Staged = 2,
    /// The loose-ball election names him, or a ball in flight that is not
    /// his side's is one he can contest.
    LooseBall = 3,
    /// He is the man for the carrier.
    Engage = 4,
    /// His side has the ball and has a job for him.
    Job = 5,
    /// He is out of his place, or beyond their last line while his side does
    /// not have the ball.
    Recover = 6,
    /// Nothing else: his place, relaxed or alert.
    Phase = 7,
}

impl ReadLevel {
    const ALL: [ReadLevel; 7] = [
        ReadLevel::Possession,
        ReadLevel::Staged,
        ReadLevel::LooseBall,
        ReadLevel::Engage,
        ReadLevel::Job,
        ReadLevel::Recover,
        ReadLevel::Phase,
    ];
}

pub struct ForwardRead;

impl ForwardRead {
    /// The phase distances of design D1, in units, measured from his place:
    /// relaxed beyond 30 m with his own side on the ball, beyond 45 m with
    /// the opposition on it, beyond 35 m from a loose ball.
    const RELAXED_OWN: f32 = 240.0;
    const RELAXED_THEIRS: f32 = 360.0;
    const RELAXED_LOOSE: f32 = 280.0;
    /// An alert man stays alert until the ball is this much (5 m) further.
    const HYSTERESIS: f32 = 40.0;
    /// Below this condition a forward relaxes sooner, by this much of each
    /// distance per point of deficit.
    const TIRED_BELOW: f32 = 0.70;
    const TIRED_SHRINK: f32 = 2.0;
    /// How far past a containing forward the carrier has to be (0.5 m)
    /// before he is closing again.
    const GONE_PAST: f32 = 4.0;
    /// How far beyond their last line (1 m) a forward is stranded while his
    /// side does not have the ball.
    const STRANDED: f32 = 8.0;

    /// The first open door above `level`.
    pub fn above(ctx: &StateProcessingContext, level: ReadLevel) -> Option<ForwardState> {
        ReadLevel::ALL
            .iter()
            .take_while(|l| **l < level)
            .find_map(|l| Self::door(ctx, *l))
    }

    /// [`Self::above`] as the state change a handler returns.
    pub fn change_above(
        ctx: &StateProcessingContext,
        level: ReadLevel,
    ) -> Option<StateChangeResult> {
        Self::above(ctx, level).map(StateChangeResult::with_forward_state)
    }

    /// The whole read: where a forward with nothing in hand belongs.
    pub fn resolve(ctx: &StateProcessingContext) -> ForwardState {
        Self::above(ctx, ReadLevel::Phase).unwrap_or_else(|| Self::phase(ctx))
    }

    /// [`Self::resolve`] as the state change a handler returns.
    pub fn change(ctx: &StateProcessingContext) -> StateChangeResult {
        StateChangeResult::with_forward_state(Self::resolve(ctx))
    }

    /// For a state the read answers whole: stay while the read still says
    /// `state`, go where it says otherwise.
    pub fn hold(ctx: &StateProcessingContext, state: ForwardState) -> Option<StateChangeResult> {
        let next = Self::resolve(ctx);
        (next != state).then(|| StateChangeResult::with_forward_state(next))
    }

    /// The read for a man who has just played the ball: he no longer has
    /// it, whatever the snapshot of this tick says.
    pub fn released(ctx: &StateProcessingContext) -> ForwardState {
        ReadLevel::ALL[1..]
            .iter()
            .find_map(|l| Self::door(ctx, *l))
            .unwrap_or_else(|| Self::phase(ctx))
    }

    fn door(ctx: &StateProcessingContext, level: ReadLevel) -> Option<ForwardState> {
        match level {
            ReadLevel::Possession => ctx.ball().mine_to_play().then_some(ForwardState::Running),
            ReadLevel::Staged => Self::staged(ctx),
            ReadLevel::LooseBall => Self::loose_ball(ctx),
            ReadLevel::Engage => Self::engage(ctx),
            ReadLevel::Job => Self::job(ctx),
            ReadLevel::Recover => Self::recover(ctx),
            ReadLevel::Phase => Some(Self::phase(ctx)),
        }
    }

    /// A high ball a contest has given him, still on its way to him: up for
    /// it in `Heading`.
    fn staged(ctx: &StateProcessingContext) -> Option<ForwardState> {
        (ctx.player.aerial_award && ctx.tick_context.ball.awaits_header_from(ctx.player.id))
            .then_some(ForwardState::Heading)
    }

    /// The election's own question, asked as the dispatcher will ask it on
    /// the next tick — so a state never starts a chase it will refuse — and
    /// a ball in flight that is not his side's and that he can contest.
    fn loose_ball(ctx: &StateProcessingContext) -> Option<ForwardState> {
        if PlayerFieldPositionGroup::should_force_takeball(
            PlayerFieldPositionGroup::Forward,
            ctx.player,
            ctx.context,
            ctx.tick_context,
        ) {
            return Some(ForwardState::TakeBall);
        }
        Self::can_intercept(ctx).then_some(ForwardState::Intercepting)
    }

    /// A ball nobody has, not his own side's, that he can reach no later
    /// than the opposition. `Intercepting` gives the chase up on the same
    /// question.
    pub fn can_intercept(ctx: &StateProcessingContext) -> bool {
        !ctx.ball().is_owned() && !ctx.team().is_control_ball() && LooseBallChase::can_contest(ctx)
    }

    /// He is the man for the carrier: closing, or containing once he is in
    /// challenge range goal-side of him. The plan's presser goes with a ball
    /// travelling to an opponent as well.
    fn engage(ctx: &StateProcessingContext) -> Option<ForwardState> {
        let Some(carrier) = ctx.players().opponents().with_ball().next() else {
            return (ctx.team().my_duty() == DefensiveDuty::Press && !ctx.team().is_control_ball())
                .then_some(ForwardState::Pressing);
        };
        if !Self::is_man_for_carrier(ctx) {
            return None;
        }
        Some(Self::duel_rung(ctx, &carrier))
    }

    /// The engagement licence: the plan's presser, the box emergency, the
    /// man a carrier runs into, or the best chaser when the plan names
    /// nobody.
    pub fn is_man_for_carrier(ctx: &StateProcessingContext) -> bool {
        TackleEngagement::may_engage_carrier(ctx)
    }

    /// Closing or containing: inside `COMMIT` and goal-side of him he
    /// contains; otherwise he is still closing.
    pub fn duel_rung(ctx: &StateProcessingContext, carrier: &MatchPlayerLite) -> ForwardState {
        if carrier.distance(ctx) <= TackleEngagement::COMMIT && Self::is_goal_side(ctx, carrier) {
            ForwardState::Jockeying
        } else {
            ForwardState::Pressing
        }
    }

    /// In front of the carrier on his line to goal. A man already
    /// containing is gone past only once the carrier is half a body by him,
    /// as he gives the carrier up at `DISENGAGE` rather than `COMMIT`.
    pub fn is_goal_side(ctx: &StateProcessingContext, carrier: &MatchPlayerLite) -> bool {
        let slack = if ctx.player.state == PlayerState::Forward(ForwardState::Jockeying) {
            Self::GONE_PAST
        } else {
            0.0
        };
        RecoveryChallenge::lead(ctx, carrier.position) > -slack
    }

    /// His job in his side's attack, as the attack plan gives it: a
    /// touchline, the run in behind, a place in the box once the box is
    /// staged, or the short outlet. Given nothing, or a place the box is not
    /// yet staged for, he holds his place.
    fn job(ctx: &StateProcessingContext) -> Option<ForwardState> {
        if !ctx.team().is_control_ball() {
            return None;
        }
        if WideChannel::still_mine(ctx) {
            return Some(ForwardState::HoldingWidth);
        }
        let team = ctx.team();
        let plan = team.attack_plan();
        let id = ctx.player.id;
        if plan.far_runner == Some(id) {
            Some(ForwardState::RunningInBehind)
        } else if plan.slot_of(id).is_some() && BoxMovement::stage(ctx) > 0.0 {
            Some(ForwardState::CrossReceiving)
        } else if plan.near_support == Some(id) {
            Some(ForwardState::CreatingSpace)
        } else {
            None
        }
    }

    /// Out of his place, or stranded beyond their last line while his side
    /// does not have the ball: getting back.
    fn recover(ctx: &StateProcessingContext) -> Option<ForwardState> {
        (ShapeStation::should_recover(ctx) || Self::is_stranded(ctx))
            .then_some(ForwardState::Returning)
    }

    /// Beyond the line the referee reads by more than a stride, with his
    /// side not on the ball: every ball cleared toward him would be offside.
    pub fn is_stranded(ctx: &StateProcessingContext) -> bool {
        if ctx.team().is_control_ball() {
            return false;
        }
        let (Some(side), Some(line)) = (ctx.player.side, RunLine::of(ctx)) else {
            return false;
        };
        let x = ctx.player.position.x;
        match side {
            PlayerSide::Left => x > line + Self::STRANDED,
            PlayerSide::Right => x < line - Self::STRANDED,
        }
    }

    /// His place: alert while the play is in reach of it, relaxed while it
    /// is not, with the ball dead or a long way off.
    pub fn phase(ctx: &StateProcessingContext) -> ForwardState {
        if Self::is_alert(ctx) {
            ForwardState::HoldingShape
        } else {
            ForwardState::Walking
        }
    }

    fn is_alert(ctx: &StateProcessingContext) -> bool {
        if RestartHold::taker(ctx.tick_context).is_some() {
            return false;
        }
        let ours = ctx.team().is_control_ball();
        let ball = ctx.tick_context.positions.ball.position;
        let to_place = (ball - ctx.team().my_anchor()).xy().norm();
        let loose = !ctx.ball().is_owned() && ctx.tick_context.ball.pass_target.is_none();
        let relaxed_from = if loose {
            Self::RELAXED_LOOSE
        } else if ours {
            Self::RELAXED_OWN
        } else {
            Self::RELAXED_THEIRS
        };
        let held = if ctx.player.state == PlayerState::Forward(ForwardState::HoldingShape) {
            Self::HYSTERESIS
        } else {
            0.0
        };
        to_place < (relaxed_from + held) * Self::tired_reach(ctx)
    }

    /// The share of each distance a tired forward still reads.
    fn tired_reach(ctx: &StateProcessingContext) -> f32 {
        let condition = ctx.player.player_attributes.condition_percentage() as f32 / 100.0;
        (1.0 - (Self::TIRED_BELOW - condition).max(0.0) * Self::TIRED_SHRINK).clamp(0.4, 1.0)
    }
}
