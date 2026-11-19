//! **One read of the game for every midfielder without the ball.**
//!
//! A midfielder is on one of two ladders. Out of possession: the shape, a
//! job, the duel, the recovery. In possession: the shape, a job in the
//! attack, the ball. Each state sits on one rung and asks this read for
//! every door above its own level; its own work at its level, and its
//! give-ups, stay in the state. So the same situation gets the same answer
//! from whatever state it finds him in, and no state hands him to an idle
//! one for the question to be asked again.

use crate::PlayerFieldPositionGroup;
use crate::r#match::common_states::{MarkEngagement, RecoveryChallenge, TackleEngagement};
use crate::r#match::midfielders::states::MidfielderState;
use crate::r#match::midfielders::states::common::Interception;
use crate::r#match::player::state::PlayerState;
use crate::r#match::player::strategies::RestartHold;
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
    /// The loose-ball election names him, or a ball he can contest is
    /// coming to him.
    LooseBall = 3,
    /// He is the man for the carrier.
    Engage = 4,
    /// He has a man and the opposition has the ball.
    Mark = 5,
    /// He is out of his place with no job that keeps him out.
    Recover = 6,
    /// His side has the ball and has a job for him.
    Support = 7,
    /// Nothing else: his place, relaxed or alert.
    Phase = 8,
}

impl ReadLevel {
    const ALL: [ReadLevel; 8] = [
        ReadLevel::Possession,
        ReadLevel::Staged,
        ReadLevel::LooseBall,
        ReadLevel::Engage,
        ReadLevel::Mark,
        ReadLevel::Recover,
        ReadLevel::Support,
        ReadLevel::Phase,
    ];
}

pub struct MidfielderRead;

impl MidfielderRead {
    /// The phase distances of design D1, in units, measured from his place:
    /// relaxed beyond 30 m with his own side on the ball, beyond 45 m with
    /// the opposition on it, beyond 35 m from a loose ball.
    const RELAXED_OWN: f32 = 240.0;
    const RELAXED_THEIRS: f32 = 360.0;
    const RELAXED_LOOSE: f32 = 280.0;
    /// An alert man stays alert until the ball is this much (5 m) further.
    const HYSTERESIS: f32 = 40.0;
    /// Below this condition a midfielder relaxes sooner, by this much of
    /// each distance per point of deficit.
    const TIRED_BELOW: f32 = 0.70;
    const TIRED_SHRINK: f32 = 2.0;
    /// How far past a containing midfielder the carrier has to be (0.5 m)
    /// before he is closing again.
    const GONE_PAST: f32 = 4.0;
    /// A man the plan has given nobody becomes his inside this (10 m), and
    /// stays his out to `PICK_RANGE` (12.5 m).
    const COMMIT_RANGE: f32 = 80.0;
    const PICK_RANGE: f32 = 100.0;
    /// How far from his place he follows his man outside his own area.
    pub const BOUND: f32 = 150.0;
    /// With the ball this far (37.5 m) away in their half there is nobody
    /// for him to mark.
    const BALL_GONE: f32 = 300.0;

    /// The first open door above `level`.
    pub fn above(ctx: &StateProcessingContext, level: ReadLevel) -> Option<MidfielderState> {
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
        Self::above(ctx, level).map(StateChangeResult::with_midfielder_state)
    }

    /// The whole read: where a midfielder with nothing in hand belongs.
    pub fn resolve(ctx: &StateProcessingContext) -> MidfielderState {
        Self::above(ctx, ReadLevel::Phase).unwrap_or_else(|| Self::phase(ctx))
    }

    /// [`Self::resolve`] as the state change a handler returns.
    pub fn change(ctx: &StateProcessingContext) -> StateChangeResult {
        StateChangeResult::with_midfielder_state(Self::resolve(ctx))
    }

    /// For a state the read answers whole: stay while the read still says
    /// `state`, go where it says otherwise.
    pub fn hold(ctx: &StateProcessingContext, state: MidfielderState) -> Option<StateChangeResult> {
        let next = Self::resolve(ctx);
        (next != state).then(|| StateChangeResult::with_midfielder_state(next))
    }

    /// The read for a man who has just played the ball: he no longer has
    /// it, whatever the snapshot of this tick says.
    pub fn released(ctx: &StateProcessingContext) -> MidfielderState {
        ReadLevel::ALL[1..]
            .iter()
            .find_map(|l| Self::door(ctx, *l))
            .unwrap_or_else(|| Self::phase(ctx))
    }

    fn door(ctx: &StateProcessingContext, level: ReadLevel) -> Option<MidfielderState> {
        match level {
            ReadLevel::Possession => ctx
                .ball()
                .mine_to_play()
                .then_some(MidfielderState::Running),
            ReadLevel::Staged => Self::staged(ctx),
            ReadLevel::LooseBall => Self::loose_ball(ctx),
            ReadLevel::Engage => Self::engage(ctx),
            ReadLevel::Mark => Self::mark(ctx),
            ReadLevel::Recover => Self::recover(ctx),
            ReadLevel::Support => Self::support(ctx),
            ReadLevel::Phase => Some(Self::phase(ctx)),
        }
    }

    /// A high ball a contest has given him, still on its way to him: up for
    /// it in `Heading`.
    fn staged(ctx: &StateProcessingContext) -> Option<MidfielderState> {
        (ctx.player.aerial_award && ctx.tick_context.ball.awaits_header_from(ctx.player.id))
            .then_some(MidfielderState::Heading)
    }

    /// The election's own question, asked as the dispatcher will ask it on
    /// the next tick — so a state never starts a chase it will refuse —
    /// and the contest test for a ball in flight that is coming to him.
    fn loose_ball(ctx: &StateProcessingContext) -> Option<MidfielderState> {
        if PlayerFieldPositionGroup::should_force_takeball(
            PlayerFieldPositionGroup::Midfielder,
            ctx.player,
            ctx.context,
            ctx.tick_context,
        ) {
            return Some(MidfielderState::TakeBall);
        }
        (Interception::is_available(ctx) && ctx.ball().is_towards_player_with_angle(0.8))
            .then_some(MidfielderState::Intercepting)
    }

    /// He is the man for the carrier: closing, or containing once he is
    /// in challenge range goal-side of him. The plan's presser goes with a
    /// ball travelling to an opponent as well.
    fn engage(ctx: &StateProcessingContext) -> Option<MidfielderState> {
        let Some(carrier) = ctx.players().opponents().with_ball().next() else {
            return (ctx.team().my_duty() == DefensiveDuty::Press && !ctx.team().is_control_ball())
                .then_some(MidfielderState::Pressing);
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
    pub fn duel_rung(ctx: &StateProcessingContext, carrier: &MatchPlayerLite) -> MidfielderState {
        if carrier.distance(ctx) <= TackleEngagement::COMMIT && Self::is_goal_side(ctx, carrier) {
            MidfielderState::Jockeying
        } else {
            MidfielderState::Pressing
        }
    }

    /// In front of the carrier on his line to goal. Level or behind, he has
    /// been gone past. A man already containing is gone past only once the
    /// carrier is half a body by him, as he gives the carrier up at
    /// `DISENGAGE` rather than `COMMIT`.
    pub fn is_goal_side(ctx: &StateProcessingContext, carrier: &MatchPlayerLite) -> bool {
        let slack = if ctx.player.state == PlayerState::Midfielder(MidfielderState::Jockeying) {
            Self::GONE_PAST
        } else {
            0.0
        };
        RecoveryChallenge::lead(ctx, carrier.position) > -slack
    }

    /// His man, near enough to go to, and nothing that makes `Guarding` give
    /// him up.
    fn mark(ctx: &StateProcessingContext) -> Option<MidfielderState> {
        let man = Self::man(ctx)?;
        let reach = if ctx.team().my_mark().is_some() {
            MarkEngagement::ENGAGE
        } else {
            Self::COMMIT_RANGE
        };
        (man.distance(ctx) <= reach && !Self::refuses(ctx, &man))
            .then_some(MidfielderState::Guarding)
    }

    /// His man: the one the plan gave him or, with none, the most dangerous
    /// opponent near him that the plan has given to nobody. Nobody while his
    /// own side has the ball. Memoised per tick: the doors and the marking
    /// state's steering ask it on the same frozen snapshot.
    pub fn man(ctx: &StateProcessingContext) -> Option<MatchPlayerLite> {
        let tick = ctx.current_tick();
        let cached = ctx
            .tick_context
            .player_agg_cache
            .borrow_mut()
            .slot_mut(ctx.player.id, tick)
            .guard_target;
        match cached {
            Some(target) => {
                debug_assert_eq!(
                    target.map(|p| p.id),
                    Self::pick(ctx).map(|p| p.id),
                    "guard-target memo mismatch (midfielder)"
                );
                target
            }
            None => {
                let target = Self::pick(ctx);
                ctx.tick_context
                    .player_agg_cache
                    .borrow_mut()
                    .slot_mut(ctx.player.id, tick)
                    .guard_target = Some(target);
                target
            }
        }
    }

    fn pick(ctx: &StateProcessingContext) -> Option<MatchPlayerLite> {
        if ctx.team().is_control_ball() {
            return None;
        }
        if let Some(man) = ctx.team().my_mark() {
            return Some(man);
        }
        let team = ctx.team();
        let plan = team.defensive_plan();
        let own_goal = ctx.ball().direction_to_own_goal();
        let ball = ctx.tick_context.positions.ball.position;
        let me = ctx.tick_context.grid.position_of(ctx.player.id);
        let threat = |o: &MatchPlayerLite| {
            let to_goal = (o.position - own_goal).magnitude().min(400.0);
            let to_ball = (o.position - ball).magnitude().min(200.0);
            let to_me = (o.position.x - me.x).hypot(o.position.y - me.y).min(60.0);
            (400.0 - to_goal) / 8.0 + (200.0 - to_ball) / 8.0 + (60.0 - to_me) / 3.0
        };
        let players = ctx.players();
        players
            .opponents()
            .nearby(Self::PICK_RANGE)
            .filter(|o| !o.has_ball(ctx) && !o.tactical_positions.is_goalkeeper())
            .filter(|o| !plan.is_marked(o.id))
            .max_by(|a, b| threat(a).total_cmp(&threat(b)))
    }

    /// What makes him give up a man: the ball gone deep into their half, or
    /// the man taking him further from his place than his bound — except
    /// inside his own area, where the man comes first.
    pub fn refuses(ctx: &StateProcessingContext, man: &MatchPlayerLite) -> bool {
        if !ctx.ball().on_own_side() && ctx.ball().distance() > Self::BALL_GONE {
            return true;
        }
        let in_own_area = ctx
            .context
            .penalty_area(ctx.player.side == Some(PlayerSide::Left))
            .contains(&man.position);
        !in_own_area && ctx.team().distance_from_anchor() > Self::BOUND
    }

    /// Out of his place with no job that keeps him out: the recovery run
    /// with the opposition on the ball, getting back into shape otherwise.
    fn recover(ctx: &StateProcessingContext) -> Option<MidfielderState> {
        if !ShapeStation::should_recover(ctx) {
            return None;
        }
        if ctx.team().is_control_ball() && Self::support(ctx).is_some() {
            return None;
        }
        let dead = RestartHold::taker(ctx.tick_context).is_some();
        Some(if ctx.team().is_control_ball() || dead {
            MidfielderState::Returning
        } else {
            MidfielderState::TrackingBack
        })
    }

    /// His job in his side's attack, as the attack plan gives it: a
    /// touchline, a place in the box or the run in behind, or the short
    /// outlet. Kept behind the ball, or given nothing, he holds his place.
    fn support(ctx: &StateProcessingContext) -> Option<MidfielderState> {
        if !ctx.team().is_control_ball() {
            return None;
        }
        if WideChannel::still_mine(ctx) {
            return Some(MidfielderState::HoldingWidth);
        }
        let team = ctx.team();
        let plan = team.attack_plan();
        let id = ctx.player.id;
        if plan.slot_of(id).is_some() || plan.far_runner == Some(id) {
            Some(MidfielderState::AttackSupporting)
        } else if plan.near_support == Some(id) {
            Some(MidfielderState::CreatingSpace)
        } else {
            None
        }
    }

    /// His place: alert while the play is in reach of it, relaxed while it
    /// is not, with the ball dead or a long way off.
    pub fn phase(ctx: &StateProcessingContext) -> MidfielderState {
        if Self::is_alert(ctx) {
            MidfielderState::HoldingShape
        } else {
            MidfielderState::Walking
        }
    }

    fn is_alert(ctx: &StateProcessingContext) -> bool {
        if RestartHold::taker(ctx.tick_context).is_some() {
            return false;
        }
        let ours = ctx.team().is_control_ball();
        if !ours && ctx.team().my_duty() == DefensiveDuty::Cover {
            return true;
        }
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
        let held = if ctx.player.state == PlayerState::Midfielder(MidfielderState::HoldingShape) {
            Self::HYSTERESIS
        } else {
            0.0
        };
        to_place < (relaxed_from + held) * Self::tired_reach(ctx)
    }

    /// The share of each distance a tired midfielder still reads.
    fn tired_reach(ctx: &StateProcessingContext) -> f32 {
        let condition = ctx.player.player_attributes.condition_percentage() as f32 / 100.0;
        (1.0 - (Self::TIRED_BELOW - condition).max(0.0) * Self::TIRED_SHRINK).clamp(0.4, 1.0)
    }
}
