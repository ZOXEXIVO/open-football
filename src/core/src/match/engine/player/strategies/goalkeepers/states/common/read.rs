//! **One read of the situation for every keeper state out of possession.**
//!
//! The same loose ball used to be claimed with or without the favourite
//! test, a cross attacked from one state and ignored from the next, and a
//! man through on goal met by a sprint or by a stand-off, depending on which
//! state the keeper happened to be in. Every door now lives here once, in
//! priority order, and a state takes the doors above its own level and does
//! its own work at it.

use crate::r#match::goalkeepers::states::GoalkeeperPunchingState;
use crate::r#match::goalkeepers::states::common::{
    KeeperAerialClaim, KeeperBallClaim, KeeperDebug, KeeperDelivery, KeeperFeetDecision,
    KeeperOneOnOne, KeeperRestPosition, KeeperSetPieceStance, KeeperSmother, KeeperSweepLimit,
};
use crate::r#match::goalkeepers::states::state::GoalkeeperState;
use crate::r#match::player::events::PlayerEvent;
use crate::r#match::player::state::PlayerState;
use crate::r#match::player::strategies::players::ops::goalkeeper_skill::GoalkeeperSkillProfile;
use crate::r#match::{
    ClaimOutcome, SaveOutcome, StateChangeResult, StateProcessingContext, VectorExtensions,
};

/// The doors of the read, highest priority first.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum ReadLevel {
    /// The ball is his.
    Possession,
    /// A save he has just made on his feet, to carry out.
    Staged,
    /// A shot is in flight at his goal.
    Shot,
    /// A man has the ball inside his spread.
    Smother,
    /// A man is through and bearing down on him.
    OneOnOne,
    /// A high ball he can claim.
    Aerial,
    /// A loose ball he is favourite for.
    LooseBall,
    /// A ball he has to set himself for.
    Threat,
    /// He is out of position.
    Position,
    /// Relaxed or alert.
    Phase,
}

impl ReadLevel {
    const ALL: [ReadLevel; 10] = [
        ReadLevel::Possession,
        ReadLevel::Staged,
        ReadLevel::Shot,
        ReadLevel::Smother,
        ReadLevel::OneOnOne,
        ReadLevel::Aerial,
        ReadLevel::LooseBall,
        ReadLevel::Threat,
        ReadLevel::Position,
        ReadLevel::Phase,
    ];
}

/// What the read says to the state that asked.
pub enum Reading {
    /// A door opened onto another state.
    Go(StateChangeResult),
    /// The door that opened leads to the state he is in: his own business.
    Mine,
    /// No door opened.
    Clear,
}

impl Reading {
    /// The transition, if there is one to make.
    pub fn go(self) -> Option<StateChangeResult> {
        match self {
            Reading::Go(change) => Some(change),
            Reading::Mine | Reading::Clear => None,
        }
    }
}

pub struct KeeperRead;

impl KeeperRead {
    /// Standing's measured constants, which every door now shares.
    ///
    /// A ball he gathers rather than goes to meet: 10u = 1.25 m.
    const GATHER_REACH: f32 = 10.0;
    /// Past this he is making a save, not collecting: 2.0 u/tick = 25 m/s.
    const DRIVEN: f32 = 2.0;
    /// Below this the ball is on the deck — scooped up, not caught.
    const GROUND_BALL: f32 = 1.2;
    /// A race he goes for, before `command_of_area` widens it: 5 m.
    const RACE: f32 = 40.0;
    /// A covered carrier this close sets him: 12.5 m.
    const CLOSE_CARRIER: f32 = 100.0;
    /// A ball coming at him from this far sets him, before anticipation
    /// widens it: 37.5 m.
    const INCOMING: f32 = 300.0;
    /// The phase distances from his goal (D1), and the margin each gets so
    /// a phase is left only beyond the distance it was entered at: the ball
    /// not his side's within 60 m, any ball within 35 m; 5 m of margin.
    const OPPOSITION_NEAR: f32 = 480.0;
    const ANY_NEAR: f32 = 280.0;
    const PHASE_MARGIN: f32 = 40.0;
    /// Possession that has just come back to his side has not settled: once
    /// alert he stays so for three seconds. A distance margin cannot do this for
    /// him, because a change of possession moves no distance. AI ticks.
    const SETTLE_TICKS: u64 = 150;

    /// The doors above `level`.
    pub fn above(ctx: &StateProcessingContext, level: ReadLevel) -> Reading {
        Self::doors(ctx, ReadLevel::ALL.into_iter().filter(|door| *door < level))
    }

    /// The doors from `level` down, the phase last.
    pub fn from(ctx: &StateProcessingContext, level: ReadLevel) -> Reading {
        Self::doors(
            ctx,
            ReadLevel::ALL.into_iter().filter(|door| *door >= level),
        )
    }

    fn doors(ctx: &StateProcessingContext, doors: impl Iterator<Item = ReadLevel>) -> Reading {
        let here = ctx.player.state;
        for door in doors {
            let Some(next) = Self::door(ctx, door) else {
                continue;
            };
            return if next == here {
                Reading::Mine
            } else {
                Reading::Go(Self::enter(ctx, door, next))
            };
        }
        Reading::Clear
    }

    /// The state a door leads to, if it is open.
    fn door(ctx: &StateProcessingContext, door: ReadLevel) -> Option<PlayerState> {
        let to = |state| Some(PlayerState::Goalkeeper(state));
        match door {
            ReadLevel::Possession => {
                // A ball he has just saved or claimed and held is the staged
                // outcome's, which takes him through the gather first.
                if !ctx.player.has_ball(ctx)
                    || ctx.player.pending_save.is_some()
                    || ctx.player.pending_claim == Some(ClaimOutcome::Caught)
                {
                    return None;
                }
                // In his gloves he holds it; his but beyond his feet is
                // still a chase.
                to(if ctx.tick_context.ball.held_in_hands {
                    GoalkeeperState::HoldingBall
                } else if ctx.ball().mine_to_play() {
                    KeeperFeetDecision::state_for(ctx)
                } else {
                    GoalkeeperState::TakeBall
                })
            }
            // Held: into his body (`Catching`, the gather) before anything
            // else. Loose: nothing to carry out, and the doors below have
            // him — a live ball close in sets him again.
            // A claim the contest has decided: caught, the same gather; his
            // fist on it, the punch; flapped, the same as a loose save.
            ReadLevel::Staged => match (ctx.player.pending_save, ctx.player.pending_claim) {
                (Some(SaveOutcome::Held), _) | (None, Some(ClaimOutcome::Caught)) => {
                    to(GoalkeeperState::Catching)
                }
                (None, Some(ClaimOutcome::Punched)) => to(GoalkeeperState::Punching),
                _ => None,
            },
            ReadLevel::Shot => Self::live_shot(ctx)
                .then_some(())
                .and_then(|_| to(GoalkeeperState::PreparingForSave)),
            ReadLevel::Smother => {
                KeeperSmother::assess(ctx).and_then(|_| to(GoalkeeperState::Diving))
            }
            ReadLevel::OneOnOne => {
                KeeperOneOnOne::duel(ctx).and_then(|_| to(GoalkeeperState::OneOnOne))
            }
            ReadLevel::Aerial => KeeperAerialClaim::assess(ctx).and_then(|claim| {
                to(if !claim.at_contact(ctx.player.position) {
                    GoalkeeperState::ComingOut
                } else if claim.standing {
                    GoalkeeperState::Catching
                } else {
                    GoalkeeperState::Jumping
                })
            }),
            ReadLevel::LooseBall => Self::loose_ball(ctx).and_then(to),
            ReadLevel::Threat => Self::threatened(ctx)
                .then_some(())
                .and_then(|_| to(GoalkeeperState::PreparingForSave)),
            ReadLevel::Position => Self::out_of_position(ctx)
                .then_some(())
                .and_then(|_| to(GoalkeeperState::ReturningToGoal)),
            ReadLevel::Phase => to(Self::phase(ctx)),
        }
    }

    /// The transition through `door`, with what going through it involves:
    /// the smother is played as he commits, a claim is booked as it starts,
    /// and a ball he may not handle is taken with his feet.
    fn enter(
        ctx: &StateProcessingContext,
        door: ReadLevel,
        next: PlayerState,
    ) -> StateChangeResult {
        let PlayerState::Goalkeeper(state) = next else {
            unreachable!("every door leads to a keeper state");
        };
        match door {
            ReadLevel::Smother => {
                let attempt = KeeperSmother::assess(ctx).expect("the door opened on it");
                KeeperSmother::commit(ctx, &attempt)
            }
            ReadLevel::Staged if state == GoalkeeperState::Punching => {
                GoalkeeperPunchingState::strike(ctx)
            }
            ReadLevel::Aerial => {
                let mut result = StateChangeResult::with_goalkeeper_state(state);
                if let Some(claim) = KeeperAerialClaim::assess(ctx) {
                    let intent = KeeperAerialClaim::intent(ctx, &claim);
                    KeeperAerialClaim::note_start(ctx, &claim, intent);
                    result.claim_intent = Some(intent);
                }
                result
            }
            ReadLevel::LooseBall
                if !ctx.ball().handling_verdict().is_legal() && Self::within_reach(ctx) =>
            {
                let mut change = StateChangeResult::with_goalkeeper_state(state);
                change
                    .events
                    .add_player_event(PlayerEvent::GainBall(ctx.player.id));
                change
            }
            _ => StateChangeResult::with_goalkeeper_state(state),
        }
    }

    /// A shot at his goal the save model has not settled. Once it has —
    /// held, tipped, spilled or beaten — the shot is over for him: what is
    /// left is a ball.
    pub fn live_shot(ctx: &StateProcessingContext) -> bool {
        Self::shot_at_his_goal(ctx)
            && ctx
                .tick_context
                .ball
                .cached_shot_target
                .as_ref()
                .is_some_and(|t| !t.save_rolled)
    }

    /// A shot at his goal still in flight, settled by the save model or not.
    pub fn shot_at_his_goal(ctx: &StateProcessingContext) -> bool {
        ctx.tick_context
            .ball
            .cached_shot_target
            .as_ref()
            .is_some_and(|t| Some(t.defending_side) == ctx.player.side)
    }

    fn within_reach(ctx: &StateProcessingContext) -> bool {
        ctx.ball().distance() < Self::GATHER_REACH
            && ctx.tick_context.positions.ball.velocity.norm() < Self::DRIVEN
    }

    /// The gather half of the loose-ball door, for the states that have
    /// gone to the ball and are past asking whether to: his loose ball,
    /// within reach.
    pub fn gather(ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        (Self::his_loose_ball(ctx) && Self::within_reach(ctx)).then(|| {
            Self::enter(
                ctx,
                ReadLevel::LooseBall,
                PlayerState::Goalkeeper(Self::gathering(ctx)),
            )
        })
    }

    /// A loose ball is his when he is favourite for it, it is not one he has
    /// just played, and it is on his side of the pitch. A shot at his goal is
    /// never one, live or settled: one that beat him stays beaten.
    fn his_loose_ball(ctx: &StateProcessingContext) -> bool {
        !ctx.ball().is_owned()
            && !Self::shot_at_his_goal(ctx)
            && ctx.ball().on_own_side()
            && !ctx.ball().blocked_from_recollecting()
            && !KeeperDelivery::is_his(ctx)
            && KeeperBallClaim::is_favourite(ctx)
    }

    /// Picked up off the ground, caught off it, or played with his feet
    /// where his hands are not allowed.
    fn gathering(ctx: &StateProcessingContext) -> GoalkeeperState {
        if !ctx.ball().handling_verdict().is_legal() {
            return KeeperFeetDecision::without_hands(ctx);
        }
        if ctx.tick_context.positions.ball.position.z < Self::GROUND_BALL {
            GoalkeeperState::PickingUpBall
        } else {
            GoalkeeperState::Catching
        }
    }

    /// His loose ball: gathered within reach, and further out raced for if
    /// it lies in the ground he defends.
    fn loose_ball(ctx: &StateProcessingContext) -> Option<GoalkeeperState> {
        if !Self::his_loose_ball(ctx) {
            return None;
        }
        if Self::within_reach(ctx) {
            return Some(Self::gathering(ctx));
        }
        let prof = GoalkeeperSkillProfile::from_ctx(ctx);
        (ctx.ball().distance() < Self::RACE * (1.0 + prof.command_of_area * 0.3)
            && KeeperSweepLimit::covers(ctx, ctx.tick_context.positions.ball.position, &prof)
            && KeeperSetPieceStance::pending(ctx).is_none())
        .then_some(GoalkeeperState::ComingOut)
    }

    /// A covered carrier close in, or a ball coming at him off the
    /// opponents' play. His own side's ball never sets him.
    fn threatened(ctx: &StateProcessingContext) -> bool {
        if ctx
            .players()
            .opponents()
            .with_ball()
            .next()
            .is_some_and(|carrier| carrier.distance(ctx) < Self::CLOSE_CARRIER)
        {
            return true;
        }
        let anticipation = GoalkeeperSkillProfile::from_ctx(ctx).positioning;
        ctx.ball().is_towards_player_with_angle(0.6)
            && ctx.ball().on_own_side()
            && (KeeperDebug::calm_off() || !ctx.team().is_control_ball())
            && ctx.tick_context.positions.ball.velocity.norm() > 0.5
            && ctx.ball().distance() < Self::INCOMING * (1.0 + anticipation * 0.5)
    }

    /// Far enough off his spot that he runs back rather than adjusts: 12.5
    /// m for an ordinary reader of the game, closer for a good one.
    fn out_of_position(ctx: &StateProcessingContext) -> bool {
        let read = GoalkeeperSkillProfile::from_ctx(ctx).positioning;
        let tolerance = 100.0 - (read - GoalkeeperSkillProfile::POPULATION_READ) * 80.0;
        ctx.player
            .position
            .distance_to(&KeeperRestPosition::for_keeper(ctx))
            > tolerance
    }

    /// Relaxed (`Walking`) or alert (`Standing`). Alert while the ball is not
    /// his side's within 60 m of his goal, or is anybody's within 35 m;
    /// relaxed otherwise. Out of the alert phase only beyond each distance
    /// plus the margin.
    ///
    /// Whose ball it is is settled possession (`TeamPossession`), not who
    /// touched it last. Read off ownership, every pass among his own men and
    /// every loose touch inside 45 m woke him and the next touch put him
    /// back: measured, 7.0 changes a minute with 16% reversed inside 300 ms.
    /// A loose ball stays his side's until the other side is decisively
    /// closer, which is the moment a contested one starts to threaten him.
    pub fn phase(ctx: &StateProcessingContext) -> GoalkeeperState {
        if ctx.player.state == PlayerState::Goalkeeper(GoalkeeperState::Standing)
            && ctx.in_state_time < Self::SETTLE_TICKS
        {
            return GoalkeeperState::Standing;
        }
        let alert_now = ctx.player.state != PlayerState::Goalkeeper(GoalkeeperState::Walking);
        let margin = if alert_now { Self::PHASE_MARGIN } else { 0.0 };
        let from_goal = (ctx.tick_context.positions.ball.position
            - ctx.ball().direction_to_own_goal())
        .magnitude();
        let alert = (!ctx.team().is_control_ball() && from_goal < Self::OPPOSITION_NEAR + margin)
            || from_goal < Self::ANY_NEAR + margin;
        if alert {
            GoalkeeperState::Standing
        } else {
            GoalkeeperState::Walking
        }
    }
}
